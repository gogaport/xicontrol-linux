mod actions;
mod config;
mod daemon;
mod http_api;
mod hw;
mod tray;
mod ui;

use actions::{
    apply_charge_limit, set_charge_care, set_display_hz_checked, set_perf_mode_checked,
    set_travel_mode, toggle_touchpad, toggle_touchscreen, SharedState,
};
use clap::{Parser, Subcommand};
use config::AppConfig;
use hw::mifs::{
    get_adapter_watts, get_battery_health, get_charge_limit, get_mic_mute, get_perf_mode,
    PerfMode,
};
use hw::sys::{
    is_autostart_enabled, query_internal_display, read_hardware_info, set_autostart_enabled,
    TelemetrySampler,
};
use hw::touchpad::{
    is_touchpad_enabled, is_touchscreen_enabled, pulse_touchpad, read_touchpad_state,
    set_touchpad_enabled, set_touchpad_heavy_press, set_touchpad_pressure,
    set_touchpad_slide_strength, set_touchpad_vibration, set_touchscreen_enabled,
    TouchpadVibration,
};

#[derive(Parser)]
#[command(
    name = "xicontrol",
    version = "0.2.0",
    about = "XiControl Linux — Xiaomi Book Pro 14 (TM2424): MIFS, BLTP7853, Display, Tray & HTTP API"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Открыть графическую панель управления (по умолчанию)
    Gui,
    /// Внутренняя команда: открыть только окно GUI (используется демоном)
    #[command(hide = true)]
    GuiWindow,
    /// Запустить фоновый демон (трей, жесты тачпада, ChargeGuard, HTTP API, AC/BAT профили)
    Daemon,
    /// Показать текущее состояние железа и демона
    Status,
    /// Установить режим производительности: eco | quiet | balance | auto | turbo | fullspeed
    SetMode { mode: String },
    /// Установить аппаратный лимит заряда батареи в процентах (40..=100)
    SetCharge { percent: u8 },
    /// Включить или выключить «Беречь батарею»: on | off
    Care { state: String },
    /// Включить или выключить режим «В дорогу» (100%): on | off
    Travel { state: String },
    /// Включить или выключить режим «Сова» (не спать) в работающем демоне: on | off
    Owl { state: String },
    /// Установить силу вибрации клика тачпада: low | medium | high
    SetVibration { level: String },
    /// Установить порог нажатия клика тачпада (напр. 100..250, завод: 125)
    SetPressure { threshold: u16 },
    /// Включить/выключить 2-й щелчок Heavy Press в прошивке тачпада: on | off
    SetHeavyPress { state: String },
    /// Установить силу тактильной отдачи скролла (0x58): 10..100
    SetSlideStrength { strength: u16 },
    /// Послать тестовый тактильный щелчок в мотор тачпада
    Pulse,
    /// Включить/выключить/переключить тачпад (KWin): on | off | toggle
    Touchpad { state: String },
    /// Включить/выключить/переключить сенсорный экран (KWin): on | off | toggle
    Touchscreen { state: String },
    /// Установить частоту обновления экрана (например 60 или 120)
    SetHz { hz: u32 },
    /// Запустить действие, привязанное к слоту: heavy_press | wmi_perf | wmi_hz | wmi_tp | custom_1 | custom_2
    Trigger { slot: String },
    /// Управление автозапуском демона systemd --user: on | off | status
    Autostart { state: String },
}

fn parse_on_off(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" | "enable" => Some(true),
        "off" | "0" | "false" | "no" | "disable" => Some(false),
        _ => None,
    }
}

