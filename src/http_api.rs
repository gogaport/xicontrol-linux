use crate::actions::{
    apply_charge_limit, set_charge_care, set_owl_mode, set_perf_mode_checked, set_travel_mode,
    SharedHandle,
};
use crate::config::{fixed_time_eq, sha256_hex};
use crate::hw::mifs::PerfMode;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const MAX_BODY_BYTES: usize = 4096;

pub fn handle_http_request(
    shared: &SharedHandle,
    method: &str,
    path: &str,
    auth_header: Option<&str>,
    body: &str,
) -> (u16, &'static str, String) {
    let api_cfg = { shared.lock().unwrap().config.http_api.clone() };
    if !api_cfg.enabled {
        return (503, "Service Unavailable", json!({"error":"http_api_disabled"}).to_string());
    }

    // Check Bearer token when a hash is configured (always required for POST)
    let token_ok = if api_cfg.token_sha256.is_empty() {
        method.eq_ignore_ascii_case("GET")
    } else if let Some(auth) = auth_header {
        let token = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .unwrap_or("")
            .trim();
        let digest = sha256_hex(token.as_bytes());
        fixed_time_eq(digest.as_bytes(), api_cfg.token_sha256.as_bytes())
    } else {
        false
    };

    if !token_ok {
        return (401, "Unauthorized", json!({"error":"unauthorized"}).to_string());
    }

    let clean_path = path.split('?').next().unwrap_or(path);
    match (method.to_ascii_uppercase().as_str(), clean_path) {
        ("GET", "/status") => {
            let s = shared.lock().unwrap();
            let payload = json!({
                "perf_mode": s.perf_mode.map(|m| m.api_name()),
                "charge_limit": s.charge_limit,
                "charge_care_enabled": s.config.charge_care_enabled,
                "charge_care_percent": s.config.charge_care_percent,
                "travel_mode": s.config.travel_mode,
                "owl_mode": s.owl_active,
                "ac_online": s.telemetry.ac_online,
                "bat_capacity": s.telemetry.bat_capacity,
                "bat_status": s.telemetry.bat_status,
                "bat_watts": s.telemetry.bat_watts,
                "cpu_pct": s.telemetry.cpu_pct,
                "cpu_temp_c": s.telemetry.cpu_temp_c,
                "cpu_watts": s.telemetry.cpu_watts,
                "gpu_pct": s.telemetry.gpu_pct,
                "npu_pct": s.telemetry.npu_pct,
                "ram_pct": s.telemetry.ram_pct,
                "display_hz": s.display.as_ref().map(|d| d.current_hz),
                "brightness_pct": s.brightness_pct,
            });
            (200, "OK", payload.to_string())
        }
        ("POST", "/mode") => {
            if !api_cfg.allow_mode {
                return (403, "Forbidden", json!({"error":"mode_not_allowed"}).to_string());
            }
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return (400, "Bad Request", json!({"error":"invalid_json"}).to_string());
            };
            let Some(name) = v.get("mode").and_then(|x| x.as_str()) else {
                return (400, "Bad Request", json!({"error":"missing_mode"}).to_string());
            };
            let Some(mode) = PerfMode::from_name(name) else {
                return (400, "Bad Request", json!({"error":"unknown_mode"}).to_string());
            };
            match set_perf_mode_checked(shared, mode) {
                Ok(()) => (200, "OK", json!({"ok":true,"mode":mode.api_name()}).to_string()),
                Err(e) => (500, "Internal Server Error", json!({"error":e}).to_string()),
            }
        }
        ("POST", "/care") => {
            if !api_cfg.allow_care {
                return (403, "Forbidden", json!({"error":"care_not_allowed"}).to_string());
            }
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return (400, "Bad Request", json!({"error":"invalid_json"}).to_string());
            };
            if let Some(pct) = v.get("percent").and_then(|x| x.as_u64()) {
                let pct = pct as u8;
                return match apply_charge_limit(shared, pct, Some(false)) {
                    Ok(()) => (200, "OK", json!({"ok":true,"charge_limit":pct}).to_string()),
                    Err(e) => (400, "Bad Request", json!({"error":e}).to_string()),
                };
            }
            if let Some(en) = v.get("enabled").and_then(|x| x.as_bool()) {
                return match set_charge_care(shared, en) {
                    Ok(pct) => (200, "OK", json!({"ok":true,"enabled":en,"charge_limit":pct}).to_string()),
                    Err(e) => (500, "Internal Server Error", json!({"error":e}).to_string()),
                };
            }
            (400, "Bad Request", json!({"error":"missing_enabled_or_percent"}).to_string())
        }
        ("POST", "/travel") => {
            if !api_cfg.allow_travel {
                return (403, "Forbidden", json!({"error":"travel_not_allowed"}).to_string());
            }
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return (400, "Bad Request", json!({"error":"invalid_json"}).to_string());
            };
            let Some(en) = v.get("enabled").and_then(|x| x.as_bool()) else {
                return (400, "Bad Request", json!({"error":"missing_enabled"}).to_string());
            };
            match set_travel_mode(shared, en) {
                Ok(()) => (200, "OK", json!({"ok":true,"travel_mode":en}).to_string()),
                Err(e) => (500, "Internal Server Error", json!({"error":e}).to_string()),
            }
        }
        ("POST", "/owl") => {
            if !api_cfg.allow_owl {
                return (403, "Forbidden", json!({"error":"owl_not_allowed"}).to_string());
            }
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return (400, "Bad Request", json!({"error":"invalid_json"}).to_string());
            };
            let Some(en) = v.get("enabled").and_then(|x| x.as_bool()) else {
                return (400, "Bad Request", json!({"error":"missing_enabled"}).to_string());
            };
            match set_owl_mode(shared, en) {
                Ok(()) => (200, "OK", json!({"ok":true,"owl_mode":en}).to_string()),
                Err(e) => (500, "Internal Server Error", json!({"error":e}).to_string()),
            }
        }
        _ => (404, "Not Found", json!({"error":"not_found"}).to_string()),
    }
}

