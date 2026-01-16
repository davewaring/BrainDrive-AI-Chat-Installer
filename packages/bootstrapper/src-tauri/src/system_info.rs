use crate::process_manager::is_port_in_use;
use crate::{GpuInfo, SystemInfo};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use sysinfo::{Disks, System};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Create a Command that won't show a console window on Windows
fn silent_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

const OLLAMA_DEFAULT_PORT: u16 = 11434;

/// Known paths where Ollama might be installed
/// GUI apps often have minimal PATH, so we check absolute paths directly
const OLLAMA_KNOWN_PATHS: &[&str] = &[
    // Unix paths
    "/usr/local/bin/ollama",
    "/opt/homebrew/bin/ollama",
    "/usr/bin/ollama",
    "/snap/bin/ollama",
    // Windows paths - checked at runtime via home directory
];

/// BrainDrive directory name for isolated installations
const DEFAULT_REPO_DIR: &str = "BrainDrive";
/// Isolated miniconda directory name
const ISOLATED_MINICONDA_DIR: &str = "miniconda3";

/// BrainDrive conda environment name
const BRAINDRIVE_ENV_NAME: &str = "BrainDriveDev";

/// Check if the isolated BrainDrive Miniconda is installed at ~/BrainDrive/miniconda3
fn check_isolated_conda_installed() -> bool {
    if let Some(home) = dirs::home_dir() {
        #[cfg(not(target_os = "windows"))]
        let isolated_path = home.join(DEFAULT_REPO_DIR).join(ISOLATED_MINICONDA_DIR).join("bin/conda");
        #[cfg(target_os = "windows")]
        let isolated_path = home.join(DEFAULT_REPO_DIR).join(ISOLATED_MINICONDA_DIR).join("Scripts\\conda.exe");

        return isolated_path.exists();
    }
    false
}

/// Check if the BrainDrive conda environment is ready (has git and node)
/// This checks if ~/BrainDrive/miniconda3/envs/BrainDriveDev exists
fn check_braindrive_env_ready() -> bool {
    if let Some(home) = dirs::home_dir() {
        let env_path = home
            .join(DEFAULT_REPO_DIR)
            .join(ISOLATED_MINICONDA_DIR)
            .join("envs")
            .join(BRAINDRIVE_ENV_NAME);

        // Check if the environment directory exists and has python
        #[cfg(not(target_os = "windows"))]
        let python_path = env_path.join("bin/python");
        #[cfg(target_os = "windows")]
        let python_path = env_path.join("python.exe");

        return python_path.exists();
    }
    false
}

/// Find Ollama binary in known paths
fn find_ollama_binary() -> Option<PathBuf> {
    // Check Unix paths
    #[cfg(not(target_os = "windows"))]
    for path in OLLAMA_KNOWN_PATHS {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    // Check Windows paths
    #[cfg(target_os = "windows")]
    {
        // Check common Windows install locations
        if let Some(local_app_data) = dirs::data_local_dir() {
            let ollama_path = local_app_data.join("Programs\\Ollama\\ollama.exe");
            if ollama_path.exists() {
                return Some(ollama_path);
            }
        }
        // Check Program Files
        let program_paths = [
            "C:\\Program Files\\Ollama\\ollama.exe",
            "C:\\Program Files (x86)\\Ollama\\ollama.exe",
        ];
        for path in &program_paths {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Fall back to which/where
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = silent_command("which").arg("ollama").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(PathBuf::from(path_str));
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = silent_command("where").arg("ollama").output() {
            if output.status.success() {
                if let Some(first_line) = String::from_utf8_lossy(&output.stdout).lines().next() {
                    if !first_line.is_empty() {
                        return Some(PathBuf::from(first_line));
                    }
                }
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

    // Check isolated conda installation at ~/BrainDrive/miniconda3
    let conda_installed = check_isolated_conda_installed();
    // Check if BrainDrive conda environment is ready (includes git, node, python)
    let braindrive_env_ready = check_braindrive_env_ready();

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
        braindrive_env_ready,
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
    let output = silent_command("system_profiler")
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

    let output = silent_command("powershell.exe")
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

/// Get Ollama version string using absolute path (e.g., "0.1.17")
fn get_ollama_version_from_path(ollama_path: &PathBuf) -> Option<String> {
    let output = silent_command(ollama_path)
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