fn print_status() {
    let mut sampler = TelemetrySampler::new();
    let cfg = AppConfig::load();
    let _ = sampler.sample(&cfg.temp_sensor_key);
    std::thread::sleep(std::time::Duration::from_millis(160));
    let tele = sampler.sample(&cfg.temp_sensor_key);
    let hw = read_hardware_info();
    let daemon_up = daemon::send_ipc_command("PING").is_ok();

    println!("============================================================");
    println!(" XiControl Linux v0.2.0 — {} {}", hw.vendor, hw.model);
    println!("============================================================");
    println!("Daemon (IPC):       {}", if daemon_up { "Running" } else { "Stopped" });
    println!("Systemd Autostart:  {}", if is_autostart_enabled() { "Enabled" } else { "Disabled" });
    println!("Power Source:       {}", if tele.ac_online { "AC Online" } else { "Battery" });
    println!("CPU:                {:.1}% @ {:.0}°C{}", tele.cpu_pct, tele.cpu_temp_c,
        tele.cpu_watts.map(|w| format!(" ({w:.1} W)")).unwrap_or_default());
    if let Some(g) = tele.gpu_pct {
        println!("GPU (Arc/xe):       {:.0}%{}", g,
            tele.gpu_freq_mhz.map(|f| format!(" @ {f} MHz")).unwrap_or_default());
    }
    if let Some(n) = tele.npu_pct {
        println!("NPU (intel_vpu):    {:.0}%{}", n,
            tele.npu_freq_mhz.map(|f| format!(" @ {f} MHz")).unwrap_or_default());
    }
    println!("RAM:                {:.1}/{:.1} GB ({:.0}%)", tele.ram_used_gb, tele.ram_total_gb, tele.ram_pct);
    println!("Battery:            {}% ({}, {:+.1} W)", tele.bat_capacity, tele.bat_status, tele.bat_watts);
    if let Some(c) = tele.bat_cycles {
        println!("Battery Cycles:     {c}");
    }
    if let (Some(f), Some(d), Some(w)) = (tele.bat_wh_full, tele.bat_wh_design, tele.bat_wear_pct) {
        println!("Battery Capacity:   {f:.1} / {d:.1} Wh ({w}% health)");
    }

    match get_perf_mode() {
        Ok(m) => println!("Performance Mode:   {} ({})", m.label(false), m.cli_name()),
        Err(e) => println!("Performance Mode:   Error ({e})"),
    }
    match get_charge_limit() {
        Ok(lim) => println!("Charge Limit (EC):  {}%", lim),
        Err(e) => println!("Charge Limit (EC):  Error ({e})"),
    }
    if let Some(w) = get_adapter_watts() {
        println!("USB-C PD Adapter:   {}", if w > 0 { format!("{w} W") } else { "Disconnected".into() });
    }
    if let Some(h) = get_battery_health() {
        println!("Battery SOH (EC):   {h}%");
    }
    if let Some(m) = get_mic_mute() {
        println!("Hardware Mic Mute:  {}", if m { "Muted" } else { "Active" });
    }
    if let Some(d) = query_internal_display() {
        println!("Display ({}):      {}x{} @ {} Hz (avail: {:?})", d.output_name, d.current_width, d.current_height, d.current_hz, d.available_hz);
    }
    println!(
        "KWin Input:         Touchpad={} | Touchscreen={}",
        is_touchpad_enabled().map(|v| if v { "On" } else { "Off" }).unwrap_or("?"),
        is_touchscreen_enabled().map(|v| if v { "On" } else { "Off" }).unwrap_or("?")
    );
    println!("------------------------------------------------------------");
    match read_touchpad_state() {
        Ok(tp) => {
            println!("Touchpad (BLTP7853):");
            println!("  MCU Firmware:     {} | Controller: {}", tp.firmware, tp.controller_firmware);
            println!("  Vibration Level:  {}", tp.vibration.map(|v| v.label(false)).unwrap_or("Custom"));
            println!("  Click Pressure:   {} (100=Light, 125=Factory, 140=Firm)", tp.pressure);
            println!("  Heavy Press 2nd:  {}", if tp.heavy_press { "On" } else { "Off" });
            if let Some(s) = tp.slide_strength {
                println!("  Slide Strength:   {s}");
            }
        }
        Err(e) => println!("Touchpad (BLTP7853): {e}"),
    }
    println!("============================================================");
}

