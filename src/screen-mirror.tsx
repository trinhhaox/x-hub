import {
  useEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type CompositionEvent,
  type FormEvent,
  type KeyboardEvent,
  type PointerEvent,
} from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';
import { androidKeyFor } from './android-keymap';
import { decoderConfigFrom, toAvcc } from './h264-avcc';

// Must match the message kinds in src-tauri/src/android_mirror.rs
const MSG_VIDEO_SIZE = 0;
const MSG_PACKET = 1;
const MSG_ENDED = 2;
const FLAG_CONFIG = 1;
const FLAG_KEY = 2;

const TOUCH_DOWN = 0;
const TOUCH_UP = 1;
const TOUCH_MOVE = 2;

// Browser wheel pixels per Android scroll "notch"; trackpads send many small deltas
const WHEEL_PIXELS_PER_NOTCH = 60;
const WHEEL_LINE_PIXELS = 16;

type SessionId = string | number;

// Android: scrcpy server over adb (touch enabled). iOS: USB capture helper (view only).
const startStream = (platform: string, deviceId: string, deviceName: string, onData: Channel<ArrayBuffer>) =>
  platform === 'iOS'
    ? invoke<SessionId>('start_ios_mirror', { udid: deviceId, name: deviceName, onData })
    : invoke<SessionId>('start_android_mirror', { serial: deviceId, onData });

const stopStream = (platform: string, deviceId: string, sessionId: SessionId) =>
  platform === 'iOS'
    ? invoke('stop_ios_mirror', { udid: deviceId, sessionId })
    : invoke('stop_android_mirror', { serial: deviceId, sessionId });

interface ScreenMirrorProps {
  deviceId: string;
  deviceName: string;
  platform: string;
  notify: (msg: string) => void;
  onEnded: () => void;
}

