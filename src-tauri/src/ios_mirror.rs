// In-app iPhone screen mirroring over USB: runs the bundled Swift helper
// (helpers/ios-screen-capture.swift), which writes a scrcpy-framed H.264 stream to stdout,
// and forwards it to the webview through the same pipeline as Android. View only: iOS
// offers no public API to inject touches.
use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{LazyLock, Mutex};

use tauri::ipc::{Channel, InvokeResponseBody};

use super::android_mirror::pump_video;
use super::is_safe_identifier;

struct Session {
    id: u32,
    helper: Child,
    // Held open: the helper exits when its stdin closes
    _stdin: Option<ChildStdin>,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn helper_path() -> Result<std::path::PathBuf, String> {
    // Tauri places sidecars next to the main executable (dev and bundled app alike)
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.with_file_name("ios-screen-capture");
    if path.exists() {
        Ok(path)
    } else {
        Err("Thiếu công cụ ios-screen-capture trong ứng dụng".to_string())
    }
}

fn stop_session(udid: &str, only_id: Option<u32>) {
    let session = SESSIONS.lock().ok().and_then(|mut sessions| {
        let matches = sessions.get(udid).is_some_and(|cur| only_id.is_none_or(|id| cur.id == id));
        if matches { sessions.remove(udid) } else { None }
    });
    if let Some(mut s) = session {
        let _ = s.helper.kill();
        let _ = s.helper.wait();
    }
}

#[tauri::command(async)]
pub fn start_ios_mirror(udid: String, name: String, on_data: Channel<InvokeResponseBody>) -> Result<u32, String> {
    if !is_safe_identifier(&udid) {
        return Err("UDID không hợp lệ".to_string());
    }
    stop_session(&udid, None);

    let mut helper = Command::new(helper_path()?)
        .arg(&udid)
        .arg(&name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Không chạy được ios-screen-capture: {}", e))?;

    let mut stdout = helper.stdout.take().ok_or("Không đọc được luồng hình")?;
    let mut codec = [0u8; 4];
    if stdout.read_exact(&mut codec).is_err() || &codec != b"h264" {
        let _ = helper.kill();
        let mut err = String::new();
        if let Some(mut stderr) = helper.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        let _ = helper.wait();
        let msg = err.trim().trim_start_matches("NOT_FOUND: ");
        return Err(if msg.is_empty() { "Không mở được màn hình iPhone".to_string() } else { msg.to_string() });
    }
    if let Some(mut stderr) = helper.stderr.take() {
        std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
    }
    std::thread::spawn(move || pump_video(stdout, on_data));

    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1);
    let stdin = helper.stdin.take();
    let replaced = SESSIONS
        .lock()
        .map_err(|_| "Phiên xem màn hình bị lỗi".to_string())?
        .insert(udid, Session { id, helper, _stdin: stdin });
    if let Some(mut old) = replaced {
        let _ = old.helper.kill();
        let _ = old.helper.wait();
    }
    Ok(id)
}

#[tauri::command(async)]
pub fn stop_ios_mirror(udid: String, session_id: u32) {
    stop_session(&udid, Some(session_id));
}
