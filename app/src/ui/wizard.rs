//! Asistente "Añadir dispositivo": elegir → probar (Windows, HID estándar) → si hace falta,
//! crear el perfil propio escuchando → confirmar con el usuario → guardar.
//!
//! Nada de esto envía datos al dispositivo, salvo que el usuario acepte el aviso de riesgo para
//! preguntarle a un modelo del catálogo (ver `yescharge_core::ask`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2};
use yescharge_core::ask::{self, Query};
use yescharge_core::config::{Config, DeviceEntry, Source, new_id};
use yescharge_core::hid::HidInterface;
use yescharge_core::i18n::{Language, tr};
use yescharge_core::learn::{BatteryOption, Capture, FindingKind, Listener};
use yescharge_core::probe::{self, Candidate, ProbeResult, ProbeStep, StepState, Tier, Transport};
use yescharge_core::reading::{Charge, DeviceKind, KnownReading, Presence, RawReading};
use yescharge_core::status::Snapshot;
use yescharge_core::trf;

use super::theme::{self, *};
use super::{Nav, widgets};
use crate::ipc;

enum ProbeMsg {
    Step(ProbeStep),
    Done(Option<ProbeResult>),
}

/// Lo que se espera a que el servicio lea por primera vez el dispositivo nuevo.
const ADDING_MAX: Duration = Duration::from_secs(12);
/// Tiempo mínimo del cartel "Añadiendo", para que se alcance a leer.
const ADDING_MIN: Duration = Duration::from_millis(700);

/// Segundos mínimos de cada fase de la escucha.
const PHASE_SECS: [u64; 5] = [8, 12, 12, 15, 15];
/// Segundos que se deja al usuario para hacer lo que pide la fase antes de preguntarle. Con 0, en
/// esa fase no se pregunta.
const ASK_SETTLE_SECS: [u64; 5] = [0, 8, 0, 8, 12];

/// Prueba de un dispositivo al que se le pregunta: como no siempre avisa por su cuenta, también se
/// mira qué responde con el cable puesto, apagado y encendido otra vez. Respeta `ask::MIN_GAP`.
struct AskTest {
    query: &'static Query,
    /// Respuesta a la primera pregunta.
    first: RawReading,
    /// Pregunta en curso de la fase actual.
    pending: Option<Receiver<RawReading>>,
    /// Respuesta de cada fase en la que se pregunta.
    answers: [Option<RawReading>; 5],
    /// Si el usuario sale de la prueba, la pregunta que estaba esperando ya no se envía.
    cancelled: Arc<AtomicBool>,
    /// Qué aparato físico se está probando.
    unit: Option<String>,
}

impl Drop for AskTest {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl AskTest {
    fn begin_phase(&mut self, phase: u8) {
        let settle = ASK_SETTLE_SECS[phase as usize];
        if settle > 0 {
            self.pending = Some(ask_later(self.query, self.unit.clone(), settle, self.cancelled.clone()));
        }
    }

    fn poll(&mut self, phase: u8) {
        if let Some(Ok(reading)) = self.pending.as_ref().map(|rx| rx.try_recv()) {
            self.answers[phase as usize] = Some(reading);
            self.pending = None;
        }
    }

    fn charging_seen(&self) -> bool {
        self.answers[1].as_ref().is_some_and(|r| matches!(r.charge, Charge::Charging | Charge::Full))
    }

    fn off_seen(&self) -> bool {
        self.answers[3].as_ref().is_some_and(|r| r.presence != Presence::Online)
            && self.answers[4].as_ref().is_some_and(|r| r.presence == Presence::Online)
    }
}

#[allow(clippy::large_enum_variant)]
enum Stage {
    Scanning(Receiver<Vec<Candidate>>),
    List {
        items: Vec<Candidate>,
        show_all: bool,
    },
    Probing {
        cand: Candidate,
        steps: Vec<ProbeStep>,
        rx: Receiver<ProbeMsg>,
    },
    NotFound {
        cand: Candidate,
        steps: Vec<ProbeStep>,
    },
    Listening {
        cand: Candidate,
        listener: Option<Listener>,
        ifaces: Vec<HidInterface>,
        phase: u8,
        since: Instant,
        ask: Option<AskTest>,
    },
    Review {
        cand: Candidate,
        capture: Capture,
        ifaces: Vec<HidInterface>,
        options: Vec<BatteryOption>,
        use_charge: bool,
        use_power: bool,
        battery: Option<usize>,
        saved: Option<String>,
        /// Porcentaje de la app oficial que escribe el usuario, para el informe.
        official: String,
    },
    /// Aviso de riesgo antes de enviarle nada al dispositivo.
    Consent {
        cand: Candidate,
        query: &'static Query,
        accepted: bool,
    },
    Asking {
        cand: Candidate,
        query: &'static Query,
        rx: Receiver<RawReading>,
    },
    AskFailed {
        cand: Candidate,
        query: &'static Query,
        text: &'static str,
    },
    /// Qué notó la prueba de un dispositivo al que se le pregunta.
    AskResult {
        cand: Candidate,
        result: Option<ProbeResult>,
        charging: bool,
        power: bool,
    },
    Confirm(Box<Confirm>),
    /// Guardado: espera a que el servicio lo lea, para que al volver ya aparezca.
    Adding {
        name: String,
        rx: Receiver<()>,
    },
}

struct Confirm {
    source: Source,
    reading: RawReading,
    method: String,
    name: String,
    kind: DeviceKind,
    threshold: u8,
    /// Para volver a la revisión si el usuario dice que no coincide (perfil de escucha).
    back: Option<Box<Stage>>,
    mismatch: bool,
}

pub struct Wizard {
    stage: Stage,
    /// Se calcula una sola vez: `show` corre en cada fotograma y no debe tocar el disco.
    has_devices: bool,
}

fn scan() -> Receiver<Vec<Candidate>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // lo que ya está añadido no se vuelve a ofrecer, pero sí otro aparato igual
        let added = Config::load().devices;
        let mut items = probe::candidates();
        items.retain(|c| {
            !added.iter().any(|d| {
                let unit = d.source.container();
                d.source.device_keys().contains(&c.key)
                    && (unit.is_none() || c.container.is_none() || unit == c.container.as_deref())
            })
        });
        let _ = tx.send(items);
    });
    rx
}

fn start_adding(id: String, name: String) -> Stage {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        std::thread::sleep(ADDING_MIN);
        let start = Instant::now();
        while start.elapsed() < ADDING_MAX && !Snapshot::load().is_some_and(|s| s.devices.iter().any(|d| d.id == id)) {
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = tx.send(());
    });
    Stage::Adding { name, rx }
}

impl Wizard {
    pub fn new() -> Self {
        Self { stage: Stage::Scanning(scan()), has_devices: !Config::load().devices.is_empty() }
    }

