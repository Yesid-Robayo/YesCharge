//! Comunicación entre la ventana y el servicio de bandeja (dos procesos del mismo exe).
//!
//! El servicio tiene una ventana oculta de clase `SERVICE_CLASS`; la ventana le manda órdenes
//! con `PostMessageW`. Las órdenes solo piden refrescar, recargar la config o salir.

use std::ptr::null;

use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP};

pub const SERVICE_CLASS: &str = "YesChargeService";

pub const MSG_REFRESH: u32 = WM_APP + 1;
pub const MSG_RELOAD: u32 = WM_APP + 2;
pub const MSG_QUIT: u32 = WM_APP + 3;
/// Uso interno del servicio: el hilo de consultas avisa de un estado nuevo.
pub const MSG_SNAPSHOT: u32 = WM_APP + 4;
/// Uso interno del servicio: eventos del icono de la bandeja.
pub const MSG_TRAY: u32 = WM_APP + 10;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `true` si el servicio está en marcha y recibió la orden.
pub fn send(msg: u32) -> bool {
    unsafe {
        let class = wide(SERVICE_CLASS);
        let hwnd = FindWindowW(class.as_ptr(), null());
        !hwnd.is_null() && PostMessageW(hwnd, msg, 0, 0) != 0
    }
}

pub fn service_running() -> bool {
    unsafe {
        let class = wide(SERVICE_CLASS);
        !FindWindowW(class.as_ptr(), null()).is_null()
    }
}

/// Arranca otra instancia del exe con `arg` (`--window` o `--background`).
pub fn spawn_self(arg: &str) {
    if let Ok(exe) = std::env::current_exe()
        && let Err(e) = std::process::Command::new(exe).arg(arg).spawn()
    {
        yescharge_core::log::error(&format!("no se pudo abrir {arg}: {e}"));
    }
}

/// Abre una URL o carpeta con el programa predeterminado (solo por acción del usuario).
pub fn open_external(target: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        let op = wide("open");
        let t = wide(target);
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), t.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}
