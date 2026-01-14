use crate::process_manager::{
    self, is_port_in_use, kill_process, kill_process_on_port,
    spawn_detached, wait_for_port, wait_for_port_free, ProcessState, ServiceInfo,
};
use crate::system_info;
use crate::websocket::{send_message, OutgoingMessage};
use crate::WsSender;
use regex::Regex;
use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

const DEFAULT_REPO_DIR: &str = "BrainDrive";
const CONDA_ENV_NAME: &str = "BrainDriveDev";
const OLLAMA_DEFAULT_PORT: u16 = 11434;

/// Known paths where Ollama might be installed
/// GUI apps often have minimal PATH, so we check absolute paths directly
const OLLAMA_KNOWN_PATHS: &[&str] = &[
    "/usr/local/bin/ollama",
    "/opt/homebrew/bin/ollama",
    "/usr/bin/ollama",
    "/snap/bin/ollama",
];

/// Find Ollama binary in known paths
/// Returns the full path if found, None otherwise
fn find_ollama_binary() -> Option<PathBuf> {
    for path in OLLAMA_KNOWN_PATHS {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    // Also check if it's in PATH (for cases where user has custom setup)
    if let Ok(output) = std::process::Command::new("which")
        .arg("ollama")
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }

    None
}

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

/// Ensure Ollama is installed and running
/// If installed: starts service if needed
/// If not installed: returns instructions for manual installation
pub async fn install_ollama() -> Result<Value, String> {
    // Check if Ollama binary exists using absolute paths
    if let Some(ollama_path) = find_ollama_binary() {
        let version = get_ollama_version();
        let running = is_port_in_use(OLLAMA_DEFAULT_PORT);

        if running {
            return Ok(json!({
                "success": true,
                "installed": true,
                "ollama_path": ollama_path.to_string_lossy(),
                "version": version,
                "service_running": true,
                "message": "Ollama is installed and running"
            }));
        }

        // Installed but not running - start the service
        let start_result = start_ollama_service().await;
        let service_ok = start_result.is_ok();
        let start_error = start_result.err();
        return Ok(json!({
            "success": service_ok,
            "installed": true,
            "ollama_path": ollama_path.to_string_lossy(),
            "version": version,
            "service_running": service_ok,
            "service_start_error": start_error,
            "message": if service_ok {
                "Ollama service started successfully"
            } else {
                "Ollama is installed but failed to start service"
            }
        }));
    }

    // Ollama not found - return instructions for manual installation
    let download_url = "https://ollama.com/download";
    let os = std::env::consts::OS;

    let install_instructions = match os {
        "macos" => format!(
            "Please install Ollama manually:\n\
            1. Visit {} and download the macOS installer\n\
            2. Open the downloaded .dmg file\n\
            3. Drag Ollama to your Applications folder\n\
            4. Open Ollama from Applications\n\
            5. Come back here and I'll detect it automatically",
            download_url
        ),
        "linux" => format!(
            "Please install Ollama manually:\n\
            1. Open a terminal\n\
            2. Run: curl -fsSL https://ollama.com/install.sh | sh\n\
            3. Start Ollama: ollama serve\n\
            4. Come back here and I'll detect it automatically\n\n\
            Or visit {} for other options",
            download_url
        ),
        "windows" => format!(
            "Please install Ollama manually:\n\
            1. Visit {} and download the Windows installer\n\
            2. Run the installer\n\
            3. Ollama will start automatically\n\
            4. Come back here and I'll detect it automatically",
            download_url
        ),
        _ => format!("Please visit {} to download and install Ollama for your system.", download_url),
    };

    Ok(json!({
        "success": false,
        "installed": false,
        "needs_manual_install": true,
        "download_url": download_url,
        "instructions": install_instructions,
        "message": "Ollama is not installed. Please install it manually and I'll detect it automatically."
    }))
}

