use crate::actions::{
    self, apply_charge_limit, set_charge_care, set_owl_mode, set_perf_mode_checked,
    set_travel_mode, toggle_touchpad, toggle_touchscreen, SharedHandle,
};
use crate::config::{
    self, charge_level_hint, generate_api_token, is_portable_mode, sha256_hex, KeyAction,
    Language, TrayMetricKind,
};
use crate::daemon::{self, notify_daemon_reload};
use crate::hw::mifs::{PerfMode, CHARGE_PRESETS};
use crate::hw::sys;
use crate::hw::touchpad::{self, TouchpadVibration};
use crate::hw::touchpad_dz;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};
use std::fs;
use std::time::{Duration, Instant};

// ============================================================================
// DESIGN TOKENS
// ============================================================================
const BG_CANVAS: Color32 = Color32::from_rgb(0x09, 0x0D, 0x15);
const BG_SIDEBAR: Color32 = Color32::from_rgb(0x0D, 0x13, 0x1F);
const BG_CARD: Color32 = Color32::from_rgb(0x11, 0x19, 0x28);
const BG_ELEVATED: Color32 = Color32::from_rgb(0x18, 0x22, 0x35);
const BG_HOVER: Color32 = Color32::from_rgb(0x21, 0x2E, 0x45);
const BORDER: Color32 = Color32::from_rgb(0x22, 0x30, 0x47);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xF1, 0xF5, 0xF9);
const TEXT_MUTED: Color32 = Color32::from_rgb(0x8B, 0x9B, 0xB4);
const ACCENT: Color32 = Color32::from_rgb(0x38, 0xBD, 0xF8);
const GREEN: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);
const AMBER: Color32 = Color32::from_rgb(0xFB, 0xBF, 0x24);
const CORAL: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);

fn mode_color(mode: PerfMode) -> Color32 {
    match mode {
        PerfMode::Eco => GREEN,
        PerfMode::Quiet => Color32::from_rgb(0x60, 0xA5, 0xFA),
        PerfMode::Balance => Color32::from_rgb(0x2D, 0xD4, 0xBF),
        PerfMode::Auto => Color32::from_rgb(0xA7, 0x8B, 0xFA),
        PerfMode::Turbo => Color32::from_rgb(0xFB, 0x92, 0x3C),
        PerfMode::FullSpeed => CORAL,
    }
}

fn mode_subtitle(mode: PerfMode, en: bool) -> &'static str {
    match (mode, en) {
        (PerfMode::Eco, false) => "Макс. автономность",
        (PerfMode::Eco, true) => "Max battery life",
        (PerfMode::Quiet, false) => "Минимум шума",
        (PerfMode::Quiet, true) => "Whisper quiet",
        (PerfMode::Balance, false) => "Сбалансированный",
        (PerfMode::Balance, true) => "Everyday balance",
        (PerfMode::Auto, false) => "Адаптация ИИ / EC",
        (PerfMode::Auto, true) => "Smart EC scaling",
        (PerfMode::Turbo, false) => "Высокий лимит TDP",
        (PerfMode::Turbo, true) => "High TDP boost",
        (PerfMode::FullSpeed, false) => "Максимум кулеров",
        (PerfMode::FullSpeed, true) => "Max fans & power",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dashboard,
    Performance,
    Battery,
    Display,
    Touchpad,
    Keys,
    Monitor,
    System,
}

impl Tab {
    const ALL: [Tab; 8] = [
        Tab::Dashboard,
        Tab::Performance,
        Tab::Battery,
        Tab::Display,
        Tab::Touchpad,
        Tab::Keys,
        Tab::Monitor,
        Tab::System,
    ];

    fn label(&self, en: bool) -> &'static str {
        match (self, en) {
            (Tab::Dashboard, false) => "Обзор",
            (Tab::Dashboard, true) => "Overview",
            (Tab::Performance, false) => "Питание",
            (Tab::Performance, true) => "Power Modes",
            (Tab::Battery, false) => "Батарея",
            (Tab::Battery, true) => "Battery",
            (Tab::Display, false) => "Экран",
            (Tab::Display, true) => "Display",
            (Tab::Touchpad, false) => "Тачпад",
            (Tab::Touchpad, true) => "Touchpad",
            (Tab::Keys, false) => "Клавиши",
            (Tab::Keys, true) => "Keys & OSD",
            (Tab::Monitor, false) => "Монитор",
            (Tab::Monitor, true) => "Monitor",
            (Tab::System, false) => "Система и API",
            (Tab::System, true) => "System & API",
        }
    }

    fn subtitle(&self, en: bool) -> &'static str {
        match (self, en) {
            (Tab::Dashboard, false) => "Быстрое управление профилем, зарядом, экраном и вводом",
            (Tab::Dashboard, true) => "Quick overview of power, battery, display and input",
            (Tab::Performance, false) => "Режимы MIFS EC, авто-профили и фильтрация цикла",
            (Tab::Performance, true) => "EC performance profiles, AC/BAT rules and cycle order",
            (Tab::Battery, false) => "Пороги заряда, ChargeGuard, режим «В дорогу» и «Сова»",
            (Tab::Battery, true) => "Charge limits, ChargeGuard, Travel and Owl mode",
            (Tab::Display, false) => "Частота 60/120 Гц, масштаб и защита OLED от выгорания",
            (Tab::Display, true) => "Refresh rate, scaling, brightness memory and OLED guard",
            (Tab::Touchpad, false) => "Вибромотор BLTP7853, боковые слайдеры и KWin-ввод",
            (Tab::Touchpad, true) => "BLTP7853 haptics, edge sliders and KWin toggles",
            (Tab::Keys, false) => "Назначение слотов Heavy Press, Fn/WMI и экранных уведомлений",
            (Tab::Keys, true) => "Custom slot actions, KDE shortcut triggers and OSD",
            (Tab::Monitor, false) => "Живая телеметрия датчиков, энергопотребление и графики 60с",
            (Tab::Monitor, true) => "Real-time CPU, GPU, NPU, RAM and power telemetry charts",
            (Tab::System, false) => "Трей KDE, автозапуск systemd, HTTP API и железо",
            (Tab::System, true) => "KDE tray badges, systemd daemon, HTTP API and DMI info",
        }
    }
}

pub struct XiGuiApp {
    shared: SharedHandle,
    tab: Tab,
    slide_draft: u16,
    status_msg: String,
    is_error: bool,
    status_set_at: Option<Instant>,
    revealed_token: Option<String>,
    daemon_online: bool,
    live_tp_pressure: u16,
    peak_tp_pressure: u16,
    wmi_input_error: Option<String>,
    tp_evdev_error: Option<String>,
    last_fast: Instant,
    last_slow: Instant,
    last_diag: Instant,
}

impl XiGuiApp {
    fn new(cc: &eframe::CreationContext<'_>, shared: SharedHandle) -> Self {
        setup_fonts(&cc.egui_ctx);
        setup_theme(&cc.egui_ctx);
        actions::refresh_fast(&shared);
        actions::refresh_slow(&shared);
        let sl = {
            let s = shared.lock().unwrap();
            s.touchpad_state.as_ref().and_then(|t| t.slide_strength).unwrap_or(80)
        };
        let daemon_online = daemon::send_ipc_command("PING").is_ok();
        let mut app = Self {
            shared,
            tab: Tab::Dashboard,
            slide_draft: sl,
            status_msg: String::new(),
            is_error: false,
            status_set_at: None,
            revealed_token: None,
            daemon_online,
            live_tp_pressure: 0,
            peak_tp_pressure: 0,
            wmi_input_error: None,
            tp_evdev_error: None,
            last_fast: Instant::now(),
            last_slow: Instant::now(),
            last_diag: Instant::now() - Duration::from_secs(10),
        };
        app.poll_diag();
        app
    }

    fn poll_diag(&mut self) {
        if self.daemon_online {
            if let Ok(resp) = daemon::send_ipc_command("DIAG") {
                let mut parts = resp.splitn(4, '|');
                self.live_tp_pressure = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                self.peak_tp_pressure = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                let err = parts.next().unwrap_or("").trim();
                self.wmi_input_error = if err.is_empty() { None } else { Some(err.to_string()) };
                let tp_err = parts.next().unwrap_or("").trim();
                self.tp_evdev_error = if tp_err.is_empty() { None } else { Some(tp_err.to_string()) };
                return;
            }
        }
        let s = self.shared.lock().unwrap();
        self.live_tp_pressure = s.live_tp_pressure;
        self.peak_tp_pressure = s.peak_tp_pressure;
        self.wmi_input_error = s
            .wmi_input_error
            .clone()
            .or_else(daemon::probe_wmi_input_error);
        self.tp_evdev_error = if s.config.touchpad_dead_zone_enabled {
            s.tp_evdev_error
                .clone()
                .or_else(touchpad_dz::probe_tp_evdev_error)
        } else {
            None
        };
    }

    fn set_status(&mut self, res: Result<(), String>, ok_msg: String) {
        self.status_set_at = Some(Instant::now());
        match res {
            Ok(()) => {
                self.status_msg = ok_msg;
                self.is_error = false;
            }
            Err(e) => {
                self.status_msg = e;
                self.is_error = true;
            }
        }
    }

    fn save_and_notify(&self) {
        let s = self.shared.lock().unwrap();
        let _ = s.config.save();
        drop(s);
        notify_daemon_reload();
    }
}

pub fn run_gui_window(shared: SharedHandle) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 650.0])
            .with_min_inner_size([760.0, 520.0])
            .with_title("XiControl Linux — Xiaomi Book Pro 14 (TM2424)"),
        ..Default::default()
    };
    eframe::run_native(
        "XiControl",
        options,
        Box::new(|cc| Ok(Box::new(XiGuiApp::new(cc, shared)))),
    )
    .map_err(|e| format!("GUI Error: {e}"))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for path in [
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ] {
        if let Ok(data) = fs::read(path) {
            fonts
                .font_data
                .insert("sans".into(), std::sync::Arc::new(egui::FontData::from_owned(data)));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "sans".into());
            break;
        }
    }
    for path in [
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    ] {
        if let Ok(data) = fs::read(path) {
            fonts
                .font_data
                .insert("mono".into(), std::sync::Arc::new(egui::FontData::from_owned(data)));
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "mono".into());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn setup_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(11.0, 6.0);
    style.spacing.slider_width = 170.0;
    style.spacing.interact_size.y = 26.0;

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_CANVAS;
    v.window_fill = BG_CARD;
    v.extreme_bg_color = BG_SIDEBAR;
    v.faint_bg_color = BG_ELEVATED;
    v.override_text_color = Some(TEXT_PRIMARY);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let round = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = BG_CARD;
    v.widgets.noninteractive.weak_bg_fill = BG_ELEVATED;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.corner_radius = round;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);

    v.widgets.inactive.bg_fill = BG_ELEVATED;
    v.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.corner_radius = round;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);

    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.hovered.corner_radius = round;
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);

    v.widgets.active.bg_fill = Color32::from_rgb(0x28, 0x39, 0x55);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x28, 0x39, 0x55);
    v.widgets.active.bg_stroke = Stroke::new(1.5, ACCENT);
    v.widgets.active.corner_radius = round;
    v.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);

    style.visuals = v;
    ctx.set_style_of(egui::Theme::Dark, style);
}

// ============================================================================
// REUSABLE UI PRIMITIVES
// ============================================================================
fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, add_contents);
}

