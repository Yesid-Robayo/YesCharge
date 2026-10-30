//! YesCharge: batería de tus periféricos inalámbricos en la bandeja de Windows.
//!
//! Un solo exe con dos modos:
//! - servicio (por defecto, o `--background` sin abrir la ventana): icono de bandeja, consultas
//!   y avisos; ocupa muy poca memoria.
//! - `--window`: la ventana, un proceso aparte que solo existe mientras está abierta.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod autostart;
mod icon;
mod ipc;
mod service;
mod tray;
mod ui;

fn main() {
    // Un pánico cierra el proceso (panic = "abort"): que al menos quede en el log para diagnosticarlo.
    std::panic::set_hook(Box::new(|info| yescharge_core::log::error(&format!("fallo inesperado: {info}"))));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--window") {
        ui::run();
    } else if has("--panel") {
        ui::panel::run();
    } else {
        service::run(!has("--background"));
    }
}
