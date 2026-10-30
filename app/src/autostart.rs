//! Inicio con Windows: valor `YesCharge` en `HKCU\...\CurrentVersion\Run` (sin permisos de
//! administrador). Apunta al exe actual con `--background`, para que arranque solo en la bandeja.

use std::ptr::{null, null_mut};

use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteKeyValueW, RegSetValueExW,
};

use crate::ipc::wide;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "YesCharge";

pub fn apply(enabled: bool) {
    let key = wide(RUN_KEY);
    let name = wide(VALUE);
    unsafe {
        if !enabled {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let cmd = wide(&format!("\"{}\" --background", exe.display()));
        let mut h: HKEY = null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut h,
            null_mut(),
        ) == 0
        {
            RegSetValueExW(h, name.as_ptr(), 0, REG_SZ, cmd.as_ptr() as *const u8, (cmd.len() * 2) as u32);
            RegCloseKey(h);
        }
    }
}
