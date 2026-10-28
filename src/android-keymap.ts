import type { KeyboardEvent } from 'react';

// Android KeyEvent codes for keys that do not produce text (text goes through input events)
const SPECIAL_KEYS: Record<string, number> = {
  Enter: 66,
  Backspace: 67,
  Delete: 112,
  Tab: 61,
  Escape: 111,
  ArrowUp: 19,
  ArrowDown: 20,
  ArrowLeft: 21,
  ArrowRight: 22,
  Home: 122,
  End: 123,
  PageUp: 92,
  PageDown: 93,
};

// Letter shortcuts forwarded with Ctrl (Cmd on the Mac maps to Ctrl on Android).
// Paste is handled separately so the Mac clipboard reaches the phone.
const SHORTCUT_KEYS: Record<string, number> = { a: 29, c: 31, x: 52, z: 54, y: 53 };

const META_SHIFT_ON = 0x41; // META_SHIFT_ON | META_SHIFT_LEFT_ON
const META_CTRL_ON = 0x3000; // META_CTRL_ON | META_CTRL_LEFT_ON

export interface AndroidKey {
  keycode: number;
  meta: number;
}

/** Returns the Android key for a keydown, or null when the browser should produce text. */
export function androidKeyFor(e: KeyboardEvent): AndroidKey | null {
  const ctrl = e.metaKey || e.ctrlKey;
  const meta = (e.shiftKey ? META_SHIFT_ON : 0) | (ctrl ? META_CTRL_ON : 0);
  const special = SPECIAL_KEYS[e.key];
  if (special !== undefined) return { keycode: special, meta };
  const shortcut = ctrl ? SHORTCUT_KEYS[e.key.toLowerCase()] : undefined;
  return shortcut !== undefined ? { keycode: shortcut, meta } : null;
}
