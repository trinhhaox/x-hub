// Switches a USB-connected Android phone to adb over Wi-Fi (`adb tcpip`) and connects to it,
// so the cable can be unplugged. Needed on Android < 11, which has no Wireless Debugging;
// the phone falls back to USB-only mode after every reboot.
use std::process::Command;
use std::time::Duration;

use super::{get_enhanced_path, is_safe_identifier, resolve_bin};

const ADB_WIFI_PORT: &str = "5555";

fn adb(args: &[&str]) -> Result<String, String> {
    let out = Command::new(resolve_bin("adb"))
        .env("PATH", get_enhanced_path())
        .args(args)
        .output()
        .map_err(|e| format!("Không chạy được adb: {}", e))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text)
}

/// Finds the IPv4 in `ip -f inet addr show wlan0` output ("inet 192.168.1.19/24 ...").
fn parse_inet_addr(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .skip_while(|w| *w != "inet")
        .nth(1)
        .and_then(|cidr| cidr.split('/').next())
        .filter(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
        .map(str::to_string)
}

#[tauri::command(async)]
pub fn enable_adb_wifi(serial: String) -> Result<String, String> {
    if !is_safe_identifier(&serial) || serial.contains(':') {
        return Err("Chỉ bật được cho điện thoại đang cắm cáp USB".to_string());
    }
    let ip = parse_inet_addr(&adb(&["-s", &serial, "shell", "ip", "-f", "inet", "addr", "show", "wlan0"])?)
        .ok_or_else(|| "Điện thoại chưa kết nối Wi-Fi. Hãy bật Wi-Fi (cùng mạng với Mac) rồi thử lại".to_string())?;

    let res = adb(&["-s", &serial, "tcpip", ADB_WIFI_PORT])?;
    if !res.contains("restarting") {
        return Err(format!("Không bật được chế độ Wi-Fi: {}", res.trim()));
    }

    // adbd restarts on the phone; give it a moment before connecting
    let address = format!("{}:{}", ip, ADB_WIFI_PORT);
    let mut last = String::new();
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(1000));
        last = adb(&["connect", &address])?;
        if last.contains("connected to") {
            return Ok(address);
        }
    }
    Err(format!("Đã bật chế độ Wi-Fi nhưng chưa kết nối được {}: {}", address, last.trim()))
}

#[cfg(test)]
mod tests {
    use super::parse_inet_addr;

    #[test]
    fn parses_wlan_ipv4() {
        let out = "30: wlan0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500\n    inet 192.168.1.19/24 brd 192.168.1.255 scope global wlan0\n";
        assert_eq!(parse_inet_addr(out).as_deref(), Some("192.168.1.19"));
    }

    #[test]
    fn no_address_when_wifi_off() {
        assert_eq!(parse_inet_addr("Device \"wlan0\" does not exist."), None);
        assert_eq!(parse_inet_addr(""), None);
    }
}
