//! Asistente "Probar": lista lo conectado y busca una forma de leer la batería del dispositivo
//! que elige el usuario. Estos pasos no envían datos al dispositivo (preguntarle es aparte, ver `ask`,
//! y solo con la aceptación del usuario):
//!
//! 1. Windows (Bluetooth / XInput): solo lee lo que Windows ya sabe.
//! 2. HID estándar: solo GET_REPORT, y solo si el dispositivo declara la batería estándar.
//! 3. Escucha: el usuario crea su propio perfil grabando lo que el dispositivo emite al
//!    conectar y quitar el cable (ver `learn`).

use std::collections::BTreeMap;

use crate::config::Source;
use crate::hid::{self, HidInterface};
use crate::i18n::tr;
use crate::reading::{DeviceKind, Presence, RawReading};
use crate::trf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// USB o dongle 2.4 GHz.
    Usb,
    Bluetooth,
    Xinput,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub transport: Transport,
    pub kind: DeviceKind,
    pub vid_pid: Option<(u16, u16)>,
    pub interfaces: Vec<HidInterface>,
    pub bt_instance: Option<String>,
    pub xinput_slot: Option<u8>,
    pub standard_battery: bool,
    /// Tiene interfaces del fabricante que se pueden escuchar.
    pub listenable: bool,
}

impl Candidate {
    /// Hay al menos una forma segura de intentarlo.
    pub fn readable(&self) -> bool {
        self.transport != Transport::Usb || self.standard_battery || self.listenable
    }
}

fn guess_kind(product: &str, ifaces: &[HidInterface]) -> DeviceKind {
    let p = product.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| p.contains(w));
    if has(&["headset", "headphone", "audio", "auricular", "earbud", "buds"]) {
        return DeviceKind::Headset;
    }
    if has(&["controller", "gamepad", "joystick", "mando", "xbox"]) {
        return DeviceKind::Gamepad;
    }
    if has(&["mouse", "ratón", "raton", "mice"]) {
        return DeviceKind::Mouse;
    }
    if has(&["keyboard", "teclado", "kbd"]) {
        return DeviceKind::Keyboard;
    }
    // Los dongles suelen declarar teclado Y ratón: gana el que tiene más colecciones.
    let count = |u: u16| ifaces.iter().filter(|i| i.usage_page == 0x01 && i.usage == u).count();
    let (mice, keyboards) = (count(0x02), count(0x06));
    if count(0x04) + count(0x05) > 0 {
        DeviceKind::Gamepad
    } else if keyboards > mice {
        DeviceKind::Keyboard
    } else if mice > 0 {
        DeviceKind::Mouse
    } else if keyboards > 0 {
        DeviceKind::Keyboard
    } else {
        DeviceKind::Other
    }
}

/// Identifica a un dispositivo USB o 2.4 GHz en la lista del asistente.
pub fn usb_key(vid: u16, pid: u16) -> String {
    format!("usb:{vid:04x}:{pid:04x}")
}

/// Todo lo conectado que podría tener batería, con lo más prometedor primero.
pub fn candidates() -> Vec<Candidate> {
    let mut out = Vec::new();

    // HID agrupado por VID:PID (un dispositivo físico expone varias interfaces)
    let mut groups: BTreeMap<(u16, u16), Vec<HidInterface>> = BTreeMap::new();
    for i in hid::enumerate() {
        if i.vid != 0 {
            groups.entry((i.vid, i.pid)).or_default().push(i);
        }
    }
    for ((vid, pid), ifaces) in groups {
        let standard = ifaces.iter().any(crate::sources::declares_standard_battery);
        let listenable = ifaces.iter().any(|i| i.is_vendor_defined() && i.input_len > 0);
        let product = ifaces.iter().map(|i| i.product.as_str()).find(|s| !s.is_empty()).unwrap_or("");
        let maker = ifaces.iter().map(|i| i.manufacturer.as_str()).find(|s| !s.is_empty()).unwrap_or("");
        let title = if product.is_empty() {
            trf!("Dispositivo USB {vid:04x}:{pid:04x}", "USB device {vid:04x}:{pid:04x}")
        } else {
            product.to_string()
        };
        let subtitle = if maker.is_empty() {
            format!("USB / 2.4 GHz · {vid:04x}:{pid:04x}")
        } else {
            format!("{maker} · {vid:04x}:{pid:04x}")
        };
        out.push(Candidate {
            key: usb_key(vid, pid),
            kind: guess_kind(product, &ifaces),
            title,
            subtitle,
            transport: Transport::Usb,
            vid_pid: Some((vid, pid)),
            interfaces: ifaces,
            bt_instance: None,
            xinput_slot: None,
            standard_battery: standard,
            listenable,
        });
    }

    for bt in crate::sources::bluetooth_batteries() {
        out.push(Candidate {
            key: format!("bt:{}", bt.instance_id),
            kind: guess_kind(&bt.name, &[]),
            title: bt.name.clone(),
            subtitle: trf!("Bluetooth · Windows informa {} %", "Bluetooth · Windows reports {} %", bt.battery),
            transport: Transport::Bluetooth,
            vid_pid: None,
            interfaces: Vec::new(),
            bt_instance: Some(bt.instance_id),
            xinput_slot: None,
            standard_battery: false,
            listenable: false,
        });
    }

    for (slot, _) in crate::sources::xinput_pads() {
        out.push(Candidate {
            key: format!("xinput:{slot}"),
            kind: DeviceKind::Gamepad,
            title: trf!("Mando XInput {}", "XInput controller {}", slot + 1),
            subtitle: tr(
                "Mando compatible con Xbox · nivel aproximado",
                "Xbox-compatible controller · approximate level",
            )
            .into(),
            transport: Transport::Xinput,
            vid_pid: None,
            interfaces: Vec::new(),
            bt_instance: None,
            xinput_slot: Some(slot),
            standard_battery: false,
            listenable: false,
        });
    }

    // Primero lo que Windows ya lee, luego lo que se puede escuchar, luego el resto.
    out.sort_by_key(|c| {
        let rank = if c.transport != Transport::Usb || c.standard_battery {
            0
        } else if c.listenable {
            1
        } else {
            2
        };
        (rank, c.title.to_lowercase())
    });
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Windows,
    Standard,
    Listen,
}

