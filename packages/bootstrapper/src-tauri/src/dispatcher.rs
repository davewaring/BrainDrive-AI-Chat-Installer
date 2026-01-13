use crate::process_manager::{
    self, is_port_in_use, kill_process, kill_process_on_port,
    spawn_detached, wait_for_port, wait_for_port_free, ProcessState, ServiceInfo,
};
use crate::system_info;
use regex::Regex;
use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

const DEFAULT_REPO_DIR: &str = "BrainDrive";
const CONDA_ENV_NAME: &str = "BrainDriveDev";

/// Detect system information and return it as JSON
pub async fn detect_system() -> Result<Value, String> {
    let info = system_info::detect().await?;
    serde_json::to_value(info).map_err(|e| format!("Failed to encode system info: {}", e))
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

/// Install or update the BrainDrive Conda environment with audited commands
pub async fn install_conda_env(
    env_name: &str,
    repo_path: Option<String>,
    environment_file: Option<String>,
) -> Result<Value, String> {
    ensure_command_available("conda")?;
    let sanitized_env = sanitize_env_name(env_name)?;
    let repo = resolve_repo_path(repo_path)?;
    let env_file = resolve_environment_file(&repo, environment_file)?;

    let mut command = Command::new("conda");
    command
        .arg("env")
        .arg("update")
        .arg("--name")
        .arg(&sanitized_env)
        .arg("--file")
        .arg(&env_file);

    let result = run_command(command).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "env_name": sanitized_env,
        "environment_file": env_file.to_string_lossy()
    }))
}

/// Install Ollama through a reviewed helper
pub async fn install_ollama() -> Result<Value, String> {
    if command_exists("ollama") {
        return Ok(json!({
            "success": true,
            "message": "Ollama already installed"
        }));
    }

    #[cfg(target_os = "windows")]
    {
        return Err("Automatic Ollama installation is not yet supported on Windows. Please install it manually from https://ollama.com/download.".to_string());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let script = "curl -fsSL https://ollama.com/install.sh | sh";
        let result = run_shell_script(script).await?;
        return Ok(json!({
            "success": result.success,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code
        }));
    }

    #[allow(unreachable_code)]
    Err("Unsupported platform for automatic Ollama installation".to_string())
}

/// Pull a vetted Ollama model
pub async fn pull_ollama_model(
    model: &str,
    registry: Option<String>,
    force: bool,
) -> Result<Value, String> {
    ensure_command_available("ollama")?;
    let sanitized_model = sanitize_model_name(model)?;

    let mut command = Command::new("ollama");
    command.arg("pull");

    if let Some(registry) = registry {
        let sanitized_registry = sanitize_registry(&registry)?;
        command.arg(format!("{}{}", sanitized_registry, sanitized_model));
    } else {
        command.arg(&sanitized_model);
    }

    if force {
        command.arg("--force");
    }

    let result = run_command(command).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "model": sanitized_model
    }))
}

/// Start BrainDrive services with proper process management
pub async fn start_braindrive(
    frontend_port: u16,
    backend_port: u16,
    process_state: &ProcessState,
) -> Result<Value, String> {
    let repo_path = resolve_repo_path(None)?;
    if !repo_path.exists() {
        return Err("BrainDrive is not installed. Please install it first.".to_string());
    }

    // Check if services are already running on these ports
    if is_port_in_use(backend_port) {
        return Err(format!(
            "Port {} is already in use. Please stop the existing service or choose a different port.",
            backend_port
        ));
    }
    if is_port_in_use(frontend_port) {
        return Err(format!(
            "Port {} is already in use. Please stop the existing service or choose a different port.",
            frontend_port
        ));
    }

    let backend_path = repo_path.join("backend");
    let frontend_path = repo_path.join("frontend");

    // Verify paths exist
    if !backend_path.exists() {
        return Err(format!(
            "Backend directory not found at {}",
            backend_path.display()
        ));
    }
    if !frontend_path.exists() {
        return Err(format!(
            "Frontend directory not found at {}",
            frontend_path.display()
        ));
    }

    // Start backend using a wrapper script approach for conda environment
    let backend_pid = start_backend_service(&backend_path, backend_port).await?;

    // Wait for backend to start (with timeout)
    if !wait_for_port(backend_port, 30).await {
        // Backend didn't start, try to clean up
        if let Some(pid) = backend_pid {
            kill_process(pid);
        }
        return Err("Backend failed to start within 30 seconds. Check the logs for details.".to_string());
    }

    // Start frontend
    let frontend_pid = start_frontend_service(&frontend_path, frontend_port).await?;

    // Wait for frontend to start (with timeout)
    if !wait_for_port(frontend_port, 30).await {
        // Frontend didn't start, clean up both services
        if let Some(pid) = frontend_pid {
            kill_process(pid);
        }
        if let Some(pid) = backend_pid {
            kill_process(pid);
        }
        kill_process_on_port(backend_port);
        return Err("Frontend failed to start within 30 seconds. Check the logs for details.".to_string());
    }

    // Update process state
    {
        let mut state = process_state.lock().await;
        state.backend = Some(ServiceInfo {
            name: "backend".to_string(),
            pid: backend_pid,
            port: backend_port,
            running: true,
        });
        state.frontend = Some(ServiceInfo {
            name: "frontend".to_string(),
            pid: frontend_pid,
            port: frontend_port,
            running: true,
        });
    }

    Ok(json!({
        "success": true,
        "message": "BrainDrive services started successfully",
        "frontend_port": frontend_port,
        "backend_port": backend_port,
        "frontend_url": format!("http://localhost:{}", frontend_port),
        "backend_url": format!("http://localhost:{}", backend_port),
        "backend_pid": backend_pid,
        "frontend_pid": frontend_pid
    }))
}

