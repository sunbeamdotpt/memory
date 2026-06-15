// Logging module for MCP Server
use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Simple file logger that holds an open file handle.
#[derive(Clone)]
pub struct FileLogger {
    file: Arc<Mutex<std::fs::File>>,
}

impl FileLogger {
    pub fn new(log_path: String) -> Self {
        if let Some(parent) = Path::new(&log_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .expect("failed to open log file");
        Self {
            file: Arc::new(Mutex::new(file)),
        }
    }

    pub fn log(&self, method: &str, path: &str, status: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let log_entry = format!("{} {} {} {}\n", timestamp, method, path, status);

        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(log_entry.as_bytes());
        }
    }
}
