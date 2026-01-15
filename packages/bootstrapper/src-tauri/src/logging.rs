//! Centralized logging system for the BrainDrive Installer
//!
//! Features:
//! - Structured JSON logging with timestamps
//! - Automatic log rotation (keeps last 7 days)
//! - Secret redaction (API keys, passwords, tokens)
//! - Export functionality for sharing logs with support

use regex::Regex;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Global regex patterns for secret redaction
static SECRET_PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

/// Get the log directory path
pub fn get_log_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".braindrive-installer")
        .join("logs")
}

/// Initialize the logging system
/// Should be called once at application startup
pub fn init_logging() -> Result<(), String> {
    let log_dir = get_log_dir();

    // Create log directory if it doesn't exist
    fs::create_dir_all(&log_dir)
        .map_err(|e| format!("Failed to create log directory: {}", e))?;

    // Create a rolling file appender (rotates daily, keeps files with date suffix)
    let file_appender = RollingFileAppender::new(Rotation::DAILY, &log_dir, "installer.log");

    // Build the subscriber with both console and file output
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(true)
                .with_line_number(true)
                .json()
                .with_writer(file_appender),
        );

    // Set as the global default
    subscriber
        .try_init()
        .map_err(|e| format!("Failed to initialize logging: {}", e))?;

    // Initialize secret patterns
    init_secret_patterns();

    tracing::info!(
        log_dir = %log_dir.display(),
        "Logging system initialized"
    );

    Ok(())
}

