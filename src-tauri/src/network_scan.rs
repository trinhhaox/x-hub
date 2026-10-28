// Discovery of devices on the same local network:
// - Android: adb services advertised over Bonjour (Wireless Debugging or `adb tcpip`)
// - iOS: devices already paired with this Mac that CoreDevice sees over the LAN
use serde::Serialize;
use std::process::Command;
use std::time::Duration;

use super::{bonjour, get_enhanced_path, is_safe_identifier, resolve_bin, scan_ios_physical_devices};

#[derive(Debug, Serialize, Clone)]
pub struct NetworkDevice {
    pub id: String,
    pub name: String,
    pub platform: String, // "iOS" | "Android"
    pub address: String,  // "ip:port" for Android, UDID for iOS
    pub kind: String,     // "connect" | "pairing"
    pub connected: bool,
}

fn run_adb(args: &[&str]) -> Result<String, String> {
    let output = Command::new(resolve_bin("adb"))
        .env("PATH", get_enhanced_path())
        .args(args)
        .output()
        .map_err(|e| format!("Không chạy được adb: {}", e))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

const ADB_SERVICES: [(&str, &str); 3] = [
    ("_adb-tls-connect._tcp", "connect"),
    ("_adb-tls-pairing._tcp", "pairing"),
    ("_adb._tcp", "connect"), // legacy `adb tcpip` mode
];

/// Instances look like "adb-<serial>" or "adb-<serial>-<random>" (Wireless Debugging)
fn display_name(instance: &str) -> String {
    let rest = instance.strip_prefix("adb-").unwrap_or(instance);
    rest.split('-').next().unwrap_or(rest).to_string()
}

fn connected_adb_serials() -> Vec<String> {
    run_adb(&["devices"])
        .map(|out| {
            out.lines()
                .skip(1)
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    match (it.next(), it.next()) {
                        (Some(serial), Some("device")) => Some(serial.to_string()),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn scan_android_network() -> Vec<NetworkDevice> {
    let browsers: Vec<_> = ADB_SERVICES
        .iter()
        .map(|&(service, kind)| {
            std::thread::spawn(move || {
                let names = bonjour::browse(service, Duration::from_millis(2000));
                // Resolve all instances of this service in parallel
                let resolvers: Vec<_> = names
                    .into_iter()
                    .map(|name| {
                        std::thread::spawn(move || {
                            bonjour::resolve(&name, service, Duration::from_millis(2500)).map(|addr| (name, addr))
                        })
                    })
                    .collect();
                resolvers
                    .into_iter()
                    .filter_map(|r| r.join().ok().flatten())
                    .map(|(name, addr)| (name, addr.to_string(), kind))
                    .collect::<Vec<_>>()
            })
        })
        .collect();

    let serials = connected_adb_serials();
    let mut list: Vec<NetworkDevice> = Vec::new();
    for (instance, address, kind) in browsers.into_iter().filter_map(|b| b.join().ok()).flatten() {
        // A phone may advertise both tls-connect and legacy _adb._tcp for the same address
        if list.iter().any(|d| d.address == address) {
            continue;
        }
        list.push(NetworkDevice {
            id: format!("{}|{}", instance, kind),
            name: display_name(&instance),
            platform: "Android".to_string(),
            connected: kind == "connect" && serials.iter().any(|s| *s == address || s.starts_with(&instance)),
            address,
            kind: kind.to_string(),
        });
    }
    list
}

fn scan_ios_network() -> Vec<NetworkDevice> {
    scan_ios_physical_devices()
        .into_iter()
        .filter(|d| d.transport_type == "Wireless")
        .map(|d| NetworkDevice {
            id: d.udid.clone(),
            name: d.name,
            platform: "iOS".to_string(),
            address: d.udid,
            kind: "connect".to_string(),
            connected: d.state == "connected",
        })
        .collect()
}

#[tauri::command(async)]
pub fn scan_network_devices() -> Vec<NetworkDevice> {
    let ios = std::thread::spawn(scan_ios_network);
    let mut all = scan_android_network();
    all.extend(ios.join().unwrap_or_default());
    all
}

#[tauri::command(async)]
pub fn pair_adb_device(address: String, code: String) -> Result<String, String> {
    if !is_safe_identifier(&address) {
        return Err("Địa chỉ ghép đôi không hợp lệ".to_string());
    }
    let code = code.trim();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("Mã ghép đôi phải gồm 6 chữ số".to_string());
    }
    let out = run_adb(&["pair", address.trim(), code])?;
    if out.contains("Successfully paired") {
        Ok(out.trim().to_string())
    } else {
        Err(out.trim().to_string())
    }
}

/// Opens a CoreDevice tunnel to a paired iOS device over Wi-Fi.
#[tauri::command(async)]
pub fn connect_ios_device(udid: String) -> Result<String, String> {
    if !is_safe_identifier(&udid) {
        return Err("UDID không hợp lệ".to_string());
    }
    let output = Command::new("xcrun")
        .args(["devicectl", "device", "info", "details", "--device", udid.trim(), "--timeout", "20"])
        .output()
        .map_err(|e| format!("Không chạy được devicectl: {}", e))?;
    if output.status.success() {
        Ok("Đã kết nối thiết bị iOS qua mạng".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("Kết nối thất bại").trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_name_from_instance() {
        assert_eq!(display_name("adb-R58M12AB-xYz12a"), "R58M12AB");
        assert_eq!(display_name("adb-056371405R100461"), "056371405R100461");
        assert_eq!(display_name("printer"), "printer");
    }
}
