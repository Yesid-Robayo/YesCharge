//! Detalles de un dispositivo: estado, gráfica de 24 h y sus ajustes.

use eframe::egui::{self, Align2, Rect, Sense, Stroke, pos2, vec2};
use yescharge_core::config::{Config, DeviceEntry, Source};
use yescharge_core::i18n::tr;
use yescharge_core::reading::{Charge, DeviceKind, Presence};
use yescharge_core::status::DeviceStatus;
use yescharge_core::trf;

use super::theme::{self, *};
use super::{Nav, widgets};

pub struct Detail {
    pub id: String,
    name: String,
    kind: DeviceKind,
    threshold: u8,
    top_off: u32,
    enabled: bool,
    source: String,
    asks: bool,
    confirm_remove: bool,
    saved_at: Option<std::time::Instant>,
}

impl Detail {
    pub fn new(e: &DeviceEntry) -> Self {
        Self {
            id: e.id.clone(),
            name: e.name.clone(),
            kind: e.kind,
            threshold: e.low_threshold,
            top_off: e.top_off_minutes,
            enabled: e.enabled,
            source: e.source.label(),
            asks: matches!(e.source, Source::Ask { .. }),
            confirm_remove: false,
            saved_at: None,
        }
    }

    fn save(&mut self) {
        let mut cfg = Config::load();
        if let Some(e) = cfg.devices.iter_mut().find(|e| e.id == self.id) {
            e.name = if self.name.trim().is_empty() { e.name.clone() } else { self.name.trim().to_string() };
            e.kind = self.kind;
            e.low_threshold = self.threshold;
            e.top_off_minutes = self.top_off;
            e.enabled = self.enabled;
        }
        if super::save_config(&cfg) {
            self.saved_at = Some(std::time::Instant::now());
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, status: Option<&DeviceStatus>, now: f64) -> Nav {
        if super::page_header(ui, tr("Detalles", "Details")) {
            return Nav::Home;
        }
        let mut nav = Nav::Stay;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // cabecera con anillo grande
            ui.vertical_centered(|ui| {
                let (rect, resp) = ui.allocate_exact_size(vec2(150.0, 150.0), Sense::hover());
                let (level, label, color) = match status {
                    Some(d) => (
                        d.battery.map(|b| ui.ctx().animate_value_with_time(resp.id, b as f32 / 100.0, 0.7)),
                        d.battery.map(|b| format!("{b}%")).unwrap_or_else(|| "—".into()),
                        super::ring_color(d),
                    ),
                    None => (None, "—".into(), FAINT),
                };
                let sub = status
                    .filter(|d| d.charge == Charge::Charging || d.topping_off)
                    .map(|_| tr("cargando", "charging"));
                widgets::ring(ui, rect, level, color, &label, sub);
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&self.name).font(theme::bold(20.0)));
                if let Some(d) = status {
                    let (text, col) = super::status_of(d);
                    widgets::chip(ui, text, col);
                    if !d.eta.is_empty() {
                        widgets::muted(ui, &d.eta);
                    }
                    if d.presence == Presence::Stale && d.last_seen > 0.0 {
                        widgets::muted(
                            ui,
                            trf!(
                                "Último mensaje del dispositivo: {}",
                                "Last message from the device: {}",
                                super::ago(now - d.last_seen)
                            ),
                        );
                    }
                }
            });
            ui.add_space(8.0);

            if let Some(d) = status {
                widgets::section_title(ui, tr("Últimas 24 horas", "Last 24 hours"));
                widgets::card(ui, |ui| history_chart(ui, d, now));
            }