impl Tier {
    pub fn title(self) -> &'static str {
        match self {
            Tier::Windows => tr("Batería que ya conoce Windows", "Battery Windows already knows"),
            Tier::Standard => tr("Batería HID estándar", "Standard HID battery"),
            Tier::Listen => tr("Crear mi perfil escuchando", "Create my profile by listening"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Running,
    Found,
    NotAvailable,
    /// No hay nada en marcha: le toca al usuario decidir si empieza este paso.
    Waiting,
}

#[derive(Debug, Clone)]
pub struct ProbeStep {
    pub tier: Tier,
    pub state: StepState,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub source: Source,
    pub reading: RawReading,
    pub method: String,
}

/// Ejecuta los niveles automáticos (1 y 2) e informa de cada paso por `report`. Si devuelve
/// `None`, el siguiente paso es la escucha (lo guía la ventana, con el usuario).
pub fn probe(c: &Candidate, report: &mut dyn FnMut(ProbeStep)) -> Option<ProbeResult> {
    let step = |tier, state, detail: &str| ProbeStep { tier, state, detail: detail.into() };

    report(step(Tier::Windows, StepState::Running, tr("Consultando a Windows…", "Asking Windows…")));
    match c.transport {
        Transport::Bluetooth => {
            let id = c.bt_instance.clone().unwrap_or_default();
            let r = crate::sources::read_bluetooth(&id);
            if r.battery.is_some() {
                report(step(
                    Tier::Windows,
                    StepState::Found,
                    tr("Windows informa la batería por Bluetooth.", "Windows reports the battery over Bluetooth."),
                ));
                return Some(ProbeResult {
                    source: Source::Bluetooth { instance_id: id },
                    reading: r,
                    method: "Bluetooth".into(),
                });
            }
            report(step(
                Tier::Windows,
                StepState::NotAvailable,
                tr("Windows no tiene el dato ahora mismo.", "Windows doesn't have that data right now."),
            ));
            return None;
        }
        Transport::Xinput => {
            let slot = c.xinput_slot.unwrap_or(0);
            let r = crate::sources::read_xinput(slot);
            if r.presence == Presence::Online {
                report(step(
                    Tier::Windows,
                    StepState::Found,
                    tr(
                        "Windows informa el nivel del mando (4 niveles).",
                        "Windows reports the controller level (4 levels).",
                    ),
                ));
                return Some(ProbeResult { source: Source::Xinput { slot }, reading: r, method: "XInput".into() });
            }
            report(step(
                Tier::Windows,
                StepState::NotAvailable,
                tr("El mando ya no está conectado.", "The controller is no longer connected."),
            ));
            return None;
        }
        Transport::Usb => report(step(
            Tier::Windows,
            StepState::NotAvailable,
            tr(
                "Es USB o 2.4 GHz: Windows solo conoce la batería de Bluetooth.",
                "It's USB or 2.4 GHz: Windows only knows Bluetooth batteries.",
            ),
        )),
    }

    report(step(
        Tier::Standard,
        StepState::Running,
        tr("Buscando la batería estándar en su descriptor…", "Looking for the standard battery in its descriptor…"),
    ));
    for i in c.interfaces.iter().filter(|i| crate::sources::declares_standard_battery(i)) {
        let r = crate::sources::read_standard_battery(i);
        if r.battery.is_some() {
            report(step(
                Tier::Standard,
                StepState::Found,
                tr(
                    "El dispositivo declara su batería de forma estándar.",
                    "The device declares its battery in the standard way.",
                ),
            ));
            return Some(ProbeResult {
                source: Source::HidStandard { vid: i.vid, pid: i.pid, usage_page: i.usage_page, usage: i.usage },
                reading: r,
                method: tr("HID estándar", "Standard HID").into(),
            });
        }
    }
    report(step(
        Tier::Standard,
        StepState::NotAvailable,
        tr(
            "No declara la batería estándar (lo normal en 2.4 GHz).",
            "It doesn't declare the standard battery (normal on 2.4 GHz).",
        ),
    ));

    if c.listenable {
        report(step(
            Tier::Listen,
            StepState::Waiting,
            tr(
                "Siguiente paso: escuchar lo que envía al conectar el cable.",
                "Next step: listen to what it sends when you plug the cable.",
            ),
        ));
    } else {
        report(step(
            Tier::Listen,
            StepState::NotAvailable,
            tr("No tiene interfaces del fabricante que escuchar.", "It has no manufacturer interfaces to listen to."),
        ));
    }
    None
}
