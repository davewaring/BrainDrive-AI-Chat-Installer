use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

mod websocket;
mod system_info;

// Application state
pub struct AppState {
    ws_connected: Arc<Mutex<bool>>,
    backend_url: Arc<Mutex<String>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            ws_connected: Arc::new(Mutex::new(false)),
            backend_url: Arc::new(Mutex::new("ws://localhost:3000/ws".to_string())),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConnectionStatus {
    connected: bool,
    url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
    os: String,
    arch: String,
    hostname: String,
    home_dir: String,
    conda_installed: bool,
    git_installed: bool,
    node_installed: bool,
    ollama_installed: bool,
    braindrive_exists: bool,
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

    websocket::connect(app, state.ws_connected.clone(), &backend_url).await
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
async fn start_braindrive() -> Result<String, String> {
    // TODO: Implement BrainDrive start logic
    Ok("BrainDrive start requested".to_string())
}

#[tauri::command]
async fn stop_braindrive() -> Result<String, String> {
    // TODO: Implement BrainDrive stop logic
    Ok("BrainDrive stop requested".to_string())
}

#[tauri::command]
async fn restart_braindrive() -> Result<String, String> {
    // TODO: Implement BrainDrive restart logic
    Ok("BrainDrive restart requested".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_connection_status,
            connect_to_backend,
            disconnect_from_backend,
            get_system_info,
            start_braindrive,
            stop_braindrive,
            restart_braindrive,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