/// Start the backend service
async fn start_backend_service(backend_path: &PathBuf, port: u16) -> Result<Option<u32>, String> {
    // Create a shell script to run the backend with conda
    let script_content = format!(
        r#"#!/bin/bash
set -e
cd "{}"
{}
exec uvicorn main:app --host 0.0.0.0 --port {}
"#,
        backend_path.display(),
        process_manager::conda_run_command(CONDA_ENV_NAME, "true").replace(" true", ""),
        port
    );

    // Write the script to a temporary location
    let script_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".braindrive-installer")
        .join("scripts");

    std::fs::create_dir_all(&script_dir)
        .map_err(|e| format!("Failed to create scripts directory: {}", e))?;

    let script_path = script_dir.join("start_backend.sh");
    std::fs::write(&script_path, &script_content)
        .map_err(|e| format!("Failed to write startup script: {}", e))?;

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set script permissions: {}", e))?;
    }

    // Spawn the script
    let pid = spawn_detached(
        "bash",
        &[script_path.to_str().unwrap()],
        backend_path,
        &[],
    )
    .await?;

    Ok(Some(pid))
}

/// Start the frontend service
async fn start_frontend_service(frontend_path: &PathBuf, port: u16) -> Result<Option<u32>, String> {
    // Create a shell script to run the frontend
    let script_content = format!(
        r#"#!/bin/bash
set -e
cd "{}"
exec npm run dev -- --host localhost --port {}
"#,
        frontend_path.display(),
        port
    );

    let script_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".braindrive-installer")
        .join("scripts");

    std::fs::create_dir_all(&script_dir)
        .map_err(|e| format!("Failed to create scripts directory: {}", e))?;

    let script_path = script_dir.join("start_frontend.sh");
    std::fs::write(&script_path, &script_content)
        .map_err(|e| format!("Failed to write startup script: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set script permissions: {}", e))?;
    }

    let pid = spawn_detached(
        "bash",
        &[script_path.to_str().unwrap()],
        frontend_path,
        &[],
    )
    .await?;

    Ok(Some(pid))
}

/// Stop BrainDrive services
pub async fn stop_braindrive(process_state: &ProcessState) -> Result<Value, String> {
    let mut stopped_backend = false;
    let mut stopped_frontend = false;
    let mut backend_port = 8005u16;
    let mut frontend_port = 5173u16;

    // Get current state
    let current_state = {
        let state = process_state.lock().await;
        state.clone()
    };

    // Stop backend
    if let Some(ref backend) = current_state.backend {
        backend_port = backend.port;

        // Try to kill by PID first
        if let Some(pid) = backend.pid {
            if kill_process(pid) {
                stopped_backend = true;
            }
        }

        // Fallback: kill by port
        if !stopped_backend {
            stopped_backend = kill_process_on_port(backend.port);
        }
    } else {
        // No tracked state, try to kill by default port
        stopped_backend = kill_process_on_port(backend_port);
    }

    // Stop frontend
    if let Some(ref frontend) = current_state.frontend {
        frontend_port = frontend.port;

        if let Some(pid) = frontend.pid {
            if kill_process(pid) {
                stopped_frontend = true;
            }
        }

        if !stopped_frontend {
            stopped_frontend = kill_process_on_port(frontend.port);
        }
    } else {
        stopped_frontend = kill_process_on_port(frontend_port);
    }

    // Wait for ports to be freed
    let backend_freed = wait_for_port_free(backend_port, 5).await;
    let frontend_freed = wait_for_port_free(frontend_port, 5).await;

    // Update process state
    {
        let mut state = process_state.lock().await;
        if let Some(ref mut backend) = state.backend {
            backend.running = false;
            backend.pid = None;
        }
        if let Some(ref mut frontend) = state.frontend {
            frontend.running = false;
            frontend.pid = None;
        }
    }

    let success = (stopped_backend || !is_port_in_use(backend_port))
        && (stopped_frontend || !is_port_in_use(frontend_port));

    Ok(json!({
        "success": success,
        "message": if success { "BrainDrive services stopped" } else { "Some services may still be running" },
        "backend_stopped": stopped_backend || backend_freed,
        "frontend_stopped": stopped_frontend || frontend_freed
    }))
}

