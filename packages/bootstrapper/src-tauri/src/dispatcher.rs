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

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const DEFAULT_REPO_DIR: &str = "BrainDrive";
const CONDA_ENV_NAME: &str = "BrainDriveDev";
const OLLAMA_DEFAULT_PORT: u16 = 11434;
/// Isolated Miniconda is installed inside the BrainDrive directory
/// This prevents conflicts with any existing user conda installation
const ISOLATED_MINICONDA_DIR: &str = "miniconda3";
const DOWNLOAD_MAX_RETRIES: u8 = 3;
const DOWNLOAD_RETRY_DELAY_SECS: u64 = 2;
/// Timeout for establishing HTTP connection (seconds)
const DOWNLOAD_CONNECT_TIMEOUT_SECS: u64 = 30;

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

/// Get the path to the isolated Miniconda installation directory
/// This is ~/BrainDrive/miniconda3 - completely separate from any system conda
fn get_isolated_miniconda_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(DEFAULT_REPO_DIR).join(ISOLATED_MINICONDA_DIR))
}

/// Get the path to the isolated conda binary
/// Returns the full path to conda binary in ~/BrainDrive/miniconda3/bin/conda
/// Only returns the path if the installation is valid (has conda binary and conda.sh on Unix)
fn get_isolated_conda_binary() -> Option<PathBuf> {
    let miniconda_dir = get_isolated_miniconda_dir()?;

    #[cfg(target_os = "windows")]
    {
        let conda_binary = miniconda_dir.join("Scripts").join("conda.exe");
        if conda_binary.exists() {
            return Some(conda_binary);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let conda_binary = miniconda_dir.join("bin").join("conda");
        let conda_sh = miniconda_dir.join("etc/profile.d/conda.sh");
        // Validate both binary and conda.sh exist for a complete installation
        if conda_binary.exists() && conda_sh.exists() {
            return Some(conda_binary);
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
/// Uses the isolated conda installation at ~/BrainDrive/miniconda3
pub async fn install_conda_env(
    env_name: &str,
    repo_path: Option<String>,
    environment_file: Option<String>,
) -> Result<Value, String> {
    // Get the conda binary path (prefers isolated installation)
    let conda_path = find_conda_binary()
        .ok_or("Conda is not installed. Please install it first using the install_conda tool.")?;

    let sanitized_env = sanitize_env_name(env_name)?;
    let repo = resolve_repo_path(repo_path)?;
    let env_file = resolve_environment_file(&repo, environment_file)?;

    let mut command = Command::new(&conda_path);
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
        "environment_file": env_file.to_string_lossy(),
        "conda_path": conda_path.to_string_lossy()
    }))
}

/// Install Miniconda automatically (no sudo required)
/// Downloads the installer for the user's platform and runs it in batch mode
/// Installs to ~/BrainDrive/miniconda3 (isolated from any system conda)
pub async fn install_conda(
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
) -> Result<Value, String> {
    // Check if isolated conda is already installed at ~/BrainDrive/miniconda3
    if let Some(conda_path) = get_isolated_conda_binary() {
        return Ok(json!({
            "success": true,
            "already_installed": true,
            "conda_path": conda_path.to_string_lossy(),
            "isolated": true,
            "message": "Isolated Miniconda is already installed in BrainDrive directory"
        }));
    }

    let home_dir = dirs::home_dir().ok_or("Could not determine home directory")?;

    // Install to ~/BrainDrive/miniconda3 (isolated installation)
    let braindrive_dir = home_dir.join(DEFAULT_REPO_DIR);
    let install_path = braindrive_dir.join(ISOLATED_MINICONDA_DIR);

    // Ensure the BrainDrive directory exists
    if !braindrive_dir.exists() {
        std::fs::create_dir_all(&braindrive_dir)
            .map_err(|e| format!("Failed to create BrainDrive directory: {}", e))?;
    }

    // Check if miniconda directory already exists at the isolated location
    if install_path.exists() {
        #[cfg(target_os = "windows")]
        let conda_in_install = install_path.join("Scripts/conda.exe");
        #[cfg(not(target_os = "windows"))]
        let conda_in_install = install_path.join("bin/conda");

        if conda_in_install.exists() {
            return Ok(json!({
                "success": true,
                "already_installed": true,
                "conda_path": conda_in_install.to_string_lossy(),
                "isolated": true,
                "message": "Isolated Miniconda is already installed"
            }));
        }
    }

    // Determine the correct installer URL based on OS and architecture
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let installer_url = match (os, arch) {
        ("macos", "aarch64") => "https://repo.anaconda.com/miniconda/Miniconda3-latest-MacOSX-arm64.sh",
        ("macos", "x86_64") => "https://repo.anaconda.com/miniconda/Miniconda3-latest-MacOSX-x86_64.sh",
        ("linux", "x86_64") => "https://repo.anaconda.com/miniconda/Miniconda3-latest-Linux-x86_64.sh",
        ("linux", "aarch64") => "https://repo.anaconda.com/miniconda/Miniconda3-latest-Linux-aarch64.sh",
        ("windows", "x86_64") => "https://repo.anaconda.com/miniconda/Miniconda3-latest-Windows-x86_64.exe",
        _ => return Err(format!("Unsupported platform: {} {}", os, arch)),
    };

    // Create temp directory for installer
    let temp_dir = home_dir.join(".braindrive-installer").join("downloads");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create download directory: {}", e))?;

    let installer_filename = if os == "windows" {
        "Miniconda3-installer.exe"
    } else {
        "Miniconda3-installer.sh"
    };
    let installer_path = temp_dir.join(installer_filename);

    // Send initial progress
    let _ = send_message(&sender, OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "install_conda".to_string(),
        percent: Some(0),
        message: "Downloading Miniconda installer...".to_string(),
        bytes_downloaded: None,
        bytes_total: None,
    }).await;

    // Download the installer with progress
    download_file_with_progress(
        installer_url,
        &installer_path,
        request_id.clone(),
        sender.clone(),
        "install_conda",
    ).await?;

    // Send progress for installation phase
    let _ = send_message(&sender, OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "install_conda".to_string(),
        percent: Some(50),
        message: "Installing Miniconda (this may take a minute)...".to_string(),
        bytes_downloaded: None,
        bytes_total: None,
    }).await;

    // Run the installer
    let install_result = if os == "windows" {
        run_windows_miniconda_installer(&installer_path, &install_path).await
    } else {
        run_unix_miniconda_installer(&installer_path, &install_path).await
    };

    // Clean up installer file
    let _ = std::fs::remove_file(&installer_path);

    match install_result {
        Ok(()) => {
            // Verify installation
            let conda_binary = if os == "windows" {
                install_path.join("Scripts/conda.exe")
            } else {
                install_path.join("bin/conda")
            };

            if !conda_binary.exists() {
                return Err("Miniconda installation completed but conda binary not found".to_string());
            }

            // Configure conda to use conda-forge and avoid TOS issues
            // This runs silently and doesn't require user interaction
            let _ = send_message(&sender, OutgoingMessage::Progress {
                id: request_id.clone(),
                operation: "install_conda".to_string(),
                percent: Some(90),
                message: "Configuring conda...".to_string(),
                bytes_downloaded: None,
                bytes_total: None,
            }).await;

            // Set conda-forge as default channel and remove defaults that require TOS
            let config_commands = [
                ["config", "--set", "auto_activate_base", "false"],
                ["config", "--add", "channels", "conda-forge"],
                ["config", "--set", "channel_priority", "strict"],
                ["config", "--remove", "channels", "defaults"],
            ];

            for args in &config_commands {
                let mut cmd = Command::new(&conda_binary);
                cmd.args(*args);
                #[cfg(target_os = "windows")]
                cmd.creation_flags(CREATE_NO_WINDOW);
                // Ignore errors - some configs may fail if already set
                let _ = cmd.output().await;
            }

            // Send completion progress
            let _ = send_message(&sender, OutgoingMessage::Progress {
                id: request_id.clone(),
                operation: "install_conda".to_string(),
                percent: Some(100),
                message: "Miniconda installed and configured!".to_string(),
                bytes_downloaded: None,
                bytes_total: None,
            }).await;

            Ok(json!({
                "success": true,
                "already_installed": false,
                "conda_path": conda_binary.to_string_lossy(),
                "install_path": install_path.to_string_lossy(),
                "isolated": true,
                "message": "Miniconda installed successfully to BrainDrive directory"
            }))
        }
        Err(e) => Err(format!("Failed to install Miniconda: {}", e)),
    }
}

