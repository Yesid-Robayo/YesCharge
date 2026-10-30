//! Núcleo de monitoreo (port de la lógica de BatteryInfo): suavizado por mediana, margen de
//! carga tras el 100 %, avisos con repetición, tendencia, autonomía aprendida y la regla de no
//! consultar a los periféricos mientras el PC está inactivo.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::{Config, DeviceEntry, Source};
use crate::history::{History, format_eta};
use crate::i18n::tr;
use crate::listen::HeardState;
use crate::reading::{Charge, KnownReading, Presence, RawReading};
use crate::status::{DeviceStatus, Snapshot};

const SMOOTH_WINDOW: usize = 5;
/// Los dispositivos apagados o no detectados se reintentan con menos frecuencia.
const ABSENT_RETRY_S: f64 = 600.0;
/// Errores seguidos al preguntar antes de dejar de hacerlo.
const ASK_TRIP_AFTER: u32 = 3;
/// Holgura al comparar con el intervalo de consulta: un ciclo nunca cae justo en el segundo exacto.
const POLL_MARGIN_S: f64 = 5.0;

/// Lo que el servicio debe anunciar tras un ciclo.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Events {
    pub low: Vec<(String, u8)>,
    pub full: Vec<String>,
    /// Dispositivos a los que se dejó de preguntar por errores repetidos.
    pub paused: Vec<String>,
}

#[derive(Default)]
struct DeviceState {
    window: Vec<u8>,
    last: Option<DeviceStatus>,
    last_probe: f64,
    full_since: f64,
    last_low_notify: f64,
    full_notified: bool,
    /// Último dato real, aunque hayan pasado horas (se conserva entre reinicios).
    known: Option<KnownReading>,
    /// Errores seguidos al preguntarle.
    ask_failures: u32,
    ask_paused: bool,
}

/// Lo que sobrevive a un reinicio de la app, por dispositivo (`state.json`).
#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    full_since: f64,
    #[serde(default)]
    known: Option<KnownReading>,
}

pub struct Monitor {
    states: HashMap<String, DeviceState>,
    history: History,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Lee una fuente configurada. Ninguna envía datos al dispositivo: Windows, la batería HID
/// estándar (GET_REPORT) o la escucha de lo que el propio dispositivo emite.
pub fn read_source(source: &Source, hub: &mut crate::listen::Hub, ask_every: std::time::Duration) -> RawReading {
    match source {
        Source::Listen { profile } => hub.read(profile),
        Source::Ask { query, .. } => {
            let profile = source.listens_to();
            let heard = profile.as_ref().and_then(|p| hub.state(p));
            let asked = crate::ask::read(query, ask_every);
            // Responder demuestra que está encendido, aunque se perdiera el aviso: así nunca se
            // queda diciendo "apagado".
            if asked.presence == Presence::Online {
                if let Some(p) = &profile {
                    hub.mark_on(p);
                }
                return asked;
            }
            let mut reading = said_off(heard).unwrap_or_else(|| merge(asked, heard));
            // el aviso del cable llega antes que su siguiente pregunta
            reading.no_percentage |= reading.charge == Charge::Charging
                && crate::ask::find(query).is_some_and(crate::ask::no_percentage_on_cable);
            reading
        }
        Source::Bluetooth { instance_id } => crate::sources::read_bluetooth(instance_id),
        Source::Xinput { slot } => crate::sources::read_xinput(*slot),
        Source::HidStandard { vid, pid, usage_page, usage } => {
            let all = crate::hid::enumerate();
            match all
                .iter()
                .find(|i| i.vid == *vid && i.pid == *pid && i.usage_page == *usage_page && i.usage == *usage)
            {
                Some(i) => crate::sources::read_standard_battery(i),
                None => RawReading::absent(Presence::NotFound, "no detectado"),
            }
        }
    }
}

/// Lectura "apagado" si el propio dispositivo lo avisó. Un dispositivo que carga no cuenta: al
/// conectar el cable algunos dejan de hablar con el receptor sin estar apagados.
fn said_off(heard: Option<HeardState>) -> Option<RawReading> {
    let h = heard?;
    (h.off && h.charge != Some(Charge::Charging)).then(|| RawReading::absent(Presence::Off, "apagado"))
}

/// Une lo que respondió al preguntarle con lo que avisó por su cuenta: la pregunta da el porcentaje
/// cada pocos minutos, y los avisos dan el cable al instante.
fn merge(asked: RawReading, heard: Option<HeardState>) -> RawReading {
    let charge = heard.and_then(|h| h.charge);
    match (asked.presence, charge) {
        // preguntado hace poco, pero sabemos su estado: no hace falta volver a preguntar
        (Presence::Stale, Some(c)) => RawReading { presence: Presence::Online, charge: c, ..asked },
        // por cable o dormido no responde, pero sigue cargando
        (Presence::Asleep, Some(Charge::Charging)) => {
            RawReading { presence: Presence::Online, charge: Charge::Charging, no_percentage: true, ..asked }
        }
        _ => asked,
    }
}

fn median(values: &[u8]) -> u8 {
    let mut v = values.to_vec();
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] as u16 + v[n / 2] as u16).div_ceil(2) as u8 }
}