fn serve_connection(shared: &SharedHandle, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let Ok(cloned) = stream.try_clone() else { return; };
    let mut reader = BufReader::new(cloned);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut auth_header = None;
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.eq_ignore_ascii_case("Authorization") {
                auth_header = Some(v.trim().to_string());
            } else if k.eq_ignore_ascii_case("Content-Length") {
                content_len = v.trim().parse::<usize>().unwrap_or(0).min(MAX_BODY_BYTES);
            }
        }
    }

    let mut body_buf = vec![0u8; content_len];
    if content_len > 0 && reader.read_exact(&mut body_buf).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body_buf);
    let (status, reason, json_body) =
        handle_http_request(shared, &method, &path, auth_header.as_deref(), &body);

    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json_body}",
        json_body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

pub fn start_http_api_thread(shared: SharedHandle) {
    std::thread::spawn(move || {
        let mut listener: Option<(TcpListener, String)> = None;
        loop {
            let (enabled, addr, shutdown) = {
                let s = shared.lock().unwrap();
                let host = if s.config.http_api.lan_access { "0.0.0.0" } else { "127.0.0.1" };
                let port = if s.config.http_api.port == 0 { 58125 } else { s.config.http_api.port };
                (s.config.http_api.enabled, format!("{host}:{port}"), s.shutdown)
            };
            if shutdown {
                break;
            }
            if !enabled {
                listener = None;
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            let needs_rebind = match &listener {
                Some((_, bound)) => bound != &addr,
                None => true,
            };
            if needs_rebind {
                listener = TcpListener::bind(&addr).ok().map(|l| {
                    let _ = l.set_nonblocking(true);
                    (l, addr.clone())
                });
                if listener.is_none() {
                    let mut s = shared.lock().unwrap();
                    s.last_error = Some(format!("HTTP API bind error on {addr}"));
                    drop(s);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }

            if let Some((ref l, _)) = listener {
                match l.accept() {
                    Ok((stream, _)) => serve_connection(&shared, stream),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(120));
                    }
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        }
    });
}
