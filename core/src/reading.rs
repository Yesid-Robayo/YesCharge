//! Tipos comunes de una lectura de batería.

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

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
            DeviceKind::Headset => tr("Auriculares", "Headset"),
            DeviceKind::Mouse => tr("Ratón", "Mouse"),
            DeviceKind::Keyboard => tr("Teclado", "Keyboard"),
            DeviceKind::Gamepad => tr("Mando", "Controller"),
            DeviceKind::Other => tr("Otro", "Other"),
        }
    }
}

/// Situación del dispositivo en la última consulta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    /// Conectado pero sin responder: dormido, apagado o fuera de alcance. Muchos receptores no
    /// distinguen entre esos casos.
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
            Presence::Online => tr("Conectado", "Connected"),
            Presence::Asleep => tr("Dormido o apagado", "Asleep or off"),
            Presence::Off => tr("Apagado", "Off"),
            Presence::NotFound => tr("No detectado", "Not detected"),
            Presence::Busy => tr("Ocupado", "Busy"),
            Presence::Stale => tr("Último dato", "Last known"),
            Presence::Paused => tr("En pausa por seguridad", "Paused for safety"),
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
    /// El dispositivo no da su porcentaje ahora mismo (p. ej. cargando por cable): lo último que se
    /// supo ya no vale y no se debe mostrar como si fuera el nivel actual.
    pub no_percentage: bool,
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