/// Get Ollama version string using absolute path
fn get_ollama_version() -> Option<String> {
    let ollama_path = find_ollama_binary()?;

    let output = std::process::Command::new(&ollama_path)
        .arg("--version")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .trim()
        .strip_prefix("ollama version ")
        .unwrap_or(stdout.trim())
        .to_string();

    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// Start the Ollama service (public API)
pub async fn start_ollama() -> Result<Value, String> {
    // Use absolute path detection
    let ollama_path = match find_ollama_binary() {
        Some(path) => path,
        None => {
            return Ok(json!({
                "success": false,
                "installed": false,
                "message": "Ollama is not installed. Please install it first from https://ollama.com/download"
            }));
        }
    };

    if is_port_in_use(OLLAMA_DEFAULT_PORT) {
        let version = get_ollama_version();
        return Ok(json!({
            "success": true,
            "already_running": true,
            "ollama_path": ollama_path.to_string_lossy(),
            "version": version,
            "message": "Ollama service is already running"
        }));
    }

    let result = start_ollama_service().await;
    let version = get_ollama_version();

    match result {
        Ok(()) => Ok(json!({
            "success": true,
            "already_running": false,
            "version": version,
            "message": "Ollama service started successfully"
        })),
        Err(e) => Ok(json!({
            "success": false,
            "error": e,
            "message": "Failed to start Ollama service"
        })),
    }
}

/// Start the Ollama service and wait for it to be ready (internal helper)
async fn start_ollama_service() -> Result<(), String> {
    // Check if already running
    if is_port_in_use(OLLAMA_DEFAULT_PORT) {
        return Ok(());
    }

    // Find the ollama binary - must exist to start service
    let ollama_path = find_ollama_binary()
        .ok_or("Ollama binary not found. Please install Ollama first.")?;
    let ollama_path_str = ollama_path.to_string_lossy().to_string();

    let home_dir = dirs::home_dir().ok_or("Could not determine home directory")?;
    let empty_env: &[(&str, &str)] = &[];

    #[cfg(target_os = "macos")]
    {
        // On macOS, try launchctl first (if installed as service), then fall back to ollama serve
        let launchctl_result = std::process::Command::new("launchctl")
            .args(["start", "com.ollama.ollama"])
            .output();

        if let Ok(output) = launchctl_result {
            if output.status.success() {
                // Wait for service to be ready
                if wait_for_port(OLLAMA_DEFAULT_PORT, 30).await {
                    return Ok(());
                }
            }
        }

        // Fall back to spawning ollama serve directly using absolute path
        spawn_detached(&ollama_path_str, &["serve"], &home_dir, empty_env).await
            .map_err(|e| format!("Failed to start Ollama service: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        // On Linux, try systemctl first, then fall back to ollama serve
        let systemctl_result = std::process::Command::new("systemctl")
            .args(["--user", "start", "ollama"])
            .output();

        if let Ok(output) = systemctl_result {
            if output.status.success() {
                if wait_for_port(OLLAMA_DEFAULT_PORT, 30).await {
                    return Ok(());
                }
            }
        }

        // Try system-level systemctl
        let systemctl_system = std::process::Command::new("systemctl")
            .args(["start", "ollama"])
            .output();

        if let Ok(output) = systemctl_system {
            if output.status.success() {
                if wait_for_port(OLLAMA_DEFAULT_PORT, 30).await {
                    return Ok(());
                }
            }
        }

        // Fall back to spawning ollama serve directly using absolute path
        spawn_detached(&ollama_path_str, &["serve"], &home_dir, empty_env).await
            .map_err(|e| format!("Failed to start Ollama service: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, just spawn ollama serve using absolute path
        spawn_detached(&ollama_path_str, &["serve"], &home_dir, empty_env).await
            .map_err(|e| format!("Failed to start Ollama service: {}", e))?;
    }

    // Wait for service to be ready
    if wait_for_port(OLLAMA_DEFAULT_PORT, 30).await {
        Ok(())
    } else {
        Err("Ollama service started but not responding on port 11434 after 30 seconds".to_string())
    }
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

/// Pull a vetted Ollama model with progress streaming
pub async fn pull_ollama_model_with_progress(
    model: &str,
    registry: Option<String>,
    force: bool,
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
) -> Result<Value, String> {
    // Find Ollama binary using absolute path
    let ollama_path = find_ollama_binary()
        .ok_or("Ollama is not installed. Please install it first from https://ollama.com/download")?;

    let sanitized_model = sanitize_model_name(model)?;

    let model_arg = if let Some(ref reg) = registry {
        let sanitized_registry = sanitize_registry(reg)?;
        format!("{}{}", sanitized_registry, sanitized_model)
    } else {
        sanitized_model.clone()
    };

    let mut command = Command::new(&ollama_path);
    command
        .arg("pull")
        .arg(&model_arg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if force {
        command.arg("--force");
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn ollama pull: {}", e))?;

    let stderr = child.stderr.take()
        .ok_or("Failed to capture stderr")?;

    let mut reader = BufReader::new(stderr).lines();
    let mut last_progress_message = String::new();

    // Read stderr line by line (ollama outputs progress to stderr)
    while let Ok(Some(line)) = reader.next_line().await {
        if let Some(progress) = parse_ollama_progress(&line) {
            // Only send if message changed (avoid spamming identical updates)
            if line != last_progress_message {
                last_progress_message = line.clone();

                let progress_msg = OutgoingMessage::Progress {
                    id: request_id.clone(),
                    operation: "pull_ollama_model".to_string(),
                    percent: progress.percent,
                    message: progress.message,
                    bytes_downloaded: progress.bytes_downloaded,
                    bytes_total: progress.bytes_total,
                };

                // Send progress, ignore errors (best effort)
                let _ = send_message(&sender, progress_msg).await;
            }
        }
    }

    // Wait for the process to complete
    let status = child.wait().await
        .map_err(|e| format!("Failed to wait for ollama: {}", e))?;

    let success = status.success();

    // Send final progress
    let final_msg = OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "pull_ollama_model".to_string(),
        percent: if success { Some(100) } else { None },
        message: if success {
            format!("Successfully pulled {}", sanitized_model)
        } else {
            "Download failed".to_string()
        },
        bytes_downloaded: None,
        bytes_total: None,
    };
    let _ = send_message(&sender, final_msg).await;

    Ok(json!({
        "success": success,
        "exit_code": status.code().unwrap_or(-1),
        "model": sanitized_model
    }))
}

/// Parsed progress information from Ollama output
struct OllamaProgress {
    percent: Option<u8>,
    message: String,
    bytes_downloaded: Option<u64>,
    bytes_total: Option<u64>,
}

/// Parse Ollama's progress output
/// Example formats:
/// "pulling manifest"
/// "pulling 8934d96d3f08... 45% ▕████░░░░░░░░░░░░▏ 1.2 GB/2.7 GB"
/// "verifying sha256 digest"
/// "writing manifest"
/// "success"
fn parse_ollama_progress(line: &str) -> Option<OllamaProgress> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Try to parse percentage from lines like "pulling abc... 45%"
    let percent_re = Regex::new(r"(\d+)%").ok()?;
    let percent = percent_re.captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u8>().ok());

    // Try to parse bytes like "1.2 GB/2.7 GB" or "500 MB/1.1 GB"
    let bytes_re = Regex::new(r"([\d.]+)\s*(KB|MB|GB)/([\d.]+)\s*(KB|MB|GB)").ok()?;
    let (bytes_downloaded, bytes_total) = if let Some(caps) = bytes_re.captures(line) {
        let downloaded = parse_size_to_bytes(
            caps.get(1).map(|m| m.as_str()).unwrap_or("0"),
            caps.get(2).map(|m| m.as_str()).unwrap_or("B"),
        );
        let total = parse_size_to_bytes(
            caps.get(3).map(|m| m.as_str()).unwrap_or("0"),
            caps.get(4).map(|m| m.as_str()).unwrap_or("B"),
        );
        (downloaded, total)
    } else {
        (None, None)
    };

    // Create a cleaner message
    let message = if line.starts_with("pulling") {
        if percent.is_some() {
            format!("Downloading model... {}%", percent.unwrap())
        } else {
            "Pulling manifest...".to_string()
        }
    } else if line.starts_with("verifying") {
        "Verifying download...".to_string()
    } else if line.starts_with("writing") {
        "Writing manifest...".to_string()
    } else if line == "success" {
        "Download complete!".to_string()
    } else {
        line.to_string()
    };

    Some(OllamaProgress {
        percent,
        message,
        bytes_downloaded,
        bytes_total,
    })
}

/// Convert size string to bytes
fn parse_size_to_bytes(value: &str, unit: &str) -> Option<u64> {
    let num: f64 = value.parse().ok()?;
    let multiplier = match unit.to_uppercase().as_str() {
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    Some((num * multiplier) as u64)
}

/// Clone the BrainDrive repository
pub async fn clone_repo(repo_url: Option<String>, target_path: Option<String>) -> Result<Value, String> {
    ensure_command_available("git")?;

    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let target = match target_path {
        Some(p) => {
            let path = PathBuf::from(&p);
            // Ensure target is inside home directory
            if !path.starts_with(&home) && !p.starts_with("~/") {
                return Err("Target path must be inside your home directory".to_string());
            }
            if p.starts_with("~/") {
                home.join(&p[2..])
            } else {
                path
            }
        }
        None => home.join(DEFAULT_REPO_DIR),
    };

    // Check if already exists
    if target.exists() {
        if target.join(".git").exists() {
            return Ok(json!({
                "success": true,
                "message": "BrainDrive repository already exists",
                "path": target.to_string_lossy(),
                "already_exists": true
            }));
        } else {
            return Err(format!(
                "Directory {} exists but is not a git repository",
                target.display()
            ));
        }
    }

    // Default to BrainDrive-Core repo
    let url = repo_url.unwrap_or_else(|| "https://github.com/BrainDriveAI/BrainDrive.git".to_string());

    // Validate URL format (basic check)
    if !url.starts_with("https://") && !url.starts_with("git@") {
        return Err("Repository URL must start with https:// or git@".to_string());
    }

    let mut command = Command::new("git");
    command
        .arg("clone")
        .arg("--depth")
        .arg("1")  // Shallow clone for faster download
        .arg(&url)
        .arg(&target);

    let result = run_command(command).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "path": target.to_string_lossy(),
        "url": url
    }))
}

/// Install backend Python dependencies using pip in conda environment
pub async fn install_backend_deps(
    env_name: Option<String>,
    repo_path: Option<String>,
) -> Result<Value, String> {
    ensure_command_available("conda")?;

    let env = sanitize_env_name(&env_name.unwrap_or_else(|| CONDA_ENV_NAME.to_string()))?;
    let repo = resolve_repo_path_or_default(repo_path)?;
    let backend_path = repo.join("backend");
    let requirements_file = backend_path.join("requirements.txt");

    if !backend_path.exists() {
        return Err(format!(
            "Backend directory not found at {}",
            backend_path.display()
        ));
    }

    if !requirements_file.exists() {
        return Err(format!(
            "requirements.txt not found at {}",
            requirements_file.display()
        ));
    }

    // Build the pip install command to run in conda environment
    let pip_cmd = format!(
        "pip install -r \"{}\"",
        requirements_file.display()
    );
    let full_cmd = process_manager::conda_run_command(&env, &pip_cmd);

    let result = run_shell_script(&full_cmd).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "env_name": env,
        "requirements_file": requirements_file.to_string_lossy()
    }))
}

