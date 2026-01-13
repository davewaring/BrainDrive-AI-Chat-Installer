use crate::dispatcher;
use crate::WsSender;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Incoming messages from the backend server
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum IncomingMessage {
    /// Tool call from Claude via the backend
    #[serde(rename = "detect_system")]
    DetectSystem { id: String },

    #[serde(rename = "run_command")]
    RunCommand { id: String, command: String },

    #[serde(rename = "check_port")]
    CheckPort { id: String, port: u16 },

    #[serde(rename = "start_braindrive")]
    StartBraindrive {
        id: String,
        #[serde(default = "default_frontend_port")]
        frontend_port: u16,
        #[serde(default = "default_backend_port")]
        backend_port: u16,
    },

    #[serde(rename = "stop_braindrive")]
    StopBraindrive { id: String },

    #[serde(rename = "restart_braindrive")]
    RestartBraindrive { id: String },

    /// Status update from backend
    #[serde(rename = "status_update")]
    StatusUpdate { bootstrapper_connected: bool },

    /// Catch-all for unknown messages
    #[serde(other)]
    Unknown,
}

fn default_frontend_port() -> u16 {
    5173
}
fn default_backend_port() -> u16 {
    8005
}

/// Outgoing messages to the backend server
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OutgoingMessage {
    #[serde(rename = "bootstrapper_connect")]
    BootstrapperConnect,

    #[serde(rename = "tool_result")]
    ToolResult {
        id: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Send a message to the backend via the WebSocket
pub async fn send_message(sender: &Arc<Mutex<Option<WsSender>>>, message: OutgoingMessage) -> Result<(), String> {
    let json = serde_json::to_string(&message)
        .map_err(|e| format!("Failed to serialize message: {}", e))?;

    let mut guard = sender.lock().await;
    if let Some(ref mut ws) = *guard {
        ws.send(Message::Text(json))
            .await
            .map_err(|e| format!("Failed to send message: {}", e))?;
        Ok(())
    } else {
        Err("WebSocket not connected".to_string())
    }
}

pub async fn connect(
    app: AppHandle,
    ws_connected: Arc<Mutex<bool>>,
    ws_sender: Arc<Mutex<Option<WsSender>>>,
    url: &str,
) -> Result<(), String> {
    let url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;

    let (ws_stream, _) = connect_async(url)
        .await
        .map_err(|e| format!("Failed to connect: {}", e))?;

    let (write, mut read) = ws_stream.split();

    // Store the sender
    {
        let mut sender_guard = ws_sender.lock().await;
        *sender_guard = Some(write);
    }

    // Mark as connected
    *ws_connected.lock().await = true;

    // Emit connection event to frontend
    app.emit("ws-connected", true).ok();

    // Send bootstrapper_connect message
    send_message(&ws_sender, OutgoingMessage::BootstrapperConnect).await?;

    // Spawn task to handle incoming messages
    let app_clone = app.clone();
    let ws_connected_clone = ws_connected.clone();
    let ws_sender_clone = ws_sender.clone();

    tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Emit raw message to frontend for logging
                    app_clone.emit("ws-message", text.clone()).ok();

                    // Parse and dispatch the message
                    match serde_json::from_str::<IncomingMessage>(&text) {
                        Ok(incoming) => {
                            handle_incoming_message(
                                incoming,
                                &app_clone,
                                &ws_sender_clone,
                            )
                            .await;
                        }
                        Err(e) => {
                            eprintln!("Failed to parse message: {} - {}", e, text);
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    cleanup_connection(&ws_connected_clone, &ws_sender_clone, &app_clone).await;
                    break;
                }
                Err(e) => {
                    eprintln!("WebSocket error: {}", e);
                    cleanup_connection(&ws_connected_clone, &ws_sender_clone, &app_clone).await;
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(())
}

async fn cleanup_connection(
    ws_connected: &Arc<Mutex<bool>>,
    ws_sender: &Arc<Mutex<Option<WsSender>>>,
    app: &AppHandle,
) {
    *ws_connected.lock().await = false;
    *ws_sender.lock().await = None;
    app.emit("ws-connected", false).ok();
}

async fn handle_incoming_message(
    message: IncomingMessage,
    app: &AppHandle,
    sender: &Arc<Mutex<Option<WsSender>>>,
) {
    match message {
        IncomingMessage::DetectSystem { id } => {
            let result = dispatcher::detect_system().await;
            send_tool_result(sender, id, result).await;
        }

        IncomingMessage::RunCommand { id, command } => {
            // Emit to UI that we're running a command
            app.emit("command-executing", command.clone()).ok();
            let result = dispatcher::run_command(&command).await;
            send_tool_result(sender, id, result).await;
        }

        IncomingMessage::CheckPort { id, port } => {
            let result = dispatcher::check_port(port).await;
            send_tool_result(sender, id, result).await;
        }

        IncomingMessage::StartBraindrive {
            id,
            frontend_port,
            backend_port,
        } => {
            app.emit("braindrive-starting", ()).ok();
            let result = dispatcher::start_braindrive(frontend_port, backend_port).await;
            send_tool_result(sender, id, result).await;
        }

        IncomingMessage::StopBraindrive { id } => {
            app.emit("braindrive-stopping", ()).ok();
            let result = dispatcher::stop_braindrive().await;
            send_tool_result(sender, id, result).await;
        }

        IncomingMessage::RestartBraindrive { id } => {
            app.emit("braindrive-restarting", ()).ok();
            let result = dispatcher::restart_braindrive().await;
            send_tool_result(sender, id, result).await;
        }

        IncomingMessage::StatusUpdate { .. } => {
            // Just informational, no response needed
        }

        IncomingMessage::Unknown => {
            // Ignore unknown message types
        }
    }
}

async fn send_tool_result(
    sender: &Arc<Mutex<Option<WsSender>>>,
    id: String,
    result: Result<serde_json::Value, String>,
) {
    let message = match result {
        Ok(data) => OutgoingMessage::ToolResult {
            id,
            success: true,
            data: Some(data),
            error: None,
        },
        Err(e) => OutgoingMessage::ToolResult {
            id,
            success: false,
            data: None,
            error: Some(e),
        },
    };

    if let Err(e) = send_message(sender, message).await {
        eprintln!("Failed to send tool result: {}", e);
    }
}
