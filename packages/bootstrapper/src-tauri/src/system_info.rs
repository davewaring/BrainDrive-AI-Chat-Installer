use crate::SystemInfo;
use std::path::PathBuf;
use std::process::Command;

pub async fn detect() -> Result<SystemInfo, String> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let home_dir = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let conda_installed = check_command_exists("conda");
    let git_installed = check_command_exists("git");
    let node_installed = check_command_exists("node");
    let ollama_installed = check_command_exists("ollama");

    let braindrive_path = dirs::home_dir()
        .map(|p| p.join("BrainDrive"))
        .unwrap_or_else(|| PathBuf::from("~/BrainDrive"));
    let braindrive_exists = braindrive_path.exists();

    Ok(SystemInfo {
        os,
        arch,
        hostname,
        home_dir,
        conda_installed,
        git_installed,
        node_installed,
        ollama_installed,
        braindrive_exists,
    })
}

fn check_command_exists(cmd: &str) -> bool {
    if cfg!(target_os = "windows") {
        Command::new("where")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
