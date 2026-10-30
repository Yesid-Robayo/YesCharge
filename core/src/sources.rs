//! Fuentes de batería que NO envían nada al dispositivo:
//!
//! - **Bluetooth**: la propiedad de batería que Windows ya mantiene (la de Configuración).
//! - **XInput**: el nivel que Windows da de los mandos compatibles con Xbox.
//! - **HID estándar**: el valor "Battery Strength" (página 0x06, uso 0x20) si el propio
//!   dispositivo lo declara; se lee con GET_REPORT, que es una lectura por definición.

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW,
    CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, HidD_FreePreparsedData, HidD_GetFeature, HidD_GetInputReport,
    HidD_GetPreparsedData, HidP_Feature, HidP_GetUsageValue, HidP_GetValueCaps, HidP_Input,
};
use windows_sys::Win32::Devices::Properties::{
    DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_FriendlyName, DEVPROP_TYPE_BOOLEAN, DEVPROP_TYPE_BYTE,
    DEVPROP_TYPE_STRING, DEVPROPTYPE,
};
use windows_sys::Win32::Foundation::{CloseHandle, DEVPROPKEY, GENERIC_READ, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows_sys::Win32::UI::Input::XboxController::{
    BATTERY_DEVTYPE_GAMEPAD, BATTERY_LEVEL_EMPTY, BATTERY_LEVEL_FULL, BATTERY_LEVEL_LOW, BATTERY_LEVEL_MEDIUM,
    BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_WIRED, XINPUT_BATTERY_INFORMATION, XInputGetBatteryInformation,
};
use windows_sys::core::GUID;

use crate::hid::HidInterface;
use crate::reading::{Charge, Presence, RawReading};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Lee un valor de texto de `HKLM\<key>`.
pub fn reg_string(key: &str, value: &str) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 128];
        let mut size = (buf.len() * 2) as u32;
        let k = wide(key);
        let v = wide(value);
        let r = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        );
        (r == 0).then(|| from_wide(&buf))
    }
}

// ---------------- Bluetooth ----------------

/// Propiedad de batería de Bluetooth que mantiene Windows (la que muestra Configuración).
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY =
    DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };

/// Si el dispositivo Bluetooth está conectado ahora mismo.
const DEVPKEY_BLUETOOTH_CONNECTED: DEVPROPKEY =
    DEVPROPKEY { fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29), pid: 15 };

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothBattery {
    pub instance_id: String,
    pub name: String,
    pub battery: u8,
}

fn devnode_property(inst: u32, key: &DEVPROPKEY) -> Option<(DEVPROPTYPE, Vec<u8>)> {
    unsafe {
        let mut ty: DEVPROPTYPE = 0;
        let mut size = 0u32;
        CM_Get_DevNode_PropertyW(inst, key, &mut ty, null_mut(), &mut size, 0);
        if size == 0 || size > 4096 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if CM_Get_DevNode_PropertyW(inst, key, &mut ty, buf.as_mut_ptr(), &mut size, 0) != CR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some((ty, buf))
    }
}

fn devnode_string(inst: u32, key: &DEVPROPKEY) -> Option<String> {
    let (ty, buf) = devnode_property(inst, key)?;
    if ty != DEVPROP_TYPE_STRING {
        return None;
    }
    let wide: Vec<u16> = buf.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(from_wide(&wide).trim().to_string()).filter(|s| !s.is_empty())
}

fn locate(instance_id: &str) -> Option<u32> {
    unsafe {
        let mut inst = 0u32;
        let id = wide(instance_id);
        (CM_Locate_DevNodeW(&mut inst, id.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) == CR_SUCCESS).then_some(inst)
    }
}