// Live device screen rendered inside xhub with WebCodecs (Android: touch-controllable)
export function ScreenMirror({ deviceId, deviceName, platform, notify, onEnded }: ScreenMirrorProps) {
  const interactive = platform === 'Android';
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const videoSize = useRef({ width: 0, height: 0 });
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const touching = useRef(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const lastPointer = useRef({ x: 0, y: 0 });
  // Control commands run on a thread pool; chaining keeps touches, keys and text in order
  const controlQueue = useRef<Promise<unknown>>(Promise.resolve());
  const [status, setStatus] = useState<'starting' | 'live'>('starting');

  useEffect(() => {
    let disposed = false;
    let waitingForKey = true;

    // Only the newest decoded frame is painted, once per display refresh; older ones are dropped
    let latest: VideoFrame | null = null;
    let rafId = 0;
    let shownFirst = false;
    const paint = () => {
      rafId = 0;
      const frame = latest;
      latest = null;
      const canvas = canvasRef.current;
      if (!frame) return;
      if (canvas && !disposed) {
        if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
          canvas.width = frame.displayWidth;
          canvas.height = frame.displayHeight;
        }
        canvas.getContext('2d')?.drawImage(frame, 0, 0);
        if (!shownFirst) {
          shownFirst = true;
          setStatus('live');
        }
      }
      frame.close();
    };

    const decoder = new VideoDecoder({
      output: (frame) => {
        latest?.close();
        latest = frame;
        if (!rafId) rafId = requestAnimationFrame(paint);
      },
      error: (e) => notify(`Lỗi giải mã video: ${e.message}`),
    });

    const handlePacket = (flags: number, data: Uint8Array) => {
      // scrcpy sends SPS/PPS as a separate config packet (again after each rotation)
      if (flags & FLAG_CONFIG) {
        const config = decoderConfigFrom(data);
        if (!config) return;
        decoder.configure(config);
        waitingForKey = true;
        return;
      }
      const isKey = (flags & FLAG_KEY) !== 0;
      if (decoder.state !== 'configured' || (waitingForKey && !isKey)) return;
      waitingForKey = false;
      decoder.decode(new EncodedVideoChunk({ type: isKey ? 'key' : 'delta', timestamp: 0, data: toAvcc(data) }));
    };

    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buffer) => {
      if (disposed) return;
      const bytes = new Uint8Array(buffer);
      if (bytes[0] === MSG_VIDEO_SIZE) {
        const view = new DataView(buffer);
        videoSize.current = { width: view.getUint32(1), height: view.getUint32(5) };
      } else if (bytes[0] === MSG_PACKET) {
        handlePacket(bytes[1], bytes.subarray(2));
      } else if (bytes[0] === MSG_ENDED) {
        notify('Đã ngắt kết nối màn hình thiết bị');
        onEnded();
      }
    };

    // Deferred one tick so StrictMode's immediate mount/unmount/mount starts the stream only once
    let started: Promise<SessionId> | null = null;
    const timer = setTimeout(() => {
      started = startStream(platform, deviceId, deviceName, channel);
      started.catch((err) => {
        if (disposed) return;
        notify(`Không mở được màn hình: ${err}`);
        onEnded();
      });
    }, 0);

    return () => {
      disposed = true;
      cancelAnimationFrame(rafId);
      latest?.close();
      clearTimeout(timer);
      if (decoder.state !== 'closed') decoder.close();
      // Stop only this view's session, so a newer stream for the same device keeps running
      started
        ?.then((sessionId) => stopStream(platform, deviceId, sessionId))
        .catch(() => undefined);
    };
    // Restart only when the device changes, not when the parent's callbacks change identity
  }, [deviceId, platform]);

  const sendControl = (command: string, args: Record<string, unknown>) => {
    controlQueue.current = controlQueue.current
      .then(() => invoke(command, { serial: deviceId, ...args }))
      .catch(() => undefined);
  };

  const sendTouch = (action: number, x: number, y: number) => {
    const { width, height } = videoSize.current;
    if (!width) return;
    lastPointer.current = { x, y };
    sendControl('android_mirror_touch', { action, x, y, width, height });
  };

  // Wheel/trackpad scrolling, coalesced per animation frame (native listener: React's is passive)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!interactive || !canvas) return;
    let acc = { x: 0, y: 0 };
    let rafId = 0;
    const flush = () => {
      rafId = 0;
      const { width, height } = videoSize.current;
      if (!width) return;
      const { x, y } = lastPointer.current;
      sendControl('android_mirror_scroll', {
        x, y, width, height,
        hScroll: acc.x / WHEEL_PIXELS_PER_NOTCH,
        vScroll: -acc.y / WHEEL_PIXELS_PER_NOTCH,
      });
      acc = { x: 0, y: 0 };
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const scale = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? WHEEL_LINE_PIXELS : 1;
      const rect = canvas.getBoundingClientRect();
      const { width, height } = videoSize.current;
      lastPointer.current = {
        x: Math.round(((e.clientX - rect.left) / rect.width) * width),
        y: Math.round(((e.clientY - rect.top) / rect.height) * height),
      };
      acc = { x: acc.x + e.deltaX * scale, y: acc.y + e.deltaY * scale };
      if (!rafId) rafId = requestAnimationFrame(flush);
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      canvas.removeEventListener('wheel', onWheel);
      cancelAnimationFrame(rafId);
    };
    // sendControl only closes over deviceId, already a dependency
  }, [interactive, deviceId]);

  // Keyboard: a hidden textarea receives keys and IME text while the mirror is focused
  const sendText = (text: string) => {
    if (text) sendControl('android_mirror_text', { text });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    const key = androidKeyFor(e);
    if (!key) return;
    e.preventDefault();
    sendControl('android_mirror_key', { keycode: key.keycode, meta: key.meta });
  };

  const onInput = (e: FormEvent<HTMLTextAreaElement>) => {
    const native = e.nativeEvent as InputEvent;
    if (native.isComposing) return;
    sendText(native.data ?? e.currentTarget.value);
    e.currentTarget.value = '';
  };

  const onCompositionEnd = (e: CompositionEvent<HTMLTextAreaElement>) => {
    sendText(e.data);
    e.currentTarget.value = '';
  };

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    e.preventDefault();
    sendText(e.clipboardData.getData('text/plain'));
  };

  // Maps a pointer position on the scaled canvas to device video pixels
  const toVideo = (e: PointerEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const { width, height } = videoSize.current;
    return {
      x: Math.round(((e.clientX - rect.left) / rect.width) * width),
      y: Math.round(((e.clientY - rect.top) / rect.height) * height),
    };
  };

  const onPointerDown = (e: PointerEvent<HTMLCanvasElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    inputRef.current?.focus({ preventScroll: true });
    touching.current = true;
    const p = toVideo(e);
    sendTouch(TOUCH_DOWN, p.x, p.y);
  };

  // Moves are coalesced to one per animation frame to keep the control socket light
  const onPointerMove = (e: PointerEvent<HTMLCanvasElement>) => {
    if (!touching.current) return;
    const first = pendingMove.current === null;
    pendingMove.current = toVideo(e);
    if (first) {
      requestAnimationFrame(() => {
        const p = pendingMove.current;
        pendingMove.current = null;
        if (p && touching.current) sendTouch(TOUCH_MOVE, p.x, p.y);
      });
    }
  };

  const onPointerUp = (e: PointerEvent<HTMLCanvasElement>) => {
    if (!touching.current) return;
    touching.current = false;
    pendingMove.current = null;
    const p = toVideo(e);
    sendTouch(TOUCH_UP, p.x, p.y);
  };

  return (
    <div className="mirror-wrap">
      <canvas
        ref={canvasRef}
        className={`mirror-canvas${interactive ? ' interactive' : ''}`}
        width={576}
        height={1280}
        onPointerDown={interactive ? onPointerDown : undefined}
        onPointerMove={interactive ? onPointerMove : undefined}
        onPointerUp={interactive ? onPointerUp : undefined}
        onPointerCancel={interactive ? onPointerUp : undefined}
      />
      {interactive && (
        <textarea
          ref={inputRef}
          className="mirror-input"
          aria-label="Gõ phím vào điện thoại"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          onKeyDown={onKeyDown}
          onInput={onInput}
          onCompositionEnd={onCompositionEnd}
          onPaste={onPaste}
        />
      )}
      {status === 'starting' && <div className="mirror-status">Đang kết nối màn hình...</div>}
    </div>
  );
}
