//! Modo aprendizaje: SOLO ESCUCHA. Abre las interfaces del fabricante sin derecho de escritura
//! (con ese handle Windows no deja enviar nada) y graba lo que el dispositivo manda por su
//! cuenta mientras el usuario conecta y desconecta el cable. Después busca bytes que cambian
//! con la carga y prepara un informe para pedir soporte en un issue.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::bytes;
use crate::hid::{HidDevice, HidInterface};

const MAX_SAMPLES: usize = 4000;

/// Fase de la escucha en que el dispositivo se apaga.
pub const PHASE_OFF: u8 = 3;
/// Última fase: el dispositivo se enciende otra vez.
pub const PHASE_ON: u8 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub t_ms: u64,
    /// 0 = en batería, 1 = con el cable conectado, 2 = cable quitado otra vez, 3 = apagado,
    /// 4 = encendido de nuevo.
    pub phase: u8,
    pub interface: usize,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedInterface {
    pub usage_page: String,
    pub usage: String,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub app_version: String,
    pub vid: String,
    pub pid: String,
    pub product: String,
    pub manufacturer: String,
    /// Interfaces escuchadas (las del fabricante).
    pub interfaces: Vec<CapturedInterface>,
    /// Todas las interfaces del dispositivo, para saber si otra podría servir.
    #[serde(default)]
    pub all_interfaces: Vec<CapturedInterface>,
    /// Porcentaje que muestra su app oficial, si el usuario lo indica.
    #[serde(default)]
    pub official_percent: Option<u8>,
    pub samples: Vec<Sample>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// Vale distinto con el cable conectado que sin él.
    Charge,
    /// Valor 1-100 estable: candidato a porcentaje.
    Battery,
    /// Solo aparece al apagarlo. `values[0]` son los valores "apagado"; `values[1]`, los demás.
    Power,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub interface: usize,
    pub report_id: u8,
    /// Tipo de mensaje (byte 1), si el reporte lo usa así. `None` si el report ID ya basta.
    pub tag: Option<u8>,
    /// Largo del reporte donde aparece.
    pub len: usize,
    pub offset: usize,
    pub kind: FindingKind,
    /// XOR con el que se lee (0 o 0xff para valores invertidos).
    pub xor: u8,
    /// Valores vistos en cada fase (ya aplicado el XOR).
    pub values: [Vec<u8>; 3],
    /// Último valor visto (ya aplicado el XOR).
    pub last: u8,
    /// Cuándo se vio por última vez (milisegundos desde que empezó la escucha).
    pub last_t: u64,
    /// En cuántas fases de la escucha apareció este tipo de mensaje.
    pub seen_in: u8,
    pub score: u8,
}

fn described(i: &HidInterface) -> CapturedInterface {
    CapturedInterface {
        usage_page: format!("{:#06x}", i.usage_page),
        usage: format!("{:#06x}", i.usage),
        input_len: i.input_len,
        output_len: i.output_len,
        feature_len: i.feature_len,
    }
}

pub struct Listener {
    stop: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    samples: Arc<Mutex<Vec<Sample>>>,
    threads: Vec<JoinHandle<()>>,
    ifaces: Vec<HidInterface>,
    all: Vec<HidInterface>,
}

