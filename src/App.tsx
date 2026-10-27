import { useEffect, useState, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import {
  Smartphone,
  Search,
  Plus,
  Maximize2,
  ZoomIn,
  ZoomOut,
  Camera,
  Power,
  Volume2,
  VolumeX,
  SlidersHorizontal,
  FileText,
  Info,
  RefreshCw,
  Monitor,
  ArrowUpCircle,
  ArrowLeft,
  Home,
  Layers,
  SunMedium
} from 'lucide-react';
import './App.css';

interface DeviceItem {
  id: string;
  name: string;
  platform: 'iOS' | 'Android' | string;
  reality: 'physical' | 'simulated' | string;
  state: string;
  os_version: string;
  model: string;
  product_type: string;
  serial_number: string;
  udid: string;
  core_device_id: string;
  transport_type: string;
  pairing_state: string;
  ddi_services: string;
  developer_mode: string;
  file_system: string;
  capacity: string;
  ecid: string;
  screen_viewing_url?: string;
}

export function App() {
  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [activeTab, setActiveTab] = useState<'info' | 'apps' | 'profiles'>('info');
  const [loading, setLoading] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  
  // Modal state
  const [showAddModal, setShowAddModal] = useState(false);
  const [adbIpInput, setAdbIpInput] = useState('100.76.122.48:5555');
  const [envWarning, setEnvWarning] = useState<string | null>(null);

  // Auto-updater state
  const [availableUpdate, setAvailableUpdate] = useState<any | null>(null);
  const [isUpdating, setIsUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<string | null>(null);

  const checkForUpdates = async (manual = false) => {
    try {
      if (manual) showToast('Đang kiểm tra bản cập nhật mới...');
      const update = await check();
      if (update?.available) {
        setAvailableUpdate(update);
        showToast(`Có bản cập nhật mới: v${update.version}`);
      } else {
        if (manual) showToast('Xhub đang ở phiên bản mới nhất!');
      }
    } catch (e: any) {
      if (manual) {
        const msg = String(e?.message || e);
        if (msg.includes('Could not fetch a valid release JSON') || msg.includes('404')) {
          showToast('Xhub đang ở phiên bản mới nhất!');
        } else {
          showToast(`Chưa thể kiểm tra cập nhật: ${msg}`);
        }
      }
    }
  };

  const handleInstallUpdate = async () => {
    if (!availableUpdate) return;
    try {
      setIsUpdating(true);
      setUpdateProgress('Đang tải bản cập nhật...');
      let downloaded = 0;
      let total = 0;
      await availableUpdate.downloadAndInstall((event: any) => {
        if (event.event === 'Started') {
          total = event.data.contentLength || 0;
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          if (total > 0) {
            setUpdateProgress(`Đang tải: ${Math.round((downloaded / total) * 100)}%`);
          }
        } else if (event.event === 'Finished') {
          setUpdateProgress('Đã tải xong! Đang cài đặt và khởi động lại...');
        }
      });
      await relaunch();
    } catch (err: any) {
      setIsUpdating(false);
      setUpdateProgress(null);
      showToast(`Lỗi cập nhật: ${err}`);
    }
  };

  // Check system dependencies and auto check updates
  useEffect(() => {
    invoke<{ has_adb: boolean; has_scrcpy: boolean }>('check_system_environment')
      .then((status) => {
        if (!status.has_adb || !status.has_scrcpy) {
          const missing = [];
          if (!status.has_adb) missing.push('adb');
          if (!status.has_scrcpy) missing.push('scrcpy');
          setEnvWarning(`Thiếu công cụ: ${missing.join(', ')}. Chạy: brew install scrcpy android-platform-tools`);
        }
      })
      .catch((e) => console.error(e));

    // Check updates silently on startup
    checkForUpdates(false);
  }, []);

  // Fetch devices
  const fetchDevices = async (isManual = false) => {
    try {
      if (isManual) setLoading(true);
      const res = await invoke<DeviceItem[]>('get_all_devices');
      setDevices(res);
      setSelectedId((prev) => {
        // 1. Nếu người dùng đã chọn thiết bị và thiết bị vẫn đang kết nối -> giữ nguyên tuyệt đối
        if (prev && res.some((d) => d.id === prev)) {
          return prev;
        }
        // 2. Nếu chưa từng chọn thiết bị nào (lần đầu khởi động app)
        if (!prev && res.length > 0) {
          return res[0].id;
        }
        // 3. Nếu thiết bị trước đó đã bị ngắt kết nối hoàn toàn
        return res[0]?.id || null;
      });
    } catch (err) {
      console.error('Error scanning devices:', err);
    } finally {
      if (isManual) setLoading(false);
    }
  };

  useEffect(() => {
    fetchDevices(true);
    const interval = setInterval(() => {
      fetchDevices(false);
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  // Filter devices
  const filtered = useMemo(() => {
    if (!searchQuery.trim()) return devices;
    const q = searchQuery.toLowerCase();
    return devices.filter(
      (d) =>
        d.name.toLowerCase().includes(q) ||
        d.model.toLowerCase().includes(q) ||
        d.udid.toLowerCase().includes(q)
    );
  }, [devices, searchQuery]);

  const physicalDevices = useMemo(
    () => filtered.filter((d) => d.reality === 'physical'),
    [filtered]
  );

  const simulatorDevices = useMemo(
    () => filtered.filter((d) => d.reality === 'simulated'),
    [filtered]
  );

  const selectedDevice = useMemo(
    () => devices.find((d) => d.id === selectedId) || devices[0] || null,
    [devices, selectedId]
  );

  const showToast = (msg: string) => {
    setStatusMessage(msg);
    setTimeout(() => setStatusMessage(null), 3500);
  };

  // Launch screen mirroring
  const handleViewScreen = async () => {
    if (!selectedDevice) return;
    try {
      showToast(`Đang kết nối màn hình ${selectedDevice.name}...`);
      const res = await invoke<string>('launch_view_screen', {
        deviceId: selectedDevice.id,
        platform: selectedDevice.platform,
        reality: selectedDevice.reality,
        screenUrl: selectedDevice.screen_viewing_url || null,
      });
      showToast(res);
    } catch (err: any) {
      showToast(`Lỗi: ${err}`);
    }
  };

  // Take screenshot
  const handleScreenshot = async () => {
    if (!selectedDevice) return;
    try {
      showToast('Đang chụp ảnh màn hình...');
      const res = await invoke<string>('take_screenshot', {
        deviceId: selectedDevice.id,
        platform: selectedDevice.platform,
      });
      showToast(res);
    } catch (err: any) {
      showToast(`Lỗi chụp: ${err}`);
    }
  };

  // Android key events
  const handleKey = async (code: number, name: string) => {
    if (!selectedDevice || selectedDevice.platform !== 'Android') return;
    try {
      await invoke('send_android_keyevent', {
        serial: selectedDevice.id,
        keycode: code,
      });
      showToast(`Đã bấm ${name}`);
    } catch (err: any) {
      showToast(`Lỗi: ${err}`);
    }
  };

  // Connect ADB
  const handleConnectAdb = async () => {
    if (!adbIpInput.trim()) return;
    try {
      showToast(`Đang kết nối ${adbIpInput}...`);
      const res = await invoke<string>('connect_adb_device', { ipPort: adbIpInput });
      showToast(res);
      setShowAddModal(false);
      fetchDevices(true);
    } catch (err: any) {
      showToast(`Lỗi kết nối: ${err}`);
    }
  };

  // Disconnect ADB
  const handleDisconnectAdb = async () => {
    if (!selectedDevice || selectedDevice.platform !== 'Android') return;
    try {
      const res = await invoke<string>('disconnect_adb_device', { ipPort: selectedDevice.id });
      showToast(res);
      fetchDevices(true);
    } catch (err: any) {
      showToast(`Lỗi ngắt kết nối: ${err}`);
    }
  };

  const truncateId = (idStr?: string) => {
    if (!idStr) return 'N/A';
    if (idStr.length <= 16) return idStr;
    return `${idStr.slice(0, 8)}...${idStr.slice(-8)}`;
  };

  return (
    <div className="device-hub-app">
      {/* CỘT 1: SIDEBAR */}
      <div className="sidebar-pane">
        <div className="toolbar-header">
          <div className="mac-traffic-placeholder" />
          <div style={{ display: 'flex', gap: '4px' }}>
            <button
              className="icon-btn"
              title="Thêm thiết bị không dây (ADB/Tailscale)"
              onClick={() => setShowAddModal(true)}
            >
              <Plus size={16} />
            </button>
            <button
              className="icon-btn"
              title="Làm mới danh sách"
              onClick={() => fetchDevices(true)}
            >
              <RefreshCw size={14} className={loading ? 'animate-spin' : ''} />
            </button>
          </div>
        </div>

        <div className="search-wrapper">
          <div className="search-box">
            <Search size={14} color="var(--text-muted)" />
            <input
              type="text"
              placeholder="Search"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
        </div>

        <div className="sidebar-list">
          {/* Physical / Available Section */}
          <div className="list-section-title">Available</div>
          {physicalDevices.map((dev) => {
            const isSelected = selectedId === dev.id;
            const isWireless = dev.transport_type.includes('Wireless');
            return (
              <div
                key={dev.id}
                className={`device-row ${isSelected ? 'selected' : ''}`}
                onClick={() => setSelectedId(dev.id)}
              >
                <div className="device-icon-box">
                  <Smartphone size={22} strokeWidth={1.5} />
                </div>
                <div className="device-info-col">
                  <div className="device-title">{dev.name}</div>
                  <div className="device-subtitle">
                    {dev.model} • {isWireless ? 'Wi-Fi/Tailscale' : 'USB'}
                  </div>
                </div>
                <div className="device-badge">
                  {dev.os_version.replace(/^(iOS|Android)\s*/, '')}
                </div>
              </div>
            );
          })}

          {/* Simulators Section */}
          {simulatorDevices.length > 0 && (
            <>
              <div className="list-section-title" style={{ marginTop: '12px' }}>
                Simulators
              </div>
              {simulatorDevices.map((dev) => {
                const isSelected = selectedId === dev.id;
                return (
                  <div
                    key={dev.id}
                    className={`device-row ${isSelected ? 'selected' : ''}`}
                    onClick={() => setSelectedId(dev.id)}
                  >
                    <div className="device-icon-box">
                      <Smartphone size={22} strokeWidth={1.5} color="var(--text-muted)" />
                    </div>
                    <div className="device-info-col">
                      <div className="device-title">{dev.name}</div>
                      <div className="device-subtitle">Simulator</div>
                    </div>
                    <div className="device-badge">
                      {dev.os_version.replace(/^(iOS|Android)\s*/, '')}
                    </div>
                  </div>
                );
              })}
            </>
          )}
        </div>
      </div>

      {/* CỘT 2: CENTER CANVAS & CONTROLLER */}
      <div className="center-pane">
        <div className="toolbar-header">
          <div>
            {selectedDevice && (
              <div style={{ display: 'flex', flexDirection: 'column' }}>
                <span style={{ fontSize: '13px', fontWeight: 600 }}>
                  {selectedDevice.name}
                </span>
                <span style={{ fontSize: '11px', color: 'var(--text-secondary)' }}>
                  {selectedDevice.os_version}
                </span>
              </div>
            )}
          </div>

          <div style={{ display: 'flex', gap: '6px', alignItems: 'center' }}>
            {selectedDevice?.platform === 'Android' && (
              <>
                <button
                  className="icon-btn"
                  title="Quay lại (Back)"
                  onClick={() => handleKey(4, 'Quay lại')}
                >
                  <ArrowLeft size={15} />
                </button>
                <button
                  className="icon-btn"
                  title="Màn hình chính (Home)"
                  onClick={() => handleKey(3, 'Home')}
                >
                  <Home size={15} />
                </button>
                <button
                  className="icon-btn"
                  title="Đa nhiệm / Ứng dụng gần đây (Recents)"
                  onClick={() => handleKey(187, 'Đa nhiệm')}
                >
                  <Layers size={15} />
                </button>
                <div style={{ width: '1px', height: '16px', background: 'var(--border-subtle)', margin: '0 2px' }} />
                <button
                  className="icon-btn"
                  title="Nút Nguồn (Power)"
                  onClick={() => handleKey(26, 'Nguồn')}
                >
                  <Power size={15} />
                </button>
                <button
                  className="icon-btn"
                  title="Đánh thức màn hình (Wake up)"
                  onClick={() => handleKey(224, 'Đánh thức')}
                >
                  <SunMedium size={15} />
                </button>
                <button
                  className="icon-btn"
                  title="Tăng âm lượng"
                  onClick={() => handleKey(24, 'Volume +')}
                >
                  <Volume2 size={15} />
                </button>
                <button
                  className="icon-btn"
                  title="Giảm âm lượng"
                  onClick={() => handleKey(25, 'Volume -')}
                >
                  <VolumeX size={15} />
                </button>
                <div style={{ width: '1px', height: '16px', background: 'var(--border-subtle)', margin: '0 2px' }} />
              </>
            )}
            <button
              className="icon-btn"
              title="Chụp ảnh màn hình"
              onClick={handleScreenshot}
            >
              <Camera size={15} />
            </button>
            <div style={{ width: '1px', height: '16px', background: 'var(--border-subtle)', margin: '0 4px' }} />
            <button className="icon-btn" title="Phóng to">
              <ZoomIn size={15} />
            </button>
            <button className="icon-btn" title="Thu nhỏ">
              <ZoomOut size={15} />
            </button>
            <button className="icon-btn" title="Toàn màn hình">
              <Maximize2 size={15} />
            </button>
          </div>
        </div>

        {envWarning && (
          <div
            style={{
              background: '#fff3cd',
              color: '#856404',
              padding: '8px 14px',
              fontSize: '12px',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              borderBottom: '1px solid #ffeeba',
            }}
          >
            <span>⚠️ {envWarning}</span>
            <button
              style={{
                background: '#856404',
                color: '#fff',
                border: 'none',
                padding: '3px 8px',
                borderRadius: '4px',
                fontSize: '11px',
                cursor: 'pointer',
              }}
              onClick={() => {
                navigator.clipboard.writeText('brew install scrcpy android-platform-tools');
                showToast('Đã copy lệnh cài đặt!');
              }}
            >
              Copy lệnh
            </button>
          </div>
        )}

        {/* Center Mockup */}
        <div className="center-canvas">
          {selectedDevice ? (
            <div className="mockup-container">
              <div className="phone-bezel">
                <div
                  className={`phone-screen ${
                    selectedDevice.platform === 'Android' ? 'android-screen' : ''
                  }`}
                  onClick={handleViewScreen}
                  style={{ cursor: 'pointer' }}
                  title="Nhấp để mở màn hình điều khiển scrcpy"
                >
                  {/* Dynamic Island cho iOS hoặc Punch Hole cho Android */}
                  {selectedDevice.platform === 'iOS' ? (
                    <div className="dynamic-island" />
                  ) : (
                    <div className="punch-hole" />
                  )}

                  {selectedDevice.platform === 'Android' && (
                    <>
                      <div className="screen-center-prompt">
                        <Monitor size={36} strokeWidth={1.6} color="rgba(255,255,255,0.95)" />
                        <span style={{ fontSize: '13px', fontWeight: 600, marginTop: '8px', color: '#fff' }}>
                          Mở màn hình điều khiển
                        </span>
                        <span style={{ fontSize: '11px', color: 'rgba(255,255,255,0.8)', marginTop: '2px' }}>
                          Nhấp để khởi chạy scrcpy
                        </span>
                      </div>

                      {/* Thanh 3 phím điều hướng ảo Android */}
                      <div className="android-nav-bar" onClick={(e) => e.stopPropagation()}>
                        <button
                          className="nav-btn"
                          title="Quay lại (Back)"
                          onClick={() => handleKey(4, 'Quay lại')}
                        >
                          ◀
                        </button>
                        <button
                          className="nav-btn"
                          title="Màn hình chính (Home)"
                          onClick={() => handleKey(3, 'Home')}
                        >
                          ●
                        </button>
                        <button
                          className="nav-btn"
                          title="Ứng dụng gần đây (Recents)"
                          onClick={() => handleKey(187, 'Đa nhiệm')}
                        >
                          ■
                        </button>
                      </div>
                    </>
                  )}
                </div>
              </div>

              <div style={{ textAlign: 'center' }}>
                <div style={{ fontSize: '15px', fontWeight: 600 }}>
                  {selectedDevice.name}
                </div>
                <div style={{ fontSize: '12px', color: 'var(--text-secondary)', marginTop: '2px' }}>
                  {selectedDevice.os_version}
                </div>
              </div>

              <button className="view-screen-btn" onClick={handleViewScreen}>
                <Monitor size={15} />
                <span>View Screen</span>
              </button>
            </div>
          ) : (
            <div style={{ color: 'var(--text-muted)', fontSize: '13px' }}>
              Không có thiết bị nào được chọn
            </div>
          )}

          {statusMessage && (
            <div
              style={{
                position: 'absolute',
                bottom: '20px',
                background: 'rgba(0,0,0,0.8)',
                color: '#fff',
                padding: '8px 16px',
                borderRadius: '20px',
                fontSize: '12px',
                backdropFilter: 'blur(8px)',
                boxShadow: '0 4px 12px rgba(0,0,0,0.15)',
              }}
            >
              {statusMessage}
            </div>
          )}
        </div>
      </div>

      {/* CỘT 3: INSPECTOR PANE */}
      <div className="inspector-pane">
        <div className="toolbar-header" style={{ justifyContent: 'flex-end', gap: '6px' }}>
          <button
            className="icon-btn"
            title="Kiểm tra bản cập nhật mới"
            onClick={() => checkForUpdates(true)}
          >
            <ArrowUpCircle size={15} color={availableUpdate ? 'var(--accent)' : undefined} />
          </button>
          <button className="icon-btn" title="Settings">
            <SlidersHorizontal size={15} />
          </button>
          <button className="icon-btn" title="Docs">
            <FileText size={15} />
          </button>
          <button className="icon-btn active" title="Device Info">
            <Info size={15} />
          </button>
        </div>

        <div className="inspector-tabs">
          <div className="segmented-pill">
            <button
              className={activeTab === 'info' ? 'active' : ''}
              onClick={() => setActiveTab('info')}
            >
              Info
            </button>
            <button
              className={activeTab === 'apps' ? 'active' : ''}
              onClick={() => setActiveTab('apps')}
            >
              Apps
            </button>
            <button
              className={activeTab === 'profiles' ? 'active' : ''}
              onClick={() => setActiveTab('profiles')}
            >
              Profiles
            </button>
          </div>
        </div>

        {selectedDevice && activeTab === 'info' && (
          <div className="inspector-table">
            <div className="inspector-row">
              <span className="inspector-label">Name</span>
              <span className="inspector-value">{selectedDevice.name}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">OS</span>
              <span className="inspector-value">{selectedDevice.os_version}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">CoreDevice ID</span>
              <span className="inspector-value" title={selectedDevice.core_device_id}>
                {truncateId(selectedDevice.core_device_id)}
              </span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">DDI Services</span>
              <span className="inspector-value">{selectedDevice.ddi_services}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Developer Mode</span>
              <span className="inspector-value">{selectedDevice.developer_mode}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">File System</span>
              <span className="inspector-value">{selectedDevice.file_system}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Capacity</span>
              <span className="inspector-value">{selectedDevice.capacity}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">ECID</span>
              <span className="inspector-value">{selectedDevice.ecid}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Model</span>
              <span className="inspector-value">{selectedDevice.model}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Platform</span>
              <span className="inspector-value">{selectedDevice.platform}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Product Type</span>
              <span className="inspector-value">{selectedDevice.product_type}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Serial Number</span>
              <span className="inspector-value">{selectedDevice.serial_number}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">UDID</span>
              <span className="inspector-value" title={selectedDevice.udid}>
                {truncateId(selectedDevice.udid)}
              </span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Pairing State</span>
              <span className="inspector-value">{selectedDevice.pairing_state}</span>
            </div>
            <div className="inspector-row">
              <span className="inspector-label">Transport Type</span>
              <span className="inspector-value">{selectedDevice.transport_type}</span>
            </div>
          </div>
        )}

        {selectedDevice && activeTab === 'apps' && (
          <div style={{ padding: '16px', fontSize: '12px', color: 'var(--text-secondary)' }}>
            <p>Hỗ trợ kéo thả file .apk hoặc .ipa vào đây để cài đặt trực tiếp vào {selectedDevice.name}.</p>
          </div>
        )}

        {selectedDevice && activeTab === 'profiles' && (
          <div style={{ padding: '16px', fontSize: '12px', color: 'var(--text-secondary)' }}>
            <p>Không có provisioning profile nào trên thiết bị này.</p>
          </div>
        )}

        <div className="inspector-footer">
          {selectedDevice?.platform === 'Android' && selectedDevice.transport_type.includes('Wireless') ? (
            <button className="secondary-pill-btn" onClick={handleDisconnectAdb}>
              Ngắt kết nối không dây
            </button>
          ) : (
            <button className="secondary-pill-btn">Edit Visibility</button>
          )}
        </div>
      </div>

      {/* POPUP MODAL THÊM KẾT NỐI ADB / TAILSCALE */}
      {showAddModal && (
        <div className="modal-backdrop" onClick={() => setShowAddModal(false)}>
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <h3>Kết nối thiết bị Android không dây</h3>
            <p style={{ fontSize: '12px', color: 'var(--text-secondary)', marginBottom: '12px' }}>
              Nhập IP & Port ADB (Tailscale hoặc Wi-Fi nội bộ):
            </p>
            <input
              type="text"
              className="modal-input"
              value={adbIpInput}
              onChange={(e) => setAdbIpInput(e.target.value)}
              placeholder="100.76.122.48:5555"
              autoFocus
            />
            <div className="modal-actions">
              <button
                className="secondary-pill-btn"
                onClick={() => setShowAddModal(false)}
              >
                Hủy
              </button>
              <button
                className="secondary-pill-btn"
                style={{ background: 'var(--accent)', color: '#fff', border: 'none' }}
                onClick={handleConnectAdb}
              >
                Kết nối
              </button>
            </div>
          </div>
        </div>
      )}

      {/* POPUP CẬP NHẬT TỰ ĐỘNG KHI CÓ BẢN MỚI */}
      {availableUpdate && (
        <div className="modal-backdrop">
          <div className="modal-card">
            <h3>🎉 Đã có phiên bản mới: v{availableUpdate.version}</h3>
            <p style={{ fontSize: '12px', color: 'var(--text-secondary)', marginBottom: '14px', lineHeight: 1.5 }}>
              {availableUpdate.body || 'Bản phát hành mới với nhiều cải tiến hiệu năng và tính năng mới.'}
            </p>
            {updateProgress && (
              <div style={{ fontSize: '12px', color: 'var(--accent)', marginBottom: '12px', fontWeight: 500 }}>
                {updateProgress}
              </div>
            )}
            <div className="modal-actions">
              <button
                className="secondary-pill-btn"
                disabled={isUpdating}
                onClick={() => setAvailableUpdate(null)}
              >
                Để sau
              </button>
              <button
                className="secondary-pill-btn"
                disabled={isUpdating}
                style={{ background: 'var(--accent)', color: '#fff', border: 'none' }}
                onClick={handleInstallUpdate}
              >
                {isUpdating ? 'Đang cập nhật...' : 'Cập nhật ngay'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
