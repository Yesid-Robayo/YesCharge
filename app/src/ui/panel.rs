//! Panel rápido de la bandeja: los niveles de un vistazo, sin abrir la ventana completa. Es un
//! proceso más, que se cierra solo al perder el foco.

use std::time::{Duration, Instant};

use eframe::egui::{self, Rect, Sense, Stroke, pos2, vec2};
use yescharge_core::config::Config;
use yescharge_core::i18n::tr;
use yescharge_core::reading::Charge;
use yescharge_core::status::{self, DeviceStatus, Snapshot};

use super::theme::{self, *};
use super::widgets;
use crate::ipc::{self, wide};

const WIDTH: f32 = 320.0;
const ROW: f32 = 58.0;
/// Lo que ocupan el margen y el botón.
const CHROME: f32 = 84.0;
const MAX_ROWS: usize = 7;
/// Separación con el borde del escritorio.
const GAP: f32 = 12.0;
/// Si nunca llega a tener el foco, tampoco debe quedarse abierto.
const NEVER_FOCUSED: Duration = Duration::from_secs(15);

pub fn run() {
    unsafe {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let _m = CreateMutexW(std::ptr::null(), 1, wide("Local\\YesCharge.Panel").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        yescharge_core::i18n::set(Config::load().language);
        let devices = devices();
        if devices.is_empty() {
            ipc::spawn_self("--window");
            return;
        }
        let height = CHROME + ROW * devices.len().min(MAX_ROWS) as f32;
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("YesCharge · Panel")
                .with_inner_size([WIDTH, height])
                .with_position(corner(height))
                .with_decorations(false)
                .with_resizable(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_active(true),
            ..Default::default()
        };
        let panel = Panel { devices, opened: Instant::now(), last_check: Instant::now(), focused: false };
        let _ = eframe::run_native(
            "YesCharge · Panel",
            options,
            Box::new(|cc| {
                theme::apply(&cc.egui_ctx);
                Ok(Box::new(panel))
            }),
        );
    }
}

/// Esquina inferior derecha del escritorio, sobre la bandeja.
fn corner(height: f32) -> [f32; 2] {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW};
    unsafe {
        let mut area: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut RECT as *mut _, 0);
        let scale = GetDpiForSystem() as f32 / 96.0;
        [area.right as f32 / scale - WIDTH - GAP, area.bottom as f32 / scale - height - GAP]
    }
}

fn devices() -> Vec<DeviceStatus> {
    let cfg = Config::load();
    let ids: Vec<String> = cfg.devices.iter().map(|d| d.id.clone()).collect();
    status::ordered(&Snapshot::load().unwrap_or_default().devices, &ids, cfg.auto_sort)
}

struct Panel {
    devices: Vec<DeviceStatus>,
    opened: Instant,
    last_check: Instant,
    focused: bool,
}

impl eframe::App for Panel {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.last_check.elapsed() >= Duration::from_secs(2) {
            self.last_check = Instant::now();
            self.devices = devices();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        match ctx.input(|i| i.viewport().focused) {
            Some(true) => self.focused = true,
            Some(false) if self.focused => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ if !self.focused && self.opened.elapsed() >= NEVER_FOCUSED => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close)
            }
            _ => {}
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                let rows = ROW * self.devices.len().min(MAX_ROWS) as f32;
                egui::ScrollArea::vertical().max_height(rows).auto_shrink([false, true]).show(ui, |ui| {
                    for d in &self.devices {
                        row(ui, d);
                    }
                });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 10.0) / 2.0;
                        if widgets::secondary_button(ui, tr("Abrir YesCharge", "Open YesCharge"), width).clicked() {
                            ipc::spawn_self("--window");
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if widgets::secondary_button(ui, tr("Cerrar", "Close"), width).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

fn row(ui: &mut egui::Ui, d: &DeviceStatus) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let p = ui.painter();
    let (text, color) = super::status_of(d);
    let level = super::ring_color(d);
    let icon = Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(20.0, 20.0));
    widgets::device_glyph(p, icon, d.kind, MUTED);
    let x = rect.left() + 34.0;
    let width = rect.width() - 34.0 - 64.0;
    let name = p.layout_no_wrap(d.name.clone(), theme::semibold(14.5), TEXT);
    p.with_clip_rect(Rect::from_min_size(pos2(x, rect.top()), vec2(width, ROW))).galley(
        pos2(x, rect.center().y - 19.0),
        name,
        TEXT,
    );
    p.text(pos2(x, rect.center().y + 11.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), color);
    let right = pos2(rect.right() - 2.0, rect.center().y);
    let label = d.battery.map_or_else(|| "—".to_string(), |b| format!("{b}%"));
    p.text(right, egui::Align2::RIGHT_CENTER, label, theme::bold(18.0), level);
    if d.charge == Charge::Charging || d.topping_off {
        widgets::bolt(p, pos2(rect.right() - 58.0, rect.center().y), 13.0, CHARGE);
    }
    p.line_segment(
        [pos2(rect.left(), rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)],
        Stroke::new(1.0, SURFACE_2),
    );
}
