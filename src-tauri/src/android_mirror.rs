// In-app Android screen mirroring: runs the scrcpy server on the device, forwards its
// H.264 stream to the webview (decoded there with WebCodecs) and injects touch/key events.
// Protocol reference: scrcpy v5 server (tunnel_forward mode, video + control sockets).
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tauri::ipc::{Channel, InvokeResponseBody};

use super::{get_enhanced_path, is_safe_identifier, resolve_bin};

const DEVICE_SERVER_PATH: &str = "/data/local/tmp/scrcpy-server.jar";

// Messages sent to the webview channel (first byte = kind)
const MSG_VIDEO_SIZE: u8 = 0; // + width u32 BE + height u32 BE
const MSG_PACKET: u8 = 1; // + flags u8 (1 = config, 2 = key frame) + H.264 Annex-B data
const MSG_ENDED: u8 = 2;

struct Session {
    id: String,
    server: Child,
    video: TcpStream,
    control: TcpStream,
    port: String,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn adb(serial: &str) -> Command {
    let mut cmd = Command::new(resolve_bin("adb"));
    cmd.env("PATH", get_enhanced_path()).args(["-s", serial]);
    cmd
}

fn find_server_jar() -> Result<String, String> {
    let scrcpy = resolve_bin("scrcpy");
    // Homebrew layout: <prefix>/bin/scrcpy -> <prefix>/share/scrcpy/scrcpy-server
    let mut candidates = vec![
        "/opt/homebrew/share/scrcpy/scrcpy-server".to_string(),
        "/usr/local/share/scrcpy/scrcpy-server".to_string(),
    ];
    if let Some(prefix) = Path::new(&scrcpy).parent().and_then(|bin| bin.parent()) {
        candidates.insert(0, prefix.join("share/scrcpy/scrcpy-server").to_string_lossy().to_string());
    }
    candidates
        .into_iter()
        .find(|p| Path::new(p).exists())
        .ok_or_else(|| "Không tìm thấy scrcpy-server. Vui lòng chạy: brew install scrcpy".to_string())
}

/// The server refuses to start unless the version argument matches its own build.
fn scrcpy_version() -> Result<String, String> {
    let out = Command::new(resolve_bin("scrcpy"))
        .env("PATH", get_enhanced_path())
        .arg("-v")
        .output()
        .map_err(|e| format!("Không chạy được scrcpy: {}", e))?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_string)
        .ok_or_else(|| "Không đọc được phiên bản scrcpy".to_string())
}