fn card_title(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.label(RichText::new(title).size(14.5).color(TEXT_PRIMARY).strong());
    if let Some(sub) = subtitle {
        ui.label(RichText::new(sub).size(12.0).color(TEXT_MUTED));
    }
    ui.add_space(6.0);
}

fn tinted_fill(c: Color32, bg: Color32, alpha: f32) -> Color32 {
    let blend = |fg: u8, b: u8| ((fg as f32 * alpha) + (b as f32 * (1.0 - alpha))).round() as u8;
    Color32::from_rgb(blend(c.r(), bg.r()), blend(c.g(), bg.g()), blend(c.b(), bg.b()))
}

fn chip_button_colored(ui: &mut egui::Ui, label: &str, active: bool, color: Color32) -> bool {
    let fill = if active { tinted_fill(color, BG_CARD, 0.22) } else { BG_ELEVATED };
    let stroke = if active { Stroke::new(1.5, color) } else { Stroke::new(1.0, BORDER) };
    let text_col = if active { Color32::WHITE } else { TEXT_MUTED };
    let btn = egui::Button::new(RichText::new(label).color(text_col).strong())
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(76.0, 30.0));
    ui.add(btn).clicked()
}

fn chip_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    chip_button_colored(ui, label, active, ACCENT)
}

fn mode_tile(
    ui: &mut egui::Ui,
    mode: PerfMode,
    active: bool,
    enabled: bool,
    badge: Option<&str>,
    tooltip: Option<&str>,
    en: bool,
) -> bool {
    let col = mode_color(mode);
    let fill = if active {
        tinted_fill(col, BG_CARD, 0.22)
    } else if !enabled {
        Color32::from_rgb(0x13, 0x1A, 0x28)
    } else {
        BG_ELEVATED
    };
    let stroke = if active {
        Stroke::new(1.5, col)
    } else if !enabled {
        Stroke::new(1.0, Color32::from_rgb(0x1B, 0x25, 0x38))
    } else {
        Stroke::new(1.0, BORDER)
    };
    let desired = Vec2::new(112.0, 48.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(desired, sense);
    let draw_fill = if enabled && resp.hovered() && !active { BG_HOVER } else { fill };
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 10.0, draw_fill);
    p.rect_stroke(rect, 10.0, stroke, StrokeKind::Inside);

    if active {
        let bar = Rect::from_min_size(rect.min + Vec2::new(10.0, 0.0), Vec2::new(rect.width() - 20.0, 2.5));
        p.rect_filled(bar, 2.0, col);
    }

    let title_col = if active {
        Color32::WHITE
    } else if !enabled {
        Color32::from_rgb(0x64, 0x74, 0x8B)
    } else {
        TEXT_PRIMARY
    };
    let sub_col = if active {
        col
    } else if !enabled {
        Color32::from_rgb(0x4E, 0x5D, 0x73)
    } else {
        TEXT_MUTED
    };

    p.text(
        rect.left_top() + Vec2::new(10.0, 9.0),
        egui::Align2::LEFT_TOP,
        mode.label(en),
        FontId::proportional(13.0),
        title_col,
    );
    if let Some(tag) = badge {
        let pill = Rect::from_min_size(rect.right_top() + Vec2::new(-32.0, 7.0), Vec2::new(24.0, 14.0));
        p.rect_filled(pill, 4.0, Color32::from_rgb(0x1E, 0x29, 0x3B));
        p.text(pill.center(), egui::Align2::CENTER_CENTER, tag, FontId::proportional(9.5), TEXT_MUTED);
    }
    p.text(
        rect.left_bottom() + Vec2::new(10.0, -8.0),
        egui::Align2::LEFT_BOTTOM,
        mode_subtitle(mode, en),
        FontId::proportional(10.5),
        sub_col,
    );
    if let Some(tip) = tooltip {
        resp = resp.on_hover_text(tip);
    }
    enabled && resp.clicked()
}

fn render_perf_mode_tiles(
    ui: &mut egui::Ui,
    shared: &SharedHandle,
    perf: Option<PerfMode>,
    ac_online: bool,
    unavail: &[String],
    en: bool,
) -> Option<(Result<(), String>, String)> {
    let mut clicked_res = None;
    ui.horizontal_wrapped(|ui| {
        for m in PerfMode::ALL {
            let is_unavail = config::contains_perf_mode(unavail, m);
            let ac_only = !ac_online && !m.is_battery_capable();
            let enabled = !is_unavail && !ac_only;
            let badge = if is_unavail {
                Some("N/A")
            } else if ac_only {
                Some("AC")
            } else {
                None
            };
            let tip = if is_unavail {
                Some(if en { "Not supported by this laptop's EC firmware" } else { "Не поддерживается прошивкой EC этой модели" })
            } else if ac_only {
                Some(if en { "Requires AC power (on battery EC allows Eco and Auto only)" } else { "Доступно только от сети (от батареи EC разрешает только Eco и Авто)" })
            } else {
                None
            };
            if mode_tile(ui, m, perf == Some(m), enabled, badge, tip, en) {
                let r = set_perf_mode_checked(shared, m);
                let ok_msg = if en {
                    format!("Mode: {}", m.label(true))
                } else {
                    format!("Режим: {}", m.label(false))
                };
                clicked_res = Some((r, ok_msg));
            }
        }
    });
    if !ac_online {
        ui.add_space(6.0);
        ui.label(
            RichText::new(if en {
                "On battery: EC firmware locks modes to Eco and Auto. Plug in charger for Quiet / Turbo / FullSpeed."
            } else {
                "От батареи прошивка EC разрешает только Eco и Авто. Для Тихого / Турбо / Полной мощности подключите зарядку."
            })
            .size(11.5)
            .color(TEXT_MUTED),
        );
    }
    clicked_res
}

fn toggle_switch(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let desired = Vec2::new(38.0, 20.0);
        let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
        if resp.clicked() {
            *value = !*value;
            changed = true;
        }
        let anim = ui.ctx().animate_bool(resp.id, *value);
        let track_col = if *value {
            tinted_fill(ACCENT, BG_ELEVATED, 0.85)
        } else {
            BG_ELEVATED
        };
        let p = ui.painter();
        p.rect_filled(rect, 10.0, track_col);
        p.rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, if *value { ACCENT } else { BORDER }),
            StrokeKind::Inside,
        );
        let knob_x = rect.left() + 10.0 + anim * (rect.width() - 20.0);
        let knob_center = Pos2::new(knob_x, rect.center().y);
        p.circle_filled(knob_center, 7.0, if *value { Color32::WHITE } else { TEXT_MUTED });

        let text_resp = ui.add(
            egui::Label::new(RichText::new(label).color(TEXT_PRIMARY)).sense(Sense::click()),
        );
        if text_resp.clicked() {
            *value = !*value;
            changed = true;
        }
    });
    changed
}

fn progress_bar(ui: &mut egui::Ui, frac: f32, color: Color32, height: f32) {
    let desired = Vec2::new(ui.available_width().max(40.0), height);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, height * 0.5, BG_ELEVATED);
    let fill_w = (rect.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.01 { height } else { 0.0 });
    if fill_w > 0.0 {
        let fill_rect = Rect::from_min_size(rect.min, Vec2::new(fill_w, height));
        p.rect_filled(fill_rect, height * 0.5, color);
    }
}

fn bento_stat_card(
    ui: &mut egui::Ui,
    title: &str,
    big_val: &str,
    sub: &str,
    frac_opt: Option<f32>,
    accent: Color32,
) {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_height(84.0);
            ui.horizontal(|ui| {
                let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 4.0, accent);
                ui.label(RichText::new(title).size(11.5).color(TEXT_MUTED).strong());
            });
            ui.add_space(4.0);
            ui.label(RichText::new(big_val).size(22.0).color(Color32::WHITE).strong());
            if let Some(frac) = frac_opt {
                ui.add_space(4.0);
                progress_bar(ui, frac, accent, 5.0);
                ui.add_space(3.0);
            }
            ui.label(RichText::new(sub).size(11.0).color(TEXT_MUTED));
        });
}

fn draw_tab_icon(p: &egui::Painter, c: Pos2, tab: Tab, col: Color32) {
    let s = Stroke::new(1.4, col);
    match tab {
        Tab::Dashboard => {
            for (dx, dy) in [(-3.0, -3.0), (3.0, -3.0), (-3.0, 3.0), (3.0, 3.0)] {
                p.rect_stroke(Rect::from_center_size(c + Vec2::new(dx, dy), Vec2::splat(4.0)), 1.0, s, StrokeKind::Inside);
            }
        }
        Tab::Performance => {
            p.add(egui::Shape::line(
                vec![c + Vec2::new(1.5, -5.5), c + Vec2::new(-2.5, 0.2), c + Vec2::new(1.5, 0.2), c + Vec2::new(-1.5, 5.5)],
                s,
            ));
        }
        Tab::Battery => {
            p.rect_stroke(Rect::from_center_size(c + Vec2::new(-0.5, 0.0), Vec2::new(9.5, 6.5)), 1.5, s, StrokeKind::Inside);
            p.line_segment([c + Vec2::new(5.5, -1.8), c + Vec2::new(5.5, 1.8)], s);
            p.rect_filled(Rect::from_center_size(c + Vec2::new(-2.0, 0.0), Vec2::new(4.0, 3.2)), 0.5, col);
        }
        Tab::Display => {
            p.rect_stroke(Rect::from_center_size(c + Vec2::new(0.0, -1.0), Vec2::new(11.0, 7.5)), 1.5, s, StrokeKind::Inside);
            p.line_segment([c + Vec2::new(-3.0, 4.5), c + Vec2::new(3.0, 4.5)], s);
        }
        Tab::Touchpad => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::new(11.0, 8.0)), 1.5, s, StrokeKind::Inside);
            p.line_segment([c + Vec2::new(-5.0, 1.5), c + Vec2::new(5.0, 1.5)], Stroke::new(1.0, col));
        }
        Tab::Keys => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::splat(10.0)), 2.0, s, StrokeKind::Inside);
            p.circle_filled(c, 1.8, col);
        }
        Tab::Monitor => {
            p.add(egui::Shape::line(
                vec![c + Vec2::new(-5.5, 1.0), c + Vec2::new(-2.0, 1.0), c + Vec2::new(0.0, -4.0), c + Vec2::new(2.0, 3.5), c + Vec2::new(4.0, 0.5), c + Vec2::new(5.5, 0.5)],
                s,
            ));
        }
        Tab::System => {
            p.circle_stroke(c, 4.2, s);
            p.circle_filled(c, 1.5, col);
        }
    }
}

fn draw_sun_icon(p: &egui::Painter, c: Pos2, col: Color32) {
    p.circle_stroke(c, 3.2, Stroke::new(1.4, col));
    for i in 0..8 {
        let a = (i as f32) * std::f32::consts::FRAC_PI_4;
        let d = Vec2::new(a.cos(), a.sin());
        p.line_segment([c + d * 4.8, c + d * 6.5], Stroke::new(1.3, col));
    }
}

