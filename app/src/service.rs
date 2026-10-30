//! Servicio residente: icono de bandeja + hilo de consultas. No carga nada gráfico pesado (la
//! ventana es otro proceso que se abre solo cuando el usuario la pide), así que ocupa muy poca
//! memoria mientras vive en la bandeja.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::NIN_SELECT;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, TranslateMessage, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use yescharge_core::config::Config;
use yescharge_core::i18n::tr;
use yescharge_core::listen::Hub;
use yescharge_core::monitor::{Monitor, read_source};
use yescharge_core::reading::{Charge, Presence};
use yescharge_core::status::Snapshot;
use yescharge_core::{ask, log, now_secs, trf};

use crate::icon::TrayState;
use crate::ipc::{self, MSG_QUIT, MSG_REFRESH, MSG_RELOAD, MSG_SNAPSHOT, MSG_TRAY, wide};
use crate::tray::{self, Tray};

/// Separación mínima entre ciclos, se pidan como se pidan: cada ciclo lee los dispositivos y escribe
/// al disco, así que pulsar "Actualizar" muchas veces o un dispositivo muy hablador no deben saturar.
const MIN_CYCLE_GAP: Duration = Duration::from_secs(3);

/// Selección del icono con el teclado (NIN_SELECT | NINF_KEY).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

#[derive(Clone, Copy)]
enum Cmd {
    Refresh,
    Reload,
    /// Un dispositivo informó de un cambio: actualizar sin esperar a la próxima consulta.
    Changed,
    Quit,
}

#[derive(Default)]
struct Shared {
    snapshot: Snapshot,
    /// Avisos pendientes de mostrar: (título, texto).
    notices: Vec<(String, String)>,
}

struct Service {
    tray: Tray,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Cmd>,
    taskbar_created: u32,
}

thread_local! {
    static SERVICE: RefCell<Option<Service>> = const { RefCell::new(None) };
}

pub fn run(open_window: bool) {
    unsafe {
        // una sola instancia del servicio por sesión
        let name = wide("Local\\YesCharge.Service");
        let _mutex = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if open_window {
                ipc::spawn_self("--window");
            }
            return;
        }
    }
    log::init("servicio");
    tray::enable_dark_menus();
    let cfg = Config::load();
    crate::autostart::apply(cfg.start_with_windows);

    let hwnd = unsafe { create_window() };
    if hwnd.is_null() {
        log::error("no se pudo crear la ventana del servicio");
        return;
    }
    let api_snapshot = Arc::new(Mutex::new(Snapshot::default()));
    if cfg.api_enabled {
        match crate::api::start(cfg.api_port, api_snapshot.clone()) {
            Ok(()) => log::info(&format!("API local en 127.0.0.1:{}", cfg.api_port)),
            Err(e) => log::warn(&format!("no se pudo abrir la API local en el puerto {}: {e}", cfg.api_port)),
        }
    }
    let shared = Arc::new(Mutex::new(Shared::default()));
    let (tx, rx) = mpsc::channel();
    {
        let (shared, h, notify) = (shared.clone(), hwnd as isize, tx.clone());
        std::thread::Builder::new()
            .name("monitor".into())
            .spawn(move || worker(h, rx, notify, shared, api_snapshot))
            .expect("hilo de consultas");
    }
    let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    let tray = Tray::new(hwnd);
    SERVICE.with(|s| *s.borrow_mut() = Some(Service { tray, shared, tx, taskbar_created }));
    if open_window {
        ipc::spawn_self("--window");
    }

    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    SERVICE.with(|s| {
        if let Some(svc) = s.borrow_mut().take() {
            let _ = svc.tx.send(Cmd::Quit);
        }
    });
    log::info("servicio cerrado");
}

unsafe fn create_window() -> HWND {
    unsafe {
        let class = wide(ipc::SERVICE_CLASS);
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("YesCharge").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        )
    }
}

