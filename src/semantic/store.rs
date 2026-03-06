// Semantic Store Implementation

use crate::error::Result;
use crate::semantic::{SemanticConfig, SemanticFact};
use std::sync::{Arc, Mutex};

/// Semantic store using SQLite for persistence and an in-memory cosine-similarity index.
pub struct SemanticStore {
    config: SemanticConfig,
    db: Arc<Mutex<super::db::SemanticDB>>,
    index: Arc<Mutex<super::index::SemanticIndex>>,
}

impl SemanticStore {
    pub async fn new(config: &SemanticConfig) -> Result<Self> {
        let db = Arc::new(Mutex::new(super::db::SemanticDB::new(&config.base_dir)?));
        let index = Arc::new(Mutex::new(super::index::SemanticIndex::new(config.dimension)));

        Ok(Self {
            config: config.clone(),
            db,
            index,
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
        self.index.lock().unwrap().add_vector(embedding, &fact_id);

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
        let search_limit = if namespace_filter.is_some() {
            (limit * 10).max(50)
        } else {
            limit
        };

        let similar_ids = self.index.lock().unwrap().search(query_embedding, search_limit);

        let mut results = vec![];
        for (fact_id, score) in similar_ids {
            if results.len() >= limit {
                break;
            }
            if let Some(fact) = self.db.lock().unwrap().get_fact(&fact_id)? {
                if let Some(namespace) = namespace_filter {
                    if fact.namespace == namespace {
                        results.push((fact, score));
                    }
                } else {
                    results.push((fact, score));
                }
            }
        }

        Ok(results)
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
        self.db.lock().unwrap().search_by_namespace(namespace, limit, from_ts, to_ts)
    }

    /// Hybrid search: keyword filter + vector ranking.
    ///
    /// If any facts contain `keyword`, those are ranked by vector similarity and
    /// the top `limit` are returned.  If no facts contain the keyword the search
    /// falls back to a pure vector search over all facts so callers always get
    /// useful results.
    pub async fn hybrid_search(
        &self,
        keyword: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticFact>> {
        let all_facts = self.db.lock().unwrap().get_all_facts()?;
        let keyword_lower = keyword.to_lowercase();

        let keyword_matches: Vec<SemanticFact> = all_facts
            .iter()
            .filter(|f| f.content.to_lowercase().contains(&keyword_lower))
            .cloned()
            .collect();

        // Fall back to all facts when the keyword matches nothing, rather than
        // returning an empty result that is worse than a plain vector search.
        let candidates = if keyword_matches.is_empty() {
            all_facts
        } else {
            keyword_matches
        };

        let mut scored: Vec<(SemanticFact, f32)> = candidates
            .into_iter()
            .map(|fact| {
                let sim = super::index::SemanticIndex::cosine_similarity(
                    query_embedding,
                    &fact.embedding,
                );
                (fact, sim)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(limit).map(|(f, _)| f).collect())
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
        let updated = self.db.lock().unwrap().update_fact(fact_id, content, source, embedding)?;
        if updated {
            self.index.lock().unwrap().add_vector(embedding, fact_id);
        }
        Ok(updated)
    }

    /// Delete a fact from both the database and the vector index.
    pub async fn delete_fact(&self, fact_id: &str) -> Result<bool> {
        let deleted = self.db.lock().unwrap().delete_fact(fact_id)?;
        if deleted {
            self.index.lock().unwrap().remove_vector(fact_id);
        }
        Ok(deleted)
    }

    /// Get a fact by ID.
    pub async fn get_fact(&self, fact_id: &str) -> Result<Option<SemanticFact>> {
        self.db.lock().unwrap().get_fact(fact_id)
    }

    pub fn get_base_dir(&self) -> String {
        self.config.base_dir.clone()
    }
}