/// Restart BrainDrive services
pub async fn restart_braindrive(
    frontend_port: u16,
    backend_port: u16,
    process_state: &ProcessState,
) -> Result<Value, String> {
    // Stop existing services
    let stop_result = stop_braindrive(process_state).await?;

    // Brief pause to ensure cleanup
    sleep(Duration::from_millis(500)).await;

    // Start services again
    let start_result = start_braindrive(frontend_port, backend_port, process_state).await?;

    Ok(json!({
        "success": true,
        "message": "BrainDrive services restarted",
        "stop_result": stop_result,
        "start_result": start_result
    }))
}

/// Get the current status of BrainDrive services
pub async fn get_braindrive_status(process_state: &ProcessState) -> Result<Value, String> {
    let state = process_state.lock().await;

    // Check actual port status
    let backend_port = state.backend.as_ref().map(|b| b.port).unwrap_or(8005);
    let frontend_port = state.frontend.as_ref().map(|f| f.port).unwrap_or(5173);

    let backend_running = is_port_in_use(backend_port);
    let frontend_running = is_port_in_use(frontend_port);

    Ok(json!({
        "backend": {
            "port": backend_port,
            "running": backend_running,
            "pid": state.backend.as_ref().and_then(|b| b.pid)
        },
        "frontend": {
            "port": frontend_port,
            "running": frontend_running,
            "pid": state.frontend.as_ref().and_then(|f| f.pid)
        },
        "overall_running": backend_running && frontend_running
    }))
}

/// Run an arbitrary command and capture stdout/stderr
async fn run_command(mut command: Command) -> Result<CommandOutput, String> {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(CommandOutput {
        success: output.status.success(),
        stdout,
        stderr,
        exit_code,
    })
}

#[cfg(not(target_os = "windows"))]
async fn run_shell_script(script: &str) -> Result<CommandOutput, String> {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    run_command(command).await
}

fn sanitize_env_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    let re = Regex::new(r"^[A-Za-z0-9_-]+$").unwrap();
    if trimmed.is_empty() || !re.is_match(trimmed) {
        return Err("Environment name may only contain letters, numbers, underscores, and dashes.".to_string());
    }
    Ok(trimmed.to_string())
}

fn sanitize_model_name(model: &str) -> Result<String, String> {
    let trimmed = model.trim();
    let re = Regex::new(r"^[A-Za-z0-9._:+/-]+$").unwrap();
    if trimmed.is_empty() || !re.is_match(trimmed) {
        return Err("Model names may only contain letters, numbers, dots, underscores, dashes, slashes, and colons.".to_string());
    }
    Ok(trimmed.to_string())
}

fn sanitize_registry(registry: &str) -> Result<String, String> {
    let trimmed = registry.trim();
    let re = Regex::new(r"^[A-Za-z0-9._:/-]+$").unwrap();
    if trimmed.is_empty() || !re.is_match(trimmed) {
        return Err("Registry must be a valid hostname or URL fragment.".to_string());
    }

    let mut normalized = trimmed.to_string();
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    Ok(normalized)
}

fn ensure_command_available(cmd: &str) -> Result<(), String> {
    if command_exists(cmd) {
        Ok(())
    } else {
        Err(format!(
            "'{}' is not available on this system. Please install it before continuing.",
            cmd
        ))
    }
}

fn command_exists(cmd: &str) -> bool {
    use std::process::Command as StdCommand;
    if cfg!(target_os = "windows") {
        StdCommand::new("where")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        StdCommand::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn resolve_repo_path(input: Option<String>) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let base = match input {
        Some(path) => PathBuf::from(path),
        None => home.join(DEFAULT_REPO_DIR),
    };

    let canonical = base
        .canonicalize()
        .unwrap_or_else(|_| base.clone());

    if !canonical.exists() {
        return Err(format!(
            "Repository path '{}' does not exist",
            canonical.display()
        ));
    }

    if !canonical.starts_with(&home) {
        return Err("Repository path must be inside your home directory".to_string());
    }

    Ok(canonical)
}

fn resolve_environment_file(repo: &Path, environment_file: Option<String>) -> Result<PathBuf, String> {
    let relative = environment_file.unwrap_or_else(|| "environment.yml".to_string());
    let candidate = repo.join(relative);
    let canonical = candidate
        .canonicalize()
        .map_err(|_| "Environment file could not be found".to_string())?;

    if !canonical.starts_with(repo) {
        return Err("Environment file must live inside the BrainDrive repository".to_string());
    }

    Ok(canonical)
}

struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
    exit_code: i32,
}
