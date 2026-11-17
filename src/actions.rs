use crate::config::{self, AppConfig, KeyAction};
use crate::daemon;
use crate::hw::mifs::{self, PerfMode};
use crate::hw::sys::{
    self, DisplayInfo, HardwareInfo, OwlGuard, SysTelemetry, TelemetrySampler, TempSensorInfo,
};
use crate::hw::touchpad::{self, TouchpadState};
use std::collections::VecDeque;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HISTORY_CAP: usize = 60;
const OLED_YIELD_SECS: u64 = 2 * 3600;

#[derive(Debug, Clone)]
pub struct TelemetrySample {
    pub cpu_pct: f32,
    pub cpu_temp_c: f32,
    pub gpu_pct: f32,
    pub npu_pct: f32,
    pub ram_pct: f32,
    pub bat_watts: f32,
    pub cpu_watts: f32,
}

#[derive(Debug, Default)]
pub struct TelemetryHistory {
    pub samples: VecDeque<TelemetrySample>,
}

impl TelemetryHistory {
    pub fn push(&mut self, t: &SysTelemetry) {
        if self.samples.len() >= HISTORY_CAP {
            self.samples.pop_front();
        }
        self.samples.push_back(TelemetrySample {
            cpu_pct: t.cpu_pct,
            cpu_temp_c: t.cpu_temp_c,
            gpu_pct: t.gpu_pct.unwrap_or(0.0),
            npu_pct: t.npu_pct.unwrap_or(0.0),
            ram_pct: t.ram_pct,
            bat_watts: t.bat_watts,
            cpu_watts: t.cpu_watts.unwrap_or(0.0),
        });
    }
}

pub struct SharedState {
    pub config: AppConfig,
    pub sampler: TelemetrySampler,
    pub telemetry: SysTelemetry,
    pub history: TelemetryHistory,
    pub owl: OwlGuard,
    pub owl_active: bool,
    pub display: Option<DisplayInfo>,
    pub brightness_pct: Option<u8>,
    pub touchpad_state: Option<TouchpadState>,
    pub perf_mode: Option<PerfMode>,
    pub charge_limit: Option<u8>,
    pub adapter_watts: Option<u8>,
    pub battery_health: Option<u8>,
    pub touchpad_enabled: Option<bool>,
    pub touchscreen_enabled: Option<bool>,
    pub mic_muted: Option<bool>,
    pub temp_sensors: Vec<TempSensorInfo>,
    pub hw_info: HardwareInfo,
    pub autostart_enabled: bool,
    pub prev_ac_online: Option<bool>,
    pub expected_hz: Option<u32>,
    pub last_brightness_seen: Option<u8>,
    pub user_yielded_oled_until: Option<Instant>,
    pub last_charge_guard_wall: u64,
    pub last_webhook_charge_pct: Option<u8>,
    pub last_error: Option<String>,
    pub live_tp_pressure: u16,
    pub peak_tp_pressure: u16,
    pub wmi_input_error: Option<String>,
    pub tp_evdev_error: Option<String>,
    pub gui_requested: bool,
    pub shutdown: bool,
}

pub type SharedHandle = Arc<Mutex<SharedState>>;