    /// Solo en desarrollo: una etapa con datos de ejemplo (`probe`, `listen`, `review`, `confirm`, `none`).
    #[cfg(debug_assertions)]
    pub fn demo(which: &str) -> Self {
        use yescharge_core::learn::Finding;
        let cand = Candidate {
            key: "usb:1234:5678".into(),
            title: "Auriculares de ejemplo".into(),
            subtitle: "Ejemplo · 1234:5678".into(),
            transport: Transport::Usb,
            kind: DeviceKind::Headset,
            vid_pid: Some((0x1234, 0x5678)),
            container: None,
            interfaces: Vec::new(),
            bt_instance: None,
            xinput_slot: None,
            standard_battery: false,
            listenable: true,
        };
        let steps = vec![
            ProbeStep {
                tier: Tier::Windows,
                state: StepState::NotAvailable,
                detail: "Es USB o 2.4 GHz: Windows solo conoce la batería de Bluetooth.".into(),
            },
            ProbeStep {
                tier: Tier::Standard,
                state: StepState::Running,
                detail: "Buscando la batería estándar en su descriptor…".into(),
            },
        ];
        let finding = |kind, offset, values: [Vec<u8>; 3], last, score| Finding {
            tag: None,
            interface: 0,
            report_id: 0,
            len: 8,
            offset,
            kind,
            xor: 0,
            values,
            last,
            last_t: 0,
            seen_in: 2,
            score,
        };
        let capture = Capture {
            app_version: String::new(),
            vid: "1234".into(),
            pid: "5678".into(),
            product: String::new(),
            manufacturer: String::new(),
            interfaces: Vec::new(),
            all_interfaces: Vec::new(),
            official_percent: None,
            samples: Vec::new(),
            findings: vec![
                finding(FindingKind::Charge, 5, [vec![0], vec![1], vec![0]], 0, 3),
                finding(FindingKind::Battery, 4, [vec![76], vec![76, 77], vec![77]], 77, 1),
                finding(FindingKind::Battery, 7, [vec![12], vec![12], vec![12]], 12, 1),
            ],
        };
        let iface = HidInterface {
            path: String::new(),
            vid: 0x1234,
            pid: 0x5678,
            version: 0,
            usage_page: 0xffc0,
            usage: 1,
            input_len: 65,
            output_len: 0,
            feature_len: 0,
            product: String::new(),
            manufacturer: String::new(),
        };
        let stage = match which {
            "probe" => {
                let (_tx, rx) = mpsc::channel();
                std::mem::forget(_tx);
                Stage::Probing { cand, steps, rx }
            }
            "none" => {
                let mut steps = steps;
                steps[1].state = StepState::NotAvailable;
                steps[1].detail = "No declara la batería estándar (lo normal en 2.4 GHz).".into();
                steps.push(ProbeStep {
                    tier: Tier::Listen,
                    state: StepState::Waiting,
                    detail: "Siguiente paso: escuchar lo que envía al conectar el cable.".into(),
                });
                Stage::NotFound { cand, steps }
            }
            "consent" => {
                Stage::Consent { cand, query: ask::find("mchose-l7-ultra-plus").expect("catálogo"), accepted: false }
            }
            "listen" => Stage::Listening {
                cand,
                listener: Some(Listener::start(&[])),
                ifaces: Vec::new(),
                phase: 1,
                since: Instant::now(),
                ask: None,
            },
            "review" => {
                let options = yescharge_core::learn::battery_options(&capture.findings);
                Stage::Review {
                    cand,
                    capture,
                    ifaces: vec![iface],
                    options,
                    use_charge: true,
                    use_power: false,
                    battery: None,
                    saved: None,
                    official: String::new(),
                }
            }
            "empty" => {
                let mut c = capture;
                c.findings.clear();
                Stage::Review {
                    cand,
                    capture: c,
                    ifaces: Vec::new(),
                    options: Vec::new(),
                    use_charge: false,
                    use_power: false,
                    battery: None,
                    saved: None,
                    official: String::new(),
                }
            }
            _ => confirm(
                cand,
                ProbeResult {
                    source: Source::Xinput { slot: 0 },
                    reading: RawReading {
                        presence: Presence::Online,
                        battery: Some(77),
                        charge: Charge::Charging,
                        approximate: false,
                        no_percentage: false,
                        heard_only: false,
                        detail: String::new(),
                    },
                    method: tr("Tu perfil de escucha", "Your listening profile").into(),
                },
                None,
            ),
        };
        Self { stage, has_devices: true }
    }

    /// Hay trabajo en segundo plano o animación: la ventana repinta en cada fotograma.
    pub fn busy(&self) -> bool {
        matches!(
            self.stage,
            Stage::Scanning(_)
                | Stage::Probing { .. }
                | Stage::Listening { .. }
                | Stage::Asking { .. }
                | Stage::Adding { .. }
        )
    }

