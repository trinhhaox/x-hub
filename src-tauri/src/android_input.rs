// Mouse wheel and keyboard text for the in-app Android mirror (scrcpy v5 control messages).
use super::android_mirror::write_control;

const TYPE_INJECT_TEXT: u8 = 1;
const TYPE_INJECT_SCROLL_EVENT: u8 = 3;
const TYPE_SET_CLIPBOARD: u8 = 9;
// scrcpy injects text through the key character map, which only covers ASCII
const INJECT_TEXT_MAX_LEN: usize = 300;
const CLIPBOARD_MAX_LEN: usize = 256 * 1024 - 14;

/// Scroll amounts are in wheel notches, clamped to scrcpy's [-16, 16] range.
fn scroll_message(x: i32, y: i32, width: u16, height: u16, h_scroll: f32, v_scroll: f32) -> Vec<u8> {
    // Signed 16-bit fixed point over [-1, 1]; the server multiplies by 16
    let to_fixed = |v: f32| ((v.clamp(-16.0, 16.0) / 16.0) * 32768.0).clamp(-32768.0, 32767.0) as i16;
    let mut msg = Vec::with_capacity(21);
    msg.push(TYPE_INJECT_SCROLL_EVENT);
    msg.extend_from_slice(&x.to_be_bytes());
    msg.extend_from_slice(&y.to_be_bytes());
    msg.extend_from_slice(&width.to_be_bytes());
    msg.extend_from_slice(&height.to_be_bytes());
    msg.extend_from_slice(&to_fixed(h_scroll).to_be_bytes());
    msg.extend_from_slice(&to_fixed(v_scroll).to_be_bytes());
    msg.extend_from_slice(&0u32.to_be_bytes()); // buttons
    msg
}

/// ASCII is typed as key events; anything else (Vietnamese, emoji) goes through the
/// phone clipboard with paste, the only way Android accepts arbitrary Unicode input.
fn text_message(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    let mut msg = Vec::with_capacity(bytes.len() + 14);
    if text.is_ascii() && bytes.len() <= INJECT_TEXT_MAX_LEN {
        msg.push(TYPE_INJECT_TEXT);
    } else if bytes.len() <= CLIPBOARD_MAX_LEN {
        msg.push(TYPE_SET_CLIPBOARD);
        msg.extend_from_slice(&0u64.to_be_bytes()); // sequence 0: no acknowledgement
        msg.push(1); // paste after setting
    } else {
        return Err("Đoạn văn bản quá dài".to_string());
    }
    msg.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    msg.extend_from_slice(bytes);
    Ok(msg)
}

#[tauri::command(async)]
pub fn android_mirror_scroll(
    serial: String,
    x: i32,
    y: i32,
    width: u16,
    height: u16,
    h_scroll: f32,
    v_scroll: f32,
) -> Result<(), String> {
    write_control(&serial, &scroll_message(x, y, width, height, h_scroll, v_scroll))
}

#[tauri::command(async)]
pub fn android_mirror_text(serial: String, text: String) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    write_control(&serial, &text_message(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_scroll_as_fixed_point() {
        let msg = scroll_message(10, 20, 576, 1280, 0.0, -1.0);
        assert_eq!(msg.len(), 21);
        assert_eq!(msg[0], TYPE_INJECT_SCROLL_EVENT);
        assert_eq!(i16::from_be_bytes([msg[15], msg[16]]), -2048); // -1 notch = -1/16 of range
        assert_eq!(i16::from_be_bytes(msg[13..15].try_into().unwrap()), 0);
        let clamped = scroll_message(0, 0, 1, 1, 0.0, 99.0);
        assert_eq!(i16::from_be_bytes([clamped[15], clamped[16]]), 32767);
    }

    #[test]
    fn ascii_is_injected_and_unicode_is_pasted() {
        let ascii = text_message("hi").unwrap();
        assert_eq!(ascii, vec![TYPE_INJECT_TEXT, 0, 0, 0, 2, b'h', b'i']);
        let viet = text_message("ă").unwrap();
        assert_eq!(viet[0], TYPE_SET_CLIPBOARD);
        assert_eq!(viet[9], 1); // paste
        assert_eq!(&viet[10..14], &2u32.to_be_bytes()); // "ă" is 2 UTF-8 bytes
    }
}