impl SharedState {
    pub fn new(config: AppConfig) -> SharedHandle {
        let mut sampler = TelemetrySampler::new();
        let telemetry = sampler.sample(&config.temp_sensor_key);
        let temp_sensors = sys::list_temp_sensors();
        let hw_info = sys::read_hardware_info();
        let display = sys::query_internal_display();
        let brightness_pct = sys::get_brightness_percent();
        let perf_mode = mifs::get_perf_mode().ok();
        let charge_limit = mifs::get_charge_limit().ok();
        let adapter_watts = mifs::get_adapter_watts();
        let battery_health = mifs::get_battery_health();
        let touchpad_state = touchpad::read_touchpad_state().ok();
        let touchpad_enabled = touchpad::is_touchpad_enabled();
        let touchscreen_enabled = touchpad::is_touchscreen_enabled();
        let mic_muted = mifs::get_mic_mute();
        let autostart_enabled = sys::is_autostart_enabled();

        let mut history = TelemetryHistory::default();
        history.push(&telemetry);

        let mut state = Self {
            config,
            sampler,
            telemetry,
            history,
            owl: OwlGuard::new(),
            owl_active: false,
            display,
            brightness_pct,
            touchpad_state,
            perf_mode,
            charge_limit,
            adapter_watts,
            battery_health,
            touchpad_enabled,
            touchscreen_enabled,
            mic_muted,
            temp_sensors,
            hw_info,
            autostart_enabled,
            prev_ac_online: None,
            expected_hz: None,
            last_brightness_seen: brightness_pct,
            user_yielded_oled_until: None,
            last_charge_guard_wall: unix_now_secs(),
            last_webhook_charge_pct: charge_limit,
            last_error: None,
            live_tp_pressure: 0,
            peak_tp_pressure: 0,
            wmi_input_error: None,
            tp_evdev_error: None,
            gui_requested: false,
            shutdown: false,
        };
        if state.config.owl_mode {
            let _ = state.owl.set_active(true);
            state.owl_active = state.owl.is_active();
        }
        Arc::new(Mutex::new(state))
    }