fn run_checked(cmd: &mut Command, what: &str) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{}: {}", what, e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("{}: {}", what, String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// adb forward accepts the TCP connection even before the server listens, so a
/// connection only counts once the server's dummy byte arrives.
fn connect_video(port: &str, server: &mut Child) -> Result<TcpStream, String> {
    for _ in 0..60 {
        if let Ok(Some(_)) = server.try_wait() {
            break;
        }
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", port)) {
            let mut dummy = [0u8; 1];
            if stream.read_exact(&mut dummy).is_ok() {
                return Ok(stream);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("Không kết nối được tới scrcpy-server trên thiết bị".to_string())
}

/// Video size/bitrate per link. Measured on a Tailscale link relayed via DERP (~3 Mbps cap,
/// 100–200 ms RTT): 6 Mbps backed up to 30 fps with 2–4 s lag, 2 Mbps kept 58 fps under 0.35 s.
fn stream_params(serial: &str) -> (&'static str, &'static str) {
    let host = serial.split(':').next().unwrap_or("");
    let octets: Vec<u8> = host.split('.').filter_map(|o| o.parse().ok()).collect();
    let is_tailscale = octets.len() == 4 && octets[0] == 100 && (64..128).contains(&octets[1]);
    if is_tailscale {
        ("max_size=1024", "video_bit_rate=2000000")
    } else if serial.contains(':') {
        ("max_size=1280", "video_bit_rate=6000000") // Wi-Fi LAN
    } else {
        ("max_size=1600", "video_bit_rate=12000000") // USB
    }
}

fn send(channel: &Channel<InvokeResponseBody>, data: Vec<u8>) -> bool {
    channel.send(InvokeResponseBody::Raw(data)).is_ok()
}

/// Forwards a scrcpy-framed H.264 stream (also produced by the iOS capture helper) to the webview.
pub(crate) fn pump_video(mut video: impl Read, channel: Channel<InvokeResponseBody>) {
    let mut header = [0u8; 12];
    loop {
        if video.read_exact(&mut header).is_err() {
            break;
        }
        // Session packet: MSB set, then width/height; no payload
        if header[0] & 0x80 != 0 {
            let mut msg = vec![MSG_VIDEO_SIZE];
            msg.extend_from_slice(&header[4..12]);
            if !send(&channel, msg) {
                break;
            }
            continue;
        }
        let pts_flags = u64::from_be_bytes(header[0..8].try_into().unwrap_or_default());
        let size = u32::from_be_bytes(header[8..12].try_into().unwrap_or_default()) as usize;
        let mut msg = vec![0u8; size + 2];
        msg[0] = MSG_PACKET;
        msg[1] = u8::from(pts_flags & (1 << 62) != 0) | (u8::from(pts_flags & (1 << 61) != 0) << 1);
        if video.read_exact(&mut msg[2..]).is_err() || !send(&channel, msg) {
            break;
        }
    }
    let _ = channel.send(InvokeResponseBody::Raw(vec![MSG_ENDED]));
}

fn close_session(serial: &str, mut s: Session) {
    let _ = s.video.shutdown(Shutdown::Both);
    let _ = s.control.shutdown(Shutdown::Both);
    let _ = s.server.kill();
    let _ = s.server.wait();
    let _ = adb(serial).args(["forward", "--remove", &format!("tcp:{}", s.port)]).output();
}

/// Stops the serial's session; with `only_id`, only if it is still that session
/// (a late stop from a closed view must not kill a newer stream).
fn stop_session(serial: &str, only_id: Option<&str>) {
    let session = SESSIONS.lock().ok().and_then(|mut sessions| {
        let matches = sessions.get(serial).is_some_and(|cur| only_id.is_none_or(|id| cur.id == id));
        if matches { sessions.remove(serial) } else { None }
    });
    if let Some(s) = session {
        close_session(serial, s);
    }
}

#[tauri::command(async)]
pub fn start_android_mirror(serial: String, on_data: Channel<InvokeResponseBody>) -> Result<String, String> {
    if !is_safe_identifier(&serial) {
        return Err("Serial thiết bị không hợp lệ".to_string());
    }
    stop_session(&serial, None);

    let jar = find_server_jar()?;
    let version = scrcpy_version()?;
    run_checked(adb(&serial).args(["push", &jar, DEVICE_SERVER_PATH]), "Không đẩy được scrcpy-server")?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1);
    let scid = format!("{:08x}", nanos & 0x7fff_ffff);
    let port = run_checked(
        adb(&serial).args(["forward", "tcp:0", &format!("localabstract:scrcpy_{}", scid)]),
        "Không mở được cổng adb forward",
    )?;

    let (max_size, bit_rate) = stream_params(&serial);
    let mut server = adb(&serial)
        .args([
            "shell",
            &format!("CLASSPATH={}", DEVICE_SERVER_PATH),
            "app_process",
            "/",
            "com.genymobile.scrcpy.Server",
            &version,
            &format!("scid={}", scid),
            "log_level=warn",
            "tunnel_forward=true",
            "audio=false",
            "control=true",
            "cleanup=true",
            "video_codec=h264",
            max_size,
            bit_rate,
            "max_fps=60",
            "stay_awake=true",
            "send_device_meta=false",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Không khởi chạy được scrcpy-server: {}", e))?;

    let connected = connect_video(&port, &mut server).and_then(|mut video| {
        let control = TcpStream::connect(format!("127.0.0.1:{}", port)).map_err(|e| e.to_string())?;
        let mut codec = [0u8; 4];
        video.read_exact(&mut codec).map_err(|e| e.to_string())?;
        if &codec != b"h264" {
            return Err("Thiết bị không mã hoá được video H.264".to_string());
        }
        Ok((video, control))
    });

    let (video, control) = match connected {
        Ok(pair) => pair,
        Err(err) => {
            let _ = server.kill();
            let mut log = String::new();
            if let Some(mut out) = server.stdout.take() {
                let _ = out.read_to_string(&mut log);
            }
            let _ = adb(&serial).args(["forward", "--remove", &format!("tcp:{}", port)]).output();
            let detail = log.lines().find(|l| l.contains("ERROR") || l.contains("Exception")).unwrap_or("");
            return Err(format!("{} {}", err, detail).trim().to_string());
        }
    };

    // Drain server logs and device->client control messages (clipboard) so neither blocks
    if let Some(mut out) = server.stdout.take() {
        std::thread::spawn(move || std::io::copy(&mut out, &mut std::io::sink()));
    }
    let mut device_msgs = control.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || std::io::copy(&mut device_msgs, &mut std::io::sink()));

    let video_reader = video.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || pump_video(video_reader, on_data));

    let replaced = SESSIONS
        .lock()
        .map_err(|_| "Phiên xem màn hình bị lỗi".to_string())?
        .insert(serial.clone(), Session { id: scid.clone(), server, video, control, port });
    if let Some(old) = replaced {
        close_session(&serial, old);
    }
    Ok(scid)
}

#[tauri::command(async)]
pub fn stop_android_mirror(serial: String, session_id: String) {
    stop_session(&serial, Some(&session_id));
}

fn write_control(serial: &str, msg: &[u8]) -> Result<(), String> {
    let mut sessions = SESSIONS.lock().map_err(|_| "Phiên xem màn hình bị lỗi".to_string())?;
    let session = sessions.get_mut(serial).ok_or_else(|| "Chưa mở màn hình thiết bị".to_string())?;
    session.control.write_all(msg).map_err(|e| e.to_string())
}

/// action: 0 = down, 1 = up, 2 = move (Android MotionEvent); x/y in video pixels.
#[tauri::command(async)]
pub fn android_mirror_touch(serial: String, action: u8, x: i32, y: i32, width: u16, height: u16) -> Result<(), String> {
    if action > 2 {
        return Err("Thao tác chạm không hợp lệ".to_string());
    }
    let mut msg = Vec::with_capacity(32);
    msg.push(2); // TYPE_INJECT_TOUCH_EVENT
    msg.push(action);
    msg.extend_from_slice(&(-2i64).to_be_bytes()); // POINTER_ID_GENERIC_FINGER
    msg.extend_from_slice(&x.to_be_bytes());
    msg.extend_from_slice(&y.to_be_bytes());
    msg.extend_from_slice(&width.to_be_bytes());
    msg.extend_from_slice(&height.to_be_bytes());
    msg.extend_from_slice(&(if action == 1 { 0u16 } else { 0xffff }).to_be_bytes()); // pressure
    msg.extend_from_slice(&0u32.to_be_bytes()); // action button
    msg.extend_from_slice(&0u32.to_be_bytes()); // buttons
    write_control(&serial, &msg)
}

/// Sends a key press (down + up) through the mirror's control socket.
#[tauri::command(async)]
pub fn android_mirror_key(serial: String, keycode: u32) -> Result<(), String> {
    let mut msg = Vec::with_capacity(28);
    for action in [0u8, 1u8] {
        msg.push(0); // TYPE_INJECT_KEYCODE
        msg.push(action);
        msg.extend_from_slice(&keycode.to_be_bytes());
        msg.extend_from_slice(&0u32.to_be_bytes()); // repeat
        msg.extend_from_slice(&0u32.to_be_bytes()); // meta state
    }
    write_control(&serial, &msg)
}

#[cfg(test)]
mod tests {
    use super::stream_params;

    #[test]
    fn picks_stream_params_by_link() {
        assert_eq!(stream_params("100.76.122.48:5555").1, "video_bit_rate=2000000");
        assert_eq!(stream_params("192.168.1.10:41235").1, "video_bit_rate=6000000");
        assert_eq!(stream_params("R58M12AB").1, "video_bit_rate=12000000");
        // 100.x outside the Tailscale CGNAT range is treated as a normal LAN address
        assert_eq!(stream_params("100.20.1.2:5555").1, "video_bit_rate=6000000");
    }
}
