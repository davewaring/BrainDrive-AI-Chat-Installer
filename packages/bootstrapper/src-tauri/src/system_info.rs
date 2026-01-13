use crate::{GpuInfo, SystemInfo};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use sysinfo::{Disks, System};

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