    pub fn show(&mut self, ui: &mut egui::Ui, logo: &TextureHandle) -> Nav {
        let title = match &self.stage {
            Stage::Scanning(_) | Stage::List { .. } | Stage::Adding { .. } => tr("Añadir dispositivo", "Add device"),
            Stage::Probing { .. } | Stage::NotFound { .. } => tr("Probando", "Testing"),
            Stage::Listening { ask: Some(_), .. } | Stage::AskResult { .. } => {
                tr("Probar el dispositivo", "Testing the device")
            }
            Stage::Listening { .. } | Stage::Review { .. } => tr("Crear mi perfil", "Create my profile"),
            Stage::Consent { .. } | Stage::Asking { .. } | Stage::AskFailed { .. } => {
                tr("Preguntarle la batería", "Ask for its battery")
            }
            Stage::Confirm(_) => tr("Confirmar", "Confirm"),
        };
        let back = if self.has_devices || !matches!(self.stage, Stage::Scanning(_) | Stage::List { .. }) {
            super::page_header(ui, title)
        } else {
            super::header(
                ui,
                logo,
                tr("Bienvenido a YesCharge", "Welcome to YesCharge"),
                tr("Empecemos por tu primer dispositivo", "Let's start with your first device"),
            );
            ui.add_space(8.0);
            false
        };
        if back {
            return match self.stage {
                Stage::Scanning(_) | Stage::List { .. } => Nav::Home,
                _ => {
                    self.stage = Stage::Scanning(scan());
                    Nav::Stay
                }
            };
        }
        let mut next: Option<Stage> = None;
        let mut nav = Nav::Stay;
        match &mut self.stage {
            Stage::Scanning(rx) => {
                if let Ok(items) = rx.try_recv() {
                    next = Some(Stage::List { items, show_all: false });
                }
                spinner_block(ui, tr("Buscando dispositivos conectados…", "Looking for connected devices…"));
            }
            Stage::List { items, show_all } => match list(ui, items, show_all, self.has_devices) {
                ListOut::Pick(c) => next = Some(start_probe(c)),
                ListOut::Rescan => next = Some(Stage::Scanning(scan())),
                ListOut::Stay => {}
            },
            Stage::Probing { cand, steps, rx } => {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        ProbeMsg::Step(s) => match steps.iter_mut().find(|x| x.tier == s.tier) {
                            Some(x) => *x = s,
                            None => steps.push(s),
                        },
                        ProbeMsg::Done(Some(r)) => next = Some(confirm(cand.clone(), r, None)),
                        ProbeMsg::Done(None) => {
                            next = Some(Stage::NotFound { cand: cand.clone(), steps: steps.clone() })
                        }
                    }
                }
                cand_header(ui, cand);
                steps_view(ui, steps);
            }
            Stage::NotFound { cand, steps } => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    cand_header(ui, cand);
                    steps_view(ui, steps);
                    ui.add_space(6.0);
                    if let Some(query) = query_for(cand) {
                        widgets::card(ui, |ui| {
                            ui.label(egui::RichText::new(tr("¿Quieres ver el porcentaje?", "Want to see the percentage?")).font(theme::semibold(15.5)));
                            widgets::muted(
                                ui,
                                tr("Este modelo solo lo dice si se le pregunta. YesCharge sabe cómo hacerlo, pero implica un riesgo.", "This model only tells it when asked. YesCharge knows how, but it involves a risk."),
                            );
                            ui.add_space(4.0);
                            if widgets::danger_button(ui, tr("Preguntarle la batería…", "Ask for its battery…"), ui.available_width()).clicked() {
                                next = Some(Stage::Consent { cand: cand.clone(), query, accepted: false });
                            }
                        });
                    }
                    if cand.listenable {
                        widgets::card(ui, |ui| {
                            ui.label(egui::RichText::new(tr("Crea tu propio perfil", "Create your own profile")).font(theme::semibold(15.5)));
                            widgets::muted(
                                ui,
                                tr(
                                    "YesCharge escuchará lo que tu dispositivo envía mientras conectas y quitas el cable de carga. \
                                     Solo escucha: lo abre sin permiso de escritura, así que no puede enviarle nada.",
                                    "YesCharge will listen to what your device sends while you plug and unplug the charging cable. \
                                     It only listens: it's opened without write permission, so nothing can be sent to it.",
                                ),
                            );
                            ui.label(
                                egui::RichText::new(turn_on_hint(cand.kind)).font(theme::semibold(13.5)).color(WARN),
                            );
                            ui.add_space(4.0);
                            let label = tr("Ya está encendido: empezar a escuchar", "It's already on: start listening");
                            if widgets::primary_button(ui, label, ui.available_width(), true).clicked() {
                                next = Some(start_listening(cand.clone()));
                            }
                        });
                    } else if query_for(cand).is_none() {
                        support_card(ui, cand, None);
                    }
                    if widgets::secondary_button(ui, tr("Elegir otro dispositivo", "Choose another device"), ui.available_width()).clicked() {
                        next = Some(Stage::Scanning(scan()));
                    }
                });
            }
            Stage::Consent { cand, query, accepted } => match consent(ui, cand, query, accepted) {
                ConsentOut::Accept => next = Some(start_asking(cand.clone(), query)),
                ConsentOut::Decline => next = Some(Stage::Scanning(scan())),
                ConsentOut::Stay => {}
            },
            Stage::Asking { cand, query, rx } => {
                if let Ok(reading) = rx.try_recv() {
                    next = Some(asked(cand.clone(), query, reading));
                }
                cand_header(ui, cand);
                spinner_block(ui, tr("Preguntándole…", "Asking…"));
            }
            Stage::AskFailed { cand, query, text } => {
                cand_header(ui, cand);
                widgets::card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(tr("No se pudo leer", "Could not read it")).font(theme::semibold(15.5)),
                    );
                    widgets::muted(ui, *text);
                });
                let wait = ask::wait_secs(query, cand.container.as_deref());
                let label = if wait > 0 {
                    trf!("Reintentar en {wait} s", "Retry in {wait} s")
                } else {
                    tr("Reintentar", "Retry").to_string()
                };
                if widgets::primary_button(ui, &label, ui.available_width(), wait == 0).clicked() {
                    next = Some(start_asking(cand.clone(), query));
                }
                if widgets::secondary_button(
                    ui,
                    tr("Elegir otro dispositivo", "Choose another device"),
                    ui.available_width(),
                )
                .clicked()
                {
                    next = Some(Stage::Scanning(scan()));
                }
            }
            Stage::AskResult { cand, result, charging, power } => {
                let percent = result.as_ref().is_some_and(|r| r.reading.battery.is_some());
                if ask_result(ui, cand, percent, *charging, *power)
                    && let Some(r) = result.take()
                {
                    next = Some(confirm(cand.clone(), r, None));
                }
            }
            Stage::Listening { cand, listener, ifaces, phase, since, ask } => {
                if listening(ui, cand, listener, phase, since, ask) {
                    let l = listener.take();
                    let (vid, pid) = cand.vid_pid.unwrap_or((0, 0));
                    if let Some(l) = l {
                        let capture = l.finish(vid, pid);
                        yescharge_core::log::info(&format!(
                            "escucha de {vid:04x}:{pid:04x}: {} muestras, {} hallazgos",
                            capture.samples.len(),
                            capture.findings.len()
                        ));
                        // el informe se guarda siempre: sirve para pedir soporte sin repetir la escucha
                        let saved = match yescharge_core::learn::save(&capture) {
                            Ok(path) => Some(path.display().to_string()),
                            Err(e) => {
                                yescharge_core::log::error(&format!("no se pudo guardar el informe: {e}"));
                                None
                            }
                        };
                        next = Some(match ask.take() {
                            Some(test) => ask_finished(cand.clone(), test, &capture, ifaces),
                            None => {
                                let options = yescharge_core::learn::battery_options(&capture.findings);
                                let use_charge = capture.findings.iter().any(|f| f.kind == FindingKind::Charge);
                                let use_power = capture.findings.iter().any(|f| f.kind == FindingKind::Power);
                                Stage::Review {
                                    cand: cand.clone(),
                                    capture,
                                    ifaces: ifaces.clone(),
                                    options,
                                    use_charge,
                                    use_power,
                                    // nunca preseleccionada: un valor candidato puede no ser la batería
                                    battery: None,
                                    saved,
                                    official: String::new(),
                                }
                            }
                        });
                    }
                }
            }
            Stage::Review { cand, capture, ifaces, options, use_charge, use_power, battery, saved, official } => {
                if let Some(stage) =
                    review(ui, cand, capture, ifaces, options, use_charge, use_power, battery, saved, official)
                {
                    next = Some(stage);
                }
            }
            Stage::Confirm(c) => match confirm_view(ui, c) {
                ConfirmOut::Saved(id) => next = Some(start_adding(id, c.name.trim().to_string())),
                ConfirmOut::Back => {
                    if let Some(b) = c.back.take() {
                        next = Some(*b);
                    } else {
                        c.mismatch = true;
                    }
                }
                ConfirmOut::Stay => {}
            },
            Stage::Adding { name, rx } => {
                if rx.try_recv().is_ok() {
                    nav = Nav::Home;
                }
                spinner_block(ui, &trf!("Añadiendo {name}…", "Adding {name}…"));
            }
        }
        if let Some(s) = next {
            self.stage = s;
        }
        nav
    }
}

pub fn spinner_block(ui: &mut egui::Ui, text: &str) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new().size(36.0).color(BRAND_A));
        ui.add_space(10.0);
        widgets::muted(ui, text);
    });
}

fn badge_for(c: &Candidate) -> (&'static str, egui::Color32) {
    match c.transport {
        Transport::Bluetooth => ("Bluetooth · Windows", GOOD),
        Transport::Xinput => (tr("Mando · Windows", "Controller · Windows"), GOOD),
        Transport::Usb if c.standard_battery => (tr("Batería estándar", "Standard battery"), GOOD),
        Transport::Usb if c.listenable => (tr("Se puede escuchar", "Can be listened to"), CHARGE),
        Transport::Usb => (tr("Sin forma segura", "No safe way"), FAINT),
    }
}

enum ListOut {
    Stay,
    Pick(Candidate),
    Rescan,
}

