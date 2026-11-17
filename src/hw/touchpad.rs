use std::fs::{self, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

pub const TP_FRAME_LEN: usize = 33;
pub const TP_REPORT_ID: u8 = 0x0D;
pub const TP_CTRL_REPORT_ID: u8 = 0x0E;
pub const TP_CMD_COMMIT: u8 = 0x01;
pub const TP_CMD_PULSE: u8 = 0x02;
pub const TP_CMD_FW: u8 = 0x10;
pub const TP_CMD_SLIDE: u8 = 0x58;
pub const TP_CMD_HEAVY: u8 = 0x59;
pub const TP_CMD_PRESSURE: u8 = 0x5B;
pub const TP_CMD_VIBRATION: u8 = 0x5D;
const TP_CMD_CTRL_VERSION: u8 = 0xB6;

const BLTP_I2C_ADDR: u16 = 0x2C;
const I2C_RDWR: u64 = 0x0707;
const I2C_M_RD: u16 = 0x0001;

// Logical ranges from BLTP7853 HID report descriptor
pub const TP_LOGICAL_MAX_X: f64 = 3282.0;
pub const TP_LOGICAL_MAX_Y: f64 = 2124.0;
pub const TP_PHYSICAL_WIDTH_MM: f64 = 139.0;
pub const TP_PHYSICAL_HEIGHT_MM: f64 = 90.0;

static TP_LOCK: Mutex<()> = Mutex::new(());

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

#[repr(C)]
struct I2cRdwrIoctlData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn flock(fd: i32, operation: i32) -> i32;
}

const LOCK_EX: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadVibration {
    Low,
    Medium,
    High,
}

impl TouchpadVibration {
    pub const ALL: [TouchpadVibration; 3] = [
        TouchpadVibration::Low,
        TouchpadVibration::Medium,
        TouchpadVibration::High,
    ];

    pub fn gears(&self) -> [u16; 2] {
        match self {
            TouchpadVibration::Low => [56, 80],
            TouchpadVibration::Medium => [80, 104],
            TouchpadVibration::High => [104, 128],
        }
    }

    pub fn from_gears(a: u16, b: u16) -> Option<Self> {
        match (a, b) {
            (56, 80) => Some(TouchpadVibration::Low),
            (80, 104) => Some(TouchpadVibration::Medium),
            (104, 128) => Some(TouchpadVibration::High),
            _ => None,
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" | "1" => Some(TouchpadVibration::Low),
            "medium" | "med" | "2" => Some(TouchpadVibration::Medium),
            "high" | "3" => Some(TouchpadVibration::High),
            _ => None,
        }
    }

