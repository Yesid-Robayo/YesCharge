//! Componentes dibujados a mano: anillo de batería, iconos vectoriales, gráfica de 24 h,
//! interruptor, chips y botones.

use std::f32::consts::{PI, TAU};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use yescharge_core::reading::DeviceKind;

use super::theme::{self, *};

fn arc_points(center: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    let n = (((to - from).abs() / TAU) * 96.0).ceil().max(2.0) as usize;
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            pos2(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect()
}

/// Anillo con el porcentaje en el centro. `level` en 0..1 (animado por quien llama).
pub fn ring(ui: &Ui, rect: Rect, level: Option<f32>, color: Color32, label: &str, sub: Option<&str>) {
    let p = ui.painter();
    let c = rect.center();
    let w = (rect.width() * 0.09).max(4.0);
    let r = rect.width() / 2.0 - w / 2.0 - 1.0;
    p.circle_stroke(c, r, Stroke::new(w, SURFACE_2));
    if let Some(l) = level.filter(|l| *l > 0.0) {
        let start = -PI / 2.0;
        let end = start + TAU * l.clamp(0.0, 1.0);
        let pts = arc_points(c, r, start, end);
        // degradado sutil a lo largo del arco: segmentos con el color aclarándose
        for (i, seg) in pts.windows(2).enumerate() {
            let t = i as f32 / pts.len() as f32;
            let col = theme::lerp(color, Color32::WHITE, 0.18 * t);
            p.line_segment([seg[0], seg[1]], Stroke::new(w, col));
        }
        p.circle_filled(pts[0], w / 2.0, color);
        p.circle_filled(*pts.last().unwrap_or(&pts[0]), w / 2.0, theme::lerp(color, Color32::WHITE, 0.18));
    }
    let size = rect.width() * if label.chars().count() >= 4 { 0.23 } else { 0.27 };
    let y = if sub.is_some() { c.y - size * 0.18 } else { c.y };
    p.text(pos2(c.x, y), Align2::CENTER_CENTER, label, theme::bold(size), TEXT);
    if let Some(s) = sub {
        p.text(pos2(c.x, y + size * 0.85), Align2::CENTER_CENTER, s, FontId::proportional(size * 0.42), MUTED);
    }
}

/// Rayo relleno (para "cargando").
pub fn bolt(p: &egui::Painter, center: Pos2, h: f32, color: Color32) {
    let pts: Vec<Pos2> =
        [(0.06, -0.5), (0.26, -0.5), (0.10, -0.06), (0.30, -0.06), (-0.12, 0.5), (0.0, 0.06), (-0.2, 0.06)]
            .iter()
            .map(|(x, y)| pos2(center.x + x * h, center.y + y * h))
            .collect();
    // dos triángulos convexos para que se rellene bien
    p.add(Shape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[5], pts[6]], color, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![pts[2], pts[3], pts[4], pts[5]], color, Stroke::NONE));
}

/// Icono vectorial del tipo de dispositivo dentro de `rect`.
pub fn device_glyph(p: &egui::Painter, rect: Rect, kind: DeviceKind, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let st = Stroke::new((s * 0.085).max(1.4), color);
    let rr = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        Rect::from_min_max(pos2(c.x + x0 * s, c.y + y0 * s), pos2(c.x + x1 * s, c.y + y1 * s)).with_corner(r * s)
    };
    match kind {
        DeviceKind::Headset => {
            p.add(Shape::line(arc_points(pos2(c.x, c.y + 0.05 * s), 0.34 * s, PI, TAU), st));
            for x in [-0.38, 0.22] {
                let (r, cr) = rr(x, 0.0, x + 0.16, 0.34, 0.07);
                p.rect_filled(r, cr, color);
            }
        }
        DeviceKind::Mouse => {
            let (r, cr) = rr(-0.24, -0.38, 0.24, 0.38, 0.24);
            p.rect_stroke(r, cr, st, StrokeKind::Middle);
            p.line_segment([pos2(c.x, c.y - 0.36 * s), pos2(c.x, c.y - 0.08 * s)], st);
        }
        DeviceKind::Keyboard => {
            let (r, cr) = rr(-0.44, -0.24, 0.44, 0.24, 0.08);
            p.rect_stroke(r, cr, st, StrokeKind::Middle);
            for row in 0..2 {
                for col in 0..5 {
                    let x = c.x + (-0.27 + col as f32 * 0.135) * s;
                    let y = c.y + (-0.1 + row as f32 * 0.12) * s;
                    p.circle_filled(pos2(x, y), s * 0.035, color);
                }
            }
            p.line_segment([pos2(c.x - 0.16 * s, c.y + 0.13 * s), pos2(c.x + 0.16 * s, c.y + 0.13 * s)], st);
        }
        DeviceKind::Gamepad => {
            let (r, cr) = rr(-0.44, -0.22, 0.44, 0.26, 0.2);
            p.rect_stroke(r, cr, st, StrokeKind::Middle);
            let d = pos2(c.x - 0.2 * s, c.y + 0.02 * s);
            p.line_segment([pos2(d.x - 0.09 * s, d.y), pos2(d.x + 0.09 * s, d.y)], st);
            p.line_segment([pos2(d.x, d.y - 0.09 * s), pos2(d.x, d.y + 0.09 * s)], st);
            p.circle_filled(pos2(c.x + 0.17 * s, c.y - 0.03 * s), s * 0.05, color);
            p.circle_filled(pos2(c.x + 0.27 * s, c.y + 0.07 * s), s * 0.05, color);
        }
        DeviceKind::Other => {
            let (r, cr) = rr(-0.38, -0.2, 0.3, 0.2, 0.07);
            p.rect_stroke(r, cr, st, StrokeKind::Middle);
            let (r, cr) = rr(0.32, -0.08, 0.4, 0.08, 0.02);
            p.rect_filled(r, cr, color);
            bolt(p, pos2(c.x - 0.04 * s, c.y), 0.3 * s, color);
        }
    }
}