/// Install frontend npm dependencies
pub async fn install_frontend_deps(repo_path: Option<String>) -> Result<Value, String> {
    ensure_command_available("npm")?;

    let repo = resolve_repo_path_or_default(repo_path)?;
    let frontend_path = repo.join("frontend");

    if !frontend_path.exists() {
        return Err(format!(
            "Frontend directory not found at {}",
            frontend_path.display()
        ));
    }

    let package_json = frontend_path.join("package.json");
    if !package_json.exists() {
        return Err(format!(
            "package.json not found at {}",
            package_json.display()
        ));
    }

    let mut command = Command::new("npm");
    command
        .arg("install")
        .current_dir(&frontend_path);

    let result = run_command(command).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "frontend_path": frontend_path.to_string_lossy()
    }))
}

/// Setup the environment file by copying .env-dev to .env
pub async fn setup_env_file(repo_path: Option<String>) -> Result<Value, String> {
    let repo = resolve_repo_path_or_default(repo_path)?;
    let backend_path = repo.join("backend");
    let env_dev = backend_path.join(".env-dev");
    let env_file = backend_path.join(".env");

    if !env_dev.exists() {
        return Err(format!(
            ".env-dev not found at {}. The repository may not be properly cloned.",
            env_dev.display()
        ));
    }

    // Check if .env already exists
    if env_file.exists() {
        return Ok(json!({
            "success": true,
            "message": ".env file already exists",
            "path": env_file.to_string_lossy(),
            "already_exists": true
        }));
    }

    // Copy .env-dev to .env
    std::fs::copy(&env_dev, &env_file)
        .map_err(|e| format!("Failed to copy .env-dev to .env: {}", e))?;

    Ok(json!({
        "success": true,
        "message": "Environment file created",
        "source": env_dev.to_string_lossy(),
        "destination": env_file.to_string_lossy()
    }))
}

