//! Bytes en texto hexadecimal (`"07 50 01"`, con repeticiones `"ff*31"`), para capturas e informes.

/// Tope de bytes de una línea (ningún reporte HID real es tan largo).
const MAX_LEN: usize = 1024;

pub fn parse(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        let (hex, count) = match tok.split_once('*') {
            Some((h, c)) => (h, c.parse::<usize>().map_err(|_| format!("repetición inválida: {tok}"))?),
            None => (tok, 1),
        };
        let b = u8::from_str_radix(hex.trim_start_matches("0x"), 16).map_err(|_| format!("byte inválido: {tok}"))?;
        if out.len() + count > MAX_LEN {
            return Err("demasiados bytes".into());
        }
        out.extend(std::iter::repeat_n(b, count));
    }
    Ok(out)
}

pub fn format(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        assert_eq!(super::parse("07 ff*3 0x10").unwrap(), vec![7, 255, 255, 255, 16]);
        assert_eq!(super::format(&[7, 0xff]), "07 ff");
        assert!(super::parse("zz").is_err());
    }
}
