//! Ventana de YesCharge (proceso aparte del servicio: solo existe mientras está abierta).

mod detail;
mod settings;
mod theme;
mod widgets;
mod wizard;

use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2};
use yescharge_core::config::Config;
use yescharge_core::i18n::tr;
use yescharge_core::reading::{Charge, Presence};
use yescharge_core::status::{self, DeviceStatus, Snapshot};
use yescharge_core::trf;

use crate::ipc::{self, wide};
use theme::*;
use widgets::Glyph;

const WINDOW_TITLE: &str = "YesCharge";
/// Separación mínima entre dos pulsaciones de "Actualizar".
const REFRESH_GAP: Duration = Duration::from_secs(3);
/// Tiempo mínimo de "Actualizando", para que se note que hizo algo aunque los datos no cambien.
const REFRESH_MIN: Duration = Duration::from_millis(700);
/// Si el servicio no responde en este tiempo, se deja de esperar.
const REFRESH_MAX: Duration = Duration::from_secs(12);

pub fn run() {
    unsafe {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow};
        let _m = CreateMutexW(std::ptr::null(), 1, wide("Local\\YesCharge.Window").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // ya hay una ventana: traerla al frente
            let h = FindWindowW(std::ptr::null(), wide(WINDOW_TITLE).as_ptr());
            if !h.is_null() {
                ShowWindow(h, SW_RESTORE);
                SetForegroundWindow(h);
            }
            return;
        }
        yescharge_core::log::init("ventana");
        yescharge_core::i18n::set(Config::load().language);
        let icon = crate::icon::app_icon(64);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(WINDOW_TITLE)
                .with_inner_size([460.0, 720.0])
                .with_min_inner_size([420.0, 560.0])
                .with_icon(egui::IconData { rgba: icon.rgba8(), width: 64, height: 64 }),
            centered: true,
            ..Default::default()
        };
        if let Err(e) = eframe::run_native(WINDOW_TITLE, options, Box::new(|cc| Ok(Box::new(App::new(cc))))) {
            yescharge_core::log::error(&format!("la ventana no pudo arrancar: {e}"));
        }
    }
}

/// Barra de título oscura y del mismo color que el fondo (Windows 11).
fn style_title_bar(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    let Ok(handle) = cc.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = h.hwnd.get() as *mut core::ffi::c_void;
    let colorref = |c: Color32| (c.r() as u32) | ((c.g() as u32) << 8) | ((c.b() as u32) << 16);
    unsafe {
        let dark: i32 = 1;
        DwmSetWindowAttribute(hwnd, 20, &dark as *const i32 as *const _, 4); // DWMWA_USE_IMMERSIVE_DARK_MODE
        let caption = colorref(BG);
        DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as *const _, 4); // DWMWA_CAPTION_COLOR
        let border = colorref(BORDER);
        DwmSetWindowAttribute(hwnd, 34, &border as *const u32 as *const _, 4); // DWMWA_BORDER_COLOR
    }
}

enum Page {
    Home,
    Detail(Box<detail::Detail>),
    Add(Box<wizard::Wizard>),
    Settings(Box<settings::Settings>),
}

/// Dispositivo que se está arrastrando y cómo va quedando la lista.
struct Drag {
    id: String,
    order: Vec<String>,
}

pub enum Nav {
    Stay,
    Home,
}

