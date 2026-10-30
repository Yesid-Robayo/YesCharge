//! Acceso HID de bajo nivel sobre la API de Windows (hid.dll + setupapi), SOLO DE LECTURA.
//!
//! Este módulo no tiene ninguna función para enviar datos a un dispositivo: `HidDevice::open` abre
//! sin derecho de escritura (`GENERIC_READ`), así que Windows rechazaría cualquier envío. Lo único
//! capaz de enviar algo es `ask`, opcional y solo con la aceptación del usuario.
//!
//! Convención de buffers: SIEMPRE incluyen el byte de report ID en `[0]` (0x00 si el
//! dispositivo no numera sus reportes), tal como los entrega Windows.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid,
    HidD_GetManufacturerString, HidD_GetPreparsedData, HidD_GetProductString, HidP_GetCaps,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_DEVICE_NOT_CONNECTED, ERROR_FILE_NOT_FOUND, ERROR_IO_INCOMPLETE,
    ERROR_IO_PENDING, ERROR_SHARING_VIOLATION, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};
use windows_sys::Win32::System::Threading::CreateEventW;
use windows_sys::core::GUID;

/// Una interfaz HID (colección de nivel superior). Un dispositivo físico suele exponer varias.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HidInterface {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub version: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    pub product: String,
    pub manufacturer: String,
}

impl HidInterface {
    /// Página de uso definida por el fabricante (0xFF00-0xFFFF). Es la única que se escucha: en
    /// las páginas estándar viajan las teclas y el movimiento del ratón, que nunca se graban.
    pub fn is_vendor_defined(&self) -> bool {
        self.usage_page >= 0xFF00
    }

    /// De qué aparato físico es esta interfaz: dos aparatos iguales comparten todo lo demás.
    pub fn container(&self) -> Option<String> {
        crate::sources::container_id(&self.path)
    }
}

