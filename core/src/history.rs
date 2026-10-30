//! Historial de lecturas y modelo de autonomía APRENDIDO (port del modelo de BatteryInfo).
//!
//! Modelo: por dispositivo, acumuladores `[segundos, muestras]` en cada tramo de 10 %
//! (0..9). Aprende cuánto tarda en bajar cada nivel, respetando la curva no lineal del
//! voltaje. Solo estima cuando ya midió todos los tramos por debajo del nivel actual.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const WINDOW_H: f64 = 24.0 * 7.0;
pub const CAP: usize = 2500;
const MODEL_MAX_GAP_S: f64 = 3600.0;
/// Separación mínima entre dos puntos con el mismo valor. Con 7 días de historial, un punto por
/// minuto reescribiría cientos de KB cada minuto sin que la gráfica lo muestre.
const MIN_POINT_GAP_S: f64 = 300.0;
/// Para estimar la carga: tiempo mínimo subiendo.
const CHARGE_MIN_SPAN_S: f64 = 600.0;
/// Un salto mayor entre dos lecturas no es carga, sino el voltaje al conectar el cable.
const CHARGE_MAX_STEP: u8 = 3;
const MODEL_MIN_PER_BUCKET: f64 = 2.0;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    /// id de dispositivo -> lista de `(epoch, %)`.
    pub points: HashMap<String, Vec<(f64, u8)>>,
    /// id de dispositivo -> tramo -> `[segundos acumulados, muestras]`.
    pub model: HashMap<String, [[f64; 2]; 10]>,
}

impl History {
    pub fn load() -> Self {
        let points = read_json(&crate::paths::history_file()).unwrap_or_default();
        let model = read_json(&crate::paths::model_file()).unwrap_or_default();
        Self { points, model }
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_vec(&self.points) {
            let _ = crate::paths::write_atomic(&crate::paths::history_file(), &j);
        }
        if let Ok(j) = serde_json::to_vec(&self.model) {
            let _ = crate::paths::write_atomic(&crate::paths::model_file(), &j);
        }
    }

    /// Añade una lectura. Devuelve `false` si no aporta nada (igual a la anterior y muy seguida),
    /// para que pulsar "Actualizar" muchas veces no llene el historial ni escriba al disco.
    pub fn push(&mut self, id: &str, now: f64, pct: u8) -> bool {
        let pts = self.points.entry(id.to_string()).or_default();
        if let Some(&(prev_ts, prev_pct)) = pts.last() {
            if prev_pct == pct && now - prev_ts < MIN_POINT_GAP_S {
                return false;
            }
            self.model_update(id, prev_ts, prev_pct, now, pct);
        }
        let pts = self.points.entry(id.to_string()).or_default();
        pts.push((now, pct));
        let cutoff = now - WINDOW_H * 3600.0;
        pts.retain(|p| p.0 >= cutoff);
        if pts.len() > CAP {
            let extra = pts.len() - CAP;
            pts.drain(..extra);
        }
        true
    }

    pub fn forget(&mut self, id: &str) {
        self.points.remove(id);
        self.model.remove(id);
    }

    fn model_update(&mut self, id: &str, prev_ts: f64, prev_pct: u8, now: f64, cur: u8) {
        let dpct = prev_pct as f64 - cur as f64;
        let dt = now - prev_ts;
        if dpct <= 0.0 || dpct > 15.0 || dt <= 0.0 || dt > MODEL_MAX_GAP_S {
            return; // no es una descarga normal (recarga o salto de tiempo)
        }
        let per = dt / dpct; // segundos por 1 %
        let dev = self.model.entry(id.to_string()).or_insert([[0.0; 2]; 10]);
        for p in cur..prev_pct {
            let b = (p as usize / 10).min(9);
            dev[b][0] += per;
            dev[b][1] += 1.0;
        }
    }

    /// Horas restantes, o `None` si aún falta aprender algún tramo.
    pub fn estimate_hours(&self, id: &str, cur: u8) -> Option<f64> {
        let dev = self.model.get(id)?;
        if cur == 0 {
            return None;
        }
        let mut secs = 0.0;
        for p in 0..cur {
            let acc = dev[(p as usize / 10).min(9)];
            if acc[1] < MODEL_MIN_PER_BUCKET {
                return None;
            }
            secs += acc[0] / acc[1];
        }
        Some(secs / 3600.0)
    }

    /// Fracción 0..1 de los tramos necesarios ya aprendidos.
    pub fn progress(&self, id: &str, cur: u8) -> f64 {
        if cur == 0 {
            return 1.0;
        }
        let needed: Vec<usize> = (0..=((cur as usize - 1) / 10).min(9)).collect();
        let have = match self.model.get(id) {
            Some(dev) => needed.iter().filter(|&&b| dev[b][1] >= MODEL_MIN_PER_BUCKET).count(),
            None => 0,
        };
        have as f64 / needed.len() as f64
    }