            widgets::section_title(ui, tr("Ajustes", "Settings"));
            widgets::card(ui, |ui| {
                ui.label(egui::RichText::new(tr("Nombre", "Name")).font(theme::semibold(13.5)));
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY).char_limit(48));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tr("Tipo", "Type")).font(theme::semibold(13.5)));
                ui.horizontal_wrapped(|ui| {
                    for k in DeviceKind::ALL {
                        if widgets::choice_chip(ui, k.label(), self.kind == k).clicked() {
                            self.kind = k;
                        }
                    }
                });
                ui.add_space(6.0);
                let desc = trf!("Cuando baje del {} %", "When it drops below {} %", self.threshold);
                widgets::slider_row(
                    ui,
                    tr("Avisar con batería baja", "Alert on low battery"),
                    &desc,
                    &mut self.threshold,
                    5..=50,
                    1.0,
                );
                ui.add_space(4.0);
                let desc = trf!(
                    "Avisar \"cargado\" {} min después de llegar al 100 %",
                    "Say \"charged\" {} min after reaching 100 %",
                    self.top_off
                );
                widgets::slider_row(
                    ui,
                    tr("Margen de carga tras el 100 %", "Charge margin after 100 %"),
                    &desc,
                    &mut self.top_off,
                    0..=120,
                    5.0,
                );
                ui.add_space(4.0);
                widgets::setting_row(
                    ui,
                    tr("Vigilar este dispositivo", "Watch this device"),
                    tr(
                        "Si lo desactivas, no se consulta ni avisa",
                        "If you turn it off, it isn't checked and gives no alerts",
                    ),
                    |ui| {
                        widgets::toggle(ui, &mut self.enabled);
                    },
                );
                ui.add_space(6.0);
                let label = if self.saved_at.is_some_and(|t| t.elapsed().as_secs() < 2) {
                    tr("Guardado", "Saved")
                } else {
                    tr("Guardar cambios", "Save changes")
                };
                if widgets::primary_button(ui, label, ui.available_width(), true).clicked() {
                    self.save();
                }
            });

            widgets::section_title(ui, tr("Cómo se lee", "How it is read"));
            widgets::card(ui, |ui| {
                ui.label(egui::RichText::new(&self.source).font(theme::semibold(14.0)));
                if self.asks {
                    widgets::muted(
                        ui,
                        tr(
                            "Le envía una pregunta de batería: aceptaste ese riesgo.",
                            "It sends it a battery request: you accepted that risk.",
                        ),
                    );
                } else {
                    widgets::muted(ui, tr("Solo lectura: no se le envía nada.", "Read-only: nothing is sent to it."));
                }
                if status.is_some_and(|d| d.presence == Presence::Paused)
                    && widgets::secondary_button(ui, tr("Volver a preguntarle", "Ask it again"), ui.available_width())
                        .clicked()
                {
                    crate::ipc::send(crate::ipc::MSG_RESUME);
                }
            });

            ui.add_space(8.0);
            if !self.confirm_remove {
                if widgets::danger_button(ui, tr("Quitar dispositivo", "Remove device"), ui.available_width()).clicked()
                {
                    self.confirm_remove = true;
                }
            } else {
                widgets::card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(tr("¿Quitar este dispositivo?", "Remove this device?"))
                            .font(theme::semibold(14.5)),
                    );
                    widgets::muted(
                        ui,
                        tr(
                            "Se borra también su historial y la autonomía aprendida.",
                            "Its history and learned battery life are deleted too.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 10.0) / 2.0;
                        if widgets::secondary_button(ui, tr("Cancelar", "Cancel"), w).clicked() {
                            self.confirm_remove = false;
                        }
                        if widgets::danger_button(ui, tr("Sí, quitar", "Yes, remove"), w).clicked() {
                            let mut cfg = Config::load();
                            cfg.devices.retain(|e| e.id != self.id);
                            super::save_config(&cfg);
                            nav = Nav::Home;
                        }
                    });
                });
            }
            ui.add_space(12.0);
        });
        nav
    }
}

fn history_chart(ui: &mut egui::Ui, d: &DeviceStatus, now: f64) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 130.0), Sense::hover());
    let p = ui.painter();
    let plot =
        Rect::from_min_max(pos2(rect.left() + 34.0, rect.top() + 6.0), pos2(rect.right() - 4.0, rect.bottom() - 18.0));
    for v in [0u8, 50, 100] {
        let y = plot.bottom() - plot.height() * v as f32 / 100.0;
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, SURFACE_2));
        p.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{v}%"),
            egui::FontId::proportional(10.5),
            FAINT,
        );
    }
    for (h, label) in [(24.0, "-24 h"), (12.0, "-12 h"), (0.0, tr("ahora", "now"))] {
        let x = plot.right() - plot.width() * (h / 24.0);
        let align = if h == 24.0 {
            Align2::LEFT_CENTER
        } else if h == 0.0 {
            Align2::RIGHT_CENTER
        } else {
            Align2::CENTER_CENTER
        };
        p.text(pos2(x, rect.bottom() - 6.0), align, label, egui::FontId::proportional(10.5), FAINT);
    }
    if d.history.len() >= 2 {
        widgets::sparkline(ui, plot, &d.history, now, super::ring_color(d));
    } else {
        p.text(
            plot.center(),
            Align2::CENTER_CENTER,
            tr("Todavía no hay suficientes lecturas", "Not enough readings yet"),
            egui::FontId::proportional(12.5),
            MUTED,
        );
    }
}