struct App {
    page: Page,
    snapshot: Snapshot,
    snap_mtime: Option<SystemTime>,
    last_check: Instant,
    cfg: Config,
    service_up: bool,
    logo: TextureHandle,
    last_refresh: Option<Instant>,
    /// Desde cuándo se espera un `status.json` más nuevo que el de ese momento.
    refreshing: Option<(Instant, f64)>,
    drag: Option<Drag>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        style_title_bar(cc);
        let icon = crate::icon::app_icon(96);
        let logo = cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([96, 96], &icon.rgba8()),
            egui::TextureOptions::LINEAR,
        );
        let cfg = Config::load();
        #[cfg_attr(not(debug_assertions), allow(unused_mut))]
        let mut page = if cfg.devices.is_empty() { Page::Add(Box::new(wizard::Wizard::new())) } else { Page::Home };
        // Solo en desarrollo: abrir una pantalla concreta con datos de ejemplo para revisarla.
        #[cfg(debug_assertions)]
        if let Ok(demo) = std::env::var("YESCHARGE_DEMO_PAGE") {
            page = match demo.as_str() {
                "settings" => Page::Settings(Box::new(settings::Settings::new())),
                "detail" => {
                    cfg.devices.first().map(|e| Page::Detail(Box::new(detail::Detail::new(e)))).unwrap_or(Page::Home)
                }
                other => Page::Add(Box::new(wizard::Wizard::demo(other))),
            };
        }
        let mut app = Self {
            page,
            snapshot: Snapshot::default(),
            snap_mtime: None,
            last_check: Instant::now() - Duration::from_secs(10),
            cfg,
            service_up: ipc::service_running(),
            logo,
            last_refresh: None,
            refreshing: None,
            drag: None,
        };
        app.poll_files();
        app
    }

    /// Relee `status.json` si cambió y comprueba que el servicio siga vivo (cada segundo).
    fn poll_files(&mut self) {
        let gap = if self.refreshing.is_some() { 200 } else { 900 };
        if self.last_check.elapsed() < Duration::from_millis(gap) {
            return;
        }
        self.last_check = Instant::now();
        self.service_up = ipc::service_running();
        let mtime = std::fs::metadata(yescharge_core::paths::status_file()).and_then(|m| m.modified()).ok();
        if mtime != self.snap_mtime {
            self.snap_mtime = mtime;
            if let Some(s) = Snapshot::load() {
                self.snapshot = s;
            }
            self.cfg = Config::load();
        }
    }

    fn finish_refresh(&mut self) {
        let Some((since, from)) = self.refreshing else { return };
        let arrived = self.snapshot.updated_at > from && since.elapsed() >= REFRESH_MIN;
        if arrived || since.elapsed() >= REFRESH_MAX {
            self.refreshing = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_files();
        self.finish_refresh();
        let ctx = ui.ctx().clone();
        let mut next: Option<Page> = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(20, 16)))
            .show(ui, |ui| match &mut self.page {
                Page::Home => next = home(ui, self),
                Page::Detail(d) => {
                    let status = self.snapshot.devices.iter().find(|s| s.id == d.id).cloned();
                    if let Nav::Home = d.show(ui, status.as_ref(), self.snapshot.updated_at) {
                        next = Some(Page::Home);
                    }
                }
                Page::Add(w) => {
                    if let Nav::Home = w.show(ui, &self.logo) {
                        next = Some(Page::Home);
                    }
                }
                Page::Settings(s) => {
                    if let Nav::Home = s.show(ui, &self.logo) {
                        next = Some(Page::Home);
                    }
                }
            });
        if let Some(p) = next {
            if matches!(p, Page::Home) {
                self.last_check = Instant::now() - Duration::from_secs(10);
                self.snap_mtime = None;
            }
            self.page = p;
        }
        if matches!(&self.page, Page::Add(w) if w.busy()) || self.refreshing.is_some() {
            // animación continua a ~60 fps, solo en estas pantallas (sin tope iría al ritmo del
            // monitor, que puede ser de 240 Hz, gastando CPU sin verse mejor)
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

fn header(ui: &mut egui::Ui, logo: &TextureHandle, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(38.0, 38.0)));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(egui::RichText::new(title).font(theme::bold(21.0)));
            ui.label(egui::RichText::new(subtitle).size(12.5).color(MUTED));
        });
    });
}

pub fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..=59 => tr("hace unos segundos", "a few seconds ago").into(),
        60..=3599 => trf!("hace {} min", "{} min ago", s / 60),
        3600..=86399 => trf!("hace {} h", "{} h ago", s / 3600),
        _ => trf!("hace {} d", "{} d ago", s / 86400),
    }
}

