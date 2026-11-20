use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct TempSensorInfo {
    pub key: String,
    pub label: String,
    pub temp_c: f32,
}

#[derive(Debug, Clone)]
pub struct SysTelemetry {
    pub bat_capacity: u8,
    pub bat_status: String,
    pub ac_online: bool,
    pub bat_watts: f32, // signed: + charging, - discharging
    pub bat_cycles: Option<u32>,
    pub bat_wh_full: Option<f32>,
    pub bat_wh_design: Option<f32>,
    pub bat_wear_pct: Option<u8>,

    pub cpu_pct: f32,
    pub cpu_temp_c: f32,
    pub cpu_watts: Option<f32>,
    pub gpu_pct: Option<f32>,
    pub gpu_freq_mhz: Option<u32>,
    pub npu_pct: Option<f32>,
    pub npu_freq_mhz: Option<u32>,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    pub ram_pct: f32,
    pub als_lux: Option<f32>,
}

pub struct TelemetrySampler {
    prev_cpu: Option<(u64, u64)>, // (total, idle)
    prev_rapl: Option<(u64, Instant)>,
    prev_gpu_idle: Option<(u64, Instant)>,
    prev_npu_busy: Option<(u64, Instant)>,
}

impl TelemetrySampler {
    pub fn new() -> Self {
        Self {
            prev_cpu: None,
            prev_rapl: None,
            prev_gpu_idle: None,
            prev_npu_busy: None,
        }
    }

    pub fn sample(&mut self, temp_sensor_key: &str) -> SysTelemetry {
        let cap = read_u32("/sys/class/power_supply/BAT0/capacity").unwrap_or(0) as u8;
        let bat_status = fs::read_to_string("/sys/class/power_supply/BAT0/status")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "Unknown".into());
        let ac_online = read_ac_online();
        let power_uw = read_f32("/sys/class/power_supply/BAT0/power_now").unwrap_or(0.0);
        let raw_w = power_uw / 1_000_000.0;
        let bat_watts = if bat_status.eq_ignore_ascii_case("Discharging") {
            -raw_w.abs()
        } else if bat_status.eq_ignore_ascii_case("Charging") {
            raw_w.abs()
        } else {
            0.0
        };

        let cycles_raw = fs::read_to_string("/sys/class/power_supply/BAT0/cycle_count")
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
            .filter(|&c| c >= 0)
            .map(|c| c as u32);
        let full_wh = read_f32("/sys/class/power_supply/BAT0/energy_full").map(|v| v / 1_000_000.0);
        let design_wh = read_f32("/sys/class/power_supply/BAT0/energy_full_design").map(|v| v / 1_000_000.0);
        let wear_pct = match (full_wh, design_wh) {
            (Some(f), Some(d)) if d > 0.1 => Some(((f / d) * 100.0).round().clamp(0.0, 110.0) as u8),
            _ => None,
        };

        let cpu_pct = self.sample_cpu_pct();
        let cpu_watts = self.sample_rapl_watts();
        let gpu_pct = self.sample_gpu_pct();
        let gpu_freq_mhz = read_u32("/sys/class/drm/card0/device/tile0/gt0/freq0/act_freq");
        let npu_pct = self.sample_npu_pct();
        let npu_freq_mhz = read_u32("/sys/class/accel/accel0/device/npu_current_frequency_mhz");
        let (ram_used_gb, ram_total_gb, ram_pct) = sample_ram();
        let cpu_temp_c = pick_temperature(temp_sensor_key);
        let als_lux = read_als_lux();