impl Listener {
    /// Empieza a escuchar las interfaces del fabricante del dispositivo (las demás se ignoran:
    /// ahí viajan las teclas y los movimientos del ratón, que no se graban nunca).
    pub fn start(all: &[HidInterface]) -> Self {
        let ifaces: Vec<HidInterface> =
            all.iter().filter(|i| i.is_vendor_defined() && i.input_len > 0).cloned().collect();
        let stop = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(AtomicU8::new(0));
        let samples = Arc::new(Mutex::new(Vec::new()));
        let started = Instant::now();
        let mut threads = Vec::new();
        for (idx, info) in ifaces.iter().enumerate() {
            let Ok(dev) = HidDevice::open(info) else { continue };
            let (stop, phase, samples) = (stop.clone(), phase.clone(), samples.clone());
            threads.push(std::thread::spawn(move || {
                let mut last: Option<Vec<u8>> = None;
                while !stop.load(Ordering::Relaxed) {
                    match dev.read(250) {
                        Ok(Some(data)) => {
                            // solo se guardan los cambios, para que el informe sea pequeño
                            if last.as_ref() == Some(&data) {
                                continue;
                            }
                            let mut s = samples.lock().unwrap_or_else(|e| e.into_inner());
                            if s.len() < MAX_SAMPLES {
                                s.push(Sample {
                                    t_ms: started.elapsed().as_millis() as u64,
                                    phase: phase.load(Ordering::Relaxed),
                                    interface: idx,
                                    data: bytes::format(&data),
                                });
                            }
                            last = Some(data);
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            }));
        }
        Self { stop, phase, samples, threads, ifaces, all: all.to_vec() }
    }

    pub fn listening_interfaces(&self) -> usize {
        self.threads.len()
    }

    pub fn set_phase(&self, phase: u8) {
        self.phase.store(phase.min(PHASE_ON), Ordering::Relaxed);
    }

    pub fn sample_count(&self) -> usize {
        self.samples.lock().map(|s| s.len()).unwrap_or(0)
    }

    pub fn finish(mut self, vid: u16, pid: u16) -> Capture {
        self.stop.store(true, Ordering::Relaxed);
        for t in std::mem::take(&mut self.threads) {
            let _ = t.join();
        }
        let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
        let first = self.ifaces.first();
        let mut cap = Capture {
            app_version: crate::VERSION.into(),
            vid: format!("{vid:04x}"),
            pid: format!("{pid:04x}"),
            product: first.map(|i| i.product.clone()).unwrap_or_default(),
            manufacturer: first.map(|i| i.manufacturer.clone()).unwrap_or_default(),
            interfaces: self.ifaces.iter().map(described).collect(),
            all_interfaces: self.all.iter().map(described).collect(),
            official_percent: None,
            samples,
            findings: Vec::new(),
        };
        cap.findings = analyze(&cap);
        cap
    }
}

/// Salir del asistente a mitad de la escucha también la detiene.
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Valores con los que los dispositivos rellenan lo que no usan.
fn is_padding(v: u8) -> bool {
    v == 0x00 || v == 0xff
}

/// Posiciones que son texto (4 o más caracteres imprimibles seguidos, como un nombre o una versión
/// de firmware): sus bytes parecen porcentajes, pero no lo son.
pub(crate) fn text_offsets(report: &[u8], xor: u8) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut start = None;
    for i in 0..=report.len() {
        let printable = report.get(i).is_some_and(|b| (0x20..=0x7e).contains(&(b ^ xor)));
        match (printable, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= 4 {
                    out.extend(s..i);
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Una pareja de valores altos y casi iguales es el porcentaje subiendo, no un estado de carga.
fn looks_like_percentage(values: &BTreeSet<u8>) -> bool {
    match (values.first(), values.last()) {
        (Some(&min), Some(&max)) => min >= 5 && max <= 100 && max - min <= 3,
        _ => false,
    }
}

/// Busca bytes candidatos a carga, apagado y batería, separando los mensajes por tipo para no
/// mezclar sus bytes.
///
/// Muchos dispositivos solo hablan cuando algo cambia: sin cable no envían nada hasta que se lo
/// quitan. Por eso "sin cable" es cualquier cosa vista fuera de la fase con cable, antes o después.
///
/// Un mismo tipo de mensaje puede venir con partes vacías (relleno): un valor solo cuenta como
/// "cargando" si no aparece sin cable ni con el dispositivo apagado.
///
/// El apagado solo se ofrece si al encenderlo también llega algo: si no, la app se quedaría
/// diciendo "apagado" sin poder enterarse de que ya no lo está. Se compara contra las fases sin
/// cable, porque al conectar el cable algunos dispositivos dejan de usar el receptor.
pub fn analyze(cap: &Capture) -> Vec<Finding> {
    let reports: Vec<(&Sample, Vec<u8>)> = cap
        .samples
        .iter()
        .filter_map(|s| {
            let r = bytes::parse(&s.data).ok()?;
            (!r.is_empty()).then_some((s, r))
        })
        .collect();
    // Sin numerar, el byte 1 es el tipo de mensaje. Numerado, el report ID ya lo indica y el byte 1
    // puede ser un dato; salvo que varíe, y entonces también es un tipo.
    let mut second_bytes: BTreeMap<(usize, u8, usize), BTreeSet<u8>> = BTreeMap::new();
    for (s, r) in &reports {
        if let Some(&b) = r.get(1) {
            second_bytes.entry((s.interface, r[0], r.len())).or_default().insert(b);
        }
    }
    // (interfaz, report id, tipo de mensaje, largo) -> lista de (fase, instante, bytes)
    type Key = (usize, u8, Option<u8>, usize);
    let mut groups: BTreeMap<Key, Vec<(u8, u64, Vec<u8>)>> = BTreeMap::new();
    for (s, r) in reports {
        let varies = second_bytes.get(&(s.interface, r[0], r.len())).is_some_and(|v| v.len() >= 2);
        let tag = if r[0] == 0 || varies { r.get(1).copied() } else { None };
        groups.entry((s.interface, r[0], tag, r.len())).or_default().push((s.phase, s.t_ms, r));
    }
    let heard_on = cap.samples.iter().any(|s| s.phase == PHASE_ON);
    let mut charge = Vec::new();
    let mut battery = Vec::new();
    let mut power = Vec::new();
    for ((interface, report_id, tag, len), rows) in groups {
        let seen_in = rows.iter().map(|r| r.0).collect::<BTreeSet<_>>().len() as u8;
        // Un tipo de mensaje que solo llega al apagarlo: su sola llegada ya lo dice.
        if heard_on && rows.iter().all(|r| r.0 == PHASE_OFF) {
            let offset = if tag.is_some() { 1 } else { 0 };
            if let Some(&value) = rows[0].2.get(offset) {
                power.push(Finding {
                    interface,
                    report_id,
                    tag,
                    len,
                    offset,
                    kind: FindingKind::Power,
                    xor: 0,
                    values: [vec![value], Vec::new(), Vec::new()],
                    last: value,
                    last_t: rows[rows.len() - 1].1,
                    seen_in,
                    score: 1,
                });
            }
        }
        let text: [BTreeSet<usize>; 2] =
            [0u8, 0xff].map(|xor| rows.iter().flat_map(|r| text_offsets(&r.2, xor)).collect());
        // si el byte 1 es el tipo de mensaje, no puede variar dentro del grupo
        let first = if tag.is_some() { 2 } else { 1 };
        for offset in first..len {
            let (last, last_t) = rows.last().map(|r| (r.2[offset], r.1)).unwrap_or((0, 0));
            let mut sets: [BTreeSet<u8>; 3] = Default::default();
            let (mut off, mut on) = (BTreeSet::new(), BTreeSet::new());
            for (phase, _, b) in &rows {
                if *phase <= 2 {
                    sets[*phase as usize].insert(b[offset]);
                }
                if *phase == PHASE_OFF {
                    off.insert(b[offset]);
                } else if *phase != 1 {
                    on.insert(b[offset]);
                }
            }
            let everything: BTreeSet<u8> = rows.iter().map(|r| r.2[offset]).collect();
            if heard_on
                && !off.is_empty()
                && !on.is_empty()
                && off.is_disjoint(&on)
                && !off.iter().all(|v| is_padding(*v))
                && !looks_like_percentage(&off.union(&on).copied().collect())
            {
                power.push(Finding {
                    interface,
                    report_id,
                    tag,
                    len,
                    offset,
                    kind: FindingKind::Power,
                    xor: 0,
                    values: [off.iter().copied().collect(), on.iter().copied().collect(), Vec::new()],
                    last,
                    last_t,
                    seen_in,
                    score: 2,
                });
            }
            let with_cable = &sets[1];
            let without: BTreeSet<u8> = sets[0].union(&sets[2]).copied().collect();
            let clean = with_cable.is_disjoint(&without);
            // lo que solo aparece con el cable; si hay mezcla, el relleno no cuenta
            let charging: BTreeSet<u8> = with_cable
                .difference(&without)
                .filter(|v| !off.contains(*v) && (clean || !is_padding(**v)))
                .copied()
                .collect();
            let all: BTreeSet<u8> = with_cable.union(&without).copied().collect();
            if !charging.is_empty() && !without.is_empty() && !looks_like_percentage(&all) {
                let both_sides = !sets[0].is_empty() && !sets[2].is_empty();
                charge.push(Finding {
                    interface,
                    report_id,
                    tag,
                    len,
                    offset,
                    kind: FindingKind::Charge,
                    xor: 0,
                    values: [
                        sets[0].iter().copied().collect(),
                        charging.iter().copied().collect(),
                        sets[2].iter().copied().collect(),
                    ],
                    last,
                    last_t,
                    seen_in,
                    // sin mezcla y visto antes y después del cable: la prueba más fuerte
                    score: if !clean {
                        1
                    } else if both_sides {
                        3
                    } else {
                        2
                    },
                });
                continue;
            }
            // La batería casi no cambia en un minuto: se buscan valores 1-100 estables, tal cual o
            // invertidos (XOR 0xff), sin contar el relleno, y el usuario elige el que coincide.
            for (i, xor) in [0u8, 0xff].into_iter().enumerate() {
                if text[i].contains(&offset) {
                    continue;
                }
                let all: BTreeSet<u8> = everything.iter().filter(|v| **v != xor).map(|v| v ^ xor).collect();
                if !all.is_empty() && all.len() <= 2 && all.iter().all(|&v| (1..=100).contains(&v)) {
                    let newest = rows.iter().rev().find(|r| r.2[offset] != xor);
                    battery.push(Finding {
                        interface,
                        report_id,
                        tag,
                        len,
                        offset,
                        kind: FindingKind::Battery,
                        xor,
                        values: sets.clone().map(|s| s.into_iter().map(|v| v ^ xor).collect()),
                        last: newest.map_or(0, |r| r.2[offset] ^ xor),
                        last_t: newest.map_or(0, |r| r.1),
                        seen_in,
                        score: 1,
                    });
                }
            }
        }
    }
    // primero la prueba más fuerte; a igualdad, el byte con menos valores distintos (un estado, no un contador)
    charge.sort_by_key(|f: &Finding| (std::cmp::Reverse(f.score), f.values.iter().map(Vec::len).sum::<usize>()));
    charge.truncate(4);
    power.sort_by_key(|f: &Finding| (std::cmp::Reverse(f.score), f.values.iter().map(Vec::len).sum::<usize>()));
    power.truncate(4);
    // los valores muy bajos suelen ser otros campos: que no desplacen a los buenos al recortar
    battery.sort_by_key(|f| f.last < LIKELY_PERCENT);
    battery.truncate(24);
    charge.into_iter().chain(power).chain(battery).collect()
}

/// Por debajo de esto, un valor casi nunca es la batería (suelen ser estados o versiones).
const LIKELY_PERCENT: u8 = 5;
/// Valores así de cercanos en mensajes distintos son el mismo dato medido en dos momentos.
const SAME_PERCENT_GAP: u8 = 3;
/// Más opciones que esto no ayudan a nadie a elegir.
const MAX_OPTIONS: usize = 6;

/// Una respuesta posible a "¿cuánta batería tiene?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryOption {
    /// Valor más reciente.
    pub value: u8,
    /// Índices en los hallazgos: un dispositivo puede dar el porcentaje en varios mensajes.
    pub members: Vec<usize>,
}

/// Agrupa los candidatos a porcentaje en opciones que una persona pueda reconocer. Un mensaje que
/// solo llegó una vez suele ser información suelta (al conectarse), no su estado: si hay candidatos
/// en mensajes que se repiten, solo se ofrecen esos.
pub fn battery_options(findings: &[Finding]) -> Vec<BatteryOption> {
    let mut found: Vec<(usize, &Finding)> =
        findings.iter().enumerate().filter(|(_, f)| f.kind == FindingKind::Battery).collect();
    if found.iter().any(|(_, f)| f.last >= LIKELY_PERCENT) {
        found.retain(|(_, f)| f.last >= LIKELY_PERCENT);
    }
    if found.iter().any(|(_, f)| f.seen_in >= 2) {
        found.retain(|(_, f)| f.seen_in >= 2);
    }
    found.sort_by_key(|(_, f)| std::cmp::Reverse(f.last));
    let same_message =
        |a: &Finding, b: &Finding| (a.interface, a.report_id, a.tag, a.len) == (b.interface, b.report_id, b.tag, b.len);
    let mut groups: Vec<Vec<(usize, &Finding)>> = Vec::new();
    for item in found {
        match groups.last_mut() {
            // un mismo mensaje no trae dos veces la batería: esos son dos candidatos distintos
            Some(g)
                if g[0].1.last - item.1.last <= SAME_PERCENT_GAP && !g.iter().any(|(_, f)| same_message(f, item.1)) =>
            {
                g.push(item)
            }
            _ => groups.push(vec![item]),
        }
    }
    groups.truncate(MAX_OPTIONS);
    groups
        .into_iter()
        .map(|g| BatteryOption {
            value: g.iter().max_by_key(|(_, f)| f.last_t).map_or(0, |(_, f)| f.last),
            members: g.iter().map(|(i, _)| *i).collect(),
        })
        .collect()
}

/// Construye el perfil de escucha con los hallazgos elegidos. `ifaces` son las interfaces escuchadas.
pub fn build_profile(
    ifaces: &[HidInterface],
    charge: Option<&Finding>,
    batteries: &[&Finding],
    power: Option<&Finding>,
) -> Option<crate::listen::ListenProfile> {
    use crate::listen::{ListenProfile, Signal, SignalKind};
    let signal = |f: &Finding, kind: SignalKind| {
        let iface = ifaces.get(f.interface)?;
        Some(Signal {
            usage_page: iface.usage_page,
            usage: iface.usage,
            report_id: f.report_id,
            tag: f.tag,
            min_len: f.len,
            at: f.offset,
            kind,
        })
    };
    let mut signals: Vec<Signal> = Vec::new();
    if let Some(c) = charge {
        signals.push(signal(c, SignalKind::Charge { charging: c.values[1].clone() })?);
    }
    for b in batteries {
        signals.push(signal(b, SignalKind::Battery { xor: b.xor })?);
    }
    if let Some(p) = power {
        signals.push(signal(p, SignalKind::Power { off: p.values[0].clone() })?);
    }
    let first = ifaces.first()?;
    (!signals.is_empty()).then_some(ListenProfile { vid: first.vid, pid: first.pid, signals })
}

impl Listener {
    /// Interfaces escuchadas, en el mismo orden que `Finding::interface`.
    pub fn interfaces(&self) -> &[HidInterface] {
        &self.ifaces
    }
}

/// Informes de escucha que se conservan.
const MAX_REPORTS: usize = 10;

/// Guarda el informe en `%APPDATA%\YesCharge\captures` y devuelve la ruta. Se queda en el PC: solo
/// sale de aquí si el usuario lo adjunta a un issue.
pub fn save(cap: &Capture) -> std::io::Result<std::path::PathBuf> {
    let name = format!("captura-{}-{}-{}.json", cap.vid, cap.pid, crate::now_secs() as u64);
    let path = crate::paths::captures_dir().join(name);
    let json = serde_json::to_vec_pretty(cap).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    prune_reports();
    Ok(path)
}

/// Borra los informes más antiguos para que la carpeta no crezca sin límite.
fn prune_reports() {
    let Ok(dir) = std::fs::read_dir(crate::paths::captures_dir()) else { return };
    let mut files: Vec<_> = dir
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("captura-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in files.into_iter().skip(MAX_REPORTS) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(phase: u8, data: &str) -> Sample {
        Sample { t_ms: 0, phase, interface: 0, data: data.into() }
    }

    #[test]
    fn finds_byte_that_follows_the_cable() {
        let cap = Capture {
            app_version: String::new(),
            vid: String::new(),
            pid: String::new(),
            product: String::new(),
            manufacturer: String::new(),
            interfaces: Vec::new(),
            all_interfaces: Vec::new(),
            official_percent: None,
            samples: vec![
                sample(0, "07 50 00 11"),
                sample(1, "07 50 01 22"),
                sample(1, "07 50 01 23"),
                sample(2, "07 50 00 24"),
            ],
            findings: Vec::new(),
        };
        let f = analyze(&cap);
        assert_eq!(f[0].offset, 2);
        assert_eq!(f[0].kind, FindingKind::Charge);
        assert_eq!(f[0].score, 3);
        assert!(f.iter().any(|x| x.kind == FindingKind::Battery && x.offset == 1 && x.last == 0x50));
    }

    fn capture(samples: Vec<Sample>) -> Capture {
        Capture {
            app_version: String::new(),
            vid: String::new(),
            pid: String::new(),
            product: String::new(),
            manufacturer: String::new(),
            interfaces: Vec::new(),
            all_interfaces: Vec::new(),
            official_percent: None,
            samples,
            findings: Vec::new(),
        }
    }

    fn sample_at(t_ms: u64, phase: u8, interface: usize, data: &str) -> Sample {
        Sample { t_ms, phase, interface, data: data.into() }
    }

    fn iface(usage_page: u16) -> HidInterface {
        HidInterface {
            path: String::new(),
            vid: 1,
            pid: 2,
            version: 0,
            usage_page,
            usage: 1,
            input_len: 65,
            output_len: 0,
            feature_len: 0,
            product: String::new(),
            manufacturer: String::new(),
        }
    }

    #[test]
    fn finds_charge_when_the_device_only_talks_on_changes() {
        // sin cable no envía nada: solo un mensaje al conectarlo y otro al quitarlo
        let cap = capture(vec![sample(1, "00 b7 00 00 4c 01 00"), sample(2, "00 b7 00 00 4c 00 00")]);
        let f = analyze(&cap);
        assert_eq!((f[0].kind, f[0].offset, f[0].score), (FindingKind::Charge, 5, 2));
    }

    #[test]
    fn a_rising_percentage_is_not_a_charge_state() {
        let cap = capture(vec![sample(1, "00 b7 4d 01"), sample(2, "00 b7 4c 00")]);
        let f = analyze(&cap);
        assert_eq!(f[0].offset, 3, "el estado de carga es el byte 3");
        assert!(f.iter().any(|x| x.kind == FindingKind::Battery && x.offset == 2));
    }

    /// Captura real: la carga y el porcentaje vienen en mensajes distintos, y por dos interfaces.
    #[test]
    fn data_spread_over_messages_and_interfaces() {
        use crate::listen::Value;
        use crate::reading::Charge;
        let cap = capture(vec![
            sample_at(15000, 1, 0, "00 bb 01 02 00*61"),
            sample_at(15100, 1, 1, "00 b0 03 02 2f 01 00*59"),
            sample_at(48300, 2, 0, "00 bb 03 02 00*61"),
            sample_at(48400, 2, 0, "00 b7 2e 02 00*61"),
        ]);
        let f = analyze(&cap);
        let c = f.iter().find(|x| x.kind == FindingKind::Charge).expect("estado de carga");
        assert_eq!((c.tag, c.offset), (Some(0xbb), 2));

        let options = battery_options(&f);
        assert_eq!(options.len(), 1, "47 y 46 son el mismo dato; 1, 2 y 3 no son la batería");
        assert_eq!(options[0].value, 46, "se muestra el valor más reciente");

        let batteries: Vec<&Finding> = options[0].members.iter().map(|&i| &f[i]).collect();
        let p = build_profile(&[iface(0xff00), iface(0xffc0)], Some(c), &batteries, None).expect("perfil");
        assert!(p.signals.iter().any(|s| s.usage_page == 0xffc0), "escucha las dos interfaces");
        let read = |hex: &str| {
            let r = bytes::parse(hex).unwrap();
            p.signals.iter().filter_map(|s| s.read(&r)).collect::<Vec<_>>()
        };
        assert_eq!(read("00 bb 01 02 00*61"), vec![Value::Charge(Charge::Charging)]);
        assert_eq!(read("00 bb 03 02 00*61"), vec![Value::Charge(Charge::Discharging)]);
        assert_eq!(read("00 b7 2e 02 00*61"), vec![Value::Battery(46)]);
        assert_eq!(read("00 b0 03 02 2f 01 00*59"), vec![Value::Battery(47)]);
    }

    #[test]
    fn learns_power_off_from_a_byte_that_changes() {
        let f = analyze(&capture(vec![
            sample(1, "00 b0 03 02 2f 01"),
            sample(2, "00 b0 03 02 2f 00"),
            sample(3, "00 b0 02 02 00 00"),
            sample(4, "00 b0 03 02 2f 00"),
        ]));
        let p = f.iter().find(|x| x.kind == FindingKind::Power).expect("apagado");
        assert_eq!((p.offset, p.values[0].clone()), (2, vec![2]));
    }

    #[test]
    fn learns_power_off_from_a_message_only_sent_when_turning_off() {
        let f = analyze(&capture(vec![sample(2, "00 b7 2e 02"), sample(3, "00 c1 00 00"), sample(4, "00 b7 2e 02")]));
        let p = f.iter().find(|x| x.kind == FindingKind::Power).expect("apagado");
        assert_eq!((p.offset, p.tag, p.values[0].clone()), (1, Some(0xc1), vec![0xc1]));
    }

    #[test]
    fn power_is_not_offered_if_it_stays_silent_when_turned_on() {
        // sin aviso al encenderlo, la app se quedaría diciendo "apagado" para siempre
        let f = analyze(&capture(vec![sample(2, "00 b0 03 02"), sample(3, "00 b0 02 02")]));
        assert!(f.iter().all(|x| x.kind != FindingKind::Power));
    }

    /// Captura real de unos auriculares: avisan de todo, pero al encender también envían su versión
    /// de firmware como texto y varios mensajes de configuración que parecen porcentajes.
    #[test]
    fn real_capture_ignores_firmware_text_and_one_off_messages() {
        let row = |phase, data: &str| sample(phase, &format!("{data} 00*49"));
        let cap = capture(vec![
            row(1, "00 bb 01 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(1, "00 b7 33 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(2, "00 bb 03 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(2, "00 b7 32 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(3, "00 b9 02 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(4, "00 b9 03 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(4, "00 bb 03 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(4, "00 10 30 2e 30 31 2e 30 32 32 2e 30 30 30 2e 2e"),
            row(4, "00 b7 32 02 00 00 00 00 00 00 00 00 00 00 00 00"),
            row(4, "00 b0 03 02 32 03 00 00 32 32 2e 30 30 30 2e 2e"),
            row(4, "00 20 20 00 01 14 86 05 40 00 01 14 86 05 7d 00"),
            row(4, "00 a6 00 00 46 6c 61 74 00 00 00 00 00 00 00 00"),
        ]);
        let f = analyze(&cap);
        let c = f.iter().find(|x| x.kind == FindingKind::Charge).expect("carga");
        assert_eq!((c.tag, c.offset), (Some(0xbb), 2));
        let p = f.iter().find(|x| x.kind == FindingKind::Power).expect("apagado");
        assert_eq!((p.tag, p.offset, p.values[0].clone()), (Some(0xb9), 2, vec![2]));
        let options = battery_options(&f);
        assert_eq!(options.iter().map(|o| o.value).collect::<Vec<_>>(), vec![50], "solo el porcentaje real");
    }

    /// Captura real de un ratón: bytes invertidos, su nombre dentro del mensaje, el receptor avisa
    /// de que perdió el enlace al apagarlo y también al conectar el cable. El 53 es un candidato: lo
    /// confirma el usuario, porque un valor que no cambia nunca se puede dar por batería.
    #[test]
    fn real_capture_with_inverted_bytes_and_a_name_in_the_message() {
        use crate::listen::Value;
        use crate::reading::Charge;
        const CABLE: &str = "13 1d fe fe fe ca ff fc fe d6 b3 c8 df aa 93 8b 8d 9e ff*47";
        const LINK: &str = "13 1d fe fe ff ca ff fc fe d6 b3 c8 df aa 93 8b 8d 9e ff*47";
        const LOST: &str = "13 1d fe fd ff*61";
        const OTHER: &str = "13 1a 98 ff*62";
        let cap = capture(vec![
            sample(1, CABLE),
            sample(1, LOST),
            sample(2, LINK),
            sample(2, OTHER),
            sample(3, LOST),
            sample(4, LINK),
            sample(4, OTHER),
        ]);
        let f = analyze(&cap);
        let options = battery_options(&f);
        assert_eq!(options.iter().map(|o| o.value).collect::<Vec<_>>(), vec![53], "ni el tipo de mensaje ni el nombre");
        let c = f.iter().find(|x| x.kind == FindingKind::Charge).expect("carga");
        assert_eq!((c.tag, c.offset), (Some(0x1d), 4));
        let p = f.iter().find(|x| x.kind == FindingKind::Power).expect("apagado");
        assert_eq!((p.tag, p.offset, p.values[0].clone()), (Some(0x1d), 3, vec![0xfd]));

        let batteries: Vec<&Finding> = options[0].members.iter().map(|&i| &f[i]).collect();
        let profile = build_profile(&[iface(0xff01)], Some(c), &batteries, Some(p)).expect("perfil");
        let read = |hex: &str| {
            let r = bytes::parse(hex).unwrap();
            profile.signals.iter().filter_map(|s| s.read(&r)).collect::<Vec<_>>()
        };
        assert!(read(CABLE).contains(&Value::Charge(Charge::Charging)));
        assert!(read(CABLE).contains(&Value::Battery(53)));
        assert!(read(LINK).contains(&Value::Charge(Charge::Discharging)));
        assert!(read(LOST).contains(&Value::Power { off: true }));
        assert!(!read(LOST).iter().any(|v| matches!(v, Value::Battery(_))), "el relleno no es un 0 %");
        assert!(read(OTHER).is_empty());
        // si ningún valor coincide, el perfil funciona igual con la carga y el apagado
        assert!(build_profile(&[iface(0xff01)], Some(c), &[], Some(p)).is_some());
    }
}