fn list(ui: &mut egui::Ui, items: &[Candidate], show_all: &mut bool, has_devices: bool) -> ListOut {
    let mut out = ListOut::Stay;
    if !has_devices {
        language_choice(ui);
    }
    widgets::muted(
        ui,
        tr(
            "Elige el dispositivo cuya batería quieres ver. Si es inalámbrico, enciéndelo y úsalo un momento para que esté despierto.",
            "Pick the device whose battery you want to see. If it's wireless, turn it on and use it for a moment so it's awake.",
        ),
    );
    widgets::muted(
        ui,
        tr(
            "Los dispositivos con receptor USB aparecen aunque estén apagados, porque Windows ve el receptor.",
            "Devices with a USB receiver show up even when they're off, because Windows sees the receiver.",
        ),
    );
    if has_devices {
        widgets::muted(
            ui,
            tr("Los que ya añadiste no aparecen aquí.", "Devices you already added don't show up here."),
        );
    }
    ui.add_space(2.0);
    let hidden = items.iter().filter(|c| !c.readable()).count();
    egui::ScrollArea::vertical().max_height(ui.available_height() - 60.0).auto_shrink([false, false]).show(ui, |ui| {
        for c in items.iter().filter(|c| *show_all || c.readable()) {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            let p = ui.painter();
            p.rect(
                rect,
                CornerRadius::same(14),
                theme::lerp(SURFACE, SURFACE_HI, hover),
                Stroke::new(1.0, theme::lerp(BORDER, theme::alpha(BRAND_A, 0.5), hover)),
                StrokeKind::Inside,
            );
            let icon = Rect::from_center_size(pos2(rect.left() + 34.0, rect.center().y), vec2(40.0, 40.0));
            p.circle_filled(icon.center(), 20.0, SURFACE_2);
            widgets::device_glyph(p, icon.shrink(10.0), c.kind, TEXT);
            let x = rect.left() + 66.0;
            let (badge, col) = badge_for(c);
            let g = p.layout_no_wrap(badge.to_string(), theme::semibold(11.5), col);
            let badge_rect = Rect::from_min_size(
                pos2(rect.right() - 14.0 - g.size().x - 14.0, rect.center().y - 11.0),
                g.size() + vec2(14.0, 6.0),
            );
            let title_w = (badge_rect.left() - x - 10.0).max(60.0);
            let title = p.layout(c.title.clone(), theme::semibold(14.5), TEXT, title_w);
            let title_h = title.size().y.min(20.0);
            p.galley(pos2(x, rect.center().y - title_h - 1.0), title, TEXT);
            p.text(
                pos2(x, rect.center().y + 10.0),
                Align2::LEFT_CENTER,
                &c.subtitle,
                egui::FontId::proportional(12.0),
                MUTED,
            );
            p.rect_filled(badge_rect, CornerRadius::same(255), theme::alpha(col, 0.14));
            p.galley(badge_rect.center() - g.size() / 2.0, g, col);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                out = ListOut::Pick(c.clone());
            }
        }
        if hidden > 0 {
            ui.add_space(4.0);
            let label = if *show_all {
                tr("Ocultar los que no se pueden leer", "Hide those that can't be read").to_string()
            } else {
                trf!(
                    "Mostrar {hidden} más que no se pueden leer de forma segura",
                    "Show {hidden} more that can't be read safely"
                )
            };
            if ui.link(label).clicked() {
                *show_all = !*show_all;
            }
        }
    });
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        if widgets::secondary_button(ui, tr("Volver a buscar", "Search again"), ui.available_width()).clicked() {
            out = ListOut::Rescan;
        }
    });
    out
}

/// La primera vez no hay otra forma de llegar a los ajustes.
fn language_choice(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (language, label) in [(Language::Spanish, "Español"), (Language::English, "English")] {
            let current = yescharge_core::i18n::is_english() == (language == Language::English);
            if widgets::choice_chip(ui, label, current).clicked() {
                let mut cfg = Config::load();
                cfg.language = language;
                super::save_config(&cfg);
            }
        }
    });
}

fn start_probe(cand: Candidate) -> Stage {
    let (tx, rx) = mpsc::channel();
    let c = cand.clone();
    std::thread::spawn(move || {
        let mut report = |s: ProbeStep| {
            let _ = tx.send(ProbeMsg::Step(s));
            // pausa breve para que el usuario vea cada paso
            std::thread::sleep(std::time::Duration::from_millis(350));
        };
        let r = probe::probe(&c, &mut report);
        let _ = tx.send(ProbeMsg::Done(r));
    });
    Stage::Probing { cand, steps: Vec::new(), rx }
}

fn cand_header(ui: &mut egui::Ui, c: &Candidate) {
    widgets::card(ui, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 22.0, SURFACE_2);
            widgets::device_glyph(ui.painter(), rect.shrink(11.0), c.kind, TEXT);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(&c.title).font(theme::semibold(15.5)));
                widgets::muted(ui, &c.subtitle);
            });
        });
    });
}

fn steps_view(ui: &mut egui::Ui, steps: &[ProbeStep]) {
    widgets::card(ui, |ui| {
        for tier in [Tier::Windows, Tier::Standard, Tier::Listen] {
            let s = steps.iter().find(|s| s.tier == tier);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                let p = ui.painter();
                match s.map(|s| s.state) {
                    Some(StepState::Running) => {
                        let t = ui.input(|i| i.time) as f32;
                        p.circle_stroke(rect.center(), 9.0, Stroke::new(2.0, SURFACE_2));
                        let a = t * 5.0;
                        p.add(egui::Shape::line(
                            (0..=12)
                                .map(|i| {
                                    let ang = a + i as f32 * 0.13;
                                    pos2(rect.center().x + 9.0 * ang.cos(), rect.center().y + 9.0 * ang.sin())
                                })
                                .collect(),
                            Stroke::new(2.0, BRAND_A),
                        ));
                    }
                    Some(StepState::Found) => {
                        p.circle_filled(rect.center(), 11.0, theme::alpha(GOOD, 0.2));
                        widgets::glyph(p, rect.shrink(5.0), widgets::Glyph::Check, GOOD);
                    }
                    Some(StepState::Waiting) => {
                        p.circle_stroke(rect.center(), 9.0, Stroke::new(2.0, BRAND_A));
                        p.circle_filled(rect.center(), 3.5, BRAND_A);
                    }
                    Some(StepState::NotAvailable) => {
                        p.circle_filled(rect.center(), 11.0, SURFACE_2);
                        p.line_segment(
                            [
                                pos2(rect.center().x - 4.0, rect.center().y),
                                pos2(rect.center().x + 4.0, rect.center().y),
                            ],
                            Stroke::new(2.0, FAINT),
                        );
                    }
                    None => {
                        p.circle_stroke(rect.center(), 9.0, Stroke::new(1.5, BORDER));
                    }
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let col = if s.is_some() { TEXT } else { FAINT };
                    ui.label(egui::RichText::new(tier.title()).font(theme::semibold(14.0)).color(col));
                    if let Some(s) = s {
                        widgets::muted(ui, &s.detail);
                    }
                });
            });
        }
    });
}

/// YesCharge no puede saber si un dispositivo con receptor USB está encendido: hay que pedirlo.
fn turn_on_hint(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Headset => tr(
            "Enciende los auriculares antes de empezar: apagados no envían nada.",
            "Turn the headset on before starting: it sends nothing while off.",
        ),
        DeviceKind::Mouse => tr(
            "Enciende el ratón antes de empezar: apagado no envía nada.",
            "Turn the mouse on before starting: it sends nothing while off.",
        ),
        DeviceKind::Keyboard => tr(
            "Enciende el teclado antes de empezar: apagado no envía nada.",
            "Turn the keyboard on before starting: it sends nothing while off.",
        ),
        DeviceKind::Gamepad => tr(
            "Enciende el mando antes de empezar: apagado no envía nada.",
            "Turn the controller on before starting: it sends nothing while off.",
        ),
        DeviceKind::Other => tr(
            "Enciende el dispositivo antes de empezar: apagado no envía nada.",
            "Turn the device on before starting: it sends nothing while off.",
        ),
    }
}

/// Instrucción del primer paso de la escucha.
fn first_step_text(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Headset => tr(
            "Enciende los auriculares y déjalos SIN el cable de carga. Póntelos o reproduce algo.",
            "Turn the headset on and leave it WITHOUT the charging cable. Put it on or play something.",
        ),
        DeviceKind::Mouse => tr(
            "Enciende el ratón y déjalo SIN el cable de carga. Muévelo un poco.",
            "Turn the mouse on and leave it WITHOUT the charging cable. Move it a little.",
        ),
        DeviceKind::Keyboard => tr(
            "Enciende el teclado y déjalo SIN el cable de carga. Pulsa alguna tecla.",
            "Turn the keyboard on and leave it WITHOUT the charging cable. Press a key.",
        ),
        DeviceKind::Gamepad => tr(
            "Enciende el mando y déjalo SIN el cable de carga. Pulsa algún botón.",
            "Turn the controller on and leave it WITHOUT the charging cable. Press a button.",
        ),
        DeviceKind::Other => tr(
            "Enciende el dispositivo y déjalo SIN el cable de carga. Úsalo un poco.",
            "Turn the device on and leave it WITHOUT the charging cable. Use it a little.",
        ),
    }
}

