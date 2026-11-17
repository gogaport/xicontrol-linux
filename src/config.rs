use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Ru,
    En,
}

impl Default for Language {
    fn default() -> Self {
        Language::Ru
    }
}

impl Language {
    pub fn is_en(self) -> bool {
        self == Language::En
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrayMetricKind {
    Watts,
    Cpu,
    Gpu,
    Npu,
    Ram,
    Temp,
}

impl TrayMetricKind {
    pub const ALL: [TrayMetricKind; 6] = [
        TrayMetricKind::Watts,
        TrayMetricKind::Cpu,
        TrayMetricKind::Gpu,
        TrayMetricKind::Npu,
        TrayMetricKind::Ram,
        TrayMetricKind::Temp,
    ];

    pub fn label(&self, en: bool) -> &'static str {
        match (self, en) {
            (TrayMetricKind::Watts, false) => "Мощность (Вт)",
            (TrayMetricKind::Watts, true) => "Power (W)",
            (TrayMetricKind::Cpu, false) => "Загрузка CPU (%)",
            (TrayMetricKind::Cpu, true) => "CPU Load (%)",
            (TrayMetricKind::Gpu, false) => "Загрузка iGPU (%)",
            (TrayMetricKind::Gpu, true) => "iGPU Load (%)",
            (TrayMetricKind::Npu, false) => "Загрузка NPU (%)",
            (TrayMetricKind::Npu, true) => "NPU Load (%)",
            (TrayMetricKind::Ram, false) => "Память RAM (%)",
            (TrayMetricKind::Ram, true) => "RAM Usage (%)",
            (TrayMetricKind::Temp, false) => "Температура (°C)",
            (TrayMetricKind::Temp, true) => "Temperature (°C)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyAction {
    None,
    CyclePerfMode,
    SetEco,
    SetQuiet,
    SetBalance,
    SetAuto,
    SetTurbo,
    SetFullSpeed,
    ToggleChargeCare,
    ToggleTravel,
    ToggleOwl,
    CycleRefreshRate,
    ToggleTouchpad,
    ToggleTouchscreen,
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    MicMuteToggle,
    Screenshot,
    Calculator,
    LaunchCustom,
}

impl Default for KeyAction {
    fn default() -> Self {
        KeyAction::None
    }
}

impl KeyAction {
    pub const ALL: [KeyAction; 21] = [
        KeyAction::None,
        KeyAction::CyclePerfMode,
        KeyAction::SetEco,
        KeyAction::SetQuiet,
        KeyAction::SetBalance,
        KeyAction::SetAuto,
        KeyAction::SetTurbo,
        KeyAction::SetFullSpeed,
        KeyAction::ToggleChargeCare,
        KeyAction::ToggleTravel,
        KeyAction::ToggleOwl,
        KeyAction::CycleRefreshRate,
        KeyAction::ToggleTouchpad,
        KeyAction::ToggleTouchscreen,
        KeyAction::MediaPlayPause,
        KeyAction::MediaNext,
        KeyAction::MediaPrev,
        KeyAction::MicMuteToggle,
        KeyAction::Screenshot,
        KeyAction::Calculator,
        KeyAction::LaunchCustom,
    ];

    pub fn label(&self, en: bool) -> &'static str {
        match (self, en) {
            (KeyAction::None, false) => "Ничего",
            (KeyAction::None, true) => "None",
            (KeyAction::CyclePerfMode, false) => "Следующий режим питания",
            (KeyAction::CyclePerfMode, true) => "Cycle performance mode",
            (KeyAction::SetEco, _) => "Режим: Eco",
            (KeyAction::SetQuiet, false) => "Режим: Тихий",
            (KeyAction::SetQuiet, true) => "Mode: Quiet",
            (KeyAction::SetBalance, false) => "Режим: Баланс",
            (KeyAction::SetBalance, true) => "Mode: Balance",
            (KeyAction::SetAuto, false) => "Режим: Авто",
            (KeyAction::SetAuto, true) => "Mode: Auto",
            (KeyAction::SetTurbo, false) => "Режим: Турбо",
            (KeyAction::SetTurbo, true) => "Mode: Turbo",
            (KeyAction::SetFullSpeed, false) => "Режим: Полная мощность",
            (KeyAction::SetFullSpeed, true) => "Mode: FullSpeed",
            (KeyAction::ToggleChargeCare, false) => "Беречь батарею вкл/выкл",
            (KeyAction::ToggleChargeCare, true) => "Toggle battery care",
            (KeyAction::ToggleTravel, false) => "Режим «В дорогу» (100%)",
            (KeyAction::ToggleTravel, true) => "Toggle Travel mode (100%)",
            (KeyAction::ToggleOwl, false) => "Режим «Сова» (не спать)",
            (KeyAction::ToggleOwl, true) => "Toggle Owl mode (keep awake)",
            (KeyAction::CycleRefreshRate, false) => "Переключить герцовку экрана",
            (KeyAction::CycleRefreshRate, true) => "Cycle refresh rate",
            (KeyAction::ToggleTouchpad, false) => "Тачпад вкл/выкл",
            (KeyAction::ToggleTouchpad, true) => "Toggle touchpad",
            (KeyAction::ToggleTouchscreen, false) => "Сенсорный экран вкл/выкл",
            (KeyAction::ToggleTouchscreen, true) => "Toggle touchscreen",
            (KeyAction::MediaPlayPause, false) => "Медиа: Пауза / Пуск",
            (KeyAction::MediaPlayPause, true) => "Media: Play / Pause",
            (KeyAction::MediaNext, false) => "Медиа: Следующий трек",
            (KeyAction::MediaNext, true) => "Media: Next track",
            (KeyAction::MediaPrev, false) => "Медиа: Предыдущий трек",
            (KeyAction::MediaPrev, true) => "Media: Previous track",
            (KeyAction::MicMuteToggle, false) => "Микрофон вкл/выкл",
            (KeyAction::MicMuteToggle, true) => "Toggle microphone mute",
            (KeyAction::Screenshot, false) => "Скриншот (Spectacle)",
            (KeyAction::Screenshot, true) => "Screenshot (Spectacle)",
            (KeyAction::Calculator, false) => "Калькулятор",
            (KeyAction::Calculator, true) => "Calculator",
            (KeyAction::LaunchCustom, false) => "Запустить свою команду",
            (KeyAction::LaunchCustom, true) => "Launch custom command",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpApiConfig {
    pub enabled: bool,
    pub port: u16,
    pub lan_access: bool,
    pub token_sha256: String,
    pub allow_mode: bool,
    pub allow_care: bool,
    pub allow_travel: bool,
    pub allow_owl: bool,
}

impl Default for HttpApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 58125,
            lan_access: false,
            token_sha256: String::new(),
            allow_mode: false,
            allow_care: false,
            allow_travel: false,
            allow_owl: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub language: Language,
    pub last_mode: String,
    pub restore_mode_on_boot: bool,
    pub hidden_modes_ac: Vec<String>,
    pub hidden_modes_bat: Vec<String>,
    pub unavailable_modes: Vec<String>,

    pub charge_care_enabled: bool,
    pub charge_care_percent: u8,
    pub travel_mode: bool,
    pub charge_guard_enabled: bool,
    pub webhook_enabled: bool,
    pub webhook_url: String,

    pub power_profiles_enabled: bool,
    pub ac_mode: String,
    pub bat_mode: String,

    pub auto_hz_enabled: bool,
    pub ac_hz: u32,
    pub bat_hz: u32,
    pub hold_hz_enabled: bool,
    pub hz_cycle_list: Vec<u32>,

    pub oled_brightness_limit_enabled: bool,
    pub ac_brightness_limit: u8,
    pub bat_brightness_limit: u8,
    pub remember_brightness_per_power: bool,
    pub saved_brightness_ac: Option<u8>,
    pub saved_brightness_bat: Option<u8>,

    pub owl_mode: bool,

    pub touchpad_remember_disabled: bool,
    pub touchpad_disabled: bool,
    pub touchscreen_disabled: bool,
    pub touchpad_edge_sliders: bool,
    pub touchpad_edge_width_mm: u8,
    pub touchpad_edge_swipes_per_range: f32,
    pub touchpad_edge_pulse: bool,
    pub touchpad_edge_pulse_strength: u16,
    pub touchpad_edge_pulse_ms: u64,
    pub touchpad_dead_zone_enabled: bool,
    pub touchpad_dead_zone_mm: u8,
    pub touchpad_heavy_press_threshold: u16,

    pub action_heavy_press: KeyAction,
    pub action_wmi_perf: KeyAction,
    pub action_wmi_hz: KeyAction,
    pub action_wmi_tp: KeyAction,
    pub action_custom_1: KeyAction,
    pub action_custom_2: KeyAction,
    pub custom_command: String,

    pub osd_enabled: bool,
    pub temp_sensor_key: String,

    pub tray_enabled: bool,
    pub tray_metric_enabled: bool,
    pub tray_metric_kind: TrayMetricKind,
    pub tray_metric_interval_sec: u64,

    pub http_api: HttpApiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language: Language::Ru,
            last_mode: "Auto".into(),
            restore_mode_on_boot: true,
            hidden_modes_ac: vec![],
            hidden_modes_bat: vec!["FullSpeed".into()],
            unavailable_modes: vec![],

            charge_care_enabled: false,
            charge_care_percent: 80,
            travel_mode: false,
            charge_guard_enabled: true,
            webhook_enabled: false,
            webhook_url: String::new(),

            power_profiles_enabled: false,
            ac_mode: "Auto".into(),
            bat_mode: "Auto".into(),

            auto_hz_enabled: false,
            ac_hz: 120,
            bat_hz: 60,
            hold_hz_enabled: false,
            hz_cycle_list: vec![60, 120],

            oled_brightness_limit_enabled: false,
            ac_brightness_limit: 90,
            bat_brightness_limit: 70,
            remember_brightness_per_power: false,
            saved_brightness_ac: None,
            saved_brightness_bat: None,

            owl_mode: false,

            touchpad_remember_disabled: false,
            touchpad_disabled: false,
            touchscreen_disabled: false,
            touchpad_edge_sliders: false,
            touchpad_edge_width_mm: 12,
            touchpad_edge_swipes_per_range: 1.5,
            touchpad_edge_pulse: true,
            touchpad_edge_pulse_strength: 80,
            touchpad_edge_pulse_ms: 80,
            touchpad_dead_zone_enabled: false,
            touchpad_dead_zone_mm: 12,
            touchpad_heavy_press_threshold: 500,

            action_heavy_press: KeyAction::None,
            action_wmi_perf: KeyAction::CyclePerfMode,
            action_wmi_hz: KeyAction::CycleRefreshRate,
            action_wmi_tp: KeyAction::ToggleTouchpad,
            action_custom_1: KeyAction::ToggleChargeCare,
            action_custom_2: KeyAction::ToggleOwl,
            custom_command: String::new(),

            osd_enabled: true,
            temp_sensor_key: "auto_max".into(),

            tray_enabled: true,
            tray_metric_enabled: false,
            tray_metric_kind: TrayMetricKind::Watts,
            tray_metric_interval_sec: 2,

            http_api: HttpApiConfig::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists()
                || dir.join(".portable").exists()
                || dir.join("config.json").exists()
            {
                return dir.join("config.json");
            }
        }
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("xicontrol").join("config.json")
}

pub fn is_portable_mode() -> bool {
    config_path()
        .parent()
        .map(|d| d.join("portable.txt").exists() || d.join(".portable").exists())
        .unwrap_or(false)
}

impl AppConfig {
    pub fn load() -> Self {
        let p = config_path();
        if let Ok(raw) = fs::read_to_string(&p) {
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(&raw) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = config_path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("json: {e}"))?;
        let tmp = p.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &p).map_err(|e| format!("rename {}: {e}", p.display()))?;
        Ok(())
    }
}

pub fn charge_level_hint(pct: u8, en: bool) -> &'static str {
    match (pct, en) {
        (40..=50, false) => "Режим «вместо десктопа» (почти всегда от сети — минимальный износ)",
        (40..=50, true) => "Desktop-replacement mode (always plugged in — minimum wear)",
        (60..=70, false) => "Компромисс: хорошая защита АКБ и ощутимый запас автономности",
        (60..=70, true) => "Compromise: strong battery protection with usable unplugged runtime",
        (80, false) => "Баланс: стандартный бережный режим на каждый день",
        (80, true) => "Balance: recommended daily battery care limit",
        (_, false) => "Лимит отключён: батарея заряжается до 100%",
        (_, true) => "Limit disabled: battery charges to 100%",
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(&mut out, "{word:08x}");
    }
    out
}

pub fn fixed_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn contains_perf_mode(list: &[String], mode: crate::hw::mifs::PerfMode) -> bool {
    list.iter()
        .any(|s| crate::hw::mifs::PerfMode::from_name(s) == Some(mode))
}

pub fn remove_perf_mode(list: &mut Vec<String>, mode: crate::hw::mifs::PerfMode) {
    list.retain(|s| crate::hw::mifs::PerfMode::from_name(s) != Some(mode));
}

pub fn generate_api_token() -> Result<String, String> {
    let mut buf = [0u8; 24];
    let mut f = fs::File::open("/dev/urandom").map_err(|e| format!("urandom: {e}"))?;
    f.read_exact(&mut buf).map_err(|e| format!("urandom read: {e}"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(48);
    for b in buf {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    Ok(s)
}
