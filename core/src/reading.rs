//! Tipos comunes de una lectura de batería.

use serde::{Deserialize, Serialize};

/// Estado de carga tal como lo reporta el dispositivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Charge {
    Discharging,
    Charging,
    /// En el cable con la carga completa.
    Full,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Headset,
    Mouse,
    Keyboard,
    Gamepad,
    #[default]
    Other,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 5] =
        [DeviceKind::Headset, DeviceKind::Mouse, DeviceKind::Keyboard, DeviceKind::Gamepad, DeviceKind::Other];

    pub fn label(self) -> &'static str {
        match self {
            DeviceKind::Headset => "Auriculares",
            DeviceKind::Mouse => "Ratón",
            DeviceKind::Keyboard => "Teclado",
            DeviceKind::Gamepad => "Mando",
            DeviceKind::Other => "Otro",
        }
    }
}

/// Situación del dispositivo en la última consulta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    /// Conectado pero sin responder (dormido o fuera de alcance).
    Asleep,
    /// Responde que está apagado.
    Off,
    #[default]
    NotFound,
    /// Otro programa lo tiene abierto en exclusiva.
    Busy,
    /// Perfil de escucha sin mensajes recientes: se muestra lo último que informó. No significa
    /// que esté dormido, porque muchos dispositivos solo envían su estado cuando cambia.
    Stale,
    /// Se dejó de preguntarle tras varios errores seguidos.
    Paused,
    Error,
}

impl Presence {
    pub fn label(self) -> &'static str {
        match self {
            Presence::Online => "Conectado",
            Presence::Asleep => "Dormido",
            Presence::Off => "Apagado",
            Presence::NotFound => "No detectado",
            Presence::Busy => "Ocupado",
            Presence::Stale => "Último dato",
            Presence::Paused => "En pausa por seguridad",
            Presence::Error => "Error",
        }
    }
}

/// Último dato conocido de un dispositivo. Se conserva entre reinicios, porque un perfil de
/// escucha puede pasar horas sin recibir un mensaje nuevo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownReading {
    pub battery: Option<u8>,
    pub charge: Charge,
    /// Época Unix, en segundos, en que se recibió.
    pub at: u64,
}

/// Lectura cruda de una fuente, antes del suavizado y la lógica de avisos.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawReading {
    pub presence: Presence,
    pub battery: Option<u8>,
    pub charge: Charge,
    /// El porcentaje es una aproximación por niveles (p. ej. XInput: vacío/bajo/medio/lleno).
    pub approximate: bool,
    /// Detalle para el log (nunca se muestra con datos personales).
    pub detail: String,
}

impl RawReading {
    pub fn absent(presence: Presence, detail: impl Into<String>) -> Self {
        Self { presence, detail: detail.into(), ..Default::default() }
    }
}

/// Normaliza la carga: al 100 % conectado se considera completa aunque el dispositivo siga
/// diciendo "cargando".
pub fn normalize(battery: Option<u8>, charge: Charge) -> Charge {
    match (battery, charge) {
        (Some(b), Charge::Charging) if b >= 100 => Charge::Full,
        _ => charge,
    }
}
