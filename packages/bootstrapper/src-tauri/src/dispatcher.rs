use crate::system_info;
use regex::Regex;
use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

const DEFAULT_REPO_DIR: &str = "BrainDrive";

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

/// Start BrainDrive services (placeholder implementation)
pub async fn start_braindrive(frontend_port: u16, backend_port: u16) -> Result<Value, String> {
    let repo_path = resolve_repo_path(None)?;
    if !repo_path.exists() {
        return Err("BrainDrive is not installed. Please install it first.".to_string());
    }

    let backend_path = repo_path.join("backend");
    let backend_cmd = format!(
        "cd \"{}\" && conda run -n BrainDriveDev uvicorn main:app --host 0.0.0.0 --port {} &",
        backend_path.display(),
        backend_port
    );

    run_shell_command(&backend_cmd).await?;
    sleep(Duration::from_secs(2)).await;

    let frontend_path = repo_path.join("frontend");
    let frontend_cmd = format!(
        "cd \"{}\" && npm run dev -- --host localhost --port {} &",
        frontend_path.display(),
        frontend_port
    );
    run_shell_command(&frontend_cmd).await?;

    Ok(json!({
        "success": true,
        "message": "BrainDrive services started",
        "frontend_port": frontend_port,
        "backend_port": backend_port,
        "frontend_url": format!("http://localhost:{}", frontend_port),
        "backend_url": format!("http://localhost:{}", backend_port)
    }))
}

/// Stop BrainDrive services (placeholder)
pub async fn stop_braindrive() -> Result<Value, String> {
    eprintln!("Stopping BrainDrive (placeholder)");

    Ok(json!({
        "success": true,
        "message": "BrainDrive stop initiated"
    }))
}

/// Restart BrainDrive services (placeholder)
pub async fn restart_braindrive() -> Result<Value, String> {
    stop_braindrive().await?;
    start_braindrive(5173, 8005).await
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

async fn run_shell_command(script: &str) -> Result<(), String> {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn command '{}': {}", script, e))?;
    Ok(())
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