        SysTelemetry {
            bat_capacity: cap,
            bat_status,
            ac_online,
            bat_watts,
            bat_cycles: cycles_raw,
            bat_wh_full: full_wh,
            bat_wh_design: design_wh,
            bat_wear_pct: wear_pct,
            cpu_pct,
            cpu_temp_c,
            cpu_watts,
            gpu_pct,
            gpu_freq_mhz,
            npu_pct,
            npu_freq_mhz,
            ram_used_gb,
            ram_total_gb,
            ram_pct,
            als_lux,
        }
    }

    fn sample_cpu_pct(&mut self) -> f32 {
        let Ok(stat) = fs::read_to_string("/proc/stat") else { return 0.0; };
        let Some(line) = stat.lines().next() else { return 0.0; };
        let nums: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse::<u64>().ok())
            .collect();
        if nums.len() < 4 {
            return 0.0;
        }
        let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
        let total: u64 = nums.iter().sum();
        let pct = match self.prev_cpu {
            Some((pt, pi)) if total > pt => {
                let dt = (total - pt) as f32;
                let di = (idle.saturating_sub(pi)) as f32;
                ((dt - di) / dt * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        self.prev_cpu = Some((total, idle));
        pct
    }

    fn sample_rapl_watts(&mut self) -> Option<f32> {
        let uj = fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj")
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()?;
        let now = Instant::now();
        let watts = match self.prev_rapl {
            Some((prev_uj, prev_t)) if uj >= prev_uj => {
                let dt = now.duration_since(prev_t).as_secs_f32();
                if dt > 0.05 {
                    Some(((uj - prev_uj) as f32 / 1_000_000.0) / dt)
                } else {
                    None
                }
            }
            _ => None,
        };
        self.prev_rapl = Some((uj, now));
        watts
    }

    fn sample_gpu_pct(&mut self) -> Option<f32> {
        let idle_ms = fs::read_to_string("/sys/class/drm/card0/device/tile0/gt0/gtidle/idle_residency_ms")
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()?;
        let now = Instant::now();
        let pct = match self.prev_gpu_idle {
            Some((prev_idle, prev_t)) if idle_ms >= prev_idle => {
                let dt_ms = now.duration_since(prev_t).as_secs_f32() * 1000.0;
                if dt_ms > 50.0 {
                    let busy = (1.0 - (idle_ms - prev_idle) as f32 / dt_ms) * 100.0;
                    Some(busy.clamp(0.0, 100.0))
                } else {
                    None
                }
            }
            _ => None,
        };
        self.prev_gpu_idle = Some((idle_ms, now));
        pct
    }

    fn sample_npu_pct(&mut self) -> Option<f32> {
        let busy_us = fs::read_to_string("/sys/class/accel/accel0/device/npu_busy_time_us")
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()?;
        let now = Instant::now();
        let pct = match self.prev_npu_busy {
            Some((prev_us, prev_t)) if busy_us >= prev_us => {
                let dt_us = now.duration_since(prev_t).as_secs_f32() * 1_000_000.0;
                if dt_us > 50_000.0 {
                    Some((((busy_us - prev_us) as f32 / dt_us) * 100.0).clamp(0.0, 100.0))
                } else {
                    None
                }
            }
            _ => None,
        };
        self.prev_npu_busy = Some((busy_us, now));
        pct
    }
}

pub fn read_ac_online() -> bool {
    if let Some(v) = read_u32("/sys/class/power_supply/ADP1/online") {
        return v == 1;
    }
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.flatten() {
            let p = entry.path();
            let kind = fs::read_to_string(p.join("type")).unwrap_or_default();
            if kind.trim().eq_ignore_ascii_case("Mains")
                && read_u32(p.join("online").to_str().unwrap_or("")).unwrap_or(0) == 1
            {
                return true;
            }
        }
    }
    false
}

fn read_u32(path: &str) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()
}

fn read_f32(path: &str) -> Option<f32> {
    fs::read_to_string(path).ok()?.trim().parse::<f32>().ok()
}

fn sample_ram() -> (f32, f32, f32) {
    let Ok(raw) = fs::read_to_string("/proc/meminfo") else { return (0.0, 0.0, 0.0); };
    let mut total_kb = 0f32;
    let mut avail_kb = 0f32;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        }
    }
    if total_kb <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let used_kb = (total_kb - avail_kb).max(0.0);
    let total_gb = total_kb / 1_048_576.0;
    let used_gb = used_kb / 1_048_576.0;
    let pct = (used_kb / total_kb * 100.0).clamp(0.0, 100.0);
    (used_gb, total_gb, pct)
}

