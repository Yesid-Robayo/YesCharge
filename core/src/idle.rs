//! Segundos desde la última entrada del usuario (teclado o ratón).

use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

pub fn idle_seconds() -> f64 {
    unsafe {
        let mut li = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut li) == 0 {
            return 0.0;
        }
        GetTickCount().wrapping_sub(li.dwTime) as f64 / 1000.0
    }
}
