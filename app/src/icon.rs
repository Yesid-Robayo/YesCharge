//! Iconos dibujados por código (sin imágenes externas): el de la app y el de la bandeja, que
//! muestra el nivel del dispositivo más bajo. Rasterizador mínimo con supermuestreo 4×4.

/// Color RGBA lineal (0..1) sin premultiplicar.
#[derive(Clone, Copy)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub const fn hex(rgb: u32) -> Self {
        Rgba(((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0, 1.0)
    }

    fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        Rgba(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t, a.3 + (b.3 - a.3) * t)
    }
}

pub const GOOD: Rgba = Rgba::hex(0x3ddc97);
pub const WARN: Rgba = Rgba::hex(0xf5c451);
pub const LOW: Rgba = Rgba::hex(0xff5d6c);
const WHITE: Rgba = Rgba::hex(0xf2f4f8);
const MUTED: Rgba = Rgba::hex(0x8c93a6);
const BRAND_A: Rgba = Rgba::hex(0x43e5a0);
const BRAND_B: Rgba = Rgba::hex(0x12a8c9);

pub struct Canvas {
    pub size: u32,
    /// RGBA premultiplicado.
    px: Vec<[f32; 4]>,
}

fn rounded_rect(x: f32, y: f32, r: [f32; 5]) -> bool {
    let [x0, y0, x1, y1, rad] = r;
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    // el radio nunca puede pasar de la mitad del lado (barras muy estrechas con batería baja)
    let rad = rad.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    let cx = x.clamp(x0 + rad, x1 - rad);
    let cy = y.clamp(y0 + rad, y1 - rad);
    (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
}

fn in_polygon(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Canvas {
    pub fn new(size: u32) -> Self {
        Self { size, px: vec![[0.0; 4]; (size * size) as usize] }
    }

    /// Rellena la figura `inside` (coordenadas 0..1) con el color `paint(x, y)`.
    fn fill(&mut self, inside: impl Fn(f32, f32) -> bool, paint: impl Fn(f32, f32) -> Rgba) {
        let s = self.size as f32;
        const N: usize = 4;
        for py in 0..self.size {
            for px in 0..self.size {
                let mut hits = 0;
                for sy in 0..N {
                    for sx in 0..N {
                        let x = (px as f32 + (sx as f32 + 0.5) / N as f32) / s;
                        let y = (py as f32 + (sy as f32 + 0.5) / N as f32) / s;
                        if inside(x, y) {
                            hits += 1;
                        }
                    }
                }
                if hits == 0 {
                    continue;
                }
                let cov = hits as f32 / (N * N) as f32;
                let c = paint((px as f32 + 0.5) / s, (py as f32 + 0.5) / s);
                let a = c.3 * cov;
                let dst = &mut self.px[(py * self.size + px) as usize];
                // "source over" premultiplicado
                dst[0] = c.0 * a + dst[0] * (1.0 - a);
                dst[1] = c.1 * a + dst[1] * (1.0 - a);
                dst[2] = c.2 * a + dst[2] * (1.0 - a);
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }

    /// Píxeles RGBA de 8 bits sin premultiplicar (formato de egui y de los PNG).
    pub fn rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3];
            let un = |v: f32| if a > 0.0 { (v / a).clamp(0.0, 1.0) } else { 0.0 };
            out.extend([
                (un(p[0]) * 255.0).round() as u8,
                (un(p[1]) * 255.0).round() as u8,
                (un(p[2]) * 255.0).round() as u8,
                (a * 255.0).round() as u8,
            ]);
        }
        out
    }
}

/// Rayo en coordenadas unitarias, centrado en (cx, cy) con alto `h`.
fn bolt(cx: f32, cy: f32, h: f32) -> Vec<(f32, f32)> {
    [(0.06, -0.50), (0.26, -0.50), (0.10, -0.06), (0.30, -0.06), (-0.12, 0.50), (0.00, 0.06), (-0.20, 0.06)]
        .iter()
        .map(|(x, y)| (cx + x * h, cy + y * h))
        .collect()
}

/// Icono de la app: insignia con degradado y una batería con un rayo.
pub fn app_icon(size: u32) -> Canvas {
    let mut c = Canvas::new(size);
    let bg = [0.03, 0.03, 0.97, 0.97, 0.24];
    let grad = |x: f32, y: f32| Rgba::lerp(BRAND_A, BRAND_B, (x + y) / 2.0);
    c.fill(|x, y| rounded_rect(x, y, bg), grad);
    // batería: contorno blanco + borne
    let outer: [f32; 5] = [0.20, 0.31, 0.76, 0.69, 0.10];
    let w = 0.065f32;
    let inner = [outer[0] + w, outer[1] + w, outer[2] - w, outer[3] - w, (outer[4] - w).max(0.01)];
    c.fill(|x, y| rounded_rect(x, y, outer) && !rounded_rect(x, y, inner), |_, _| WHITE);
    c.fill(|x, y| rounded_rect(x, y, [0.775, 0.43, 0.835, 0.57, 0.025]), |_, _| WHITE);
    // rayo con un "hueco" del color de fondo para que corte el contorno
    let gap = bolt(0.48, 0.5, 0.62);
    let gap: Vec<(f32, f32)> = gap.iter().map(|(x, y)| (0.48 + (x - 0.48) * 1.22, 0.5 + (y - 0.5) * 1.12)).collect();
    c.fill(|x, y| in_polygon(x, y, &gap), grad);
    let b = bolt(0.48, 0.5, 0.62);
    c.fill(|x, y| in_polygon(x, y, &b), |_, _| WHITE);
    c
}

/// Estado que representa el icono de la bandeja.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrayState {
    /// Nivel más bajo entre los dispositivos con dato; `None` si no hay ninguno.
    pub level: Option<u8>,
    pub charging: bool,
    pub low: bool,
}

