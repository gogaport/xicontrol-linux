use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const MIFS_DEV: &str = "/dev/xiaomi_mifs";
const MIFS_OP_GET: u8 = 0xFA;
const MIFS_OP_SET: u8 = 0xFB;
const MIFS_STATUS_OK: u8 = 0x80;
const MIFS_STATUS_UNSUPPORTED: u8 = 0xE0;

const CMD_PERF: u8 = 0x08;
const CMD_MIC: u8 = 0x0A;
const CMD_CHARGE: u8 = 0x10;

const SUB_BATTERY_HEALTH: u8 = 0x01;
const SUB_CHARGE_LIMIT: u8 = 0x02;
const SUB_ADAPTER_WATTS: u8 = 0x06;

static MIFS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfMode {
    Eco = 0x0A,
    Quiet = 0x02,
    Balance = 0x01,
    Auto = 0x09,
    Turbo = 0x03,
    FullSpeed = 0x04,
}

impl PerfMode {
    pub const ALL: [PerfMode; 6] = [
        PerfMode::Eco,
        PerfMode::Quiet,
        PerfMode::Balance,
        PerfMode::Auto,
        PerfMode::Turbo,
        PerfMode::FullSpeed,
    ];

    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x0A => Some(PerfMode::Eco),
            0x02 => Some(PerfMode::Quiet),
            0x01 => Some(PerfMode::Balance),
            0x09 => Some(PerfMode::Auto),
            0x03 => Some(PerfMode::Turbo),
            0x04 => Some(PerfMode::FullSpeed),
            _ => None,
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "eco" => Some(PerfMode::Eco),
            "quiet" | "silent" | "silence" => Some(PerfMode::Quiet),
            "balance" | "balanced" => Some(PerfMode::Balance),
            "auto" | "smart" => Some(PerfMode::Auto),
            "turbo" => Some(PerfMode::Turbo),
            "fullspeed" | "full" | "max" => Some(PerfMode::FullSpeed),
            _ => None,
        }
    }

    pub fn label(&self, en: bool) -> &'static str {
        match (self, en) {
            (PerfMode::Eco, _) => "Eco",
            (PerfMode::Quiet, false) => "Тихий",
            (PerfMode::Quiet, true) => "Quiet",
            (PerfMode::Balance, false) => "Баланс",
            (PerfMode::Balance, true) => "Balance",
            (PerfMode::Auto, false) => "Авто",
            (PerfMode::Auto, true) => "Auto",
            (PerfMode::Turbo, false) => "Турбо",
            (PerfMode::Turbo, true) => "Turbo",
            (PerfMode::FullSpeed, false) => "Полная",
            (PerfMode::FullSpeed, true) => "FullSpeed",
        }
    }

    pub fn cli_name(&self) -> &'static str {
        match self {
            PerfMode::Eco => "eco",
            PerfMode::Quiet => "quiet",
            PerfMode::Balance => "balance",
            PerfMode::Auto => "auto",
            PerfMode::Turbo => "turbo",
            PerfMode::FullSpeed => "fullspeed",
        }
    }

    pub fn api_name(&self) -> &'static str {
        match self {
            PerfMode::Eco => "Eco",
            PerfMode::Quiet => "Quiet",
            PerfMode::Balance => "Balance",
            PerfMode::Auto => "Auto",
            PerfMode::Turbo => "Turbo",
            PerfMode::FullSpeed => "FullSpeed",
        }
    }

    pub fn is_battery_capable(&self) -> bool {
        matches!(self, PerfMode::Eco | PerfMode::Auto)
    }
}

pub const CHARGE_PRESETS: [u8; 6] = [40, 50, 60, 70, 80, 100];

pub fn charge_code_for_percent(pct: u8) -> Option<u8> {
    match pct {
        100 => Some(0),
        80 => Some(1),
        70 => Some(5),
        60 => Some(6),
        50 => Some(7),
        40 => Some(8),
        _ => None,
    }
}

