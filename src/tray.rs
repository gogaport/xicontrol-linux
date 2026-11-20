use crate::actions::{
    apply_charge_limit, set_charge_care, set_display_hz_checked, set_owl_mode,
    set_perf_mode_checked, set_travel_mode, toggle_touchpad, toggle_touchscreen, SharedHandle,
};
use crate::config::{contains_perf_mode, TrayMetricKind};
use crate::hw::mifs::{PerfMode, CHARGE_PRESETS};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip, Tray};
use std::time::{Duration, Instant};

struct TraySnapshot {
    en: bool,
    ac_online: bool,
    mode: Option<PerfMode>,
    unavail: Vec<String>,
    charge_lim: u8,
    care_en: bool,
    travel: bool,
    owl: bool,
    tp_en: bool,
    ts_en: bool,
    hz_list: Vec<u32>,
    cur_hz: u32,
    tip: String,
}

struct MainTray {
    shared: SharedHandle,
}

impl MainTray {
    fn snapshot(&self) -> TraySnapshot {
        let s = self.shared.lock().unwrap();
        let en = s.config.language.is_en();
        let mode_str = s.perf_mode.map(|m| m.label(en)).unwrap_or("—");
        let tip = format!(
            "XiControl · {} · {}% ({:+.1}W) · {:.0}°C · CPU {:.0}% · RAM {:.0}%",
            mode_str,
            s.telemetry.bat_capacity,
            s.telemetry.bat_watts,
            s.telemetry.cpu_temp_c,
            s.telemetry.cpu_pct,
            s.telemetry.ram_pct,
        );
        let (hz_list, cur_hz) = match &s.display {
            Some(d) => (d.available_hz.clone(), d.current_hz),
            None => (vec![60, 120], 120),
        };
        TraySnapshot {
            en,
            ac_online: s.telemetry.ac_online,
            mode: s.perf_mode,
            unavail: s.config.unavailable_modes.clone(),
            charge_lim: s.charge_limit.unwrap_or(s.config.charge_care_percent),
            care_en: s.config.charge_care_enabled,
            travel: s.config.travel_mode,
            owl: s.owl_active,
            tp_en: s.touchpad_enabled.unwrap_or(true),
            ts_en: s.touchscreen_enabled.unwrap_or(true),
            hz_list,
            cur_hz,
            tip,
        }
    }
}

impl Tray for MainTray {
    fn id(&self) -> String {
        "xicontrol-main".into()
    }

    fn title(&self) -> String {
        "XiControl".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.shared.lock().unwrap().gui_requested = true;
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let mode = self.shared.lock().unwrap().perf_mode;
        let (r, g, b) = match mode {
            Some(PerfMode::Eco) => (46, 178, 102),
            Some(PerfMode::Quiet) => (64, 148, 224),
            Some(PerfMode::Balance) => (42, 168, 168),
            Some(PerfMode::Auto) => (138, 102, 232),
            Some(PerfMode::Turbo) => (235, 134, 42),
            Some(PerfMode::FullSpeed) => (224, 68, 78),
            None => (110, 118, 129),
        };
        vec![render_badge_icon(r, g, b, "XI")]
    }

