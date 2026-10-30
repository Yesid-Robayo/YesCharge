//! Análisis de capturas USB (USBPcap): encuentra qué petición de la app oficial trae el porcentaje.
//!
//! Solo lee un archivo: no toca ningún dispositivo. Una captura registra todo lo que pasa por ese
//! puerto USB, así que aquí nunca se conserva nada que no sea del dispositivo elegido.
//!
//! La idea: con la app oficial abierta, buscar una respuesta del dispositivo que contenga el
//! porcentaje que el usuario ve, y quedarse con lo último que se le envió justo antes.

use std::collections::{BTreeMap, HashMap};

use crate::learn::text_offsets;

/// Tipo de enlace de USBPcap en los archivos pcap.
const LINKTYPE_USBPCAP: u32 = 249;
/// Una respuesta solo cuenta si llega tan poco después de lo enviado.
const REPLY_WINDOW_MS: u64 = 1000;
const MAX_CANDIDATES: usize = 5;

const TRANSFER_INTERRUPT: u8 = 1;
const TRANSFER_CONTROL: u8 = 2;
const STAGE_SETUP: u8 = 0;
const STAGE_DATA: u8 = 1;
const STAGE_COMPLETE: u8 = 3;

const SET_REPORT: u8 = 0x09;
const GET_REPORT: u8 = 0x01;
const GET_DESCRIPTOR: u8 = 0x06;
const REPORT_TYPE_OUTPUT: u8 = 2;
const REPORT_TYPE_FEATURE: u8 = 3;

