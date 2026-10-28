import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Radar, Wifi, Link2, KeyRound } from 'lucide-react';
import { rememberDevice } from './remembered-devices';

export interface NetworkDevice {
  id: string;
  name: string;
  platform: 'iOS' | 'Android' | string;
  address: string;
  kind: 'connect' | 'pairing' | string;
  connected: boolean;
}

interface NetworkScanPanelProps {
  notify: (msg: string) => void;
  onConnected: () => void;
}

// Lists Android (Wireless Debugging via mDNS) and paired iOS devices found on the same LAN
export function NetworkScanPanel({ notify, onConnected }: NetworkScanPanelProps) {
  const [devices, setDevices] = useState<NetworkDevice[]>([]);
  const [scanning, setScanning] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [pairingId, setPairingId] = useState<string | null>(null);
  const [pairCode, setPairCode] = useState('');

  const scan = async () => {
    setScanning(true);
    try {
      setDevices(await invoke<NetworkDevice[]>('scan_network_devices'));
    } catch (err) {
      notify(`Lỗi quét mạng: ${err}`);
    } finally {
      setScanning(false);
    }
  };

  // Scan once when the panel opens
  useEffect(() => {
    scan();
  }, []);

  const runAction = async (dev: NetworkDevice, action: () => Promise<string>) => {
    setBusyId(dev.id);
    try {
      notify(await action());
      setPairingId(null);
      setPairCode('');
      onConnected();
      await scan();
    } catch (err) {
      notify(`Lỗi: ${err}`);
    } finally {
      setBusyId(null);
    }
  };

  const connect = (dev: NetworkDevice) =>
    runAction(dev, async () => {
      if (dev.platform === 'iOS') return invoke<string>('connect_ios_device', { udid: dev.address });
      const res = await invoke<string>('connect_adb_device', { ipPort: dev.address });
      rememberDevice({ address: dev.address, bonjourName: dev.name });
      return res;
    });

  const pair = (dev: NetworkDevice) =>
    runAction(dev, () => invoke<string>('pair_adb_device', { address: dev.address, code: pairCode }));

  return (
    <div className="network-scan">
      <div className="network-scan-header">
        <span>Thiết bị cùng mạng</span>
        <button className="secondary-pill-btn" onClick={scan} disabled={scanning}>
          <Radar size={13} className={scanning ? 'spin' : undefined} />
          {scanning ? 'Đang quét...' : 'Quét lại'}
        </button>
      </div>

      {!scanning && devices.length === 0 && (
        <p className="network-scan-empty">
          Không tìm thấy thiết bị. Android: bật <strong>Gỡ lỗi không dây</strong> (Wireless Debugging) và dùng chung
          Wi-Fi với Mac. iOS: ghép đôi qua cáp một lần, sau đó sẽ hiện ở đây khi cùng mạng.
        </p>
      )}

      <ul className="network-scan-list">
        {devices.map((dev) => (
          <li key={dev.id} className="network-scan-item">
            <div className="network-scan-row">
              <Wifi size={14} />
              <div className="network-scan-info">
                <span className="network-scan-name">{dev.name}</span>
                <span className="network-scan-meta">
                  {dev.platform} • {dev.platform === 'iOS' ? 'Đã ghép đôi' : dev.address}
                  {dev.kind === 'pairing' && ' • chờ ghép đôi'}
                </span>
              </div>
              {dev.connected ? (
                <span className="network-scan-badge">Đã kết nối</span>
              ) : dev.kind === 'pairing' ? (
                <button
                  className="secondary-pill-btn"
                  disabled={busyId !== null}
                  onClick={() => setPairingId(pairingId === dev.id ? null : dev.id)}
                >
                  <KeyRound size={13} /> Ghép đôi
                </button>
              ) : (
                <button className="secondary-pill-btn" disabled={busyId !== null} onClick={() => connect(dev)}>
                  <Link2 size={13} /> {busyId === dev.id ? 'Đang nối...' : 'Kết nối'}
                </button>
              )}
            </div>

            {pairingId === dev.id && (
              <div className="network-scan-pair">
                <input
                  className="modal-input"
                  value={pairCode}
                  onChange={(e) => setPairCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
                  placeholder="Mã 6 số trên điện thoại"
                  autoFocus
                />
                <button
                  className="secondary-pill-btn"
                  style={{ background: 'var(--accent)', color: '#fff', border: 'none' }}
                  disabled={pairCode.length !== 6 || busyId !== null}
                  onClick={() => pair(dev)}
                >
                  {busyId === dev.id ? 'Đang ghép...' : 'Xác nhận'}
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