fn query_for(c: &Candidate) -> Option<&'static Query> {
    c.vid_pid.and_then(|(vid, pid)| ask::for_device(vid, pid))
}

enum ConsentOut {
    Stay,
    Accept,
    Decline,
}

/// Aviso de riesgo: nada se envía al dispositivo hasta que el usuario lo acepte aquí.
fn consent(ui: &mut egui::Ui, cand: &Candidate, query: &Query, accepted: &mut bool) -> ConsentOut {
    let mut out = ConsentOut::Stay;
    let bullet = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(format!("•  {text}")).size(13.0).color(MUTED));
    };
    egui::ScrollArea::vertical().max_height(ui.available_height() - 110.0).auto_shrink([false, false]).show(ui, |ui| {
        cand_header(ui, cand);
        egui::Frame::new()
            .fill(theme::alpha(LOW, 0.08))
            .stroke(Stroke::new(1.0, theme::alpha(LOW, 0.5)))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(tr("Esto le envía datos a tu dispositivo", "This sends data to your device")).font(theme::semibold(15.5)).color(LOW));
                ui.label(
                    egui::RichText::new(trf!(
                        "El porcentaje de {} solo se obtiene preguntándoselo. Si aceptas, YesCharge le enviará \
                         periódicamente una petición de estado, comprobada en una unidad de este modelo.",
                        "The percentage of {} can only be obtained by asking. If you accept, YesCharge will \
                         periodically send it a status request, checked on one unit of this model.",
                        query.name
                    ))
                    .size(13.0),
                );
            });
        widgets::section_title(ui, tr("Cómo lo hacemos lo más seguro posible", "How we make it as safe as possible"));
        widgets::card(ui, |ui| {
            bullet(ui, tr("Solo se envía esa petición exacta, que viene dentro de la app y no se puede cambiar.", "Only that exact request is sent. It comes inside the app and can't be changed."));
            bullet(ui, tr("Nunca se prueban comandos al azar ni se cambia ningún ajuste.", "Commands are never tried at random and no setting is changed."));
            bullet(ui, tr("Como mucho una vez por minuto, y nunca con el PC inactivo.", "At most once a minute, and never while the PC is idle."));
            bullet(ui, tr("Si da errores seguidos, deja de preguntar sola.", "If it gives repeated errors, it stops asking on its own."));
            bullet(ui, tr("Es solo para este dispositivo: al quitarlo de YesCharge, deja de preguntarle.", "It's only for this device: when you remove it from YesCharge, it stops asking."));
        });
        widgets::section_title(ui, tr("El riesgo", "The risk"));
        widgets::card(ui, |ui| {
            ui.label(
                egui::RichText::new(tr(
                    "Ningún envío a un dispositivo es 100 % seguro. Con otro firmware podría comportarse distinto, y \
                     no hay manera de garantizar lo contrario. La app se ofrece sin garantía: lo usas bajo tu responsabilidad.",
                    "Nothing sent to a device is 100 % safe. A different firmware could behave differently, and \
                     there's no way to guarantee otherwise. The app comes with no warranty: you use it at your own risk.",
                ))
                .size(13.0)
                .color(MUTED),
            );
            ui.add_space(4.0);
            ui.checkbox(accepted, tr("Entiendo el riesgo y quiero que le pregunte", "I understand the risk and want it to ask"));
        });
    });
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        if widgets::secondary_button(ui, tr("No, mejor no", "No, better not"), ui.available_width()).clicked() {
            out = ConsentOut::Decline;
        }
        if *accepted {
            if widgets::danger_button(ui, tr("Aceptar y preguntarle", "Accept and ask"), ui.available_width()).clicked()
            {
                out = ConsentOut::Accept;
            }
        } else {
            widgets::primary_button(
                ui,
                tr("Marca la casilla para continuar", "Tick the box to continue"),
                ui.available_width(),
                false,
            );
        }
    });
    out
}

fn start_asking(cand: Candidate, query: &'static Query) -> Stage {
    yescharge_core::log::info(&format!("el usuario aceptó preguntarle a {}", query.id));
    let (tx, rx) = mpsc::channel();
    let unit = cand.container.clone();
    std::thread::spawn(move || {
        // si se le acaba de preguntar, espera su turno en vez de darlo por fallido
        std::thread::sleep(std::time::Duration::from_secs(ask::wait_secs(query, unit.as_deref())));
        let _ = tx.send(ask::read(query.id, unit.as_deref(), Some(ask::MIN_GAP)));
    });
    Stage::Asking { cand, query, rx }
}

/// Pregunta cuando el usuario ya tuvo tiempo de hacer lo que pide la fase y le toca el turno.
fn ask_later(
    query: &'static Query,
    unit: Option<String>,
    settle: u64,
    cancelled: Arc<AtomicBool>,
) -> Receiver<RawReading> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(settle));
        std::thread::sleep(Duration::from_secs(ask::wait_secs(query, unit.as_deref())));
        if !cancelled.load(Ordering::Relaxed) {
            let _ = tx.send(ask::read(query.id, unit.as_deref(), Some(ask::MIN_GAP)));
        }
    });
    rx
}

/// Une lo que respondió en cada fase con lo que envió por su cuenta.
fn ask_finished(cand: Candidate, test: AskTest, capture: &Capture, ifaces: &[HidInterface]) -> Stage {
    let best = |kind| capture.findings.iter().find(|f| f.kind == kind);
    let (charge, power) = (best(FindingKind::Charge), best(FindingKind::Power));
    let notices = yescharge_core::learn::build_profile(ifaces, charge, &[], power);
    let charging = test.charging_seen() || charge.is_some();
    let off = test.off_seen() || power.is_some();
    yescharge_core::log::info(&format!(
        "prueba de {}: carga {charging}, apagado {off}, avisos {}",
        test.query.id,
        notices.is_some()
    ));
    let source = Source::Ask {
        query: test.query.id.into(),
        accepted_at: yescharge_core::now_secs() as u64,
        notices,
        container: test.unit.clone(),
    };
    let reading = test.first.clone();
    let result = ProbeResult { source, reading, method: tr("Preguntándoselo", "Asking it").into() };
    Stage::AskResult { cand, result: Some(result), charging, power: off }
}

/// Resultado de la prueba. `true` cuando el usuario quiere seguir.
fn ask_result(ui: &mut egui::Ui, cand: &Candidate, percent: bool, charging: bool, power: bool) -> bool {
    cand_header(ui, cand);
    widgets::section_title(ui, tr("Lo que notamos", "What we noticed"));
    widgets::card(ui, |ui| {
        let line = |ui: &mut egui::Ui, ok: bool, yes: &str, no: &str| {
            let (text, color) = if ok { (yes, GOOD) } else { (no, WARN) };
            ui.label(egui::RichText::new(text).font(theme::semibold(14.0)).color(color));
        };
        line(
            ui,
            percent,
            tr("Lee el porcentaje", "It reads the percentage"),
            tr("Con el cable puesto no da el porcentaje", "It gives no percentage while on the cable"),
        );
        line(
            ui,
            charging,
            tr("Nota cuándo está cargando", "It notices when it's charging"),
            tr("No nota cuándo está cargando", "It doesn't notice when it's charging"),
        );
        line(
            ui,
            power,
            tr("Nota cuándo lo apagas y lo enciendes", "It notices when you turn it off and on"),
            tr("No nota cuándo lo apagas", "It doesn't notice when you turn it off"),
        );
    });
    if !charging || !power {
        widgets::muted(
            ui,
            tr(
                "Lo que no notó no aparecerá en la app. Lo demás funciona igual.",
                "What it didn't notice won't appear in the app. Everything else works the same.",
            ),
        );
    }
    widgets::primary_button(ui, tr("Continuar", "Continue"), ui.available_width(), true).clicked()
}

