use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;
use futures_util::stream::SplitSink;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream, MaybeTlsStream};
use tokio::net::TcpStream;

mod websocket;
mod system_info;
mod dispatcher;
pub mod process_manager;
pub mod logging;

// Type alias for the WebSocket sender
pub type WsSender = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

// Application state
pub struct AppState {
    ws_connected: Arc<Mutex<bool>>,
    backend_url: Arc<Mutex<String>>,
    ws_sender: Arc<Mutex<Option<WsSender>>>,
    process_state: process_manager::ProcessState,
}

impl Default for AppState {
    fn default() -> Self {
        let default_url = std::env::var("BRAINDRIVE_BACKEND_URL")
            .unwrap_or_else(|_| "wss://backend-production-f57a.up.railway.app".to_string());
        Self {
            ws_connected: Arc::new(Mutex::new(false)),
            backend_url: Arc::new(Mutex::new(default_url)),
            ws_sender: Arc::new(Mutex::new(None)),
            process_state: process_manager::new_process_state(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConnectionStatus {
    connected: bool,
    url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GpuInfo {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vram_gb: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
    os: String,
    arch: String,
    hostname: String,
    home_dir: String,
    /// Whether isolated conda is installed at ~/BrainDrive/miniconda3
    conda_installed: bool,
    /// Whether the BrainDrive conda environment exists with git/node
    braindrive_env_ready: bool,
    ollama_installed: bool,
    ollama_running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ollama_version: Option<String>,
    braindrive_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_physical_cores: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_logical_cores: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_gb: Option<f64>,
    #[serde(default)]
    gpus: Vec<GpuInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_free_gb: Option<f64>,
}

// Tauri commands
#[tauri::command]
async fn get_connection_status(state: State<'_, AppState>) -> Result<ConnectionStatus, String> {
    let connected = *state.ws_connected.lock().await;
    let url = state.backend_url.lock().await.clone();
    Ok(ConnectionStatus { connected, url })
}

#[tauri::command]
async fn connect_to_backend(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: Option<String>,
) -> Result<(), String> {
    let backend_url = match url {
        Some(u) => {
            *state.backend_url.lock().await = u.clone();
            u
        }
        None => state.backend_url.lock().await.clone(),
    };

    websocket::connect(
        app,
        state.ws_connected.clone(),
        state.ws_sender.clone(),
        state.process_state.clone(),
        &backend_url,
    ).await
}

#[tauri::command]
async fn disconnect_from_backend(state: State<'_, AppState>) -> Result<(), String> {
    *state.ws_connected.lock().await = false;
    Ok(())
}

#[tauri::command]
async fn get_system_info() -> Result<SystemInfo, String> {
    system_info::detect().await
}

#[tauri::command]
async fn start_braindrive(
    state: State<'_, AppState>,
    frontend_port: Option<u16>,
    backend_port: Option<u16>,
) -> Result<serde_json::Value, String> {
    let fp = frontend_port.unwrap_or(5173);
    let bp = backend_port.unwrap_or(8005);
    dispatcher::start_braindrive(fp, bp, &state.process_state).await
}

#[tauri::command]
async fn stop_braindrive(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    dispatcher::stop_braindrive(&state.process_state).await
}

#[tauri::command]
async fn restart_braindrive(
    state: State<'_, AppState>,
    frontend_port: Option<u16>,
    backend_port: Option<u16>,
) -> Result<serde_json::Value, String> {
    let fp = frontend_port.unwrap_or(5173);
    let bp = backend_port.unwrap_or(8005);
    dispatcher::restart_braindrive(fp, bp, &state.process_state).await
}

#[tauri::command]
async fn get_braindrive_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    dispatcher::get_braindrive_status(&state.process_state).await
}

#[tauri::command]
async fn export_logs() -> Result<String, String> {
    tracing::info!("Exporting logs for sharing");
    let path = logging::export_logs_for_sharing(Some(2000))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
async fn get_recent_logs(count: Option<usize>) -> Result<Vec<String>, String> {
    let num_lines = count.unwrap_or(50);
    logging::get_recent_events(num_lines)
}

#[tauri::command]
async fn get_log_directory() -> Result<String, String> {
    Ok(logging::get_log_dir().to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging system
    if let Err(e) = logging::init_logging() {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    // Clean up old logs (keep last 7 days)
    if let Err(e) = logging::cleanup_old_logs(7) {
        tracing::warn!(error = %e, "Failed to clean up old logs");
    }

    tracing::info!("BrainDrive Installer starting");

    // Create app state and keep a clone of process_state for the exit handler
    let app_state = AppState::default();
    let exit_process_state = app_state.process_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_connection_status,
            connect_to_backend,
            disconnect_from_backend,
            get_system_info,
            start_braindrive,
            stop_braindrive,
            restart_braindrive,
            get_braindrive_status,
            export_logs,
            get_recent_logs,
            get_log_directory,
        ])
        .on_window_event(move |_window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                tracing::info!("Window close requested, stopping BrainDrive processes");

                // Clone the process_state for the async block
                let process_state = exit_process_state.clone();

                // Stop BrainDrive processes synchronously before exit
                tauri::async_runtime::block_on(async move {
                    match dispatcher::stop_braindrive(&process_state).await {
                        Ok(result) => {
                            tracing::info!(result = %result, "BrainDrive stopped on exit");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to stop BrainDrive on exit");
                        }
                    }
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
