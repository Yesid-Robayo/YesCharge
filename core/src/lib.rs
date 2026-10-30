//! Motor de YesCharge: detección de dispositivos, perfiles de escucha, lógica de avisos e
//! historial. No tiene interfaz gráfica; la usan el servicio de bandeja y la ventana.
//!
//! Por diseño, el motor solo LEE. La única excepción es `ask`: opcional, por dispositivo, con la
//! aceptación expresa del usuario y limitada a las peticiones comprobadas de su catálogo.

pub mod ask;
pub mod bytes;
pub mod config;
pub mod hid;
pub mod history;
pub mod i18n;
pub mod idle;
pub mod learn;
pub mod listen;
pub mod log;
pub mod monitor;
pub mod paths;
pub mod probe;
pub mod reading;
pub mod sniff;
pub mod sources;
pub mod status;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/Yesid-Robayo/YesCharge";

/// Segundos desde la época Unix.
pub fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}
