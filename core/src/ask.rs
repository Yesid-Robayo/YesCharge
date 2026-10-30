//! Modo "preguntar": OPCIONAL, por dispositivo y solo con la aceptación expresa del usuario.
//!
//! Es el ÚNICO módulo que envía datos a un dispositivo. Muchos modelos 2.4 GHz solo dicen su
//! batería si se les pregunta, y la pregunta es distinta en cada uno, así que no existe una forma
//! genérica de hacerlo. Reglas, todas para que sea lo más seguro posible:
//!
//! - Solo se envían las peticiones de este catálogo, compiladas en el exe. `config.json` solo guarda
//!   el id: editando archivos no se puede hacer que la app envíe otra cosa.
//! - NUNCA se prueban comandos con un dispositivo desconocido: sin una entrada exacta aquí, no se
//!   envía nada. Probar a ciegas es lo que puede cambiar ajustes o dejar un dispositivo inservible.
//! - Cada petición es solo para el VID:PID e interfaz del fabricante para los que se comprobó.
//! - Como mucho una consulta cada `MIN_GAP` (y nunca con el PC inactivo, ver `monitor`), con un
//!   máximo de envíos por consulta. Lo que no cabe en el reporte se rechaza, no se recorta.
//! - Tras varios errores seguidos, `monitor` deja de preguntar hasta que el usuario lo pida.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::HumanInterfaceDevice::{HidD_GetFeature, HidD_SetFeature};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GetLastError, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use crate::hid::{self, HidDevice, HidError, HidInterface};
use crate::listen::{ListenProfile, Signal, SignalKind};
use crate::reading::{Charge, Presence, RawReading};

/// Separación mínima entre dos consultas a un mismo dispositivo.
pub const MIN_GAP: Duration = Duration::from_secs(60);
/// Envíos permitidos en una consulta.
const MAX_SENDS: u32 = 16;
/// Lecturas permitidas en una consulta.
const MAX_READS: u32 = 96;