pub fn list_temp_sensors() -> Vec<TempSensorInfo> {
    let mut list = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else { return list; };
    let mut hw_paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    hw_paths.sort();

    for hw in hw_paths {
        let hw_name = fs::read_to_string(hw.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "hwmon".into());
        if !matches!(hw_name.as_str(), "coretemp" | "k10temp" | "acpitz" | "nvme") {
            continue;
        }
        let Ok(files) = fs::read_dir(&hw) else { continue; };
        let mut inputs: Vec<_> = files
            .flatten()
            .filter_map(|f| {
                let n = f.file_name().to_string_lossy().to_string();
                if n.starts_with("temp") && n.ends_with("_input") {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        inputs.sort();
        for inp in inputs {
            let prefix = inp.trim_end_matches("_input");
            let temp_mc = match fs::read_to_string(hw.join(&inp)).ok().and_then(|s| s.trim().parse::<f32>().ok()) {
                Some(v) if v > 0.0 && v < 150_000.0 => v,
                _ => continue,
            };
            let lbl = fs::read_to_string(hw.join(format!("{prefix}_label")))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| prefix.to_string());
            let hw_id = hw.file_name().unwrap_or_default().to_string_lossy();
            list.push(TempSensorInfo {
                key: format!("{hw_name}:{hw_id}:{prefix}"),
                label: format!("{hw_name} — {lbl}"),
                temp_c: temp_mc / 1000.0,
            });
        }
    }
    list
}

fn pick_temperature(sensor_key: &str) -> f32 {
    let sensors = list_temp_sensors();
    if sensor_key != "auto_max" {
        if let Some(found) = sensors.iter().find(|s| s.key == sensor_key) {
            return found.temp_c;
        }
    }
    // Default: max across coretemp sensors, fallback to thermal_zone0
    let core_max = sensors
        .iter()
        .filter(|s| s.key.starts_with("coretemp:") || s.key.starts_with("k10temp:"))
        .map(|s| s.temp_c)
        .fold(0.0f32, f32::max);
    if core_max > 1.0 {
        return core_max;
    }
    read_f32("/sys/class/thermal/thermal_zone0/temp")
        .map(|v| v / 1000.0)
        .unwrap_or(0.0)
}

pub fn read_als_lux() -> Option<f32> {
    let entries = fs::read_dir("/sys/bus/iio/devices").ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if let Ok(v) = fs::read_to_string(p.join("in_illuminance_input")) {
            if let Ok(lux) = v.trim().parse::<f32>() {
                return Some(lux);
            }
        }
        if let Ok(v) = fs::read_to_string(p.join("in_illuminance_raw")) {
            if let Ok(lux) = v.trim().parse::<f32>() {
                return Some(lux);
            }
        }
    }
    None
}

// ============================================================================
// DISPLAY CONTROL (kscreen-doctor -j)
// ============================================================================
#[derive(Debug, Clone)]
pub struct DisplayMode {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub hz_rounded: u32,
}

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub output_name: String,
    pub current_width: u32,
    pub current_height: u32,
    pub current_hz: u32,
    pub scale: f32,
    pub available_hz: Vec<u32>,
    pub modes: Vec<DisplayMode>,
}

