//! Diagnóstico para desarrollo (no se incluye en la app):
//!
//!   cargo run -p yescharge-core --example diag -- scan
//!   cargo run -p yescharge-core --example diag -- candidates
//!   cargo run -p yescharge-core --example diag -- listen 1234:5678 60
//!   cargo run -p yescharge-core --example diag -- sniff captura.pcapng 53 5253:1020
//!
//! Igual que la app, solo lee: no envía nada a ningún dispositivo.

use yescharge_core::{hid, probe, sources};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scan") => scan(),
        Some("candidates") => candidates(),
        Some("sniff") => sniff(args.get(1).expect("falta el archivo"), args.get(2).expect("falta el %"), args.get(3)),
        Some("watch") => watch(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(180)),
        Some("raw") => raw(args.get(1).expect("falta vid:pid"), args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5)),
        Some("listen") => {
            listen(args.get(1).expect("falta vid:pid"), args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15))
        }
        _ => eprintln!("uso: diag scan | candidates | listen <vid:pid> [segundos]"),
    }
}

/// Escucha (solo lectura) las interfaces del fabricante de un VID:PID e imprime lo que llega.
fn listen(vid_pid: &str, secs: u64) {
    let (v, p) = vid_pid.split_once(':').expect("formato vid:pid");
    let (vid, pid) = (u16::from_str_radix(v, 16).unwrap(), u16::from_str_radix(p, 16).unwrap());
    let all: Vec<_> = hid::enumerate().into_iter().filter(|i| i.vid == vid && i.pid == pid).collect();
    let l = yescharge_core::learn::Listener::start(&all);
    println!("escuchando {} interfaces del fabricante durante {secs} s…", l.listening_interfaces());
    for s in 0..secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        // fases a mitad de camino por si el usuario conecta/desconecta el cable
        l.set_phase(((s * 3) / secs.max(1)) as u8);
    }
    let cap = l.finish(vid, pid);
    for s in cap.samples.iter().take(40) {
        println!("{:>6} ms f{} i{}  {}", s.t_ms, s.phase, s.interface, s.data);
    }
    println!("total {} muestras distintas", cap.samples.len());
    for f in &cap.findings {
        println!("{f:?}");
    }
}

fn scan() {
    for i in hid::enumerate() {
        println!(
            "{:04x}:{:04x} página {:#06x} uso {:#06x} in/out/feat {}/{}/{}  {} | {}",
            i.vid, i.pid, i.usage_page, i.usage, i.input_len, i.output_len, i.feature_len, i.manufacturer, i.product
        );
    }
    println!("\nBluetooth con batería:");
    for b in sources::bluetooth_batteries() {
        println!("  {} -> {} %", b.name, b.battery);
    }
    println!("\nXInput:");
    for (slot, r) in sources::xinput_pads() {
        println!("  ranura {slot}: {r:?}");
    }
}

fn candidates() {
    for c in probe::candidates() {
        println!(
            "[{}] {:?} {:?} estándar={} escuchable={}  {} — {}",
            c.key, c.transport, c.kind, c.standard_battery, c.listenable, c.title, c.subtitle
        );
    }
}

/// Lee (solo lectura) cada interfaz de un VID:PID unos segundos e imprime resultados y errores.
fn raw(vid_pid: &str, secs: u64) {
    let (v, p) = vid_pid.split_once(':').expect("formato vid:pid");
    let (vid, pid) = (u16::from_str_radix(v, 16).unwrap(), u16::from_str_radix(p, 16).unwrap());
    for i in hid::enumerate().into_iter().filter(|i| i.vid == vid && i.pid == pid) {
        print!("página {:#06x} uso {:#06x} in={}: ", i.usage_page, i.usage, i.input_len);
        match hid::HidDevice::open(&i) {
            Err(e) => println!("no se abre: {e}"),
            Ok(d) => {
                let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
                let (mut got, mut timeouts) = (0, 0);
                let mut err = None;
                while std::time::Instant::now() < end {
                    match d.read(500) {
                        Ok(Some(r)) => {
                            got += 1;
                            if got <= 3 {
                                print!("[{}] ", yescharge_core::bytes::format(&r));
                            }
                        }
                        Ok(None) => timeouts += 1,
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                println!("reportes={got} esperas={timeouts} error={err:?}");
            }
        }
    }
}

/// Escucha (solo lectura) durante `secs` todas las interfaces del fabricante de TODO lo conectado
/// (y las multimedia, para comprobar que la captura funciona), volviendo a buscar cada 2 s porque
/// algunos dispositivos cambian de VID:PID al enchufar el cable.
fn watch(secs: u64) {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let start = std::time::Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let mut seen: HashSet<String> = HashSet::new();
    let mut threads = Vec::new();
    while start.elapsed().as_secs() < secs {
        for i in hid::enumerate() {
            if !(i.is_vendor_defined() || i.usage_page == 0x0c) || i.input_len == 0 {
                continue;
            }
            if !seen.insert(i.path.clone()) {
                continue;
            }
            let tag = format!("{:04x}:{:04x}/{:04x}", i.vid, i.pid, i.usage_page);
            println!("{:>6.1}s + escuchando {tag} {}", start.elapsed().as_secs_f32(), i.product);
            let Ok(d) = hid::HidDevice::open(&i) else {
                println!("        (no se pudo abrir {tag})");
                continue;
            };
            let st = stop.clone();
            threads.push(std::thread::spawn(move || {
                let mut last = None;
                while !st.load(Ordering::Relaxed) {
                    match d.read(500) {
                        Ok(Some(r)) if last.as_ref() != Some(&r) => {
                            println!(
                                "{:>6.1}s {tag}  {}",
                                start.elapsed().as_secs_f32(),
                                yescharge_core::bytes::format(&r)
                            );
                            last = Some(r);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            println!("{:>6.1}s {tag}  fin: {e}", start.elapsed().as_secs_f32());
                            break;
                        }
                    }
                }
            }));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    stop.store(true, Ordering::Relaxed);
    for t in threads {
        let _ = t.join();
    }
    println!("fin");
}

/// Busca en una captura USB la petición cuya respuesta trae el porcentaje. Solo lee el archivo, e
/// imprime únicamente lo del dispositivo indicado.
fn sniff(path: &str, percent: &str, device: Option<&String>) {
    let file = std::fs::read(path).expect("no se pudo leer el archivo");
    let percent: u8 = percent.parse().expect("porcentaje inválido");
    let only = device.map(|d| {
        let (v, p) = d.split_once(':').expect("formato vid:pid");
        (u16::from_str_radix(v, 16).unwrap(), u16::from_str_radix(p, 16).unwrap())
    });
    match yescharge_core::sniff::find_requests(&file, percent, only) {
        Err(e) => println!("no se pudo analizar: {e}"),
        Ok(found) if found.is_empty() => println!("ninguna respuesta trae el {percent} %"),
        Ok(found) => {
            for c in found {
                println!(
                    "{}x  {:04x?}  byte {} xor {:#04x}  enviado {:?}  respuesta {}",
                    c.times,
                    c.id,
                    c.offset,
                    c.xor,
                    c.sent,
                    yescharge_core::bytes::format(&c.reply[..c.reply.len().min(20)])
                );
            }
        }
    }
}