/// Find conda binary in known paths
/// PRIORITY ORDER:
/// 1. Isolated BrainDrive installation (~/BrainDrive/miniconda3) - preferred
/// 2. User home directory installations (~/miniconda3, ~/anaconda3)
/// 3. System-wide paths
/// 4. PATH lookup via which/where
fn find_conda_binary() -> Option<PathBuf> {
    // FIRST: Check the isolated BrainDrive installation (highest priority)
    if let Some(isolated_conda) = get_isolated_conda_binary() {
        return Some(isolated_conda);
    }

    // Check other home directory paths (for fallback/detection)
    if let Some(home) = dirs::home_dir() {
        let home_paths = [
            home.join("miniconda3/bin/conda"),
            home.join("anaconda3/bin/conda"),
            home.join(".conda/bin/conda"),
            // Windows paths
            home.join("miniconda3/Scripts/conda.exe"),
            home.join("anaconda3/Scripts/conda.exe"),
        ];
        for path in &home_paths {
            if path.exists() {
                return Some(path.clone());
            }
        }
    }

    // Check system-wide paths
    let system_paths = [
        "/opt/miniconda3/bin/conda",
        "/opt/anaconda3/bin/conda",
        "/opt/homebrew/bin/conda",
        "/usr/local/bin/conda",
    ];
    for path in &system_paths {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    // Fall back to which/where
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("where");
        cmd.arg("conda").creation_flags(CREATE_NO_WINDOW);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                if let Some(path_str) = String::from_utf8_lossy(&output.stdout).lines().next() {
                    if !path_str.is_empty() {
                        return Some(PathBuf::from(path_str));
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = std::process::Command::new("which").arg("conda").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(PathBuf::from(path_str));
                }
            }
        }
    }

    None
}

