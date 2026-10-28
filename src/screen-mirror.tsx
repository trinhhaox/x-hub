import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';

// Must match the message kinds in src-tauri/src/android_mirror.rs
const MSG_VIDEO_SIZE = 0;
const MSG_PACKET = 1;
const MSG_ENDED = 2;
const FLAG_CONFIG = 1;
const FLAG_KEY = 2;

const TOUCH_DOWN = 0;
const TOUCH_UP = 1;
const TOUCH_MOVE = 2;

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

const NAL_SPS = 7;
const NAL_PPS = 8;

const hex2 = (n: number) => n.toString(16).padStart(2, '0');

// Splits an Annex-B byte stream (00 00 01 / 00 00 00 01 start codes) into NAL units
function splitNals(data: Uint8Array): Uint8Array[] {
  const nals: Uint8Array[] = [];
  let start = -1;
  for (let i = 0; i + 2 < data.length; i++) {
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) {
      if (start >= 0) nals.push(data.subarray(start, data[i - 1] === 0 ? i - 1 : i));
      start = i + 3;
      i += 2;
    }
  }
  if (start >= 0 && start < data.length) nals.push(data.subarray(start));
  return nals;
}

// Annex-B -> AVCC (4-byte big-endian length prefixes), the layout WebCodecs expects with a description
function toAvcc(data: Uint8Array): Uint8Array {
  const nals = splitNals(data);
  const out = new Uint8Array(nals.reduce((n, nal) => n + 4 + nal.length, 0));
  const view = new DataView(out.buffer);
  let offset = 0;
  for (const nal of nals) {
    view.setUint32(offset, nal.length);
    out.set(nal, offset + 4);
    offset += 4 + nal.length;
  }
  return out;
}

// Builds the decoder config (codec string + avcC record) from scrcpy's SPS/PPS config packet
function decoderConfigFrom(data: Uint8Array): VideoDecoderConfig | null {
  const nals = splitNals(data);
  const sps = nals.find((n) => (n[0] & 0x1f) === NAL_SPS);
  const pps = nals.find((n) => (n[0] & 0x1f) === NAL_PPS);
  if (!sps || !pps || sps.length < 4) return null;
  const avcc = new Uint8Array([
    1, sps[1], sps[2], sps[3], 0xff, 0xe1,
    sps.length >> 8, sps.length & 0xff, ...sps,
    1, pps.length >> 8, pps.length & 0xff, ...pps,
  ]);
  return {
    codec: `avc1.${hex2(sps[1])}${hex2(sps[2])}${hex2(sps[3])}`,
    description: avcc,
    optimizeForLatency: true,
  };
}

// Live device screen rendered inside xhub with WebCodecs (Android: touch-controllable)
export function ScreenMirror({ deviceId, deviceName, platform, notify, onEnded }: ScreenMirrorProps) {
  const interactive = platform === 'Android';
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const videoSize = useRef({ width: 0, height: 0 });
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const touching = useRef(false);
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

  const sendTouch = (action: number, x: number, y: number) => {
    const { width, height } = videoSize.current;
    if (!width) return;
    invoke('android_mirror_touch', { serial: deviceId, action, x, y, width, height }).catch(() => undefined);
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
      {status === 'starting' && <div className="mirror-status">Đang kết nối màn hình...</div>}
    </div>
  );
}
