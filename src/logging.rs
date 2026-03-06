// Logging module for MCP Server
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use chrono::Local;

/// Simple file logger
#[derive(Clone)]
pub struct FileLogger {
    log_path: String,
}

impl FileLogger {
    pub fn new(log_path: String) -> Self {
        // Create directory if it doesn't exist
        if let Some(parent) = Path::new(&log_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self { log_path }
    }
    
    pub fn log(&self, method: &str, path: &str, status: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let log_entry = format!("{} {} {} {}\n", timestamp, method, path, status);
        
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path) {
            let _ = file.write_all(log_entry.as_bytes());
        }
    }
}