pub fn query_internal_display() -> Option<DisplayInfo> {
    let out = Command::new("kscreen-doctor").arg("-j").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let root: Value = serde_json::from_slice(&out.stdout).ok()?;
    let outputs = root.get("outputs")?.as_array()?;
    let panel = outputs
        .iter()
        .find(|o| {
            o.get("connected").and_then(|v| v.as_bool()) == Some(true)
                && o.get("enabled").and_then(|v| v.as_bool()) == Some(true)
                && o.get("name").and_then(|v| v.as_str()).map(|n| n.starts_with("eDP")).unwrap_or(false)
        })
        .or_else(|| {
            outputs.iter().find(|o| {
                o.get("connected").and_then(|v| v.as_bool()) == Some(true)
                    && o.get("enabled").and_then(|v| v.as_bool()) == Some(true)
            })
        })?;

    let output_name = panel.get("name")?.as_str()?.to_string();
    let cur_id = panel.get("currentModeId")?.as_str().unwrap_or("");
    let scale = panel.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;

    let mut modes = Vec::new();
    let mut cur_w = 0u32;
    let mut cur_h = 0u32;
    let mut cur_hz = 0u32;

    if let Some(arr) = panel.get("modes").and_then(|m| m.as_array()) {
        for m in arr {
            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let hz_f = m.get("refreshRate").and_then(|v| v.as_f64()).unwrap_or(60.0);
            let hz_rounded = hz_f.round() as u32;
            let w = m.get("size").and_then(|s| s.get("width")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let h = m.get("size").and_then(|s| s.get("height")).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            if id == cur_id {
                cur_w = w;
                cur_h = h;
                cur_hz = hz_rounded;
            }
            if !id.is_empty() && w > 0 && h > 0 {
                modes.push(DisplayMode { id, width: w, height: h, hz_rounded });
            }
        }
    }

    let mut hz_set = BTreeSet::new();
    for m in &modes {
        if m.width == cur_w && m.height == cur_h {
            hz_set.insert(m.hz_rounded);
        }
    }
    let available_hz: Vec<u32> = hz_set.into_iter().collect();

    Some(DisplayInfo {
        output_name,
        current_width: cur_w,
        current_height: cur_h,
        current_hz: cur_hz,
        scale,
        available_hz,
        modes,
    })
}

pub fn set_display_hz(hz: u32) -> Result<(), String> {
    if let Some(info) = query_internal_display() {
        if let Some(m) = info
            .modes
            .iter()
            .find(|m| m.width == info.current_width && m.height == info.current_height && m.hz_rounded == hz)
        {
            let status = Command::new("kscreen-doctor")
                .arg(format!("output.{}.mode.{}", info.output_name, m.id))
                .status()
                .map_err(|e| format!("kscreen-doctor: {e}"))?;
            if status.success() {
                return Ok(());
            }
            return Err("kscreen-doctor вернул ошибку".into());
        }
        return Err(format!("Частота {hz} Гц недоступна для {}×{}", info.current_width, info.current_height));
    }
    // Fallback for fixed TM2424 IDs
    let mode_id = match hz {
        120 => "1",
        60 => "2",
        _ => return Err("Не найдена панель через kscreen-doctor".into()),
    };
    let status = Command::new("kscreen-doctor")
        .arg(format!("output.eDP-1.mode.{mode_id}"))
        .status()
        .map_err(|e| format!("kscreen-doctor: {e}"))?;
    if status.success() { Ok(()) } else { Err("kscreen-doctor error".into()) }
}

pub fn set_display_preset(high_res: bool) -> Result<(), String> {
    let (w, h, scale) = if high_res { (3120, 2080, "2") } else { (1920, 1280, "1.25") };
    if let Some(info) = query_internal_display() {
        if let Some(m) = info
            .modes
            .iter()
            .filter(|m| m.width == w && m.height == h)
            .max_by_key(|m| m.hz_rounded)
        {
            let status = Command::new("kscreen-doctor")
                .arg(format!("output.{}.mode.{}", info.output_name, m.id))
                .arg(format!("output.{}.scale.{scale}", info.output_name))
                .status()
                .map_err(|e| format!("kscreen-doctor: {e}"))?;
            if status.success() {
                return Ok(());
            }
        }
    }
    let mode_id = if high_res { "1" } else { "27" };
    let status = Command::new("kscreen-doctor")
        .arg(format!("output.eDP-1.mode.{mode_id}"))
        .arg(format!("output.eDP-1.scale.{scale}"))
        .status()
        .map_err(|e| format!("kscreen-doctor: {e}"))?;
    if status.success() { Ok(()) } else { Err("kscreen-doctor error".into()) }
}

// ============================================================================
// BRIGHTNESS (KDE PowerDevil D-Bus)
// ============================================================================
static BRIGHTNESS_MAX_CACHE: std::sync::OnceLock<i32> = std::sync::OnceLock::new();

fn cached_brightness_max() -> Option<i32> {
    if let Some(&v) = BRIGHTNESS_MAX_CACHE.get() {
        return Some(v);
    }
    let v = qdbus_int(
        "org.kde.Solid.PowerManagement",
        "/org/kde/Solid/PowerManagement/Actions/BrightnessControl",
        "brightnessMax",
    )?;
    if v > 0 {
        let _ = BRIGHTNESS_MAX_CACHE.set(v);
        Some(v)
    } else {
        None
    }
}

pub fn get_brightness_percent() -> Option<u8> {
    let cur = qdbus_int("org.kde.Solid.PowerManagement", "/org/kde/Solid/PowerManagement/Actions/BrightnessControl", "brightness")?;
    let max = cached_brightness_max()?;
    Some(((cur as f32 / max as f32) * 100.0).round().clamp(0.0, 100.0) as u8)
}

pub fn set_brightness_percent(pct: u8, silent: bool) -> Result<(), String> {
    let max = cached_brightness_max().unwrap_or(10000);
    let raw = ((pct.clamp(1, 100) as f32 / 100.0) * max as f32).round() as i32;
    let method = if silent { "setBrightnessSilent" } else { "setBrightness" };
    let status = Command::new("qdbus6")
        .args([
            "org.kde.Solid.PowerManagement",
            "/org/kde/Solid/PowerManagement/Actions/BrightnessControl",
            method,
            &raw.to_string(),
        ])
        .status()
        .map_err(|e| format!("qdbus6 brightness: {e}"))?;
    if status.success() { Ok(()) } else { Err("Ошибка установки яркости PowerDevil".into()) }
}

pub fn step_brightness(delta_pct: i32) {
    if let Some(cur) = get_brightness_percent() {
        let next = (cur as i32 + delta_pct).clamp(2, 100) as u8;
        let _ = set_brightness_percent(next, false);
    }
}

fn qdbus_int(service: &str, path: &str, method: &str) -> Option<i32> {
    let out = Command::new("qdbus6").args([service, path, method]).output().ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout).trim().parse::<i32>().ok()
}

