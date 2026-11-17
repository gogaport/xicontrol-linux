use crate::actions::{self, SharedHandle};
use crate::config::{AppConfig, KeyAction};
use crate::http_api;
use crate::hw::sys;
use crate::hw::touchpad::{
    self, HeavyPressDetector, TouchpadEdge, TouchpadEdgeGesture,
};
use crate::hw::touchpad_dz::GrabbedTouchpadBridge;
use crate::tray;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

static DZ_ENABLED: AtomicBool = AtomicBool::new(false);
static DZ_MM: AtomicU8 = AtomicU8::new(12);
static DAEMON_STOP: AtomicBool = AtomicBool::new(false);
static IS_DAEMON: AtomicBool = AtomicBool::new(false);

pub fn is_daemon_process() -> bool {
    IS_DAEMON.load(Ordering::Relaxed)
}

fn sync_dz_atomics(shared: &SharedHandle) {
    let s = shared.lock().unwrap();
    DZ_ENABLED.store(s.config.touchpad_dead_zone_enabled, Ordering::Relaxed);
    DZ_MM.store(s.config.touchpad_dead_zone_mm, Ordering::Relaxed);
    DAEMON_STOP.store(s.shutdown, Ordering::Relaxed);
}

pub fn ipc_socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("xicontrol.sock");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    PathBuf::from(format!("/tmp/xicontrol-{user}.sock"))
}

