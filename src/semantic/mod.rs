//! Semantic storage layer.
//!
//! Provides SQLite persistence, FTS5 full-text search, and HNSW vector
//! indexing via `usearch`. The [`SemanticStore`] in [`store`] is the main
//! entry point; [`db`] contains the low-level schema and SQL.

pub mod db;
pub mod store;

/// A stored fact with its embedding vector.
#[derive(Debug, Clone)]
pub struct SemanticFact {
    pub id: String,
    pub namespace: String,
    pub content: String,
    /// Unix epoch seconds.
    pub created_at: i64,
    pub embedding: Vec<f32>,
    /// Optional smem URN identifying where this fact came from.
    pub source: Option<String>,
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