impl Monitor {
    pub fn new() -> Self {
        let mut m = Self { states: HashMap::new(), history: History::load() };
        m.load_state();
        m
    }

    /// Restaura lo que debe sobrevivir a un reinicio: cuándo llegó cada dispositivo al 100 % (para
    /// que el margen de carga no empiece de nuevo) y su último dato conocido.
    fn load_state(&mut self) {
        let Ok(text) = std::fs::read_to_string(crate::paths::state_file()) else { return };
        let Ok(map) = serde_json::from_str::<HashMap<String, Persisted>>(&text) else { return };
        for (id, p) in map {
            let st = self.states.entry(id).or_default();
            st.full_since = p.full_since;
            st.known = p.known;
        }
    }

    fn save_state(&self) {
        let map: HashMap<&String, Persisted> = self
            .states
            .iter()
            .filter(|(_, s)| s.full_since > 0.0 || s.known.is_some())
            .map(|(k, s)| (k, Persisted { full_since: s.full_since, known: s.known.clone() }))
            .collect();
        if let Ok(j) = serde_json::to_vec(&map) {
            let _ = crate::paths::write_atomic(&crate::paths::state_file(), &j);
        }
    }

    /// Vuelve a permitir las preguntas pausadas (el usuario lo pide desde Detalles).
    pub fn resume_asking(&mut self) {
        for s in self.states.values_mut() {
            s.ask_failures = 0;
            s.ask_paused = false;
        }
    }

    pub fn forget_removed(&mut self, cfg: &Config) {
        let ids: Vec<String> =
            self.states.keys().filter(|k| !cfg.devices.iter().any(|d| &d.id == *k)).cloned().collect();
        for id in ids {
            self.states.remove(&id);
            self.history.forget(&id);
        }
    }