fn draw_speaker_icon(p: &egui::Painter, c: Pos2, col: Color32) {
    let s = Stroke::new(1.4, col);
    p.rect_filled(Rect::from_center_size(c + Vec2::new(-3.2, 0.0), Vec2::new(2.8, 4.0)), 0.5, col);
    p.add(egui::Shape::line(
        vec![c + Vec2::new(-1.8, -2.0), c + Vec2::new(1.5, -4.5), c + Vec2::new(1.5, 4.5), c + Vec2::new(-1.8, 2.0)],
        s,
    ));
    p.line_segment([c + Vec2::new(3.8, -2.2), c + Vec2::new(4.4, 0.0)], s);
    p.line_segment([c + Vec2::new(4.4, 0.0), c + Vec2::new(3.8, 2.2)], s);
}

fn touchpad_diagram(ui: &mut egui::Ui, width_mm: u8, dz_mm: u8, edge_on: bool, en: bool) {
    let desired = Vec2::new(220.0, 142.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let p = ui.painter_at(rect);

    p.rect_filled(rect, 10.0, BG_ELEVATED);
    p.rect_stroke(rect, 10.0, Stroke::new(1.2, BORDER), StrokeKind::Inside);

    // 139 x 90 mm proportions
    let left_w = (width_mm as f32 / 139.0) * rect.width();
    let right_w = left_w;
    let dz_h = (dz_mm as f32 / 90.0) * rect.height();

    if edge_on {
        let left_rect = Rect::from_min_max(rect.min, Pos2::new(rect.left() + left_w, rect.bottom() - dz_h));
        let right_rect = Rect::from_min_max(Pos2::new(rect.right() - right_w, rect.top()), Pos2::new(rect.right(), rect.bottom() - dz_h));
        p.rect_filled(left_rect, CornerRadius { nw: 10, sw: if dz_h < 1.0 { 10 } else { 0 }, ne: 0, se: 0 }, tinted_fill(AMBER, BG_ELEVATED, 0.28));
        p.rect_filled(right_rect, CornerRadius { ne: 10, se: if dz_h < 1.0 { 10 } else { 0 }, nw: 0, sw: 0 }, tinted_fill(ACCENT, BG_ELEVATED, 0.28));

        draw_sun_icon(&p, left_rect.center(), AMBER);
        draw_speaker_icon(&p, right_rect.center(), ACCENT);
    }

    if dz_h > 2.0 {
        let dz_rect = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - dz_h), rect.max);
        p.rect_filled(dz_rect, CornerRadius { sw: 10, se: 10, nw: 0, ne: 0 }, tinted_fill(CORAL, BG_ELEVATED, 0.22));
        p.line_segment([dz_rect.left_top(), dz_rect.right_top()], Stroke::new(1.0, CORAL));
        p.text(
            dz_rect.center(),
            egui::Align2::CENTER_CENTER,
            if en { "Dead zone" } else { "Мёртвая зона" },
            FontId::proportional(10.0),
            CORAL,
        );
    }

    p.text(
        Pos2::new(rect.center().x, rect.center().y - dz_h * 0.25),
        egui::Align2::CENTER_CENTER,
        "139 × 90 mm\nBLTP7853",
        FontId::proportional(11.0),
        TEXT_MUTED,
    );
}

// ============================================================================
// MAIN APP SHELL (SIDEBAR + HEADER + TOAST + CONTENT)
// ============================================================================
impl eframe::App for XiGuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_fast.elapsed() >= Duration::from_secs(2) {
            actions::refresh_fast(&self.shared);
            self.last_fast = Instant::now();
        }
        if self.last_slow.elapsed() >= Duration::from_secs(10) {
            actions::refresh_slow(&self.shared);
            self.daemon_online = daemon::send_ipc_command("PING").is_ok();
            self.last_slow = Instant::now();
        }
        let diag_interval = if self.tab == Tab::Touchpad {
            Duration::from_millis(120)
        } else {
            Duration::from_secs(2)
        };
        if self.last_diag.elapsed() >= diag_interval {
            self.poll_diag();
            self.last_diag = Instant::now();
        }
        if let Some(t) = self.status_set_at {
            if t.elapsed() >= Duration::from_secs(6) {
                self.status_msg.clear();
                self.status_set_at = None;
            }
        }
        ui.ctx().request_repaint_after(if self.tab == Tab::Touchpad {
            Duration::from_millis(120)
        } else {
            Duration::from_secs(1)
        });

        let (en, cur_lang, tele, perf) = {
            let s = self.shared.lock().unwrap();
            (
                s.config.language.is_en(),
                s.config.language,
                s.telemetry.clone(),
                s.perf_mode,
            )
        };

        // Left Sidebar Navigation
        egui::Panel::left("xi_sidebar")
            .resizable(false)
            .exact_size(188.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x18, 0x22, 0x33)))
                    .inner_margin(Margin::symmetric(12, 14)),
            )
            .show(ui, |ui| {
                // Brand block
                ui.horizontal(|ui| {
                    let (badge, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                    let mode_col = perf.map(mode_color).unwrap_or(ACCENT);
                    let p = ui.painter();
                    p.rect_filled(badge, 8.0, tinted_fill(mode_col, BG_SIDEBAR, 0.25));
                    p.rect_stroke(badge, 8.0, Stroke::new(1.3, mode_col), StrokeKind::Inside);
                    p.text(badge.center(), egui::Align2::CENTER_CENTER, "Xi", FontId::proportional(14.0), Color32::WHITE);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("XiControl").size(15.0).color(Color32::WHITE).strong());
                        ui.label(RichText::new("Book Pro 14").size(11.0).color(TEXT_MUTED));
                    });
                });
                ui.add_space(16.0);

                // Nav buttons
                for t in Tab::ALL {
                    let selected = self.tab == t;
                    let desired = Vec2::new(ui.available_width(), 34.0);
                    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
                    let p = ui.painter_at(rect);
                    let fill = if selected {
                        tinted_fill(ACCENT, BG_SIDEBAR, 0.16)
                    } else if resp.hovered() {
                        BG_ELEVATED
                    } else {
                        Color32::TRANSPARENT
                    };
                    p.rect_filled(rect, 8.0, fill);
                    if selected {
                        let indicator = Rect::from_min_size(
                            rect.left_top() + Vec2::new(0.0, 6.0),
                            Vec2::new(3.0, rect.height() - 12.0),
                        );
                        p.rect_filled(indicator, 2.0, ACCENT);
                    }
                    draw_tab_icon(
                        &p,
                        Pos2::new(rect.left() + 18.0, rect.center().y),
                        t,
                        if selected { ACCENT } else { TEXT_MUTED },
                    );
                    p.text(
                        Pos2::new(rect.left() + 34.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        t.label(en),
                        FontId::proportional(13.0),
                        if selected { Color32::WHITE } else { TEXT_MUTED },
                    );
                    if resp.clicked() {
                        self.tab = t;
                    }
                    ui.add_space(2.0);
                }

                // Bottom live status card inside sidebar
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    egui::Frame::new()
                        .fill(BG_CARD)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                let col = if self.daemon_online { GREEN } else { AMBER };
                                ui.painter().circle_filled(dot.center(), 4.0, col);
                                let d_txt = if self.daemon_online {
                                    if en { "Daemon active" } else { "Демон активен" }
                                } else {
                                    if en { "Standalone" } else { "Автономно" }
                                };
                                ui.label(RichText::new(d_txt).size(11.5).color(Color32::WHITE).strong());
                            });
                            ui.add_space(2.0);
                            let pwr = if tele.ac_online { "AC" } else { "BAT" };
                            ui.label(
                                RichText::new(format!(
                                    "{pwr} · {}% · {:.0}°C",
                                    tele.bat_capacity, tele.cpu_temp_c
                                ))
                                .size(11.0)
                                .color(TEXT_MUTED),
                            );
                        });
                });
            });

        // Bottom status toast (only when present)
        if !self.status_msg.is_empty() {
            egui::Panel::bottom("xi_toast")
                .frame(
                    egui::Frame::new()
                        .fill(if self.is_error {
                            tinted_fill(CORAL, BG_CANVAS, 0.14)
                        } else {
                            tinted_fill(GREEN, BG_CANVAS, 0.14)
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if self.is_error { CORAL } else { GREEN },
                        ))
                        .inner_margin(Margin::symmetric(14, 7)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let col = if self.is_error { CORAL } else { GREEN };
                        ui.label(RichText::new(&self.status_msg).color(col).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("x").clicked() {
                                self.status_msg.clear();
                                self.status_set_at = None;
                            }
                        });
                    });
                });
        }

        // Main Canvas
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG_CANVAS)
                    .inner_margin(Margin::symmetric(18, 16)),
            )
            .show(ui, |ui| {
                // Header
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(self.tab.label(en)).size(20.0).color(Color32::WHITE).strong());
                        ui.label(RichText::new(self.tab.subtitle(en)).size(12.0).color(TEXT_MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(if en { "Refresh" } else { "Обновить" })
                                    .min_size(Vec2::new(88.0, 28.0)),
                            )
                            .clicked()
                        {
                            actions::refresh_fast(&self.shared);
                            actions::refresh_slow(&self.shared);
                            self.daemon_online = daemon::send_ipc_command("PING").is_ok();
                            self.poll_diag();
                            {
                                let s = self.shared.lock().unwrap();
                                if let Some(tp) = &s.touchpad_state {
                                    if let Some(sl) = tp.slide_strength {
                                        self.slide_draft = sl;
                                    }
                                }
                            }
                            self.set_status(Ok(()), if en { "Hardware state refreshed" } else { "Состояние устройств обновлено" }.into());
                        }
                        if chip_button(ui, if cur_lang == Language::Ru { "RU" } else { "EN" }, false) {
                            {
                                let mut s = self.shared.lock().unwrap();
                                s.config.language = if cur_lang == Language::Ru { Language::En } else { Language::Ru };
                            }
                            self.save_and_notify();
                        }
                    });
                });
                ui.add_space(10.0);

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.tab {
                        Tab::Dashboard => self.ui_dashboard(ui, en),
                        Tab::Performance => self.ui_performance(ui, en),
                        Tab::Battery => self.ui_battery(ui, en),
                        Tab::Display => self.ui_display(ui, en),
                        Tab::Touchpad => self.ui_touchpad(ui, en),
                        Tab::Keys => self.ui_keys(ui, en),
                        Tab::Monitor => self.ui_monitor(ui, en),
                        Tab::System => self.ui_system(ui, en),
                    }
                    ui.add_space(6.0);
                });
            });
    }
}