/// Elige entre interfaces equivalentes la del aparato `container`. Si ese aparato no aparece y solo
/// hay uno de ese modelo, es ese: conectarlo por cable o en otro puerto puede cambiar su identidad.
pub fn pick<'a>(found: impl Iterator<Item = &'a HidInterface>, container: Option<&str>) -> Option<&'a HidInterface> {
    let found: Vec<&HidInterface> = found.collect();
    let Some(wanted) = container else { return found.first().copied() };
    found.iter().find(|i| i.container().as_deref() == Some(wanted)).copied().or(if found.len() == 1 {
        found.first().copied()
    } else {
        None
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HidError {
    /// Otro programa tiene la interfaz abierta en exclusiva.
    Busy,
    /// El dispositivo se desconectó.
    Gone,
    /// Un límite de seguridad de `ask` impidió la operación.
    Blocked(&'static str),
    /// La interfaz no envía input reports.
    Unsupported,
    Io(u32),
}

impl std::fmt::Display for HidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HidError::Busy => write!(f, "ocupado por otro programa"),
            HidError::Gone => write!(f, "desconectado"),
            HidError::Blocked(why) => write!(f, "bloqueado por seguridad: {why}"),
            HidError::Unsupported => write!(f, "no envía reportes"),
            HidError::Io(code) => write!(f, "error de E/S {code}"),
        }
    }
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

pub(crate) fn last_error() -> HidError {
    let code = unsafe { GetLastError() };
    match code {
        ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION => HidError::Busy,
        ERROR_DEVICE_NOT_CONNECTED | ERROR_FILE_NOT_FOUND => HidError::Gone,
        _ => HidError::Io(code),
    }
}

/// Lista todas las interfaces HID presentes. Solo consulta metadatos (abre con acceso 0,
/// que no permite leer ni escribir reportes).
pub fn enumerate() -> Vec<HidInterface> {
    let mut out = Vec::new();
    unsafe {
        let mut guid: GUID = zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(&guid, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == -1 {
            return out;
        }
        let mut index = 0u32;
        loop {
            let mut data: SP_DEVICE_INTERFACE_DATA = zeroed();
            data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, null(), &guid, index, &mut data) == 0 {
                break;
            }
            index += 1;
            let mut required = 0u32;
            SetupDiGetDeviceInterfaceDetailW(set, &data, null_mut(), 0, &mut required, null_mut());
            if required == 0 {
                continue;
            }
            // u64 para que el buffer quede alineado como la estructura
            let mut raw = vec![0u64; (required as usize).div_ceil(8)];
            let detail = raw.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &data, detail, required, null_mut(), null_mut()) == 0 {
                continue;
            }
            let path_ptr = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
            let path_len = (required as usize - 4) / 2;
            let path = from_wide(std::slice::from_raw_parts(path_ptr, path_len));
            if let Some(info) = query(&path) {
                out.push(info);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

fn query(path: &str) -> Option<HidInterface> {
    unsafe {
        let wpath = wide(path);
        let h =
            CreateFileW(wpath.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut attrs: HIDD_ATTRIBUTES = zeroed();
        attrs.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
        let ok = HidD_GetAttributes(h, &mut attrs);
        let mut caps: HIDP_CAPS = zeroed();
        let mut pp = 0isize;
        let mut caps_ok = false;
        if HidD_GetPreparsedData(h, &mut pp) {
            caps_ok = HidP_GetCaps(pp, &mut caps) == HIDP_STATUS_SUCCESS;
            HidD_FreePreparsedData(pp);
        }
        let mut buf = [0u16; 128];
        let product = if HidD_GetProductString(h, buf.as_mut_ptr() as *mut c_void, 256) {
            from_wide(&buf)
        } else {
            String::new()
        };
        buf = [0u16; 128];
        let manufacturer = if HidD_GetManufacturerString(h, buf.as_mut_ptr() as *mut c_void, 256) {
            from_wide(&buf)
        } else {
            String::new()
        };
        CloseHandle(h);
        if !ok || !caps_ok {
            return None;
        }
        Some(HidInterface {
            path: path.to_string(),
            vid: attrs.VendorID,
            pid: attrs.ProductID,
            version: attrs.VersionNumber,
            usage_page: caps.UsagePage,
            usage: caps.Usage,
            input_len: caps.InputReportByteLength,
            output_len: caps.OutputReportByteLength,
            feature_len: caps.FeatureReportByteLength,
            product,
            manufacturer,
        })
    }
}

/// Interfaz HID abierta SOLO para leer, con tiempos de espera.
pub struct HidDevice {
    handle: HANDLE,
    event: HANDLE,
    pub info: HidInterface,
}

// El handle es de uso exclusivo del dueño del valor; Windows permite usarlo desde otro hilo.
unsafe impl Send for HidDevice {}

impl HidDevice {
    /// Abre la interfaz con `GENERIC_READ` únicamente: con este handle no se puede escribir.
    pub fn open(info: &HidInterface) -> Result<Self, HidError> {
        Self::open_with(info, GENERIC_READ)
    }

    /// Solo para `ask`, que únicamente lo usa tras la aceptación del usuario.
    pub(crate) fn open_for_query(info: &HidInterface) -> Result<Self, HidError> {
        Self::open_with(info, GENERIC_READ | GENERIC_WRITE)
    }

    pub(crate) fn raw_handles(&self) -> (HANDLE, HANDLE) {
        (self.handle, self.event)
    }

    fn open_with(info: &HidInterface, rights: u32) -> Result<Self, HidError> {
        unsafe {
            let wpath = wide(&info.path);
            let handle = CreateFileW(
                wpath.as_ptr(),
                rights,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(last_error());
            }
            let event = CreateEventW(null(), 1, 0, null());
            if event.is_null() {
                let err = last_error();
                CloseHandle(handle);
                return Err(err);
            }
            Ok(Self { handle, event, info: info.clone() })
        }
    }

    /// Lee el siguiente input report que el dispositivo envíe por su cuenta. `Ok(None)` si no
    /// llegó nada en `timeout_ms` (la lectura pendiente se cancela).
    pub fn read(&self, timeout_ms: u32) -> Result<Option<Vec<u8>>, HidError> {
        let len = self.info.input_len as usize;
        if len == 0 {
            return Err(HidError::Unsupported);
        }
        let mut buf = vec![0u8; len];
        unsafe {
            let mut ov: OVERLAPPED = zeroed();
            ov.hEvent = self.event;
            if ReadFile(self.handle, buf.as_mut_ptr(), len as u32, null_mut(), &mut ov) == 0
                && GetLastError() != ERROR_IO_PENDING
            {
                return Err(last_error());
            }
            let mut n = 0u32;
            if GetOverlappedResultEx(self.handle, &ov, &mut n, timeout_ms, 0) != 0 {
                buf.truncate(n as usize);
                return Ok(Some(buf));
            }
            let code = GetLastError();
            if code == WAIT_TIMEOUT || code == ERROR_IO_INCOMPLETE {
                CancelIoEx(self.handle, &ov);
                // esperar a que la cancelación termine antes de soltar el buffer
                GetOverlappedResult(self.handle, &ov, &mut n, 1);
                return Ok(None);
            }
            Err(match code {
                ERROR_DEVICE_NOT_CONNECTED | ERROR_FILE_NOT_FOUND => HidError::Gone,
                _ => HidError::Io(code),
            })
        }
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            CancelIoEx(self.handle, null());
            CloseHandle(self.handle);
            CloseHandle(self.event);
        }
    }
}

#[cfg(test)]
mod pick_tests {
    use super::*;

    fn iface() -> HidInterface {
        HidInterface {
            path: String::new(),
            vid: 1,
            pid: 2,
            version: 0,
            usage_page: 0xff00,
            usage: 1,
            input_len: 0,
            output_len: 0,
            feature_len: 0,
            product: String::new(),
            manufacturer: String::new(),
        }
    }

    #[test]
    fn with_one_unit_of_a_model_it_is_that_one_even_if_its_identity_changed() {
        let one = [iface()];
        assert!(pick(one.iter(), None).is_some());
        assert!(pick(one.iter(), Some("otro")).is_some());
    }

    #[test]
    fn with_two_units_it_never_guesses_which_one() {
        let two = [iface(), iface()];
        assert!(pick(two.iter(), Some("otro")).is_none());
        assert!(pick(two.iter(), None).is_some(), "sin contenedor guardado sigue funcionando como antes");
    }
}
