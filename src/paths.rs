use std::path::PathBuf;

/// Platform-appropriate cache directory for sunbeam.
/// Falls back to current working directory if platform dirs are unavailable.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("sunbeam")
}

/// Platform-appropriate data directory for sunbeam.
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("sunbeam")
}

/// Where fastembed ONNX models are cached.
pub fn model_cache_dir() -> PathBuf {
    cache_dir().join("fastembed")
}
