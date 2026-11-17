use super::touchpad::{find_raw_touchpad_event, TP_PHYSICAL_HEIGHT_MM};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const EV_MSC: u16 = 0x04;

const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;

const BTN_LEFT: u16 = 272;
const BTN_TOOL_FINGER: u16 = 325;
const BTN_TOOL_QUINTTAP: u16 = 328;
const BTN_TOUCH: u16 = 330;
const BTN_TOOL_DOUBLETAP: u16 = 333;
const BTN_TOOL_TRIPLETAP: u16 = 334;
const BTN_TOOL_QUADTAP: u16 = 335;

const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_PRESSURE: u16 = 24;
const ABS_MT_SLOT: u16 = 47;
const ABS_MT_POSITION_X: u16 = 53;
const ABS_MT_POSITION_Y: u16 = 54;
const ABS_MT_TRACKING_ID: u16 = 57;
const ABS_MT_PRESSURE: u16 = 58;

const MSC_TIMESTAMP: u16 = 5;

const INPUT_PROP_POINTER: i32 = 0;
const INPUT_PROP_BUTTONPAD: i32 = 2;
const INPUT_PROP_PRESSUREPAD: i32 = 7;

const EVIOCGID: u64 = 0x8008_4502;
const EVIOCGRAB: u64 = 0x4004_4590;
const EVIOCGABS_BASE: u64 = 0x8018_4540;

const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const UI_DEV_SETUP: u64 = 0x405c_5503;
const UI_ABS_SETUP: u64 = 0x401c_5504;
const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_ABSBIT: u64 = 0x4004_5567;
const UI_SET_MSCBIT: u64 = 0x4004_5568;
const UI_SET_PHYS: u64 = 0x4008_556c;
const UI_SET_PROPBIT: u64 = 0x4004_556e;

const ABS_CODES: [u16; 8] = [
    ABS_X,
    ABS_Y,
    ABS_PRESSURE,
    ABS_MT_SLOT,
    ABS_MT_POSITION_X,
    ABS_MT_POSITION_Y,
    ABS_MT_TRACKING_ID,
    ABS_MT_PRESSURE,
];

const KEY_CODES: [u16; 7] = [
    BTN_LEFT,
    BTN_TOOL_FINGER,
    BTN_TOOL_QUINTTAP,
    BTN_TOUCH,
    BTN_TOOL_DOUBLETAP,
    BTN_TOOL_TRIPLETAP,
    BTN_TOOL_QUADTAP,
];

const POLLIN: i16 = 0x0001;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputEvent {
    pub tv_sec: i64,
    pub tv_usec: i64,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputAbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    _pad: u16,
    absinfo: InputAbsInfo,
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
}

const MAX_SLOTS: usize = 5;

#[derive(Debug, Clone, Copy)]
struct RawSlot {
    tracking_id: i32,
    x: i32,
    y: i32,
    pressure: i32,
    started: bool,
    suppressed: bool,
}

