use crate::process_manager::is_port_in_use;
use crate::{GpuInfo, SystemInfo};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use sysinfo::{Disks, System};

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
fn find_ollama_binary() -> Option<PathBuf> {
    for path in OLLAMA_KNOWN_PATHS {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    // Also check if it's in PATH (for cases where user has custom setup)
    if let Ok(output) = Command::new("which").arg("ollama").output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }

    None
}

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

    // Use absolute path detection for Ollama (GUI apps have minimal PATH)
    let ollama_path = find_ollama_binary();
    let ollama_installed = ollama_path.is_some();
    let ollama_running = is_port_in_use(OLLAMA_DEFAULT_PORT);
    let ollama_version = if let Some(ref path) = ollama_path {
        get_ollama_version_from_path(path)
    } else {
        None
    };

    let braindrive_path = dirs::home_dir()
        .map(|p| p.join("BrainDrive"))
        .unwrap_or_else(|| PathBuf::from("~/BrainDrive"));
    let braindrive_exists = braindrive_path.exists();

    let system = System::new_all();

    let cpu_brand = system.cpus().first().map(|cpu| cpu.brand().trim().to_string()).filter(|s| !s.is_empty());
    let cpu_logical_cores = match system.cpus().len() {
        0 => None,
        count => Some(count as u32),
    };
    let cpu_physical_cores = system.physical_core_count().map(|count| count as u32);

    let memory_gb = {
        let total_memory = system.total_memory();
        if total_memory > 0 {
            Some(bytes_to_gib(total_memory))
        } else {
            None
        }
    };

    let disk_free_gb = {
        let disks = Disks::new_with_refreshed_list();
        let available_bytes: u64 = disks.iter().map(|d| d.available_space()).sum();
        if available_bytes > 0 {
            Some(bytes_to_gib(available_bytes))
        } else {
            None
        }
    };

    let gpus = detect_gpus();

    Ok(SystemInfo {
        os,
        arch,
        hostname,
        home_dir,
        conda_installed,
        git_installed,
        node_installed,
        ollama_installed,
        ollama_running,
        ollama_version,
        braindrive_exists,
        cpu_brand,
        cpu_physical_cores,
        cpu_logical_cores,
        memory_gb,
        gpus,
        disk_free_gb,
    })
}

fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024f64 * 1024f64 * 1024f64)
}

fn detect_gpus() -> Vec<GpuInfo> {
    #[cfg(target_os = "macos")]
    {
        return detect_macos_gpus();
    }

    #[cfg(target_os = "windows")]
    {
        return detect_windows_gpus();
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn detect_macos_gpus() -> Vec<GpuInfo> {
    let output = Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(entries) = value
                    .get("SPDisplaysDataType")
                    .and_then(|v| v.as_array())
                {
                    return entries
                        .iter()
                        .filter_map(|entry| {
                            let name = entry
                                .get("_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown GPU");

                            let vram = entry
                                .get("spdisplays_vram")
                                .or_else(|| entry.get("spdisplays_vram_shared"))
                                .and_then(|v| v.as_str())
                                .and_then(parse_vram_string);

                            Some(GpuInfo {
                                name: name.to_string(),
                                vram_gb: vram,
                            })
                        })
                        .collect();
                }
            }
        }
    }

    Vec::new()
}

#[cfg(target_os = "windows")]
fn detect_windows_gpus() -> Vec<GpuInfo> {
    let script =
        "Get-CimInstance Win32_VideoController | Select-Object Name,AdapterRAM | ConvertTo-Json";

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(array) = value.as_array() {
                    return array
                        .iter()
                        .filter_map(|entry| {
                            let name = entry
                                .get("Name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown GPU")
                                .to_string();

                            let vram_gb = entry
                                .get("AdapterRAM")
                                .and_then(|v| v.as_u64())
                                .map(bytes_to_gib);

                            Some(GpuInfo { name, vram_gb })
                        })
                        .collect();
                } else if let Some(obj) = value.as_object() {
                    let name = obj
                        .get("Name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown GPU")
                        .to_string();
                    let vram_gb = obj
                        .get("AdapterRAM")
                        .and_then(|v| v.as_u64())
                        .map(bytes_to_gib);
                    return vec![GpuInfo { name, vram_gb }];
                }
            }
        }
    }

    Vec::new()
}

fn parse_vram_string(input: &str) -> Option<f64> {
    let mut parts = input.trim().split_whitespace();
    let value_part = parts.next()?;
    let unit_part = parts.next().unwrap_or("MB").to_uppercase();
    let numeric = value_part.replace(',', "");
    let value = numeric.parse::<f64>().ok()?;

    match unit_part.as_str() {
        "KB" => Some(value / 1_048_576.0),
        "MB" => Some(value / 1024.0),
        "GB" => Some(value),
        _ => None,
    }
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

/// Get Ollama version string using absolute path (e.g., "0.1.17")
fn get_ollama_version_from_path(ollama_path: &PathBuf) -> Option<String> {
    let output = Command::new(ollama_path)
        .arg("--version")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output format is typically "ollama version 0.1.17" or just "0.1.17"
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