/// Interfaz exacta para la que se comprobó una petición.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
pub enum Step {
    Write(&'static [u8]),
    SetFeature(&'static [u8]),
    GetFeature {
        report_id: u8,
        len: usize,
    },
    /// Lee hasta `max_reads` input reports y se queda con el primero que sea la respuesta.
    Read {
        timeout_ms: u32,
        max_reads: u32,
    },
    /// Descarta los input reports pendientes.
    Drain {
        timeout_ms: u32,
        max: u32,
    },
    Sleep(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Data {
        battery: Option<u8>,
        charge: Charge,
    },
    Off,
    /// No es la respuesta a la pregunta (otro mensaje, o está dormido).
    NotThis,
}

/// Aviso que el dispositivo envía por su cuenta. Solo se escucha: nunca se le pide nada.
#[derive(Debug, Clone, Copy)]
pub struct Event {
    usage_page: u16,
    report_id: u8,
    tag: u8,
    len: usize,
    at: usize,
    kind: EventKind,
}

#[derive(Debug, Clone, Copy)]
enum EventKind {
    Battery { xor: u8 },
    Charging(&'static [u8]),
    Off(&'static [u8]),
}

/// Una petición de batería comprobada para un modelo.
#[derive(Debug)]
pub struct Query {
    pub id: &'static str,
    pub name: &'static str,
    /// Avisos conocidos: dan el cable y el apagado al instante, sin esperar a la próxima pregunta.
    events: &'static [Event],
    pub targets: &'static [Target],
    /// VID:PID con que aparece por USB al conectarle el cable estando apagado. Verlo basta para
    /// saber que está cargando, sin enviarle nada.
    on_cable: &'static [(u16, u16)],
    /// Por cable responde si está cargando o completo, aunque su porcentaje no sirva.
    state_on_cable: bool,
    /// VID:PID del receptor cuando no hay nada al otro lado. Verlo basta para saber que el
    /// dispositivo está apagado, sin enviarle nada.
    receiver_alone: &'static [(u16, u16)],
    /// Consulta al receptor, antes de nada: dice si el dispositivo está al otro lado.
    status: Option<Status>,
    /// Pasos que se ejecutan una sola vez antes de preguntar.
    prepare: &'static [Step],
    request: &'static [Step],
    attempts: u32,
    /// Respuestas a reunir antes de decidir el estado de carga (votación por mayoría).
    collect: u32,
    parse: fn(&[u8]) -> Answer,
}

/// Pregunta al receptor. La responde él mismo, sin molestar al dispositivo.
#[derive(Debug)]
pub struct Status {
    steps: &'static [Step],
    /// `Data` si se le puede preguntar; `NotThis` si no está disponible.
    parse: fn(&[u8]) -> Answer,
}

const fn target(vid: u16, pid: u16, usage_page: u16) -> Target {
    Target { vid, pid, usage_page, usage: None }
}

fn arctis(r: &[u8]) -> Answer {
    if r.len() < 6 || r[1] != 0xb0 {
        return Answer::NotThis;
    }
    if r[2] == 0x02 {
        return Answer::Off;
    }
    if r[4] > 100 {
        return Answer::NotThis;
    }
    let charge = match r[5] {
        0x01 => Charge::Charging,
        0x02 => Charge::Full,
        _ => Charge::Discharging,
    };
    Answer::Data { battery: Some(r[4]), charge }
}

fn mchose(r: &[u8]) -> Answer {
    if r.len() < 13 || r[2..12].iter().all(|b| *b == 0xff) || r[2..].iter().all(|b| *b == 0) {
        return Answer::NotThis; // dormido: responde vacío
    }
    let battery = r[11] ^ 0xff;
    // sin enlace, el receptor responde con ese byte vacío: no es que esté al 0 %
    if battery == 0 || battery > 100 {
        return Answer::NotThis;
    }
    let charge = match r[12] ^ 0xff {
        1 => Charge::Charging,
        2 => Charge::Full,
        _ => Charge::Discharging,
    };
    Answer::Data { battery: Some(battery), charge }
}

/// `00 <respuesta lista> <batería> 00 <dormido> ...`. Apagado o dormido, el receptor lo indica sin
/// distinguir cuál de los dos, y aun así sigue entregando la última respuesta guardada: esto es lo
/// único fiable. Al perder o recuperar el enlace, la batería llega a 0 un momento.
fn mars68_status(r: &[u8]) -> Answer {
    if r.len() < 5 || r[4] == 1 || !(1..=100).contains(&r[2]) {
        return Answer::NotThis;
    }
    Answer::Data { battery: Some(r[2]), charge: Charge::Unknown }
}

fn mars68(r: &[u8]) -> Answer {
    if r.len() < 5 || r[1] != 0x82 || r[4] != 0x0a || !(1..=100).contains(&r[2]) {
        return Answer::NotThis;
    }
    let charge = if r[3] == 1 { Charge::Charging } else { Charge::Discharging };
    Answer::Data { battery: Some(r[2]), charge }
}

fn gamesir(r: &[u8]) -> Answer {
    if r.len() < 20 || r[1] != 0x01 {
        return Answer::NotThis;
    }
    match r[19] {
        0xff => Answer::Data { battery: None, charge: Charge::Charging }, // por cable no da porcentaje
        b => Answer::Data { battery: Some(b.min(100)), charge: Charge::Discharging },
    }
}

const MCHOSE_ASK: [u8; 33] = {
    let mut a = [0xff; 33];
    a[0] = 0x11;
    a[1] = 0xf9;
    a
};

/// Catálogo: cada entrada se comprobó en hardware real. Añadir una exige saber la petición exacta y
/// probarla en un dispositivo de ese modelo (los usuarios pueden enviar su informe, ver `learn`).
pub static CATALOG: &[Query] = &[
    Query {
        id: "steelseries-arctis-nova-5",
        name: "SteelSeries Arctis Nova 5",
        events: &[
            Event { usage_page: 0xff00, report_id: 0, tag: 0xbb, len: 65, at: 2, kind: EventKind::Charging(&[0x01]) },
            Event { usage_page: 0xff00, report_id: 0, tag: 0xb9, len: 65, at: 2, kind: EventKind::Off(&[0x02]) },
            Event { usage_page: 0xff00, report_id: 0, tag: 0xb7, len: 65, at: 2, kind: EventKind::Battery { xor: 0 } },
        ],
        targets: &[target(0x1038, 0x2232, 0xffc0)],
        on_cable: &[],
        state_on_cable: false,
        receiver_alone: &[],
        status: None,
        prepare: &[],
        request: &[Step::Write(&[0x00, 0xb0]), Step::Read { timeout_ms: 1000, max_reads: 8 }],
        attempts: 1,
        collect: 1,
        parse: arctis,
    },
    Query {
        id: "mchose-l7-ultra-plus",
        name: "MCHOSE L7 Ultra+",
        // el receptor avisa de que perdió el enlace al apagarlo, y también al conectar el cable
        events: &[
            Event {
                usage_page: 0xff01,
                report_id: 0x13,
                tag: 0x1d,
                len: 65,
                at: 4,
                kind: EventKind::Charging(&[0xfe]),
            },
            Event { usage_page: 0xff01, report_id: 0x13, tag: 0x1d, len: 65, at: 3, kind: EventKind::Off(&[0xfd]) },
            Event {
                usage_page: 0xff01,
                report_id: 0x13,
                tag: 0x1d,
                len: 65,
                at: 5,
                kind: EventKind::Battery { xor: 0xff },
            },
        ],
        // por cable sigue dando el último porcentaje sin actualizarlo: solo vale su estado de carga
        targets: &[target(0x5253, 0x1020, 0xff01), target(0x3837, 0x4016, 0xff01)],
        on_cable: &[(0x3837, 0x4016)],
        state_on_cable: true,
        receiver_alone: &[],
        status: None,
        prepare: &[],
        request: &[Step::SetFeature(&MCHOSE_ASK), Step::Sleep(60), Step::GetFeature { report_id: 0x11, len: 64 }],
        attempts: 8,
        collect: 1,
        parse: mchose,
    },
    Query {
        id: "carotmas-mars68",
        name: "Mars68",
        events: &[],
        targets: &[Target { vid: 0x3151, pid: 0x502b, usage_page: 0xffff, usage: Some(0x0002) }],
        on_cable: &[(0x3151, 0x5030)],
        state_on_cable: false,
        receiver_alone: &[],
        status: Some(Status {
            steps: &[
                Step::SetFeature(&[0x00, 0xf7, 0, 0, 0, 0, 0, 0, 0x08]),
                Step::Sleep(30),
                Step::GetFeature { report_id: 0, len: 65 },
            ],
            parse: mars68_status,
        }),
        prepare: &[
            Step::SetFeature(&[0x00, 0x82, 0, 0, 0, 0, 0, 0, 0x7d]),
            Step::Sleep(30),
            Step::GetFeature { report_id: 0, len: 65 },
        ],
        request: &[
            Step::SetFeature(&[0x00, 0xfc, 0x0a, 0, 0, 0, 0, 0, 0xf9]),
            Step::Sleep(50),
            Step::GetFeature { report_id: 0, len: 65 },
        ],
        attempts: 6,
        collect: 3,
        parse: mars68,
    },
    Query {
        id: "gamesir-nova-2-lite",
        name: "GameSir Nova 2 Lite",
        events: &[],
        // inalámbrico es 1098; con cable, 100f; apagado solo queda el receptor, 2106
        targets: &[target(0x3537, 0x1098, 0xff7a), target(0x3537, 0x100f, 0xff7a)],
        on_cable: &[(0x3537, 0x100f)],
        state_on_cable: false,
        receiver_alone: &[(0x3537, 0x2106)],
        status: None,
        prepare: &[],
        request: &[
            Step::Drain { timeout_ms: 120, max: 32 },
            Step::Write(&[0x00, 0x01, 0x01]),
            Step::Read { timeout_ms: 500, max_reads: 8 },
        ],
        attempts: 1,
        collect: 1,
        parse: gamesir,
    },
];

pub fn find(id: &str) -> Option<&'static Query> {
    CATALOG.iter().find(|q| q.id == id)
}

/// Petición comprobada para un VID:PID, si existe.
pub fn for_device(vid: u16, pid: u16) -> Option<&'static Query> {
    CATALOG.iter().find(|q| q.targets.iter().any(|t| t.vid == vid && t.pid == pid))
}

/// Conectado por cable no da su porcentaje actual, aunque un aviso diga que está cargando.
pub fn no_percentage_on_cable(q: &Query) -> bool {
    !q.on_cable.is_empty()
}

/// Todos los VID:PID con que puede aparecer este modelo.
pub fn usb_ids(q: &Query) -> impl Iterator<Item = (u16, u16)> + '_ {
    q.targets.iter().map(|t| (t.vid, t.pid)).chain(q.on_cable.iter().chain(q.receiver_alone).copied())
}

/// Perfil de escucha con los avisos conocidos del modelo. `None` si no tiene.
pub fn event_profile(q: &Query) -> Option<ListenProfile> {
    let target = q.targets.first()?;
    let signals: Vec<Signal> = q
        .events
        .iter()
        .map(|e| Signal {
            usage_page: e.usage_page,
            usage: target.usage.unwrap_or(0x0001),
            report_id: e.report_id,
            tag: Some(e.tag),
            min_len: e.len,
            at: e.at,
            kind: match e.kind {
                EventKind::Battery { xor } => SignalKind::Battery { xor },
                EventKind::Charging(v) => SignalKind::Charge { charging: v.to_vec() },
                EventKind::Off(v) => SignalKind::Power { off: v.to_vec() },
            },
        })
        .collect();
    (!signals.is_empty()).then_some(ListenProfile { vid: target.vid, pid: target.pid, signals, container: None })
}

/// Operaciones con el dispositivo. La implementa `Device` y el simulado de las pruebas.
pub trait Link {
    fn write(&mut self, data: &[u8]) -> Result<(), HidError>;
    fn read(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, HidError>;
    fn set_feature(&mut self, data: &[u8]) -> Result<(), HidError>;
    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, HidError>;
    fn sleep(&mut self, ms: u32);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Data {
        battery: Option<u8>,
        charge: Charge,
    },
    Off,
    /// No respondió lo esperado: dormido o fuera de alcance.
    NoResponse,
    Failed(HidError),
}

/// Ejecuta los pasos y devuelve la última respuesta útil.
fn run_steps(q: &Query, steps: &[Step], l: &mut dyn Link) -> Result<Option<Vec<u8>>, HidError> {
    let mut last = None;
    for step in steps {
        match *step {
            Step::Write(data) => l.write(data)?,
            Step::SetFeature(data) => l.set_feature(data)?,
            Step::GetFeature { report_id, len } => last = Some(l.get_feature(report_id, len)?),
            Step::Read { timeout_ms, max_reads } => {
                last = None;
                for _ in 0..max_reads {
                    match l.read(timeout_ms)? {
                        Some(r) if (q.parse)(&r) != Answer::NotThis => {
                            last = Some(r);
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            }
            Step::Drain { timeout_ms, max } => {
                for _ in 0..max {
                    if l.read(timeout_ms)?.is_none() {
                        break;
                    }
                }
            }
            Step::Sleep(ms) => l.sleep(ms),
        }
    }
    Ok(last)
}

/// Estado más votado; en empate gana "en batería" (no anunciar una carga dudosa).
fn majority(votes: &[Charge]) -> Charge {
    let mut best = Charge::Discharging;
    let mut best_n = 0;
    for c in [Charge::Discharging, Charge::Charging, Charge::Full] {
        let n = votes.iter().filter(|v| **v == c).count();
        if n > best_n {
            best = c;
            best_n = n;
        }
    }
    best
}

pub fn execute(q: &Query, l: &mut dyn Link) -> Outcome {
    let mut battery = None;
    if let Some(status) = &q.status {
        match run_steps(q, status.steps, l).map(|r| r.map(|r| (status.parse)(&r))) {
            Ok(Some(Answer::Off)) => return Outcome::Off,
            Ok(Some(Answer::Data { battery: b, .. })) => battery = b,
            Ok(_) => return Outcome::NoResponse,
            Err(e) => return Outcome::Failed(e),
        }
    }
    if let Err(e) = run_steps(q, q.prepare, l) {
        return Outcome::Failed(e);
    }
    let mut votes = Vec::new();
    for _ in 0..q.attempts {
        let response = match run_steps(q, q.request, l) {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e),
        };
        match response.map(|r| (q.parse)(&r)) {
            Some(Answer::Off) => return Outcome::Off,
            Some(Answer::Data { battery: b, charge }) => {
                battery = battery.or(b);
                votes.push(charge);
                if votes.len() as u32 >= q.collect {
                    break;
                }
            }
            Some(Answer::NotThis) | None => {}
        }
    }
    if votes.is_empty() {
        // el receptor ya dio la batería: solo falta saber si carga
        return battery.map_or(Outcome::NoResponse, |b| Outcome::Data { battery: Some(b), charge: Charge::Unknown });
    }
    Outcome::Data { battery, charge: majority(&votes) }
}

/// Dispositivo real, con presupuesto de operaciones por consulta.
struct Device<'a> {
    dev: &'a HidDevice,
    sends: u32,
    reads: u32,
}

impl Device<'_> {
    fn spend_send(&mut self) -> Result<(), HidError> {
        self.sends += 1;
        if self.sends > MAX_SENDS { Err(HidError::Blocked("demasiados envíos")) } else { Ok(()) }
    }

    fn spend_read(&mut self) -> Result<(), HidError> {
        self.reads += 1;
        if self.reads > MAX_READS { Err(HidError::Blocked("demasiadas lecturas")) } else { Ok(()) }
    }

    /// Copia `data` a un reporte del largo que declara el dispositivo. Nunca recorta.
    fn padded(data: &[u8], len: usize) -> Result<Vec<u8>, HidError> {
        if len == 0 {
            return Err(HidError::Unsupported);
        }
        if data.len() > len {
            return Err(HidError::Blocked("no cabe en el reporte"));
        }
        let mut buf = vec![0u8; len];
        buf[..data.len()].copy_from_slice(data);
        Ok(buf)
    }
}

impl Link for Device<'_> {
    fn write(&mut self, data: &[u8]) -> Result<(), HidError> {
        self.spend_send()?;
        let buf = Self::padded(data, self.dev.info.output_len as usize)?;
        let (handle, event) = self.dev.raw_handles();
        unsafe {
            let mut ov: OVERLAPPED = zeroed();
            ov.hEvent = event;
            if WriteFile(handle, buf.as_ptr(), buf.len() as u32, null_mut(), &mut ov) == 0
                && GetLastError() != ERROR_IO_PENDING
            {
                return Err(hid::last_error());
            }
            let mut n = 0u32;
            if GetOverlappedResultEx(handle, &ov, &mut n, 1000, 0) != 0 {
                return Ok(());
            }
            let code = GetLastError();
            CancelIoEx(handle, &ov);
            // esperar a que la cancelación termine antes de soltar el buffer
            GetOverlappedResult(handle, &ov, &mut n, 1);
            Err(HidError::Io(code))
        }
    }

    fn read(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, HidError> {
        self.spend_read()?;
        self.dev.read(timeout_ms)
    }

    fn set_feature(&mut self, data: &[u8]) -> Result<(), HidError> {
        self.spend_send()?;
        let buf = Self::padded(data, self.dev.info.feature_len as usize)?;
        let (handle, _) = self.dev.raw_handles();
        unsafe {
            if HidD_SetFeature(handle, buf.as_ptr() as *const c_void, buf.len() as u32) {
                Ok(())
            } else {
                Err(hid::last_error())
            }
        }
    }

    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, HidError> {
        self.spend_read()?;
        if self.dev.info.feature_len == 0 {
            return Err(HidError::Unsupported);
        }
        let mut buf = vec![0u8; (self.dev.info.feature_len as usize).max(len)];
        buf[0] = report_id;
        let (handle, _) = self.dev.raw_handles();
        unsafe {
            if HidD_GetFeature(handle, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) {
                Ok(buf)
            } else {
                Err(hid::last_error())
            }
        }
    }

    fn sleep(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
}

fn last_asked() -> &'static Mutex<HashMap<String, Instant>> {
    static MAP: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// El límite de `MIN_GAP` es por aparato, no por modelo.
fn asked_key(q: &Query, container: Option<&str>) -> String {
    format!("{}/{}", q.id, container.unwrap_or(""))
}

/// Segundos que faltan para poder volver a preguntar, redondeados hacia arriba.
pub fn wait_secs(q: &Query, container: Option<&str>) -> u64 {
    let map = last_asked().lock().unwrap_or_else(|e| e.into_inner());
    map.get(&asked_key(q, container)).map_or(0, |t| MIN_GAP.saturating_sub(t.elapsed()).as_secs_f64().ceil() as u64)
}

fn asked_within(q: &Query, container: Option<&str>, gap: Duration) -> bool {
    let map = last_asked().lock().unwrap_or_else(|e| e.into_inner());
    map.get(&asked_key(q, container)).is_some_and(|t| t.elapsed() < gap)
}

/// Evita que dos procesos de YesCharge mezclen sus conversaciones con un dispositivo.
fn with_lock<T>(f: impl FnOnce() -> T) -> Option<T> {
    unsafe {
        let name = hid::wide("Local\\YesCharge.Ask");
        let mutex = CreateMutexW(null(), 0, name.as_ptr());
        if mutex.is_null() {
            return None;
        }
        let wait = WaitForSingleObject(mutex, 5000);
        let result = if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            let value = f();
            ReleaseMutex(mutex);
            Some(value)
        } else {
            None
        };
        CloseHandle(mutex);
        result
    }
}

fn find_interface<'a>(t: &Target, all: &'a [HidInterface], container: Option<&str>) -> Option<&'a HidInterface> {
    let found = all.iter().filter(|i| {
        i.vid == t.vid
            && i.pid == t.pid
            && i.usage_page == t.usage_page
            && t.usage.is_none_or(|u| u == i.usage)
            && i.is_vendor_defined()
    });
    hid::pick(found, container)
}

fn is_connected(ids: &[(u16, u16)], all: &[HidInterface]) -> bool {
    ids.iter().any(|&(vid, pid)| all.iter().any(|i| i.vid == vid && i.pid == pid))
}

fn charging_on_cable() -> RawReading {
    RawReading {
        presence: Presence::Online,
        charge: Charge::Charging,
        no_percentage: true,
        heard_only: true,
        detail: "conectado por cable".into(),
        ..Default::default()
    }
}

/// Pregunta la batería a un dispositivo del catálogo. Si se preguntó hace menos de `every` (nunca
/// menos de `MIN_GAP`), no envía nada y devuelve `Presence::Stale`. Con `None` tampoco envía nada:
/// solo mira lo que se distingue por los dispositivos que ve Windows.
pub fn read(id: &str, container: Option<&str>, every: Option<Duration>) -> RawReading {
    let Some(q) = find(id) else {
        return RawReading::absent(Presence::Error, "petición desconocida");
    };
    let all = hid::enumerate();
    let on_cable = is_connected(q.on_cable, &all);
    if on_cable && !q.state_on_cable {
        return charging_on_cable();
    }
    let Some(every) = every else {
        let linked = q.targets.iter().any(|t| find_interface(t, &all, container).is_some());
        return if !linked && is_connected(q.receiver_alone, &all) {
            RawReading { heard_only: true, ..RawReading::absent(Presence::Off, "apagado") }
        } else {
            RawReading::absent(Presence::Stale, "sin preguntar")
        };
    };
    if asked_within(q, container, every.max(MIN_GAP)) {
        return RawReading::absent(Presence::Stale, "preguntado hace poco");
    }
    let (mut found, mut busy) = (false, false);
    let mut failure = None;
    // con el cable puesto, el receptor ya no tiene nada al otro lado
    for t in q.targets.iter().filter(|t| !on_cable || q.on_cable.contains(&(t.vid, t.pid))) {
        let Some(info) = find_interface(t, &all, container) else { continue };
        found = true;
        let dev = match HidDevice::open_for_query(info) {
            Ok(d) => d,
            Err(HidError::Busy) => {
                busy = true;
                continue;
            }
            Err(e) => {
                failure = Some(e);
                continue;
            }
        };
        last_asked().lock().unwrap_or_else(|e| e.into_inner()).insert(asked_key(q, container), Instant::now());
        let outcome = with_lock(|| execute(q, &mut Device { dev: &dev, sends: 0, reads: 0 }));
        match outcome {
            Some(Outcome::Data { battery, charge }) => {
                return RawReading {
                    presence: Presence::Online,
                    battery: battery.filter(|_| !on_cable),
                    charge,
                    approximate: false,
                    no_percentage: on_cable || battery.is_none(),
                    heard_only: false,
                    detail: format!("pregunta {}", q.id),
                };
            }
            Some(Outcome::Off) => return RawReading::absent(Presence::Off, "apagado"),
            Some(Outcome::Failed(e)) => failure = Some(e),
            Some(Outcome::NoResponse) => {}
            None => busy = true,
        }
    }
    if on_cable && failure.is_none() {
        return charging_on_cable();
    }
    match failure {
        Some(HidError::Gone) => RawReading::absent(Presence::NotFound, "desconectado"),
        Some(e) => RawReading::absent(Presence::Error, e.to_string()),
        None if !found && is_connected(q.receiver_alone, &all) => RawReading::absent(Presence::Off, "apagado"),
        None if !found => RawReading::absent(Presence::NotFound, "no detectado"),
        None if busy => RawReading::absent(Presence::Busy, "ocupado"),
        None => RawReading::absent(Presence::Asleep, "sin respuesta"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes;

    /// Dispositivo simulado: responde en orden y anota todo lo que se le envía.
    #[derive(Default)]
    struct Fake {
        responses: Vec<Vec<u8>>,
        sent: Vec<Vec<u8>>,
    }

    impl Fake {
        fn with(responses: &[&str]) -> Self {
            Self { responses: responses.iter().map(|r| bytes::parse(r).unwrap()).collect(), sent: Vec::new() }
        }

        fn next(&mut self) -> Option<Vec<u8>> {
            (!self.responses.is_empty()).then(|| self.responses.remove(0))
        }
    }

    impl Link for Fake {
        fn write(&mut self, data: &[u8]) -> Result<(), HidError> {
            self.sent.push(data.to_vec());
            Ok(())
        }
        fn read(&mut self, _: u32) -> Result<Option<Vec<u8>>, HidError> {
            Ok(self.next())
        }
        fn set_feature(&mut self, data: &[u8]) -> Result<(), HidError> {
            self.sent.push(data.to_vec());
            Ok(())
        }
        fn get_feature(&mut self, _: u8, len: usize) -> Result<Vec<u8>, HidError> {
            Ok(self.next().unwrap_or_else(|| vec![0; len]))
        }
        fn sleep(&mut self, _: u32) {}
    }

    fn query(id: &str) -> &'static Query {
        find(id).expect("está en el catálogo")
    }

    #[test]
    fn the_catalog_only_targets_manufacturer_interfaces() {
        for q in CATALOG {
            assert!(q.targets.iter().all(|t| t.usage_page >= 0xff00), "{} usa una página estándar", q.id);
            let sends = |steps: &[Step]| {
                steps.iter().filter(|s| matches!(s, Step::Write(_) | Step::SetFeature(_))).count() as u32
            };
            assert!(sends(q.prepare) + sends(q.request) * q.attempts <= MAX_SENDS, "{} envía demasiado", q.id);
        }
    }

    #[test]
    fn unknown_devices_have_no_query() {
        assert!(for_device(0x1234, 0x5678).is_none());
        assert!(find("inventado").is_none());
    }

    #[test]
    fn arctis_reads_a_real_response_skipping_other_messages() {
        let mut f = Fake::with(&["00 bb 03 02 00 00", "00 b0 03 02 2f 01 00"]);
        let o = execute(query("steelseries-arctis-nova-5"), &mut f);
        assert_eq!(o, Outcome::Data { battery: Some(47), charge: Charge::Charging });
        assert_eq!(f.sent, vec![vec![0x00, 0xb0]], "solo esa petición");
    }

    #[test]
    fn arctis_says_when_it_is_off() {
        let mut f = Fake::with(&["00 b0 02 02 00 00 00"]);
        assert_eq!(execute(query("steelseries-arctis-nova-5"), &mut f), Outcome::Off);
    }

    #[test]
    fn mouse_inverts_its_bytes_and_retries_while_asleep() {
        let awake = "11 00 ff ff ff ff ff ff ff ff ff cd fe ff";
        let mut f = Fake::with(&["11 00 ff*30", "11 00 00*30", awake]);
        let o = execute(query("mchose-l7-ultra-plus"), &mut f);
        assert_eq!(o, Outcome::Data { battery: Some(50), charge: Charge::Charging });
        assert_eq!(f.sent.len(), 3, "se detiene en cuanto responde");
        assert_eq!(f.sent[0][..3], [0x11, 0xf9, 0xff]);
    }

    #[test]
    fn asleep_is_no_response_not_an_error() {
        let mut f = Fake::default();
        assert_eq!(execute(query("mchose-l7-ultra-plus"), &mut f), Outcome::NoResponse);
    }

    const STATUS_AWAKE: &str = "00 00 48 00 00 01 01 01";

    #[test]
    fn keyboard_decides_charge_by_majority() {
        let frame = |c| format!("00 82 48 {c} 0a");
        let mut f = Fake::with(&[STATUS_AWAKE, "00", &frame("01"), &frame("00"), &frame("00")]);
        let o = execute(query("carotmas-mars68"), &mut f);
        assert_eq!(o, Outcome::Data { battery: Some(72), charge: Charge::Discharging });
    }

    #[test]
    fn keyboard_charging_is_read_from_its_answer() {
        let frame = "00 82 49 01 0a";
        let mut f = Fake::with(&["00 00 49 00 00 01 01 01", "00", "00", frame, frame, frame]);
        let o = execute(query("carotmas-mars68"), &mut f);
        assert_eq!(o, Outcome::Data { battery: Some(73), charge: Charge::Charging });
    }

    #[test]
    fn keyboard_plugged_in_while_off_shows_up_as_another_usb_device() {
        let iface = |vid, pid| HidInterface {
            path: String::new(),
            vid,
            pid,
            version: 0,
            usage_page: 0xffff,
            usage: 2,
            input_len: 0,
            output_len: 0,
            feature_len: 65,
            product: String::new(),
            manufacturer: String::new(),
        };
        let q = query("carotmas-mars68");
        assert!(!is_connected(q.on_cable, &[iface(0x3151, 0x502b)]));
        assert!(is_connected(q.on_cable, &[iface(0x3151, 0x502b), iface(0x3151, 0x5030)]));
    }

    #[test]
    fn an_added_model_is_known_by_all_its_usb_ids() {
        let source = crate::config::Source::Ask {
            query: "carotmas-mars68".into(),
            accepted_at: 1,
            notices: None,
            container: None,
        };
        assert_eq!(source.device_keys(), ["usb:3151:502b", "usb:3151:5030"]);
        let source = crate::config::Source::Ask {
            query: "gamesir-nova-2-lite".into(),
            accepted_at: 1,
            notices: None,
            container: None,
        };
        assert_eq!(source.device_keys(), ["usb:3537:1098", "usb:3537:100f", "usb:3537:100f", "usb:3537:2106"]);
    }

    #[test]
    fn mouse_receiver_without_link_is_not_zero_percent() {
        let mut r = bytes::parse("11 f9 c8 c7 e9 bf dd fa fa ff ff ff ff").unwrap();
        assert_eq!(mchose(&r), Answer::NotThis);
        r[11] = 0xcc;
        assert_eq!(mchose(&r), Answer::Data { battery: Some(51), charge: Charge::Discharging });
    }

    #[test]
    fn keyboard_out_of_reach_ignores_the_saved_answer() {
        for status in ["00 00 3d 00 01 01 01 01", "00 01 00 00 00 01 01 01"] {
            let mut f = Fake::with(&[status, "00 82 3a 00 0a"]);
            assert_eq!(execute(query("carotmas-mars68"), &mut f), Outcome::NoResponse);
            assert_eq!(f.sent.len(), 1, "sin enlace no se le pregunta nada más");
        }
    }

    #[test]
    fn gamepad_on_cable_charges_without_percentage() {
        let mut r = bytes::parse("00 01 00*18").unwrap();
        r[19] = 0xff;
        assert_eq!(gamesir(&r), Answer::Data { battery: None, charge: Charge::Charging });
        r[19] = 18;
        assert_eq!(gamesir(&r), Answer::Data { battery: Some(18), charge: Charge::Discharging });
        r[1] = 0xf4;
        assert_eq!(gamesir(&r), Answer::NotThis, "las tramas de juego no son la respuesta");
    }

    #[test]
    fn a_failure_stops_the_conversation() {
        struct Broken;
        impl Link for Broken {
            fn write(&mut self, _: &[u8]) -> Result<(), HidError> {
                Err(HidError::Gone)
            }
            fn read(&mut self, _: u32) -> Result<Option<Vec<u8>>, HidError> {
                panic!("no debe seguir tras el fallo")
            }
            fn set_feature(&mut self, _: &[u8]) -> Result<(), HidError> {
                Err(HidError::Gone)
            }
            fn get_feature(&mut self, _: u8, _: usize) -> Result<Vec<u8>, HidError> {
                panic!("no debe seguir tras el fallo")
            }
            fn sleep(&mut self, _: u32) {}
        }
        let o = execute(query("steelseries-arctis-nova-5"), &mut Broken);
        assert_eq!(o, Outcome::Failed(HidError::Gone));
    }

    #[test]
    fn a_device_matches_at_most_one_query() {
        for q in CATALOG {
            for t in q.targets {
                let n = CATALOG.iter().filter(|o| o.targets.iter().any(|x| x.vid == t.vid && x.pid == t.pid)).count();
                assert_eq!(n, 1, "{:04x}:{:04x} está repetido", t.vid, t.pid);
            }
        }
    }

    fn event_values(id: &str, hex: &str) -> Vec<crate::listen::Value> {
        let profile = event_profile(query(id)).expect("tiene avisos");
        let r = bytes::parse(hex).unwrap();
        profile.signals.iter().filter_map(|s| s.read(&r)).collect()
    }

    /// Mensajes reales capturados al conectar y quitar el cable y al apagar y encender.
    #[test]
    fn real_events_of_the_mouse() {
        use crate::listen::Value;
        const ID: &str = "mchose-l7-ultra-plus";
        let cable = event_values(ID, "13 1d fe fe fe ca ff fc fe d6 b3 c8 df aa 93 8b 8d 9e ff*47");
        assert!(cable.contains(&Value::Charge(Charge::Charging)) && cable.contains(&Value::Battery(53)));
        let link = event_values(ID, "13 1d fe fe ff ca ff fc fe d6 b3 c8 df aa 93 8b 8d 9e ff*47");
        assert!(link.contains(&Value::Charge(Charge::Discharging)) && link.contains(&Value::Power { off: false }));
        assert!(event_values(ID, "13 1d fe fd ff*61").contains(&Value::Power { off: true }));
        assert!(event_values(ID, "13 1a 98 ff*62").is_empty());
    }

    #[test]
    fn real_events_of_the_headset() {
        use crate::listen::Value;
        const ID: &str = "steelseries-arctis-nova-5";
        assert_eq!(event_values(ID, "00 bb 01 02 00*61"), vec![Value::Charge(Charge::Charging)]);
        assert_eq!(event_values(ID, "00 bb 03 02 00*61"), vec![Value::Charge(Charge::Discharging)]);
        assert_eq!(event_values(ID, "00 b9 02 02 00*61"), vec![Value::Power { off: true }]);
        assert_eq!(event_values(ID, "00 b9 03 02 00*61"), vec![Value::Power { off: false }]);
        assert_eq!(event_values(ID, "00 b7 2e 02 00*61"), vec![Value::Battery(46)]);
    }

    #[test]
    fn models_without_known_events_have_no_listening() {
        assert!(event_profile(query("gamesir-nova-2-lite")).is_none());
    }
}
