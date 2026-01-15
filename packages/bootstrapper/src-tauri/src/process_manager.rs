use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

/// Information about a running BrainDrive service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub pid: Option<u32>,
    pub port: u16,
    pub running: bool,
}

/// Tracks the state of BrainDrive processes
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrainDriveState {
    pub backend: Option<ServiceInfo>,
    pub frontend: Option<ServiceInfo>,
}

impl BrainDriveState {
    pub fn is_running(&self) -> bool {
        self.backend.as_ref().map_or(false, |s| s.running)
            || self.frontend.as_ref().map_or(false, |s| s.running)
    }
}

/// Shared state for process management
pub type ProcessState = Arc<Mutex<BrainDriveState>>;

/// Create a new process state
pub fn new_process_state() -> ProcessState {
    Arc::new(Mutex::new(BrainDriveState::default()))
}

/// Check if a process is running by PID
#[cfg(unix)]
fn is_pid_running(pid: u32) -> bool {
    use std::process::Command as StdCommand;
    // On Unix, we can use kill -0 to check if a process exists
    StdCommand::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_pid_running(pid: u32) -> bool {
    use std::process::Command as StdCommand;
    // On Windows, use tasklist to check if PID exists
    let output = StdCommand::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            !stdout.contains("No tasks are running")
        }
        Err(_) => false,
    }
}

/// Find the PID of a process listening on a given port
#[cfg(unix)]
pub fn find_pid_on_port(port: u16) -> Option<u32> {
    use std::process::Command as StdCommand;

    let output = StdCommand::new("lsof")
        .args(["-ti", &format!(":{}", port)])
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // lsof may return multiple PIDs, take the first one
        stdout
            .lines()
            .next()
            .and_then(|line| line.trim().parse::<u32>().ok())
    } else {
        None
    }
}

#[cfg(windows)]
pub fn find_pid_on_port(port: u16) -> Option<u32> {
    use std::process::Command as StdCommand;

    let output = StdCommand::new("netstat")
        .args(["-ano"])
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let port_str = format!(":{}", port);

        for line in stdout.lines() {
            if line.contains(&port_str) && line.contains("LISTENING") {
                // Last column is the PID
                if let Some(pid_str) = line.split_whitespace().last() {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        return Some(pid);
                    }
                }
            }
        }
    }
    None
}

/// Kill a process by PID
#[cfg(unix)]
pub fn kill_process(pid: u32) -> bool {
    use std::process::Command as StdCommand;

    // First try SIGTERM for graceful shutdown
    let term_result = StdCommand::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if term_result.map(|s| s.success()).unwrap_or(false) {
        // Give process time to terminate gracefully
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Check if still running, if so use SIGKILL
        if is_pid_running(pid) {
            let _ = StdCommand::new("kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        true
    } else {
        false
    }
}

#[cfg(windows)]
pub fn kill_process(pid: u32) -> bool {
    use std::process::Command as StdCommand;

    StdCommand::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Kill any process listening on a port
pub fn kill_process_on_port(port: u16) -> bool {
    if let Some(pid) = find_pid_on_port(port) {
        kill_process(pid)
    } else {
        // No process on port, consider it a success
        true
    }
}

/// Check if a port has a listening process that is accepting connections
/// Checks both IPv4 (127.0.0.1) and IPv6 ([::1]) localhost addresses
pub fn is_port_in_use(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let timeout = Duration::from_millis(100);

    // Check IPv4 localhost
    let ipv4_addr: SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("Valid IPv4 address");

    if TcpStream::connect_timeout(&ipv4_addr, timeout).is_ok() {
        return true;
    }

    // Check IPv6 localhost
    let ipv6_addr: SocketAddr = format!("[::1]:{}", port)
        .parse()
        .expect("Valid IPv6 address");

    TcpStream::connect_timeout(&ipv6_addr, timeout).is_ok()
}

/// Wait for a service to start listening on a port
pub async fn wait_for_port(port: u16, timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    while start.elapsed() < timeout {
        if is_port_in_use(port) {
            return true;
        }
        sleep(Duration::from_millis(250)).await;
    }
    false
}

/// Wait for a service to stop listening on a port
pub async fn wait_for_port_free(port: u16, timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    while start.elapsed() < timeout {
        if !is_port_in_use(port) {
            return true;
        }
        sleep(Duration::from_millis(250)).await;
    }
    false
}

/// Spawn a detached process that survives parent exit
#[cfg(unix)]
pub async fn spawn_detached(
    program: &str,
    args: &[&str],
    working_dir: &PathBuf,
    env_vars: &[(&str, &str)],
) -> Result<u32, String> {
    use std::os::unix::process::CommandExt;
    use std::process::Command as StdCommand;

    // Create log files for debugging
    let log_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".braindrive-installer")
        .join("logs");

    std::fs::create_dir_all(&log_dir)
        .map_err(|e| format!("Failed to create log directory: {}", e))?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let log_file = log_dir.join(format!("{}_{}.log", program.replace("/", "_"), timestamp));

    let stdout_file = std::fs::File::create(&log_file)
        .map_err(|e| format!("Failed to create log file: {}", e))?;
    let stderr_file = stdout_file.try_clone()
        .map_err(|e| format!("Failed to clone file handle: {}", e))?;

    let mut command = StdCommand::new(program);
    command
        .args(args)
        .current_dir(working_dir)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .stdin(Stdio::null());

    // Set environment variables
    for (key, value) in env_vars {
        command.env(key, value);
    }

    // Create a new process group so the process survives parent death
    unsafe {
        command.pre_exec(|| {
            // Create new session and process group
            libc::setsid();
            Ok(())
        });
    }

    let child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {}", e))?;

    let pid = child.id();

    Ok(pid)
}

#[cfg(windows)]
pub async fn spawn_detached(
    program: &str,
    args: &[&str],
    working_dir: &PathBuf,
    env_vars: &[(&str, &str)],
) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command as StdCommand;

    // Windows flags for detached process
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut command = StdCommand::new(program);
    command
        .args(args)
        .current_dir(working_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);

    for (key, value) in env_vars {
        command.env(key, value);
    }

    let child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {}", e))?;

    Ok(child.id())
}

/// Constants for isolated conda location
const DEFAULT_REPO_DIR: &str = "BrainDrive";
const ISOLATED_MINICONDA_DIR: &str = "miniconda3";

/// Get the path to the isolated conda installation (~/BrainDrive/miniconda3)
fn get_isolated_conda_base() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let isolated_path = home.join(DEFAULT_REPO_DIR).join(ISOLATED_MINICONDA_DIR);
    if isolated_path.exists() {
        Some(isolated_path)
    } else {
        None
    }
}