fn exit_err(e: String) -> ! {
    eprintln!("Ошибка: {e}");
    std::process::exit(1);
}

fn main() {
    let cli = Cli::parse();
    let cfg = AppConfig::load();
    let shared = match &cli.command {
        None | Some(Commands::Gui | Commands::GuiWindow | Commands::Daemon) => SharedState::new(cfg),
        _ => SharedState::new_light(cfg),
    };

    match cli.command {
        None | Some(Commands::Gui) => {
            if daemon::send_ipc_command("PING").is_err() {
                daemon::start_background_services(&shared);
            }
            if let Err(e) = ui::run_gui_window(shared) {
                exit_err(e);
            }
        }
        Some(Commands::GuiWindow) => {
            if let Err(e) = ui::run_gui_window(shared) {
                exit_err(e);
            }
        }
        Some(Commands::Daemon) => {
            if daemon::send_ipc_command("PING").is_ok() {
                println!("XiControl daemon уже запущен.");
                return;
            }
            daemon::run_daemon_loop(shared);
        }
        Some(Commands::Status) => print_status(),
        Some(Commands::SetMode { mode }) => {
            let m = PerfMode::from_name(&mode)
                .unwrap_or_else(|| exit_err("Доступные режимы: eco, quiet, balance, auto, turbo, fullspeed".into()));
            set_perf_mode_checked(&shared, m).unwrap_or_else(|e| exit_err(e));
            println!("✓ Режим установлен: {} ({})", m.label(false), m.cli_name());
        }
        Some(Commands::SetCharge { percent }) => {
            apply_charge_limit(&shared, percent, None).unwrap_or_else(|e| exit_err(e));
            daemon::notify_daemon_reload();
            println!("✓ Лимит заряда батареи установлен на {percent}%");
        }
        Some(Commands::Care { state }) => {
            let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off".into()));
            let pct = set_charge_care(&shared, on).unwrap_or_else(|e| exit_err(e));
            daemon::notify_daemon_reload();
            println!("✓ Беречь батарею: {} ({pct}%)", if on { "вкл" } else { "выкл" });
        }
        Some(Commands::Travel { state }) => {
            let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off".into()));
            set_travel_mode(&shared, on).unwrap_or_else(|e| exit_err(e));
            daemon::notify_daemon_reload();
            println!("✓ Режим «В дорогу»: {}", if on { "вкл (100%)" } else { "выкл" });
        }
        Some(Commands::Owl { state }) => {
            let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off".into()));
            let mut c = shared.lock().unwrap().config.clone();
            c.owl_mode = on;
            let _ = c.save();
            match daemon::send_ipc_command("RELOAD_CONFIG") {
                Ok(_) => println!("✓ Режим «Сова» в демоне: {}", if on { "вкл" } else { "выкл" }),
                Err(_) => println!("Настройка owl_mode={on} сохранена. Запустите `xicontrol daemon`, чтобы удерживать systemd-inhibit."),
            }
        }
        Some(Commands::SetVibration { level }) => {
            let v = TouchpadVibration::from_name(&level)
                .unwrap_or_else(|| exit_err("Доступные уровни: low, medium, high".into()));
            set_touchpad_vibration(v).unwrap_or_else(|e| exit_err(e));
            println!("✓ Вибрация тачпада: {}", v.label(false));
        }
        Some(Commands::SetPressure { threshold }) => {
            set_touchpad_pressure(threshold).unwrap_or_else(|e| exit_err(e));
            println!("✓ Усилие нажатия тачпада: {threshold}");
        }
        Some(Commands::SetHeavyPress { state }) => {
            let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off".into()));
            set_touchpad_heavy_press(on).unwrap_or_else(|e| exit_err(e));
            println!("✓ Аппаратный Heavy Press: {}", if on { "вкл" } else { "выкл" });
        }
        Some(Commands::SetSlideStrength { strength }) => {
            set_touchpad_slide_strength(strength).unwrap_or_else(|e| exit_err(e));
            println!("✓ Сила отдачи скролла (0x58): {strength}");
        }
        Some(Commands::Pulse) => {
            pulse_touchpad().unwrap_or_else(|e| exit_err(e));
            println!("✓ Щелчок мотора тачпада выполнен");
        }
        Some(Commands::Touchpad { state }) => {
            let res = if state.eq_ignore_ascii_case("toggle") {
                toggle_touchpad(&shared)
            } else {
                let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off | toggle".into()));
                set_touchpad_enabled(on).map(|()| on)
            };
            let on = res.unwrap_or_else(|e| exit_err(e));
            daemon::notify_daemon_reload();
            println!("✓ Тачпад: {}", if on { "включён" } else { "выключен" });
        }
        Some(Commands::Touchscreen { state }) => {
            let res = if state.eq_ignore_ascii_case("toggle") {
                toggle_touchscreen(&shared)
            } else {
                let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off | toggle".into()));
                set_touchscreen_enabled(on).map(|()| on)
            };
            let on = res.unwrap_or_else(|e| exit_err(e));
            daemon::notify_daemon_reload();
            println!("✓ Сенсорный экран: {}", if on { "включён" } else { "выключен" });
        }
        Some(Commands::SetHz { hz }) => {
            set_display_hz_checked(&shared, hz).unwrap_or_else(|e| exit_err(e));
            println!("✓ Частота экрана установлена на {hz} Гц");
        }
        Some(Commands::Trigger { slot }) => {
            if let Ok(reply) = daemon::send_ipc_command(&format!("TRIGGER {slot}")) {
                if reply == "OK" {
                    println!("✓ Слот `{slot}` выполнен (через демон)");
                    return;
                }
            }
            actions::trigger_slot(&shared, &slot).unwrap_or_else(|e| exit_err(e));
            println!("✓ Слот `{slot}` выполнен");
        }
        Some(Commands::Autostart { state }) => {
            if state.eq_ignore_ascii_case("status") {
                println!("Autostart: {}", if is_autostart_enabled() { "enabled" } else { "disabled" });
                return;
            }
            let on = parse_on_off(&state).unwrap_or_else(|| exit_err("Используйте: on | off | status".into()));
            set_autostart_enabled(on).unwrap_or_else(|e| exit_err(e));
            println!("✓ Автозапуск systemd --user: {}", if on { "включён" } else { "выключен" });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{fixed_time_eq, sha256_hex};
    use crate::http_api::handle_http_request;
    use crate::hw::touchpad::{
        parse_hid_report_04, tp_checksum, tp_frame, HeavyPressDetector, TouchContact,
        TouchpadEdge, TouchpadEdgeGesture, TP_CMD_PULSE, TP_FRAME_LEN,
    };

    #[test]
    fn touchpad_pulse_and_checksum_match_live_vectors() {
        let pulse = tp_frame(TP_CMD_PULSE, &[0x32, 0xCE], false);
        assert_eq!(
            &pulse[..11],
            &[0x0D, 0x09, 0xFD, 0x01, 0x00, 0x03, 0x00, 0x00, 0x02, 0x32, 0xCE]
        );

        let mut live = [0u8; TP_FRAME_LEN];
        live[..8].copy_from_slice(&[0x0D, 0x04, 0x51, 0x00, 0x50, 0x00, 0x00, 0x00]);
        assert_eq!(tp_checksum(&live), live[2]);
        assert_eq!((live[4] as u16) | ((live[5] as u16) << 8), 80);
    }

    #[test]
    fn hid_report_04_parses_contacts() {
        let mut r = [0u8; 40];
        r[0] = 0x04;
        // Slot 0: active (tip=1), id=2, x=1641 (~0.5), y=1062 (~0.5), pressure=620
        r[1] = 0x02 | (2 << 2);
        r[2..4].copy_from_slice(&1641u16.to_le_bytes());
        r[4..6].copy_from_slice(&1062u16.to_le_bytes());
        r[6..8].copy_from_slice(&620u16.to_le_bytes());
        let contacts = parse_hid_report_04(&r).expect("valid report 04");
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].id, 2);
        assert_eq!(contacts[0].pressure, 620);
        assert!((contacts[0].x - 0.5).abs() < 0.01);
        assert!((contacts[0].y - 0.5).abs() < 0.01);
    }

    #[test]
    fn heavy_press_fires_once_until_lifted() {
        let mut det = HeavyPressDetector::new(500);
        let light = [TouchContact { id: 0, x: 0.5, y: 0.5, pressure: 200 }];
        let heavy = [TouchContact { id: 0, x: 0.5, y: 0.5, pressure: 600 }];
        assert!(!det.update(&light));
        assert!(det.update(&heavy));
        assert!(!det.update(&heavy));
        assert!(!det.update(&[]));
        assert!(det.update(&heavy));
    }

    #[test]
    fn edge_sliders_left_right_deadzone_and_multitouch() {
        let mut g = TouchpadEdgeGesture::new(12, 1.5, 10);
        // Right edge down->up yields positive steps
        let c1 = [TouchContact { id: 1, x: 0.98, y: 0.70, pressure: 100 }];
        let c2 = [TouchContact { id: 1, x: 0.98, y: 0.50, pressure: 100 }];
        assert_eq!(g.update(&c1), (TouchpadEdge::Right, 0));
        let (edge, steps) = g.update(&c2);
        assert_eq!(edge, TouchpadEdge::Right);
        assert!(steps >= 3);

        // Two fingers cancel gesture
        let two = [c2[0], TouchContact { id: 2, x: 0.95, y: 0.52, pressure: 100 }];
        assert_eq!(g.update(&two), (TouchpadEdge::None, 0));
        g.update(&[]);

        // Bottom dead zone cancels gesture
        let bottom = [TouchContact { id: 3, x: 0.02, y: 0.98, pressure: 100 }];
        assert_eq!(g.update(&bottom), (TouchpadEdge::None, 0));
    }

    #[test]
    fn sha256_and_fixed_time_eq_match_rfc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(fixed_time_eq(b"secret123", b"secret123"));
        assert!(!fixed_time_eq(b"secret123", b"secret124"));
        assert!(!fixed_time_eq(b"short", b"longer"));
    }

    #[test]
    fn http_api_auth_and_permissions() {
        let mut cfg = AppConfig::default();
        cfg.http_api.enabled = true;
        cfg.http_api.token_sha256 = sha256_hex(b"my-token");
        cfg.http_api.allow_mode = false;
        let shared = SharedState::new(cfg);

        // Missing auth -> 401
        let (st, _, _) = handle_http_request(&shared, "GET", "/status", None, "");
        assert_eq!(st, 401);

        // Wrong token -> 401
        let (st, _, _) = handle_http_request(&shared, "GET", "/status", Some("Bearer wrong"), "");
        assert_eq!(st, 401);

        // Valid token GET /status -> 200
        let (st, _, body) = handle_http_request(&shared, "GET", "/status", Some("Bearer my-token"), "");
        assert_eq!(st, 200);
        assert!(body.contains("bat_capacity"));

        // Valid token POST /mode when disallowed -> 403
        let (st, _, _) = handle_http_request(&shared, "POST", "/mode", Some("Bearer my-token"), r#"{"mode":"auto"}"#);
        assert_eq!(st, 403);
    }

    #[test]
    fn deadzone_evdev_filter_blocks_touch_in_strip_keeps_drag_from_above() {
        use crate::hw::touchpad_dz::{DeadZoneFilter, InputEvent};
        let ev = |type_: u16, code: u16, value: i32| InputEvent {
            tv_sec: 1,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        let mut f = DeadZoneFilter::new(0, 2124, 15);
        // 15/90 * 2124 = 354 -> threshold = 1770
        // 1) Touch down inside dead zone (slot 0, y=1950): completely suppressed
        for e in [
            ev(3, 47, 0),    // ABS_MT_SLOT
            ev(3, 57, 10),   // ABS_MT_TRACKING_ID
            ev(3, 53, 1500), // ABS_MT_POSITION_X
            ev(3, 54, 1950), // ABS_MT_POSITION_Y
            ev(1, 330, 1),   // BTN_TOUCH
            ev(1, 325, 1),   // BTN_TOOL_FINGER
        ] {
            assert!(f.process_event(&e).is_empty());
        }
        assert!(f.process_event(&ev(0, 0, 0)).is_empty()); // SYN_REPORT emits nothing

        // Even shifting upward slightly stays suppressed until lift
        assert!(f.process_event(&ev(3, 54, 1600)).is_empty());
        assert!(f.process_event(&ev(0, 0, 0)).is_empty());

        // 2) Second finger starts ABOVE dead zone (slot 1, y=1000) while palm still in dead zone:
        // must emit ONLY 1 finger (slot 1), NOT 2 fingers!
        for e in [
            ev(3, 47, 1),
            ev(3, 57, 11),
            ev(3, 53, 1000),
            ev(3, 54, 1000),
        ] {
            assert!(f.process_event(&e).is_empty());
        }
        let out = f.process_event(&ev(0, 0, 0));
        assert!(out.iter().any(|x| x.type_ == 1 && x.code == 330 && x.value == 1)); // BTN_TOUCH=1
        assert!(out.iter().any(|x| x.type_ == 1 && x.code == 325 && x.value == 1)); // BTN_TOOL_FINGER=1
        assert!(!out.iter().any(|x| x.type_ == 1 && x.code == 333)); // NO BTN_TOOL_DOUBLETAP

        // 3) Dragging finger 1 down into the dead zone continues working
        f.process_event(&ev(3, 54, 2000));
        let drag_out = f.process_event(&ev(0, 0, 0));
        assert!(drag_out.iter().any(|x| x.type_ == 3 && x.code == 54 && x.value == 2000));

        // 4) Physical click BTN_LEFT still forwards
        f.process_event(&ev(1, 272, 1));
        let click_out = f.process_event(&ev(0, 0, 0));
        assert!(click_out.iter().any(|x| x.type_ == 1 && x.code == 272 && x.value == 1));
    }

    #[test]
    fn config_roundtrip_preserves_fields() {
        let mut cfg = AppConfig::default();
        cfg.charge_care_percent = 70;
        cfg.owl_mode = true;
        cfg.http_api.port = 59000;
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.charge_care_percent, 70);
        assert!(parsed.owl_mode);
        assert_eq!(parsed.http_api.port, 59000);
    }

    #[test]
    fn perf_cycle_respects_battery_and_case_insensitive_hidden() {
        let hidden = vec!["fullspeed".into(), "quiet".into()];
        let unavail = vec!["Balance".into()];
        assert_eq!(
            actions::select_cycle_modes(false, &hidden, &unavail),
            vec![PerfMode::Eco, PerfMode::Auto]
        );
        assert_eq!(
            actions::select_cycle_modes(true, &hidden, &unavail),
            vec![PerfMode::Eco, PerfMode::Auto, PerfMode::Turbo]
        );
        let mut empty_bat = vec!["eco".into(), "auto".into()];
        assert_eq!(
            actions::select_cycle_modes(false, &empty_bat, &[]),
            vec![PerfMode::Eco, PerfMode::Auto]
        );
        config::remove_perf_mode(&mut empty_bat, PerfMode::Eco);
        assert!(!config::contains_perf_mode(&empty_bat, PerfMode::Eco));
        assert!(config::contains_perf_mode(&empty_bat, PerfMode::Auto));
    }
}