    pub fn label(&self, en: bool) -> &'static str {
        match (self, en) {
            (TouchpadVibration::Low, false) => "Слабая",
            (TouchpadVibration::Low, true) => "Low",
            (TouchpadVibration::Medium, false) => "Средняя",
            (TouchpadVibration::Medium, true) => "Medium",
            (TouchpadVibration::High, false) => "Сильная",
            (TouchpadVibration::High, true) => "High",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TouchpadState {
    pub vibration: Option<TouchpadVibration>,
    pub pressure: u16,
    pub heavy_press: bool,
    pub slide_strength: Option<u16>,
    pub firmware: String,
    pub controller_firmware: String,
}

struct BltpI2c {
    file: fs::File,
    cmd_reg: u16,
    data_reg: u16,
}

pub fn find_bltp7853_hidraw() -> Option<String> {
    let entries = fs::read_dir("/sys/class/hidraw").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let uevent_path = format!("/sys/class/hidraw/{name}/device/uevent");
        if let Ok(uevent) = fs::read_to_string(&uevent_path) {
            if uevent.contains("347D:00007853") || uevent.contains("BLTP7853") {
                return Some(format!("/dev/{name}"));
            }
        }
    }
    None
}

fn find_bltp7853_i2c_dev() -> Option<String> {
    let resolved = fs::canonicalize("/sys/bus/i2c/devices/i2c-BLTP7853:00").ok()?;
    for comp in resolved.components() {
        let s = comp.as_os_str().to_string_lossy();
        if let Some(rest) = s.strip_prefix("i2c-") {
            if rest.chars().all(|c| c.is_ascii_digit()) {
                return Some(format!("/dev/i2c-{rest}"));
            }
        }
    }
    None
}

fn i2c_rw(fd: i32, msgs: &mut [I2cMsg]) -> bool {
    let mut rdwr = I2cRdwrIoctlData {
        msgs: msgs.as_mut_ptr(),
        nmsgs: msgs.len() as u32,
    };
    unsafe { ioctl(fd, I2C_RDWR, &mut rdwr) >= 0 }
}

fn open_bltp7853_i2c() -> Result<BltpI2c, String> {
    let path = find_bltp7853_i2c_dev().ok_or("Тачпад BLTP7853 не найден на шине I2C")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Нет доступа к {path}: {e}"))?;
    let fd = file.as_raw_fd();
    // Serialize BLTP7853 SET_REPORT/GET_REPORT transactions across daemon, GUI, and CLI processes.
    unsafe {
        let _ = flock(fd, LOCK_EX);
    }

    let mut reg = [0x01u8, 0x00];
    let mut desc = [0u8; 30];
    let mut msgs = [
        I2cMsg { addr: BLTP_I2C_ADDR, flags: 0, len: 2, buf: reg.as_mut_ptr() },
        I2cMsg { addr: BLTP_I2C_ADDR, flags: I2C_M_RD, len: 30, buf: desc.as_mut_ptr() },
    ];
    if !i2c_rw(fd, &mut msgs) {
        return Err(format!("Ошибка чтения HID-дескриптора BLTP7853 через {path}"));
    }
    let desc_len = u16::from_le_bytes([desc[0], desc[1]]);
    let vid = u16::from_le_bytes([desc[20], desc[21]]);
    let pid = u16::from_le_bytes([desc[22], desc[23]]);
    if desc_len != 30 || vid != 0x347D || pid != 0x7853 {
        return Err(format!("Неожиданный HID-дескриптор: len={desc_len}, {vid:#06x}:{pid:#06x}"));
    }
    let cmd_reg = u16::from_le_bytes([desc[16], desc[17]]);
    let data_reg = u16::from_le_bytes([desc[18], desc[19]]);
    Ok(BltpI2c { file, cmd_reg, data_reg })
}

pub fn tp_checksum(f: &[u8; TP_FRAME_LEN]) -> u8 {
    let mut x = 0u8;
    for &b in &f[3..TP_FRAME_LEN - 1] {
        x ^= b;
    }
    x.wrapping_add(1)
}

pub fn tp_frame(cmd: u8, data: &[u8], first: bool) -> [u8; TP_FRAME_LEN] {
    let mut f = [0u8; TP_FRAME_LEN];
    f[0] = TP_REPORT_ID;
    f[1] = (data.len() + 7) as u8;
    f[3] = if first { 0 } else { 1 };
    f[5] = (data.len() + 1) as u8;
    f[8] = cmd;
    f[9..9 + data.len()].copy_from_slice(data);
    f[2] = tp_checksum(&f);
    f
}

fn tp_write_frame(cmd: u8, values: &[u16]) -> [u8; TP_FRAME_LEN] {
    let mut data = Vec::with_capacity(values.len() * 2);
    for &v in values {
        data.push((v & 0xFF) as u8);
        data.push((v >> 8) as u8);
    }
    tp_frame(cmd, &data, true)
}

fn tp_commit_frame(cmd: u8) -> [u8; TP_FRAME_LEN] {
    tp_frame(TP_CMD_COMMIT, &[cmd, (0x100u16 - cmd as u16) as u8], false)
}

fn tp_read_request(cmd: u8, count: usize) -> [u8; TP_FRAME_LEN] {
    let mut f = [0u8; TP_FRAME_LEN];
    f[0] = TP_REPORT_ID;
    f[1] = 7;
    f[3] = 1;
    f[5] = 1;
    f[7] = (count * 2) as u8;
    f[8] = cmd;
    f[2] = tp_checksum(&f);
    f
}

fn tp_send(dev: &BltpI2c, frame: &[u8; TP_FRAME_LEN]) -> bool {
    let mut pkt = [0u8; 8 + TP_FRAME_LEN];
    let cmd = dev.cmd_reg.to_le_bytes();
    let data = dev.data_reg.to_le_bytes();
    pkt[0] = cmd[0];
    pkt[1] = cmd[1];
    pkt[2] = 0x20 | (frame[0] & 0x0F); // SET_REPORT Output, Report ID
    pkt[3] = 0x03;
    pkt[4] = data[0];
    pkt[5] = data[1];
    pkt[6] = (2 + TP_FRAME_LEN) as u8;
    pkt[7] = 0x00;
    pkt[8..].copy_from_slice(frame);
    let mut msg = [I2cMsg {
        addr: BLTP_I2C_ADDR,
        flags: 0,
        len: pkt.len() as u16,
        buf: pkt.as_mut_ptr(),
    }];
    i2c_rw(dev.file.as_raw_fd(), &mut msg)
}

fn tp_get_report(dev: &BltpI2c, report_id: u8) -> Option<[u8; TP_FRAME_LEN]> {
    let cmd = dev.cmd_reg.to_le_bytes();
    let data = dev.data_reg.to_le_bytes();
    let mut req = [cmd[0], cmd[1], 0x10 | (report_id & 0x0F), 0x02, data[0], data[1]];
    let mut raw = [0u8; 2 + TP_FRAME_LEN];
    let mut msgs = [
        I2cMsg { addr: BLTP_I2C_ADDR, flags: 0, len: req.len() as u16, buf: req.as_mut_ptr() },
        I2cMsg { addr: BLTP_I2C_ADDR, flags: I2C_M_RD, len: raw.len() as u16, buf: raw.as_mut_ptr() },
    ];
    if !i2c_rw(dev.file.as_raw_fd(), &mut msgs) {
        return None;
    }
    let total = u16::from_le_bytes([raw[0], raw[1]]) as usize;
    if total < 3 {
        return None;
    }
    let body_len = (total - 2).min(TP_FRAME_LEN);
    let mut out = [0u8; TP_FRAME_LEN];
    out[..body_len].copy_from_slice(&raw[2..2 + body_len]);
    Some(out)
}

fn tp_read_values(dev: &BltpI2c, cmd: u8, count: usize) -> Option<Vec<u16>> {
    let req = tp_read_request(cmd, count);
    for _ in 0..3 {
        if !tp_send(dev, &req) {
            return None;
        }
        thread::sleep(Duration::from_millis(80));
        let Some(r) = tp_get_report(dev, TP_REPORT_ID) else {
            continue;
        };
        if r[0] != TP_REPORT_ID || r[2] != tp_checksum(&r) {
            continue;
        }
        let mut vals = Vec::with_capacity(count);
        let mut any_non_zero = false;
        for i in 0..count {
            let v = (r[4 + i * 2] as u16) | ((r[5 + i * 2] as u16) << 8);
            if v != 0 {
                any_non_zero = true;
            }
            vals.push(v);
        }
        if any_non_zero {
            return Some(vals);
        }
    }
    None
}

fn tp_read_controller_version(dev: &BltpI2c) -> Option<String> {
    let mut req = [0u8; TP_FRAME_LEN];
    req[..9].copy_from_slice(&[0x0E, 0x07, 0xB4, 0x00, 0x00, 0x01, 0x00, 0x04, TP_CMD_CTRL_VERSION]);
    for _ in 0..3 {
        if !tp_send(dev, &req) {
            return None;
        }
        thread::sleep(Duration::from_millis(35));
        let Some(r) = tp_get_report(dev, TP_CTRL_REPORT_ID) else {
            continue;
        };
        if r[0] == TP_CTRL_REPORT_ID && r[1] == 6 && r[2] == TP_CMD_CTRL_VERSION {
            return Some(format!("{}.{}.{}.{}", r[7], r[6], r[4], r[5]));
        }
    }
    None
}

fn tp_write_confirmed(cmd: u8, values: &[u16]) -> Result<(), String> {
    let _guard = TP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dev = open_bltp7853_i2c()?;
    let wf = tp_write_frame(cmd, values);
    let cf = tp_commit_frame(cmd);
    if !tp_send(&dev, &wf) {
        return Err("Ошибка отправки кадра записи в тачпад".into());
    }
    thread::sleep(Duration::from_millis(120));
    if !tp_send(&dev, &cf) {
        return Err("Ошибка отправки кадра подтверждения в тачпад".into());
    }
    thread::sleep(Duration::from_millis(120));
    let back = tp_read_values(&dev, cmd, values.len())
        .ok_or("Тачпад не подтвердил запись чтением")?;
    if back[0] != values[0] {
        return Err(format!("Тачпад вернул {}, ожидалось {}", back[0], values[0]));
    }
    Ok(())
}

pub fn read_touchpad_state() -> Result<TouchpadState, String> {
    let _guard = TP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dev = open_bltp7853_i2c()?;

    let vib = tp_read_values(&dev, TP_CMD_VIBRATION, 2).ok_or("Не удалось прочитать вибрацию тачпада")?;
    let press = tp_read_values(&dev, TP_CMD_PRESSURE, 4).ok_or("Не удалось прочитать усилие нажатия")?;
    let heavy = tp_read_values(&dev, TP_CMD_HEAVY, 1).unwrap_or_else(|| vec![2]);
    let slide = tp_read_values(&dev, TP_CMD_SLIDE, 1).map(|v| v[0]);
    let fw = tp_read_values(&dev, TP_CMD_FW, 2)
        .map(|v| format!("{}.{}.{}.{}", v[0] & 0xFF, v[0] >> 8, v[1] & 0xFF, v[1] >> 8))
        .unwrap_or_else(|| "Unknown".into());
    let ctrl_fw = tp_read_controller_version(&dev).unwrap_or_else(|| "Unknown".into());

    Ok(TouchpadState {
        vibration: TouchpadVibration::from_gears(vib[0], vib[1]),
        pressure: press[0],
        heavy_press: heavy[0] == 1,
        slide_strength: slide,
        firmware: fw,
        controller_firmware: ctrl_fw,
    })
}

pub fn set_touchpad_vibration(level: TouchpadVibration) -> Result<(), String> {
    tp_write_confirmed(TP_CMD_VIBRATION, &level.gears())
}

pub fn set_touchpad_pressure(threshold: u16) -> Result<(), String> {
    if ![100, 125, 140].contains(&threshold) {
        return Err("Поддерживаемые пороги нажатия: 100 (Лёгкое), 125 (Заводское), 140 (Тугое)".into());
    }
    let vals = [threshold, threshold * 33 / 100, 500, 400];
    tp_write_confirmed(TP_CMD_PRESSURE, &vals)
}

pub fn set_touchpad_heavy_press(on: bool) -> Result<(), String> {
    tp_write_confirmed(TP_CMD_HEAVY, &[if on { 1 } else { 2 }])
}

pub fn set_touchpad_slide_strength(strength: u16) -> Result<(), String> {
    let clamped = strength.clamp(10, 100);
    tp_write_confirmed(TP_CMD_SLIDE, &[clamped])
}

pub fn pulse_touchpad_effect(effect: u8) -> Result<(), String> {
    let _guard = TP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dev = open_bltp7853_i2c()?;
    let frame = tp_frame(TP_CMD_PULSE, &[effect, 0u8.wrapping_sub(effect)], false);
    if tp_send(&dev, &frame) {
        Ok(())
    } else {
        Err("Не удалось отправить импульс в тачпад".into())
    }
}

pub fn pulse_touchpad() -> Result<(), String> {
    pulse_touchpad_effect(0x32)
}

// ============================================================================
// KDE KWIN INPUT DEVICE ENABLE/DISABLE (Touchpad & Touchscreen)
// ============================================================================
fn find_input_event_names(substr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/input") else {
        return out;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("event") {
            continue;
        }
        let name_path = format!("/sys/class/input/{fname}/device/name");
        if let Ok(dev_name) = fs::read_to_string(&name_path) {
            if dev_name.contains(substr) {
                out.push(fname);
            }
        }
    }
    out
}

