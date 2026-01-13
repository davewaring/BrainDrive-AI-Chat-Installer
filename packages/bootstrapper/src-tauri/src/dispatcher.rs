use crate::system_info;
use serde_json::{json, Value};
use std::net::TcpListener;
use std::process::Stdio;
use tokio::process::Command;

/// Detect system information
pub async fn detect_system() -> Result<Value, String> {
    let info = system_info::detect().await?;
    Ok(json!({
        "os": info.os,
        "arch": info.arch,
        "hostname": info.hostname,
        "home_dir": info.home_dir,
        "conda_installed": info.conda_installed,
        "git_installed": info.git_installed,
        "node_installed": info.node_installed,
        "ollama_installed": info.ollama_installed,
        "braindrive_exists": info.braindrive_exists
    }))
}

/// Execute a shell command
///
/// NOTE: This is a temporary implementation for Phase 1 demo purposes.
/// Phase 2 will replace this with audited helper functions that only
/// allow specific, validated commands.
pub async fn run_command(command: &str) -> Result<Value, String> {
    // For Phase 1, we execute commands but log them
    // Phase 2 will add command validation/allowlisting
    eprintln!("Executing command: {}", command);

    let shell = if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "sh"
    };

    let shell_arg = if cfg!(target_os = "windows") {
        "/C"
    } else {
        "-c"
    };

    let output = Command::new(shell)
        .arg(shell_arg)
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(json!({
        "success": output.status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code
    }))
}

/// Check if a port is available
pub async fn check_port(port: u16) -> Result<Value, String> {
    let addr = format!("127.0.0.1:{}", port);
    let available = TcpListener::bind(&addr).is_ok();

    Ok(json!({
        "port": port,
        "available": available
    }))
}

/// Start BrainDrive services
pub async fn start_braindrive(frontend_port: u16, backend_port: u16) -> Result<Value, String> {
    let home_dir = dirs::home_dir().ok_or("Could not determine home directory")?;
    let braindrive_path = home_dir.join("BrainDrive");

    if !braindrive_path.exists() {
        return Err("BrainDrive is not installed. Please install it first.".to_string());
    }

    eprintln!(
        "Starting BrainDrive on ports: frontend={}, backend={}",
        frontend_port, backend_port
    );

    // Start backend using conda run
    let backend_path = braindrive_path.join("backend");
    let backend_cmd = format!(
        "cd {} && conda run -n BrainDriveDev uvicorn main:app --host 0.0.0.0 --port {} &",
        backend_path.display(),
        backend_port
    );

    let backend_result = Command::new("sh")
        .arg("-c")
        .arg(&backend_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    if let Err(e) = backend_result {
        return Err(format!("Failed to start backend: {}", e));
    }

    // Give backend a moment to start
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Start frontend
    let frontend_path = braindrive_path.join("frontend");
    let frontend_cmd = format!(
        "cd {} && npm run dev -- --host localhost --port {} &",
        frontend_path.display(),
        frontend_port
    );

    let frontend_result = Command::new("sh")
        .arg("-c")
        .arg(&frontend_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    if let Err(e) = frontend_result {
        return Err(format!("Failed to start frontend: {}", e));
    }

    Ok(json!({
        "success": true,
        "message": "BrainDrive services started",
        "frontend_port": frontend_port,
        "backend_port": backend_port,
        "frontend_url": format!("http://localhost:{}", frontend_port),
        "backend_url": format!("http://localhost:{}", backend_port)
    }))
}

/// Stop BrainDrive services
///
/// TODO: Implement actual BrainDrive stop logic in Phase 2
pub async fn stop_braindrive() -> Result<Value, String> {
    eprintln!("Stopping BrainDrive");

    // For Phase 1, return a mock success
    Ok(json!({
        "success": true,
        "message": "BrainDrive stop initiated"
    }))
}

/// Restart BrainDrive services
///
/// TODO: Implement actual BrainDrive restart logic in Phase 2
pub async fn restart_braindrive() -> Result<Value, String> {
    eprintln!("Restarting BrainDrive");

    // For Phase 1, just call stop then start
    stop_braindrive().await?;
    start_braindrive(5173, 8005).await
}