/// Download a file with progress updates
async fn download_file_with_progress(
    url: &str,
    dest: &PathBuf,
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
    operation: &str,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("BrainDrive-Installer/1.0")
        .connect_timeout(Duration::from_secs(DOWNLOAD_CONNECT_TIMEOUT_SECS))
        // Note: No overall timeout set - large downloads (100MB+) need unlimited time
        // The connect_timeout handles initial connection issues
        // Streaming errors are handled per-chunk in download_file_with_progress_once
        .build()
        .map_err(|e| {
            let err_msg = format!("Failed to build HTTP client: {}", e);
            tracing::error!("{}", err_msg);
            err_msg
        })?;

    let mut last_error: Option<String> = None;

    for attempt in 1..=DOWNLOAD_MAX_RETRIES {
        if attempt > 1 {
            let _ = send_message(&sender, OutgoingMessage::Progress {
                id: request_id.clone(),
                operation: operation.to_string(),
                percent: Some(0),
                message: format!(
                    "Retrying download (attempt {} of {})...",
                    attempt, DOWNLOAD_MAX_RETRIES
                ),
                bytes_downloaded: None,
                bytes_total: None,
            }).await;
        }

        match download_file_with_progress_once(
            &client,
            url,
            dest,
            request_id.clone(),
            sender.clone(),
            operation,
        ).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    "Download attempt {} failed for {}: {}",
                    attempt, url, e
                );
                last_error = Some(e);
                let _ = std::fs::remove_file(dest);
                if attempt < DOWNLOAD_MAX_RETRIES {
                    sleep(Duration::from_secs(DOWNLOAD_RETRY_DELAY_SECS * attempt as u64)).await;
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = send_message(&sender, OutgoingMessage::Progress {
            id: request_id.clone(),
            operation: operation.to_string(),
            percent: Some(0),
            message: "Retrying download with system curl...".to_string(),
            bytes_downloaded: None,
            bytes_total: None,
        }).await;

        if let Err(curl_err) = download_file_with_curl(url, dest).await {
            let combined_error = match last_error {
                Some(err) => format!("{} | curl fallback failed: {}", err, curl_err),
                None => format!("curl fallback failed: {}", curl_err),
            };
            let final_error = format!(
                "Download failed after {} attempts: {}",
                DOWNLOAD_MAX_RETRIES,
                combined_error
            );
            tracing::error!("{}", final_error);
            return Err(final_error);
        }

        tracing::info!("Download succeeded via curl fallback for {}", url);
        return Ok(());
    }

    // Windows-only fallback (no curl available by default)
    #[cfg(target_os = "windows")]
    {
        let last_error = last_error.unwrap_or_else(|| "Unknown download error".to_string());
        let final_error = format!(
            "Download failed after {} attempts: {}",
            DOWNLOAD_MAX_RETRIES, last_error
        );
        tracing::error!("{}", final_error);
        return Err(final_error);
    }

    // This is unreachable on non-Windows (curl fallback always returns above)
    // but needed for type checking when cfg doesn't match
    #[cfg(not(target_os = "windows"))]
    unreachable!()
}

async fn download_file_with_progress_once(
    client: &reqwest::Client,
    url: &str,
    dest: &PathBuf,
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
    operation: &str,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    tracing::info!("Starting download from {}", url);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| {
            // Log detailed error info for debugging
            let err_msg = format!("Failed to start download from {}: {} (is_timeout={}, is_connect={}, is_request={})",
                url, e, e.is_timeout(), e.is_connect(), e.is_request());
            tracing::error!("{}", err_msg);
            format!("Failed to start download: {}", e)
        })?;

    if !response.status().is_success() {
        let err_msg = format!(
            "Download failed with status: {} ({})",
            response.status(),
            url
        );
        tracing::error!("{}", err_msg);
        return Err(err_msg);
    }

    let total_size = response.content_length();
    let mut downloaded: u64 = 0;

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create file: {}", e))?;

    let mut stream = response.bytes_stream();
    let mut last_percent: u8 = 0;

    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write file: {}", e))?;

        downloaded += chunk.len() as u64;

        // Calculate and send progress (only if we know total size)
        if let Some(total) = total_size {
            let percent = ((downloaded as f64 / total as f64) * 50.0) as u8; // 0-50% for download
            if percent > last_percent {
                last_percent = percent;
                let _ = send_message(&sender, OutgoingMessage::Progress {
                    id: request_id.clone(),
                    operation: operation.to_string(),
                    percent: Some(percent),
                    message: format!("Downloading... {:.1} MB / {:.1} MB",
                        downloaded as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    ),
                    bytes_downloaded: Some(downloaded),
                    bytes_total: Some(total),
                }).await;
            }
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush file: {}", e))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
async fn download_file_with_curl(url: &str, dest: &PathBuf) -> Result<(), String> {
    tracing::info!("Attempting curl fallback download from {}", url);

    let output = Command::new("curl")
        .arg("--fail")
        .arg("--location")
        .arg("--show-error")
        .arg("--connect-timeout")
        .arg("30")
        .arg("--max-time")
        .arg("600")  // 10 minute max for large downloads
        .arg("--retry")
        .arg("2")
        .arg("--output")
        .arg(dest)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            let err_msg = format!("Failed to run curl: {}", e);
            tracing::error!("{}", err_msg);
            err_msg
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let err_msg = format!("curl download failed (exit code {:?}): {}",
            output.status.code(), stderr.trim());
        tracing::error!("{}", err_msg);
        return Err(err_msg);
    }

    tracing::info!("curl fallback download completed successfully");
    Ok(())
}

