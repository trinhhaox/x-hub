import { invoke } from '@tauri-apps/api/core';
import type { NetworkDevice } from './network-scan-panel';

// Wireless Android devices the user connected before, reconnected automatically on launch.
// bonjourName (the serial in the Bonjour instance name) lets us find the phone again after
// its IP or Wireless Debugging port changes; address alone covers Tailscale/manual IPs.
export interface RememberedDevice {
  address: string;
  bonjourName?: string;
}

const STORAGE_KEY = 'xhub.rememberedDevices';

export function loadRemembered(): RememberedDevice[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]');
    return Array.isArray(parsed)
      ? parsed.filter((d): d is RememberedDevice => typeof d?.address === 'string')
      : [];
  } catch (err) {
    console.warn('Không đọc được danh sách thiết bị đã lưu', err);
    return [];
  }
}

function save(list: RememberedDevice[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(list));
  } catch (err) {
    console.warn('Không lưu được danh sách thiết bị', err);
  }
}

const isSame = (a: RememberedDevice, b: RememberedDevice) =>
  a.address === b.address || (!!a.bonjourName && a.bonjourName === b.bonjourName);

export function rememberDevice(device: RememberedDevice) {
  save([...loadRemembered().filter((d) => !isSame(d, device)), device]);
}

/** Remembers wireless adb serials ("ip:port") that are connected but not saved yet. */
export function rememberConnected(serials: string[]) {
  const known = loadRemembered();
  const fresh = serials.filter((s) => s.includes(':') && !known.some((k) => k.address === s));
  if (fresh.length > 0) save([...known, ...fresh.map((address) => ({ address }))]);
}

export function forgetDevice(address: string) {
  save(loadRemembered().filter((d) => d.address !== address));
}

/** Scans the network and reconnects remembered devices; returns how many were newly connected. */
export async function reconnectRemembered(): Promise<number> {
  const known = loadRemembered();
  if (known.length === 0) return 0;
  const found = await invoke<NetworkDevice[]>('scan_network_devices').catch(() => [] as NetworkDevice[]);

  const results = await Promise.all(
    known.map(async (k) => {
      const match = found.find(
        (d) =>
          d.platform === 'Android' &&
          d.kind === 'connect' &&
          ((!!k.bonjourName && d.name === k.bonjourName) || d.address === k.address)
      );
      // Learn the Bonjour name so the phone is still found after its IP changes
      const entry = match && !k.bonjourName ? { ...k, bonjourName: match.name } : k;
      if (entry !== k) rememberDevice(entry);
      if (match?.connected) return false;
      const address = match?.address ?? k.address;
      try {
        const res = await invoke<string>('connect_adb_device', { ipPort: address });
        if (address !== k.address) rememberDevice({ ...entry, address });
        return !res.includes('already');
      } catch {
        return false; // phone is off or out of range; keep it remembered for next time
      }
    })
  );
  return results.filter(Boolean).length;
}
