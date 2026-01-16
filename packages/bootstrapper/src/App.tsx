import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './App.css';

interface GpuInfo {
  name: string;
  vram_gb?: number;
}

interface SystemInfo {
  os: string;
  arch: string;
  hostname: string;
  home_dir: string;
  conda_installed: boolean;
  braindrive_env_ready: boolean;
  ollama_installed: boolean;
  braindrive_exists: boolean;
  cpu_brand?: string;
  cpu_physical_cores?: number;
  cpu_logical_cores?: number;
  memory_gb?: number;
  disk_free_gb?: number;
  gpus?: GpuInfo[];
}

interface ConnectionStatus {
  connected: boolean;
  url: string;
}

type BrainDriveStatus = 'unknown' | 'stopped' | 'starting' | 'running' | 'stopping';

function App() {
  const [wsConnected, setWsConnected] = useState(false);
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [braindriveStatus, setBraindriveStatus] = useState<BrainDriveStatus>('unknown');
  const [logs, setLogs] = useState<string[]>([]);

  const addLog = (message: string) => {
    const timestamp = new Date().toLocaleTimeString();
    setLogs(prev => [...prev.slice(-49), `[${timestamp}] ${message}`]);
  };

  useEffect(() => {
    let unlistenConnectedFn: (() => void) | null = null;
    let unlistenMessageFn: (() => void) | null = null;

    // Initial setup - wait for listeners before connecting
    const init = async () => {
      try {
        // Set up listeners FIRST, before attempting connection
        unlistenConnectedFn = await listen<boolean>('ws-connected', (event) => {
          setWsConnected(event.payload);
          addLog(event.payload ? 'Connected to backend' : 'Disconnected from backend');
        });

        unlistenMessageFn = await listen<string>('ws-message', (event) => {
          try {
            const msg = JSON.parse(event.payload);
            addLog(`Received: ${msg.type}`);
          } catch {
            addLog(`Message: ${event.payload}`);
          }
        });

        // Get system info
        const info = await invoke<SystemInfo>('get_system_info');
        setSystemInfo(info);
        addLog(`System: ${info.os} ${info.arch}`);

        // Check current connection status
        const status = await invoke<ConnectionStatus>('get_connection_status');
        setWsConnected(status.connected);

        // Try to connect to backend (listeners are ready now)
        if (!status.connected) {
          addLog('Connecting to backend...');
          await invoke('connect_to_backend', { url: null });

          // Re-check status after connection attempt
          const newStatus = await invoke<ConnectionStatus>('get_connection_status');
          setWsConnected(newStatus.connected);
        }
      } catch (err) {
        addLog(`Error: ${err}`);
      }
    };

    init();

    return () => {
      if (unlistenConnectedFn) unlistenConnectedFn();
      if (unlistenMessageFn) unlistenMessageFn();
    };
  }, []);

  const handleStart = async () => {
    setBraindriveStatus('starting');
    addLog('Starting BrainDrive...');
    try {
      const result = await invoke<string>('start_braindrive');
      addLog(result);
      setBraindriveStatus('running');
    } catch (err) {
      addLog(`Start failed: ${err}`);
      setBraindriveStatus('stopped');
    }
  };

  const handleStop = async () => {
    setBraindriveStatus('stopping');
    addLog('Stopping BrainDrive...');
    try {
      const result = await invoke<string>('stop_braindrive');
      addLog(result);
      setBraindriveStatus('stopped');
    } catch (err) {
      addLog(`Stop failed: ${err}`);
    }
  };

  const handleRestart = async () => {
    setBraindriveStatus('stopping');
    addLog('Restarting BrainDrive...');
    try {
      const result = await invoke<string>('restart_braindrive');
      addLog(result);
      setBraindriveStatus('running');
    } catch (err) {
      addLog(`Restart failed: ${err}`);
    }
  };

  const getStatusColor = () => {
    if (!wsConnected) return 'status-disconnected';
    switch (braindriveStatus) {
      case 'running': return 'status-running';
      case 'starting':
      case 'stopping': return 'status-pending';
      default: return 'status-stopped';
    }
  };

  const getStatusText = () => {
    if (!wsConnected) return 'Disconnected';
    switch (braindriveStatus) {
      case 'running': return 'Running';
      case 'starting': return 'Starting...';
      case 'stopping': return 'Stopping...';
      case 'stopped': return 'Stopped';
      default: return 'Ready';
    }
  };

  const formatNumber = (value: number, digits = 1) => {
    return Number(value).toFixed(digits);
  };

  return (
    <div className="container">
      <header className="header">
        <h1>BrainDrive</h1>
        <p className="subtitle">AI Chat Installer</p>
      </header>

      <section className="status-section">
        <div className={`status-indicator ${getStatusColor()}`}>
          <span className="status-dot"></span>
          <span className="status-text">{getStatusText()}</span>
        </div>
        {wsConnected && (
          <p className="status-message">Connected to installation server</p>
        )}
        {!wsConnected && (
          <p className="status-message warning">
            Open braindrive.ai/install to begin
          </p>
        )}
      </section>

      <section className="controls-section">
        <h2>Controls</h2>
        <div className="button-group">
          <button
            onClick={handleStart}
            disabled={!wsConnected || braindriveStatus === 'running' || braindriveStatus === 'starting'}
            className="btn btn-start"
          >
            Start
          </button>
          <button
            onClick={handleStop}
            disabled={!wsConnected || braindriveStatus === 'stopped' || braindriveStatus === 'stopping'}
            className="btn btn-stop"
          >
            Stop
          </button>
          <button
            onClick={handleRestart}
            disabled={!wsConnected || braindriveStatus !== 'running'}
            className="btn btn-restart"
          >
            Restart
          </button>
        </div>
      </section>

      {systemInfo && (
        <section className="info-section">
          <h2>System</h2>
          <div className="info-grid">
            <div className="info-item">
              <span className="info-label">OS</span>
              <span className="info-value">{systemInfo.os} ({systemInfo.arch})</span>
            </div>
            {systemInfo.cpu_brand && (
              <div className="info-item">
                <span className="info-label">CPU</span>
                <span className="info-value">
                  {systemInfo.cpu_brand}
                  {systemInfo.cpu_physical_cores && (
                    <> · {systemInfo.cpu_physical_cores} cores</>
                  )}
                  {systemInfo.cpu_logical_cores && (
                    <> ({systemInfo.cpu_logical_cores} threads)</>
                  )}
                </span>
              </div>
            )}
            {systemInfo.memory_gb && (
              <div className="info-item">
                <span className="info-label">Memory</span>
                <span className="info-value">
                  {formatNumber(systemInfo.memory_gb, 1)} GB
                </span>
              </div>
            )}
            {systemInfo.disk_free_gb && (
              <div className="info-item">
                <span className="info-label">Disk Free</span>
                <span className="info-value">
                  {formatNumber(systemInfo.disk_free_gb, 1)} GB
                </span>
              </div>
            )}
            <div className="info-item">
              <span className="info-label">BrainDrive</span>
              <span className={`info-value ${systemInfo.braindrive_exists ? 'installed' : 'missing'}`}>
                {systemInfo.braindrive_exists ? 'Installed' : 'Not Installed'}
              </span>
            </div>
            <div className="info-item">
              <span className="info-label">Environment</span>
              <span className={`info-value ${systemInfo.braindrive_env_ready ? 'installed' : 'missing'}`}>
                {systemInfo.braindrive_env_ready ? 'Ready' : 'Not Set Up'}
              </span>
            </div>
            <div className="info-item">
              <span className="info-label">Ollama</span>
              <span className={`info-value ${systemInfo.ollama_installed ? 'installed' : 'missing'}`}>
                {systemInfo.ollama_installed ? 'Installed' : 'Not Installed'}
              </span>
            </div>
          </div>
          {systemInfo.gpus && systemInfo.gpus.length > 0 && (
            <div className="gpu-list">
              <span className="info-label">GPU</span>
              <ul>
                {systemInfo.gpus.map((gpu, index) => (
                  <li key={`${gpu.name}-${index}`}>
                    {gpu.name}
                    {typeof gpu.vram_gb === 'number' && (
                      <> · {formatNumber(gpu.vram_gb, 1)} GB VRAM</>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </section>
      )}

      <section className="logs-section">
        <h2>Activity Log</h2>
        <div className="logs-container">
          {logs.length === 0 ? (
            <p className="logs-empty">No activity yet</p>
          ) : (
            logs.map((log, i) => (
              <div key={i} className="log-entry">{log}</div>
            ))
          )}
        </div>
      </section>
    </div>
  );
}

export default App;