pub fn level_color(level: u8, low: bool) -> Rgba {
    if low || level <= 15 {
        LOW
    } else if level <= 35 {
        WARN
    } else {
        GOOD
    }
}

/// Icono de la bandeja: batería horizontal rellena según el nivel, con rayo si algo carga.
pub fn tray_icon(size: u32, st: TrayState) -> Canvas {
    let mut c = Canvas::new(size);
    let px = 1.0 / size as f32;
    let stroke = (0.09f32).max(1.2 * px);
    let outer: [f32; 5] = [0.04, 0.22, 0.86, 0.78, 0.14];
    let inner =
        [outer[0] + stroke, outer[1] + stroke, outer[2] - stroke, outer[3] - stroke, (outer[4] - stroke).max(px)];
    let frame = if st.level.is_some() { WHITE } else { MUTED };
    c.fill(|x, y| rounded_rect(x, y, outer) && !rounded_rect(x, y, inner), |_, _| frame);
    c.fill(|x, y| rounded_rect(x, y, [0.875, 0.38, 0.96, 0.62, 0.03]), |_, _| frame);
    if let Some(level) = st.level {
        let pad = (0.05f32).max(px);
        let x0 = inner[0] + pad;
        let x1 = inner[2] - pad;
        let fill_x = x0 + (x1 - x0) * (level.max(8) as f32 / 100.0);
        let bar = [x0, inner[1] + pad, fill_x, inner[3] - pad, (inner[4] - pad).max(px * 0.5)];
        let color = level_color(level, st.low);
        c.fill(|x, y| rounded_rect(x, y, bar), |_, _| color);
    }
    if st.charging {
        // rayo dentro de la batería, con un borde oscuro para que se distinga del relleno
        let (cx, cy, h) = (0.45, 0.5, 0.6);
        let gap: Vec<(f32, f32)> =
            bolt(cx, cy, h).iter().map(|(x, y)| (cx + (x - cx) * 1.4, cy + (y - cy) * 1.25)).collect();
        c.fill(|x, y| in_polygon(x, y, &gap), |_, _| Rgba(0.05, 0.06, 0.08, 1.0));
        let b = bolt(cx, cy, h);
        c.fill(|x, y| in_polygon(x, y, &b), |_, _| WHITE);
    }
    c
}