pub fn charge_percent_for_code(code: u8) -> Option<u8> {
    match code {
        0 => Some(100),
        1 | 4 => Some(80),
        5 => Some(70),
        6 => Some(60),
        7 => Some(50),
        8 => Some(40),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub enum PerfModeSetError {
    /// Device open/read/write error (e.g. driver unloaded, permission denied, transient I/O).
    Io(String),
    /// EC firmware explicitly rejected or did not retain the requested mode.
    FirmwareRejected(String),
}

impl PerfModeSetError {
    pub fn is_firmware_rejection(&self) -> bool {
        matches!(self, PerfModeSetError::FirmwareRejected(_))
    }

    pub fn into_message(self) -> String {
        match self {
            PerfModeSetError::Io(s) | PerfModeSetError::FirmwareRejected(s) => s,
        }
    }
}

fn mifs_call(op: u8, cmd: u8, arg: u8, val: u8) -> Result<[u8; 32], PerfModeSetError> {
    let _guard = MIFS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(MIFS_DEV)
        .map_err(|e| PerfModeSetError::Io(format!("Нет доступа к {MIFS_DEV}: {e}")))?;

    let mut pkt = [0u8; 32];
    pkt[1] = op;
    pkt[3] = cmd;
    pkt[4] = arg;
    pkt[6] = val;

    file.write_all(&pkt)
        .map_err(|e| PerfModeSetError::Io(format!("Ошибка записи MIFS: {e}")))?;
    let mut out = [0u8; 32];
    file.read_exact(&mut out)
        .map_err(|e| PerfModeSetError::Io(format!("Ошибка чтения MIFS: {e}")))?;

    if out[1] == MIFS_STATUS_UNSUPPORTED {
        return Err(PerfModeSetError::FirmwareRejected(format!(
            "Команда 0x{cmd:02X} не поддерживается прошивкой (0xE0)"
        )));
    }
    if out[1] != MIFS_STATUS_OK && out[1] != op {
        return Err(PerfModeSetError::FirmwareRejected(format!(
            "Прошивка отклонила команду 0x{cmd:02X} (статус 0x{:02X})",
            out[1]
        )));
    }
    Ok(out)
}

pub fn get_perf_mode() -> Result<PerfMode, String> {
    let out = mifs_call(MIFS_OP_GET, CMD_PERF, 0, 0).map_err(PerfModeSetError::into_message)?;
    PerfMode::from_byte(out[4]).ok_or_else(|| format!("Неизвестный код режима: 0x{:02X}", out[4]))
}

pub fn set_perf_mode_detailed(mode: PerfMode) -> Result<(), PerfModeSetError> {
    let prev = get_perf_mode().ok();
    mifs_call(MIFS_OP_SET, CMD_PERF, mode as u8, 0)?;
    let mut actual = get_perf_mode().map_err(PerfModeSetError::Io)?;
    // On TM2424, switching directly out of Eco can first land in Auto; retry once.
    if actual != mode && prev == Some(PerfMode::Eco) && actual == PerfMode::Auto {
        thread::sleep(Duration::from_millis(120));
        mifs_call(MIFS_OP_SET, CMD_PERF, mode as u8, 0)?;
        actual = get_perf_mode().map_err(PerfModeSetError::Io)?;
    }
    // On TM2424 battery power, EC echoes Quiet/Turbo/FullSpeed at t=0 and reverts ~150ms later.
    if actual == mode && !mode.is_battery_capable() {
        thread::sleep(Duration::from_millis(220));
        actual = get_perf_mode().map_err(PerfModeSetError::Io)?;
    }
    if actual != mode {
        return Err(PerfModeSetError::FirmwareRejected(format!(
            "Прошивка EC не удержала режим {} (активен {})",
            mode.label(false),
            actual.label(false)
        )));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn set_perf_mode(mode: PerfMode) -> Result<(), String> {
    set_perf_mode_detailed(mode).map_err(PerfModeSetError::into_message)
}

pub fn get_charge_limit() -> Result<u8, String> {
    let out = mifs_call(MIFS_OP_GET, CMD_CHARGE, SUB_CHARGE_LIMIT, 0)
        .map_err(PerfModeSetError::into_message)?;
    charge_percent_for_code(out[6]).ok_or_else(|| format!("Неизвестный код лимита заряда: {}", out[6]))
}

pub fn set_charge_limit(percent: u8) -> Result<(), String> {
    let code = charge_code_for_percent(percent)
        .ok_or_else(|| "Поддерживаемые уровни заряда: 40, 50, 60, 70, 80, 100%".to_string())?;
    if code != 0 {
        // Reset EC state before setting granular care threshold (matches XiControl ChargeGuard)
        let _ = mifs_call(MIFS_OP_SET, CMD_CHARGE, SUB_CHARGE_LIMIT, 0);
        thread::sleep(Duration::from_millis(80));
    }
    mifs_call(MIFS_OP_SET, CMD_CHARGE, SUB_CHARGE_LIMIT, code)
        .map_err(PerfModeSetError::into_message)?;
    Ok(())
}

pub fn get_adapter_watts() -> Option<u8> {
    mifs_call(MIFS_OP_GET, CMD_CHARGE, SUB_ADAPTER_WATTS, 0)
        .ok()
        .map(|o| o[6])
}

pub fn get_battery_health() -> Option<u8> {
    mifs_call(MIFS_OP_GET, CMD_CHARGE, SUB_BATTERY_HEALTH, 0)
        .ok()
        .map(|o| o[6])
}

pub fn get_mic_mute() -> Option<bool> {
    mifs_call(MIFS_OP_GET, CMD_MIC, 0x05, 0)
        .ok()
        .map(|o| o[4] == 0 || o[6] == 0)
}
