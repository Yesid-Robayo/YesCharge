//! Paleta, tipografía y estilo de la ventana.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals,
};

pub const BG: Color32 = Color32::from_rgb(0x0e, 0x10, 0x16);
pub const SURFACE: Color32 = Color32::from_rgb(0x16, 0x19, 0x22);
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x1c, 0x20, 0x2b);
pub const SURFACE_2: Color32 = Color32::from_rgb(0x23, 0x28, 0x35);
pub const BORDER: Color32 = Color32::from_rgb(0x26, 0x2b, 0x38);
pub const TEXT: Color32 = Color32::from_rgb(0xe9, 0xec, 0xf2);
pub const MUTED: Color32 = Color32::from_rgb(0x8c, 0x93, 0xa6);
pub const FAINT: Color32 = Color32::from_rgb(0x5a, 0x61, 0x73);
pub const GOOD: Color32 = Color32::from_rgb(0x3d, 0xdc, 0x97);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xc4, 0x51);
pub const LOW: Color32 = Color32::from_rgb(0xff, 0x5d, 0x6c);
pub const CHARGE: Color32 = Color32::from_rgb(0x5a, 0xb0, 0xff);
pub const BRAND_A: Color32 = Color32::from_rgb(0x43, 0xe5, 0xa0);
pub const BRAND_B: Color32 = Color32::from_rgb(0x12, 0xa8, 0xc9);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x06, 0x1a, 0x12);

pub const SEMIBOLD: &str = "semibold";
pub const BOLD: &str = "bold";

pub fn level_color(level: u8, low: bool) -> Color32 {
    if low || level <= 15 {
        LOW
    } else if level <= 35 {
        WARN
    } else {
        GOOD
    }
}

pub fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// Usa Segoe UI de Windows (se lee de C:\Windows\Fonts, no se incluye en el exe). Si falta,
/// quedan las fuentes de egui.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::path::Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(key.into(), FontData::from_owned(bytes).into());
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let semi = load("segoe-sb", "seguisb.ttf");
    let bold = load("segoe-b", "segoeuib.ttf");
    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |main: Option<&str>| -> Vec<String> {
        let mut v: Vec<String> = main.map(|m| vec![m.to_string()]).unwrap_or_default();
        v.extend(fallback.iter().cloned());
        v
    };
    let base = regular.then_some("segoe");
    fonts.families.insert(FontFamily::Proportional, family(base));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family(if semi { Some("segoe-sb") } else { base }));
    fonts.families.insert(FontFamily::Name(BOLD.into()), family(if bold { Some("segoe-b") } else { base }));
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE_2;
    v.faint_bg_color = SURFACE;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = BRAND_A;
    v.selection.bg_fill = alpha(BRAND_A, 0.35);
    v.selection.stroke = Stroke::new(1.0, BRAND_A);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_stroke = Stroke::new(1.0, BORDER);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, SURFACE),
        (&mut v.widgets.inactive, SURFACE_2),
        (&mut v.widgets.hovered, lerp(SURFACE_2, TEXT, 0.08)),
        (&mut v.widgets.active, lerp(SURFACE_2, TEXT, 0.14)),
        (&mut v.widgets.open, SURFACE_2),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = CornerRadius::same(10);
        w.bg_stroke = Stroke::new(1.0, BORDER);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, alpha(BRAND_A, 0.6));
    v.widgets.active.bg_stroke = Stroke::new(1.0, BRAND_A);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, semibold(22.0)),
            (TextStyle::Body, FontId::proportional(14.5)),
            (TextStyle::Button, FontId::proportional(14.5)),
            (TextStyle::Small, FontId::proportional(12.5)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 8.0);
        s.spacing.interact_size.y = 34.0;
        s.spacing.window_margin = Margin::same(16);
        s.spacing.slider_width = 220.0;
        s.interaction.selectable_labels = false;
    });
}
