// Semantic Store Implementation

use crate::error::Result;
use crate::semantic::{SemanticConfig, SemanticFact};
use std::sync::{Arc, Mutex};

/// Semantic store using SQLite for persistence and sqlite-vec for vector search.
pub struct SemanticStore {
    config: SemanticConfig,
    db: Arc<Mutex<super::db::SemanticDB>>,
}

impl SemanticStore {
    pub async fn new(config: &SemanticConfig) -> Result<Self> {
        let db = Arc::new(Mutex::new(super::db::SemanticDB::new(
            &config.base_dir,
            config.dimension,
        )?));
        Ok(Self {
            config: config.clone(),
            db,
        })
    }

    /// Add a fact with its embedding. Returns (fact_id, created_at unix timestamp).
    pub async fn add_fact(
        &self,
        namespace: &str,
        content: &str,
        embedding: &[f32],
        source: Option<&str>,
    ) -> Result<(String, i64)> {
        let fact = SemanticFact {
            id: String::new(),
            namespace: namespace.to_string(),
            content: content.to_string(),
            created_at: 0,
            embedding: embedding.to_vec(),
            source: source.map(|s| s.to_string()),
        };

        let (fact_id, created_at) = self.db.lock().unwrap().add_fact(&fact)?;
        Ok((fact_id, created_at))
    }

    /// Vector similarity search. Returns facts paired with their cosine similarity
    /// score (0–1, higher is better). When a namespace filter is supplied,
    /// oversamples from the index to ensure `limit` results after filtering.
    pub async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<(SemanticFact, f32)>> {
        self.db
            .lock()
            .unwrap()
            .search_similar(query_embedding, limit, namespace_filter)
    }

    /// Return facts in a namespace ordered by creation time, without scoring.
    /// Optional `from_ts`/`to_ts` are Unix timestamps (seconds) for date filtering.
    pub async fn list_facts(
        &self,
        namespace: &str,
        limit: usize,
        from_ts: Option<i64>,
        to_ts: Option<i64>,
    ) -> Result<Vec<SemanticFact>> {
        self.db
            .lock()
            .unwrap()
            .search_by_namespace(namespace, limit, from_ts, to_ts)
    }

    /// Fused BM25 + vector search via Reciprocal Rank Fusion.
    ///
    /// Combines BM25 keyword relevance (via FTS5) with cosine vector similarity
    /// using RRF (k=60). This is the default search path.
    pub async fn fused_search(
        &self,
        query: &str,
        query_embedding: &[f32],
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<(SemanticFact, f32)>> {
        self.db
            .lock()
            .unwrap()
            .fused_search(query, query_embedding, limit, namespace_filter)
    }

    /// Update an existing fact in place. Re-embeds with the new content and
    /// replaces the vector in the index. Returns false if the ID doesn't exist.
    pub async fn update_fact(
        &self,
        fact_id: &str,
        content: &str,
        embedding: &[f32],
        source: Option<&str>,
    ) -> Result<bool> {
        self.db
            .lock()
            .unwrap()
            .update_fact(fact_id, content, source, embedding)
    }

    /// Delete a fact from both the database and the vector index.
    pub async fn delete_fact(&self, fact_id: &str) -> Result<bool> {
        self.db.lock().unwrap().delete_fact(fact_id)
    }

    /// Get a fact by ID.
    pub async fn get_fact(&self, fact_id: &str) -> Result<Option<SemanticFact>> {
        self.db.lock().unwrap().get_fact(fact_id)
    }

    pub fn get_base_dir(&self) -> String {
        self.config.base_dir.clone()
    }

    pub fn dimension(&self) -> usize {
        self.config.dimension
    }

    /// Direct access to the underlying database (for indexer and advanced use).
    pub fn db(&self) -> Arc<Mutex<super::db::SemanticDB>> {
        self.db.clone()
    }

    /// Rebuild the vector table with a new dimension and re-embed all facts.
    /// Returns the number of facts re-embedded.
    pub async fn rebuild_vectors<F>(&self, new_dimension: usize, mut embed_fn: F) -> Result<usize>
    where
        F: FnMut(&[&str]) -> Result<Vec<Vec<f32>>>,
    {
        let facts = {
            let mut db = self.db.lock().unwrap();
            db.recreate_vec_table(new_dimension)?;
            db.get_all_facts(false)?
        };

        let total = facts.len();
        for (i, fact) in facts.iter().enumerate() {
            let embeddings = embed_fn(&[&fact.content])?;
            if let Some(emb) = embeddings.first() {
                let mut db = self.db.lock().unwrap();
                db.insert_vec(&fact.id, emb)?;
            }
            if (i + 1) % 100 == 0 || i + 1 == total {
                let db = self.db.lock().unwrap();
                let err_id = ulid::Ulid::new().to_string();
                let _ = db.log_error(
                    &err_id,
                    "semantic",
                    "warn",
                    &format!("re-embedded {}/{} facts", i + 1, total),
                    None,
                );
            }
        }

        Ok(total)
    }
}