/// Un paquete de la captura.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Packet {
    t_ms: u64,
    irp: u64,
    from_device: bool,
    device: (u16, u16),
    endpoint: u8,
    transfer: u8,
    stage: Option<u8>,
    data: Vec<u8>,
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64le(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Interpreta la cabecera de USBPcap de un paquete.
fn parse_frame(frame: &[u8], t_ms: u64) -> Option<Packet> {
    let header_len = u16le(frame, 0)? as usize;
    if header_len < 27 || header_len > frame.len() {
        return None;
    }
    let transfer = frame[22];
    let data_len = u32le(frame, 23)? as usize;
    let end = header_len.saturating_add(data_len).min(frame.len());
    Some(Packet {
        t_ms,
        irp: u64le(frame, 2)?,
        from_device: frame[16] & 1 != 0,
        device: (u16le(frame, 17)?, u16le(frame, 19)?),
        endpoint: frame[21],
        transfer,
        stage: (transfer == TRANSFER_CONTROL && header_len >= 28).then(|| frame[27]),
        data: frame[header_len..end].to_vec(),
    })
}

fn read_pcap(file: &[u8]) -> Result<Vec<Packet>, String> {
    let link = u32le(file, 20).ok_or("archivo incompleto")?;
    if link != LINKTYPE_USBPCAP {
        return Err("no es una captura de USBPcap".into());
    }
    let mut out = Vec::new();
    let mut at = 24;
    while let (Some(sec), Some(usec), Some(len)) = (u32le(file, at), u32le(file, at + 4), u32le(file, at + 8)) {
        let start = at + 16;
        let Some(frame) = file.get(start..start + len as usize) else { break };
        out.extend(parse_frame(frame, sec as u64 * 1000 + usec as u64 / 1000));
        at = start + len as usize;
    }
    Ok(out)
}

fn read_pcapng(file: &[u8]) -> Result<Vec<Packet>, String> {
    const INTERFACE: u32 = 1;
    const ENHANCED_PACKET: u32 = 6;
    let mut links = Vec::new();
    let mut out = Vec::new();
    let mut at = 0;
    while let (Some(kind), Some(total)) = (u32le(file, at), u32le(file, at + 4)) {
        let total = total as usize;
        if total < 12 || at + total > file.len() {
            break;
        }
        let body = &file[at + 8..at + total - 4];
        if kind == INTERFACE {
            links.push(u16le(body, 0).unwrap_or(0) as u32);
        } else if kind == ENHANCED_PACKET
            && let (Some(iface), Some(high), Some(low), Some(len)) =
                (u32le(body, 0), u32le(body, 4), u32le(body, 8), u32le(body, 12))
            && links.get(iface as usize) == Some(&LINKTYPE_USBPCAP)
            && let Some(frame) = body.get(20..20 + len as usize)
        {
            let micros = ((high as u64) << 32) | low as u64;
            out.extend(parse_frame(frame, micros / 1000));
        }
        at += total;
    }
    if links.contains(&LINKTYPE_USBPCAP) { Ok(out) } else { Err("no es una captura de USBPcap".into()) }
}

fn read_packets(file: &[u8]) -> Result<Vec<Packet>, String> {
    match file.get(..4) {
        Some([0xd4, 0xc3, 0xb2, 0xa1]) | Some([0x4d, 0x3c, 0xb2, 0xa1]) => read_pcap(file),
        Some([0x0a, 0x0d, 0x0d, 0x0a]) => read_pcapng(file),
        _ => Err("formato de archivo desconocido".into()),
    }
}

/// Lo que se le envió a un dispositivo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sent {
    /// Output report, tal como viaja por el cable (sin report ID si el reporte no va numerado).
    Output(Vec<u8>),
    /// Feature report (SET_REPORT), con el report ID delante.
    Feature(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Traffic {
    Sent(Sent),
    Reply(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Event {
    t_ms: u64,
    device: (u16, u16),
    traffic: Traffic,
}

#[derive(Debug, Clone, Copy)]
struct Setup {
    request: u8,
    report_type: u8,
    report_id: u8,
    value_high: u8,
}

/// Con el report ID delante si es un reporte numerado o se sabe que es el 0.
fn with_report_id(id: u8, data: &[u8]) -> Vec<u8> {
    if id == 0 { std::iter::once(0).chain(data.iter().copied()).collect() } else { data.to_vec() }
}

/// Dirección USB de cada dispositivo -> su VID:PID.
type DeviceIds = HashMap<(u16, u16), (u16, u16)>;

/// Convierte los paquetes en lo que se envió y lo que respondió cada dispositivo, y anota el
/// VID:PID de los que aparecen en sus descriptores.
fn extract(packets: &[Packet]) -> (Vec<Event>, DeviceIds) {
    let mut events = Vec::new();
    let mut ids = HashMap::new();
    let mut pending: HashMap<u64, Setup> = HashMap::new();
    for p in packets {
        match (p.transfer, p.stage) {
            (TRANSFER_INTERRUPT, _) if !p.data.is_empty() => {
                let inward = p.endpoint & 0x80 != 0;
                let traffic = match (inward, p.from_device) {
                    (true, true) => Traffic::Reply(p.data.clone()),
                    (false, false) => Traffic::Sent(Sent::Output(p.data.clone())),
                    _ => continue,
                };
                events.push(Event { t_ms: p.t_ms, device: p.device, traffic });
            }
            (TRANSFER_CONTROL, Some(STAGE_SETUP)) if p.data.len() >= 8 => {
                let setup =
                    Setup { request: p.data[1], report_id: p.data[2], value_high: p.data[3], report_type: p.data[3] };
                pending.insert(p.irp, setup);
                // algunas versiones traen los datos de salida en el mismo paquete
                if setup.request == SET_REPORT && p.data.len() > 8 {
                    push_set_report(&mut events, p, setup, &p.data[8..]);
                }
            }
            (TRANSFER_CONTROL, Some(STAGE_DATA | STAGE_COMPLETE)) if !p.data.is_empty() => {
                let Some(setup) = pending.get(&p.irp).copied() else { continue };
                match setup.request {
                    SET_REPORT if !p.from_device => push_set_report(&mut events, p, setup, &p.data),
                    GET_REPORT if p.from_device => events.push(Event {
                        t_ms: p.t_ms,
                        device: p.device,
                        traffic: Traffic::Reply(with_report_id(setup.report_id, &p.data)),
                    }),
                    // descriptor de dispositivo: VID en el byte 8 y PID en el 10
                    GET_DESCRIPTOR if p.from_device && setup.value_high == 1 && p.data.len() >= 12 => {
                        if let (Some(vid), Some(pid)) = (u16le(&p.data, 8), u16le(&p.data, 10)) {
                            ids.insert(p.device, (vid, pid));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    (events, ids)
}

fn push_set_report(events: &mut Vec<Event>, p: &Packet, setup: Setup, data: &[u8]) {
    let sent = match setup.report_type {
        REPORT_TYPE_FEATURE => Sent::Feature(with_report_id(setup.report_id, data)),
        REPORT_TYPE_OUTPUT => Sent::Output(with_report_id(setup.report_id, data)),
        _ => return,
    };
    events.push(Event { t_ms: p.t_ms, device: p.device, traffic: Traffic::Sent(sent) });
}

/// Una petición que parece ser la de la batería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// VID:PID, si la captura incluye sus descriptores.
    pub id: Option<(u16, u16)>,
    pub sent: Sent,
    /// Posición del porcentaje en la respuesta.
    pub offset: usize,
    /// 0, o 0xff si el valor viaja invertido.
    pub xor: u8,
    /// Cuántas veces se vio esta misma pregunta con el porcentaje en la respuesta.
    pub times: u32,
    /// Una respuesta de ejemplo.
    pub reply: Vec<u8>,
}

/// Busca en una captura las peticiones cuya respuesta trae `percent`. Con `only`, mira solo ese
/// dispositivo.
pub fn find_requests(file: &[u8], percent: u8, only: Option<(u16, u16)>) -> Result<Vec<Candidate>, String> {
    let packets = read_packets(file)?;
    let (events, ids) = extract(&packets);
    if let Some(wanted) = only
        && !ids.values().any(|id| *id == wanted)
    {
        return Err("la captura no incluye los descriptores de ese dispositivo".into());
    }
    let wanted = |device: (u16, u16)| only.is_none_or(|id| ids.get(&device) == Some(&id));
    let mut last_sent: HashMap<(u16, u16), (u64, Sent)> = HashMap::new();
    type Key = ((u16, u16), Sent, usize, u8);
    let mut found: BTreeMap<Key, (u32, Vec<u8>)> = BTreeMap::new();
    for e in events.iter().filter(|e| wanted(e.device)) {
        match &e.traffic {
            Traffic::Sent(s) => {
                last_sent.insert(e.device, (e.t_ms, s.clone()));
            }
            Traffic::Reply(r) => {
                let Some((t, sent)) = last_sent.get(&e.device) else { continue };
                if e.t_ms.saturating_sub(*t) > REPLY_WINDOW_MS || is_echo(sent, r) {
                    continue;
                }
                for xor in [0u8, 0xff] {
                    let text = text_offsets(r, xor);
                    for (offset, b) in r.iter().enumerate() {
                        if b ^ xor == percent && !text.contains(&offset) {
                            let entry = found.entry((e.device, sent.clone(), offset, xor));
                            entry.or_insert_with(|| (0, r.clone())).0 += 1;
                        }
                    }
                }
            }
        }
    }
    let mut out: Vec<Candidate> = found
        .into_iter()
        .map(|((device, sent, offset, xor), (times, reply))| Candidate {
            id: ids.get(&device).copied(),
            sent,
            offset,
            xor,
            times,
            reply,
        })
        .collect();
    // lo que más se repite es lo más probable: la app oficial consulta la batería una y otra vez
    out.sort_by_key(|c| std::cmp::Reverse(c.times));
    out.truncate(MAX_CANDIDATES);
    Ok(out)
}

/// Algunos receptores devuelven tal cual lo que se les envió: eso no es una respuesta. Las
/// respuestas de verdad también suelen repetir el comando al principio, así que se compara todo,
/// sin contar el relleno del final.
fn is_echo(sent: &Sent, reply: &[u8]) -> bool {
    let (Sent::Output(s) | Sent::Feature(s)) = sent;
    let trim = |b: &'_ [u8]| b.len() - b.iter().rev().take_while(|v| **v == 0 || **v == 0xff).count();
    s[..trim(s)] == reply[..trim(reply)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE: (u16, u16) = (0x5253, 0x1020);
    const OTHER: (u16, u16) = (0x1234, 0x5678);

    /// Paquete USBPcap: cabecera + datos.
    fn frame(irp: u64, from_device: bool, addr: u16, ep: u8, transfer: u8, stage: Option<u8>, data: &[u8]) -> Vec<u8> {
        let header_len: u16 = if stage.is_some() { 28 } else { 27 };
        let mut f = Vec::new();
        f.extend(header_len.to_le_bytes());
        f.extend(irp.to_le_bytes());
        f.extend(0u32.to_le_bytes());
        f.extend(0u16.to_le_bytes());
        f.push(from_device as u8);
        f.extend(1u16.to_le_bytes());
        f.extend(addr.to_le_bytes());
        f.push(ep);
        f.push(transfer);
        f.extend((data.len() as u32).to_le_bytes());
        if let Some(s) = stage {
            f.push(s);
        }
        f.extend(data);
        f
    }

    fn pcap(frames: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0];
        out.extend([0u8; 8]);
        out.extend(65535u32.to_le_bytes());
        out.extend(LINKTYPE_USBPCAP.to_le_bytes());
        for (ms, f) in frames {
            out.extend(((ms / 1000) as u32).to_le_bytes());
            out.extend((((ms % 1000) * 1000) as u32).to_le_bytes());
            out.extend((f.len() as u32).to_le_bytes());
            out.extend((f.len() as u32).to_le_bytes());
            out.extend(f);
        }
        out
    }

    fn descriptor(irp: u64, addr: u16, id: (u16, u16)) -> Vec<(u64, Vec<u8>)> {
        let mut d = vec![18, 1, 0, 2, 0, 0, 0, 64];
        d.extend(id.0.to_le_bytes());
        d.extend(id.1.to_le_bytes());
        d.extend([0u8; 6]);
        vec![
            (1, frame(irp, false, addr, 0x80, 2, Some(0), &[0x80, 0x06, 0x00, 0x01, 0, 0, 18, 0])),
            (2, frame(irp, true, addr, 0x80, 2, Some(1), &d)),
        ]
    }

    /// Pregunta por feature report y respuesta invertida, como un ratón con receptor.
    fn feature_exchange(irp: u64, t: u64, addr: u16, percent: u8) -> Vec<(u64, Vec<u8>)> {
        let mut ask = vec![0x11, 0xf9];
        ask.extend([0xff; 31]);
        let mut reply = vec![0x11, 0xf9];
        reply.extend([0xff; 62]);
        reply[11] = percent ^ 0xff;
        vec![
            (t, frame(irp, false, addr, 0, 2, Some(0), &[0x21, 0x09, 0x11, 0x03, 2, 0, 33, 0])),
            (t, frame(irp, false, addr, 0, 2, Some(1), &ask)),
            (t + 1, frame(irp + 1, false, addr, 0x80, 2, Some(0), &[0xa1, 0x01, 0x11, 0x03, 2, 0, 64, 0])),
            (t + 60, frame(irp + 1, true, addr, 0x80, 2, Some(1), &reply)),
        ]
    }

    #[test]
    fn finds_a_feature_request_with_an_inverted_reply() {
        let mut frames = descriptor(1, 5, MOUSE);
        for i in 0..3 {
            frames.extend(feature_exchange(10 + i * 2, 1000 + i * 5000, 5, 53));
        }
        let found = find_requests(&pcap(&frames), 53, Some(MOUSE)).unwrap();
        let c = &found[0];
        assert_eq!((c.id, c.offset, c.xor, c.times), (Some(MOUSE), 11, 0xff, 3));
        let Sent::Feature(ask) = &c.sent else { panic!("debe ser un feature report") };
        assert_eq!(ask[..2], [0x11, 0xf9]);
    }

    #[test]
    fn finds_an_output_request_answered_by_an_input_report() {
        let frames = vec![
            (1000, frame(1, false, 7, 0x03, 1, None, &[0xb0])),
            (1010, frame(2, true, 7, 0x83, 1, None, &[0xb0, 0x03, 0x02, 0x2f, 0x01])),
        ];
        let found = find_requests(&pcap(&frames), 47, None).unwrap();
        assert_eq!((found[0].sent.clone(), found[0].offset, found[0].xor), (Sent::Output(vec![0xb0]), 3, 0));
    }

    #[test]
    fn other_devices_are_never_looked_at() {
        let mut frames = descriptor(1, 5, MOUSE);
        frames.extend(descriptor(3, 6, OTHER));
        frames.extend(feature_exchange(10, 1000, 6, 53));
        assert!(find_requests(&pcap(&frames), 53, Some(MOUSE)).unwrap().is_empty());
    }

    #[test]
    fn a_reply_long_after_the_request_does_not_count() {
        let frames = vec![
            (1000, frame(1, false, 7, 0x03, 1, None, &[0xb0])),
            (9000, frame(2, true, 7, 0x83, 1, None, &[0xb0, 0x03, 0x02, 0x2f])),
        ];
        assert!(find_requests(&pcap(&frames), 47, None).unwrap().is_empty());
    }

    #[test]
    fn an_echo_of_the_request_is_not_an_answer() {
        let frames = vec![
            (1000, frame(1, false, 7, 0x03, 1, None, &[0xb0, 0x2f])),
            (1010, frame(2, true, 7, 0x83, 1, None, &[0xb0, 0x2f, 0x00])),
        ];
        assert!(find_requests(&pcap(&frames), 47, None).unwrap().is_empty());
    }

    #[test]
    fn text_inside_a_reply_is_ignored() {
        // la versión de firmware "0.01.0" trae bytes que parecen un 48 %
        let frames = vec![
            (1000, frame(1, false, 7, 0x03, 1, None, &[0x10])),
            (1010, frame(2, true, 7, 0x83, 1, None, &[0x10, 0x30, 0x2e, 0x30, 0x31, 0x2e, 0x30])),
        ];
        assert!(find_requests(&pcap(&frames), 48, None).unwrap().is_empty());
    }

    #[test]
    fn reads_pcapng_files_from_wireshark() {
        let frames = [
            (1000u64, frame(1, false, 7, 0x03, 1, None, &[0xb0])),
            (1010, frame(2, true, 7, 0x83, 1, None, &[0xb0, 0x03, 0x02, 0x2f, 0x01])),
        ];
        let block = |kind: u32, body: Vec<u8>| {
            let total = (body.len() + 12) as u32;
            let mut b = kind.to_le_bytes().to_vec();
            b.extend(total.to_le_bytes());
            b.extend(body);
            b.extend(total.to_le_bytes());
            b
        };
        let mut file = block(0x0a0d0d0a, vec![0x4d, 0x3c, 0x2b, 0x1a, 1, 0, 0, 0]);
        let mut idb = (LINKTYPE_USBPCAP as u16).to_le_bytes().to_vec();
        idb.extend([0u8; 6]);
        file.extend(block(1, idb));
        for (ms, f) in &frames {
            let micros = ms * 1000;
            let mut body = 0u32.to_le_bytes().to_vec();
            body.extend(((micros >> 32) as u32).to_le_bytes());
            body.extend((micros as u32).to_le_bytes());
            body.extend((f.len() as u32).to_le_bytes());
            body.extend((f.len() as u32).to_le_bytes());
            body.extend(f);
            while !body.len().is_multiple_of(4) {
                body.push(0);
            }
            file.extend(block(6, body));
        }
        let found = find_requests(&file, 47, None).unwrap();
        assert_eq!(found[0].sent, Sent::Output(vec![0xb0]));
    }

    #[test]
    fn rejects_files_that_are_not_captures() {
        assert!(find_requests(b"esto no es una captura", 50, None).is_err());
        assert!(find_requests(&[], 50, None).is_err());
    }

    #[test]
    fn a_broken_file_does_not_crash() {
        let mut file = pcap(&[(1000, frame(1, false, 7, 0x03, 1, None, &[0xb0]))]);
        file.truncate(file.len() - 2);
        assert!(find_requests(&file, 47, None).unwrap().is_empty());
    }
}