fn is_virtual_input(fname: &str) -> bool {
    fs::canonicalize(format!("/sys/class/input/{fname}"))
        .map(|p| p.to_string_lossy().contains("/virtual/"))
        .unwrap_or(false)
}

pub fn find_raw_touchpad_event() -> Option<String> {
    find_input_event_names("BLTP7853")
        .into_iter()
        .find(|f| !is_virtual_input(f))
}

pub fn find_touchscreen_event() -> Option<String> {
    find_input_event_names("2808:4001").into_iter().next()
}

fn kwin_input_get_enabled(event_name: &str) -> Option<bool> {
    let out = Command::new("qdbus6")
        .args([
            "org.kde.KWin",
            &format!("/org/kde/KWin/InputDevice/{event_name}"),
            "org.freedesktop.DBus.Properties.Get",
            "org.kde.KWin.InputDevice",
            "enabled",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match s.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn kwin_input_set_enabled(event_name: &str, enabled: bool) -> Result<(), String> {
    let status = Command::new("qdbus6")
        .args([
            "org.kde.KWin",
            &format!("/org/kde/KWin/InputDevice/{event_name}"),
            "org.freedesktop.DBus.Properties.Set",
            "org.kde.KWin.InputDevice",
            "enabled",
            if enabled { "true" } else { "false" },
        ])
        .status()
        .map_err(|e| format!("qdbus6 error: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Не удалось переключить {event_name} через KWin D-Bus"))
    }
}

pub fn is_touchpad_enabled() -> Option<bool> {
    let mut devs = find_input_event_names("BLTP7853");
    // Check virtual uinput first, fall back to raw if KWin has not registered virtual yet
    devs.sort_by_key(|f| !is_virtual_input(f));
    for ev in devs {
        if let Some(v) = kwin_input_get_enabled(&ev) {
            return Some(v);
        }
    }
    None
}

pub fn set_touchpad_enabled(enabled: bool) -> Result<(), String> {
    let devs = find_input_event_names("BLTP7853");
    if devs.is_empty() {
        return Err("Устройство тачпада (event*) не найдено".into());
    }
    let mut last_err = None;
    let mut any_ok = false;
    for ev in devs {
        match kwin_input_set_enabled(&ev, enabled) {
            Ok(()) => any_ok = true,
            Err(e) => last_err = Some(e),
        }
    }
    if any_ok {
        Ok(())
    } else {
        Err(last_err.unwrap_or_else(|| "Не удалось переключить тачпад через KWin".into()))
    }
}

pub fn is_touchscreen_enabled() -> Option<bool> {
    let ev = find_touchscreen_event()?;
    kwin_input_get_enabled(&ev)
}

pub fn set_touchscreen_enabled(enabled: bool) -> Result<(), String> {
    let ev = find_touchscreen_event().ok_or("Сенсорный экран (2808:4001) не найден")?;
    kwin_input_set_enabled(&ev, enabled)
}

// ============================================================================
// RAW PTP CONTACT PARSER (Report ID 0x04) + EDGE SLIDERS + HEAVY PRESS
// ============================================================================
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchContact {
    pub id: u8,
    pub x: f64, // 0.0..1.0
    pub y: f64, // 0.0..1.0
    pub pressure: u16,
}

/// Parses BLTP7853 Report ID 0x04 (40 bytes): 5 contact slots x 7 bytes.
pub fn parse_hid_report_04(buf: &[u8]) -> Option<Vec<TouchContact>> {
    if buf.len() < 40 || buf[0] != 0x04 {
        return None;
    }
    let mut out = Vec::with_capacity(5);
    for slot in 0..5 {
        let off = 1 + slot * 7;
        let flags = buf[off];
        let tip = (flags & 0x02) != 0;
        if !tip {
            continue;
        }
        let id = (flags >> 2) & 0x07;
        let raw_x = u16::from_le_bytes([buf[off + 1], buf[off + 2]]) as f64;
        let raw_y = u16::from_le_bytes([buf[off + 3], buf[off + 4]]) as f64;
        let pressure = u16::from_le_bytes([buf[off + 5], buf[off + 6]]);
        out.push(TouchContact {
            id,
            x: (raw_x / TP_LOGICAL_MAX_X).clamp(0.0, 1.0),
            y: (raw_y / TP_LOGICAL_MAX_Y).clamp(0.0, 1.0),
            pressure,
        });
    }
    Some(out)
}

#[derive(Debug, Clone)]
pub struct HeavyPressDetector {
    pub threshold: u16,
    fired: bool,
}

impl HeavyPressDetector {
    pub fn new(threshold: u16) -> Self {
        Self { threshold, fired: false }
    }

    pub fn update(&mut self, contacts: &[TouchContact]) -> bool {
        if contacts.is_empty() {
            self.fired = false;
            return false;
        }
        if self.fired {
            return false;
        }
        if contacts.iter().any(|c| c.pressure > self.threshold) {
            self.fired = true;
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadEdge {
    None,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct TouchpadEdgeGesture {
    strip: f64,
    step: f64,
    dead_zone_bottom: f64,
    active_id: i16,
    edge: TouchpadEdge,
    anchor_y: f64,
    cancelled: bool,
}

impl TouchpadEdgeGesture {
    pub fn new(width_mm: u8, swipes_per_range: f32, dead_zone_bottom_mm: u8) -> Self {
        let strip = ((width_mm as f64) / TP_PHYSICAL_WIDTH_MM).clamp(0.02, 0.35);
        // Step fraction matches XiControl EdgeSlideScale (swipes_per_range ~1.5 -> ~0.045 per 2% step)
        let swipes = (swipes_per_range as f64).clamp(0.5, 5.0);
        let step = (0.0675 / swipes).clamp(0.015, 0.25);
        let dz = ((dead_zone_bottom_mm as f64) / TP_PHYSICAL_HEIGHT_MM).clamp(0.0, 0.35);
        Self {
            strip,
            step,
            dead_zone_bottom: dz,
            active_id: -1,
            edge: TouchpadEdge::None,
            anchor_y: 0.0,
            cancelled: false,
        }
    }

    pub fn reset(&mut self) {
        self.active_id = -1;
        self.edge = TouchpadEdge::None;
        self.cancelled = false;
    }

    fn edge_of(&self, x: f64) -> TouchpadEdge {
        if x <= self.strip {
            TouchpadEdge::Left
        } else if x >= 1.0 - self.strip {
            TouchpadEdge::Right
        } else {
            TouchpadEdge::None
        }
    }

    pub fn update(&mut self, contacts: &[TouchContact]) -> (TouchpadEdge, i32) {
        if contacts.is_empty() {
            self.reset();
            return (TouchpadEdge::None, 0);
        }
        if contacts.len() > 1 {
            self.cancelled = true;
            self.active_id = -1;
            return (TouchpadEdge::None, 0);
        }
        if self.cancelled {
            return (TouchpadEdge::None, 0);
        }
        let c = contacts[0];
        if self.active_id != c.id as i16 {
            if self.dead_zone_bottom > 0.0 && c.y >= 1.0 - self.dead_zone_bottom {
                self.cancelled = true;
                self.active_id = -1;
                return (TouchpadEdge::None, 0);
            }
            let e = self.edge_of(c.x);
            if e == TouchpadEdge::None {
                self.cancelled = true;
                self.active_id = -1;
                return (TouchpadEdge::None, 0);
            }
            self.active_id = c.id as i16;
            self.edge = e;
            self.anchor_y = c.y;
            return (e, 0);
        }

        let moved = self.anchor_y - c.y;
        let steps = (moved / self.step) as i32;
        if steps == 0 {
            return (self.edge, 0);
        }
        self.anchor_y -= (steps as f64) * self.step;
        (self.edge, steps)
    }
}