pub fn send_ipc_command(cmd: &str) -> Result<String, String> {
    let sock = ipc_socket_path();
    let mut stream =
        UnixStream::connect(&sock).map_err(|e| format!("Демон не запущен ({e})"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    writeln!(stream, "{cmd}").map_err(|e| format!("IPC write: {e}"))?;
    let _ = stream.flush();
    let mut resp = String::new();
    let _ = stream.read_to_string(&mut resp);
    Ok(resp.trim().to_string())
}

pub fn notify_daemon_reload() {
    if !is_daemon_process() {
        let _ = send_ipc_command("RELOAD_CONFIG");
    }
}

pub fn notify_expected_hz(hz: u32) {
    if !is_daemon_process() {
        let _ = send_ipc_command(&format!("EXPECTED_HZ {hz}"));
    }
}

fn start_ipc_listener(shared: SharedHandle) {
    let sock = ipc_socket_path();
    if sock.exists() {
        if UnixStream::connect(&sock).is_ok() {
            return;
        }
        let _ = fs::remove_file(&sock);
    }
    let Ok(listener) = UnixListener::bind(&sock) else { return; };
    let _ = listener.set_nonblocking(true);

    std::thread::spawn(move || {
        loop {
            if shared.lock().unwrap().shutdown {
                let _ = fs::remove_file(&sock);
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut line = String::new();
                    let ok = {
                        let Ok(cloned) = stream.try_clone() else { continue; };
                        let mut reader = BufReader::new(cloned);
                        reader.read_line(&mut line).is_ok()
                    };
                    if !ok {
                        continue;
                    }
                    let cmd = line.trim();
                    let reply = handle_ipc_line(&shared, cmd);
                    let _ = writeln!(stream, "{reply}");
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(120));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    });
}

fn handle_ipc_line(shared: &SharedHandle, cmd: &str) -> String {
    if cmd == "PING" {
        return "PONG".into();
    }
    if cmd == "SHOW_GUI" {
        shared.lock().unwrap().gui_requested = true;
        return "OK".into();
    }
    if cmd == "QUIT" {
        shared.lock().unwrap().shutdown = true;
        DAEMON_STOP.store(true, Ordering::Relaxed);
        return "OK".into();
    }
    if cmd == "RELOAD_CONFIG" {
        let new_cfg = AppConfig::load();
        {
            let mut s = shared.lock().unwrap();
            let owl_target = new_cfg.owl_mode;
            s.config = new_cfg;
            if s.owl_active != owl_target {
                let _ = s.owl.set_active(owl_target);
                s.owl_active = s.owl.is_active();
            }
        }
        sync_dz_atomics(shared);
        return "OK".into();
    }
    if cmd == "DIAG" {
        let s = shared.lock().unwrap();
        let wmi = s.wmi_input_error.as_deref().unwrap_or("");
        let tp_ev = s.tp_evdev_error.as_deref().unwrap_or("");
        return format!("{}|{}|{}|{}", s.live_tp_pressure, s.peak_tp_pressure, wmi, tp_ev);
    }
    if cmd == "RESET_TP_PEAK" {
        let mut s = shared.lock().unwrap();
        s.peak_tp_pressure = s.live_tp_pressure;
        return "OK".into();
    }
    if let Some(hz_str) = cmd.strip_prefix("EXPECTED_HZ ") {
        if let Ok(hz) = hz_str.trim().parse::<u32>() {
            shared.lock().unwrap().expected_hz = Some(hz);
            return "OK".into();
        }
        return "ERR invalid_hz".into();
    }
    if let Some(slot) = cmd.strip_prefix("TRIGGER ") {
        return match actions::trigger_slot(shared, slot.trim()) {
            Ok(()) => "OK".into(),
            Err(e) => format!("ERR {e}"),
        };
    }
    "ERR unknown_command".into()
}

fn start_touchpad_hidraw_thread(shared: SharedHandle) {
    enum EdgeWork {
        Brightness(i32),
        Volume(i32),
        Pulse,
    }
    let (work_tx, work_rx) = std::sync::mpsc::channel::<EdgeWork>();
    std::thread::spawn(move || {
        while let Ok(first) = work_rx.recv() {
            let mut b_steps = 0i32;
            let mut v_steps = 0i32;
            let mut do_pulse = false;
            let mut handle_ev = |ev: EdgeWork| match ev {
                EdgeWork::Brightness(d) => b_steps += d,
                EdgeWork::Volume(d) => v_steps += d,
                EdgeWork::Pulse => do_pulse = true,
            };
            handle_ev(first);
            while let Ok(more) = work_rx.try_recv() {
                handle_ev(more);
            }
            if do_pulse {
                let _ = touchpad::pulse_touchpad();
            }
            if b_steps != 0 {
                sys::step_brightness(b_steps);
            }
            if v_steps != 0 {
                sys::step_volume(v_steps);
            }
        }
    });

    std::thread::spawn(move || {
        loop {
            if shared.lock().unwrap().shutdown {
                break;
            }
            let Some(path) = touchpad::find_bltp7853_hidraw() else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            let Ok(mut file) = File::open(&path) else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };

            let (w_mm, swipes, dz_mm, dz_on, hp_thresh) = {
                let s = shared.lock().unwrap();
                (
                    s.config.touchpad_edge_width_mm,
                    s.config.touchpad_edge_swipes_per_range,
                    if s.config.touchpad_dead_zone_enabled { s.config.touchpad_dead_zone_mm } else { 0 },
                    s.config.touchpad_dead_zone_enabled,
                    s.config.touchpad_heavy_press_threshold,
                )
            };
            let mut prev_cfg = (w_mm, swipes, dz_mm, dz_on);
            let mut gesture = TouchpadEdgeGesture::new(w_mm, swipes, dz_mm);
            let mut heavy = HeavyPressDetector::new(hp_thresh);
            let mut last_pulse = Instant::now() - Duration::from_secs(1);
            let mut buf = [0u8; 64];

            loop {
                let n = match file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if shared.lock().unwrap().shutdown {
                    return;
                }
                let Some(contacts) = touchpad::parse_hid_report_04(&buf[..n]) else {
                    continue;
                };

                let max_p = contacts.iter().map(|c| c.pressure).max().unwrap_or(0);
                let (
                    edge_on,
                    cur_w,
                    cur_swipes,
                    cur_dz_mm,
                    cur_dz_on,
                    pulse_on,
                    pulse_ms,
                    cur_hp_thresh,
                    hp_action,
                ) = {
                    let mut s = shared.lock().unwrap();
                    s.live_tp_pressure = max_p;
                    if max_p > s.peak_tp_pressure {
                        s.peak_tp_pressure = max_p;
                    }
                    let c = &s.config;
                    (
                        c.touchpad_edge_sliders,
                        c.touchpad_edge_width_mm,
                        c.touchpad_edge_swipes_per_range,
                        if c.touchpad_dead_zone_enabled { c.touchpad_dead_zone_mm } else { 0 },
                        c.touchpad_dead_zone_enabled,
                        c.touchpad_edge_pulse,
                        c.touchpad_edge_pulse_ms,
                        c.touchpad_heavy_press_threshold,
                        c.action_heavy_press,
                    )
                };

                if prev_cfg != (cur_w, cur_swipes, cur_dz_mm, cur_dz_on) {
                    prev_cfg = (cur_w, cur_swipes, cur_dz_mm, cur_dz_on);
                    gesture = TouchpadEdgeGesture::new(cur_w, cur_swipes, cur_dz_mm);
                }
                heavy.threshold = cur_hp_thresh;

                if hp_action != KeyAction::None && heavy.update(&contacts) {
                    let sh = shared.clone();
                    let _ = work_tx.send(EdgeWork::Pulse);
                    std::thread::spawn(move || {
                        let _ = actions::execute_action(&sh, hp_action);
                    });
                }

                if edge_on {
                    let (edge, steps) = gesture.update(&contacts);
                    if steps != 0 {
                        match edge {
                            TouchpadEdge::Left => { let _ = work_tx.send(EdgeWork::Brightness(steps * 2)); }
                            TouchpadEdge::Right => { let _ = work_tx.send(EdgeWork::Volume(steps)); }
                            TouchpadEdge::None => {}
                        }
                        if pulse_on && last_pulse.elapsed() >= Duration::from_millis(pulse_ms.max(15)) {
                            last_pulse = Instant::now();
                            let _ = work_tx.send(EdgeWork::Pulse);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn find_wmi_keys_event() -> Option<String> {
    let entries = fs::read_dir("/sys/class/input").ok()?;
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("event") {
            continue;
        }
        let name_path = format!("/sys/class/input/{fname}/device/name");
        if let Ok(name) = fs::read_to_string(&name_path) {
            if name.contains("Redmibook WMI") || name.contains("Xiaomi WMI") {
                return Some(format!("/dev/input/{fname}"));
            }
        }
    }
    None
}

pub fn probe_wmi_input_error() -> Option<String> {
    let Some(path) = find_wmi_keys_event() else {
        return Some("Redmibook WMI keys (/dev/input/event*) не найдено".into());
    };
    match File::open(&path) {
        Ok(_) => None,
        Err(e) => Some(format!("{path}: {e}")),
    }
}

fn start_wmi_hotkey_thread(shared: SharedHandle) {
    std::thread::spawn(move || {
        loop {
            if shared.lock().unwrap().shutdown {
                break;
            }
            let Some(path) = find_wmi_keys_event() else {
                shared.lock().unwrap().wmi_input_error =
                    Some("Redmibook WMI keys (/dev/input/event*) не найдено".into());
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            let mut file = match File::open(&path) {
                Ok(f) => {
                    shared.lock().unwrap().wmi_input_error = None;
                    f
                }
                Err(e) => {
                    shared.lock().unwrap().wmi_input_error = Some(format!("{path}: {e}"));
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            };
            let mut ev = [0u8; 24]; // 64-bit Linux struct input_event = 24 bytes
            loop {
                if shared.lock().unwrap().shutdown {
                    return;
                }
                if file.read_exact(&mut ev).is_err() {
                    break;
                }
                let ev_type = u16::from_le_bytes([ev[16], ev[17]]);
                let code = u16::from_le_bytes([ev[18], ev[19]]);
                let value = i32::from_le_bytes([ev[20], ev[21], ev[22], ev[23]]);
                if ev_type != 1 || value != 1 {
                    continue;
                }
                // Redmibook WMI keys (verified via capabilities/key):
                // KEY_PERFORMANCE=701, KEY_REFRESH_RATE_TOGGLE=562, KEY_ASSISTANT=583,
                // KEY_VENDOR=360, KEY_CONFIG=171, KEY_TOUCHPAD_TOGGLE=530
                let slot = match code {
                    701 | 148 => Some("wmi_perf"),
                    562 | 149 => Some("wmi_hz"),
                    530 | 191 => Some("wmi_tp"),
                    583 | 192 | 193 => Some("custom_1"),
                    360 | 171 | 194 => Some("custom_2"),
                    _ => None,
                };
                if let Some(slot_name) = slot {
                    let sh = shared.clone();
                    std::thread::spawn(move || {
                        let _ = actions::trigger_slot(&sh, slot_name);
                    });
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn spawn_gui_child(child_slot: &mut Option<Child>) {
    if let Some(ch) = child_slot {
        match ch.try_wait() {
            Ok(None) => return, // Still open
            _ => *child_slot = None,
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(ch) = Command::new(exe).arg("gui-window").spawn() {
            *child_slot = Some(ch);
        }
    }
}

fn start_touchpad_deadzone_thread(shared: SharedHandle) {
    sync_dz_atomics(&shared);
    std::thread::spawn(move || {
        loop {
            if DAEMON_STOP.load(Ordering::Relaxed) {
                break;
            }
            let enabled = DZ_ENABLED.load(Ordering::Relaxed);
            let dz_mm = DZ_MM.load(Ordering::Relaxed);
            if !enabled || dz_mm == 0 {
                shared.lock().unwrap().tp_evdev_error = None;
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            let mut bridge = match GrabbedTouchpadBridge::open(dz_mm) {
                Ok(b) => {
                    shared.lock().unwrap().tp_evdev_error = None;
                    b
                }
                Err(e) => {
                    shared.lock().unwrap().tp_evdev_error = Some(e);
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };
            loop {
                if DAEMON_STOP.load(Ordering::Relaxed)
                    || !DZ_ENABLED.load(Ordering::Relaxed)
                    || DZ_MM.load(Ordering::Relaxed) == 0
                {
                    drop(bridge);
                    break;
                }
                bridge.filter.set_dead_zone(DZ_MM.load(Ordering::Relaxed));
                if let Err(e) = bridge.poll_and_forward(150) {
                    shared.lock().unwrap().tp_evdev_error = Some(e);
                    drop(bridge);
                    std::thread::sleep(Duration::from_secs(2));
                    break;
                }
            }
        }
    });
}

pub fn start_background_services(shared: &SharedHandle) {
    IS_DAEMON.store(true, Ordering::Relaxed);
    start_ipc_listener(shared.clone());

    actions::sync_power_state(shared, true);
    actions::check_charge_guard(shared);
    actions::refresh_slow(shared);

    tray::start_tray_controller(shared.clone());
    http_api::start_http_api_thread(shared.clone());
    start_touchpad_hidraw_thread(shared.clone());
    start_touchpad_deadzone_thread(shared.clone());
    start_wmi_hotkey_thread(shared.clone());
}

pub fn run_daemon_loop(shared: SharedHandle) {
    start_background_services(&shared);

    let mut gui_child: Option<Child> = None;
    let mut tick = 0u64;

    loop {
        if shared.lock().unwrap().shutdown {
            break;
        }

        let need_gui = {
            let mut s = shared.lock().unwrap();
            let req = s.gui_requested;
            s.gui_requested = false;
            req
        };
        if need_gui {
            spawn_gui_child(&mut gui_child);
        }

        actions::refresh_fast(&shared);
        actions::sync_power_state(&shared, false);

        if tick % 10 == 0 {
            actions::refresh_slow(&shared);
        }
        if tick % 30 == 0 {
            actions::check_charge_guard(&shared);
        }

        tick = tick.wrapping_add(1);
        std::thread::sleep(Duration::from_secs(1));
    }

    if let Some(mut ch) = gui_child {
        let _ = ch.kill();
    }
}