/// Initialize the secret redaction patterns
fn init_secret_patterns() {
    SECRET_PATTERNS.get_or_init(|| {
        vec![
            // API keys (various formats)
            (
                Regex::new(r#"(?i)(api[_-]?key|apikey)[=:\s]+['"]?([a-zA-Z0-9_-]{20,})['"]?"#)
                    .unwrap(),
                "$1=[REDACTED]",
            ),
            // Anthropic API keys
            (
                Regex::new(r"sk-ant-[a-zA-Z0-9_-]{20,}").unwrap(),
                "[REDACTED_ANTHROPIC_KEY]",
            ),
            // OpenAI API keys
            (
                Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(),
                "[REDACTED_OPENAI_KEY]",
            ),
            // Generic secrets/tokens
            (
                Regex::new(r#"(?i)(secret|token|password|passwd|pwd)[=:\s]+['"]?([^\s'"]{8,})['"]?"#)
                    .unwrap(),
                "$1=[REDACTED]",
            ),
            // Bearer tokens
            (
                Regex::new(r"(?i)bearer\s+[a-zA-Z0-9_.-]{20,}").unwrap(),
                "Bearer [REDACTED]",
            ),
            // Authorization headers
            (
                Regex::new(r#"(?i)authorization[=:\s]+['"]?[^\s'"]{20,}['"]?"#).unwrap(),
                "Authorization: [REDACTED]",
            ),
            // Environment variable assignments with sensitive names
            (
                Regex::new(
                    r"(?i)(ANTHROPIC_API_KEY|OPENAI_API_KEY|DATABASE_URL|SECRET_KEY|PRIVATE_KEY)[=][^\s]{8,}",
                )
                .unwrap(),
                "$1=[REDACTED]",
            ),
            // Connection strings
            (
                Regex::new(r"(?i)(mongodb|postgres|mysql|redis)://[^\s]+@[^\s]+").unwrap(),
                "$1://[REDACTED]@[REDACTED]",
            ),
            // SSH private key markers
            (
                Regex::new(r"-----BEGIN[^-]*PRIVATE KEY-----[\s\S]*?-----END[^-]*PRIVATE KEY-----")
                    .unwrap(),
                "[REDACTED_PRIVATE_KEY]",
            ),
        ]
    });
}

/// Redact secrets from a string
pub fn redact_secrets(input: &str) -> String {
    let patterns = SECRET_PATTERNS.get().expect("Secret patterns not initialized");
    let mut result = input.to_string();

    for (pattern, replacement) in patterns {
        result = pattern.replace_all(&result, *replacement).to_string();
    }

    result
}

/// Log an installer event with structured data
#[macro_export]
macro_rules! log_event {
    ($level:ident, $event:expr, $($field:tt)*) => {
        tracing::$level!(
            event = $event,
            $($field)*
        )
    };
}

/// Log levels for convenience
pub use tracing::{debug, error, info, warn};

/// Clean up old log files (keeps last N days)
pub fn cleanup_old_logs(keep_days: u32) -> Result<usize, String> {
    let log_dir = get_log_dir();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(keep_days as i64);
    let mut removed_count = 0;

    let entries = fs::read_dir(&log_dir).map_err(|e| format!("Failed to read log directory: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process .log files
        if path.extension().map_or(false, |ext| ext == "log") {
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    let modified_time: chrono::DateTime<chrono::Utc> = modified.into();
                    if modified_time < cutoff {
                        if fs::remove_file(&path).is_ok() {
                            removed_count += 1;
                            tracing::debug!(
                                path = %path.display(),
                                "Removed old log file"
                            );
                        }
                    }
                }
            }
        }
    }

    if removed_count > 0 {
        tracing::info!(
            removed_count,
            keep_days,
            "Cleaned up old log files"
        );
    }

    Ok(removed_count)
}

/// Export logs for sharing with support
/// Returns the path to the exported file with secrets redacted
pub fn export_logs_for_sharing(lines_limit: Option<usize>) -> Result<PathBuf, String> {
    let log_dir = get_log_dir();
    let export_dir = log_dir.join("exports");

    fs::create_dir_all(&export_dir)
        .map_err(|e| format!("Failed to create export directory: {}", e))?;

    // Find all log files and sort by modification time (newest first)
    let mut log_files: Vec<PathBuf> = fs::read_dir(&log_dir)
        .map_err(|e| format!("Failed to read log directory: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "log"))
        .collect();

    log_files.sort_by(|a, b| {
        let a_time = fs::metadata(a).and_then(|m| m.modified()).ok();
        let b_time = fs::metadata(b).and_then(|m| m.modified()).ok();
        b_time.cmp(&a_time) // Newest first
    });

    if log_files.is_empty() {
        return Err("No log files found".to_string());
    }

    // Create export file with timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let export_path = export_dir.join(format!("braindrive_logs_{}.txt", timestamp));

    let mut export_file =
        fs::File::create(&export_path).map_err(|e| format!("Failed to create export file: {}", e))?;

    // Write header
    writeln!(export_file, "=== BrainDrive Installer Logs (Redacted) ===").ok();
    writeln!(export_file, "Exported: {}", chrono::Local::now().to_rfc3339()).ok();
    writeln!(export_file, "").ok();

    let max_lines = lines_limit.unwrap_or(1000);
    let mut total_lines = 0;

    // Read from newest log file
    for log_file in log_files.iter().take(3) {
        // Read up to 3 most recent log files
        if total_lines >= max_lines {
            break;
        }

        writeln!(export_file, "--- {} ---", log_file.file_name().unwrap_or_default().to_string_lossy()).ok();

        let file = match fs::File::open(log_file) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

        // Take last N lines from this file
        let remaining = max_lines - total_lines;
        let start = lines.len().saturating_sub(remaining);

        for line in lines.into_iter().skip(start) {
            let redacted = redact_secrets(&line);
            writeln!(export_file, "{}", redacted).ok();
            total_lines += 1;

            if total_lines >= max_lines {
                break;
            }
        }

        writeln!(export_file, "").ok();
    }

    writeln!(export_file, "=== End of Export ({} lines) ===", total_lines).ok();

    tracing::info!(
        export_path = %export_path.display(),
        lines = total_lines,
        "Exported logs for sharing"
    );

    Ok(export_path)
}

/// Get a summary of recent log events (for UI display)
pub fn get_recent_events(count: usize) -> Result<Vec<String>, String> {
    let log_dir = get_log_dir();

    // Find the most recent log file
    let mut log_files: Vec<PathBuf> = fs::read_dir(&log_dir)
        .map_err(|e| format!("Failed to read log directory: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "log"))
        .collect();

    log_files.sort_by(|a, b| {
        let a_time = fs::metadata(a).and_then(|m| m.modified()).ok();
        let b_time = fs::metadata(b).and_then(|m| m.modified()).ok();
        b_time.cmp(&a_time)
    });

    let log_file = log_files.first().ok_or("No log files found")?;

    let file = fs::File::open(log_file).map_err(|e| format!("Failed to open log file: {}", e))?;

    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .filter_map(|l| l.ok())
        .map(|l| redact_secrets(&l))
        .collect();

    // Return last N lines
    let start = lines.len().saturating_sub(count);
    Ok(lines.into_iter().skip(start).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_anthropic_key() {
        init_secret_patterns();
        let input = "Using API key: sk-ant-api03-abc123xyz789defghijklmnop";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED_ANTHROPIC_KEY]"));
        assert!(!redacted.contains("sk-ant-"));
    }

    #[test]
    fn test_redact_env_var() {
        init_secret_patterns();
        let input = "ANTHROPIC_API_KEY=sk-ant-secret123456789";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("secret123456789"));
    }

    #[test]
    fn test_redact_password() {
        init_secret_patterns();
        let input = "password=mysecretpassword123";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("mysecretpassword123"));
    }

    #[test]
    fn test_redact_bearer_token() {
        init_secret_patterns();
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.xxxxx";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
    }
}