/// Decide qué sigue según la respuesta a la primera pregunta.
fn asked(cand: Candidate, query: &'static Query, reading: RawReading) -> Stage {
    let answered = reading.battery.is_some() || matches!(reading.charge, Charge::Charging | Charge::Full);
    let text = match reading.presence {
        Presence::Online if answered => {
            let test = AskTest {
                query,
                first: reading,
                pending: None,
                answers: Default::default(),
                cancelled: Arc::new(AtomicBool::new(false)),
                unit: cand.container.clone(),
            };
            return start_test(cand, Some(test));
        }
        Presence::Off => {
            tr("Dice que está apagado. Enciéndelo y reinténtalo.", "It says it's off. Turn it on and retry.")
        }
        Presence::Busy => tr(
            "Otro programa lo tiene ocupado. Ciérralo y reinténtalo.",
            "Another program is using it. Close it and retry.",
        ),
        Presence::NotFound => tr(
            "Ya no aparece conectado. Comprueba el receptor USB y reinténtalo.",
            "It no longer shows as connected. Check the USB receiver and retry.",
        ),
        Presence::Error => tr(
            "Dio un error al preguntarle. Puedes reintentarlo; si se repite, mira yescharge.log.",
            "Asking it gave an error. You can retry; if it repeats, check yescharge.log.",
        ),
        _ => tr(
            "No respondió. Enciéndelo o úsalo un momento para despertarlo y reinténtalo.",
            "It didn't answer. Turn it on or use it for a moment to wake it up, and retry.",
        ),
    };
    yescharge_core::log::warn(&format!("pregunta a {}: {}", query.id, reading.detail));
    Stage::AskFailed { cand, query, text }
}

fn start_listening(cand: Candidate) -> Stage {
    start_test(cand, None)
}

fn start_test(cand: Candidate, ask: Option<AskTest>) -> Stage {
    let listener = Listener::start(&cand.interfaces);
    let ifaces = listener.interfaces().to_vec();
    Stage::Listening { cand, listener: Some(listener), ifaces, phase: 0, since: Instant::now(), ask }
}