    /// Un ciclo de consulta. `read` permite sustituir el hardware en las pruebas.
    pub fn cycle(
        &mut self,
        cfg: &Config,
        idle_s: f64,
        force: bool,
        now: f64,
        read: &mut dyn FnMut(&DeviceEntry) -> RawReading,
    ) -> (Snapshot, Events) {
        let idle = idle_s > cfg.idle_seconds as f64;
        let mut events = Events::default();
        let mut devices = Vec::new();
        let mut fresh = false;

        for entry in cfg.devices.iter().filter(|d| d.enabled) {
            let st = self.states.entry(entry.id.clone()).or_default();
            if st.known.is_none() {
                st.known = entry.initial.clone();
            }
            let prev_presence = st.last.as_ref().map(|l| l.presence);
            // Lo que se escucha ya está en memoria, y a un dispositivo que no aparece conectado no se
            // le envía nada: ninguno de los dos debe esperar al reintento para notar que volvió.
            let asked = matches!(entry.source, Source::Ask { .. });
            let listen = asked || matches!(entry.source, Source::Listen { .. });
            let absent = (prev_presence == Some(Presence::NotFound) && !asked)
                || (prev_presence == Some(Presence::Off) && !listen);
            // El ciclo corre cada minuto si algún dispositivo solo se nota preguntándole: los que no
            // dependen de eso siguen a su ritmo.
            let own_pace = !matches!(entry.source, Source::Listen { .. } | Source::Ask { .. });
            let rested = !own_pace || now - st.last_probe >= cfg.poll_seconds as f64 - POLL_MARGIN_S;
            let due = (!absent || now - st.last_probe >= ABSENT_RETRY_S) && rested;

            // Con el PC inactivo no se pregunta a nadie, para no despertar a los periféricos. Las
            // fuentes pasivas no llegan al dispositivo, y además un PC "inactivo" no significa que
            // no se use: se puede estar escuchando música con los auriculares.
            let idle_here = idle && !entry.source.is_passive();
            let skip = st.ask_paused || (st.last.is_some() && !force && (idle_here || !due));
            if skip {
                let mut cached = st.last.clone().unwrap_or_else(|| base_status(entry));
                if st.ask_paused {
                    cached.presence = Presence::Paused;
                    cached.eta = tr("Dio errores: se dejó de preguntarle", "It gave errors: stopped asking it").into();
                } else if idle_here && cached.presence == Presence::Online {
                    // no se le ha preguntado: lo único cierto es que es lo último que se supo
                    cached.presence = Presence::Stale;
                }
                cached.name = entry.name.clone();
                cached.kind = entry.kind;
                devices.push(cached);
                continue;
            }

            st.last_probe = now;
            let raw = read(entry);
            let asks = matches!(entry.source, Source::Ask { .. });
            // preguntado hace poco: no se envió nada y todo sigue como estaba
            if asks
                && raw.presence == Presence::Stale
                && let Some(prev) = &st.last
            {
                let mut cached = prev.clone();
                cached.name = entry.name.clone();
                cached.kind = entry.kind;
                devices.push(cached);
                continue;
            }
            if asks {
                match raw.presence {
                    Presence::Online | Presence::Off => st.ask_failures = 0,
                    Presence::Error => {
                        st.ask_failures += 1;
                        if st.ask_failures >= ASK_TRIP_AFTER {
                            st.ask_paused = true;
                            crate::log::warn(&format!("{}: demasiados errores, se deja de preguntar", entry.id));
                            events.paused.push(entry.name.clone());
                        }
                    }
                    _ => {}
                }
            }
            let mut ds = base_status(entry);
            ds.presence = raw.presence;
            ds.approximate = raw.approximate;

            if raw.presence == Presence::Error {
                crate::log::warn(&format!("{}: {}", entry.id, raw.detail));
            }
            // Un mensaje de un perfil de escucha puede traer solo uno de los dos datos: el otro sigue
            // valiendo lo último que se supo.
            let listening =
                raw.presence == Presence::Online && matches!(entry.source, Source::Listen { .. } | Source::Ask { .. });
            let known = st.known.clone().filter(|_| listening);
            let last_known = known.as_ref().and_then(|k| k.battery).filter(|_| !raw.no_percentage);
            let battery_now = raw.battery.or(last_known);
            let charge_now =
                if raw.charge == Charge::Unknown { known.map_or(raw.charge, |k| k.charge) } else { raw.charge };
            let charge = crate::reading::normalize(battery_now, charge_now);
            ds.charge = charge;
            // Al conectar o quitar el cable el porcentaje salta (muchos lo calculan por voltaje): la
            // mediana mezclaría dos situaciones y tardaría minutos en ponerse al día.
            let on_cable = |c| matches!(c, Charge::Charging | Charge::Full);
            if st.last.as_ref().is_some_and(|l| on_cable(l.charge) != on_cable(charge)) {
                st.window.clear();
            }
            match battery_now {
                Some(b) => {
                    st.window.push(b);
                    if st.window.len() > SMOOTH_WINDOW {
                        st.window.remove(0);
                    }
                    let battery = median(&st.window);
                    ds.battery = Some(battery);
                    ds.last_seen = now;

                    apply_top_off(st, entry, cfg, now, charge, &mut ds, &mut events);

                    // Un perfil de escucha informa pocas veces y con valores exactos: no se esperan
                    // 3 lecturas, que podrían tardar días.
                    let confirmed = st.window.len() >= 3 || matches!(entry.source, Source::Listen { .. });
                    let charging = matches!(charge, Charge::Charging | Charge::Full);
                    ds.low = battery <= entry.low_threshold && confirmed && !charging;
                    apply_low(st, entry, cfg, now, battery, &ds, &mut events);

                    // solo se escribe al disco si hay algo nuevo que guardar
                    fresh |= st.known.as_ref().map(|k| (k.battery, k.charge)) != Some((Some(battery), charge));
                    st.known = Some(KnownReading { battery: Some(battery), charge, at: now as u64 });
                    fresh |= self.history.push(&entry.id, now, battery);
                    ds.trend = self.history.trend(&entry.id, 600.0);
                    ds.eta = match charge_eta(st, entry, now, &ds) {
                        Some(text) => text,
                        None => match self.history.estimate_hours(&entry.id, battery) {
                            Some(h) => format_eta(h),
                            None => crate::trf!(
                                "Aprendiendo autonomía… ({} %)",
                                "Learning battery life… ({} %)",
                                (self.history.progress(&entry.id, battery) * 100.0).round()
                            ),
                        },
                    };
                }
                None if charge == Charge::Charging => {
                    // p. ej. mando por cable: carga sin porcentaje
                    let battery = st.known.as_ref().and_then(|k| k.battery);
                    st.known = Some(KnownReading { battery, charge, at: now as u64 });
                    ds.eta = tr("Cargando (sin porcentaje)", "Charging (no percentage)").into();
                    ds.last_seen = now;
                }
                None => {
                    // Sin dato nuevo: se conserva lo último que se supo del dispositivo.
                    let stale = raw.presence == Presence::Stale;
                    if let Some(k) = st.known.clone() {
                        ds.battery = k.battery;
                        ds.last_seen = k.at as f64;
                        if stale {
                            ds.charge = k.charge;
                            apply_top_off(st, entry, cfg, now, k.charge, &mut ds, &mut events);
                        }
                    }
                    let charging = matches!(ds.charge, Charge::Charging | Charge::Full);
                    ds.low = ds.battery.is_some_and(|b| b <= entry.low_threshold) && !charging;
                    if stale && let Some(b) = ds.battery {
                        apply_low(st, entry, cfg, now, b, &ds, &mut events);
                    }
                    ds.eta = if matches!(raw.presence, Presence::Off | Presence::Asleep) {
                        String::new() // ya lo dice el estado
                    } else if !stale {
                        raw.presence.label().into()
                    } else if ds.battery.is_none() && ds.charge == Charge::Unknown {
                        tr("Esperando su primer mensaje", "Waiting for its first message").into()
                    } else {
                        charge_eta(st, entry, now, &ds).unwrap_or_default()
                    };
                }
            }
            ds.history = self.history.sparkline(&entry.id, 96);
            st.last = Some(ds.clone());
            devices.push(ds);
        }

        if fresh {
            self.history.save();
            self.save_state();
        }
        let snapshot = Snapshot {
            version: crate::VERSION.into(),
            updated_at: now,
            next_poll_at: now + cfg.cycle_seconds() as f64,
            service_pid: std::process::id(),
            devices,
        };
        (snapshot, events)
    }
}