/// Texto y color del estado de un dispositivo.
pub fn status_of(d: &DeviceStatus) -> (&'static str, Color32) {
    if d.full {
        (tr("Cargado", "Charged"), GOOD)
    } else if d.topping_off {
        (tr("Completando carga", "Topping off"), CHARGE)
    } else if d.charge == Charge::Charging {
        (tr("Cargando", "Charging"), CHARGE)
    } else if d.low {
        (tr("Batería baja", "Low battery"), LOW)
    } else {
        match d.presence {
            Presence::Online => (tr("En uso", "In use"), MUTED),
            Presence::Asleep => (tr("Dormido o apagado", "Asleep or off"), FAINT),
            Presence::Off => (tr("Apagado", "Off"), FAINT),
            Presence::NotFound => (tr("No detectado", "Not detected"), FAINT),
            Presence::Busy => (tr("Ocupado por otro programa", "Busy with another program"), WARN),
            Presence::Stale => (tr("Último dato", "Last known"), MUTED),
            Presence::Paused => (tr("En pausa por seguridad", "Paused for safety"), WARN),
            Presence::Error => ("Error", LOW),
        }
    }
}

pub fn ring_color(d: &DeviceStatus) -> Color32 {
    if d.presence == Presence::Off {
        FAINT
    } else if d.topping_off || d.charge == Charge::Charging {
        CHARGE
    } else if matches!(d.presence, Presence::Online) || d.battery.is_some() {
        level_color(d.battery.unwrap_or(0), d.low)
    } else {
        FAINT
    }
}

fn home(ui: &mut egui::Ui, app: &mut App) -> Option<Page> {
    let mut next = None;
    ui.horizontal(|ui| {
        let n = app.snapshot.devices.len();
        let sub = match n {
            0 => tr("Sin dispositivos todavía", "No devices yet").to_string(),
            1 => tr("1 dispositivo", "1 device").to_string(),
            _ => trf!("{n} dispositivos", "{n} devices"),
        };
        header(ui, &app.logo, "YesCharge", &sub);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, Glyph::Settings, tr("Ajustes", "Settings")).clicked() {
                next = Some(Page::Settings(Box::new(settings::Settings::new())));
            }
            let clicked = widgets::icon_button(ui, Glyph::Refresh, tr("Actualizar ahora", "Refresh now")).clicked();
            // los clics seguidos se ignoran: no aportan nada y cada uno sería un ciclo completo
            if clicked && app.last_refresh.is_none_or(|t| t.elapsed() >= REFRESH_GAP) {
                app.last_refresh = Some(Instant::now());
                app.refreshing = Some((Instant::now(), app.snapshot.updated_at));
                if !ipc::send(ipc::MSG_REFRESH) {
                    ipc::spawn_self("--background");
                }
            }
        });
    });
    ui.add_space(6.0);

    if !app.service_up {
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(tr("El servicio está detenido", "The service is stopped"))
                            .font(theme::semibold(14.5))
                            .color(WARN),
                    );
                    widgets::muted(
                        ui,
                        tr(
                            "Sin él no se actualiza la batería ni llegan avisos.",
                            "Without it the battery doesn't update and no alerts arrive.",
                        ),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary_button(ui, tr("Iniciar", "Start"), 90.0).clicked() {
                        ipc::spawn_self("--background");
                    }
                });
            });
        });
    }

    let footer_h = 96.0;
    let ids: Vec<String> = app
        .drag
        .as_ref()
        .map_or_else(|| app.cfg.devices.iter().map(|d| d.id.clone()).collect(), |drag| drag.order.clone());
    let devices = status::ordered(&app.snapshot.devices, &ids, app.cfg.auto_sort && app.drag.is_none());
    egui::ScrollArea::vertical().max_height(ui.available_height() - footer_h).auto_shrink([false, false]).show(
        ui,
        |ui| {
            if app.refreshing.is_some() {
                wizard::spinner_block(ui, tr("Actualizando…", "Updating…"));
                return;
            }
            if devices.is_empty() {
                empty_state(ui);
            }
            let mut cards = Vec::with_capacity(devices.len());
            for d in &devices {
                let dragged = app.drag.as_ref().is_some_and(|drag| drag.id == d.id);
                let card = device_card(ui, d, app.snapshot.updated_at, dragged);
                cards.push((d.id.clone(), card.rect));
                if card.drag_started() && devices.len() > 1 {
                    let order = devices.iter().map(|x| x.id.clone()).collect();
                    app.drag = Some(Drag { id: d.id.clone(), order });
                }
                if card.clicked()
                    && let Some(entry) = app.cfg.devices.iter().find(|e| e.id == d.id)
                {
                    next = Some(Page::Detail(Box::new(detail::Detail::new(entry))));
                }
            }
            drag_cards(ui, app, &cards);
        },
    );

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        let footer = if app.snapshot.updated_at > 0.0 {
            let now = yescharge_core::now_secs();
            let next_in = (app.snapshot.next_poll_at - now).max(0.0);
            trf!(
                "Actualizado {} · próxima consulta en {} min",
                "Updated {} · next check in {} min",
                ago(now - app.snapshot.updated_at),
                (next_in / 60.0).ceil() as u64
            )
        } else {
            tr("Esperando la primera lectura…", "Waiting for the first reading…").into()
        };
        widgets::muted(ui, footer);
        if widgets::primary_button(ui, tr("+  Añadir dispositivo", "+  Add device"), ui.available_width(), true)
            .clicked()
        {
            next = Some(Page::Add(Box::new(wizard::Wizard::new())));
        }
    });
    next
}