/// Archivo .ico con varias resoluciones (entradas BMP de 32 bits). Lo usa `build.rs`.
#[allow(dead_code)]
pub fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| bmp_entry(&app_icon(s))).collect();
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend([dim, dim, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((img.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}

/// Imagen BMP de icono: cabecera + BGRA de abajo arriba + máscara AND.
#[allow(dead_code)]
fn bmp_entry(c: &Canvas) -> Vec<u8> {
    let s = c.size;
    let rgba = c.rgba8();
    let mut out = Vec::new();
    out.extend(40u32.to_le_bytes());
    out.extend((s as i32).to_le_bytes());
    out.extend((s as i32 * 2).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend([0u8; 24]);
    for y in (0..s).rev() {
        for x in 0..s {
            let i = ((y * s + x) * 4) as usize;
            out.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    let mask_row = (s.div_ceil(32) * 4) as usize;
    out.extend(vec![0u8; mask_row * s as usize]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vuelca los iconos a BMP para revisarlos a ojo: `cargo test -p yescharge dump_icons -- --ignored`.
    #[test]
    #[ignore]
    fn dump_icons() {
        let states = [
            TrayState { level: Some(80), charging: false, low: false },
            TrayState { level: Some(30), charging: false, low: false },
            TrayState { level: Some(10), charging: false, low: true },
            TrayState { level: Some(55), charging: true, low: false },
            TrayState { level: None, charging: false, low: false },
        ];
        let dir = std::env::temp_dir();
        for (i, st) in states.iter().enumerate() {
            for size in [16u32, 32, 96] {
                write_bmp(&tray_icon(size, *st), &dir.join(format!("yc-tray-{i}-{size}.bmp")));
            }
        }
        write_bmp(&app_icon(96), &dir.join("yc-app-96.bmp"));
    }

    /// Ningún nivel ni tamaño puede hacer fallar el dibujo (antes, la barra estrecha de batería baja entraba en pánico).
    #[test]
    fn tray_icon_never_panics() {
        for size in [16u32, 20, 24, 32, 48] {
            for level in 0..=100u8 {
                for charging in [false, true] {
                    tray_icon(size, TrayState { level: Some(level), charging, low: level < 15 });
                }
            }
            tray_icon(size, TrayState { level: None, charging: false, low: false });
        }
    }

    fn write_bmp(c: &Canvas, path: &std::path::Path) {
        let s = c.size;
        let rgba = c.rgba8();
        let mut px = Vec::new();
        for y in (0..s).rev() {
            for x in 0..s {
                let i = ((y * s + x) * 4) as usize;
                // fondo oscuro como la barra de tareas
                let a = rgba[i + 3] as u32;
                let blend = |v: u8, bg: u32| ((v as u32 * a + bg * (255 - a)) / 255) as u8;
                px.extend([blend(rgba[i + 2], 0x20), blend(rgba[i + 1], 0x20), blend(rgba[i], 0x20), 255]);
            }
        }
        let mut f = Vec::new();
        f.extend(b"BM");
        f.extend((54 + px.len() as u32).to_le_bytes());
        f.extend([0u8; 4]);
        f.extend(54u32.to_le_bytes());
        f.extend(40u32.to_le_bytes());
        f.extend((s as i32).to_le_bytes());
        f.extend((s as i32).to_le_bytes());
        f.extend(1u16.to_le_bytes());
        f.extend(32u16.to_le_bytes());
        f.extend([0u8; 24]);
        f.extend(px);
        std::fs::write(path, f).unwrap();
    }
}