/// Create a new conda environment for BrainDrive
pub async fn create_conda_env(env_name: Option<String>) -> Result<Value, String> {
    ensure_command_available("conda")?;

    let env = sanitize_env_name(&env_name.unwrap_or_else(|| CONDA_ENV_NAME.to_string()))?;

    // Check if environment already exists
    let check_cmd = Command::new("conda")
        .args(["env", "list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to list conda environments: {}", e))?;

    let env_list = String::from_utf8_lossy(&check_cmd.stdout);
    if env_list.lines().any(|line| line.split_whitespace().next() == Some(&env)) {
        return Ok(json!({
            "success": true,
            "message": format!("Conda environment '{}' already exists", env),
            "env_name": env,
            "already_exists": true
        }));
    }

    // Create the environment with Python 3.11, nodejs, and git
    let mut command = Command::new("conda");
    command
        .args([
            "create",
            "-n", &env,
            "-c", "conda-forge",
            "python=3.11",
            "nodejs",
            "git",
            "-y"
        ]);

    let result = run_command(command).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "env_name": env
    }))
}

// Port fallback options
const BACKEND_PORTS: [u16; 3] = [8005, 8006, 8007];
const FRONTEND_PORTS: [u16; 3] = [5173, 5174, 5175];

/// Find an available port from a list of options
fn find_available_port(preferred: u16, fallbacks: &[u16]) -> Option<u16> {
    // Try preferred first
    if !is_port_in_use(preferred) {
        return Some(preferred);
    }
    // Try fallbacks
    for &port in fallbacks {
        if !is_port_in_use(port) {
            return Some(port);
        }
    }
    None
}