// ============================================================================
// TABS
// ============================================================================
impl XiGuiApp {
    fn ui_dashboard(&mut self, ui: &mut egui::Ui, en: bool) {
        let (tele, perf, unavail, charge_lim, pd_w, soh, disp, bright, owl, tp_en, ts_en, tp_state) = {
            let s = self.shared.lock().unwrap();
            (
                s.telemetry.clone(),
                s.perf_mode,
                s.config.unavailable_modes.clone(),
                s.charge_limit,
                s.adapter_watts,
                s.battery_health,
                s.display.clone(),
                s.brightness_pct,
                s.owl_active,
                s.touchpad_enabled,
                s.touchscreen_enabled,
                s.touchpad_state.clone(),
            )
        };

        // 4 Bento Stat Tiles
        ui.columns(4, |cols| {
            let bat_col = if tele.ac_online { GREEN } else if tele.bat_capacity <= 20 { CORAL } else { ACCENT };
            let bat_sub = format!(
                "{:+.1} W · {}",
                tele.bat_watts,
                pd_w.filter(|&w| w > 0)
                    .map(|w| format!("{w}W PD"))
                    .unwrap_or_else(|| soh.map(|v| format!("SOH {v}%")).unwrap_or_else(|| tele.bat_status.clone()))
            );
            bento_stat_card(
                &mut cols[0],
                if en { "BATTERY" } else { "БАТАРЕЯ" },
                &format!("{}%", tele.bat_capacity),
                &bat_sub,
                Some(tele.bat_capacity as f32 / 100.0),
                bat_col,
            );

            let cpu_col = if tele.cpu_temp_c >= 82.0 { CORAL } else if tele.cpu_temp_c >= 68.0 { AMBER } else { ACCENT };
            let cpu_sub = format!(
                "{:.0}% {}{}",
                tele.cpu_pct,
                if en { "load" } else { "загр." },
                tele.cpu_watts.map(|w| format!(" · {w:.1} W")).unwrap_or_default()
            );
            bento_stat_card(
                &mut cols[1],
                if en { "PROCESSOR" } else { "ПРОЦЕССОР" },
                &format!("{:.0}°C", tele.cpu_temp_c),
                &cpu_sub,
                Some(tele.cpu_pct / 100.0),
                cpu_col,
            );

            let gpu_s = tele.gpu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into());
            let npu_s = tele.npu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into());
            bento_stat_card(
                &mut cols[2],
                "GPU / NPU / RAM",
                &format!("{:.0}% RAM", tele.ram_pct),
                &format!("GPU {gpu_s} · NPU {npu_s} · {:.1} GB", tele.ram_used_gb),
                Some(tele.ram_pct / 100.0),
                Color32::from_rgb(0xA7, 0x8B, 0xFA),
            );

            let hz_big = disp.as_ref().map(|d| format!("{} Hz", d.current_hz)).unwrap_or_else(|| "—".into());
            let res_s = disp.as_ref().map(|d| format!("{}×{}", d.current_width, d.current_height)).unwrap_or_else(|| "eDP-1".into());
            let br_s = bright.map(|b| if en { format!(" · {b}% bright") } else { format!(" · {b}% ярк.") }).unwrap_or_default();
            bento_stat_card(
                &mut cols[3],
                if en { "DISPLAY" } else { "ЭКРАН" },
                &hz_big,
                &format!("{res_s}{br_s}"),
                bright.map(|b| b as f32 / 100.0),
                Color32::from_rgb(0x2D, 0xD4, 0xBF),
            );
        });
        ui.add_space(10.0);

        // Performance Mode selector (6 rich tiles)
        card(ui, |ui| {
            card_title(
                ui,
                if en { "Performance Profile (MIFS EC)" } else { "Режим производительности (MIFS EC)" },
                Some(if en { "Controls TDP limits and fan curve directly in Embedded Controller" } else { "Прямое управление TDP-лимитами и кривой вентиляторов в EC" }),
            );
            if let Some((r, ok_msg)) = render_perf_mode_tiles(ui, &self.shared, perf, tele.ac_online, &unavail, en) {
                self.set_status(r, ok_msg);
            }
        });
        ui.add_space(10.0);

        // 2-column quick controls row: Charge/Owl + Display/Input
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| {
                card_title(
                    ui,
                    if en { "Charge Limit & Keep Awake" } else { "Порог заряда и сон" },
                    None,
                );
                let lim_val = charge_lim.unwrap_or(100);
                ui.horizontal_wrapped(|ui| {
                    for &pct in &CHARGE_PRESETS {
                        let lbl = format!("{pct}%");
                        if chip_button_colored(ui, &lbl, charge_lim == Some(pct), GREEN) {
                            let r = apply_charge_limit(&self.shared, pct, None);
                            notify_daemon_reload();
                            let ok = if en { format!("Charge limit: {pct}%") } else { format!("Лимит заряда: {pct}%") };
                            self.set_status(r, ok);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let lim_lbl = if en { format!("Limit: {lim_val}%") } else { format!("Лимит: {lim_val}%") };
                    ui.label(RichText::new(lim_lbl).size(11.5).color(TEXT_MUTED));
                    progress_bar(ui, lim_val as f32 / 100.0, if lim_val < 100 { GREEN } else { AMBER }, 5.0);
                });
                ui.add_space(8.0);
                let mut owl_v = owl;
                if toggle_switch(
                    ui,
                    &mut owl_v,
                    if en { "Owl Mode (keep awake / inhibit sleep)" } else { "Режим «Сова» (не давать ноутбуку уснуть)" },
                ) {
                    let r = set_owl_mode(&self.shared, owl_v);
                    notify_daemon_reload();
                    let ok = match (owl_v, en) {
                        (true, true) => "Owl Mode: ON",
                        (false, true) => "Owl Mode: OFF",
                        (true, false) => "Режим «Сова»: вкл",
                        (false, false) => "Режим «Сова»: выкл",
                    };
                    self.set_status(r, ok.into());
                }
            });

            card(&mut cols[1], |ui| {
                card_title(
                    ui,
                    if en { "Display & Input Devices" } else { "Экран и устройства ввода" },
                    None,
                );
                ui.horizontal_wrapped(|ui| {
                    if let Some(d) = &disp {
                        for &hz in &d.available_hz {
                            if chip_button(ui, &format!("{hz} Hz"), d.current_hz == hz) {
                                let r = actions::set_display_hz_checked(&self.shared, hz);
                                actions::refresh_slow(&self.shared);
                                self.set_status(r, format!("{hz} Hz"));
                            }
                        }
                    }
                    if let Some(tp) = &tp_state {
                        ui.label(RichText::new(format!("BLTP {} / {}", tp.firmware, tp.controller_firmware)).size(11.5).color(TEXT_MUTED));
                    }
                });
                ui.add_space(8.0);
                let mut tp_v = tp_en.unwrap_or(true);
                if toggle_switch(ui, &mut tp_v, if en { "Touchpad enabled (KWin)" } else { "Тачпад включён (KWin)" }) {
                    let r = toggle_touchpad(&self.shared).map(|_| ());
                    notify_daemon_reload();
                    self.set_status(r, if en { "Touchpad updated" } else { "Тачпад обновлён" }.into());
                }
                ui.add_space(4.0);
                let mut ts_v = ts_en.unwrap_or(true);
                if toggle_switch(ui, &mut ts_v, if en { "Touchscreen enabled (KWin)" } else { "Сенсорный экран включён (KWin)" }) {
                    let r = toggle_touchscreen(&self.shared).map(|_| ());
                    notify_daemon_reload();
                    self.set_status(r, if en { "Touchscreen updated" } else { "Сенсорный экран обновлён" }.into());
                }
            });
        });
    }

    fn ui_performance(&mut self, ui: &mut egui::Ui, en: bool) {
        let (perf, ac_online, cfg) = {
            let s = self.shared.lock().unwrap();
            (s.perf_mode, s.telemetry.ac_online, s.config.clone())
        };

        card(ui, |ui| {
            card_title(ui, if en { "Active MIFS Mode" } else { "Активный режим MIFS" }, None);
            if let Some((r, ok_msg)) = render_perf_mode_tiles(
                ui,
                &self.shared,
                perf,
                ac_online,
                &cfg.unavailable_modes,
                en,
            ) {
                self.set_status(r, ok_msg);
            }
            ui.add_space(8.0);
            let mut restore = cfg.restore_mode_on_boot;
            if toggle_switch(ui, &mut restore, if en { "Restore last mode on daemon start" } else { "Восстанавливать режим при запуске демона" }) {
                self.shared.lock().unwrap().config.restore_mode_on_boot = restore;
                self.save_and_notify();
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "AC / Battery Auto Profiles" } else { "Автопереключение от сети / батареи" },
                None,
            );
            let mut en_prof = cfg.power_profiles_enabled;
            let mut changed = toggle_switch(
                ui,
                &mut en_prof,
                if en { "Automatically switch mode when charger is plugged/unplugged" } else { "Автоматически менять режим при подключении или отключении зарядки" },
            );
            ui.add_space(8.0);
            let mut ac_m = cfg.ac_mode.clone();
            let mut bat_m = cfg.bat_mode.clone();
            ui.horizontal_wrapped(|ui| {
                ui.label(if en { "On AC:" } else { "От сети:" });
                egui::ComboBox::from_id_salt("ac_mode_cb")
                    .selected_text(PerfMode::from_name(&ac_m).map(|m| m.label(en)).unwrap_or(&ac_m))
                    .show_ui(ui, |ui| {
                        for m in PerfMode::ALL {
                            if config::contains_perf_mode(&cfg.unavailable_modes, m) {
                                continue;
                            }
                            if ui.selectable_value(&mut ac_m, m.api_name().into(), m.label(en)).changed() {
                                changed = true;
                            }
                        }
                    });
                ui.add_space(20.0);
                ui.label(if en { "On Battery:" } else { "От батареи:" });
                let bat_sel = match PerfMode::from_name(&bat_m) {
                    Some(m) if m.is_battery_capable() => m.label(en).to_string(),
                    Some(m) => if en {
                        format!("Auto ({}: AC only)", m.label(en))
                    } else {
                        format!("Авто ({}: от сети)", m.label(en))
                    },
                    None => bat_m.clone(),
                };
                egui::ComboBox::from_id_salt("bat_mode_cb")
                    .selected_text(bat_sel)
                    .show_ui(ui, |ui| {
                        for m in [PerfMode::Eco, PerfMode::Auto] {
                            if ui.selectable_value(&mut bat_m, m.api_name().into(), m.label(en)).changed() {
                                changed = true;
                            }
                        }
                    });
            });
            if changed {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.power_profiles_enabled = en_prof;
                    s.config.ac_mode = ac_m;
                    s.config.bat_mode = bat_m;
                }
                self.save_and_notify();
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "Modes Included in Cycle" } else { "Режимы при циклическом переключении" },
                Some(if en { "Toggle which modes participate when cycling on AC or Battery." } else { "Отключите режимы, которые не нужно включать при переборе по кнопке." }),
            );
            let mut hac = cfg.hidden_modes_ac.clone();
            let mut hbat = cfg.hidden_modes_bat.clone();
            let mut ch = false;
            egui::Grid::new("cycle_modes_grid").spacing([24.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new(if en { "Mode" } else { "Режим" }).color(TEXT_MUTED).strong());
                ui.label(RichText::new(if en { "AC" } else { "Сеть" }).color(TEXT_MUTED).strong());
                ui.label(RichText::new(if en { "Battery" } else { "АКБ" }).color(TEXT_MUTED).strong());
                ui.end_row();
                for m in PerfMode::ALL {
                    let is_unavail = config::contains_perf_mode(&cfg.unavailable_modes, m);
                    let key = m.api_name().to_string();
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, mode_color(m));
                        let lbl = if is_unavail {
                            format!("{} (N/A)", m.label(en))
                        } else {
                            m.label(en).into()
                        };
                        ui.label(RichText::new(lbl).color(if is_unavail { TEXT_MUTED } else { TEXT_PRIMARY }));
                    });
                    let mut vis_ac = !is_unavail && !config::contains_perf_mode(&hac, m);
                    let ac_resp = ui.add_enabled(!is_unavail, egui::Checkbox::new(&mut vis_ac, ""));
                    if ac_resp.changed() {
                        if vis_ac {
                            config::remove_perf_mode(&mut hac, m);
                        } else if !config::contains_perf_mode(&hac, m) {
                            hac.push(key.clone());
                        }
                        ch = true;
                    }
                    let bat_ok = !is_unavail && m.is_battery_capable();
                    let mut vis_bat = bat_ok && !config::contains_perf_mode(&hbat, m);
                    let bat_resp = ui
                        .add_enabled(bat_ok, egui::Checkbox::new(&mut vis_bat, ""))
                        .on_disabled_hover_text(if en {
                            "Only Eco and Auto are accepted on battery by EC firmware"
                        } else {
                            "От батареи прошивка EC поддерживает только Eco и Авто"
                        });
                    if bat_resp.changed() {
                        if vis_bat {
                            config::remove_perf_mode(&mut hbat, m);
                        } else if !config::contains_perf_mode(&hbat, m) {
                            hbat.push(key);
                        }
                        ch = true;
                    }
                    ui.end_row();
                }
            });
            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.hidden_modes_ac = hac;
                    s.config.hidden_modes_bat = hbat;
                }
                self.save_and_notify();
            }
        });
    }

    fn ui_battery(&mut self, ui: &mut egui::Ui, en: bool) {
        let (tele, cfg, charge_lim, pd_w, soh, owl) = {
            let s = self.shared.lock().unwrap();
            (
                s.telemetry.clone(),
                s.config.clone(),
                s.charge_limit,
                s.adapter_watts,
                s.battery_health,
                s.owl_active,
            )
        };

        ui.columns(3, |cols| {
            bento_stat_card(
                &mut cols[0],
                if en { "STATE OF CHARGE" } else { "ТЕКУЩИЙ ЗАРЯД" },
                &format!("{}%", tele.bat_capacity),
                &format!("{} · {:+.1} W", tele.bat_status, tele.bat_watts),
                Some(tele.bat_capacity as f32 / 100.0),
                if tele.ac_online { GREEN } else { ACCENT },
            );
            let wh_big = match (tele.bat_wh_full, tele.bat_wear_pct) {
                (Some(f), Some(p)) => format!("{f:.1} Wh ({p}%)"),
                _ => "—".into(),
            };
            let design_s = tele.bat_wh_design.map(|d| {
                if en { format!("Design {d:.1} Wh") } else { format!("Проект {d:.1} Wh") }
            }).unwrap_or_default();
            let cyc_s = tele.bat_cycles.map(|c| {
                if en { format!(" · {c} cycles") } else { format!(" · {c} цикл.") }
            }).unwrap_or_default();
            bento_stat_card(
                &mut cols[1],
                if en { "WEAR & CYCLES" } else { "ИЗНОС И ЦИКЛЫ" },
                &wh_big,
                &format!("{design_s}{cyc_s}"),
                tele.bat_wear_pct.map(|p| p as f32 / 100.0),
                Color32::from_rgb(0xA7, 0x8B, 0xFA),
            );
            let pd = pd_w.filter(|&w| w > 0).map(|w| format!("{w} W PD")).unwrap_or_else(|| if en { "On Battery" } else { "От АКБ" }.into());
            let ec_soh = soh.map(|v| format!("EC SOH: {v}%")).unwrap_or_else(|| "EC SOH: —".into());
            bento_stat_card(
                &mut cols[2],
                if en { "ADAPTER & EC HEALTH" } else { "АДАПТЕР И ЗДОРОВЬЕ EC" },
                &pd,
                &ec_soh,
                soh.map(|v| v as f32 / 100.0),
                AMBER,
            );
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "Charge Care, Travel Mode & Guard" } else { "Защита АКБ, режим «В дорогу» и ChargeGuard" },
                None,
            );
            let mut care_on = cfg.charge_care_enabled;
            if toggle_switch(ui, &mut care_on, if en { "Enable Battery Care charge threshold" } else { "Включить ограничение заряда (Беречь батарею)" }) {
                let r = set_charge_care(&self.shared, care_on).map(|_| ());
                notify_daemon_reload();
                self.set_status(r, if en { "Battery Care updated" } else { "Защита АКБ обновлена" }.into());
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for &pct in &CHARGE_PRESETS {
                    let lbl = format!("{pct}%");
                    if chip_button_colored(ui, &lbl, charge_lim == Some(pct), GREEN) {
                        let r = apply_charge_limit(&self.shared, pct, None);
                        notify_daemon_reload();
                        let msg = if en { format!("Charge limit: {pct}%") } else { format!("Лимит заряда: {pct}%") };
                        self.set_status(r, msg);
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new(charge_level_hint(charge_lim.unwrap_or(cfg.charge_care_percent), en)).size(12.0).color(TEXT_MUTED));
            ui.add_space(8.0);

            let mut travel = cfg.travel_mode;
            if toggle_switch(ui, &mut travel, if en { "Travel Mode (charge to 100% temporarily without losing saved limit)" } else { "Режим «В дорогу» (временно до 100%, не теряя сохранённый лимит)" }) {
                let r = set_travel_mode(&self.shared, travel);
                notify_daemon_reload();
                let msg = match (travel, en) {
                    (true, true) => "Travel Mode: ON",
                    (false, true) => "Travel Mode: OFF",
                    (true, false) => "Режим «В дорогу»: вкл",
                    (false, false) => "Режим «В дорогу»: выкл",
                };
                self.set_status(r, msg.into());
            }
            ui.add_space(4.0);
            let mut guard = cfg.charge_guard_enabled;
            if toggle_switch(ui, &mut guard, if en { "ChargeGuard (re-apply threshold every 30s & after sleep)" } else { "ChargeGuard (удерживать порог каждые 30с и после выхода из сна)" }) {
                self.shared.lock().unwrap().config.charge_guard_enabled = guard;
                self.save_and_notify();
            }
            ui.add_space(4.0);
            let mut owl_cb = owl;
            if toggle_switch(ui, &mut owl_cb, if en { "Owl Mode (systemd-inhibit: prevent sleep & idle)" } else { "Режим «Сова» (systemd-inhibit: блокировать засыпание)" }) {
                let r = set_owl_mode(&self.shared, owl_cb);
                notify_daemon_reload();
                let msg = match (owl_cb, en) {
                    (true, true) => "Owl Mode: ON",
                    (false, true) => "Owl Mode: OFF",
                    (true, false) => "Режим «Сова»: вкл",
                    (false, false) => "Режим «Сова»: выкл",
                };
                self.set_status(r, msg.into());
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "Charge Threshold Webhook" } else { "Webhook при изменении порога заряда" },
                Some(if en { "Sends POST JSON via curl when the battery charge limit changes." } else { "Отправляет POST JSON через curl при смене порога заряда." }),
            );
            let mut wh_en = cfg.webhook_enabled;
            let mut wh_url = cfg.webhook_url.clone();
            let mut ch = toggle_switch(ui, &mut wh_en, if en { "Enable webhook" } else { "Включить webhook" });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("URL:");
                if ui.add(egui::TextEdit::singleline(&mut wh_url).desired_width(460.0)).changed() {
                    ch = true;
                }
            });
            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.webhook_enabled = wh_en;
                    s.config.webhook_url = wh_url;
                }
                self.save_and_notify();
            }
        });
    }

    fn ui_display(&mut self, ui: &mut egui::Ui, en: bool) {
        let (disp, bright, tele, cfg) = {
            let s = self.shared.lock().unwrap();
            (s.display.clone(), s.brightness_pct, s.telemetry.clone(), s.config.clone())
        };

        card(ui, |ui| {
            let info_sub = disp.as_ref().map(|d| {
                format!("{} · {}×{} @ {} Hz · scale {:.2}", d.output_name, d.current_width, d.current_height, d.current_hz, d.scale)
            });
            card_title(
                ui,
                if en { "Refresh Rate & Scaling" } else { "Частота обновления и масштаб" },
                info_sub.as_deref(),
            );
            if let Some(d) = &disp {
                ui.horizontal_wrapped(|ui| {
                    for &hz in &d.available_hz {
                        if chip_button(ui, &format!("{hz} Hz"), d.current_hz == hz) {
                            let r = actions::set_display_hz_checked(&self.shared, hz);
                            actions::refresh_slow(&self.shared);
                            self.set_status(r, format!("{hz} Hz"));
                        }
                    }
                    ui.separator();
                    let is_3k = d.current_width == 3120;
                    if chip_button(ui, "3120×2080 (2x)", is_3k) {
                        let r = sys::set_display_preset(true);
                        actions::refresh_slow(&self.shared);
                        self.set_status(r, "3120×2080 (2x)".into());
                    }
                    if chip_button(ui, "1920×1280 (1.25x)", !is_3k) {
                        let r = sys::set_display_preset(false);
                        actions::refresh_slow(&self.shared);
                        self.set_status(r, "1920×1280 (1.25x)".into());
                    }
                });
            } else {
                ui.label(RichText::new(if en { "Display info unavailable via kscreen-doctor" } else { "Не удалось прочитать дисплей через kscreen-doctor" }).color(TEXT_MUTED));
            }
            ui.add_space(10.0);

            let mut auto_hz = cfg.auto_hz_enabled;
            let mut hold_hz = cfg.hold_hz_enabled;
            let mut ac_hz = cfg.ac_hz;
            let mut bat_hz = cfg.bat_hz;
            let mut ch = toggle_switch(ui, &mut auto_hz, if en { "Auto-Hz on AC / Battery switch" } else { "Авто-герцовка при переключении Сеть / АКБ" });
            ui.add_space(4.0);
            ch |= toggle_switch(ui, &mut hold_hz, if en { "Hold-Hz (restore target Hz if compositor resets it)" } else { "Удерживать Hz (восстанавливать частоту при внешнем сбросе)" });
            ui.add_space(6.0);
            let hz_choices = disp.as_ref().map(|d| d.available_hz.clone()).unwrap_or_else(|| vec![60, 120]);
            ui.horizontal(|ui| {
                ui.label(if en { "AC Hz:" } else { "От сети:" });
                egui::ComboBox::from_id_salt("ac_hz_cb").selected_text(format!("{ac_hz} Hz")).show_ui(ui, |ui| {
                    for h in &hz_choices {
                        if ui.selectable_value(&mut ac_hz, *h, format!("{h} Hz")).changed() { ch = true; }
                    }
                });
                ui.add_space(16.0);
                ui.label(if en { "BAT Hz:" } else { "От АКБ:" });
                egui::ComboBox::from_id_salt("bat_hz_cb").selected_text(format!("{bat_hz} Hz")).show_ui(ui, |ui| {
                    for h in &hz_choices {
                        if ui.selectable_value(&mut bat_hz, *h, format!("{h} Hz")).changed() { ch = true; }
                    }
                });
            });
            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.auto_hz_enabled = auto_hz;
                    s.config.hold_hz_enabled = hold_hz;
                    s.config.ac_hz = ac_hz;
                    s.config.bat_hz = bat_hz;
                }
                self.save_and_notify();
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "Brightness & OLED Protection" } else { "Яркость и защита OLED" },
                None,
            );
            if let Some(mut b) = bright {
                ui.horizontal(|ui| {
                    ui.label(if en { "Screen brightness:" } else { "Яркость экрана:" });
                    if ui.add(egui::Slider::new(&mut b, 2..=100).suffix("%")).changed() {
                        let _ = sys::set_brightness_percent(b, false);
                        self.shared.lock().unwrap().brightness_pct = Some(b);
                    }
                });
                ui.add_space(6.0);
            }
            let mut rem_b = cfg.remember_brightness_per_power;
            let mut oled = cfg.oled_brightness_limit_enabled;
            let mut ac_lim = cfg.ac_brightness_limit;
            let mut bat_lim = cfg.bat_brightness_limit;
            let mut ch = toggle_switch(ui, &mut rem_b, if en { "Remember brightness separately for AC & Battery" } else { "Запоминать яркость отдельно для сети и батареи" });
            ui.add_space(4.0);
            ch |= toggle_switch(ui, &mut oled, if en { "OLED brightness cap (yields 2h if raised manually)" } else { "Лимит яркости OLED (уступает на 2ч при ручном повышении)" });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(if en { "AC cap:" } else { "Макс. от сети:" });
                ch |= ui.add(egui::Slider::new(&mut ac_lim, 20..=100).suffix("%")).changed();
                ui.add_space(12.0);
                ui.label(if en { "BAT cap:" } else { "Макс. от АКБ:" });
                ch |= ui.add(egui::Slider::new(&mut bat_lim, 20..=100).suffix("%")).changed();
            });
            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.remember_brightness_per_power = rem_b;
                    s.config.oled_brightness_limit_enabled = oled;
                    s.config.ac_brightness_limit = ac_lim;
                    s.config.bat_brightness_limit = bat_lim;
                }
                self.save_and_notify();
            }
            ui.add_space(6.0);
            let als_text = match tele.als_lux {
                Some(lux) => format!("ALS: {lux:.0} lux"),
                None => if en {
                    "ALS sensor: not exposed in Linux sysfs (/sys/bus/iio/devices absent on TM2424)"
                } else {
                    "Датчик освещённости (ALS): не проброшен ядром Linux (/sys/bus/iio/devices отсутствует на TM2424)"
                }.into(),
            };
            ui.label(RichText::new(als_text).size(11.5).color(TEXT_MUTED));
        });
    }

    fn ui_touchpad(&mut self, ui: &mut egui::Ui, en: bool) {
        let (tp, tp_en, ts_en, cfg) = {
            let s = self.shared.lock().unwrap();
            (s.touchpad_state.clone(), s.touchpad_enabled, s.touchscreen_enabled, s.config.clone())
        };

        card(ui, |ui| {
            let fw_sub = tp.as_ref().map(|t| format!("I2C 0x2c · MCU {} · Controller {}", t.firmware, t.controller_firmware));
            card_title(ui, "BLTP7853 Haptic Touchpad", fw_sub.as_deref());
            if let Some(tp) = &tp {
                ui.horizontal_wrapped(|ui| {
                    ui.label(if en { "Click Vibration:" } else { "Сила щелчка при нажатии:" });
                    for v in TouchpadVibration::ALL {
                        if chip_button(ui, v.label(en), tp.vibration == Some(v)) {
                            let r = touchpad::set_touchpad_vibration(v);
                            actions::refresh_slow(&self.shared);
                            let msg = if en { format!("Vibration: {}", v.label(true)) } else { format!("Вибрация: {}", v.label(false)) };
                            self.set_status(r, msg);
                        }
                    }
                    if ui.button(if en { "Test Pulse" } else { "Тест щелчка" }).clicked() {
                        let r = touchpad::pulse_touchpad();
                        self.set_status(r, if en { "Test pulse sent" } else { "Тестовый щелчок отправлен" }.into());
                    }
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(if en { "Click Threshold (0x5B):" } else { "Усилие клика (0x5B):" });
                    let presets = [
                        (100u16, if en { "Light (100)" } else { "Мягко (100)" }),
                        (125u16, if en { "Factory (125)" } else { "Завод (125)" }),
                        (140u16, if en { "Firm (140)" } else { "Туго (140)" }),
                    ];
                    for (val, lbl) in presets {
                        if chip_button(ui, lbl, tp.pressure == val) {
                            let r = touchpad::set_touchpad_pressure(val);
                            actions::refresh_slow(&self.shared);
                            let msg = if en { format!("Click threshold: {val}") } else { format!("Усилие клика: {val}") };
                            self.set_status(r, msg);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(if en { "Edge Swipe Click Strength (0x58):" } else { "Щелчок боковых свайпов (0x58):" });
                    for val in [40u16, 60, 80] {
                        if chip_button(ui, &format!("{val}"), self.slide_draft == val && tp.slide_strength == Some(val)) {
                            self.slide_draft = val;
                            let r = touchpad::set_touchpad_slide_strength(val);
                            actions::refresh_slow(&self.shared);
                            let msg = if en { format!("Slide click strength: {val}") } else { format!("Сила щелчка свайпа: {val}") };
                            self.set_status(r, msg);
                        }
                    }
                    ui.add(egui::Slider::new(&mut self.slide_draft, 40..=100));
                    if ui.button(if en { "Apply" } else { "Применить" }).clicked() {
                        let r = touchpad::set_touchpad_slide_strength(self.slide_draft);
                        actions::refresh_slow(&self.shared);
                        let msg = if en { format!("Slide click strength: {}", self.slide_draft) } else { format!("Сила щелчка свайпа: {}", self.slide_draft) };
                        self.set_status(r, msg);
                    }
                });
                ui.label(
                    RichText::new(if en {
                        "Sets the hardware motor amplitude for haptic ticks when sliding along the left/right edges."
                    } else {
                        "Задаёт аппаратную амплитуду щелчка мотора (0x32) при свайпе громкости/яркости по краям."
                    })
                    .size(11.5)
                    .color(TEXT_MUTED),
                );
                ui.add_space(6.0);
                let mut hp = tp.heavy_press;
                if toggle_switch(ui, &mut hp, if en { "Hardware Heavy Press 2nd click (0x59)" } else { "Аппаратное 2-е нажатие Heavy Press (0x59)" }) {
                    let r = touchpad::set_touchpad_heavy_press(hp);
                    actions::refresh_slow(&self.shared);
                    let msg = match (hp, en) {
                        (true, true) => "Hardware Heavy Press: ON",
                        (false, true) => "Hardware Heavy Press: OFF",
                        (true, false) => "Аппаратный Heavy Press: вкл",
                        (false, false) => "Аппаратный Heavy Press: выкл",
                    };
                    self.set_status(r, msg.into());
                }
            } else {
                ui.label(RichText::new(if en { "BLTP7853 unavailable (/dev/i2c-1 permissions required)" } else { "BLTP7853 недоступен (проверьте права на /dev/i2c-1)" }).color(TEXT_MUTED));
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "Edge Sliders, Heavy Press & Input Devices" } else { "Слайдеры, сильное нажатие и KWin-ввод" },
                Some(if en { "Reads hidraw (BLTP7853) directly: left edge controls brightness, right edge controls volume" } else { "Чтение hidraw (BLTP7853): левый край регулирует яркость, правый — громкость" }),
            );

            let mut rem_tp = cfg.touchpad_remember_disabled;
            ui.horizontal_wrapped(|ui| {
                if chip_button_colored(ui, if en { "Touchpad" } else { "Тачпад" }, tp_en.unwrap_or(true), GREEN) {
                    let r = toggle_touchpad(&self.shared).map(|_| ());
                    notify_daemon_reload();
                    self.set_status(r, if en { "Touchpad updated" } else { "Тачпад обновлён" }.into());
                }
                if chip_button_colored(ui, if en { "Touchscreen" } else { "Сенсор экрана" }, ts_en.unwrap_or(true), GREEN) {
                    let r = toggle_touchscreen(&self.shared).map(|_| ());
                    notify_daemon_reload();
                    self.set_status(r, if en { "Touchscreen updated" } else { "Сенсорный экран обновлён" }.into());
                }
                if ui.checkbox(&mut rem_tp, if en { "Remember disabled touchpad" } else { "Помнить выключенный тачпад" }).changed() {
                    self.shared.lock().unwrap().config.touchpad_remember_disabled = rem_tp;
                    self.save_and_notify();
                }
            });
            ui.add_space(8.0);

            let mut edge_en = cfg.touchpad_edge_sliders;
            let mut w_mm = cfg.touchpad_edge_width_mm;
            let mut sw = cfg.touchpad_edge_swipes_per_range;
            let mut pulse = cfg.touchpad_edge_pulse;
            let mut pulse_ms = cfg.touchpad_edge_pulse_ms as u32;
            let mut dz_en = cfg.touchpad_dead_zone_enabled;
            let mut dz_mm = cfg.touchpad_dead_zone_mm;
            let mut hp_th = cfg.touchpad_heavy_press_threshold;
            let mut hp_act = cfg.action_heavy_press;
            let mut ch = false;

            let total_w = ui.available_width();
            let right_w = 228.0;
            let left_w = (total_w - right_w - 16.0).max(260.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(left_w);
                        ch |= toggle_switch(ui, &mut edge_en, if en { "Enable Edge Sliders" } else { "Включить жесты по краям" });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(if en { "Strip width:" } else { "Ширина полосы:" });
                            ch |= ui.add(egui::Slider::new(&mut w_mm, 5..=25).suffix(" mm")).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(if en { "Swipes 0..100%:" } else { "Свайпов 0..100%:" });
                            ch |= ui.add(egui::Slider::new(&mut sw, 0.5..=3.0)).changed();
                        });
                        ch |= toggle_switch(ui, &mut pulse, if en { "Haptic ticks while sliding (strength = 0x58)" } else { "Тактильные щелчки при свайпе (сила = 0x58)" });
                        ui.horizontal(|ui| {
                            ui.label(if en { "Tick interval:" } else { "Пауза между щелчками:" });
                            ch |= ui.add(egui::Slider::new(&mut pulse_ms, 20..=200).suffix(" ms")).changed();
                        });
                        ui.add_space(4.0);
                        ch |= toggle_switch(ui, &mut dz_en, if en { "Bottom dead zone (palm rejection)" } else { "Нижняя мёртвая зона (защита от ладони)" });
                        ui.horizontal(|ui| {
                            ui.label(if en { "Dead zone height:" } else { "Высота зоны:" });
                            ch |= ui.add(egui::Slider::new(&mut dz_mm, 0..=25).suffix(" mm")).changed();
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(if en {
                                    "Suppresses touches starting inside the bottom strip (cursor/taps & edge sliders). Gestures started above and physical clicks still work."
                                } else {
                                    "Гасит старт касаний (курсор, тапы и слайдеры) в нижней полосе. Жесты сверху и физический клик работают штатно."
                                })
                                .size(11.0)
                                .color(TEXT_MUTED),
                            )
                            .wrap(),
                        );
                        if dz_en && dz_mm > 0 {
                            if let Some(err) = self.tp_evdev_error.clone() {
                                ui.add_space(4.0);
                                let col = AMBER;
                                egui::Frame::new()
                                    .fill(tinted_fill(col, BG_CARD, 0.14))
                                    .stroke(Stroke::new(1.0, col))
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(Margin::symmetric(8, 6))
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(if en {
                                                    format!("Cursor/tap filter inactive: {err}. Run sudo ./setup-xicontrol.sh")
                                                } else {
                                                    format!("Фильтр курсора/тапов неактивен: {err}. Выполните: sudo ./setup-xicontrol.sh")
                                                })
                                                .size(11.0)
                                                .color(col),
                                            )
                                            .wrap(),
                                        );
                                        if ui.small_button(if en { "Copy fix cmd" } else { "Копировать команду" }).clicked() {
                                            ui.ctx().copy_text("sudo ./setup-xicontrol.sh".into());
                                            self.set_status(Ok(()), if en { "Command copied" } else { "Команда скопирована" }.into());
                                        }
                                    });
                            } else if !self.daemon_online {
                                ui.label(
                                    RichText::new(if en {
                                        "Note: cursor/tap dead zone requires background daemon running."
                                    } else {
                                        "Примечание: фильтрация курсора работает при запущенном демоне."
                                    })
                                    .size(11.0)
                                    .color(AMBER),
                                );
                            } else {
                                ui.label(
                                    RichText::new(if en {
                                        "Dead zone evdev filter active."
                                    } else {
                                        "Фильтр касаний evdev активен."
                                    })
                                    .size(11.0)
                                    .color(GREEN),
                                );
                            }
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(if en { "Heavy press threshold:" } else { "Порог сильного нажатия:" });
                            ch |= ui.add(egui::Slider::new(&mut hp_th, 100..=1200)).changed();
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if en { "Heavy press action:" } else { "Действие при сильном нажатии:" });
                            egui::ComboBox::from_id_salt("tp_hp_act_cb")
                                .width(210.0)
                                .selected_text(hp_act.label(en))
                                .show_ui(ui, |ui| {
                                    for a in KeyAction::ALL {
                                        if ui.selectable_value(&mut hp_act, a, a.label(en)).changed() {
                                            ch = true;
                                        }
                                    }
                                });
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{}: {} | {}: {}",
                                    if en { "Live pressure" } else { "Давление сейчас" },
                                    self.live_tp_pressure,
                                    if en { "Peak" } else { "Пик" },
                                    self.peak_tp_pressure
                                ))
                                .size(11.5)
                                .color(if self.live_tp_pressure >= hp_th { GREEN } else { TEXT_MUTED }),
                            );
                            if ui.small_button(if en { "Reset" } else { "Сброс пика" }).clicked() {
                                if self.daemon_online {
                                    let _ = daemon::send_ipc_command("RESET_TP_PEAK");
                                }
                                self.shared.lock().unwrap().peak_tp_pressure = self.live_tp_pressure;
                                self.peak_tp_pressure = self.live_tp_pressure;
                            }
                        });
                        progress_bar(
                            ui,
                            (self.live_tp_pressure as f32 / 1200.0).clamp(0.0, 1.0),
                            if self.live_tp_pressure >= hp_th { GREEN } else { ACCENT },
                            5.0,
                        );
                    },
                );

                ui.add_space(16.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.label(RichText::new(if en { "Zone Preview" } else { "Карта зон" }).size(12.0).color(TEXT_MUTED).strong());
                        ui.add_space(4.0);
                        touchpad_diagram(ui, w_mm, if dz_en { dz_mm } else { 0 }, edge_en, en);
                    },
                );
            });

            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.touchpad_edge_sliders = edge_en;
                    s.config.touchpad_edge_width_mm = w_mm;
                    s.config.touchpad_edge_swipes_per_range = sw;
                    s.config.touchpad_edge_pulse = pulse;
                    s.config.touchpad_edge_pulse_ms = pulse_ms as u64;
                    s.config.touchpad_dead_zone_enabled = dz_en;
                    s.config.touchpad_dead_zone_mm = dz_mm;
                    s.config.touchpad_heavy_press_threshold = hp_th;
                    s.config.action_heavy_press = hp_act;
                }
                self.save_and_notify();
            }
        });
    }

    fn ui_keys(&mut self, ui: &mut egui::Ui, en: bool) {
        let cfg = self.shared.lock().unwrap().config.clone();

        card(ui, |ui| {
            card_title(
                ui,
                if en { "WMI Hardware Keys Status" } else { "Статус аппаратных клавиш WMI" },
                None,
            );
            if let Some(err) = self.wmi_input_error.clone() {
                let col = AMBER;
                egui::Frame::new()
                    .fill(tinted_fill(col, BG_CARD, 0.14))
                    .stroke(Stroke::new(1.0, col))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if en {
                                format!("Hardware WMI listener blocked: {err}")
                            } else {
                                format!("Аппаратный перехват Fn/WMI заблокирован: {err}")
                            })
                            .color(col)
                            .strong(),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(if en {
                                "Your user does not have read access to Redmibook WMI keys (/dev/input/event*). Run setup-xicontrol.sh with sudo to install udev rules:"
                            } else {
                                "Нет прав на чтение Redmibook WMI keys (/dev/input/event*). Выполните в терминале, чтобы выдать udev-права:"
                            })
                            .size(12.0)
                            .color(TEXT_PRIMARY),
                        );
                        ui.add_space(4.0);
                        let fix_cmd = "sudo ./setup-xicontrol.sh";
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(fix_cmd).monospace().color(ACCENT));
                            if ui.small_button(if en { "Copy" } else { "Копировать" }).clicked() {
                                ui.ctx().copy_text(fix_cmd.into());
                                self.set_status(Ok(()), if en { "Command copied" } else { "Команда скопирована" }.into());
                            }
                        });
                    });
            } else {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, GREEN);
                    ui.label(
                        RichText::new(if en {
                            "Redmibook WMI keys listener active (/dev/input/event*)"
                        } else {
                            "Аппаратные WMI-клавиши активны (Fn+K / Настройки / Проекция)"
                        })
                        .color(GREEN)
                        .strong(),
                    );
                });
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                if en { "Action Slots (Heavy Press / WMI / KDE Shortcuts)" } else { "Слоты действий и привязка клавиш" },
                Some(if en {
                    "Runs automatically via daemon WMI/hidraw listener, or bind `xicontrol trigger <slot>` in KDE Shortcuts."
                } else {
                    "Срабатывают автоматически через демон, либо по команде `xicontrol trigger <slot>` в Горячих клавишах KDE."
                }),
            );

            let mut hp = cfg.action_heavy_press;
            let mut wp = cfg.action_wmi_perf;
            let mut wh = cfg.action_wmi_hz;
            let mut wt = cfg.action_wmi_tp;
            let mut c1 = cfg.action_custom_1;
            let mut c2 = cfg.action_custom_2;
            let mut cmd = cfg.custom_command.clone();
            let mut osd = cfg.osd_enabled;
            let mut ch = false;

            let rows: [(&str, &str, &mut KeyAction); 6] = [
                ("heavy_press", if en { "Touchpad Heavy Press:" } else { "Сильное нажатие тачпада:" }, &mut hp),
                ("wmi_perf", if en { "Perf key (WMI / Fn+K):" } else { "Кнопка режима (WMI / Fn+K):" }, &mut wp),
                ("wmi_hz", if en { "Refresh rate slot (CLI / KDE):" } else { "Смена герцовки (CLI / KDE):" }, &mut wh),
                ("wmi_tp", if en { "Touchpad toggle slot:" } else { "Слот переключения тачпада:" }, &mut wt),
                ("custom_1", if en { "Custom slot 1 (CLI / KDE):" } else { "Слот 1 (custom_1, CLI / KDE):" }, &mut c1),
                ("custom_2", if en { "Settings key / slot 2 (WMI 171):" } else { "Клавиша «Настройки» / слот 2 (WMI 171):" }, &mut c2),
            ];

            let mut test_act: Option<KeyAction> = None;
            let mut copied_cmd: Option<String> = None;
            egui::Grid::new("key_actions_grid").spacing([12.0, 8.0]).show(ui, |ui| {
                for (id, lbl, slot) in rows {
                    ui.label(lbl);
                    egui::ComboBox::from_id_salt(format!("ka_{id}"))
                        .width(240.0)
                        .selected_text(slot.label(en))
                        .show_ui(ui, |ui| {
                            for a in KeyAction::ALL {
                                if ui.selectable_value(slot, a, a.label(en)).changed() {
                                    ch = true;
                                }
                            }
                        });
                    if ui.button(if en { "Run" } else { "Тест" }).clicked() {
                        test_act = Some(*slot);
                    }
                    if ui.small_button(if en { "Copy CLI" } else { "Копировать CLI" }).clicked() {
                        copied_cmd = Some(format!("xicontrol trigger {id}"));
                    }
                    ui.end_row();
                }
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(if en { "Custom command:" } else { "Своя команда:" });
                if ui.add(egui::TextEdit::singleline(&mut cmd).desired_width(380.0)).changed() {
                    ch = true;
                }
            });
            ui.add_space(6.0);
            if toggle_switch(ui, &mut osd, if en { "Show KDE Plasma OSD notifications" } else { "Показывать экранные уведомления KDE OSD" }) {
                ch = true;
            }

            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.action_heavy_press = hp;
                    s.config.action_wmi_perf = wp;
                    s.config.action_wmi_hz = wh;
                    s.config.action_wmi_tp = wt;
                    s.config.action_custom_1 = c1;
                    s.config.action_custom_2 = c2;
                    s.config.custom_command = cmd;
                    s.config.osd_enabled = osd;
                }
                self.save_and_notify();
            }
            if let Some(a) = test_act {
                let r = actions::execute_action(&self.shared, a);
                self.set_status(r, a.label(en).into());
            }
            if let Some(cli) = copied_cmd {
                ui.ctx().copy_text(cli.clone());
                let msg = if en { format!("Copied: {cli}") } else { format!("Скопировано: {cli}") };
                self.set_status(Ok(()), msg);
            }
        });
    }

    fn ui_monitor(&mut self, ui: &mut egui::Ui, en: bool) {
        let (tele, samples, sensors, mut sensor_key) = {
            let s = self.shared.lock().unwrap();
            let vec: Vec<_> = s.history.samples.iter().cloned().collect();
            (
                s.telemetry.clone(),
                vec,
                s.temp_sensors.clone(),
                s.config.temp_sensor_key.clone(),
            )
        };

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(if en { "Telemetry Summary" } else { "Сводка датчиков" }).size(14.5).color(Color32::WHITE).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cur_label = sensors
                        .iter()
                        .find(|x| x.key == sensor_key)
                        .map(|x| x.label.clone())
                        .unwrap_or_else(|| if en { "Auto (max CPU)" } else { "Авто (макс. CPU)" }.into());
                    let mut ch = false;
                    egui::ComboBox::from_id_salt("temp_sensor_cb")
                        .selected_text(cur_label)
                        .show_ui(ui, |ui| {
                            if ui.selectable_value(&mut sensor_key, "auto_max".into(), if en { "Auto (max CPU)" } else { "Авто (макс. CPU)" }).changed() {
                                ch = true;
                            }
                            for sinfo in &sensors {
                                let lbl = format!("{} ({:.0}°C)", sinfo.label, sinfo.temp_c);
                                if ui.selectable_value(&mut sensor_key, sinfo.key.clone(), lbl).changed() {
                                    ch = true;
                                }
                            }
                        });
                    if ch {
                        self.shared.lock().unwrap().config.temp_sensor_key = sensor_key;
                        self.save_and_notify();
                    }
                    ui.label(RichText::new(if en { "Temp sensor:" } else { "Датчик °C:" }).color(TEXT_MUTED));
                });
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                let cpu_w = tele.cpu_watts.map(|w| format!(" · {w:.1}W")).unwrap_or_default();
                ui.label(RichText::new(format!("CPU {:.1}% · {:.0}°C{cpu_w}", tele.cpu_pct, tele.cpu_temp_c)).color(ACCENT).strong());
                ui.separator();
                let gpu = tele.gpu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into());
                let gpu_f = tele.gpu_freq_mhz.map(|f| format!(" @ {f} MHz")).unwrap_or_default();
                ui.label(RichText::new(format!("GPU {gpu}{gpu_f}")).color(Color32::from_rgb(0xA7, 0x8B, 0xFA)).strong());
                ui.separator();
                let npu = tele.npu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into());
                let npu_f = tele.npu_freq_mhz.map(|f| format!(" @ {f} MHz")).unwrap_or_default();
                ui.label(RichText::new(format!("NPU {npu}{npu_f}")).color(Color32::from_rgb(0x2D, 0xD4, 0xBF)).strong());
                ui.separator();
                ui.label(RichText::new(format!("RAM {:.1}/{:.0} GB ({:.0}%)", tele.ram_used_gb, tele.ram_total_gb, tele.ram_pct)).color(GREEN).strong());
                ui.separator();
                ui.label(RichText::new(format!("BAT {:+.1} W", tele.bat_watts)).color(AMBER).strong());
            });
        });
        ui.add_space(10.0);

        let cpu_pts: Vec<f32> = samples.iter().map(|x| x.cpu_pct).collect();
        let temp_pts: Vec<f32> = samples.iter().map(|x| x.cpu_temp_c).collect();
        let gpu_pts: Vec<f32> = samples.iter().map(|x| x.gpu_pct).collect();
        let npu_pts: Vec<f32> = samples.iter().map(|x| x.npu_pct).collect();
        let ram_pts: Vec<f32> = samples.iter().map(|x| x.ram_pct).collect();
        let cpu_w_pts: Vec<f32> = samples.iter().map(|x| x.cpu_watts).collect();
        let bat_w_pts: Vec<f32> = samples.iter().map(|x| x.bat_watts.abs()).collect();

        ui.columns(2, |cols| {
            draw_area_chart_card(&mut cols[0], if en { "CPU Load" } else { "Загрузка CPU" }, "%", &cpu_pts, 100.0, ACCENT);
            draw_area_chart_card(&mut cols[1], if en { "CPU Temperature" } else { "Температура CPU" }, "°C", &temp_pts, 105.0, Color32::from_rgb(0xFB, 0x92, 0x3C));
        });
        ui.add_space(8.0);
        ui.columns(2, |cols| {
            draw_area_chart_card(&mut cols[0], if en { "CPU Package Power" } else { "Мощность CPU" }, "W", &cpu_w_pts, 65.0, Color32::from_rgb(0xF4, 0x72, 0xB6));
            draw_area_chart_card(&mut cols[1], if en { "Battery Power" } else { "Мощность АКБ" }, "W", &bat_w_pts, 100.0, AMBER);
        });
        ui.add_space(8.0);
        ui.columns(2, |cols| {
            draw_area_chart_card(&mut cols[0], if en { "GPU Load" } else { "Загрузка GPU" }, "%", &gpu_pts, 100.0, Color32::from_rgb(0xA7, 0x8B, 0xFA));
            draw_area_chart_card(&mut cols[1], if en { "NPU Load" } else { "Загрузка NPU" }, "%", &npu_pts, 100.0, Color32::from_rgb(0x2D, 0xD4, 0xBF));
        });
        ui.add_space(8.0);
        draw_area_chart_card(ui, if en { "RAM Usage" } else { "Использование RAM" }, "%", &ram_pts, 100.0, GREEN);
    }

    fn ui_system(&mut self, ui: &mut egui::Ui, en: bool) {
        let (cfg, hw, autostart) = {
            let s = self.shared.lock().unwrap();
            (s.config.clone(), s.hw_info.clone(), s.autostart_enabled)
        };

        card(ui, |ui| {
            card_title(
                ui,
                if en { "KDE Tray & Autostart" } else { "Системный трей KDE и автозапуск" },
                None,
            );
            let mut tray_on = cfg.tray_enabled;
            let mut metric_on = cfg.tray_metric_enabled;
            let mut metric_kind = cfg.tray_metric_kind;
            let mut interval = cfg.tray_metric_interval_sec as u32;
            let mut auto = autostart;

            let mut ch = toggle_switch(ui, &mut tray_on, if en { "Show main status icon in KDE tray" } else { "Показывать основной значок в трее KDE" });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ch |= toggle_switch(ui, &mut metric_on, if en { "Second live metric icon:" } else { "Второй живой индикатор:" });
                egui::ComboBox::from_id_salt("tray_metric_cb")
                    .selected_text(metric_kind.label(en))
                    .show_ui(ui, |ui| {
                        for k in TrayMetricKind::ALL {
                            if ui.selectable_value(&mut metric_kind, k, k.label(en)).changed() { ch = true; }
                        }
                    });
                ui.label(if en { "Interval:" } else { "Интервал:" });
                ch |= ui.add(egui::Slider::new(&mut interval, 1..=15).suffix(" s")).changed();
            });
            if ch {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.config.tray_enabled = tray_on;
                    s.config.tray_metric_enabled = metric_on;
                    s.config.tray_metric_kind = metric_kind;
                    s.config.tray_metric_interval_sec = interval as u64;
                }
                self.save_and_notify();
            }

            ui.add_space(6.0);
            if toggle_switch(ui, &mut auto, if en { "Start daemon at login (systemd --user)" } else { "Автозапуск фонового демона (systemd --user)" }) {
                let r = sys::set_autostart_enabled(auto);
                actions::refresh_slow(&self.shared);
                let msg = match (auto, en) {
                    (true, true) => "Autostart: enabled",
                    (false, true) => "Autostart: disabled",
                    (true, false) => "Автозапуск: включён",
                    (false, false) => "Автозапуск: выключен",
                };
                self.set_status(r, msg.into());
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(
                ui,
                "HTTP API",
                Some(if en { "Local automation endpoint with SHA-256 Bearer authentication" } else { "Локальный HTTP-сервер автоматизации с Bearer-аутентификацией (SHA-256)" }),
            );
            let mut api = cfg.http_api.clone();
            let mut ch = toggle_switch(ui, &mut api.enabled, if en { "Enable HTTP API server" } else { "Включить HTTP API сервер" });
            ui.add_space(6.0);
            let mut port_i = api.port as i32;
            ui.horizontal(|ui| {
                ui.label(if en { "Port:" } else { "Порт:" });
                if ui.add(egui::DragValue::new(&mut port_i).range(1024..=65535)).changed() {
                    api.port = port_i as u16;
                    ch = true;
                }
                ui.add_space(12.0);
                ch |= ui.checkbox(&mut api.lan_access, if en { "Allow LAN (0.0.0.0)" } else { "Доступ из сети (0.0.0.0)" }).changed();
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ch |= ui.checkbox(&mut api.allow_mode, "POST /mode").changed();
                ch |= ui.checkbox(&mut api.allow_care, "POST /care").changed();
                ch |= ui.checkbox(&mut api.allow_travel, "POST /travel").changed();
                ch |= ui.checkbox(&mut api.allow_owl, "POST /owl").changed();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(if en { "Generate Bearer Token" } else { "Создать Bearer токен" }).clicked() {
                    match generate_api_token() {
                        Ok(tok) => {
                            api.token_sha256 = sha256_hex(tok.as_bytes());
                            self.revealed_token = Some(tok);
                            ch = true;
                        }
                        Err(e) => self.set_status(Err(e), String::new()),
                    }
                }
                let has_tok = !api.token_sha256.is_empty();
                let tok_info = match (has_tok, en) {
                    (true, true) => "SHA-256 token active",
                    (true, false) => "SHA-256 токен активен",
                    (false, true) => "No token (GET /status open locally)",
                    (false, false) => "Токен не задан (GET /status открыт локально)",
                };
                ui.label(RichText::new(tok_info).size(11.5).color(TEXT_MUTED));
            });
            if let Some(tok) = &self.revealed_token {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}: {tok}", if en { "Token" } else { "Токен" })).color(AMBER).monospace());
                    if ui.button(if en { "Copy" } else { "Копировать" }).clicked() {
                        ui.ctx().copy_text(tok.clone());
                    }
                });
            }
            if ch {
                self.shared.lock().unwrap().config.http_api = api;
                self.save_and_notify();
            }
        });
        ui.add_space(10.0);

        card(ui, |ui| {
            card_title(ui, if en { "Hardware & Config" } else { "Система и конфигурация" }, None);
            ui.label(RichText::new(format!("{} {} ({}) · BIOS {} ({})", hw.vendor, hw.model, hw.sku, hw.bios_version, hw.bios_date)).color(TEXT_PRIMARY));
            let mode_str = if is_portable_mode() { "Portable" } else { "User XDG" };
            ui.label(RichText::new(format!("Config ({mode_str}): {}", config::config_path().display())).size(11.5).color(TEXT_MUTED));
        });
    }
}