/// Run the Miniconda installer on Unix (macOS/Linux)
async fn run_unix_miniconda_installer(installer_path: &PathBuf, install_path: &PathBuf) -> Result<(), String> {
    // Make installer executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(installer_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to make installer executable: {}", e))?;
    }

    // Run installer in batch mode
    // -b = batch mode (no prompts)
    // -p = prefix (install location)
    // -u = update existing installation
    let output = Command::new("bash")
        .arg(installer_path)
        .arg("-b")
        .arg("-p")
        .arg(install_path)
        .arg("-u")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run installer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Installer failed: {}", stderr));
    }

    Ok(())
}

/// Run the Miniconda installer on Windows
#[cfg(target_os = "windows")]
async fn run_windows_miniconda_installer(installer_path: &PathBuf, install_path: &PathBuf) -> Result<(), String> {
    // Run installer silently
    // /S = silent
    // /D= = destination (no space after =)
    let output = Command::new(installer_path)
        .arg("/S")
        .arg(format!("/D={}", install_path.display()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run installer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Installer failed: {}", stderr));
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
async fn run_windows_miniconda_installer(_installer_path: &PathBuf, _install_path: &PathBuf) -> Result<(), String> {
    Err("Windows installer not supported on this platform".to_string())
}

/// Install Git automatically
/// - macOS: Triggers Xcode Command Line Tools installation (native GUI dialog)
/// - Windows: Downloads and runs Git installer silently
/// - Linux: Returns instructions (requires sudo)
pub async fn install_git(
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
) -> Result<Value, String> {
    // Check if git is already installed
    if let Some(git_path) = find_git_binary() {
        return Ok(json!({
            "success": true,
            "already_installed": true,
            "git_path": git_path.to_string_lossy(),
            "message": "Git is already installed"
        }));
    }

    let os = std::env::consts::OS;

    match os {
        "macos" => install_git_macos(request_id, sender).await,
        "windows" => install_git_windows(request_id, sender).await,
        "linux" => {
            // Linux typically requires sudo for package manager
            Ok(json!({
                "success": false,
                "needs_manual_install": true,
                "instructions": "Please install Git using your package manager:\n\
                    - Ubuntu/Debian: sudo apt install git\n\
                    - Fedora: sudo dnf install git\n\
                    - Arch: sudo pacman -S git\n\n\
                    After installing, come back and I'll detect it automatically.",
                "message": "Git installation on Linux requires sudo. Please install manually."
            }))
        }
        _ => Err(format!("Unsupported platform: {}", os)),
    }
}

/// Find git binary in known paths
fn find_git_binary() -> Option<PathBuf> {
    // Check common paths
    let known_paths = [
        "/usr/bin/git",
        "/usr/local/bin/git",
        "/opt/homebrew/bin/git",
        // Windows paths
        "C:\\Program Files\\Git\\bin\\git.exe",
        "C:\\Program Files (x86)\\Git\\bin\\git.exe",
    ];

    for path in &known_paths {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    // Fall back to which/where
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("where");
        cmd.arg("git").creation_flags(CREATE_NO_WINDOW);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                if let Some(first_line) = String::from_utf8_lossy(&output.stdout).lines().next() {
                    if !first_line.is_empty() {
                        return Some(PathBuf::from(first_line));
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = std::process::Command::new("which").arg("git").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(PathBuf::from(path_str));
                }
            }
        }
    }

    None
}

/// Install Git on macOS via Xcode Command Line Tools
/// This triggers a native macOS GUI dialog - no terminal needed
async fn install_git_macos(
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
) -> Result<Value, String> {
    // Send initial progress
    let _ = send_message(&sender, OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "install_git".to_string(),
        percent: Some(10),
        message: "Triggering Xcode Command Line Tools installation...".to_string(),
        bytes_downloaded: None,
        bytes_total: None,
    }).await;

    // Check if Xcode CLI tools are already installed
    let xcode_check = std::process::Command::new("xcode-select")
        .arg("-p")
        .output();

    if let Ok(output) = xcode_check {
        if output.status.success() {
            // Xcode CLI tools installed, but git might not be in expected path
            // Try to find git
            if let Some(git_path) = find_git_binary() {
                return Ok(json!({
                    "success": true,
                    "already_installed": true,
                    "git_path": git_path.to_string_lossy(),
                    "message": "Git is already installed via Xcode Command Line Tools"
                }));
            }
        }
    }

    // Trigger Xcode Command Line Tools installation
    // This opens a native macOS dialog asking the user to install
    let install_result = std::process::Command::new("xcode-select")
        .arg("--install")
        .output();

    match install_result {
        Ok(output) => {
            if output.status.success() || output.status.code() == Some(1) {
                // status code 1 means installation dialog was triggered
                // Now we need to wait for the user to complete the installation
                let _ = send_message(&sender, OutgoingMessage::Progress {
                    id: request_id.clone(),
                    operation: "install_git".to_string(),
                    percent: Some(20),
                    message: "Installation dialog opened. Please click 'Install' in the popup...".to_string(),
                    bytes_downloaded: None,
                    bytes_total: None,
                }).await;

                // Poll for git to become available (user needs to click Install in the dialog)
                // Wait up to 10 minutes for the installation to complete
                let max_wait_secs = 600;
                let poll_interval_secs = 5;
                let mut waited = 0;

                while waited < max_wait_secs {
                    sleep(Duration::from_secs(poll_interval_secs)).await;
                    waited += poll_interval_secs;

                    // Check if git is now available
                    if let Some(git_path) = find_git_binary() {
                        let _ = send_message(&sender, OutgoingMessage::Progress {
                            id: request_id.clone(),
                            operation: "install_git".to_string(),
                            percent: Some(100),
                            message: "Git installed successfully!".to_string(),
                            bytes_downloaded: None,
                            bytes_total: None,
                        }).await;

                        return Ok(json!({
                            "success": true,
                            "already_installed": false,
                            "git_path": git_path.to_string_lossy(),
                            "message": "Git installed successfully via Xcode Command Line Tools"
                        }));
                    }

                    // Update progress
                    let progress = 20 + ((waited as f64 / max_wait_secs as f64) * 70.0) as u8;
                    let _ = send_message(&sender, OutgoingMessage::Progress {
                        id: request_id.clone(),
                        operation: "install_git".to_string(),
                        percent: Some(progress.min(90)),
                        message: "Waiting for Xcode Command Line Tools installation to complete...".to_string(),
                        bytes_downloaded: None,
                        bytes_total: None,
                    }).await;
                }

                // Timed out waiting for installation
                Ok(json!({
                    "success": false,
                    "pending_install": true,
                    "message": "Installation dialog was opened. Please complete the installation and try again.",
                    "instructions": "Click 'Install' in the Xcode Command Line Tools dialog, wait for it to complete, then continue."
                }))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Check if it says already installed
                if stderr.contains("already installed") {
                    if let Some(git_path) = find_git_binary() {
                        return Ok(json!({
                            "success": true,
                            "already_installed": true,
                            "git_path": git_path.to_string_lossy(),
                            "message": "Xcode Command Line Tools already installed"
                        }));
                    }
                }
                Err(format!("Failed to trigger Xcode CLI tools installation: {}", stderr))
            }
        }
        Err(e) => Err(format!("Failed to run xcode-select: {}", e)),
    }
}

/// Install Git on Windows by downloading and running the installer silently
async fn install_git_windows(
    request_id: String,
    sender: Arc<Mutex<Option<WsSender>>>,
) -> Result<Value, String> {
    let home_dir = dirs::home_dir().ok_or("Could not determine home directory")?;

    // Send initial progress
    let _ = send_message(&sender, OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "install_git".to_string(),
        percent: Some(0),
        message: "Fetching latest Git for Windows version...".to_string(),
        bytes_downloaded: None,
        bytes_total: None,
    }).await;

    // Get the latest Git for Windows release URL
    // We'll use a known stable version to avoid API calls
    let arch = std::env::consts::ARCH;
    let installer_url = if arch == "x86_64" {
        // Use a recent stable version - Git for Windows 2.43.0
        "https://github.com/git-for-windows/git/releases/download/v2.43.0.windows.1/Git-2.43.0-64-bit.exe"
    } else {
        "https://github.com/git-for-windows/git/releases/download/v2.43.0.windows.1/Git-2.43.0-32-bit.exe"
    };

    // Create temp directory for installer
    let temp_dir = home_dir.join(".braindrive-installer").join("downloads");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create download directory: {}", e))?;

    let installer_path = temp_dir.join("Git-installer.exe");

    // Download the installer
    let _ = send_message(&sender, OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "install_git".to_string(),
        percent: Some(5),
        message: "Downloading Git for Windows...".to_string(),
        bytes_downloaded: None,
        bytes_total: None,
    }).await;

    download_file_with_progress(
        installer_url,
        &installer_path,
        request_id.clone(),
        sender.clone(),
        "install_git",
    ).await?;

    // Run the installer silently
    let _ = send_message(&sender, OutgoingMessage::Progress {
        id: request_id.clone(),
        operation: "install_git".to_string(),
        percent: Some(60),
        message: "Installing Git (this may take a minute)...".to_string(),
        bytes_downloaded: None,
        bytes_total: None,
    }).await;

    // Run Git installer with silent options
    // /VERYSILENT = no UI at all
    // /NORESTART = don't restart
    // /NOCANCEL = prevent user from cancelling
    // /SP- = skip "This will install..." prompt
    let output = Command::new(&installer_path)
        .args(["/VERYSILENT", "/NORESTART", "/NOCANCEL", "/SP-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run Git installer: {}", e))?;

    // Clean up installer
    let _ = std::fs::remove_file(&installer_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git installer failed: {}", stderr));
    }

    // Verify installation
    // Give it a moment to finish
    sleep(Duration::from_secs(2)).await;

    if let Some(git_path) = find_git_binary() {
        let _ = send_message(&sender, OutgoingMessage::Progress {
            id: request_id.clone(),
            operation: "install_git".to_string(),
            percent: Some(100),
            message: "Git installed successfully!".to_string(),
            bytes_downloaded: None,
            bytes_total: None,
        }).await;

        Ok(json!({
            "success": true,
            "already_installed": false,
            "git_path": git_path.to_string_lossy(),
            "message": "Git installed successfully"
        }))
    } else {
        Err("Git installation completed but git binary not found. You may need to restart the bootstrapper.".to_string())
    }
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

    let mut cmd = std::process::Command::new(&ollama_path);
    cmd.arg("--version");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output().ok()?;

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
/// Handles the case where ~/BrainDrive already exists with miniconda3 (from install_conda)
pub async fn clone_repo(repo_url: Option<String>, target_path: Option<String>) -> Result<Value, String> {
    // Use find_git_binary to get absolute path (GUI apps have limited PATH)
    let git_path = find_git_binary()
        .ok_or("Git is not installed. Please install Git first.")?;

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

    // Default to BrainDrive-Core repo
    let url = repo_url.unwrap_or_else(|| "https://github.com/BrainDriveAI/BrainDrive.git".to_string());

    // Validate URL format (basic check)
    if !url.starts_with("https://") && !url.starts_with("git@") {
        return Err("Repository URL must start with https:// or git@".to_string());
    }

    // Check if already exists
    if target.exists() {
        if target.join(".git").exists() {
            return Ok(json!({
                "success": true,
                "message": "BrainDrive repository already exists",
                "path": target.to_string_lossy(),
                "already_exists": true
            }));
        }

        // Check if directory only contains installer artifacts (miniconda3, .braindrive-installer)
        // If so, we can clone into it using git init + fetch approach
        let has_only_installer_artifacts = check_only_installer_artifacts(&target);

        if has_only_installer_artifacts {
            // Use git init + fetch + checkout approach for existing directory
            return clone_into_existing_dir(&target, &url, &git_path).await;
        } else {
            return Err(format!(
                "Directory {} exists but is not a git repository and contains non-installer files",
                target.display()
            ));
        }
    }

    // Standard clone for non-existing directory
    let mut command = Command::new(&git_path);
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

/// Check if a directory only contains installer artifacts (miniconda3, .braindrive-installer)
fn check_only_installer_artifacts(dir: &PathBuf) -> bool {
    let allowed_names = ["miniconda3", ".braindrive-installer"];

    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !allowed_names.contains(&name_str.as_ref()) {
                    return false;
                }
            }
            true
        }
        Err(_) => false,
    }
}