trait WithCorner {
    fn with_corner(self, r: f32) -> (Rect, CornerRadius);
}

impl WithCorner for Rect {
    fn with_corner(self, r: f32) -> (Rect, CornerRadius) {
        (self, CornerRadius::same(r.round().clamp(0.0, 255.0) as u8))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Refresh,
    Settings,
    Back,
    Check,
}

pub fn glyph(p: &egui::Painter, rect: Rect, g: Glyph, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let st = Stroke::new((s * 0.11).max(1.5), color);
    match g {
        Glyph::Refresh => {
            let a1 = PI * 1.45;
            let pts = arc_points(c, 0.3 * s, -PI * 0.1, a1);
            let tip = *pts.last().unwrap_or(&c);
            p.add(Shape::line(pts, st));
            // punta de flecha orientada según la tangente del arco
            let t = vec2(-a1.sin(), a1.cos());
            let n = vec2(a1.cos(), a1.sin());
            p.add(Shape::convex_polygon(
                vec![tip + t * 0.2 * s, tip + n * 0.15 * s, tip - n * 0.15 * s],
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Settings => {
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                let (x, y) = (a.cos(), a.sin());
                p.line_segment(
                    [pos2(c.x + x * 0.26 * s, c.y + y * 0.26 * s), pos2(c.x + x * 0.4 * s, c.y + y * 0.4 * s)],
                    Stroke::new(st.width * 1.5, color),
                );
            }
            p.circle_stroke(c, 0.25 * s, st);
            p.circle_stroke(c, 0.09 * s, st);
        }
        Glyph::Back => {
            p.add(Shape::line(
                vec![
                    pos2(c.x + 0.1 * s, c.y - 0.28 * s),
                    pos2(c.x - 0.18 * s, c.y),
                    pos2(c.x + 0.1 * s, c.y + 0.28 * s),
                ],
                st,
            ));
        }
        Glyph::Check => {
            p.add(Shape::line(
                vec![
                    pos2(c.x - 0.28 * s, c.y),
                    pos2(c.x - 0.07 * s, c.y + 0.22 * s),
                    pos2(c.x + 0.3 * s, c.y - 0.24 * s),
                ],
                st,
            ));
        }
    }
}

/// Botón redondo con un icono.
pub fn icon_button(ui: &mut Ui, g: Glyph, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let fill = theme::lerp(SURFACE, SURFACE_2, 0.4 + hover * 0.6);
    ui.painter().circle_filled(rect.center(), 18.0, fill);
    glyph(ui.painter(), rect.shrink(9.0), g, theme::lerp(MUTED, TEXT, hover));
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tooltip)
}

/// Botón principal (degradado de marca) a lo ancho de `width`.
pub fn primary_button(ui: &mut Ui, text: &str, width: f32, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && enabled, 0.12);
    let p = ui.painter();
    if enabled {
        let (a, b) =
            (theme::lerp(BRAND_A, Color32::WHITE, hover * 0.08), theme::lerp(BRAND_B, Color32::WHITE, hover * 0.08));
        gradient_rounded_rect(p, rect, 12.0, a, b);
        p.text(rect.center(), Align2::CENTER_CENTER, text, theme::semibold(15.0), ON_ACCENT);
    } else {
        p.rect_filled(rect, CornerRadius::same(12), SURFACE_2);
        p.text(rect.center(), Align2::CENTER_CENTER, text, theme::semibold(15.0), FAINT);
    }
    if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp }
}

