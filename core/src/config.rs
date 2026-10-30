//! Configuración en `%APPDATA%\YesCharge\config.json`. La escribe la ventana y la lee el
//! servicio (que la recarga cuando la ventana le avisa).

use serde::{Deserialize, Serialize};

use crate::i18n::{Language, tr};
use crate::reading::{DeviceKind, KnownReading};
use crate::trf;

/// De dónde sale la batería de un dispositivo añadido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// Batería Bluetooth que mantiene Windows.
    Bluetooth { instance_id: String },
    /// Mando XInput en una ranura (0-3).
    Xinput { slot: u8 },
    /// Batería HID estándar declarada por el propio dispositivo.
    HidStandard { vid: u16, pid: u16, usage_page: u16, usage: u16 },
    /// Perfil de escucha creado por el usuario (nunca envía nada).
    Listen { profile: crate::listen::ListenProfile },
    /// Pregunta comprobada del catálogo (`ask`). Es lo único que envía datos: existe solo si el
    /// usuario aceptó el aviso de riesgo, y aquí se guarda solo el id, nunca los datos a enviar.
    Ask {
        query: String,
        accepted_at: u64,
        /// Avisos que aprendió la prueba del asistente: dan el cable y el apagado sin preguntarle.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notices: Option<crate::listen::ListenProfile>,
    },
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Bluetooth { .. } => tr("Bluetooth · batería de Windows", "Bluetooth · battery from Windows").into(),
            Source::Xinput { slot } => trf!("Mando XInput · ranura {}", "XInput controller · slot {}", slot + 1),
            Source::HidStandard { vid, pid, .. } => {
                trf!("HID estándar · {vid:04x}:{pid:04x}", "Standard HID · {vid:04x}:{pid:04x}")
            }
            Source::Listen { profile } => trf!(
                "Mi perfil de escucha · {:04x}:{:04x}",
                "My listening profile · {:04x}:{:04x}",
                profile.vid,
                profile.pid
            ),
            Source::Ask { query, .. } => match crate::ask::find(query) {
                Some(q) => trf!("Le pregunta la batería · {}", "Asks for its battery · {}", q.name),
                None => tr("Le pregunta la batería", "Asks for its battery").into(),
            },
        }
    }

    /// Con qué claves (`probe::Candidate::key`) aparece en el asistente el dispositivo de esta fuente.
    pub fn device_keys(&self) -> Vec<String> {
        use crate::probe::usb_key;
        match self {
            Source::Bluetooth { instance_id } => vec![format!("bt:{instance_id}")],
            Source::Xinput { slot } => vec![format!("xinput:{slot}")],
            Source::HidStandard { vid, pid, .. } => vec![usb_key(*vid, *pid)],
            Source::Listen { profile } => vec![usb_key(profile.vid, profile.pid)],
            Source::Ask { query, .. } => crate::ask::find(query)
                .map(|q| crate::ask::usb_ids(q).map(|(vid, pid)| usb_key(vid, pid)).collect())
                .unwrap_or_default(),
        }
    }

    /// Perfil que hay que mantener escuchando para esta fuente, si lo hay.
    pub fn listens_to(&self) -> Option<crate::listen::ListenProfile> {
        match self {
            Source::Listen { profile } => Some(profile.clone()),
            Source::Ask { query, notices, .. } => {
                let known = crate::ask::find(query).and_then(crate::ask::event_profile);
                match (known, notices) {
                    (Some(mut k), Some(n)) => {
                        k.signals.extend(n.signals.iter().cloned());
                        Some(k)
                    }
                    (k, n) => k.or_else(|| n.clone()),
                }
            }
            _ => None,
        }
    }

    /// No avisa de nada por su cuenta: el cable y el apagado solo se notan al preguntarle.
    pub fn only_by_asking(&self) -> bool {
        matches!(self, Source::Ask { .. }) && self.listens_to().is_none()
    }

    /// Leerla no llega al dispositivo, así que no hay riesgo de despertarlo: se puede leer aunque
    /// el PC esté inactivo. La batería HID estándar (GET_REPORT) y preguntarle sí llegan.
    pub fn is_passive(&self) -> bool {
        !matches!(self, Source::HidStandard { .. } | Source::Ask { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub source: Source,
    /// Avisar cuando baje de este porcentaje.
    pub low_threshold: u8,
    /// Minutos que se sigue cargando tras llegar al 100 % antes de avisar "completa".
    pub top_off_minutes: u32,
    pub enabled: bool,
    /// Lo que vio el asistente al crear un perfil de escucha, para no empezar en blanco.
    pub initial: Option<KnownReading>,
}

impl Default for DeviceEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: DeviceKind::Other,
            source: Source::Xinput { slot: 0 },
            low_threshold: 15,
            top_off_minutes: 30,
            enabled: true,
            initial: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub devices: Vec<DeviceEntry>,
    pub notify_low: bool,
    pub notify_full: bool,
    pub renotify_minutes: u32,
    /// Cada cuánto se consulta la batería.
    pub poll_seconds: u32,
    /// Con el PC inactivo más de esto, no se despierta a los periféricos.
    pub idle_seconds: u32,
    pub start_with_windows: bool,
    /// API HTTP local (solo 127.0.0.1). Desactivada por defecto.
    pub api_enabled: bool,
    pub api_port: u16,
    pub language: Language,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            notify_low: true,
            notify_full: true,
            renotify_minutes: 10,
            poll_seconds: 300,
            idle_seconds: 120,
            start_with_windows: true,
            api_enabled: false,
            api_port: 8765,
            language: Language::Auto,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(crate::paths::config_file()) else {
            return Self::default();
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(c) => c.sanitized(),
            Err(e) => {
                crate::log::warn(&format!("config.json ilegible, se usan los valores por defecto: {e}"));
                Self::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&self.clone().sanitized()).map_err(std::io::Error::other)?;
        crate::paths::write_atomic(&crate::paths::config_file(), &json)
    }

    /// Cada cuánto corre el ciclo de consulta. Un dispositivo que no avisa de nada solo cambia de
    /// estado al preguntarle: sin esto tardaría hasta `poll_seconds` en notarse que lo apagaron, lo
    /// encendieron o lo conectaron.
    pub fn cycle_seconds(&self) -> u32 {
        if self.devices.iter().any(|d| d.enabled && d.source.only_by_asking()) {
            self.poll_seconds.min(crate::ask::MIN_GAP.as_secs() as u32)
        } else {
            self.poll_seconds
        }
    }

    /// Lleva cada valor a un rango sensato (el archivo lo puede editar cualquiera).
    pub fn sanitized(mut self) -> Self {
        self.renotify_minutes = self.renotify_minutes.clamp(1, 240);
        self.poll_seconds = self.poll_seconds.clamp(60, 3600);
        self.idle_seconds = self.idle_seconds.clamp(30, 3600);
        if self.api_port < 1024 {
            self.api_port = 8765;
        }
        self.devices.truncate(32);
        for d in &mut self.devices {
            d.low_threshold = d.low_threshold.clamp(1, 80);
            d.top_off_minutes = d.top_off_minutes.min(240);
            d.name = d.name.chars().filter(|c| !c.is_control()).take(48).collect();
            if d.id.is_empty() {
                d.id = new_id();
            }
        }
        self
    }
}

/// Identificador corto y único para un dispositivo añadido.
pub fn new_id() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}", (t as u64) ^ (std::process::id() as u64).rotate_left(40))
}