/// Mueve la tarjeta arrastrada sobre la que tiene debajo. Al soltarla, ese orden queda guardado y
/// la lista pasa a orden fijo: el usuario ya eligió dónde va cada una.
fn drag_cards(ui: &egui::Ui, app: &mut App, cards: &[(String, Rect)]) {
    let Some(drag) = &mut app.drag else { return };
    let (pointer, released) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.any_released()));
    if let Some(y) = pointer.map(|p| p.y)
        && let Some((over, _)) = cards.iter().find(|(id, r)| *id != drag.id && y >= r.top() && y <= r.bottom())
        && let (Some(from), Some(to)) =
            (drag.order.iter().position(|id| *id == drag.id), drag.order.iter().position(|id| id == over))
    {
        let id = drag.order.remove(from);
        drag.order.insert(to, id);
    }
    if released {
        let order = std::mem::take(&mut drag.order);
        app.drag = None;
        let mut cfg = Config::load();
        cfg.devices.sort_by_key(|d| order.iter().position(|id| *id == d.id).unwrap_or(usize::MAX));
        cfg.auto_sort = false;
        if save_config(&cfg) {
            app.cfg = cfg;
        }
    }
}

fn empty_state(ui: &mut egui::Ui) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(120.0, 120.0), Sense::hover());
        let p = ui.painter();
        p.circle_stroke(rect.center(), 54.0, Stroke::new(8.0, SURFACE_2));
        p.circle_stroke(rect.center(), 54.0, Stroke::new(1.0, BORDER));
        widgets::bolt(p, rect.center(), 46.0, theme::alpha(BRAND_A, 0.9));
        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(tr("Añade tu primer dispositivo", "Add your first device")).font(theme::semibold(18.0)),
        );
        ui.add_space(2.0);
        widgets::muted(
            ui,
            tr(
                "Elige tu auricular, ratón, teclado o mando y YesCharge",
                "Pick your headset, mouse, keyboard or controller and YesCharge",
            ),
        );
        widgets::muted(
            ui,
            tr("probará cómo leer su batería de forma segura.", "will test how to read its battery safely."),
        );
    });
}