/// Clone into an existing directory that contains only installer artifacts
/// Uses git init + fetch + checkout approach
async fn clone_into_existing_dir(target: &PathBuf, url: &str, git_path: &PathBuf) -> Result<Value, String> {
    // Initialize git repo
    let mut init_cmd = Command::new(git_path);
    init_cmd.arg("init").current_dir(target);
    let init_result = run_command(init_cmd).await?;
    if !init_result.success {
        return Err(format!("Failed to initialize git repository: {}", init_result.stderr));
    }

    // Add remote origin
    let mut remote_cmd = Command::new(git_path);
    remote_cmd
        .args(["remote", "add", "origin", url])
        .current_dir(target);
    let remote_result = run_command(remote_cmd).await?;
    if !remote_result.success {
        return Err(format!("Failed to add remote: {}", remote_result.stderr));
    }

    // Fetch with depth 1
    let mut fetch_cmd = Command::new(git_path);
    fetch_cmd
        .args(["fetch", "--depth", "1", "origin", "main"])
        .current_dir(target);
    let fetch_result = run_command(fetch_cmd).await?;
    if !fetch_result.success {
        // Try 'master' branch if 'main' doesn't exist
        let mut fetch_master = Command::new(git_path);
        fetch_master
            .args(["fetch", "--depth", "1", "origin", "master"])
            .current_dir(target);
        let fetch_master_result = run_command(fetch_master).await?;
        if !fetch_master_result.success {
            return Err(format!("Failed to fetch repository: {}", fetch_result.stderr));
        }
        // Checkout master
        let mut checkout_cmd = Command::new(git_path);
        checkout_cmd
            .args(["checkout", "-b", "master", "origin/master"])
            .current_dir(target);
        let checkout_result = run_command(checkout_cmd).await?;
        if !checkout_result.success {
            return Err(format!("Failed to checkout: {}", checkout_result.stderr));
        }
    } else {
        // Checkout main
        let mut checkout_cmd = Command::new(git_path);
        checkout_cmd
            .args(["checkout", "-b", "main", "origin/main"])
            .current_dir(target);
        let checkout_result = run_command(checkout_cmd).await?;
        if !checkout_result.success {
            return Err(format!("Failed to checkout: {}", checkout_result.stderr));
        }
    }

    Ok(json!({
        "success": true,
        "message": "BrainDrive repository cloned into existing directory",
        "path": target.to_string_lossy(),
        "url": url,
        "method": "init_fetch_checkout"
    }))
}