/// Get the conda base path
/// Priority: 1. Isolated installation (~/BrainDrive/miniconda3), 2. PATH-based conda
pub fn get_conda_base() -> Option<PathBuf> {
    // First check for isolated conda installation
    if let Some(isolated) = get_isolated_conda_base() {
        return Some(isolated);
    }

    // Fall back to PATH-based conda
    use std::process::Command as StdCommand;

    let output = StdCommand::new("conda")
        .args(["info", "--base"])
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();
        Some(PathBuf::from(path))
    } else {
        None
    }
}

/// Get the conda base path from a specific conda binary
pub fn get_conda_base_from_binary(conda_path: &PathBuf) -> Option<PathBuf> {
    use std::process::Command as StdCommand;

    let output = StdCommand::new(conda_path)
        .args(["info", "--base"])
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();
        Some(PathBuf::from(path))
    } else {
        None
    }
}

/// Build the shell command to run something in a conda environment
/// Uses the isolated conda installation if available
#[cfg(unix)]
pub fn conda_run_command(env_name: &str, command: &str) -> String {
    // Source conda.sh to ensure conda is available, then run the command
    if let Some(conda_base) = get_conda_base() {
        let conda_sh = conda_base.join("etc/profile.d/conda.sh");
        let conda_bin = conda_base.join("bin/conda");
        format!(
            "source \"{}\" && \"{}\" activate {} && {}",
            conda_sh.display(),
            conda_bin.display(),
            env_name,
            command
        )
    } else {
        // Fallback to conda run (requires conda in PATH)
        format!("conda run -n {} {}", env_name, command)
    }
}

#[cfg(windows)]
pub fn conda_run_command(env_name: &str, command: &str) -> String {
    if let Some(conda_base) = get_conda_base() {
        let conda_bin = conda_base.join("Scripts/conda.exe");
        format!("\"{}\" run -n {} {}", conda_bin.display(), env_name, command)
    } else {
        format!("conda run -n {} {}", env_name, command)
    }
}

/// Build the shell command to run something in a conda environment using a specific conda binary
#[cfg(unix)]
pub fn conda_run_command_with_path(conda_path: &PathBuf, env_name: &str, command: &str) -> String {
    if let Some(conda_base) = get_conda_base_from_binary(conda_path) {
        let conda_sh = conda_base.join("etc/profile.d/conda.sh");
        format!(
            "source \"{}\" && \"{}\" activate {} && {}",
            conda_sh.display(),
            conda_path.display(),
            env_name,
            command
        )
    } else {
        // Fallback to conda run with explicit path
        format!("\"{}\" run -n {} {}", conda_path.display(), env_name, command)
    }
}

#[cfg(windows)]
pub fn conda_run_command_with_path(conda_path: &PathBuf, env_name: &str, command: &str) -> String {
    format!("\"{}\" run -n {} {}", conda_path.display(), env_name, command)
}