/// `true` cuando el usuario terminó las tres fases.
fn listening(
    ui: &mut egui::Ui,
    cand: &Candidate,
    listener: &mut Option<Listener>,
    phase: &mut u8,
    since: &mut Instant,
    ask: &mut Option<AskTest>,
) -> bool {
    let Some(l) = listener.as_ref() else { return true };
    if let Some(test) = ask {
        test.poll(*phase);
    }
    let (title, text) = match phase {
        0 => (
            tr("Paso 1 de 5 · Enciéndelo, sin cable", "Step 1 of 5 · Turn it on, no cable"),
            first_step_text(cand.kind),
        ),
        1 => (
            tr("Paso 2 de 5 · Conecta el cable", "Step 2 of 5 · Plug in the cable"),
            tr(
                "Conecta ahora el cable de carga y espera a que cuente el tiempo.",
                "Plug in the charging cable now and wait for the time to run.",
            ),
        ),
        2 => (
            tr("Paso 3 de 5 · Quita el cable", "Step 3 of 5 · Unplug the cable"),
            tr("Desconecta el cable otra vez y espera un momento.", "Unplug the cable again and wait a moment."),
        ),
        3 => (
            tr("Paso 4 de 5 · Apágalo", "Step 4 of 5 · Turn it off"),
            tr(
                "Apágalo y espera: esto sigue solo, no hace falta pulsar nada.",
                "Turn it off and wait: this continues on its own, no need to press anything.",
            ),
        ),
        _ => (
            tr("Paso 5 de 5 · Enciéndelo", "Step 5 of 5 · Turn it on"),
            tr(
                "Enciéndelo otra vez y espera: esto también sigue solo.",
                "Turn it on again and wait: this also continues on its own.",
            ),
        ),
    };
    cand_header(ui, cand);
    let elapsed = since.elapsed().as_secs();
    let need = PHASE_SECS[*phase as usize];
    let waiting_answer = ask.as_ref().is_some_and(|t| t.pending.is_some());
    let ready = elapsed >= need && !waiting_answer;
    widgets::card(ui, |ui| {
        ui.vertical_centered(|ui| {
            // ondas animadas: "escuchando"
            let (rect, _) = ui.allocate_exact_size(vec2(140.0, 140.0), Sense::hover());
            let t = ui.input(|i| i.time) as f32;
            let p = ui.painter();
            for k in 0..3 {
                let f = ((t * 0.6 + k as f32 / 3.0) % 1.0).clamp(0.0, 1.0);
                p.circle_stroke(
                    rect.center(),
                    22.0 + f * 46.0,
                    Stroke::new(2.0, theme::alpha(BRAND_A, (1.0 - f) * 0.6)),
                );
            }
            p.circle_filled(rect.center(), 24.0, SURFACE_2);
            widgets::device_glyph(p, Rect::from_center_size(rect.center(), vec2(26.0, 26.0)), cand.kind, BRAND_A);
            ui.label(egui::RichText::new(title).font(theme::semibold(16.0)));
            ui.label(egui::RichText::new(text).size(13.0).color(MUTED));
            ui.add_space(4.0);
            if let Some(test) = ask.as_ref() {
                let msg = match (&test.pending, &test.answers[*phase as usize]) {
                    (Some(_), _) => match ask::wait_secs(test.query, test.unit.as_deref()) {
                        0 => tr("Preguntándole…", "Asking…").to_string(),
                        s => trf!("Le preguntará en {s} s", "It will ask in {s} s"),
                    },
                    (None, Some(r)) if r.presence == Presence::Online => tr("Respondió", "It answered").to_string(),
                    (None, Some(_)) => tr("No respondió", "It didn't answer").to_string(),
                    (None, None) => tr("Listo para seguir", "Ready to continue").to_string(),
                };
                widgets::chip(ui, &msg, MUTED);
            } else {
                let n = l.sample_count();
                let msg = match n {
                    0 if *phase < 3 => {
                        tr("Sin mensajes todavía: comprueba que esté encendido", "No messages yet: check that it's on")
                            .to_string()
                    }
                    0 => tr("Sin mensajes todavía", "No messages yet").to_string(),
                    1 => tr("1 mensaje recibido", "1 message received").into(),
                    _ => trf!("{n} mensajes recibidos", "{n} messages received"),
                };
                widgets::chip(ui, &msg, if n > 0 { GOOD } else { MUTED });
            }
        });
        ui.add_space(6.0);
        // barra de progreso de la fase
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
        ui.painter().rect_filled(bar, CornerRadius::same(3), SURFACE_2);
        let f = if waiting_answer { 0.98 } else { (since.elapsed().as_secs_f32() / need as f32).clamp(0.0, 1.0) };
        ui.painter().rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())),
            CornerRadius::same(3),
            BRAND_A,
        );
    });
    widgets::muted(
        ui,
        if ask.is_some() {
            tr(
                "Escucha lo que envía y le hace su pregunta de batería, como mucho una vez por minuto.",
                "It listens to what it sends and asks for its battery, at most once a minute.",
            )
        } else {
            tr(
                "Solo escuchando: el dispositivo se abrió sin permiso de escritura y no se le envía nada.",
                "Only listening: the device was opened without write permission and nothing is sent to it.",
            )
        },
    );
    // Con el ratón o el teclado apagados no se puede pulsar nada: estos pasos avanzan solos.
    let automatic = *phase >= yescharge_core::learn::PHASE_OFF;
    let label = match (automatic, ready) {
        (true, _) if waiting_answer => {
            tr("Continúa solo cuando responda…", "Continues on its own when it answers…").into()
        }
        (false, false) if waiting_answer => tr("Esperando su respuesta…", "Waiting for its answer…").into(),
        (true, _) => trf!("Continúa solo en {} s…", "Continues on its own in {} s…", need.saturating_sub(elapsed)),
        (false, false) => trf!("Espera {} s…", "Wait {} s…", need - elapsed),
        (false, true) => tr("Siguiente", "Next").into(),
    };
    let clicked = widgets::primary_button(ui, &label, ui.available_width(), ready && !automatic).clicked();
    if clicked || (automatic && ready) {
        if *phase >= yescharge_core::learn::PHASE_ON {
            return true;
        }
        *phase += 1;
        l.set_phase(*phase);
        *since = Instant::now();
        if let Some(test) = ask {
            test.begin_phase(*phase);
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn review(
    ui: &mut egui::Ui,
    cand: &Candidate,
    capture: &Capture,
    ifaces: &[HidInterface],
    options: &[BatteryOption],
    use_charge: &mut bool,
    use_power: &mut bool,
    battery: &mut Option<usize>,
    saved: &mut Option<String>,
    official: &mut String,
) -> Option<Stage> {
    let mut next = None;
    cand_header(ui, cand);
    if capture.findings.is_empty() {
        widgets::card(ui, |ui| {
            let title = if capture.samples.is_empty() {
                tr("Tu dispositivo no envió nada por su cuenta", "Your device sent nothing on its own")
            } else {
                tr("No encontramos la batería en lo que envió", "We didn't find the battery in what it sent")
            };
            ui.label(egui::RichText::new(title).font(theme::semibold(15.5)));
            widgets::muted(
                ui,
                tr(
                    "Si estaba apagado, enciéndelo y repite la escucha. Si estaba encendido, es probable que solo diga su \
                     batería cuando su app oficial se la pregunta: con esos modelos no basta con escuchar.",
                    "If it was off, turn it on and listen again. If it was on, it probably only tells its battery when \
                     its official app asks: listening isn't enough for those models.",
                ),
            );
        });
        support_card(ui, cand, Some((capture, saved, official)));
        if widgets::secondary_button(ui, tr("Repetir la escucha", "Listen again"), ui.available_width()).clicked() {
            next = Some(start_listening(cand.clone()));
        }
        return next;
    }
    // los hallazgos vienen ordenados: el primero es el mejor candidato
    let best_charge = capture.findings.iter().find(|f| f.kind == FindingKind::Charge);
    let best_power = capture.findings.iter().find(|f| f.kind == FindingKind::Power);
    egui::ScrollArea::vertical().max_height(ui.available_height() - 110.0).auto_shrink([false, false]).show(ui, |ui| {
        widgets::section_title(ui, tr("Carga", "Charging"));
        widgets::card(ui, |ui| {
            if best_charge.is_some() {
                widgets::setting_row(
                    ui,
                    tr("Mostrar cuándo está cargando", "Show when it's charging"),
                    tr("Notamos cuándo conectas y quitas el cable.", "We notice when you plug and unplug the cable."),
                    |ui| {
                        widgets::toggle(ui, use_charge);
                    },
                );
            } else {
                widgets::muted(
                    ui,
                    tr(
                        "No notamos ningún cambio al conectar el cable, así que no podremos mostrar cuándo carga.",
                        "We saw no change when plugging the cable, so we can't show when it's charging.",
                    ),
                );
            }
        });
        widgets::section_title(ui, tr("Encendido", "Power"));
        widgets::card(ui, |ui| {
            if best_power.is_some() {
                widgets::setting_row(
                    ui,
                    tr("Mostrar cuándo está apagado", "Show when it's off"),
                    tr("Notamos cuándo lo apagas y lo enciendes.", "We notice when you turn it off and on."),
                    |ui| {
                        widgets::toggle(ui, use_power);
                    },
                );
            } else {
                widgets::muted(
                    ui,
                    tr(
                        "No avisó al apagarlo y encenderlo, así que no podremos saber cuándo está apagado.",
                        "It gave no notice when turned off and on, so we can't know when it's off.",
                    ),
                );
            }
        });
        widgets::section_title(ui, tr("Batería", "Battery"));
        widgets::card(ui, |ui| {
            if options.is_empty() {
                widgets::muted(
                    ui,
                    tr("No vimos ningún porcentaje en lo que envió.", "We saw no percentage in what it sent."),
                );
                return;
            }
            ui.label(
                egui::RichText::new(tr("¿Cuánta batería tiene ahora?", "How much battery does it have now?"))
                    .font(theme::semibold(14.5)),
            );
            widgets::muted(
                ui,
                tr(
                    "Elige el valor que coincida con tu dispositivo o con su app oficial.",
                    "Pick the value that matches your device or its official app.",
                ),
            );
            for (i, o) in options.iter().enumerate() {
                if ui.radio(*battery == Some(i), format!("{} %", o.value)).clicked() {
                    *battery = Some(i);
                }
            }
            if ui.radio(battery.is_none(), tr("Ninguno coincide / no lo sé", "None match / I don't know")).clicked() {
                *battery = None;
            }
        });
        support_card(ui, cand, Some((capture, saved, official)));
        if let Some(query) = query_for(cand) {
            widgets::card(ui, |ui| {
                widgets::muted(
                    ui,
                    tr(
                        "¿No aparece tu porcentaje? Este modelo se puede preguntar, aceptando un riesgo.",
                        "Your percentage isn't listed? This model can be asked, accepting a risk.",
                    ),
                );
                if widgets::danger_button(
                    ui,
                    tr("Preguntarle la batería…", "Ask for its battery…"),
                    ui.available_width(),
                )
                .clicked()
                {
                    next = Some(Stage::Consent { cand: cand.clone(), query, accepted: false });
                }
            });
        }
    });
    let c = best_charge.filter(|_| *use_charge);
    let pw = best_power.filter(|_| *use_power);
    let option = battery.and_then(|i| options.get(i));
    let batteries: Vec<_> =
        option.map(|o| o.members.iter().filter_map(|&i| capture.findings.get(i)).collect()).unwrap_or_default();
    let profile = yescharge_core::learn::build_profile(ifaces, c, &batteries, pw)
        .map(|p| yescharge_core::listen::ListenProfile { container: cand.container.clone(), ..p });
    if widgets::primary_button(ui, tr("Crear mi perfil", "Create my profile"), ui.available_width(), profile.is_some())
        .clicked()
        && let Some(profile) = profile
    {
        let reading = RawReading {
            presence: Presence::Online,
            battery: option.map(|o| o.value),
            charge: match c {
                Some(f) if f.values[1].contains(&f.last) => Charge::Charging,
                Some(_) => Charge::Discharging,
                None => Charge::Unknown,
            },
            approximate: false,
            no_percentage: false,
            heard_only: false,
            detail: "perfil de escucha".into(),
        };
        let back = Stage::Review {
            cand: cand.clone(),
            capture: capture.clone(),
            ifaces: ifaces.to_vec(),
            options: options.to_vec(),
            use_charge: *use_charge,
            use_power: *use_power,
            battery: *battery,
            saved: saved.clone(),
            official: official.clone(),
        };
        next = Some(confirm(
            cand.clone(),
            ProbeResult {
                source: Source::Listen { profile },
                reading,
                method: tr("Tu perfil de escucha", "Your listening profile").into(),
            },
            Some(Box::new(back)),
        ));
    }
    if widgets::secondary_button(ui, tr("Repetir la escucha", "Listen again"), ui.available_width()).clicked() {
        next = Some(start_listening(cand.clone()));
    }
    next
}

/// Tarjeta para enviarnos el dispositivo: con el informe se puede añadir su modelo a la lista.
fn support_card(ui: &mut egui::Ui, cand: &Candidate, capture: Option<(&Capture, &mut Option<String>, &mut String)>) {
    widgets::card(ui, |ui| {
        ui.label(egui::RichText::new(tr("Envíanos tu dispositivo", "Send us your device")).font(theme::semibold(15.0)));
        widgets::muted(
            ui,
            tr(
                "Abre un issue en GitHub y adjunta el informe: con él podemos intentar añadir tu modelo. No incluye datos personales.",
                "Open a GitHub issue and attach the report: with it we can try to add your model. It has no personal data.",
            ),
        );
        let mut report = None;
        if let Some((cap, saved, official)) = capture {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr(
                        "Porcentaje en su app oficial (opcional)",
                        "Percentage in its official app (optional)",
                    ))
                    .size(13.0),
                );
                ui.add(egui::TextEdit::singleline(official).desired_width(52.0).char_limit(3));
            });
            report = Some((cap, saved, official.trim().parse::<u8>().ok().filter(|p| *p <= 100)));
        }
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 10.0) / 2.0;
            match report {
                Some((cap, saved, official)) => {
                    if widgets::secondary_button(ui, tr("Ver informe", "View report"), w).clicked() {
                        // se guarda de nuevo por si el usuario escribió el porcentaje
                        let mut full = cap.clone();
                        full.official_percent = official;
                        match yescharge_core::learn::save(&full) {
                            Ok(path) => *saved = Some(path.display().to_string()),
                            Err(e) => yescharge_core::log::error(&format!("no se pudo guardar el informe: {e}")),
                        }
                        ipc::open_external(&yescharge_core::paths::captures_dir().display().to_string());
                    }
                }
                None => {
                    ui.add_space(w + 10.0);
                }
            }
            if widgets::secondary_button(ui, tr("Abrir issue", "Open issue"), w).clicked() {
                let title = trf!("Dispositivo nuevo: {} ({})", "New device: {} ({})", cand.title, cand.subtitle);
                let url =
                    format!("{}/issues/new?template=device.yml&title={}", yescharge_core::REPO_URL, encode(&title));
                ipc::open_external(&url);
            }
        });
    });
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn confirm(cand: Candidate, r: ProbeResult, back: Option<Box<Stage>>) -> Stage {
    let name = cand.title.clone();
    Stage::Confirm(Box::new(Confirm {
        kind: cand.kind,
        source: r.source,
        reading: r.reading,
        method: r.method,
        name,
        threshold: 15,
        back,
        mismatch: false,
    }))
}