/// Margen de carga: tras el 100 % se sigue mostrando "cargando" los minutos configurados.
fn apply_top_off(
    st: &mut DeviceState,
    entry: &DeviceEntry,
    cfg: &Config,
    now: f64,
    charge: Charge,
    ds: &mut DeviceStatus,
    events: &mut Events,
) {
    if charge == Charge::Full {
        if st.full_since == 0.0 {
            st.full_since = now;
        }
        ds.full = now - st.full_since >= entry.top_off_minutes as f64 * 60.0;
        ds.topping_off = !ds.full;
    } else {
        st.full_since = 0.0;
        st.full_notified = false;
    }
    if ds.full && !st.full_notified {
        st.full_notified = true;
        if cfg.notify_full {
            events.full.push(entry.name.clone());
        }
    }
}

/// Aviso de batería baja, repetido cada `renotify_minutes` mientras siga baja.
fn apply_low(
    st: &mut DeviceState,
    entry: &DeviceEntry,
    cfg: &Config,
    now: f64,
    battery: u8,
    ds: &DeviceStatus,
    events: &mut Events,
) {
    if !ds.low {
        st.last_low_notify = 0.0;
    } else if cfg.notify_low && now - st.last_low_notify >= cfg.renotify_minutes as f64 * 60.0 {
        st.last_low_notify = now;
        events.low.push((entry.name.clone(), battery));
    }
}

/// Texto de estado cuando está cargando; `None` si está en batería.
fn charge_eta(st: &DeviceState, entry: &DeviceEntry, now: f64, ds: &DeviceStatus) -> Option<String> {
    if ds.full {
        Some(tr("Carga completa", "Fully charged").into())
    } else if ds.topping_off {
        let rem = (entry.top_off_minutes as f64 * 60.0 - (now - st.full_since)).max(0.0);
        Some(crate::trf!("Completando carga… (~{} min)", "Topping off… (~{} min)", ((rem / 60.0).ceil() as u64).max(1)))
    } else if ds.charge == Charge::Charging {
        Some(tr("Cargando…", "Charging…").into())
    } else {
        None
    }
}

