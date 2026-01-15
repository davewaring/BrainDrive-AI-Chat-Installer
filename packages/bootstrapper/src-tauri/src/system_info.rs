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

/// Known paths where Conda might be installed (Unix)
const CONDA_KNOWN_PATHS_UNIX: &[&str] = &[
    "/opt/miniconda3/bin/conda",
    "/opt/anaconda3/bin/conda",
    "/opt/homebrew/bin/conda",
    "/usr/local/bin/conda",
];

/// Known paths where Node.js might be installed (Unix)
const NODE_KNOWN_PATHS_UNIX: &[&str] = &[
    "/usr/local/bin/node",
    "/opt/homebrew/bin/node",
    "/usr/bin/node",
];

/// Known paths where Node.js might be installed (Windows)
#[cfg(target_os = "windows")]
const NODE_KNOWN_PATHS_WINDOWS: &[&str] = &[
    "C:\\Program Files\\nodejs\\node.exe",
    "C:\\Program Files (x86)\\nodejs\\node.exe",
];

/// Known paths where Git might be installed (Windows)
#[cfg(target_os = "windows")]
const GIT_KNOWN_PATHS_WINDOWS: &[&str] = &[
    "C:\\Program Files\\Git\\bin\\git.exe",
    "C:\\Program Files (x86)\\Git\\bin\\git.exe",
];

/// Check if a binary exists at known paths or via which/where command
#[allow(dead_code)]
fn check_binary_exists(known_paths: &[&str], cmd: &str) -> bool {
    // Check known paths first
    for path in known_paths {
        if PathBuf::from(path).exists() {
            return true;
        }
    }
    // Fall back to which/where
    check_command_exists(cmd)
}

/// Check if conda is installed (includes home directory paths)
/// Priority: 1. Isolated BrainDrive installation, 2. User home, 3. System-wide, 4. PATH
fn check_conda_installed() -> bool {
    // FIRST: Check the isolated BrainDrive installation (highest priority)
    if let Some(home) = dirs::home_dir() {
        #[cfg(not(target_os = "windows"))]
        let isolated_path = home.join(DEFAULT_REPO_DIR).join(ISOLATED_MINICONDA_DIR).join("bin/conda");
        #[cfg(target_os = "windows")]
        let isolated_path = home.join(DEFAULT_REPO_DIR).join(ISOLATED_MINICONDA_DIR).join("Scripts\\conda.exe");

        if isolated_path.exists() {
            return true;
        }
    }

    // Check system-wide paths (Unix only)
    #[cfg(not(target_os = "windows"))]
    for path in CONDA_KNOWN_PATHS_UNIX {
        if PathBuf::from(path).exists() {
            return true;
        }
    }

    // Check other home directory paths (works for both Unix and Windows)
    if let Some(home) = dirs::home_dir() {
        #[cfg(not(target_os = "windows"))]
        let home_paths = [
            home.join("miniconda3/bin/conda"),
            home.join("anaconda3/bin/conda"),
            home.join(".conda/bin/conda"),
        ];

        #[cfg(target_os = "windows")]
        let home_paths = [
            home.join("miniconda3\\Scripts\\conda.exe"),
            home.join("anaconda3\\Scripts\\conda.exe"),
            home.join("miniconda3\\condabin\\conda.bat"),
            home.join("anaconda3\\condabin\\conda.bat"),
        ];

        for path in &home_paths {
            if path.exists() {
                return true;
            }
        }
    }

    // Check Windows system-wide paths
    #[cfg(target_os = "windows")]
    {
        let system_paths = [
            "C:\\ProgramData\\miniconda3\\Scripts\\conda.exe",
            "C:\\ProgramData\\anaconda3\\Scripts\\conda.exe",
        ];
        for path in &system_paths {
            if PathBuf::from(path).exists() {
                return true;
            }
        }
    }

    // Fall back to which/where
    check_command_exists("conda")
}

/// Check if git is installed
fn check_git_installed() -> bool {
    #[cfg(target_os = "windows")]
    {
        for path in GIT_KNOWN_PATHS_WINDOWS {
            if PathBuf::from(path).exists() {
                return true;
            }
        }
    }
    check_command_exists("git")
}

/// Check if node is installed
fn check_node_installed() -> bool {
    #[cfg(not(target_os = "windows"))]
    {
        for path in NODE_KNOWN_PATHS_UNIX {
            if PathBuf::from(path).exists() {
                return true;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for path in NODE_KNOWN_PATHS_WINDOWS {
            if PathBuf::from(path).exists() {
                return true;
            }
        }
    }

    check_command_exists("node")
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
        if let Ok(output) = Command::new("which").arg("ollama").output() {
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
        if let Ok(output) = Command::new("where").arg("ollama").output() {
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

    let conda_installed = check_conda_installed();
    let git_installed = check_git_installed();
    let node_installed = check_node_installed();

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