/// Install backend Python dependencies using pip in conda environment
/// Uses the isolated conda installation at ~/BrainDrive/miniconda3
pub async fn install_backend_deps(
    env_name: Option<String>,
    repo_path: Option<String>,
) -> Result<Value, String> {
    // Get the conda binary path (prefers isolated installation)
    let conda_path = find_conda_binary()
        .ok_or("Conda is not installed. Please install it first using the install_conda tool.")?;

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

    // Build the pip install command to run in conda environment using the isolated conda
    let pip_cmd = format!(
        "pip install -r \"{}\"",
        requirements_file.display()
    );
    let full_cmd = process_manager::conda_run_command_with_path(&conda_path, &env, &pip_cmd);

    let result = run_shell_script(&full_cmd).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "env_name": env,
        "requirements_file": requirements_file.to_string_lossy(),
        "conda_path": conda_path.to_string_lossy()
    }))
}

/// Install frontend npm dependencies
pub async fn install_frontend_deps(
    env_name: Option<String>,
    repo_path: Option<String>,
) -> Result<Value, String> {
    // Get the conda binary path (prefers isolated installation)
    let conda_path = find_conda_binary()
        .ok_or("Conda is not installed. Please install it first using the install_conda tool.")?;

    let env = sanitize_env_name(&env_name.unwrap_or_else(|| CONDA_ENV_NAME.to_string()))?;
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

    let npm_cmd = if cfg!(target_os = "windows") {
        format!(
            "cmd /C \"cd /d {} && npm install\"",
            frontend_path.display()
        )
    } else {
        format!("cd \"{}\" && npm install", frontend_path.display())
    };
    let full_cmd = process_manager::conda_run_command_with_path(&conda_path, &env, &npm_cmd);

    let result = run_shell_script(&full_cmd).await?;

    Ok(json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "frontend_path": frontend_path.to_string_lossy(),
        "env_name": env,
        "conda_path": conda_path.to_string_lossy()
    }))
}

