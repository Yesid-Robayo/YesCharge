//! Ajustes generales y "Acerca de".

use eframe::egui::{self, TextureHandle};
use yescharge_core::config::Config;

use super::theme::{self, *};
use super::{Nav, widgets};
use crate::ipc;

pub struct Settings {
    cfg: Config,
    original: Config,
    saved_at: Option<std::time::Instant>,
}

const POLL_CHOICES: [(u32, &str); 5] =
    [(60, "1 min"), (120, "2 min"), (300, "5 min"), (600, "10 min"), (900, "15 min")];

impl Settings {
    pub fn new() -> Self {
        let cfg = Config::load();
        Self { original: cfg.clone(), cfg, saved_at: None }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, logo: &TextureHandle) -> Nav {
        if super::page_header(ui, "Ajustes") {
            return Nav::Home;
        }
        let dirty = self.cfg != self.original;
        let bottom = if dirty { 60.0 } else { 0.0 };
        egui::ScrollArea::vertical().max_height(ui.available_height() - bottom).auto_shrink([false, false]).show(ui, |ui| {
            let c = &mut self.cfg;
            widgets::section_title(ui, "Avisos");
            widgets::card(ui, |ui| {
                widgets::setting_row(ui, "Batería baja", "Notificación de Windows al bajar del umbral", |ui| {
                    widgets::toggle(ui, &mut c.notify_low);
                });
                widgets::setting_row(ui, "Carga completa", "Avisar cuando ya puedes desconectarlo", |ui| {
                    widgets::toggle(ui, &mut c.notify_full);
                });
                let desc = format!("Cada {} min mientras siga baja", c.renotify_minutes);
                widgets::slider_row(ui, "Repetir el aviso de batería baja", &desc, &mut c.renotify_minutes, 5..=120, 5.0);
            });

            widgets::section_title(ui, "Lecturas");
            widgets::card(ui, |ui| {
                ui.label(egui::RichText::new("Consultar la batería cada").font(theme::semibold(14.5)));
                ui.horizontal_wrapped(|ui| {
                    for (secs, label) in POLL_CHOICES {
                        if widgets::choice_chip(ui, label, c.poll_seconds == secs).clicked() {
                            c.poll_seconds = secs;
                        }
                    }
                });
                ui.add_space(4.0);
                let desc = format!("No consultar tras {} min sin usar el PC (así no se despiertan)", c.idle_seconds / 60);
                widgets::slider_row(ui, "Reposo", &desc, &mut c.idle_seconds, 60..=1800, 60.0);
            });

            widgets::section_title(ui, "Inicio");
            widgets::card(ui, |ui| {
                widgets::setting_row(ui, "Iniciar con Windows", "Arranca en la bandeja, sin abrir la ventana", |ui| {
                    widgets::toggle(ui, &mut c.start_with_windows);
                });
            });

            widgets::section_title(ui, "API local (avanzado)");
            widgets::card(ui, |ui| {
                widgets::setting_row(ui, "Publicar el estado en HTTP", "Solo en 127.0.0.1, para pantallas externas", |ui| {
                    widgets::toggle(ui, &mut c.api_enabled);
                });
                if c.api_enabled {
                    widgets::setting_row(ui, "Puerto", "Rutas /v1/status y /min · se aplica al reiniciar", |ui| {
                        ui.add(egui::DragValue::new(&mut c.api_port).range(1024..=65535));
                    });
                }
            });

            widgets::section_title(ui, "Datos y diagnóstico");
            widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 10.0) / 2.0;
                    if widgets::secondary_button(ui, "Abrir carpeta de datos", w).clicked() {
                        ipc::open_external(&yescharge_core::paths::data_dir().display().to_string());
                    }
                    if widgets::secondary_button(ui, "Abrir el log", w).clicked() {
                        ipc::open_external(&yescharge_core::paths::log_file().display().to_string());
                    }
                });
                widgets::muted(ui, "Si algo falla, adjunta yescharge.log en un issue de GitHub.");
            });

            widgets::section_title(ui, "Acerca de");
            widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(44.0, 44.0)));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(egui::RichText::new("YesCharge").font(theme::bold(17.0)));
                        widgets::muted(ui, format!("Versión {} · Licencia MIT", yescharge_core::VERSION));
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Por defecto YesCharge solo lee. Solo le envía algo a un dispositivo si aceptas el aviso de riesgo, y entonces únicamente su pregunta de batería.")
                        .size(13.0)
                        .color(MUTED),
                );
                if ui.link("GitHub").clicked() {
                    ipc::open_external(yescharge_core::REPO_URL);
                }
            });

            ui.add_space(8.0);
            if widgets::danger_button(ui, "Cerrar YesCharge por completo", ui.available_width()).clicked() {
                ipc::send(ipc::MSG_QUIT);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.add_space(12.0);
        });
        if dirty {
            ui.add_space(8.0);
            if widgets::primary_button(ui, "Guardar ajustes", ui.available_width(), true).clicked()
                && super::save_config(&self.cfg)
            {
                self.original = self.cfg.clone();
                self.saved_at = Some(std::time::Instant::now());
                crate::autostart::apply(self.cfg.start_with_windows);
            }
        }
        Nav::Stay
    }
}
