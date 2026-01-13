use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, AppHandle};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[allow(dead_code)]  // Will be used for parsing incoming messages
pub enum WsMessage {
    #[serde(rename = "bootstrapper_connect")]
    BootstrapperConnect,
    #[serde(rename = "system_info")]
    SystemInfo { id: String, data: serde_json::Value },
    #[serde(rename = "command_result")]
    CommandResult {
        id: String,
        success: bool,
        output: String,
        exit_code: Option<i32>,
    },
    #[serde(rename = "status_update")]
    StatusUpdate { bootstrapper_connected: bool },
    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        tool: String,
        input: serde_json::Value,
    },
}

pub async fn connect(
    app: AppHandle,
    ws_connected: Arc<Mutex<bool>>,
    url: &str,
) -> Result<(), String> {
    let url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;

    let (ws_stream, _) = connect_async(url)
        .await
        .map_err(|e| format!("Failed to connect: {}", e))?;

    let (mut write, mut read) = ws_stream.split();

    // Mark as connected
    *ws_connected.lock().await = true;

    // Emit connection event to frontend
    app.emit("ws-connected", true).ok();

    // Send bootstrapper_connect message
    let connect_msg = serde_json::json!({
        "type": "bootstrapper_connect"
    });
    write
        .send(Message::Text(connect_msg.to_string()))
        .await
        .map_err(|e| format!("Failed to send connect message: {}", e))?;

    // Spawn task to handle incoming messages
    let app_clone = app.clone();
    let ws_connected_clone = ws_connected.clone();

    tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Forward message to frontend
                    app_clone.emit("ws-message", text).ok();
                }
                Ok(Message::Close(_)) => {
                    *ws_connected_clone.lock().await = false;
                    app_clone.emit("ws-connected", false).ok();
                    break;
                }
                Err(e) => {
                    eprintln!("WebSocket error: {}", e);
                    *ws_connected_clone.lock().await = false;
                    app_clone.emit("ws-connected", false).ok();
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(())
}