    /// Lightweight state for short-lived CLI commands (avoids spawning display/owl or sleeping on I2C).
    pub fn new_light(config: AppConfig) -> SharedHandle {
        let mut sampler = TelemetrySampler::new();
        let telemetry = sampler.sample(&config.temp_sensor_key);
        Arc::new(Mutex::new(Self {
            config,
            sampler,
            telemetry,
            history: TelemetryHistory::default(),
            owl: OwlGuard::new(),
            owl_active: false,
            display: None,
            brightness_pct: None,
            touchpad_state: None,
            perf_mode: mifs::get_perf_mode().ok(),
            charge_limit: mifs::get_charge_limit().ok(),
            adapter_watts: None,
            battery_health: None,
            touchpad_enabled: None,
            touchscreen_enabled: None,
            mic_muted: None,
            temp_sensors: Vec::new(),
            hw_info: HardwareInfo {
                vendor: String::new(),
                model: String::new(),
                sku: String::new(),
                bios_version: String::new(),
                bios_date: String::new(),
            },
            autostart_enabled: false,
            prev_ac_online: None,
            expected_hz: None,
            last_brightness_seen: None,
            user_yielded_oled_until: None,
            last_charge_guard_wall: unix_now_secs(),
            last_webhook_charge_pct: None,
            last_error: None,
            live_tp_pressure: 0,
            peak_tp_pressure: 0,
            wmi_input_error: None,
            tp_evdev_error: None,
            gui_requested: false,
            shutdown: false,
        }))
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn refresh_fast(shared: &SharedHandle) {
    let temp_key = {
        let s = shared.lock().unwrap();
        s.config.temp_sensor_key.clone()
    };

    // Perform I/O without holding the lock across slow subprocesses/sysfs calls where possible.
    let perf = mifs::get_perf_mode().ok();
    let charge = mifs::get_charge_limit().ok();
    let mic = mifs::get_mic_mute();
    let bright = sys::get_brightness_percent();

    let mut s = shared.lock().unwrap();
    let tele = s.sampler.sample(&temp_key);
    s.history.push(&tele);
    s.telemetry = tele;
    s.perf_mode = perf;
    s.charge_limit = charge;
    if mic.is_some() {
        s.mic_muted = mic;
    }
    if bright.is_some() {
        s.brightness_pct = bright;
    }
    s.owl_active = s.owl.is_active();
}

pub fn refresh_slow(shared: &SharedHandle) {
    let display = sys::query_internal_display();
    let tp_state = touchpad::read_touchpad_state().ok();
    let tp_en = touchpad::is_touchpad_enabled();
    let ts_en = touchpad::is_touchscreen_enabled();
    let sensors = sys::list_temp_sensors();
    let adapter = mifs::get_adapter_watts();
    let health = mifs::get_battery_health();
    let autostart = sys::is_autostart_enabled();

    let mut s = shared.lock().unwrap();
    s.display = display;
    s.touchpad_state = tp_state;
    if tp_en.is_some() {
        s.touchpad_enabled = tp_en;
    }
    if ts_en.is_some() {
        s.touchscreen_enabled = ts_en;
    }
    s.temp_sensors = sensors;
    s.adapter_watts = adapter;
    s.battery_health = health;
    s.autostart_enabled = autostart;
}

pub fn select_cycle_modes(ac_online: bool, hidden: &[String], unavail: &[String]) -> Vec<PerfMode> {
    let mut candidates: Vec<PerfMode> = PerfMode::ALL
        .iter()
        .copied()
        .filter(|&m| {
            let allowed_on_power = ac_online || m.is_battery_capable();
            allowed_on_power
                && !config::contains_perf_mode(hidden, m)
                && !config::contains_perf_mode(unavail, m)
        })
        .collect();
    if candidates.is_empty() {
        candidates = if ac_online {
            PerfMode::ALL.to_vec()
        } else {
            vec![PerfMode::Eco, PerfMode::Auto]
        };
    }
    candidates
}

pub fn set_perf_mode_checked(shared: &SharedHandle, mode: PerfMode) -> Result<(), String> {
    let ac_online = sys::read_ac_online();
    let en = shared.lock().unwrap().config.language.is_en();
    if !ac_online && !mode.is_battery_capable() {
        let msg = if en {
            format!("On battery, EC firmware supports only Eco and Auto ({} requires AC power)", mode.label(true))
        } else {
            format!("От батареи прошивка EC поддерживает только Eco и Авто (режим «{}» доступен от сети)", mode.label(false))
        };
        shared.lock().unwrap().last_error = Some(msg.clone());
        return Err(msg);
    }

    let res = mifs::set_perf_mode(mode);
    let osd_enabled = {
        let mut s = shared.lock().unwrap();
        let name = mode.api_name().to_string();
        match &res {
            Ok(()) => {
                config::remove_perf_mode(&mut s.config.unavailable_modes, mode);
                s.config.last_mode = name;
                s.perf_mode = Some(mode);
                s.last_error = None;
                let _ = s.config.save();
            }
            Err(e) => {
                if ac_online && !config::contains_perf_mode(&s.config.unavailable_modes, mode) {
                    s.config.unavailable_modes.push(name);
                    let _ = s.config.save();
                }
                s.last_error = Some(e.clone());
            }
        }
        s.config.osd_enabled
    };
    if res.is_ok() {
        let prefix = if en { "Power" } else { "Режим" };
        sys::show_osd(osd_enabled, "battery-profile-performance-symbolic", &format!("{prefix}: {}", mode.label(en)));
        daemon::notify_daemon_reload();
    }
    res
}

pub fn cycle_perf_mode(shared: &SharedHandle) -> Result<PerfMode, String> {
    let (ac, hidden_ac, hidden_bat, unavail, current) = {
        let s = shared.lock().unwrap();
        (
            sys::read_ac_online(),
            s.config.hidden_modes_ac.clone(),
            s.config.hidden_modes_bat.clone(),
            s.config.unavailable_modes.clone(),
            s.perf_mode,
        )
    };
    let hidden = if ac { &hidden_ac } else { &hidden_bat };
    let candidates = select_cycle_modes(ac, hidden, &unavail);
    let next = match current.and_then(|c| candidates.iter().position(|&m| m == c)) {
        Some(idx) => candidates[(idx + 1) % candidates.len()],
        None => candidates[0],
    };
    set_perf_mode_checked(shared, next)?;
    Ok(next)
}

pub fn apply_charge_limit(shared: &SharedHandle, pct: u8, update_travel: Option<bool>) -> Result<(), String> {
    mifs::set_charge_limit(pct)?;
    let (en, osd_enabled, webhook_enabled, webhook_url, notify_pct) = {
        let mut s = shared.lock().unwrap();
        s.charge_limit = Some(pct);
        if let Some(tr) = update_travel {
            s.config.travel_mode = tr;
        } else if pct < 100 {
            s.config.travel_mode = false;
            s.config.charge_care_enabled = true;
            s.config.charge_care_percent = pct;
        } else {
            s.config.charge_care_enabled = false;
        }
        s.last_error = None;
        let _ = s.config.save();
        let changed = s.last_webhook_charge_pct != Some(pct);
        if changed {
            s.last_webhook_charge_pct = Some(pct);
        }
        (
            s.config.language.is_en(),
            s.config.osd_enabled,
            s.config.webhook_enabled && changed,
            s.config.webhook_url.clone(),
            pct,
        )
    };
    let label = if en { format!("Charge limit: {pct}%") } else { format!("Порог заряда: {pct}%") };
    sys::show_osd(osd_enabled, "battery-good-symbolic", &label);
    if webhook_enabled && !webhook_url.trim().is_empty() {
        send_webhook_async(webhook_url, notify_pct);
    }
    Ok(())
}

pub fn set_charge_care(shared: &SharedHandle, enabled: bool) -> Result<u8, String> {
    let pct = {
        let mut s = shared.lock().unwrap();
        s.config.charge_care_enabled = enabled;
        if enabled {
            s.config.travel_mode = false;
            if s.config.charge_care_percent >= 100 || s.config.charge_care_percent < 40 {
                s.config.charge_care_percent = 80;
            }
            s.config.charge_care_percent
        } else {
            100
        }
    };
    apply_charge_limit(shared, pct, Some(false))?;
    Ok(pct)
}

pub fn set_travel_mode(shared: &SharedHandle, enabled: bool) -> Result<(), String> {
    let target = {
        let s = shared.lock().unwrap();
        if enabled {
            100
        } else if s.config.charge_care_enabled {
            s.config.charge_care_percent.clamp(40, 80)
        } else {
            100
        }
    };
    apply_charge_limit(shared, target, Some(enabled))
}

pub fn set_owl_mode(shared: &SharedHandle, enabled: bool) -> Result<(), String> {
    let (en, osd_enabled) = {
        let mut s = shared.lock().unwrap();
        s.owl.set_active(enabled)?;
        s.owl_active = s.owl.is_active();
        s.config.owl_mode = enabled;
        let _ = s.config.save();
        (s.config.language.is_en(), s.config.osd_enabled)
    };
    let text = match (enabled, en) {
        (true, false) => "Сова: сон заблокирован",
        (false, false) => "Сова: выключена",
        (true, true) => "Owl mode: keep awake ON",
        (false, true) => "Owl mode: OFF",
    };
    sys::show_osd(osd_enabled, "preferences-system-power-symbolic", text);
    Ok(())
}

pub fn set_display_hz_checked(shared: &SharedHandle, hz: u32) -> Result<(), String> {
    sys::set_display_hz(hz)?;
    let disp = sys::query_internal_display();
    {
        let mut s = shared.lock().unwrap();
        s.expected_hz = Some(hz);
        if disp.is_some() {
            s.display = disp;
        }
    }
    daemon::notify_expected_hz(hz);
    Ok(())
}

pub fn cycle_refresh_rate(shared: &SharedHandle) -> Result<u32, String> {
    let display = sys::query_internal_display().ok_or_else(|| "Экран не найден".to_string())?;
    let (cfg_list, en, osd_enabled) = {
        let s = shared.lock().unwrap();
        (s.config.hz_cycle_list.clone(), s.config.language.is_en(), s.config.osd_enabled)
    };
    let mut rates: Vec<u32> = cfg_list
        .into_iter()
        .filter(|hz| display.available_hz.contains(hz))
        .collect();
    if rates.len() < 2 {
        rates = display.available_hz.clone();
    }
    if rates.is_empty() {
        return Err("Доступные частоты не найдены".into());
    }
    let next = match rates.iter().position(|&h| h == display.current_hz) {
        Some(idx) => rates[(idx + 1) % rates.len()],
        None => rates[0],
    };
    set_display_hz_checked(shared, next)?;
    let msg = if en { format!("Refresh rate: {next} Hz") } else { format!("Частота экрана: {next} Гц") };
    sys::show_osd(osd_enabled, "video-display-symbolic", &msg);
    Ok(next)
}

pub fn toggle_touchpad(shared: &SharedHandle) -> Result<bool, String> {
    let cur = touchpad::is_touchpad_enabled().unwrap_or(true);
    let next = !cur;
    touchpad::set_touchpad_enabled(next)?;
    let (en, osd_enabled) = {
        let mut s = shared.lock().unwrap();
        s.touchpad_enabled = Some(next);
        if s.config.touchpad_remember_disabled {
            s.config.touchpad_disabled = !next;
            let _ = s.config.save();
        }
        (s.config.language.is_en(), s.config.osd_enabled)
    };
    let text = match (next, en) {
        (true, false) => "Тачпад включён",
        (false, false) => "Тачпад выключен",
        (true, true) => "Touchpad enabled",
        (false, true) => "Touchpad disabled",
    };
    sys::show_osd(osd_enabled, "input-touchpad-symbolic", text);
    Ok(next)
}

pub fn toggle_touchscreen(shared: &SharedHandle) -> Result<bool, String> {
    let cur = touchpad::is_touchscreen_enabled().unwrap_or(true);
    let next = !cur;
    touchpad::set_touchscreen_enabled(next)?;
    let (en, osd_enabled) = {
        let mut s = shared.lock().unwrap();
        s.touchscreen_enabled = Some(next);
        s.config.touchscreen_disabled = !next;
        let _ = s.config.save();
        (s.config.language.is_en(), s.config.osd_enabled)
    };
    let text = match (next, en) {
        (true, false) => "Сенсорный экран включён",
        (false, false) => "Сенсорный экран выключен",
        (true, true) => "Touchscreen enabled",
        (false, true) => "Touchscreen disabled",
    };
    sys::show_osd(osd_enabled, "input-tablet-symbolic", text);
    Ok(next)
}

pub fn execute_action(shared: &SharedHandle, action: KeyAction) -> Result<(), String> {
    match action {
        KeyAction::None => Ok(()),
        KeyAction::CyclePerfMode => cycle_perf_mode(shared).map(|_| ()),
        KeyAction::SetEco => set_perf_mode_checked(shared, PerfMode::Eco),
        KeyAction::SetQuiet => set_perf_mode_checked(shared, PerfMode::Quiet),
        KeyAction::SetBalance => set_perf_mode_checked(shared, PerfMode::Balance),
        KeyAction::SetAuto => set_perf_mode_checked(shared, PerfMode::Auto),
        KeyAction::SetTurbo => set_perf_mode_checked(shared, PerfMode::Turbo),
        KeyAction::SetFullSpeed => set_perf_mode_checked(shared, PerfMode::FullSpeed),
        KeyAction::ToggleChargeCare => {
            let cur = shared.lock().unwrap().config.charge_care_enabled;
            set_charge_care(shared, !cur).map(|_| ())
        }
        KeyAction::ToggleTravel => {
            let cur = shared.lock().unwrap().config.travel_mode;
            set_travel_mode(shared, !cur)
        }
        KeyAction::ToggleOwl => {
            let cur = shared.lock().unwrap().config.owl_mode;
            set_owl_mode(shared, !cur)
        }
        KeyAction::CycleRefreshRate => cycle_refresh_rate(shared).map(|_| ()),
        KeyAction::ToggleTouchpad => toggle_touchpad(shared).map(|_| ()),
        KeyAction::ToggleTouchscreen => toggle_touchscreen(shared).map(|_| ()),
        KeyAction::MediaPlayPause => {
            sys::mpris_command("PlayPause");
            Ok(())
        }
        KeyAction::MediaNext => {
            sys::mpris_command("Next");
            Ok(())
        }
        KeyAction::MediaPrev => {
            sys::mpris_command("Previous");
            Ok(())
        }
        KeyAction::MicMuteToggle => {
            sys::toggle_mic_mute();
            let mut s = shared.lock().unwrap();
            s.mic_muted = mifs::get_mic_mute();
            Ok(())
        }
        KeyAction::Screenshot => {
            sys::take_screenshot();
            Ok(())
        }
        KeyAction::Calculator => {
            sys::open_calculator();
            Ok(())
        }
        KeyAction::LaunchCustom => {
            let cmd = shared.lock().unwrap().config.custom_command.clone();
            sys::launch_custom_command(&cmd);
            Ok(())
        }
    }
}

pub fn trigger_slot(shared: &SharedHandle, slot: &str) -> Result<(), String> {
    let action = {
        let s = shared.lock().unwrap();
        match slot.to_ascii_lowercase().as_str() {
            "heavy" | "heavy_press" => s.config.action_heavy_press,
            "perf" | "wmi_perf" | "fn_f" => s.config.action_wmi_perf,
            "hz" | "wmi_hz" | "fn_s" => s.config.action_wmi_hz,
            "tp" | "wmi_tp" | "touchpad" => s.config.action_wmi_tp,
            "custom1" | "custom_1" | "ai" => s.config.action_custom_1,
            "custom2" | "custom_2" | "mi" => s.config.action_custom_2,
            _ => return Err(format!("Неизвестный слот: {slot}")),
        }
    };
    execute_action(shared, action)
}

/// Applies boot settings and AC/BAT power automation (profiles, auto-Hz, Hold-Hz, remember brightness, OLED cap).
pub fn sync_power_state(shared: &SharedHandle, on_boot: bool) {
    let ac_now = sys::read_ac_online();
    let (need_disp, need_bright, prev_bright_cached) = {
        let s = shared.lock().unwrap();
        let switched = s.prev_ac_online.map(|p| p != ac_now).unwrap_or(false);
        (
            on_boot || switched || s.config.hold_hz_enabled,
            on_boot || switched || s.config.remember_brightness_per_power || s.config.oled_brightness_limit_enabled,
            s.brightness_pct,
        )
    };
    let cur_bright = if need_bright {
        if on_boot {
            sys::get_brightness_percent()
        } else {
            prev_bright_cached.or_else(sys::get_brightness_percent)
        }
    } else {
        prev_bright_cached
    };
    let cur_display = if need_disp {
        sys::query_internal_display()
    } else {
        None
    };

    let (
        switched,
        apply_mode,
        apply_hz,
        apply_bright,
        cap_bright,
        restore_tp,
        restore_ts,
        restore_charge,
    ) = {
        let mut s = shared.lock().unwrap();
        s.telemetry.ac_online = ac_now;
        if cur_bright.is_some() {
            s.brightness_pct = cur_bright;
        }
        if cur_display.is_some() {
            s.display = cur_display.clone();
        }

        let switched = match s.prev_ac_online {
            Some(prev) => prev != ac_now,
            None => false,
        };
        s.prev_ac_online = Some(ac_now);

        // Remember previous brightness before switching AC/BAT
        if switched && s.config.remember_brightness_per_power {
            if let Some(prev_b) = s.last_brightness_seen {
                if ac_now {
                    s.config.saved_brightness_bat = Some(prev_b);
                } else {
                    s.config.saved_brightness_ac = Some(prev_b);
                }
                let _ = s.config.save();
            }
        }

        // Detect manual brightness raise above OLED cap -> 2h yield
        let limit = if ac_now {
            s.config.ac_brightness_limit
        } else {
            s.config.bat_brightness_limit
        }
        .clamp(10, 100);

        if s.config.oled_brightness_limit_enabled {
            if let (Some(prev_b), Some(now_b)) = (s.last_brightness_seen, cur_bright) {
                if now_b > limit && now_b > prev_b.saturating_add(1) {
                    s.user_yielded_oled_until = Some(Instant::now() + Duration::from_secs(OLED_YIELD_SECS));
                }
            }
        }
        if cur_bright.is_some() {
            s.last_brightness_seen = cur_bright;
        }

        // Determine mode to apply (never try AC-only mode while on battery)
        let raw_mode = if on_boot && s.config.restore_mode_on_boot {
            if s.config.power_profiles_enabled {
                let name = if ac_now { &s.config.ac_mode } else { &s.config.bat_mode };
                PerfMode::from_name(name)
            } else {
                PerfMode::from_name(&s.config.last_mode)
            }
        } else if switched && s.config.power_profiles_enabled {
            let name = if ac_now { &s.config.ac_mode } else { &s.config.bat_mode };
            PerfMode::from_name(name)
        } else {
            None
        };
        let apply_mode = raw_mode.map(|m| {
            if !ac_now && !m.is_battery_capable() {
                PerfMode::Auto
            } else {
                m
            }
        });

        // Determine Hz to apply (auto-Hz on boot/switch, or Hold-Hz if KScreen reset it externally)
        let target_hz = if ac_now { s.config.ac_hz } else { s.config.bat_hz };
        let apply_hz = if s.config.auto_hz_enabled && (on_boot || switched) {
            s.expected_hz = Some(target_hz);
            Some(target_hz)
        } else if s.config.hold_hz_enabled {
            if let (Some(exp), Some(disp)) = (s.expected_hz, &cur_display) {
                if disp.current_hz != exp && disp.available_hz.contains(&exp) {
                    Some(exp)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Determine brightness to restore or OLED cap to apply
        let apply_bright = if switched && s.config.remember_brightness_per_power {
            if ac_now { s.config.saved_brightness_ac } else { s.config.saved_brightness_bat }
        } else {
            None
        };

        let yielded = s
            .user_yielded_oled_until
            .map(|until| Instant::now() < until)
            .unwrap_or(false);
        let cap_bright = if s.config.oled_brightness_limit_enabled && !yielded && !switched {
            cur_bright.filter(|&b| b > limit).map(|_| limit)
        } else {
            None
        };

        let restore_tp = if on_boot && s.config.touchpad_remember_disabled && s.config.touchpad_disabled {
            Some(false)
        } else {
            None
        };
        let restore_ts = if on_boot && s.config.touchscreen_disabled {
            Some(false)
        } else {
            None
        };
        let restore_charge = if on_boot && !s.config.travel_mode && s.config.charge_care_enabled {
            Some(s.config.charge_care_percent.clamp(40, 80))
        } else if on_boot && s.config.travel_mode {
            Some(100)
        } else {
            None
        };

        (
            switched,
            apply_mode,
            apply_hz,
            apply_bright,
            cap_bright,
            restore_tp,
            restore_ts,
            restore_charge,
        )
    };

    if let Some(mode) = apply_mode {
        let _ = set_perf_mode_checked(shared, mode);
    }
    if let Some(pct) = restore_charge {
        let _ = mifs::set_charge_limit(pct);
    }
    if let Some(hz) = apply_hz {
        let _ = sys::set_display_hz(hz);
    }
    if let Some(b) = apply_bright {
        let _ = sys::set_brightness_percent(b, true);
        let mut s = shared.lock().unwrap();
        s.last_brightness_seen = Some(b);
        s.brightness_pct = Some(b);
    } else if let Some(limit) = cap_bright {
        let _ = sys::set_brightness_percent(limit, true);
        let mut s = shared.lock().unwrap();
        s.last_brightness_seen = Some(limit);
        s.brightness_pct = Some(limit);
    }
    if let Some(en) = restore_tp {
        let _ = touchpad::set_touchpad_enabled(en);
    }
    if let Some(en) = restore_ts {
        let _ = touchpad::set_touchscreen_enabled(en);
    }
    let _ = switched;
}

/// ChargeGuard: re-applies configured charge limit periodically and after sleep/resume jumps.
pub fn check_charge_guard(shared: &SharedHandle) {
    let now_wall = unix_now_secs();
    let (guard_on, expected_pct, jumped) = {
        let mut s = shared.lock().unwrap();
        let jumped = now_wall.saturating_sub(s.last_charge_guard_wall) > 45;
        s.last_charge_guard_wall = now_wall;
        let expected = if s.config.travel_mode {
            Some(100)
        } else if s.config.charge_care_enabled {
            Some(s.config.charge_care_percent.clamp(40, 80))
        } else {
            None
        };
        (s.config.charge_guard_enabled || jumped, expected, jumped)
    };

    if !guard_on {
        return;
    }
    if let Some(target) = expected_pct {
        if let Ok(actual) = mifs::get_charge_limit() {
            if actual != target || jumped {
                let _ = mifs::set_charge_limit(target);
                let mut s = shared.lock().unwrap();
                s.charge_limit = Some(target);
            }
        }
    }
}

fn send_webhook_async(url: String, charge_limit: u8) {
    std::thread::spawn(move || {
        let payload = format!(
            "{{\"event\":\"charge_limit_changed\",\"charge_limit\":{charge_limit},\"timestamp\":{}}}",
            unix_now_secs()
        );
        let _ = Command::new("curl")
            .args([
                "-fsS",
                "-m",
                "5",
                "-H",
                "Content-Type: application/json",
                "-X",
                "POST",
                "-d",
                &payload,
                &url,
            ])
            .status();
    });
}