    /// Tendencia de los últimos `window_s` segundos.
    pub fn trend(&self, id: &str, window_s: f64) -> Option<Trend> {
        let pts = self.points.get(id)?;
        let &(now, cur) = pts.last()?;
        if pts.len() < 2 {
            return None;
        }
        let past = pts.iter().rev().skip(1).find(|p| now - p.0 >= window_s).unwrap_or(&pts[0]).1;
        let diff = cur as i32 - past as i32;
        Some(if diff >= 2 {
            Trend::Up
        } else if diff <= -2 {
            Trend::Down
        } else {
            Trend::Flat
        })
    }

    /// Minutos que faltan para llenarse al ritmo de la subida actual, si ya hay una subida medible.
    pub fn charge_minutes(&self, id: &str) -> Option<f64> {
        let pts = self.points.get(id)?;
        let &(last_t, last) = pts.last()?;
        let mut first = (last_t, last);
        for pair in pts.windows(2).rev() {
            let (a, b) = (pair[0], pair[1]);
            if b.1 < a.1 || b.1 - a.1 > CHARGE_MAX_STEP {
                break;
            }
            first = a;
        }
        let (span, rise) = (last_t - first.0, last.saturating_sub(first.1));
        (span >= CHARGE_MIN_SPAN_S && rise >= 2).then(|| (100 - last.min(100)) as f64 * span / 60.0 / rise as f64)
    }

    /// Serie de las últimas `hours` reducida a `max_points` para dibujar la gráfica.
    pub fn sparkline(&self, id: &str, now: f64, hours: f64, max_points: usize) -> Vec<(f64, u8)> {
        let Some(all) = self.points.get(id) else { return Vec::new() };
        let cutoff = now - hours * 3600.0;
        let pts: Vec<(f64, u8)> = all.iter().filter(|p| p.0 >= cutoff).copied().collect();
        if pts.len() <= max_points {
            return pts;
        }
        let step = pts.len() as f64 / max_points as f64;
        (0..max_points)
            .map(|i| pts[((i as f64 * step) as usize).min(pts.len() - 1)])
            .chain(pts.last().copied())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    Up,
    Down,
    Flat,
}

pub fn format_eta(hours: f64) -> String {
    use crate::trf;
    let total = (hours * 60.0).round() as i64;
    let (d, rem) = (total / 1440, total % 1440);
    let (h, m) = (rem / 60, rem % 60);
    if d > 0 {
        trf!("~{d} d {h} h restantes", "~{d} d {h} h left")
    } else if h > 0 {
        trf!("~{h} h {m} min restantes", "~{h} h {m} min left")
    } else {
        trf!("~{m} min restantes", "~{m} min left")
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_drain_and_estimates_only_when_complete() {
        let mut h = History::default();
        let mut t = 0.0;
        // baja de 30 % a 0 % a 1 % cada 10 minutos, dos veces (2 muestras por tramo)
        for _ in 0..2 {
            for pct in (0..=30u8).rev() {
                h.push("d", t, pct);
                t += 600.0;
            }
            t += 7200.0; // recarga: salto que el modelo ignora
        }
        assert!(h.estimate_hours("d", 40).is_none(), "el tramo 30-39 no está aprendido");
        let hours = h.estimate_hours("d", 30).expect("tramos 0-29 aprendidos");
        assert!((hours - 5.0).abs() < 0.01, "30 % × 10 min = 5 h, salió {hours}");
        assert_eq!(h.progress("d", 30), 1.0);
    }

    #[test]
    fn eta_format() {
        assert_eq!(format_eta(0.5), "~30 min restantes");
        assert_eq!(format_eta(2.25), "~2 h 15 min restantes");
        assert_eq!(format_eta(26.0), "~1 d 2 h restantes");
    }

    #[test]
    fn repeated_readings_do_not_fill_the_history() {
        let mut h = History::default();
        assert!(h.push("d", 1000.0, 50));
        for i in 1..20 {
            assert!(!h.push("d", 1000.0 + i as f64 * 0.2, 50), "lo mismo, muy seguido: no es un punto nuevo");
        }
        assert!(h.push("d", 1010.0, 49), "un valor distinto siempre cuenta");
        assert!(!h.push("d", 1100.0, 49), "mismo valor al poco rato: no hace falta guardarlo");
        assert!(h.push("d", 1400.0, 49), "pasados 5 minutos, el mismo valor también cuenta");
        assert_eq!(h.points["d"].len(), 3);
    }

    #[test]
    fn charge_time_uses_the_steady_rise_and_ignores_the_cable_jump() {
        let mut h = History::default();
        for (t, pct) in [(0.0, 60), (60.0, 73), (360.0, 74), (660.0, 75), (960.0, 76), (1260.0, 77)] {
            h.push("d", t, pct);
        }
        // 3 % en 15 minutos desde el salto: faltan 23 % a 5 minutos cada uno
        assert_eq!(h.charge_minutes("d"), Some(115.0));
        assert_eq!(h.sparkline("d", 1300.0, 0.1, 96).len(), 2, "solo la última parte");
    }

    #[test]
    fn no_charge_time_until_it_really_rises() {
        let mut h = History::default();
        h.push("d", 0.0, 50);
        h.push("d", 1000.0, 49);
        assert_eq!(h.charge_minutes("d"), None);
    }
}