/// Start BrainDrive services with proper process management
/// This function is idempotent - if services are already running, it returns success
pub async fn start_braindrive(
    frontend_port: u16,
    backend_port: u16,
    process_state: &ProcessState,
) -> Result<Value, String> {
    let repo_path = resolve_repo_path(None)?;
    if !repo_path.exists() {
        return Err("BrainDrive is not installed. Please install it first.".to_string());
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

    // Check current state - maybe services are already running
    let current_state = {
        let state = process_state.lock().await;
        state.clone()
    };

    let mut backend_already_running = false;
    let mut frontend_already_running = false;
    let mut actual_backend_port = backend_port;
    let mut actual_frontend_port = frontend_port;
    let mut backend_pid: Option<u32> = None;
    let mut frontend_pid: Option<u32> = None;

    // Check if backend is already running (from our tracking or port detection)
    if let Some(ref backend) = current_state.backend {
        if backend.running && is_port_in_use(backend.port) {
            backend_already_running = true;
            actual_backend_port = backend.port;
            backend_pid = backend.pid;
        }
    }

    // Check if frontend is already running
    if let Some(ref frontend) = current_state.frontend {
        if frontend.running && is_port_in_use(frontend.port) {
            frontend_already_running = true;
            actual_frontend_port = frontend.port;
            frontend_pid = frontend.pid;
        }
    }

    // If both already running, return success immediately (idempotent)
    if backend_already_running && frontend_already_running {
        return Ok(json!({
            "success": true,
            "message": "BrainDrive is already running",
            "already_running": true,
            "frontend_port": actual_frontend_port,
            "backend_port": actual_backend_port,
            "frontend_url": format!("http://localhost:{}", actual_frontend_port),
            "backend_url": format!("http://localhost:{}", actual_backend_port)
        }));
    }

    // Start backend if not running
    if !backend_already_running {
        // Find available port (try preferred, then fallbacks)
        actual_backend_port = find_available_port(backend_port, &BACKEND_PORTS)
            .ok_or_else(|| format!(
                "No available backend ports. Tried: {}, {:?}",
                backend_port, BACKEND_PORTS
            ))?;

        backend_pid = start_backend_service(&backend_path, actual_backend_port).await?;

        // Wait for backend to start (with timeout)
        if !wait_for_port(actual_backend_port, 45).await {
            if let Some(pid) = backend_pid {
                kill_process(pid);
            }
            return Err("Backend failed to start within 45 seconds. Check ~/.braindrive-installer/logs/ for details.".to_string());
        }
    }

    // Start frontend if not running
    if !frontend_already_running {
        // Find available port (try preferred, then fallbacks)
        actual_frontend_port = find_available_port(frontend_port, &FRONTEND_PORTS)
            .ok_or_else(|| format!(
                "No available frontend ports. Tried: {}, {:?}",
                frontend_port, FRONTEND_PORTS
            ))?;

        frontend_pid = start_frontend_service(&frontend_path, actual_frontend_port).await?;

        // Wait for frontend to start (with timeout)
        // Note: We don't kill backend if frontend fails - backend is still useful
        if !wait_for_port(actual_frontend_port, 45).await {
            if let Some(pid) = frontend_pid {
                kill_process(pid);
            }
            // Backend is still running, report partial success
            return Ok(json!({
                "success": false,
                "partial": true,
                "message": "Backend started but frontend failed to start within 45 seconds",
                "backend_port": actual_backend_port,
                "backend_url": format!("http://localhost:{}", actual_backend_port),
                "backend_running": true,
                "frontend_running": false,
                "error": "Frontend startup timed out. Check ~/.braindrive-installer/logs/ for details."
            }));
        }
    }

    // Update process state
    {
        let mut state = process_state.lock().await;
        state.backend = Some(ServiceInfo {
            name: "backend".to_string(),
            pid: backend_pid,
            port: actual_backend_port,
            running: true,
        });
        state.frontend = Some(ServiceInfo {
            name: "frontend".to_string(),
            pid: frontend_pid,
            port: actual_frontend_port,
            running: true,
        });
    }

    let mut message = "BrainDrive services started successfully".to_string();
    if backend_already_running || frontend_already_running {
        let mut parts = vec![];
        if backend_already_running {
            parts.push("backend was already running");
        }
        if frontend_already_running {
            parts.push("frontend was already running");
        }
        message = format!("BrainDrive started ({})", parts.join(", "));
    }

    Ok(json!({
        "success": true,
        "message": message,
        "frontend_port": actual_frontend_port,
        "backend_port": actual_backend_port,
        "frontend_url": format!("http://localhost:{}", actual_frontend_port),
        "backend_url": format!("http://localhost:{}", actual_backend_port),
        "backend_pid": backend_pid,
        "frontend_pid": frontend_pid,
        "backend_already_running": backend_already_running,
        "frontend_already_running": frontend_already_running
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

/// Resolve repo path, returning default if not specified.
/// Unlike resolve_repo_path, this expects the path to exist and validates it.
fn resolve_repo_path_or_default(input: Option<String>) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let base = match input {
        Some(path) => {
            let p = PathBuf::from(&path);
            if path.starts_with("~/") {
                home.join(&path[2..])
            } else {
                p
            }
        }
        None => home.join(DEFAULT_REPO_DIR),
    };

    // Try to canonicalize if it exists
    let resolved = if base.exists() {
        base.canonicalize().unwrap_or(base)
    } else {
        return Err(format!(
            "Repository path '{}' does not exist. Please clone the repository first.",
            base.display()
        ));
    };

    // Security: ensure path is inside home directory
    if !resolved.starts_with(&home) {
        return Err("Repository path must be inside your home directory".to_string());
    }

    Ok(resolved)
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