/// Install both backend and frontend dependencies in parallel
/// This saves ~1-1.5 minutes compared to sequential installation
pub async fn install_all_deps(
    env_name: Option<String>,
    repo_path: Option<String>,
) -> Result<Value, String> {
    // Clone the values for the parallel tasks
    let env_name_backend = env_name.clone();
    let env_name_frontend = env_name.clone();
    let repo_path_backend = repo_path.clone();
    let repo_path_frontend = repo_path;

    // Run both installations in parallel
    let (backend_result, frontend_result) = tokio::join!(
        install_backend_deps(env_name_backend, repo_path_backend),
        install_frontend_deps(env_name_frontend, repo_path_frontend)
    );

    // Process results
    let backend_success = backend_result.as_ref().map(|v| {
        v.get("success").and_then(|s| s.as_bool()).unwrap_or(false)
    }).unwrap_or(false);

    let frontend_success = frontend_result.as_ref().map(|v| {
        v.get("success").and_then(|s| s.as_bool()).unwrap_or(false)
    }).unwrap_or(false);

    let overall_success = backend_success && frontend_success;

    // Build detailed response
    let backend_data = match backend_result {
        Ok(data) => data,
        Err(e) => json!({ "success": false, "error": e }),
    };

    let frontend_data = match frontend_result {
        Ok(data) => data,
        Err(e) => json!({ "success": false, "error": e }),
    };

    let message = if overall_success {
        "Both backend and frontend dependencies installed successfully"
    } else if backend_success {
        "Backend dependencies installed, but frontend failed"
    } else if frontend_success {
        "Frontend dependencies installed, but backend failed"
    } else {
        "Both backend and frontend dependency installations failed"
    };

    // Return Err if overall operation failed, so ToolResult.success reflects actual status
    if overall_success {
        Ok(json!({
            "success": true,
            "message": message,
            "parallel": true,
            "backend": backend_data,
            "frontend": frontend_data
        }))
    } else {
        // Include detailed results in the error for debugging
        Err(format!(
            "{}\n\nBackend: {}\nFrontend: {}",
            message,
            serde_json::to_string_pretty(&backend_data).unwrap_or_default(),
            serde_json::to_string_pretty(&frontend_data).unwrap_or_default()
        ))
    }
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
/// Uses the isolated conda installation at ~/BrainDrive/miniconda3
/// If force_recreate is true, removes existing env and creates fresh one
pub async fn create_conda_env(env_name: Option<String>, force_recreate: Option<bool>) -> Result<Value, String> {
    // Get the conda binary path (prefers isolated installation)
    let conda_path = find_conda_binary()
        .ok_or("Conda is not installed. Please install it first using the install_conda tool.")?;

    let env = sanitize_env_name(&env_name.unwrap_or_else(|| CONDA_ENV_NAME.to_string()))?;
    let force = force_recreate.unwrap_or(false);

    // Check if environment already exists
    let check_cmd = Command::new(&conda_path)
        .args(["env", "list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to list conda environments: {}", e))?;

    let env_list = String::from_utf8_lossy(&check_cmd.stdout);
    let env_exists = env_list.lines().any(|line| line.split_whitespace().next() == Some(&env));

    if env_exists && !force {
        return Ok(json!({
            "success": true,
            "message": format!("Conda environment '{}' already exists", env),
            "env_name": env,
            "already_exists": true
        }));
    }

    // If force_recreate and env exists, remove it first
    if env_exists && force {
        let mut remove_cmd = Command::new(&conda_path);
        remove_cmd.args(["env", "remove", "-n", &env, "-y"]);
        let remove_result = run_command(remove_cmd).await?;
        if !remove_result.success {
            return Err(format!("Failed to remove existing environment: {}", remove_result.stderr));
        }
    }

    // Create the environment with Python 3.11, nodejs, and git from conda-forge
    // Use --override-channels to bypass Anaconda channel TOS requirements
    let mut command = Command::new(&conda_path);
    command
        .args([
            "create",
            "-n", &env,
            "--override-channels",
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
        "env_name": env,
        "conda_path": conda_path.to_string_lossy(),
        "recreated": force && env_exists
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
#[cfg(not(target_os = "windows"))]
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

/// Start the backend service on Windows
#[cfg(target_os = "windows")]
async fn start_backend_service(backend_path: &PathBuf, port: u16) -> Result<Option<u32>, String> {
    // Create a batch script to run the backend with conda
    let script_content = format!(
        r#"@echo off
cd /d "{}"
call conda activate {}
uvicorn main:app --host 0.0.0.0 --port {}
"#,
        backend_path.display(),
        CONDA_ENV_NAME,
        port
    );

    // Write the script to a temporary location
    let script_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".braindrive-installer")
        .join("scripts");

    std::fs::create_dir_all(&script_dir)
        .map_err(|e| format!("Failed to create scripts directory: {}", e))?;

    let script_path = script_dir.join("start_backend.bat");
    std::fs::write(&script_path, &script_content)
        .map_err(|e| format!("Failed to write startup script: {}", e))?;

    // Spawn the script using cmd.exe
    let pid = spawn_detached(
        "cmd.exe",
        &["/C", script_path.to_str().unwrap()],
        backend_path,
        &[],
    )
    .await?;

    Ok(Some(pid))
}

/// Start the frontend service
#[cfg(not(target_os = "windows"))]
async fn start_frontend_service(frontend_path: &PathBuf, port: u16) -> Result<Option<u32>, String> {
    // Create a shell script to run the frontend
    let conda_activate = process_manager::conda_run_command(CONDA_ENV_NAME, "true")
        .replace(" true", "");
    let script_content = format!(
        r#"#!/bin/bash
set -e
cd "{}"
{}
exec npm run dev -- --host localhost --port {}
"#,
        frontend_path.display(),
        conda_activate,
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

/// Start the frontend service on Windows
#[cfg(target_os = "windows")]
async fn start_frontend_service(frontend_path: &PathBuf, port: u16) -> Result<Option<u32>, String> {
    // Create a batch script to run the frontend
    let script_content = format!(
        r#"@echo off
cd /d "{}"
call conda activate {}
npm run dev -- --host localhost --port {}
"#,
        frontend_path.display(),
        CONDA_ENV_NAME,
        port
    );

    let script_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".braindrive-installer")
        .join("scripts");

    std::fs::create_dir_all(&script_dir)
        .map_err(|e| format!("Failed to create scripts directory: {}", e))?;

    let script_path = script_dir.join("start_frontend.bat");
    std::fs::write(&script_path, &script_content)
        .map_err(|e| format!("Failed to write startup script: {}", e))?;

    // Spawn the script using cmd.exe
    let pid = spawn_detached(
        "cmd.exe",
        &["/C", script_path.to_str().unwrap()],
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

#[cfg(target_os = "windows")]
async fn run_shell_script(script: &str) -> Result<CommandOutput, String> {
    let mut command = Command::new("cmd.exe");
    // Use /S /C with the command wrapped in quotes to handle nested quotes properly
    // Without /S, cmd.exe has special parsing when the command starts with a quote
    // that can break commands with multiple quoted paths
    command.arg("/S").arg("/C").arg(format!("\"{}\"", script));
    command.creation_flags(CREATE_NO_WINDOW);
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
    #[cfg(target_os = "windows")]
    {
        let mut command = StdCommand::new("where");
        command
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        command.status().map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
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

    // On Windows, canonicalize() can add \\?\ prefix or change casing,
    // Strip the prefix and compare case-insensitively
    #[cfg(target_os = "windows")]
    let result = {
        let canonical_str = canonical.to_string_lossy();
        let clean_path = canonical_str.trim_start_matches(r"\\?\");
        let clean_pathbuf = PathBuf::from(clean_path);

        let home_str = home.to_string_lossy().to_lowercase();
        let clean_lower = clean_path.to_lowercase();
        if !clean_lower.starts_with(&home_str) {
            return Err("Repository path must be inside your home directory".to_string());
        }
        clean_pathbuf
    };

    #[cfg(not(target_os = "windows"))]
    let result = {
        if !canonical.starts_with(&home) {
            return Err("Repository path must be inside your home directory".to_string());
        }
        canonical
    };

    Ok(result)
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
    // On Windows, canonicalize() can add \\?\ prefix - strip it and compare case-insensitively
    #[cfg(target_os = "windows")]
    let result = {
        let resolved_str = resolved.to_string_lossy();
        let clean_path = resolved_str.trim_start_matches(r"\\?\");
        let clean_pathbuf = PathBuf::from(clean_path);

        let home_str = home.to_string_lossy().to_lowercase();
        let clean_lower = clean_path.to_lowercase();
        if !clean_lower.starts_with(&home_str) {
            return Err("Repository path must be inside your home directory".to_string());
        }
        clean_pathbuf
    };

    #[cfg(not(target_os = "windows"))]
    let result = {
        if !resolved.starts_with(&home) {
            return Err("Repository path must be inside your home directory".to_string());
        }
        resolved
    };

    Ok(result)
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