fn device_card(ui: &mut egui::Ui, d: &DeviceStatus, now: f64, dragged: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 100.0), Sense::hover());
    // el id no depende del sitio en la lista: si no, se perdería el arrastre al cambiar de lugar
    let resp = ui.interact(rect, egui::Id::unique(("device_card", &d.id)), Sense::click_and_drag());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    let border = if dragged {
        BRAND_A
    } else if d.low {
        theme::alpha(LOW, 0.55)
    } else {
        theme::lerp(BORDER, theme::alpha(BRAND_A, 0.5), hover)
    };
    p.rect(
        rect,
        CornerRadius::same(18),
        theme::lerp(SURFACE, SURFACE_HI, hover),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );

    // anillo
    let ring_rect = Rect::from_center_size(pos2(rect.left() + 52.0, rect.center().y), vec2(66.0, 66.0));
    let target = d.battery.map(|b| b as f32 / 100.0);
    let shown = target.map(|t| ui.ctx().animate_value_with_time(resp.id.with("ring"), t, 0.7));
    let label = match d.battery {
        Some(b) => format!("{b}%"),
        None => "—".into(),
    };
    let color = ring_color(d);
    widgets::ring(ui, ring_rect, shown, color, &label, None);
    if d.charge == Charge::Charging || d.topping_off {
        let c = pos2(ring_rect.right() - 6.0, ring_rect.bottom() - 6.0);
        p.circle_filled(c, 10.0, SURFACE);
        widgets::bolt(p, c, 13.0, CHARGE);
    }

    // textos
    let x = rect.left() + 100.0;
    widgets::device_glyph(p, Rect::from_min_size(pos2(x, rect.top() + 17.0), vec2(18.0, 18.0)), d.kind, MUTED);
    let name_w = rect.width() - 100.0 - 130.0;
    let name = ui.painter().layout(d.name.clone(), theme::semibold(16.0), TEXT, name_w.max(60.0));
    p.galley(pos2(x + 26.0, rect.top() + 15.0), name, TEXT);
    let (text, col) = status_of(d);
    let galley = p.layout_no_wrap(text.to_string(), theme::semibold(12.0), col);
    let chip = Rect::from_min_size(pos2(x, rect.top() + 44.0), galley.size() + vec2(16.0, 6.0));
    p.rect_filled(chip, CornerRadius::same(255), theme::alpha(col, 0.14));
    p.galley(chip.center() - galley.size() / 2.0, galley, col);
    if d.approximate {
        p.text(
            pos2(chip.right() + 8.0, chip.center().y),
            Align2::LEFT_CENTER,
            tr("aprox.", "approx."),
            egui::FontId::proportional(11.5),
            FAINT,
        );
    }
    let seen = (d.last_seen > 0.0 && d.presence != Presence::Online)
        .then(|| trf!("visto {}", "seen {}", ago(now - d.last_seen)));
    let line =
        [Some(d.eta.clone()).filter(|e| !e.is_empty()), seen].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    p.text(pos2(x, rect.top() + 80.0), Align2::LEFT_CENTER, line, egui::FontId::proportional(12.5), MUTED);

    // gráfica 24 h
    let spark = Rect::from_min_max(
        pos2(rect.right() - 116.0, rect.top() + 30.0),
        pos2(rect.right() - 18.0, rect.bottom() - 30.0),
    );
    if d.history.len() >= 2 {
        widgets::sparkline(ui, spark, &d.history, now, 24.0, color);
        p.text(
            pos2(spark.right(), spark.bottom() + 12.0),
            Align2::RIGHT_CENTER,
            "24 h",
            egui::FontId::proportional(10.5),
            FAINT,
        );
    }
    ui.add_space(2.0);
    resp.on_hover_cursor(if dragged { egui::CursorIcon::Grabbing } else { egui::CursorIcon::PointingHand })
}

/// Guarda la configuración y avisa al servicio (o lo arranca si no está).
pub fn save_config(cfg: &Config) -> bool {
    match cfg.save() {
        Ok(()) => {
            yescharge_core::i18n::set(cfg.language);
            if !ipc::send(ipc::MSG_RELOAD) {
                ipc::spawn_self("--background");
            }
            true
        }
        Err(e) => {
            yescharge_core::log::error(&format!("no se pudo guardar config.json: {e}"));
            false
        }
    }
}

/// Fila "atrás + título" de las páginas interiores. Devuelve `true` si se pulsó atrás.
pub fn page_header(ui: &mut egui::Ui, title: &str) -> bool {
    let mut back = false;
    ui.horizontal(|ui| {
        back = widgets::icon_button(ui, Glyph::Back, tr("Volver", "Back")).clicked();
        ui.label(egui::RichText::new(title).font(theme::bold(20.0)));
    });
    ui.add_space(4.0);
    back
}
