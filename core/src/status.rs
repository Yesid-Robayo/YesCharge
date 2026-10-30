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
    /// Últimos 7 días, reducidos para la gráfica.
    pub history_week: Vec<(f64, u8)>,
}

impl DeviceStatus {
    /// Lugar en la lista automática: primero lo que está en uso, al final lo que no aparece.
    fn rank(&self) -> u8 {
        match self.presence {
            Presence::Online => 0,
            Presence::Stale => 1,
            Presence::Busy | Presence::Paused | Presence::Error => 2,
            Presence::Asleep => 3,
            Presence::Off => 4,
            Presence::NotFound => 5,
        }
    }
}

/// Lista para mostrar: en el orden guardado (`ids`) y, si `auto`, con lo encendido primero.
pub fn ordered(devices: &[DeviceStatus], ids: &[String], auto: bool) -> Vec<DeviceStatus> {
    let mut list = devices.to_vec();
    list.sort_by_key(|d| ids.iter().position(|id| *id == d.id).unwrap_or(usize::MAX));
    if auto {
        list.sort_by_key(DeviceStatus::rank);
    }
    list
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

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, presence: Presence) -> DeviceStatus {
        DeviceStatus { id: id.into(), presence, ..Default::default() }
    }

    #[test]
    fn automatic_order_puts_what_is_on_first_and_keeps_the_saved_order_inside_each_group() {
        let list = [
            device("off", Presence::Off),
            device("b", Presence::Online),
            device("asleep", Presence::Asleep),
            device("a", Presence::Online),
        ];
        let ids: Vec<String> = ["a", "off", "asleep", "b"].map(String::from).to_vec();
        let order = |auto| ordered(&list, &ids, auto).into_iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!(order(true), ["a", "b", "asleep", "off"]);
        assert_eq!(order(false), ["a", "off", "asleep", "b"]);
    }
}