fn with_service<R>(f: impl FnOnce(&mut Service) -> R) -> Option<R> {
    SERVICE.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        MSG_TRAY => {
            let event = (lparam as u32) & 0xffff;
            match event {
                NIN_SELECT | NIN_KEYSELECT | WM_LBUTTONUP => ipc::spawn_self("--window"),
                WM_CONTEXTMENU | WM_RBUTTONUP => {
                    let lines =
                        with_service(|s| menu_lines(&s.shared.lock().map(|g| g.snapshot.clone()).unwrap_or_default()))
                            .unwrap_or_default();
                    match tray::show_menu(hwnd, &lines) {
                        tray::CMD_OPEN => ipc::spawn_self("--window"),
                        tray::CMD_REFRESH => {
                            with_service(|s| s.tx.send(Cmd::Refresh));
                        }
                        tray::CMD_QUIT => unsafe {
                            DestroyWindow(hwnd);
                        },
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }
        MSG_REFRESH => {
            with_service(|s| s.tx.send(Cmd::Refresh));
            0
        }
        MSG_RELOAD => {
            with_service(|s| s.tx.send(Cmd::Reload));
            crate::autostart::apply(Config::load().start_with_windows);
            0
        }
        MSG_QUIT => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        MSG_SNAPSHOT => {
            with_service(|s| {
                let (snap, notices) = match s.shared.lock() {
                    Ok(mut g) => (g.snapshot.clone(), std::mem::take(&mut g.notices)),
                    Err(_) => return,
                };
                s.tray.update(tray_state(&snap), &tooltip(&snap));
                for (title, text) in notices {
                    s.tray.notify(&title, &text);
                }
            });
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => {
            if with_service(|s| s.taskbar_created == msg && msg != 0).unwrap_or(false) {
                // el Explorador se reinició: volver a poner el icono
                with_service(|s| s.tray.add());
                return 0;
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

fn worker(hwnd: isize, rx: Receiver<Cmd>, notify: Sender<Cmd>, shared: Arc<Mutex<Shared>>, api: Arc<Mutex<Snapshot>>) {
    let mut cfg = Config::load();
    yescharge_core::i18n::set(cfg.language);
    let mut monitor = Monitor::new();
    let mut hub = Hub::new(move || {
        let _ = notify.send(Cmd::Changed);
    });
    let mut force = true;
    log::info(&format!("{} dispositivos configurados", cfg.devices.len()));
    loop {
        let idle = yescharge_core::idle::idle_seconds();
        let poll = Duration::from_secs(cfg.poll_seconds as u64);
        let (snap, events) = monitor.cycle(&cfg, idle, force, now_secs(), &mut |e| {
            // con avisos no hace falta preguntarle tan seguido
            let every = if force || e.source.only_by_asking() { ask::MIN_GAP } else { poll };
            read_source(&e.source, &mut hub, every)
        });
        let listening: Vec<_> = cfg.devices.iter().filter_map(|d| d.source.listens_to()).collect();
        hub.retain(&listening.iter().collect::<Vec<_>>());
        snap.save();
        if let Ok(mut a) = api.lock() {
            *a = snap.clone();
        }
        if let Ok(mut g) = shared.lock() {
            for (name, pct) in &events.low {
                g.notices.push((
                    tr("Batería baja", "Low battery").into(),
                    trf!("{name}: {pct} %. Conéctalo para cargarlo.", "{name}: {pct} %. Plug it in to charge it."),
                ));
            }
            for name in &events.full {
                g.notices.push((
                    tr("Carga completa", "Fully charged").into(),
                    trf!("{name} ya está cargado. Puedes desconectarlo.", "{name} is charged. You can unplug it."),
                ));
            }
            for name in &events.paused {
                g.notices.push((
                    tr("Preguntas en pausa", "Asking paused").into(),
                    trf!(
                        "{name} dio errores al preguntarle y YesCharge dejó de hacerlo por seguridad.",
                        "{name} gave errors when asked and YesCharge stopped asking for safety."
                    ),
                ));
            }
            g.snapshot = snap;
        }
        unsafe { PostMessageW(hwnd as HWND, MSG_SNAPSHOT, 0, 0) };
        let Some(next) = wait_next(&rx, Instant::now(), Duration::from_secs(cfg.cycle_seconds() as u64), MIN_CYCLE_GAP)
        else {
            break;
        };
        force = next.force || next.reload;
        if next.reload {
            cfg = Config::load();
            yescharge_core::i18n::set(cfg.language);
            monitor.forget_removed(&cfg);
            monitor.resume_asking();
            log::info(&format!("configuración recargada: {} dispositivos", cfg.devices.len()));
        }
    }
}

/// Lo que se pidió para el próximo ciclo.
#[derive(Debug, Default, PartialEq)]
struct Pending {
    force: bool,
    reload: bool,
}

impl Pending {
    /// `false` si hay que salir.
    fn add(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Refresh => self.force = true,
            Cmd::Reload => self.reload = true,
            Cmd::Changed => {}
            Cmd::Quit => return false,
        }
        true
    }
}

/// Espera a que toque el siguiente ciclo y agrupa en uno solo todo lo que se pida mientras tanto.
/// `None` si hay que salir. Solo guardar la configuración pasa sin esperar la separación mínima:
/// lo pide el usuario una vez y quiere ver el resultado.
fn wait_next(rx: &Receiver<Cmd>, last_cycle: Instant, poll: Duration, gap: Duration) -> Option<Pending> {
    let mut pending = Pending::default();
    match rx.recv_timeout(poll) {
        Ok(cmd) if pending.add(cmd) => {}
        Ok(_) | Err(RecvTimeoutError::Disconnected) => return None,
        Err(RecvTimeoutError::Timeout) => return Some(pending),
    }
    while !pending.reload {
        let left = (last_cycle + gap).saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(cmd) if pending.add(cmd) => {}
            Ok(_) | Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => break,
        }
    }
    while let Ok(cmd) = rx.try_recv() {
        if !pending.add(cmd) {
            return None;
        }
    }
    Some(pending)
}

fn charge_mark(d: &yescharge_core::status::DeviceStatus) -> &'static str {
    if d.full {
        " ✓"
    } else if d.topping_off || d.charge == Charge::Charging {
        " ⚡"
    } else if d.low {
        " !"
    } else {
        ""
    }
}

fn menu_lines(s: &Snapshot) -> Vec<String> {
    s.devices
        .iter()
        .map(|d| match d.battery {
            Some(b) => format!("{}  —  {b} %{}", d.name, charge_mark(d)),
            None if d.charge == Charge::Charging => trf!("{}  —  cargando", "{}  —  charging", d.name),
            None => format!("{}  —  {}", d.name, d.presence.label().to_lowercase()),
        })
        .collect()
}

fn tooltip(s: &Snapshot) -> String {
    if s.devices.is_empty() {
        return tr("YesCharge · añade un dispositivo", "YesCharge · add a device").into();
    }
    let mut t = String::from("YesCharge");
    for d in &s.devices {
        let line = match d.battery {
            Some(b) => format!("\n{} {b} %{}", d.name, charge_mark(d)),
            None => format!("\n{} —", d.name),
        };
        if t.chars().count() + line.chars().count() > 120 {
            break;
        }
        t.push_str(&line);
    }
    t
}

fn tray_state(s: &Snapshot) -> TrayState {
    let with_data = s.devices.iter().filter(|d| d.battery.is_some() && d.presence != Presence::NotFound);
    let lowest = with_data.clone().min_by_key(|d| d.battery);
    TrayState {
        level: lowest.and_then(|d| d.battery),
        charging: s.devices.iter().any(|d| d.topping_off || d.charge == Charge::Charging),
        low: s.devices.iter().any(|d| d.low),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(60);
    const GAP: Duration = Duration::from_millis(80);

    #[test]
    fn a_burst_of_refreshes_is_a_single_cycle_after_the_gap() {
        let (tx, rx) = mpsc::channel();
        for _ in 0..50 {
            tx.send(Cmd::Refresh).unwrap();
        }
        let start = Instant::now();
        assert_eq!(wait_next(&rx, start, LONG, GAP), Some(Pending { force: true, reload: false }));
        assert!(start.elapsed() >= GAP, "respeta la separación mínima");
        assert!(rx.try_recv().is_err(), "no queda ningún pedido en cola");
    }

    #[test]
    fn device_changes_do_not_force_and_are_grouped() {
        let (tx, rx) = mpsc::channel();
        for _ in 0..50 {
            tx.send(Cmd::Changed).unwrap();
        }
        assert_eq!(wait_next(&rx, Instant::now(), LONG, GAP), Some(Pending::default()));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn saving_the_config_does_not_wait() {
        let (tx, rx) = mpsc::channel();
        tx.send(Cmd::Reload).unwrap();
        let start = Instant::now();
        assert_eq!(wait_next(&rx, start, LONG, LONG), Some(Pending { force: false, reload: true }));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn quit_stops_at_once() {
        let (tx, rx) = mpsc::channel();
        tx.send(Cmd::Refresh).unwrap();
        tx.send(Cmd::Quit).unwrap();
        assert_eq!(wait_next(&rx, Instant::now(), LONG, LONG), None);
    }

    #[test]
    fn the_timer_is_a_normal_cycle() {
        let (_tx, rx) = mpsc::channel::<Cmd>();
        let p = wait_next(&rx, Instant::now(), Duration::from_millis(10), GAP);
        assert_eq!(p, Some(Pending::default()));
    }
}
