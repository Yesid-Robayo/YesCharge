//! Ajustes generales y "Acerca de".

use eframe::egui::{self, TextureHandle};
use yescharge_core::config::Config;
use yescharge_core::i18n::{Language, tr};
use yescharge_core::trf;

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
        if super::page_header(ui, tr("Ajustes", "Settings")) {
            return Nav::Home;
        }
        let dirty = self.cfg != self.original;
        let bottom = if dirty { 60.0 } else { 0.0 };
        egui::ScrollArea::vertical().max_height(ui.available_height() - bottom).auto_shrink([false, false]).show(ui, |ui| {
            let c = &mut self.cfg;
            widgets::section_title(ui, tr("Idioma", "Language"));
            widgets::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (language, label) in [(Language::Spanish, "Español"), (Language::English, "English")] {
                        if widgets::choice_chip(ui, label, c.language == language).clicked() {
                            c.language = language;
                        }
                    }
                });
            });

            widgets::section_title(ui, tr("Avisos", "Alerts"));
            widgets::card(ui, |ui| {
                widgets::setting_row(ui, tr("Batería baja", "Low battery"), tr("Notificación de Windows al bajar del umbral", "Windows notification when it drops below the threshold"), |ui| {
                    widgets::toggle(ui, &mut c.notify_low);
                });
                widgets::setting_row(ui, tr("Carga completa", "Fully charged"), tr("Avisar cuando ya puedes desconectarlo", "Tell me when I can unplug it"), |ui| {
                    widgets::toggle(ui, &mut c.notify_full);
                });
                let desc = trf!("Cada {} min mientras siga baja", "Every {} min while it stays low", c.renotify_minutes);
                widgets::slider_row(ui, tr("Repetir el aviso de batería baja", "Repeat the low battery alert"), &desc, &mut c.renotify_minutes, 5..=120, 5.0);
            });

            widgets::section_title(ui, tr("Lecturas", "Readings"));
            widgets::card(ui, |ui| {
                ui.label(egui::RichText::new(tr("Consultar la batería cada", "Check the battery every")).font(theme::semibold(14.5)));
                ui.horizontal_wrapped(|ui| {
                    for (secs, label) in POLL_CHOICES {
                        if widgets::choice_chip(ui, label, c.poll_seconds == secs).clicked() {
                            c.poll_seconds = secs;
                        }
                    }
                });
                ui.add_space(4.0);
                let desc = trf!("No consultar tras {} min sin usar el PC (así no se despiertan)", "Don't check after {} min without using the PC (so they don't wake up)", c.idle_seconds / 60);
                widgets::slider_row(ui, tr("Reposo", "Idle"), &desc, &mut c.idle_seconds, 60..=1800, 60.0);
            });

            widgets::section_title(ui, tr("Inicio", "Startup"));
            widgets::card(ui, |ui| {
                widgets::setting_row(ui, tr("Iniciar con Windows", "Start with Windows"), tr("Arranca en la bandeja, sin abrir la ventana", "Starts in the tray, without opening the window"), |ui| {
                    widgets::toggle(ui, &mut c.start_with_windows);
                });
            });

            widgets::section_title(ui, tr("API local (avanzado)", "Local API (advanced)"));
            widgets::card(ui, |ui| {
                widgets::setting_row(ui, tr("Publicar el estado en HTTP", "Publish the status over HTTP"), tr("Solo en 127.0.0.1, para pantallas externas", "Only on 127.0.0.1, for external screens"), |ui| {
                    widgets::toggle(ui, &mut c.api_enabled);
                });
                if c.api_enabled {
                    widgets::setting_row(ui, tr("Puerto", "Port"), tr("Rutas /v1/status y /min · se aplica al reiniciar", "Routes /v1/status and /min · applied on restart"), |ui| {
                        ui.add(egui::DragValue::new(&mut c.api_port).range(1024..=65535));
                    });
                }
            });

            widgets::section_title(ui, tr("Datos y diagnóstico", "Data and diagnostics"));
            widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 10.0) / 2.0;
                    if widgets::secondary_button(ui, tr("Abrir carpeta de datos", "Open data folder"), w).clicked() {
                        ipc::open_external(&yescharge_core::paths::data_dir().display().to_string());
                    }
                    if widgets::secondary_button(ui, tr("Abrir el log", "Open the log"), w).clicked() {
                        ipc::open_external(&yescharge_core::paths::log_file().display().to_string());
                    }
                });
                widgets::muted(ui, tr("Si algo falla, adjunta yescharge.log en un issue de GitHub.", "If something fails, attach yescharge.log to a GitHub issue."));
            });

            widgets::section_title(ui, tr("Acerca de", "About"));
            widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(44.0, 44.0)));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(egui::RichText::new("YesCharge").font(theme::bold(17.0)));
                        widgets::muted(ui, trf!("Versión {} · Licencia MIT", "Version {} · MIT license", yescharge_core::VERSION));
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(tr("Por defecto YesCharge solo lee. Solo le envía algo a un dispositivo si aceptas el aviso de riesgo, y entonces únicamente su pregunta de batería.", "By default YesCharge only reads. It only sends something to a device if you accept the risk warning, and then only its battery request."))
                        .size(13.0)
                        .color(MUTED),
                );
                if ui.link("GitHub").clicked() {
                    ipc::open_external(yescharge_core::REPO_URL);
                }
            });

            ui.add_space(8.0);
            if widgets::danger_button(ui, tr("Cerrar YesCharge por completo", "Quit YesCharge completely"), ui.available_width()).clicked() {
                ipc::send(ipc::MSG_QUIT);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.add_space(12.0);
        });
        if dirty {
            ui.add_space(8.0);
            if widgets::primary_button(ui, tr("Guardar ajustes", "Save settings"), ui.available_width(), true).clicked()
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
