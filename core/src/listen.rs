//! Perfiles de ESCUCHA creados por el usuario con el asistente.
//!
//! Sirven para dispositivos que envían su estado por su cuenta. Las interfaces se abren siempre
//! sin derecho de escritura, así que con un perfil de escucha es imposible enviar nada al
//! dispositivo, aunque alguien edite `config.json` a mano.
//!
//! Un dispositivo puede contar cada cosa en un mensaje distinto, e incluso por interfaces
//! distintas: por eso un perfil es una lista de señales, no un único reporte.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::hid::HidDevice;
use crate::reading::{Charge, Presence, RawReading};

/// Un dato que viene en un tipo de mensaje concreto de una interfaz concreta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    pub usage_page: u16,
    pub usage: u16,
    /// Primer byte del reporte.
    pub report_id: u8,
    /// Tipo de mensaje (byte 1). Un mismo report id puede traer mensajes con otro significado.
    #[serde(default)]
    pub tag: Option<u8>,
    pub min_len: usize,
    /// Posición del dato dentro del reporte.
    pub at: usize,
    pub kind: SignalKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalKind {
    Battery {
        #[serde(default)]
        xor: u8,
    },
    Charge {
        /// Valores que significan "cargando"; cualquier otro, "en batería".
        charging: Vec<u8>,
    },
    Power {
        /// Valores que significan "apagado"; cualquier otro, "encendido".
        off: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Battery(u8),
    Charge(Charge),
    Power { off: bool },
}

impl Signal {
    /// Interpreta un reporte; `None` si no es este mensaje o el valor es imposible.
    pub fn read(&self, r: &[u8]) -> Option<Value> {
        if r.len() < self.min_len
            || r.first() != Some(&self.report_id)
            || !self.tag.is_none_or(|t| r.get(1) == Some(&t))
        {
            return None;
        }
        let v = *r.get(self.at)?;
        match &self.kind {
            // un 0 es relleno: con 0 % real el dispositivo ya estaría apagado
            SignalKind::Battery { xor } => Some(v ^ xor).filter(|p| (1..=100).contains(p)).map(Value::Battery),
            SignalKind::Charge { charging } => {
                Some(Value::Charge(if charging.contains(&v) { Charge::Charging } else { Charge::Discharging }))
            }
            SignalKind::Power { off } => Some(Value::Power { off: off.contains(&v) }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenProfile {
    pub vid: u16,
    pub pid: u16,
    pub signals: Vec<Signal>,
}

/// Un mensaje cuenta como reciente este tiempo; después se muestra como "último dato".
const FRESH_S: f64 = 120.0;

/// Lo último que se oyó de cada dato, y cuándo.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Heard {
    battery: Option<(u8, f64)>,
    charge: Option<(Charge, f64)>,
    off: bool,
}

impl Heard {
    /// Aplica un reporte. Devuelve `true` si cambió algún valor.
    fn apply(&mut self, signals: &[Signal], r: &[u8], now: f64) -> bool {
        let values: Vec<Value> = signals.iter().filter_map(|s| s.read(r)).collect();
        if values.is_empty() {
            return false;
        }
        // cualquier otro mensaje del dispositivo significa que está encendido
        let off = values.iter().any(|v| matches!(v, Value::Power { off: true }));
        let mut changed = off != self.off;
        self.off = off;
        if off {
            return changed; // lo demás que traiga ese mensaje no es una medición real
        }
        for v in values {
            match v {
                Value::Battery(b) => {
                    changed |= self.battery.map(|x| x.0) != Some(b);
                    self.battery = Some((b, now));
                }
                Value::Charge(c) => {
                    changed |= self.charge.map(|x| x.0) != Some(c);
                    self.charge = Some((c, now));
                }
                Value::Power { .. } => {}
            }
        }
        changed
    }

    fn reading(&self, now: f64) -> RawReading {
        // Cargando no cuenta como apagado: al conectar el cable, algunos dispositivos también dejan
        // de hablar con el receptor, y un dispositivo apagado que carga sigue siendo útil saberlo.
        let charging = matches!(self.charge, Some((Charge::Charging, _)));
        if self.off && !charging {
            return RawReading::absent(Presence::Off, "apagado");
        }
        let newest = [self.battery.map(|b| b.1), self.charge.map(|c| c.1)].into_iter().flatten().fold(0.0, f64::max);
        if newest <= 0.0 || now - newest > FRESH_S {
            return RawReading::absent(Presence::Stale, "sin mensajes recientes");
        }
        RawReading {
            presence: Presence::Online,
            battery: self.battery.map(|b| b.0),
            charge: self.charge.map(|c| c.0).unwrap_or_default(),
            approximate: false,
            no_percentage: false,
            heard_only: false,
            detail: "perfil de escucha".into(),
        }
    }
}

/// Lo último que dijo un dispositivo sobre su estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeardState {
    pub off: bool,
    pub charge: Option<Charge>,
}

/// Escucha en marcha de un perfil: un hilo por interfaz, todos sobre el mismo `Heard`.
struct Listening {
    heard: Arc<Mutex<Heard>>,
    alive: Vec<Arc<AtomicBool>>,
    stop: Arc<AtomicBool>,
}

/// Mantiene la escucha de fondo de cada perfil en uso.
#[derive(Default)]
pub struct Hub {
    active: HashMap<String, Listening>,
    /// Se llama cuando un dispositivo informa de un cambio, para no esperar a la próxima consulta.
    on_change: Option<Arc<dyn Fn() + Send + Sync>>,
}

fn key(p: &ListenProfile) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

impl Hub {
    pub fn new(on_change: impl Fn() + Send + Sync + 'static) -> Self {
        Self { active: HashMap::new(), on_change: Some(Arc::new(on_change)) }
    }

    /// Pone en marcha la escucha del perfil si hace falta. `Err` si no se pudo.
    fn ensure(&mut self, p: &ListenProfile, k: &str) -> Result<(), RawReading> {
        let running = self.active.get(k).is_some_and(|l| l.alive.iter().all(|a| a.load(Ordering::Relaxed)));
        if running {
            return Ok(());
        }
        let previous = self.active.remove(k);
        if let Some(old) = &previous {
            old.stop.store(true, Ordering::Relaxed);
        }
        let heard = previous.map(|old| old.heard).unwrap_or_default();
        let listening = self.start(p, heard)?;
        self.active.insert(k.to_string(), listening);
        Ok(())
    }

    fn heard(&self, k: &str) -> Option<Heard> {
        self.active.get(k).and_then(|l| l.heard.lock().ok().map(|h| *h))
    }

    pub fn read(&mut self, p: &ListenProfile) -> RawReading {
        let k = key(p);
        if let Err(reading) = self.ensure(p, &k) {
            return reading;
        }
        match self.heard(&k) {
            Some(h) => h.reading(crate::now_secs()),
            None => RawReading::absent(Presence::Stale, "sin mensajes recientes"),
        }
    }

    /// Anota lo que respondió, por si se perdió algún aviso: lo último que se oyó no debe
    /// contradecir a una respuesta más nueva.
    pub fn mark_answered(&mut self, p: &ListenProfile, charge: Charge) {
        if let Some(l) = self.active.get(&key(p))
            && let Ok(mut h) = l.heard.lock()
        {
            h.off = false;
            if charge != Charge::Unknown {
                h.charge = Some((charge, crate::now_secs()));
            }
        }
    }

    /// Lo último que dijo el dispositivo, aunque haya pasado tiempo. `None` si no se puede escuchar.
    pub fn state(&mut self, p: &ListenProfile) -> Option<HeardState> {
        let k = key(p);
        self.ensure(p, &k).ok()?;
        self.heard(&k).map(|h| HeardState { off: h.off, charge: h.charge.map(|c| c.0) })
    }

    fn start(&self, p: &ListenProfile, heard: Arc<Mutex<Heard>>) -> Result<Listening, RawReading> {
        let all = crate::hid::enumerate();
        let stop = Arc::new(AtomicBool::new(false));
        let mut alive = Vec::new();
        let mut busy = None;
        let wanted: BTreeSet<(u16, u16)> = p.signals.iter().map(|s| (s.usage_page, s.usage)).collect();
        for (page, usage) in wanted {
            let Some(info) =
                all.iter().find(|i| i.vid == p.vid && i.pid == p.pid && i.usage_page == page && i.usage == usage)
            else {
                continue;
            };
            let dev = match HidDevice::open(info) {
                Ok(d) => d,
                Err(e) => {
                    busy = Some(e);
                    continue;
                }
            };
            let signals: Vec<Signal> =
                p.signals.iter().filter(|s| s.usage_page == page && s.usage == usage).cloned().collect();
            let running = Arc::new(AtomicBool::new(true));
            let (h, a, s, on_change) = (heard.clone(), running.clone(), stop.clone(), self.on_change.clone());
            std::thread::spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    match dev.read(1000) {
                        Ok(Some(r)) => {
                            let now = crate::now_secs();
                            let changed = h.lock().unwrap_or_else(|e| e.into_inner()).apply(&signals, &r, now);
                            // solo si cambió: un dispositivo muy hablador no debe provocar un ciclo por mensaje
                            if changed && let Some(f) = &on_change {
                                f();
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
                a.store(false, Ordering::Relaxed);
            });
            alive.push(running);
        }
        if alive.is_empty() {
            return Err(match busy {
                Some(e) => RawReading::absent(Presence::Busy, e.to_string()),
                None => RawReading::absent(Presence::NotFound, "no detectado"),
            });
        }
        Ok(Listening { heard, alive, stop })
    }

    /// Detiene las escuchas que ya no pertenecen a ningún dispositivo configurado.
    pub fn retain(&mut self, in_use: &[&ListenProfile]) {
        let keep: Vec<String> = in_use.iter().map(|p| key(p)).collect();
        self.active.retain(|k, l| {
            let kept = keep.contains(k);
            if !kept {
                l.stop.store(true, Ordering::Relaxed);
            }
            kept
        });
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        for l in self.active.values() {
            l.stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(tag: u8, at: usize, kind: SignalKind) -> Signal {
        Signal { usage_page: 0xff00, usage: 1, report_id: 0, tag: Some(tag), min_len: 4, at, kind }
    }

    fn signals() -> Vec<Signal> {
        vec![signal(0xb7, 2, SignalKind::Battery { xor: 0 }), signal(0xbb, 2, SignalKind::Charge { charging: vec![1] })]
    }

    #[test]
    fn each_signal_reads_only_its_own_message() {
        let s = signals();
        assert_eq!(s[0].read(&[0, 0xb7, 46, 0]), Some(Value::Battery(46)));
        assert_eq!(s[0].read(&[0, 0xbb, 46, 0]), None, "otro tipo de mensaje");
        assert_eq!(s[0].read(&[0, 0xb7, 200, 0]), None, "porcentaje imposible");
        assert_eq!(s[0].read(&[7, 0xb7, 46, 0]), None, "otro report id");
        assert_eq!(s[1].read(&[0, 0xbb, 1, 0]), Some(Value::Charge(Charge::Charging)));
        assert_eq!(s[1].read(&[0, 0xbb, 3, 0]), Some(Value::Charge(Charge::Discharging)));
    }

    #[test]
    fn combines_messages_and_only_flags_real_changes() {
        let s = signals();
        let mut h = Heard::default();
        assert!(h.apply(&s, &[0, 0xb7, 46, 0], 100.0));
        assert!(h.apply(&s, &[0, 0xbb, 1, 0], 110.0), "la carga viene en otro mensaje");
        assert!(!h.apply(&s, &[0, 0xbb, 1, 0], 120.0), "repetir lo mismo no es un cambio");
        let r = h.reading(130.0);
        assert_eq!((r.presence, r.battery, r.charge), (Presence::Online, Some(46), Charge::Charging));
    }

    #[test]
    fn without_recent_messages_it_is_stale() {
        let mut h = Heard::default();
        assert_eq!(h.reading(100.0).presence, Presence::Stale);
        h.apply(&signals(), &[0, 0xb7, 46, 0], 100.0);
        assert_eq!(h.reading(100.0 + FRESH_S + 1.0).presence, Presence::Stale);
    }

    #[test]
    fn learns_it_is_off_and_any_other_message_turns_it_back_on() {
        let mut s = signals();
        s.push(signal(0xb0, 2, SignalKind::Power { off: vec![2] }));
        s.push(signal(0xb0, 4, SignalKind::Battery { xor: 0 }));
        let mut h = Heard::default();
        h.apply(&s, &[0, 0xb7, 46, 0, 0], 100.0);
        // al apagarse, ese mismo mensaje trae un 0 que no es la batería
        assert!(h.apply(&s, &[0, 0xb0, 2, 0, 0], 110.0));
        assert_eq!(h.reading(111.0).presence, Presence::Off);
        assert_eq!(h.battery.map(|b| b.0), Some(46), "no debe creer que bajó a 0 %");
        assert_eq!(h.reading(999.0).presence, Presence::Off, "sigue apagado aunque pase el tiempo");
        assert!(h.apply(&s, &[0, 0xbb, 3, 0, 0], 120.0), "volvió a hablar: está encendido");
        assert_eq!(h.reading(121.0).presence, Presence::Online);
    }

    #[test]
    fn charging_wins_over_lost_link() {
        let mut s = signals();
        s.push(signal(0xb0, 2, SignalKind::Power { off: vec![2] }));
        let mut h = Heard::default();
        h.apply(&s, &[0, 0xbb, 1, 0, 0], 100.0);
        h.apply(&s, &[0, 0xb0, 2, 0, 0], 101.0);
        let r = h.reading(102.0);
        assert_eq!((r.presence, r.charge), (Presence::Online, Charge::Charging));
    }
}
