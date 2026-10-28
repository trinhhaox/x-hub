use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

mod adb_wifi;
mod android_mirror;
mod bonjour;
mod ios_mirror;
mod network_scan;

fn get_enhanced_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let default_paths = format!(
        "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:{}/Library/Android/sdk/platform-tools:/usr/bin:/bin:/usr/sbin:/sbin",
        home
    );
    if let Ok(existing) = std::env::var("PATH") {
        format!("{}:{}", default_paths, existing)
    } else {
        default_paths
    }
}

fn resolve_bin(name: &str) -> String {
    // 1. Check in standard Homebrew paths (Apple Silicon & Intel)
    let candidates = [
        format!("/opt/homebrew/bin/{}", name),
        format!("/usr/local/bin/{}", name),
        format!("{}/Library/Android/sdk/platform-tools/{}", std::env::var("HOME").unwrap_or_default(), name),
    ];

    for path in &candidates {
        if Path::new(path).exists() {
            return path.clone();
        }
    }

    // 2. Try 'which' with enhanced path
    let enhanced_path = get_enhanced_path();
    if let Ok(out) = Command::new("which").arg(name).env("PATH", &enhanced_path).output() {
        if out.status.success() {
            let found = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !found.is_empty() {
                return found;
            }
        }
    }

    name.to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvStatus {
    pub has_adb: bool,
    pub has_scrcpy: bool,
    pub has_xcrun: bool,
    pub adb_path: String,
    pub scrcpy_path: String,
}

#[tauri::command]
fn check_system_environment() -> EnvStatus {
    let adb = resolve_bin("adb");
    let scrcpy = resolve_bin("scrcpy");
    let enhanced_path = get_enhanced_path();

    let xcrun_ok = Command::new("which")
        .arg("xcrun")
        .env("PATH", &enhanced_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let adb_ok = Command::new(&adb)
        .arg("version")
        .env("PATH", &enhanced_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let scrcpy_ok = Command::new(&scrcpy)
        .arg("-v")
        .env("PATH", &enhanced_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    EnvStatus {
        has_adb: adb_ok,
        has_scrcpy: scrcpy_ok,
        has_xcrun: xcrun_ok,
        adb_path: adb,
        scrcpy_path: scrcpy,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceItem {
    pub id: String,
    pub name: String,
    pub platform: String, // "iOS" | "Android"
    pub reality: String,  // "physical" | "simulated"
    pub state: String,    // "connected" | "disconnected" | "booted" | "shutdown"
    pub os_version: String,
    pub model: String,
    pub product_type: String,
    pub serial_number: String,
    pub udid: String,
    pub core_device_id: String,
    pub transport_type: String, // "Wired" | "Wireless"
    pub pairing_state: String,  // "Paired" | "Unpaired"
    pub ddi_services: String,   // "Enabled" | "Disabled"
    pub developer_mode: String, // "Enabled" | "Disabled"
    pub file_system: String,
    pub capacity: String,
    pub ecid: String,
    pub screen_viewing_url: Option<String>,
}

#[derive(Deserialize, Default)]
struct DevicectlOutput {
    result: Option<DevicectlResult>,
}

#[derive(Deserialize, Default)]
struct DevicectlResult {
    devices: Option<Vec<DevicectlDevice>>,
}

#[derive(Deserialize, Default)]
struct DevicectlDevice {
    identifier: Option<String>,
    #[serde(rename = "deviceProperties")]
    device_properties: Option<DevicectlDeviceProps>,
    #[serde(rename = "hardwareProperties")]
    hardware_properties: Option<DevicectlHardwareProps>,
    #[serde(rename = "connectionProperties")]
    connection_properties: Option<DevicectlConnProps>,
}

#[derive(Deserialize, Default)]
struct DevicectlDeviceProps {
    name: Option<String>,
    #[serde(rename = "osVersionNumber")]
    os_version_number: Option<String>,
    #[serde(rename = "bootState")]
    boot_state: Option<String>,
    #[serde(rename = "ddiServicesAvailable")]
    ddi_services_available: Option<bool>,
    #[serde(rename = "developerModeStatus")]
    developer_mode_status: Option<String>,
    #[serde(rename = "bootedFromSnapshot")]
    booted_from_snapshot: Option<bool>,
    #[serde(rename = "screenViewingURL")]
    screen_viewing_url: Option<String>,
}

#[derive(Deserialize, Default)]
struct DevicectlHardwareProps {
    #[serde(rename = "marketingName")]
    marketing_name: Option<String>,
    #[serde(rename = "productType")]
    product_type: Option<String>,
    #[serde(rename = "serialNumber")]
    serial_number: Option<String>,
    udid: Option<String>,
    reality: Option<String>,
    ecid: Option<serde_json::Value>,
    #[serde(rename = "internalStorageCapacity")]
    internal_storage_capacity: Option<u64>,
}

#[derive(Deserialize, Default)]
struct DevicectlConnProps {
    #[serde(rename = "transportType")]
    transport_type: Option<String>,
    #[serde(rename = "pairingState")]
    pairing_state: Option<String>,
    #[serde(rename = "tunnelState")]
    tunnel_state: Option<String>,
}

#[derive(Deserialize, Default)]
struct SimctlOutput {
    devices: Option<std::collections::HashMap<String, Vec<SimctlDevice>>>,
}

#[derive(Deserialize, Default)]
struct SimctlDevice {
    name: String,
    udid: String,
    state: String,
    #[serde(rename = "isAvailable")]
    is_available: bool,
    #[serde(rename = "deviceTypeIdentifier")]
    device_type_identifier: Option<String>,
}

fn scan_ios_physical_devices() -> Vec<DeviceItem> {
    let mut list = Vec::new();
    // Unique path per call: the network scan may run concurrently with the device list refresh
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let tmp_file = std::env::temp_dir().join(format!("devicectl_scan_{}.json", nanos));

    let _ = Command::new("xcrun")
        .args(["devicectl", "list", "devices", "--json-output"])
        .arg(&tmp_file)
        .output();

    let content = std::fs::read_to_string(&tmp_file);
    let _ = std::fs::remove_file(&tmp_file);
    if let Ok(content) = content {
        if let Ok(parsed) = serde_json::from_str::<DevicectlOutput>(&content) {
            if let Some(res) = parsed.result {
                if let Some(devices) = res.devices {
                    for dev in devices {
                        let hw = dev.hardware_properties.unwrap_or_default();
                        let dp = dev.device_properties.unwrap_or_default();
                        let cp = dev.connection_properties.unwrap_or_default();

                        let reality = hw.reality.clone().unwrap_or_else(|| "physical".to_string());
                        if reality != "physical" {
                            continue;
                        }

                        let name = dp.name.unwrap_or_else(|| "iPhone".to_string());
                        let os_ver = format!("iOS {}", dp.os_version_number.unwrap_or_else(|| "Unknown".to_string()));
                        let model = hw.marketing_name.unwrap_or_else(|| "iPhone".to_string());
                        let product_type = hw.product_type.unwrap_or_else(|| "Unknown".to_string());
                        let serial = hw.serial_number.unwrap_or_else(|| "Unknown".to_string());
                        let udid = hw.udid.unwrap_or_else(|| "Unknown".to_string());
                        let core_id = dev.identifier.unwrap_or_else(|| udid.clone());

                        let transport = match cp.transport_type.as_deref() {
                            Some("wired") => "Wired".to_string(),
                            Some("wireless") | Some("localNetwork") => "Wireless".to_string(),
                            _ => "Wired".to_string(),
                        };

                        let pairing = match cp.pairing_state.as_deref() {
                            Some("paired") => "Paired".to_string(),
                            _ => "Unpaired".to_string(),
                        };

                        let ddi = if dp.ddi_services_available.unwrap_or(false) {
                            "Enabled".to_string()
                        } else {
                            "Disabled".to_string()
                        };

                        let dev_mode = match dp.developer_mode_status.as_deref() {
                            Some("enabled") => "Enabled".to_string(),
                            _ => "Disabled".to_string(),
                        };

                        let file_sys = if dp.booted_from_snapshot.unwrap_or(false) {
                            "Booted From Snapshot".to_string()
                        } else {
                            "Standard".to_string()
                        };

                        let capacity = if let Some(cap) = hw.internal_storage_capacity {
                            format!("{} GB", cap / 1_000_000_000)
                        } else {
                            "256 GB".to_string()
                        };

                        let ecid_str = hw.ecid.map(|e| e.to_string()).unwrap_or_else(|| "N/A".to_string());

                        let state = match cp.tunnel_state.as_deref() {
                            Some("connected") => "connected".to_string(),
                            _ => dp.boot_state.unwrap_or_else(|| "connected".to_string()),
                        };

                        list.push(DeviceItem {
                            id: udid.clone(),
                            name,
                            platform: "iOS".to_string(),
                            reality: "physical".to_string(),
                            state,
                            os_version: os_ver,
                            model,
                            product_type,
                            serial_number: serial,
                            udid,
                            core_device_id: core_id,
                            transport_type: transport,
                            pairing_state: pairing,
                            ddi_services: ddi,
                            developer_mode: dev_mode,
                            file_system: file_sys,
                            capacity,
                            ecid: ecid_str,
                            screen_viewing_url: dp.screen_viewing_url,
                        });
                    }
                }
            }
        }
    }

    list
}

fn scan_ios_simulators() -> Vec<DeviceItem> {
    let mut list = Vec::new();

    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output();

    if let Ok(out) = output {
        if let Ok(parsed) = serde_json::from_slice::<SimctlOutput>(&out.stdout) {
            if let Some(runtimes) = parsed.devices {
                for (runtime_key, sim_list) in runtimes {
                    let os_ver = if runtime_key.contains("iOS-") {
                        let parts: Vec<&str> = runtime_key.split("iOS-").collect();
                        if parts.len() > 1 {
                            format!("iOS {}", parts[1].replace('-', "."))
                        } else {
                            "iOS Simulator".to_string()
                        }
                    } else {
                        "iOS Simulator".to_string()
                    };

                    for sim in sim_list {
                        if !sim.is_available {
                            continue;
                        }
                        let model_name = sim.name.clone();
                        let state = sim.state.to_lowercase();

                        list.push(DeviceItem {
                            id: sim.udid.clone(),
                            name: sim.name,
                            platform: "iOS".to_string(),
                            reality: "simulated".to_string(),
                            state,
                            os_version: os_ver.clone(),
                            model: model_name,
                            product_type: sim.device_type_identifier.unwrap_or_else(|| "Simulator".to_string()),
                            serial_number: "SIMULATOR".to_string(),
                            udid: sim.udid.clone(),
                            core_device_id: sim.udid.clone(),
                            transport_type: "Local".to_string(),
                            pairing_state: "Paired".to_string(),
                            ddi_services: "Enabled".to_string(),
                            developer_mode: "Enabled".to_string(),
                            file_system: "CoreSimulator Snapshot".to_string(),
                            capacity: "Shared Mac Storage".to_string(),
                            ecid: "N/A".to_string(),
                            screen_viewing_url: None,
                        });
                    }
                }
            }
        }
    }

    list
}

fn scan_android_devices() -> Vec<DeviceItem> {
    let mut list = Vec::new();
    let adb = resolve_bin("adb");
    let enhanced_path = get_enhanced_path();

    let output = Command::new(&adb)
        .env("PATH", &enhanced_path)
        .args(["devices", "-l"])
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let serial = parts[0].to_string();
            let status = parts[1].to_string();

            let transport = if serial.contains(':') {
                "Wireless (Tailscale/LAN)".to_string()
            } else {
                "Wired USB".to_string()
            };

            let mut model = "Android Device".to_string();
            let mut os_ver = "Android".to_string();
            let mut manufacturer = "".to_string();

            if status == "device" {
                if let Ok(prop_out) = Command::new(&adb)
                    .env("PATH", &enhanced_path)
                    .args(["-s", &serial, "shell", "getprop ro.product.model; getprop ro.build.version.release; getprop ro.product.manufacturer"])
                    .output()
                {
                    let prop_str = String::from_utf8_lossy(&prop_out.stdout);
                    let prop_lines: Vec<&str> = prop_str.lines().collect();
                    if !prop_lines.is_empty() && !prop_lines[0].trim().is_empty() {
                        model = prop_lines[0].trim().to_string();
                    }
                    if prop_lines.len() > 1 && !prop_lines[1].trim().is_empty() {
                        os_ver = format!("Android {}", prop_lines[1].trim());
                    }
                    if prop_lines.len() > 2 {
                        manufacturer = prop_lines[2].trim().to_string();
                    }
                }
            }

            let display_name = if !manufacturer.is_empty() && !model.to_lowercase().contains(&manufacturer.to_lowercase()) {
                format!("{} {}", manufacturer, model)
            } else {
                model.clone()
            };

            list.push(DeviceItem {
                id: serial.clone(),
                name: display_name,
                platform: "Android".to_string(),
                reality: "physical".to_string(),
                state: if status == "device" { "connected".to_string() } else { status },
                os_version: os_ver,
                model,
                product_type: "Android ARM64".to_string(),
                serial_number: serial.clone(),
                udid: serial.clone(),
                core_device_id: serial.clone(),
                transport_type: transport,
                pairing_state: "Paired".to_string(),
                ddi_services: "ADB Daemon Active".to_string(),
                developer_mode: "USB Debugging Enabled".to_string(),
                file_system: "ext4/f2fs".to_string(),
                capacity: "Internal Flash".to_string(),
                ecid: "N/A".to_string(),
                screen_viewing_url: None,
            });
        }
    }

    list
}

#[tauri::command]
fn get_all_devices() -> Vec<DeviceItem> {
    let mut all = Vec::new();
    all.extend(scan_ios_physical_devices());
    all.extend(scan_android_devices());
    all.extend(scan_ios_simulators());
    all
}

fn is_safe_identifier(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.starts_with('-')
        && s.len() <= 128
        && s.chars().all(|c| c.is_alphanumeric() || c == '.' || c == ':' || c == '-' || c == '_')
}

// async: `adb connect` to an unreachable host can block for seconds
#[tauri::command(async)]
fn connect_adb_device(ip_port: String) -> Result<String, String> {
    if !is_safe_identifier(&ip_port) {
        return Err("Địa chỉ IP hoặc cổng không hợp lệ (chỉ chấp nhận ký tự an toàn)".to_string());
    }

    let adb = resolve_bin("adb");
    let enhanced_path = get_enhanced_path();
    let output = Command::new(&adb)
        .env("PATH", &enhanced_path)
        .args(["connect", &ip_port])
        .output()
        .map_err(|e| e.to_string())?;

    let res = String::from_utf8_lossy(&output.stdout).to_string();
    if res.contains("connected") {
        Ok(res)
    } else {
        Err(res)
    }
}

#[tauri::command]
fn disconnect_adb_device(ip_port: String) -> Result<String, String> {
    if !is_safe_identifier(&ip_port) {
        return Err("Địa chỉ IP không hợp lệ".to_string());
    }

    let adb = resolve_bin("adb");
    let enhanced_path = get_enhanced_path();
    let output = Command::new(&adb)
        .env("PATH", &enhanced_path)
        .args(["disconnect", &ip_port])
        .output()
        .map_err(|e| e.to_string())?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
fn launch_view_screen(device_id: String, platform: String, reality: String, screen_url: Option<String>) -> Result<String, String> {
    if !is_safe_identifier(&device_id) {
        return Err("Device ID không hợp lệ".to_string());
    }

    if platform == "Android" {
        let adb = resolve_bin("adb");
        let scrcpy = resolve_bin("scrcpy");

        if !Path::new(&scrcpy).exists() && scrcpy == "scrcpy" {
            return Err("Chưa cài đặt công cụ scrcpy. Vui lòng chạy lệnh: brew install scrcpy".to_string());
        }

        let enhanced_path = get_enhanced_path();

        // Kiểm tra xem thiết bị có online trong ADB không
        if let Ok(chk) = Command::new(&adb)
            .env("PATH", &enhanced_path)
            .args(["devices"])
            .output()
        {
            let chk_str = String::from_utf8_lossy(&chk.stdout);
            let mut found = false;
            let mut state = "not found";
            for line in chk_str.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[0] == device_id {
                    found = true;
                    state = parts[1];
                    break;
                }
            }
            if found {
                if state == "unauthorized" {
                    return Err("Thiết bị chưa cho phép USB/Wireless Debugging. Vui lòng bấm 'Cho phép' (Allow) trên màn hình điện thoại.".to_string());
                } else if state == "offline" {
                    return Err("Thiết bị đang ở trạng thái Offline. Vui lòng kiểm tra lại kết nối Tailscale/Wi-Fi.".to_string());
                }
            }
        }

        let mut cmd = Command::new(&scrcpy);
        cmd.arg("-s").arg(&device_id);
        cmd.arg("--window-title").arg(format!("Xhub - {}", device_id));
        cmd.arg("--stay-awake");

        // Tối ưu cho kết nối mạng không dây (Tailscale/LAN qua cổng mạng :5555)
        if device_id.contains(':') {
            cmd.arg("--force-adb-forward");
            cmd.arg("--video-bit-rate=8M");
            cmd.arg("--max-fps=60");
        }

        cmd.env("PATH", &enhanced_path);
        cmd.env("ADB", &adb);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(_) => Ok("Đã mở cửa sổ điều khiển scrcpy thành công".to_string()),
            Err(e) => Err(format!("Lỗi chạy scrcpy: {}", e)),
        }
    } else if platform == "iOS" {
        if reality == "simulated" {
            let _ = Command::new("xcrun")
                .args(["simctl", "boot", &device_id])
                .output();
            let _ = Command::new("open")
                .args(["-a", "Simulator", "--args", "-CurrentDeviceUDID", &device_id])
                .spawn();
            Ok("Đã mở iOS Simulator".to_string())
        } else {
            if let Some(url) = screen_url {
                // Ensure URL begins with devices://
                if url.starts_with("devices://") {
                    let _ = Command::new("open").arg(&url).spawn();
                    Ok("Đã mở CoreDevice Screen Viewer".to_string())
                } else {
                    Err("URL xem màn hình không an toàn".to_string())
                }
            } else {
                let _ = Command::new("open")
                    .args(["-a", "QuickTime Player"])
                    .spawn();
                Ok("Mở QuickTime để chiếu màn hình iPhone qua cáp Lightning/USB-C".to_string())
            }
        }
    } else {
        Err("Nền tảng không hỗ trợ".to_string())
    }
}

#[tauri::command]
fn send_android_keyevent(serial: String, keycode: u32) -> Result<String, String> {
    if !is_safe_identifier(&serial) {
        return Err("Serial thiết bị không hợp lệ".to_string());
    }

    let adb = resolve_bin("adb");
    let enhanced_path = get_enhanced_path();
    let output = Command::new(&adb)
        .env("PATH", &enhanced_path)
        .args(["-s", &serial, "shell", "input", "keyevent", &keycode.to_string()])
        .output()
        .map_err(|e| e.to_string())?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
fn take_screenshot(device_id: String, platform: String) -> Result<String, String> {
    if !is_safe_identifier(&device_id) {
        return Err("Device ID không hợp lệ".to_string());
    }

    let safe_platform = match platform.as_str() {
        "Android" => "Android",
        "iOS" => "iOS",
        _ => return Err("Nền tảng không hỗ trợ chụp màn hình".to_string()),
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/trinhhao".to_string());
    let dest_path = format!("{}/Downloads/Screenshot_{}_{}.png", home, safe_platform, timestamp);

    if safe_platform == "Android" {
        let adb = resolve_bin("adb");
        let enhanced_path = get_enhanced_path();
        let remote_tmp = "/sdcard/screenshot_tmp.png";
        let _ = Command::new(&adb)
            .env("PATH", &enhanced_path)
            .args(["-s", &device_id, "shell", "screencap", "-p", remote_tmp])
            .output();
        let _ = Command::new(&adb)
            .env("PATH", &enhanced_path)
            .args(["-s", &device_id, "pull", remote_tmp, &dest_path])
            .output();
        let _ = Command::new(&adb)
            .env("PATH", &enhanced_path)
            .args(["-s", &device_id, "shell", "rm", remote_tmp])
            .output();

        let _ = Command::new("open").args(["-R", &dest_path]).spawn();
        Ok(format!("Ảnh chụp đã lưu tại: {}", dest_path))
    } else {
        let _ = Command::new("xcrun")
            .args(["simctl", "io", &device_id, "screenshot", &dest_path])
            .output();
        let _ = Command::new("open").args(["-R", &dest_path]).spawn();
        Ok(format!("Ảnh chụp đã lưu tại: {}", dest_path))
    }
}

#[tauri::command]
fn pick_app_file() -> Result<Option<String>, String> {
    let script = r#"
        try
            set pickedFile to (choose file with prompt "Chọn file ứng dụng (.apk, .ipa, .app):" of type {"apk", "ipa", "app"})
            return POSIX path of pickedFile
        on error
            return ""
        end try
    "#;

    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("Không thể mở hộp thoại chọn file: {}", e))?;

    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path_str.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path_str))
    }
}

#[tauri::command]
fn install_app_package(
    device_id: String,
    platform: String,
    reality: String,
    file_path: String,
) -> Result<String, String> {
    if !is_safe_identifier(&device_id) {
        return Err("Mã thiết bị không hợp lệ".to_string());
    }

    let path_obj = Path::new(&file_path);
    if !path_obj.exists() {
        return Err(format!("File không tồn tại: {}", file_path));
    }

    let enhanced_path = get_enhanced_path();

    if platform == "Android" {
        let adb = resolve_bin("adb");
        let output = Command::new(&adb)
            .env("PATH", &enhanced_path)
            .args(["-s", &device_id, "install", "-r", "-t", "-d", &file_path])
            .output()
            .map_err(|e| format!("Lỗi khởi chạy ADB install: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let combined = format!("{}\n{}", stdout, stderr).trim().to_string();

        if combined.contains("Success") {
            Ok(format!("Cài đặt thành công APK lên thiết bị {}", device_id))
        } else {
            Err(format!("Cài đặt thất bại:\n{}", combined))
        }
    } else if platform == "iOS" {
        if reality == "simulated" {
            let output = Command::new("xcrun")
                .env("PATH", &enhanced_path)
                .args(["simctl", "install", &device_id, &file_path])
                .output()
                .map_err(|e| format!("Lỗi khởi chạy simctl install: {}", e))?;

            if output.status.success() {
                Ok(format!("Cài đặt thành công lên iOS Simulator {}", device_id))
            } else {
                let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
                Err(format!("Lỗi cài đặt Simulator:\n{}", err_msg))
            }
        } else {
            let output = Command::new("xcrun")
                .env("PATH", &enhanced_path)
                .args(["devicectl", "device", "install", "app", "--device", &device_id, &file_path])
                .output()
                .map_err(|e| format!("Lỗi khởi chạy devicectl install: {}", e))?;

            if output.status.success() {
                Ok(format!("Cài đặt thành công lên thiết bị iOS {}", device_id))
            } else {
                let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
                let out_msg = String::from_utf8_lossy(&output.stdout).to_string();
                Err(format!("Lỗi cài đặt iOS:\n{}\n{}", err_msg, out_msg))
            }
        }
    } else {
        Err("Nền tảng không hỗ trợ".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            get_all_devices,
            connect_adb_device,
            disconnect_adb_device,
            launch_view_screen,
            send_android_keyevent,
            take_screenshot,
            check_system_environment,
            pick_app_file,
            install_app_package,
            network_scan::scan_network_devices,
            network_scan::pair_adb_device,
            network_scan::connect_ios_device,
            adb_wifi::enable_adb_wifi,
            android_mirror::start_android_mirror,
            android_mirror::stop_android_mirror,
            android_mirror::android_mirror_touch,
            android_mirror::android_mirror_key,
            ios_mirror::start_ios_mirror,
            ios_mirror::stop_ios_mirror
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