fn present_device_ids() -> Vec<String> {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_ID_List_SizeW(&mut len, null(), CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(null(), buf.as_mut_ptr(), len, CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
    }
}

fn bluetooth_battery_of(inst: u32) -> Option<u8> {
    let (ty, buf) = devnode_property(inst, &DEVPKEY_BLUETOOTH_BATTERY)?;
    (ty == DEVPROP_TYPE_BYTE && !buf.is_empty() && buf[0] <= 100).then_some(buf[0])
}

fn bluetooth_connected(inst: u32) -> Option<bool> {
    let (ty, buf) = devnode_property(inst, &DEVPKEY_BLUETOOTH_CONNECTED)?;
    (ty == DEVPROP_TYPE_BOOLEAN && !buf.is_empty()).then_some(buf[0] != 0)
}

/// Dispositivos Bluetooth con batería conocida por Windows (sin repetir nombres).
pub fn bluetooth_batteries() -> Vec<BluetoothBattery> {
    let mut out: Vec<BluetoothBattery> = Vec::new();
    for id in present_device_ids() {
        if !id.to_ascii_uppercase().starts_with("BTH") {
            continue;
        }
        let Some(inst) = locate(&id) else { continue };
        let Some(battery) = bluetooth_battery_of(inst) else { continue };
        let name = devnode_string(inst, &DEVPKEY_Device_FriendlyName)
            .or_else(|| devnode_string(inst, &DEVPKEY_Device_DeviceDesc))
            .unwrap_or_else(|| crate::i18n::tr("Dispositivo Bluetooth", "Bluetooth device").into());
        if out.iter().any(|b| b.name == name) {
            continue;
        }
        out.push(BluetoothBattery { instance_id: id, name, battery });
    }
    out
}

pub fn read_bluetooth(instance_id: &str) -> RawReading {
    match locate(instance_id) {
        None => RawReading::absent(Presence::NotFound, "Bluetooth no presente"),
        // Windows conserva la última batería aunque el dispositivo ya no esté conectado
        Some(inst) if bluetooth_connected(inst) == Some(false) => {
            RawReading::absent(Presence::Asleep, "Bluetooth desconectado")
        }
        Some(inst) => match bluetooth_battery_of(inst) {
            Some(b) => RawReading {
                presence: Presence::Online,
                battery: Some(b),
                charge: Charge::Unknown,
                approximate: false,
                no_percentage: false,
                heard_only: false,
                detail: "Bluetooth (Windows)".into(),
            },
            None => RawReading::absent(Presence::Asleep, "Bluetooth sin dato de batería"),
        },
    }
}

// ---------------- XInput ----------------

/// Mandos XInput conectados: `(ranura, lectura)`.
pub fn xinput_pads() -> Vec<(u8, RawReading)> {
    (0..4u8).filter_map(|slot| Some(slot).zip(read_xinput_raw(slot))).collect()
}

fn read_xinput_raw(slot: u8) -> Option<RawReading> {
    unsafe {
        let mut info: XINPUT_BATTERY_INFORMATION = zeroed();
        if XInputGetBatteryInformation(slot as u32, BATTERY_DEVTYPE_GAMEPAD, &mut info) != 0
            || info.BatteryType == BATTERY_TYPE_DISCONNECTED
        {
            return None;
        }
        if info.BatteryType == BATTERY_TYPE_WIRED {
            return Some(RawReading {
                presence: Presence::Online,
                battery: None,
                charge: Charge::Charging,
                approximate: true,
                no_percentage: false,
                heard_only: false,
                detail: "XInput por cable".into(),
            });
        }
        // XInput solo da 4 niveles: se muestran como porcentaje aproximado.
        let pct = match info.BatteryLevel {
            BATTERY_LEVEL_EMPTY => 5,
            BATTERY_LEVEL_LOW => 25,
            BATTERY_LEVEL_MEDIUM => 60,
            BATTERY_LEVEL_FULL => 100,
            _ => return None,
        };
        Some(RawReading {
            presence: Presence::Online,
            battery: Some(pct),
            charge: Charge::Discharging,
            approximate: true,
            no_percentage: false,
            heard_only: false,
            detail: "XInput".into(),
        })
    }
}

pub fn read_xinput(slot: u8) -> RawReading {
    read_xinput_raw(slot).unwrap_or_else(|| RawReading::absent(Presence::NotFound, "mando XInput no conectado"))
}

// ---------------- HID estándar ----------------

const PAGE_GENERIC_DEVICE: u16 = 0x06;
const USAGE_BATTERY_STRENGTH: u16 = 0x20;

/// `true` si la interfaz declara la batería estándar en su descriptor.
pub fn declares_standard_battery(iface: &HidInterface) -> bool {
    with_preparsed(iface, 0, |_, pp| battery_caps(pp).is_some()).unwrap_or(false)
}

fn with_preparsed<T>(iface: &HidInterface, access: u32, f: impl FnOnce(*mut c_void, isize) -> T) -> Option<T> {
    unsafe {
        let path = wide(&iface.path);
        let h = CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut pp = 0isize;
        let out = if HidD_GetPreparsedData(h, &mut pp) {
            let r = f(h, pp);
            HidD_FreePreparsedData(pp);
            Some(r)
        } else {
            None
        };
        CloseHandle(h);
        out
    }
}

/// Busca el valor Battery Strength en los reportes feature e input.
fn battery_caps(pp: isize) -> Option<(i32, HIDP_VALUE_CAPS)> {
    for report_type in [HidP_Feature, HidP_Input] {
        unsafe {
            let mut caps = vec![HIDP_VALUE_CAPS::default(); 64];
            let mut n = caps.len() as u16;
            if HidP_GetValueCaps(report_type, caps.as_mut_ptr(), &mut n, pp) != HIDP_STATUS_SUCCESS {
                continue;
            }
            for c in &caps[..n as usize] {
                let usage = if c.IsRange { c.Anonymous.Range.UsageMin } else { c.Anonymous.NotRange.Usage };
                if c.UsagePage == PAGE_GENERIC_DEVICE && usage == USAGE_BATTERY_STRENGTH {
                    return Some((report_type, *c));
                }
            }
        }
    }
    None
}

/// Lee la batería estándar (solo GET_REPORT, nunca SET_REPORT). Prueba con un handle de
/// lectura y, si Windows lo
/// tiene en exclusiva (ratones y teclados), con uno sin permisos.
pub fn read_standard_battery(iface: &HidInterface) -> RawReading {
    for access in [GENERIC_READ, 0] {
        let r = with_preparsed(iface, access, |h, pp| unsafe {
            let (report_type, caps) = battery_caps(pp)?;
            let len = if report_type == HidP_Feature { iface.feature_len } else { iface.input_len } as usize;
            if len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            buf[0] = caps.ReportID;
            let ok = if report_type == HidP_Feature {
                HidD_GetFeature(h, buf.as_mut_ptr() as *mut c_void, len as u32)
            } else {
                HidD_GetInputReport(h, buf.as_mut_ptr() as *mut c_void, len as u32)
            };
            if !ok {
                return None;
            }
            let mut value = 0u32;
            if HidP_GetUsageValue(
                report_type,
                PAGE_GENERIC_DEVICE,
                caps.LinkCollection,
                USAGE_BATTERY_STRENGTH,
                &mut value,
                pp,
                buf.as_ptr(),
                len as u32,
            ) != HIDP_STATUS_SUCCESS
            {
                return None;
            }
            let (min, max) = (caps.LogicalMin as i64, caps.LogicalMax as i64);
            if max <= min {
                return None;
            }
            let pct = ((value as i64 - min).clamp(0, max - min) * 100 / (max - min)) as u8;
            Some(pct)
        });
        if let Some(Some(pct)) = r {
            return RawReading {
                presence: Presence::Online,
                battery: Some(pct),
                charge: Charge::Unknown,
                approximate: false,
                no_percentage: false,
                heard_only: false,
                detail: "HID estándar".into(),
            };
        }
    }
    RawReading::absent(Presence::Asleep, "batería estándar no disponible")
}
