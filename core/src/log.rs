//! Log en `%APPDATA%\YesCharge\yescharge.log`.
//!
//! Cada línea lleva hora, proceso y nivel, y se escribe al disco al momento. Al arrancar, si
//! pasa de `MAX_BYTES`, se rota a `yescharge.old.log`. Nunca se registran datos personales
//! (ni rutas con el usuario, ni nombre del equipo).

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 512 * 1024;

static ROLE: OnceLock<&'static str> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

/// Prepara el log para este proceso (`"servicio"` o `"ventana"`) y escribe la cabecera.
pub fn init(role: &'static str) {
    let _ = ROLE.set(role);
    let path = crate::paths::log_file();
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_file_name("yescharge.old.log"));
    }
    info(&format!("YesCharge {} arrancando ({role}), Windows {}", crate::VERSION, windows_build()));
}

/// Versión de Windows desde el registro (p. ej. "25H2 build 26300"); "?" si no se puede leer.
fn windows_build() -> String {
    let key = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let build = crate::sources::reg_string(key, "CurrentBuild").unwrap_or_else(|| "?".into());
    let display = crate::sources::reg_string(key, "DisplayVersion").unwrap_or_default();
    format!("{display} build {build}").trim().to_string()
}

fn stamp() -> String {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

fn write(level: &str, msg: &str) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let role = ROLE.get().copied().unwrap_or("?");
    let line = format!("{} [{role}] {level:5} {msg}\r\n", stamp());
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(crate::paths::log_file()) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
    #[cfg(debug_assertions)]
    eprint!("{line}");
}

pub fn info(msg: &str) {
    write("INFO", msg);
}

pub fn warn(msg: &str) {
    write("AVISO", msg);
}

pub fn error(msg: &str) {
    write("ERROR", msg);
}
