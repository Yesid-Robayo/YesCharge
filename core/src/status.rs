//! Estado publicado por el servicio en `status.json` (lo leen la ventana y la API local).

use serde::{Deserialize, Serialize};

use crate::history::Trend;
use crate::reading::{Charge, DeviceKind, Presence};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Snapshot {
    pub version: String,
    pub updated_at: f64,
    pub next_poll_at: f64,
    /// PID del servicio que escribió el estado (la ventana lo usa para saber si sigue vivo).
    pub service_pid: u32,
    pub devices: Vec<DeviceStatus>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeviceStatus {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub source: String,
    pub presence: Presence,
    pub battery: Option<u8>,
    pub charge: Charge,
    /// Llegó al 100 % y sigue cargando el margen de mantenimiento configurado.
    pub topping_off: bool,
    pub full: bool,
    pub low: bool,
    pub approximate: bool,
    pub trend: Option<Trend>,
    pub eta: String,
    /// Última vez que dio un dato de batería.
    pub last_seen: f64,
    /// Últimas 24 h, reducidas para la gráfica.
    pub history: Vec<(f64, u8)>,
}

impl Snapshot {
    pub fn load() -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(crate::paths::status_file()).ok()?).ok()
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_vec(self) {
            let _ = crate::paths::write_atomic(&crate::paths::status_file(), &j);
        }
    }
}
