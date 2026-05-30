// Main semantic module
// Re-exports semantic functionality

pub mod db;
pub mod store;
// index.rs removed — replaced by persistent sqlite-vec indexing
// search.rs removed — SemanticSearch was dead code, functionality lives in store.rs

/// Semantic fact storage
#[derive(Debug, Clone)]
pub struct SemanticFact {
    pub id: String,
    pub namespace: String,
    pub content: String,
    pub created_at: i64, // Unix timestamp
    pub embedding: Vec<f32>,
    pub source: Option<String>, // smem URN identifying where this fact came from
}

#[derive(Debug, Clone)]
pub struct SemanticConfig {
    pub base_dir: String,
    pub dimension: usize,
    pub model_name: String,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            base_dir: crate::paths::data_dir()
                .join("semantic")
                .to_string_lossy()
                .to_string(),
            dimension: 768,
            model_name: "bge-base-en-v1.5".to_string(),
        }
    }
}

pub use store::SemanticStore;
