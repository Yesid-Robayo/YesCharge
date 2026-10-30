//! API HTTP local opcional (desactivada por defecto) para pantallas externas.
//!
//! - Escucha solo en 127.0.0.1: no es accesible desde la red.
//! - Solo `GET`, sin cabeceras CORS: una web abierta en el navegador no puede leer los datos.
//! - Valida la cabecera `Host` (solo `127.0.0.1` o `localhost`), lo que bloquea el *DNS
//!   rebinding* (una web que se hace pasar por localhost).
//! - Peticiones de tamaño y tiempo acotados, y un máximo de conexiones simultáneas.
//!
//! Rutas: `/v1/status` (estado completo) y `/min` (formato compacto compatible con BatteryInfo).

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use yescharge_core::reading::Charge;
use yescharge_core::status::Snapshot;

const MAX_CONNECTIONS: usize = 8;
const MAX_HEADER_BYTES: usize = 8 * 1024;

pub fn start(port: u16, snapshot: Arc<Mutex<Snapshot>>) -> Result<(), String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| e.to_string())?;
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("api".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue; // se descarta: demasiadas conexiones a la vez
                }
                active.fetch_add(1, Ordering::Relaxed);
                let (snap, act) = (snapshot.clone(), active.clone());
                std::thread::spawn(move || {
                    let _ = handle(stream, port, &snap);
                    act.fetch_sub(1, Ordering::Relaxed);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn handle(stream: TcpStream, port: u16, snap: &Mutex<Snapshot>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut host = String::new();
    let mut total = request_line.len();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        total += line.len();
        if total > MAX_HEADER_BYTES {
            return respond(stream, "431 Request Header Fields Too Large", "{}");
        }
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("host")
        {
            host = v.trim().to_ascii_lowercase();
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let allowed = [format!("127.0.0.1:{port}"), format!("localhost:{port}"), "127.0.0.1".into(), "localhost".into()];
    if !allowed.contains(&host) {
        return respond(stream, "403 Forbidden", "{}");
    }
    if method != "GET" {
        return respond(stream, "405 Method Not Allowed", "{}");
    }
    let s = snap.lock().map(|g| g.clone()).unwrap_or_default();
    let route = path.split('?').next().unwrap_or("/").trim_end_matches('/');
    match route {
        "/v1/status" | "" => respond(stream, "200 OK", &serde_json::to_string(&s).unwrap_or_default()),
        "/min" => respond(stream, "200 OK", &compact(&s).to_string()),
        _ => respond(stream, "404 Not Found", "{}"),
    }
}

/// Formato compacto de BatteryInfo (`/min`), para pantallas que ya lo usan.
fn compact(s: &Snapshot) -> serde_json::Value {
    let d: Vec<_> = s
        .devices
        .iter()
        .map(|d| {
            let chg = match d.charge {
                Charge::Charging => Some(true),
                Charge::Full => Some(d.topping_off),
                Charge::Discharging => Some(false),
                Charge::Unknown => None,
            };
            serde_json::json!({
                "n": d.name, "b": d.battery, "low": d.low, "chg": chg, "full": d.full,
                "sat": d.topping_off, "tr": d.trend, "eta": d.eta,
            })
        })
        .collect();
    serde_json::json!({
        "t": s.updated_at as u64,
        "charge": s.devices.iter().any(|d| d.low),
        "full": s.devices.iter().any(|d| d.full),
        "d": d,
    })
}

fn respond(mut stream: TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}