    fn tool_tip(&self) -> ToolTip {
        let snap = self.snapshot();
        ToolTip {
            title: "XiControl — Xiaomi Book Pro 14".into(),
            description: snap.tip,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let snap = self.snapshot();
        let en = snap.en;
        let care_en = snap.care_en;
        let travel = snap.travel;
        let owl = snap.owl;
        let charge_lim = snap.charge_lim;
        let cur_hz = snap.cur_hz;

        let mode_items: Vec<MenuItem<Self>> = PerfMode::ALL
            .iter()
            .copied()
            .map(|m| {
                let is_unavail = contains_perf_mode(&snap.unavail, m);
                let allowed = snap.ac_online || m.is_battery_capable();
                let lbl = if !snap.ac_online && !m.is_battery_capable() {
                    if en { format!("{} (AC only)", m.label(en)) } else { format!("{} (от сети)", m.label(en)) }
                } else if is_unavail {
                    format!("{} (N/A)", m.label(en))
                } else {
                    m.label(en).into()
                };
                CheckmarkItem {
                    label: lbl,
                    enabled: allowed,
                    checked: snap.mode == Some(m),
                    activate: Box::new(move |this: &mut Self| {
                        let shared = this.shared.clone();
                        std::thread::spawn(move || {
                            let _ = set_perf_mode_checked(&shared, m);
                        });
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

        let charge_items: Vec<MenuItem<Self>> = CHARGE_PRESETS
            .iter()
            .copied()
            .map(|pct| {
                CheckmarkItem {
                    label: format!("{pct}%"),
                    checked: charge_lim == pct,
                    activate: Box::new(move |this: &mut Self| {
                        let shared = this.shared.clone();
                        std::thread::spawn(move || {
                            let _ = apply_charge_limit(&shared, pct, None);
                        });
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

        let hz_items: Vec<MenuItem<Self>> = snap
            .hz_list
            .into_iter()
            .map(|hz| {
                CheckmarkItem {
                    label: format!("{hz} Hz"),
                    checked: hz == cur_hz,
                    activate: Box::new(move |this: &mut Self| {
                        let shared = this.shared.clone();
                        std::thread::spawn(move || {
                            let _ = set_display_hz_checked(&shared, hz);
                        });
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

        vec![
            StandardItem {
                label: if en { "Open XiControl" } else { "Открыть XiControl" }.into(),
                activate: Box::new(|this: &mut Self| {
                    this.shared.lock().unwrap().gui_requested = true;
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            SubMenu {
                label: if en { "Performance Mode" } else { "Режим питания" }.into(),
                submenu: mode_items,
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: if en { "Battery Care" } else { "Беречь батарею" }.into(),
                checked: care_en,
                activate: Box::new(move |this: &mut Self| {
                    let shared = this.shared.clone();
                    std::thread::spawn(move || {
                        let _ = set_charge_care(&shared, !care_en);
                    });
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: if en { format!("Charge Limit ({charge_lim}%)") } else { format!("Порог заряда ({charge_lim}%)") },
                submenu: charge_items,
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: if en { "Travel Mode (100%)" } else { "В дорогу (100%)" }.into(),
                checked: travel,
                activate: Box::new(move |this: &mut Self| {
                    let shared = this.shared.clone();
                    std::thread::spawn(move || {
                        let _ = set_travel_mode(&shared, !travel);
                    });
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: if en { "Owl Mode (Keep Awake)" } else { "Сова (не спать)" }.into(),
                checked: owl,
                activate: Box::new(move |this: &mut Self| {
                    let shared = this.shared.clone();
                    std::thread::spawn(move || {
                        let _ = set_owl_mode(&shared, !owl);
                    });
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: if en { format!("Refresh Rate ({cur_hz} Hz)") } else { format!("Частота экрана ({cur_hz} Гц)") },
                submenu: hz_items,
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: if en { "Touchpad" } else { "Тачпад" }.into(),
                checked: snap.tp_en,
                activate: Box::new(|this: &mut Self| {
                    let shared = this.shared.clone();
                    std::thread::spawn(move || {
                        let _ = toggle_touchpad(&shared);
                    });
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: if en { "Touchscreen" } else { "Сенсорный экран" }.into(),
                checked: snap.ts_en,
                activate: Box::new(|this: &mut Self| {
                    let shared = this.shared.clone();
                    std::thread::spawn(move || {
                        let _ = toggle_touchscreen(&shared);
                    });
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if en { "Quit" } else { "Выход" }.into(),
                activate: Box::new(|this: &mut Self| {
                    this.shared.lock().unwrap().shutdown = true;
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

struct MetricTray {
    shared: SharedHandle,
}

impl MetricTray {
    fn current_text_and_color(&self) -> (String, (u8, u8, u8), String) {
        let s = self.shared.lock().unwrap();
        let kind = s.config.tray_metric_kind;
        let t = &s.telemetry;
        let text = match kind {
            TrayMetricKind::Watts => format!("{:.0}", t.bat_watts.abs().clamp(0.0, 99.0)),
            TrayMetricKind::Cpu => format!("{:.0}", t.cpu_pct.clamp(0.0, 99.0)),
            TrayMetricKind::Gpu => format!("{:.0}", t.gpu_pct.unwrap_or(0.0).clamp(0.0, 99.0)),
            TrayMetricKind::Npu => format!("{:.0}", t.npu_pct.unwrap_or(0.0).clamp(0.0, 99.0)),
            TrayMetricKind::Ram => format!("{:.0}", t.ram_pct.clamp(0.0, 99.0)),
            TrayMetricKind::Temp => format!("{:.0}", t.cpu_temp_c.clamp(0.0, 99.0)),
        };
        let color = match kind {
            TrayMetricKind::Watts => {
                if t.ac_online { (46, 178, 102) } else { (64, 148, 224) }
            }
            TrayMetricKind::Temp => {
                if t.cpu_temp_c >= 85.0 { (224, 68, 78) }
                else if t.cpu_temp_c >= 70.0 { (235, 134, 42) }
                else { (46, 178, 102) }
            }
            TrayMetricKind::Cpu | TrayMetricKind::Gpu | TrayMetricKind::Npu | TrayMetricKind::Ram => {
                (90, 128, 210)
            }
        };
        let tip = format!(
            "BAT: {:+.1}W ({}%) | CPU: {:.0}% {:.0}°C | GPU: {} | NPU: {} | RAM: {:.1}/{:.0} GB",
            t.bat_watts,
            t.bat_capacity,
            t.cpu_pct,
            t.cpu_temp_c,
            t.gpu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into()),
            t.npu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into()),
            t.ram_used_gb,
            t.ram_total_gb,
        );
        (text, color, tip)
    }
}

impl Tray for MetricTray {
    fn id(&self) -> String {
        "xicontrol-metric".into()
    }

    fn title(&self) -> String {
        "XiControl Telemetry".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.shared.lock().unwrap().gui_requested = true;
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let (text, (r, g, b), _) = self.current_text_and_color();
        vec![render_badge_icon(r, g, b, &text)]
    }

    fn tool_tip(&self) -> ToolTip {
        let (_, _, tip) = self.current_text_and_color();
        ToolTip {
            title: "XiControl Telemetry".into(),
            description: tip,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let (en, cur_kind) = {
            let s = self.shared.lock().unwrap();
            (s.config.language.is_en(), s.config.tray_metric_kind)
        };
        let mut items: Vec<MenuItem<Self>> = TrayMetricKind::ALL
            .iter()
            .copied()
            .map(|k| {
                CheckmarkItem {
                    label: k.label(en).into(),
                    checked: k == cur_kind,
                    activate: Box::new(move |this: &mut Self| {
                        let mut s = this.shared.lock().unwrap();
                        s.config.tray_metric_kind = k;
                        let _ = s.config.save();
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: if en { "Open XiControl" } else { "Открыть XiControl" }.into(),
                activate: Box::new(|this: &mut Self| {
                    this.shared.lock().unwrap().gui_requested = true;
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub fn start_tray_controller(shared: SharedHandle) {
    std::thread::spawn(move || {
        let mut main_handle: Option<Handle<MainTray>> = None;
        let mut metric_handle: Option<Handle<MetricTray>> = None;
        let mut last_metric_update = Instant::now();

        loop {
            let (tray_on, metric_on, metric_interval, shutdown) = {
                let s = shared.lock().unwrap();
                (
                    s.config.tray_enabled,
                    s.config.tray_metric_enabled,
                    s.config.tray_metric_interval_sec.clamp(1, 30),
                    s.shutdown,
                )
            };
            if shutdown {
                if let Some(h) = main_handle.take() {
                    h.shutdown().wait();
                }
                if let Some(h) = metric_handle.take() {
                    h.shutdown().wait();
                }
                break;
            }

            if tray_on && main_handle.is_none() {
                main_handle = MainTray { shared: shared.clone() }
                    .assume_sni_available(true)
                    .spawn()
                    .ok();
            } else if !tray_on {
                if let Some(h) = main_handle.take() {
                    h.shutdown().wait();
                }
            } else if let Some(h) = &main_handle {
                h.update(|_| {});
            }

            if tray_on && metric_on && metric_handle.is_none() {
                metric_handle = MetricTray { shared: shared.clone() }
                    .assume_sni_available(true)
                    .spawn()
                    .ok();
                last_metric_update = Instant::now();
            } else if (!tray_on || !metric_on) && metric_handle.is_some() {
                if let Some(h) = metric_handle.take() {
                    h.shutdown().wait();
                }
            } else if let Some(h) = &metric_handle {
                if last_metric_update.elapsed() >= Duration::from_secs(metric_interval) {
                    h.update(|_| {});
                    last_metric_update = Instant::now();
                }
            }

            std::thread::sleep(Duration::from_millis(900));
        }
    });
}

// ============================================================================
// 32x32 ARGB32 BADGE + BITMAP GLYPH RENDERER
// ============================================================================
fn glyph_bits(ch: char) -> [u8; 7] {
    // 5-bit wide x 7-row bitmap font
    match ch {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        _ => [0; 7],
    }
}

fn render_badge_icon(r: u8, g: u8, b: u8, text: &str) -> Icon {
    const W: usize = 32;
    const H: usize = 32;
    let mut data = vec![0u8; W * H * 4];

    // Draw filled rounded square background (2..30)
    for y in 2..30 {
        for x in 2..30 {
            let corner = (x < 6 || x >= 26) && (y < 6 || y >= 26);
            if corner {
                let dx = if x < 6 { 6 - x } else { x - 25 };
                let dy = if y < 6 { 6 - y } else { y - 25 };
                if dx * dx + dy * dy > 16 {
                    continue;
                }
            }
            let idx = (y * W + x) * 4;
            data[idx] = 255; // A (ARGB32 big-endian / network byte order)
            data[idx + 1] = r;
            data[idx + 2] = g;
            data[idx + 3] = b;
        }
    }

    // Center up to 2 glyphs at 2x scale (each glyph 10x14, gap 2)
    let chars: Vec<char> = text.chars().take(2).collect();
    let total_w = if chars.len() == 2 { 22 } else { 10 };
    let start_x = (W - total_w) / 2;
    let start_y = (H - 14) / 2;

    for (ci, ch) in chars.iter().enumerate() {
        let bits = glyph_bits(*ch);
        let gx0 = start_x + ci * 12;
        for (row, &row_bits) in bits.iter().enumerate() {
            for col in 0..5 {
                if (row_bits & (1 << (4 - col))) != 0 {
                    for sy in 0..2 {
                        for sx in 0..2 {
                            let px = gx0 + col * 2 + sx;
                            let py = start_y + row * 2 + sy;
                            let idx = (py * W + px) * 4;
                            data[idx] = 255;
                            data[idx + 1] = 255;
                            data[idx + 2] = 255;
                            data[idx + 3] = 255;
                        }
                    }
                }
            }
        }
    }

    Icon {
        width: W as i32,
        height: H as i32,
        data,
    }
}