impl Default for RawSlot {
    fn default() -> Self {
        Self {
            tracking_id: -1,
            x: 0,
            y: 0,
            pressure: 0,
            started: false,
            suppressed: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EmittedSlot {
    active: bool,
    tracking_id: i32,
    x: i32,
    y: i32,
    pressure: i32,
}

impl Default for EmittedSlot {
    fn default() -> Self {
        Self {
            active: false,
            tracking_id: -1,
            x: 0,
            y: 0,
            pressure: 0,
        }
    }
}

/// Filters touchpad evdev Type-B multitouch frames so contacts starting inside the bottom
/// strip do not generate pointer motion or taps, while preserving mid-pad swipes and physical clicks.
pub struct DeadZoneFilter {
    y_min: i32,
    y_max: i32,
    y_threshold: i32,
    cur_slot: usize,
    raw_slots: [RawSlot; MAX_SLOTS],
    emitted_slots: [EmittedSlot; MAX_SLOTS],
    emitted_slot: usize,
    emitted_btn_touch: bool,
    emitted_tools_mask: u8,
    emitted_abs_x: Option<i32>,
    emitted_abs_y: Option<i32>,
    emitted_abs_pressure: Option<i32>,
    cur_btn_left: bool,
    emitted_btn_left: bool,
    pending_msc: Option<i32>,
}

impl DeadZoneFilter {
    pub fn new(y_min: i32, y_max: i32, dead_zone_mm: u8) -> Self {
        let mut s = Self {
            y_min,
            y_max,
            y_threshold: i32::MAX,
            cur_slot: 0,
            raw_slots: [RawSlot::default(); MAX_SLOTS],
            emitted_slots: [EmittedSlot::default(); MAX_SLOTS],
            emitted_slot: 0,
            emitted_btn_touch: false,
            emitted_tools_mask: 0,
            emitted_abs_x: None,
            emitted_abs_y: None,
            emitted_abs_pressure: None,
            cur_btn_left: false,
            emitted_btn_left: false,
            pending_msc: None,
        };
        s.set_dead_zone(dead_zone_mm);
        s
    }

    pub fn set_dead_zone(&mut self, dead_zone_mm: u8) {
        if dead_zone_mm == 0 || self.y_max <= self.y_min {
            self.y_threshold = i32::MAX;
        } else {
            let frac = (dead_zone_mm as f64 / TP_PHYSICAL_HEIGHT_MM).clamp(0.0, 0.45);
            let span = (self.y_max - self.y_min) as f64;
            self.y_threshold = self.y_max - (span * frac).round() as i32;
        }
    }

    pub fn process_event(&mut self, ev: &InputEvent) -> Vec<InputEvent> {
        match ev.type_ {
            EV_ABS => match ev.code {
                ABS_MT_SLOT => {
                    self.cur_slot = ev.value.clamp(0, (MAX_SLOTS - 1) as i32) as usize;
                }
                ABS_MT_TRACKING_ID => {
                    let sl = &mut self.raw_slots[self.cur_slot];
                    if ev.value < 0 {
                        sl.tracking_id = -1;
                        sl.started = false;
                        sl.suppressed = false;
                    } else if sl.tracking_id != ev.value {
                        sl.tracking_id = ev.value;
                        sl.started = false;
                        sl.suppressed = false;
                    }
                }
                ABS_MT_POSITION_X => self.raw_slots[self.cur_slot].x = ev.value,
                ABS_MT_POSITION_Y => self.raw_slots[self.cur_slot].y = ev.value,
                ABS_MT_PRESSURE => self.raw_slots[self.cur_slot].pressure = ev.value,
                _ => {}
            },
            EV_KEY if ev.code == BTN_LEFT => {
                self.cur_btn_left = ev.value != 0;
            }
            EV_MSC if ev.code == MSC_TIMESTAMP => {
                self.pending_msc = Some(ev.value);
            }
            EV_SYN if ev.code == SYN_DROPPED => {
                self.raw_slots = [RawSlot::default(); MAX_SLOTS];
                self.cur_btn_left = false;
                self.pending_msc = None;
                return self.flush_frame(ev.tv_sec, ev.tv_usec);
            }
            EV_SYN if ev.code == SYN_REPORT => {
                return self.flush_frame(ev.tv_sec, ev.tv_usec);
            }
            _ => {}
        }
        Vec::new()
    }

    fn flush_frame(&mut self, tv_sec: i64, tv_usec: i64) -> Vec<InputEvent> {
        let mk = |type_: u16, code: u16, value: i32| InputEvent {
            tv_sec,
            tv_usec,
            type_,
            code,
            value,
        };
        let mut out = Vec::new();
        let mut primary: Option<(i32, i32, i32)> = None;
        let mut vis_count = 0usize;

        for i in 0..MAX_SLOTS {
            let r = &mut self.raw_slots[i];
            if r.tracking_id >= 0 && !r.started {
                r.started = true;
                r.suppressed = r.y >= self.y_threshold;
            }
            let vis = r.tracking_id >= 0 && !r.suppressed;
            if vis {
                vis_count += 1;
                if primary.is_none() {
                    primary = Some((r.x, r.y, r.pressure));
                }
            }

            let em = &mut self.emitted_slots[i];
            if em.active && !vis {
                if self.emitted_slot != i {
                    out.push(mk(EV_ABS, ABS_MT_SLOT, i as i32));
                    self.emitted_slot = i;
                }
                out.push(mk(EV_ABS, ABS_MT_TRACKING_ID, -1));
                em.active = false;
                em.tracking_id = -1;
            } else if vis && (!em.active || em.tracking_id != r.tracking_id) {
                if self.emitted_slot != i {
                    out.push(mk(EV_ABS, ABS_MT_SLOT, i as i32));
                    self.emitted_slot = i;
                }
                out.push(mk(EV_ABS, ABS_MT_TRACKING_ID, r.tracking_id));
                out.push(mk(EV_ABS, ABS_MT_POSITION_X, r.x));
                out.push(mk(EV_ABS, ABS_MT_POSITION_Y, r.y));
                out.push(mk(EV_ABS, ABS_MT_PRESSURE, r.pressure));
                em.active = true;
                em.tracking_id = r.tracking_id;
                em.x = r.x;
                em.y = r.y;
                em.pressure = r.pressure;
            } else if vis {
                let mut need_slot = true;
                let mut push_abs = |code: u16, val: i32| {
                    if need_slot && self.emitted_slot != i {
                        out.push(mk(EV_ABS, ABS_MT_SLOT, i as i32));
                        self.emitted_slot = i;
                    }
                    need_slot = false;
                    out.push(mk(EV_ABS, code, val));
                };
                if em.x != r.x {
                    push_abs(ABS_MT_POSITION_X, r.x);
                    em.x = r.x;
                }
                if em.y != r.y {
                    push_abs(ABS_MT_POSITION_Y, r.y);
                    em.y = r.y;
                }
                if em.pressure != r.pressure {
                    push_abs(ABS_MT_PRESSURE, r.pressure);
                    em.pressure = r.pressure;
                }
            }
        }

        if self.cur_btn_left != self.emitted_btn_left {
            out.push(mk(EV_KEY, BTN_LEFT, i32::from(self.cur_btn_left)));
            self.emitted_btn_left = self.cur_btn_left;
        }

        let want_touch = vis_count > 0;
        if want_touch != self.emitted_btn_touch {
            out.push(mk(EV_KEY, BTN_TOUCH, i32::from(want_touch)));
            self.emitted_btn_touch = want_touch;
        }

        let want_tools: u8 = match vis_count {
            0 => 0,
            1 => 1 << 0,
            2 => 1 << 1,
            3 => 1 << 2,
            4 => 1 << 3,
            _ => 1 << 4,
        };
        let tool_codes = [
            BTN_TOOL_FINGER,
            BTN_TOOL_DOUBLETAP,
            BTN_TOOL_TRIPLETAP,
            BTN_TOOL_QUADTAP,
            BTN_TOOL_QUINTTAP,
        ];
        if want_tools != self.emitted_tools_mask {
            for (bit, &code) in tool_codes.iter().enumerate() {
                let was = (self.emitted_tools_mask >> bit) & 1 != 0;
                let now = (want_tools >> bit) & 1 != 0;
                if was != now {
                    out.push(mk(EV_KEY, code, i32::from(now)));
                }
            }
            self.emitted_tools_mask = want_tools;
        }

        if let Some((px, py, pp)) = primary {
            if self.emitted_abs_x != Some(px) {
                out.push(mk(EV_ABS, ABS_X, px));
                self.emitted_abs_x = Some(px);
            }
            if self.emitted_abs_y != Some(py) {
                out.push(mk(EV_ABS, ABS_Y, py));
                self.emitted_abs_y = Some(py);
            }
            if self.emitted_abs_pressure != Some(pp) {
                out.push(mk(EV_ABS, ABS_PRESSURE, pp));
                self.emitted_abs_pressure = Some(pp);
            }
        } else {
            self.emitted_abs_x = None;
            self.emitted_abs_y = None;
            self.emitted_abs_pressure = None;
        }

        let msc = self.pending_msc.take();
        if !out.is_empty() {
            if let Some(ts) = msc {
                out.push(mk(EV_MSC, MSC_TIMESTAMP, ts));
            }
            out.push(mk(EV_SYN, SYN_REPORT, 0));
        }
        out
    }
}

pub fn probe_tp_evdev_error() -> Option<String> {
    let Some(ev_name) = find_raw_touchpad_event() else {
        return Some("BLTP7853 Touchpad (/dev/input/event*) не найден".into());
    };
    let path = format!("/dev/input/{ev_name}");
    if let Err(e) = File::open(&path) {
        return Some(format!("{path}: {e}"));
    }
    if let Err(e) = OpenOptions::new().write(true).open("/dev/uinput") {
        return Some(format!("/dev/uinput: {e}"));
    }
    None
}

pub struct GrabbedTouchpadBridge {
    raw_file: File,
    uinput_file: File,
    pub filter: DeadZoneFilter,
}

impl GrabbedTouchpadBridge {
    pub fn open(dead_zone_mm: u8) -> Result<Self, String> {
        let ev_name = find_raw_touchpad_event()
            .ok_or_else(|| "BLTP7853 Touchpad (/dev/input/event*) не найден".to_string())?;
        let raw_path = format!("/dev/input/{ev_name}");
        let raw_file = OpenOptions::new()
            .read(true)
            .custom_flags(0x800) // O_NONBLOCK on Linux x86_64
            .open(&raw_path)
            .map_err(|e| format!("{raw_path}: {e}"))?;

        let raw_fd = raw_file.as_raw_fd();
        let mut dev_id = InputId::default();
        if unsafe { ioctl(raw_fd, EVIOCGID, &mut dev_id as *mut InputId) } < 0 {
            return Err(format!("EVIOCGID failed: {}", io::Error::last_os_error()));
        }

        let mut abs_infos = Vec::with_capacity(ABS_CODES.len());
        let mut y_min = 0i32;
        let mut y_max = 2124i32;
        for &code in &ABS_CODES {
            let mut info = InputAbsInfo::default();
            if unsafe {
                ioctl(
                    raw_fd,
                    EVIOCGABS_BASE + code as u64,
                    &mut info as *mut InputAbsInfo,
                )
            } < 0
            {
                return Err(format!(
                    "EVIOCGABS({code}) failed: {}",
                    io::Error::last_os_error()
                ));
            }
            if code == ABS_MT_POSITION_Y {
                y_min = info.minimum;
                y_max = info.maximum;
            }
            abs_infos.push((code, info));
        }

        let uinput_file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| format!("/dev/uinput: {e}"))?;
        let ufd = uinput_file.as_raw_fd();

        for ev in [EV_SYN, EV_KEY, EV_ABS, EV_MSC] {
            if unsafe { ioctl(ufd, UI_SET_EVBIT, ev as i32) } < 0 {
                return Err(format!("UI_SET_EVBIT: {}", io::Error::last_os_error()));
            }
        }
        for &k in &KEY_CODES {
            if unsafe { ioctl(ufd, UI_SET_KEYBIT, k as i32) } < 0 {
                return Err(format!("UI_SET_KEYBIT: {}", io::Error::last_os_error()));
            }
        }
        if unsafe { ioctl(ufd, UI_SET_MSCBIT, MSC_TIMESTAMP as i32) } < 0 {
            return Err(format!("UI_SET_MSCBIT: {}", io::Error::last_os_error()));
        }
        let phys = b"i2c-BLTP7853:00\0";
        let _ = unsafe { ioctl(ufd, UI_SET_PHYS, phys.as_ptr()) };
        for prop in [INPUT_PROP_POINTER, INPUT_PROP_BUTTONPAD, INPUT_PROP_PRESSUREPAD] {
            if unsafe { ioctl(ufd, UI_SET_PROPBIT, prop) } < 0 {
                return Err(format!("UI_SET_PROPBIT: {}", io::Error::last_os_error()));
            }
        }
        for (code, info) in abs_infos {
            if unsafe { ioctl(ufd, UI_SET_ABSBIT, code as i32) } < 0 {
                return Err(format!("UI_SET_ABSBIT: {}", io::Error::last_os_error()));
            }
            let abs_setup = UinputAbsSetup {
                code,
                _pad: 0,
                absinfo: info,
            };
            if unsafe { ioctl(ufd, UI_ABS_SETUP, &abs_setup as *const UinputAbsSetup) } < 0 {
                return Err(format!("UI_ABS_SETUP: {}", io::Error::last_os_error()));
            }
        }

        let mut name = [0u8; 80];
        let vname = b"BLTP7853:00 347D:7853 Touchpad";
        name[..vname.len()].copy_from_slice(vname);
        let setup = UinputSetup {
            id: dev_id,
            name,
            ff_effects_max: 0,
        };
        if unsafe { ioctl(ufd, UI_DEV_SETUP, &setup as *const UinputSetup) } < 0 {
            return Err(format!("UI_DEV_SETUP: {}", io::Error::last_os_error()));
        }
        if unsafe { ioctl(ufd, UI_DEV_CREATE) } < 0 {
            return Err(format!("UI_DEV_CREATE: {}", io::Error::last_os_error()));
        }
        if unsafe { ioctl(raw_fd, EVIOCGRAB, 1i32) } < 0 {
            unsafe { ioctl(ufd, UI_DEV_DESTROY) };
            return Err(format!("EVIOCGRAB failed: {}", io::Error::last_os_error()));
        }

        Ok(Self {
            raw_file,
            uinput_file,
            filter: DeadZoneFilter::new(y_min, y_max, dead_zone_mm),
        })
    }

    pub fn poll_and_forward(&mut self, timeout_ms: i32) -> Result<(), String> {
        let mut pfd = PollFd {
            fd: self.raw_file.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let ret = unsafe { poll(&mut pfd as *mut PollFd, 1, timeout_ms) };
        if ret <= 0 {
            return Ok(());
        }

        let mut buf = [0u8; std::mem::size_of::<InputEvent>() * 64];
        loop {
            match self.raw_file.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let sz = std::mem::size_of::<InputEvent>();
                    let count = n / sz;
                    let mut out_bytes = Vec::with_capacity(n);
                    for i in 0..count {
                        let ev = unsafe {
                            std::ptr::read_unaligned(
                                buf[i * sz..(i + 1) * sz].as_ptr() as *const InputEvent
                            )
                        };
                        for fev in self.filter.process_event(&ev) {
                            let bytes = unsafe {
                                std::slice::from_raw_parts(
                                    &fev as *const InputEvent as *const u8,
                                    sz,
                                )
                            };
                            out_bytes.extend_from_slice(bytes);
                        }
                    }
                    if !out_bytes.is_empty() {
                        self.uinput_file
                            .write_all(&out_bytes)
                            .map_err(|e| format!("uinput write: {e}"))?;
                    }
                }
                Ok(_) => return Err("Touchpad evdev EOF".into()),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(format!("Touchpad evdev read: {e}")),
            }
        }
    }
}

impl Drop for GrabbedTouchpadBridge {
    fn drop(&mut self) {
        unsafe {
            ioctl(self.raw_file.as_raw_fd(), EVIOCGRAB, 0i32);
            ioctl(self.uinput_file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}