/// Rectángulo redondeado con degradado horizontal de `a` a `b` (abanico de triángulos).
pub fn gradient_rounded_rect(p: &egui::Painter, rect: Rect, r: f32, a: Color32, b: Color32) {
    let r = r.min(rect.height() / 2.0).min(rect.width() / 2.0);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -PI / 2.0),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), PI / 2.0),
        (pos2(rect.left() + r, rect.top() + r), PI),
    ];
    let mut outline = Vec::new();
    for (center, start) in corners {
        outline.extend(arc_points(center, r, start, start + PI / 2.0));
    }
    let color_at = |p: Pos2| theme::lerp(a, b, (p.x - rect.left()) / rect.width());
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), color_at(rect.center()));
    for p in &outline {
        mesh.colored_vertex(*p, color_at(*p));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
}

/// Botón secundario (contorno).
pub fn secondary_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(12),
        theme::lerp(SURFACE, SURFACE_2, hover),
        Stroke::new(1.0, theme::lerp(BORDER, MUTED, hover)),
        StrokeKind::Inside,
    );
    p.text(rect.center(), Align2::CENTER_CENTER, text, theme::semibold(14.0), TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Botón de texto peligroso (rojo).
pub fn danger_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        theme::alpha(LOW, 0.08 + hover * 0.1),
        Stroke::new(1.0, theme::alpha(LOW, 0.5)),
        StrokeKind::Inside,
    );
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, theme::semibold(14.0), LOW);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Píldora de estado.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = theme::semibold(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + vec2(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), theme::alpha(color, 0.14));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Chip seleccionable (para elegir el tipo de dispositivo).
pub fn choice_chip(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let font = theme::semibold(13.0);
    let color = if selected { ON_ACCENT } else { TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + vec2(22.0, 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let fill = if selected { BRAND_A } else { theme::lerp(SURFACE_2, BORDER, hover) };
    ui.painter().rect_filled(rect, CornerRadius::same(255), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Interruptor tipo Windows 11.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
    let p = ui.painter();
    let fill = theme::lerp(SURFACE_2, BRAND_A, t);
    p.rect(rect, CornerRadius::same(12), fill, Stroke::new(1.0, theme::lerp(MUTED, BRAND_A, t)), StrokeKind::Inside);
    let x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), t);
    p.circle_filled(pos2(x, rect.center().y), 7.0, theme::lerp(TEXT, ON_ACCENT, t));
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Gráfica de las últimas `hours` horas: línea + relleno degradado.
pub fn sparkline(ui: &Ui, rect: Rect, pts: &[(f64, u8)], now: f64, hours: f64, color: Color32) {
    if pts.len() < 2 {
        return;
    }
    let span = hours * 3600.0;
    let to_pos = |(t, v): (f64, u8)| {
        let x = rect.left() + rect.width() * (1.0 - ((now - t) / span).clamp(0.0, 1.0) as f32);
        let y = rect.bottom() - rect.height() * (v as f32 / 100.0);
        pos2(x, y)
    };
    let line: Vec<Pos2> = pts.iter().map(|&p| to_pos(p)).collect();
    let mut mesh = Mesh::default();
    for (i, p) in line.iter().enumerate() {
        mesh.colored_vertex(*p, theme::alpha(color, 0.28));
        mesh.colored_vertex(pos2(p.x, rect.bottom()), theme::alpha(color, 0.0));
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter().add(Shape::line(line, Stroke::new(2.0, color)));
}

/// Fila de ajustes: título + descripción a la izquierda, control a la derecha.
pub fn setting_row(ui: &mut Ui, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(egui::RichText::new(title).font(theme::semibold(14.5)));
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).size(12.5).color(MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

/// Ajuste con deslizador: título y valor arriba, deslizador a lo ancho debajo.
pub fn slider_row<N: egui::emath::Numeric>(
    ui: &mut Ui,
    title: &str,
    desc: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    step: f64,
) -> Response {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(egui::RichText::new(title).font(theme::semibold(14.5)));
        ui.label(egui::RichText::new(desc).size(12.5).color(MUTED));
        ui.add_space(4.0);
        ui.spacing_mut().slider_width = ui.available_width() - 4.0;
        ui.visuals_mut().slider_trailing_fill = true;
        ui.visuals_mut().selection.bg_fill = BRAND_A;
        let mut s = egui::Slider::new(value, range).show_value(false).handle_shape(egui::style::HandleShape::Circle);
        if step > 0.0 {
            s = s.step_by(step);
        }
        ui.add(s)
    })
    .inner
}

/// Contenedor tipo tarjeta.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Texto pequeño y apagado.
pub fn muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).size(12.5).color(MUTED));
}

pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text.to_uppercase()).font(theme::semibold(11.5)).color(FAINT).extra_letter_spacing(1.2),
    );
}