enum ConfirmOut {
    Stay,
    /// Id del dispositivo añadido.
    Saved(String),
    Back,
}

fn confirm_view(ui: &mut egui::Ui, c: &mut Confirm) -> ConfirmOut {
    let mut out = ConfirmOut::Stay;
    let has_battery = c.reading.battery.is_some();
    ui.vertical_centered(|ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(140.0, 140.0), Sense::hover());
        let level = c.reading.battery.map(|b| ui.ctx().animate_value_with_time(resp.id, b as f32 / 100.0, 0.8));
        let charging = matches!(c.reading.charge, Charge::Charging | Charge::Full);
        let color = if charging { CHARGE } else { level_color(c.reading.battery.unwrap_or(50), false) };
        let label = c.reading.battery.map(|b| format!("{b}%")).unwrap_or_else(|| "—".into());
        let sub = match c.reading.charge {
            Charge::Charging => Some(tr("cargando", "charging")),
            Charge::Full => Some(tr("cargado", "charged")),
            _ if c.reading.presence == Presence::Off => Some(tr("apagado", "off")),
            _ => None,
        };
        widgets::ring(ui, rect, level, color, &label, sub);
        ui.add_space(6.0);
        let title =
            if has_battery { tr("¡Lo encontramos!", "Found it!") } else { tr("Sin porcentaje", "No percentage") };
        ui.label(egui::RichText::new(title).font(theme::bold(19.0)));
        widgets::muted(
            ui,
            trf!(
                "Leído con: {}{}",
                "Read with: {}{}",
                c.method,
                if c.reading.approximate { tr(" (aproximado)", " (approximate)") } else { "" }
            ),
        );
    });
    ui.add_space(4.0);
    widgets::card(ui, |ui| {
        let question = if has_battery {
            tr(
                "¿Coincide con lo que ves en el dispositivo o en su app oficial?",
                "Does it match what you see on the device or in its official app?",
            )
        } else {
            tr(
                "Este dispositivo no envía su batería: solo verás lo que sí detectamos, como la carga.",
                "This device doesn't send its battery: you'll only see what we do detect, like charging.",
            )
        };
        ui.label(egui::RichText::new(question).font(theme::semibold(14.5)));
        if c.mismatch {
            ui.label(
                egui::RichText::new(tr("Gracias por avisar. Con este método no se puede leer bien; puedes pedir soporte abriendo un issue en GitHub.", "Thanks for letting us know. This method can't read it properly; you can ask for support by opening a GitHub issue."))
                    .size(12.5)
                    .color(WARN),
            );
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tr("Nombre", "Name")).font(theme::semibold(13.5)));
        ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(f32::INFINITY).char_limit(48));
        ui.label(egui::RichText::new(tr("Tipo", "Type")).font(theme::semibold(13.5)));
        ui.horizontal_wrapped(|ui| {
            for k in DeviceKind::ALL {
                if widgets::choice_chip(ui, k.label(), c.kind == k).clicked() {
                    c.kind = k;
                }
            }
        });
        let desc = trf!("Cuando baje del {} %", "When it drops below {} %", c.threshold);
        widgets::slider_row(
            ui,
            tr("Avisar con batería baja", "Alert on low battery"),
            &desc,
            &mut c.threshold,
            5..=50,
            1.0,
        );
    });
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        let back = if has_battery { tr("No coincide", "It doesn't match") } else { tr("Volver", "Back") };
        if widgets::secondary_button(ui, back, ui.available_width()).clicked() {
            out = ConfirmOut::Back;
        }
        let add = if has_battery { tr("Sí, añadir", "Yes, add it") } else { tr("Añadir así", "Add it like this") };
        if widgets::primary_button(ui, add, ui.available_width(), !c.name.trim().is_empty()).clicked() {
            let mut cfg = Config::load();
            let id = new_id();
            cfg.devices.push(DeviceEntry {
                id: id.clone(),
                name: c.name.trim().to_string(),
                kind: c.kind,
                source: c.source.clone(),
                low_threshold: c.threshold,
                top_off_minutes: 30,
                enabled: true,
                // el servicio empieza a escuchar de cero: que arranque con lo que ya se vio aquí
                muted: false,
                initial: matches!(c.source, Source::Listen { .. }).then(|| KnownReading {
                    battery: c.reading.battery,
                    charge: c.reading.charge,
                    at: yescharge_core::now_secs() as u64,
                }),
            });
            if super::save_config(&cfg) {
                yescharge_core::log::info(&format!("dispositivo añadido ({})", c.source.label()));
                out = ConfirmOut::Saved(id);
            }
        }
    });
    out
}