// ============================================================================
// AREA CHART WIDGET (GRID + SHADED MESH + GLOW POINT)
// ============================================================================
fn draw_area_chart_card(ui: &mut egui::Ui, title: &str, unit: &str, values: &[f32], max_y: f32, color: Color32) {
    card(ui, |ui| {
        let cur = values.last().copied().unwrap_or(0.0);
        let peak = values.iter().copied().fold(0.0f32, f32::max);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(13.5).color(TEXT_PRIMARY).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{cur:.1}{unit}")).size(14.0).color(color).strong());
                ui.label(RichText::new(format!("max {peak:.1}{unit} ·")).size(11.0).color(TEXT_MUTED));
            });
        });
        ui.add_space(4.0);

        let desired = Vec2::new(ui.available_width().max(180.0), 82.0);
        let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, BG_SIDEBAR);
        p.rect_stroke(rect, 8.0, Stroke::new(1.0, BORDER), StrokeKind::Inside);

        // Subtle horizontal guide lines
        for frac in [0.25f32, 0.5, 0.75] {
            let y = rect.bottom() - 6.0 - frac * (rect.height() - 12.0);
            p.line_segment(
                [Pos2::new(rect.left() + 6.0, y), Pos2::new(rect.right() - 6.0, y)],
                Stroke::new(1.0, Color32::from_rgb(0x18, 0x22, 0x33)),
            );
        }

        if values.len() >= 2 {
            let n = values.len() as f32;
            let pts: Vec<Pos2> = values
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let x = rect.left() + 6.0 + (i as f32 / (n - 1.0).max(1.0)) * (rect.width() - 12.0);
                    let frac = (v / max_y).clamp(0.0, 1.0);
                    let y = rect.bottom() - 6.0 - frac * (rect.height() - 12.0);
                    Pos2::new(x, y)
                })
                .collect();

            // Shaded vertical gradient ribbon under line
            let top_col = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 52);
            let bot_col = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 4);
            let mut mesh = egui::Mesh::default();
            let base_y = rect.bottom() - 6.0;
            for pt in &pts {
                mesh.colored_vertex(*pt, top_col);
                mesh.colored_vertex(Pos2::new(pt.x, base_y), bot_col);
            }
            for i in 0..pts.len() - 1 {
                let idx = (i * 2) as u32;
                mesh.add_triangle(idx, idx + 1, idx + 2);
                mesh.add_triangle(idx + 1, idx + 2, idx + 3);
            }
            p.add(egui::Shape::mesh(mesh));
            p.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, color)));

            if let Some(&last) = pts.last() {
                p.circle_filled(last, 4.5, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 90));
                p.circle_filled(last, 2.5, Color32::WHITE);
            }
        } else {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "...",
                FontId::proportional(12.0),
                TEXT_MUTED,
            );
        }
    });
}