fn base_status(entry: &DeviceEntry) -> DeviceStatus {
    DeviceStatus {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: entry.kind,
        source: entry.source.label(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::DeviceKind;

    fn cfg() -> Config {
        Config {
            devices: vec![DeviceEntry {
                id: "test".into(),
                name: "Ratón".into(),
                kind: DeviceKind::Mouse,
                source: Source::Xinput { slot: 0 },
                low_threshold: 15,
                top_off_minutes: 30,
                enabled: true,
                initial: None,
            }],
            ..Config::default()
        }
    }

    fn cfg_with(source: Source) -> Config {
        let mut c = cfg();
        c.devices[0].source = source;
        c
    }

    fn listen_profile() -> crate::listen::ListenProfile {
        crate::listen::ListenProfile { vid: 1, pid: 2, signals: Vec::new() }
    }

    fn online(b: u8, charge: Charge) -> RawReading {
        RawReading { presence: Presence::Online, battery: Some(b), charge, ..Default::default() }
    }

    fn monitor() -> Monitor {
        Monitor { states: HashMap::new(), history: History::default() }
    }

    #[test]
    fn low_battery_notifies_once_per_interval() {
        let (mut m, c) = (monitor(), cfg());
        let mut t = 1000.0;
        let mut lows = 0;
        for _ in 0..6 {
            let (_, ev) = m.cycle(&c, 0.0, true, t, &mut |_| online(10, Charge::Discharging));
            lows += ev.low.len();
            t += 60.0;
        }
        assert_eq!(lows, 1, "con 3 lecturas avisa, y no repite antes de renotify_minutes");
    }

    #[test]
    fn charging_without_a_percentage_does_not_show_the_old_number() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        m.cycle(&c, 0.0, false, 1000.0, &mut |_| online(76, Charge::Discharging));
        let on_cable = || RawReading {
            presence: Presence::Online,
            charge: Charge::Charging,
            no_percentage: true,
            ..Default::default()
        };
        let (snap, _) = m.cycle(&c, 0.0, false, 1060.0, &mut |_| on_cable());
        assert_eq!((snap.devices[0].battery, snap.devices[0].charge), (None, Charge::Charging));
        // al volver a ser inalámbrico, el número es otra vez real
        let (snap, _) = m.cycle(&c, 0.0, false, 1120.0, &mut |_| online(80, Charge::Discharging));
        assert_eq!(snap.devices[0].battery, Some(80));
    }

    #[test]
    fn an_asked_device_that_disappeared_comes_back_at_once() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        m.cycle(&c, 0.0, false, 1000.0, &mut |_| RawReading::absent(Presence::NotFound, "no detectado"));
        let (snap, _) = m.cycle(&c, 0.0, false, 1060.0, &mut |_| online(78, Charge::Discharging));
        assert_eq!(snap.devices[0].presence, Presence::Online);
    }

    #[test]
    fn idle_pc_does_not_poll_sources_that_reach_the_device() {
        let (mut m, c) = (monitor(), cfg_with(Source::HidStandard { vid: 1, pid: 2, usage_page: 6, usage: 32 }));
        let mut calls = 0;
        m.cycle(&c, 0.0, false, 0.0, &mut |_| {
            calls += 1;
            online(50, Charge::Discharging)
        });
        let (snap, _) = m.cycle(&c, 9999.0, false, 400.0, &mut |_| {
            calls += 1;
            online(50, Charge::Discharging)
        });
        assert_eq!(calls, 1, "con el PC inactivo no se consulta");
        assert_eq!(snap.devices[0].presence, Presence::Stale, "sin consultar no se afirma que esté dormido");
    }

    #[test]
    fn top_off_then_full_once() {
        let (mut m, c) = (monitor(), cfg());
        let t0 = 1000.0;
        let (s, ev) = m.cycle(&c, 0.0, true, t0, &mut |_| online(100, Charge::Full));
        assert!(s.devices[0].topping_off && !s.devices[0].full && ev.full.is_empty());
        let (s, ev) = m.cycle(&c, 0.0, true, t0 + 31.0 * 60.0, &mut |_| online(100, Charge::Full));
        assert!(s.devices[0].full && ev.full.len() == 1);
        let (_, ev) = m.cycle(&c, 0.0, true, t0 + 40.0 * 60.0, &mut |_| online(100, Charge::Full));
        assert!(ev.full.is_empty(), "el aviso de completa sale una sola vez");
    }

    #[test]
    fn idle_pc_does_not_mark_passive_devices_asleep() {
        // el PC cuenta como inactivo si solo se escucha música, pero los auriculares están en uso
        let (mut m, c) = (monitor(), cfg());
        let mut calls = 0;
        m.cycle(&c, 0.0, false, 1000.0, &mut |_| online(50, Charge::Discharging));
        let (snap, _) = m.cycle(&c, 9999.0, false, 1400.0, &mut |_| {
            calls += 1;
            online(50, Charge::Discharging)
        });
        assert_eq!(calls, 1);
        assert_eq!(snap.devices[0].presence, Presence::Online);
    }

    #[test]
    fn listen_device_without_new_messages_keeps_its_last_value() {
        let mut c = cfg_with(Source::Listen { profile: listen_profile() });
        c.devices[0].initial = Some(KnownReading { battery: Some(73), charge: Charge::Discharging, at: 900 });
        let mut m = monitor();
        let (snap, _) = m.cycle(&c, 0.0, true, 1000.0, &mut |_| RawReading::absent(Presence::Stale, ""));
        let d = &snap.devices[0];
        assert_eq!((d.battery, d.presence, d.last_seen), (Some(73), Presence::Stale, 900.0));
        assert!(m.history.points.is_empty(), "un dato repetido no es una medición nueva");
    }

    #[test]
    fn listen_device_with_no_data_yet_says_it_is_waiting() {
        let c = cfg_with(Source::Listen { profile: listen_profile() });
        let (snap, _) = monitor().cycle(&c, 0.0, true, 1000.0, &mut |_| RawReading::absent(Presence::Stale, ""));
        assert_eq!(snap.devices[0].battery, None);
        assert_eq!(snap.devices[0].eta, tr("Esperando su primer mensaje", "Waiting for its first message"));
    }

    #[test]
    fn listen_device_warns_on_its_first_low_report_and_repeats_while_stale() {
        let c = cfg_with(Source::Listen { profile: listen_profile() });
        let mut m = monitor();
        let (_, ev) = m.cycle(&c, 0.0, true, 1000.0, &mut |_| online(10, Charge::Discharging));
        assert_eq!(ev.low.len(), 1, "no debe esperar 3 lecturas");
        let (_, ev) = m.cycle(&c, 0.0, true, 1060.0, &mut |_| RawReading::absent(Presence::Stale, ""));
        assert!(ev.low.is_empty());
        let (_, ev) = m.cycle(&c, 0.0, true, 1700.0, &mut |_| RawReading::absent(Presence::Stale, ""));
        assert_eq!(ev.low.len(), 1, "se repite cada renotify_minutes");
    }

    #[test]
    fn a_charge_message_keeps_the_last_known_percentage() {
        let c = cfg_with(Source::Listen { profile: listen_profile() });
        let mut m = monitor();
        m.cycle(&c, 0.0, true, 1000.0, &mut |_| online(46, Charge::Discharging));
        // llega solo el mensaje "cable conectado", sin porcentaje
        let (snap, _) = m.cycle(&c, 0.0, true, 1060.0, &mut |_| RawReading {
            presence: Presence::Online,
            charge: Charge::Charging,
            ..Default::default()
        });
        let d = &snap.devices[0];
        assert_eq!((d.battery, d.charge, d.eta.as_str()), (Some(46), Charge::Charging, tr("Cargando…", "Charging…")));
    }

    #[test]
    fn a_device_turned_off_keeps_its_last_percentage() {
        let c = cfg_with(Source::Listen { profile: listen_profile() });
        let mut m = monitor();
        m.cycle(&c, 0.0, false, 1000.0, &mut |_| online(46, Charge::Discharging));
        let (snap, _) = m.cycle(&c, 0.0, false, 1010.0, &mut |_| RawReading::absent(Presence::Off, "apagado"));
        assert_eq!((snap.devices[0].presence, snap.devices[0].battery), (Presence::Off, Some(46)));
        // al encenderlo se entera enseguida, sin esperar el reintento de los apagados
        let (snap, _) = m.cycle(&c, 0.0, false, 1020.0, &mut |_| online(46, Charge::Discharging));
        assert_eq!(snap.devices[0].presence, Presence::Online);
    }

    #[test]
    fn repeated_errors_stop_the_questions_until_the_user_resumes() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        let mut calls = 0;
        let mut paused = 0;
        for i in 0..6 {
            let (snap, ev) = m.cycle(&c, 0.0, true, 1000.0 + i as f64 * 60.0, &mut |_| {
                calls += 1;
                RawReading::absent(Presence::Error, "fallo")
            });
            paused += ev.paused.len();
            if i >= 3 {
                assert_eq!(snap.devices[0].presence, Presence::Paused);
            }
        }
        assert_eq!(calls, 3, "tras 3 errores seguidos no se le vuelve a enviar nada");
        assert_eq!(paused, 1);
        m.resume_asking();
        m.cycle(&c, 0.0, true, 2000.0, &mut |_| {
            calls += 1;
            online(50, Charge::Discharging)
        });
        assert_eq!(calls, 4);
    }

    #[test]
    fn asking_never_happens_with_the_pc_idle() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        let mut calls = 0;
        m.cycle(&c, 0.0, false, 1000.0, &mut |_| {
            calls += 1;
            online(50, Charge::Discharging)
        });
        m.cycle(&c, 9999.0, false, 2000.0, &mut |_| {
            calls += 1;
            online(50, Charge::Discharging)
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_question_too_soon_keeps_the_device_as_it_was() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        m.cycle(&c, 0.0, true, 1000.0, &mut |_| online(50, Charge::Discharging));
        let (snap, _) = m.cycle(&c, 0.0, true, 1010.0, &mut |_| RawReading::absent(Presence::Stale, ""));
        assert_eq!((snap.devices[0].presence, snap.devices[0].battery), (Presence::Online, Some(50)));
    }

    fn heard(off: bool, charge: Option<Charge>) -> Option<HeardState> {
        Some(HeardState { off, charge })
    }

    #[test]
    fn a_device_that_said_it_is_off_shows_as_off() {
        assert_eq!(said_off(heard(true, None)).map(|r| r.presence), Some(Presence::Off));
        assert!(said_off(heard(false, None)).is_none());
        assert!(said_off(None).is_none());
    }

    #[test]
    fn charging_is_not_off_even_if_the_link_was_lost() {
        assert!(said_off(heard(true, Some(Charge::Charging))).is_none());
    }

    #[test]
    fn a_cable_notice_shows_at_once_between_questions() {
        let too_soon = || RawReading::absent(Presence::Stale, "");
        let r = merge(too_soon(), heard(false, Some(Charge::Charging)));
        assert_eq!((r.presence, r.charge), (Presence::Online, Charge::Charging));
        let r = merge(too_soon(), heard(false, Some(Charge::Discharging)));
        assert_eq!((r.presence, r.charge), (Presence::Online, Charge::Discharging));
        assert_eq!(merge(too_soon(), None).presence, Presence::Stale);
    }

    #[test]
    fn an_answer_wins_over_older_notices() {
        let r = merge(online(53, Charge::Discharging), heard(false, Some(Charge::Charging)));
        assert_eq!((r.battery, r.charge), (Some(53), Charge::Discharging));
    }

    #[test]
    fn a_charge_notice_keeps_the_percentage_for_an_asking_device() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        m.cycle(&c, 0.0, true, 1000.0, &mut |_| online(53, Charge::Discharging));
        let notice = merge(RawReading::absent(Presence::Stale, ""), heard(false, Some(Charge::Charging)));
        let (snap, _) = m.cycle(&c, 0.0, true, 1010.0, &mut |_| notice.clone());
        let d = &snap.devices[0];
        assert_eq!((d.battery, d.charge, d.presence), (Some(53), Charge::Charging, Presence::Online));
    }

    #[test]
    fn turning_an_asking_device_on_shows_at_once() {
        let c = cfg_with(Source::Ask { query: "x".into(), accepted_at: 1, notices: None });
        let mut m = monitor();
        m.cycle(&c, 0.0, false, 1000.0, &mut |_| online(53, Charge::Discharging));
        m.cycle(&c, 0.0, false, 1010.0, &mut |_| RawReading::absent(Presence::Off, "apagado"));
        // sin forzar, como cuando llega el aviso de encendido: no debe esperar 10 minutos
        let (snap, _) = m.cycle(&c, 0.0, false, 1020.0, &mut |_| online(53, Charge::Discharging));
        assert_eq!(snap.devices[0].presence, Presence::Online);
    }
}