// ============================================================================
// AUDIO / MEDIA / SCREENSHOT / LAUNCH / OSD / OWL / HARDWARE INFO / AUTOSTART
// ============================================================================
pub fn step_volume(delta_steps: i32) {
    if delta_steps == 0 { return; }
    let pct = (delta_steps.abs() * 2).min(20);
    let arg = if delta_steps > 0 { format!("{pct}%+") } else { format!("{pct}%-") };
    let _ = Command::new("wpctl")
        .args(["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &arg])
        .status();
}

pub fn toggle_mic_mute() {
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
        .status();
}

pub fn mpris_command(method: &str) {
    let Ok(out) = Command::new("qdbus6").output() else { return; };
    let list = String::from_utf8_lossy(&out.stdout);
    for line in list.lines() {
        let svc = line.trim();
        if svc.starts_with("org.mpris.MediaPlayer2.") {
            let _ = Command::new("qdbus6")
                .args([svc, "/org/mpris/MediaPlayer2", &format!("org.mpris.MediaPlayer2.Player.{method}")])
                .spawn();
            break;
        }
    }
}

pub fn take_screenshot() {
    let _ = Command::new("spectacle").arg("-r").spawn();
}

pub fn open_calculator() {
    for bin in ["kcalc", "qalculate-gtk", "gnome-calculator"] {
        if Command::new(bin).spawn().is_ok() {
            return;
        }
    }
}

pub fn launch_custom_command(cmd: &str) {
    let trimmed = cmd.trim();
    if trimmed.is_empty() { return; }
    let _ = Command::new("sh").args(["-c", trimmed]).spawn();
}

pub fn show_osd(enabled: bool, icon: &str, text: &str) {
    if !enabled { return; }
    let _ = Command::new("qdbus6")
        .args(["org.kde.plasmashell", "/org/kde/osdService", "showText", icon, text])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub struct OwlGuard {
    child: Option<Child>,
}

impl OwlGuard {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub fn set_active(&mut self, active: bool) -> Result<(), String> {
        if active {
            if self.child.is_some() {
                return Ok(());
            }
            let child = Command::new("systemd-inhibit")
                .args([
                    "--what=idle:sleep:handle-lid-switch",
                    "--who=XiControl",
                    "--why=Owl mode (keep awake)",
                    "sleep",
                    "infinity",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("systemd-inhibit: {e}"))?;
            self.child = Some(child);
            Ok(())
        } else {
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            Ok(())
        }
    }

    pub fn is_active(&mut self) -> bool {
        if let Some(c) = &mut self.child {
            if let Ok(Some(_)) = c.try_wait() {
                self.child = None;
                return false;
            }
            return true;
        }
        false
    }
}

impl Drop for OwlGuard {
    fn drop(&mut self) {
        let _ = self.set_active(false);
    }
}

#[derive(Debug, Clone)]
pub struct HardwareInfo {
    pub vendor: String,
    pub model: String,
    pub sku: String,
    pub bios_version: String,
    pub bios_date: String,
}

pub fn read_hardware_info() -> HardwareInfo {
    let read = |p: &str| {
        fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "—".into())
    };
    HardwareInfo {
        vendor: read("/sys/class/dmi/id/sys_vendor"),
        model: read("/sys/class/dmi/id/product_name"),
        sku: read("/sys/class/dmi/id/product_sku"),
        bios_version: read("/sys/class/dmi/id/bios_version"),
        bios_date: read("/sys/class/dmi/id/bios_date"),
    }
}

fn systemd_service_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".config/systemd/user/xicontrol.service"))
}

pub fn is_autostart_enabled() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-enabled", "xicontrol.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn set_autostart_enabled(enable: bool) -> Result<(), String> {
    let path = systemd_service_path().ok_or("HOME не задан")?;
    if enable {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir systemd user: {e}"))?;
        }
        let unit = format!(
            "[Unit]\nDescription=XiControl Linux Daemon & Tray (Xiaomi Book Pro 14)\nAfter=graphical-session.target\n\n[Service]\nType=simple\nExecStart={} daemon\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=graphical-session.target\n",
            exe.display()
        );
        fs::write(&path, unit).map_err(|e| format!("write service: {e}"))?;
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        let st = Command::new("systemctl")
            .args(["--user", "enable", "--now", "xicontrol.service"])
            .status()
            .map_err(|e| format!("systemctl enable: {e}"))?;
        if !st.success() {
            return Err("systemctl --user enable --now вернул ошибку".into());
        }
        Ok(())
    } else {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "xicontrol.service"])
            .status();
        let _ = fs::remove_file(&path);
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        Ok(())
    }
}
