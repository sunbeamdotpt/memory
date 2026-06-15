use crate::config::MemoryConfig;
use crate::embedding::service::{EmbeddingModelType, EmbeddingService};
use crate::error::{Result, ServerError};
use crate::semantic::SemanticConfig;
use crate::semantic::store::SemanticStore;
use chrono::TimeZone;
use std::sync::Arc;

#[derive(Clone)]
pub struct MemoryService {
    store: Arc<SemanticStore>,
    embedding_service: Arc<tokio::sync::Mutex<EmbeddingService>>,
}

#[derive(Debug, Clone)]
pub struct MemoryFact {
    pub id: String,
    pub namespace: String,
    pub content: String,
    pub created_at: String,     // RFC 3339
    pub score: f32,             // cosine similarity; 0.0 when not from a search
    pub source: Option<String>, // smem URN identifying where this fact came from
}

impl MemoryService {
    pub async fn new(config: &MemoryConfig) -> Result<Self> {
        let model_type = EmbeddingModelType::BgeBaseEnglish;
        let store = SemanticStore::new(&SemanticConfig {
            base_dir: config.base_dir.clone(),
            dimension: model_type.dimensions(),
            model_name: model_type.model_name().to_string(),
        })
        .await?;
        let svc = EmbeddingService::new(model_type).await?;
        Ok(Self {
            store: Arc::new(store),
            embedding_service: Arc::new(tokio::sync::Mutex::new(svc)),
        })
    }

    pub async fn new_with_model(
        config: &MemoryConfig,
        model_type: EmbeddingModelType,
    ) -> Result<Self> {
        let store = SemanticStore::new(&SemanticConfig {
            base_dir: config.base_dir.clone(),
            dimension: model_type.dimensions(),
            model_name: model_type.model_name().to_string(),
        })
        .await?;
        let svc = EmbeddingService::new(model_type).await?;
        Ok(Self {
            store: Arc::new(store),
            embedding_service: Arc::new(tokio::sync::Mutex::new(svc)),
        })
    }

    pub fn get_store(&self) -> Arc<SemanticStore> {
        Arc::clone(&self.store)
    }

    pub fn embedding_service(&self) -> Arc<tokio::sync::Mutex<EmbeddingService>> {
        Arc::clone(&self.embedding_service)
    }

    pub async fn current_model(&self) -> EmbeddingModelType {
        self.embedding_service.lock().await.current_model()
    }

    /// Embed and store content. Returns the created fact.
    pub async fn add_fact(
        &self,
        namespace: &str,
        content: &str,
        source: Option<&str>,
    ) -> Result<MemoryFact> {
        let svc = self.embedding_service.lock().await.clone();
        let embeddings = svc.embed(&[content]).await?;
        let (fact_id, created_at_ts) = self
            .store
            .add_fact(namespace, content, &embeddings[0], source)
            .await?;
        Ok(MemoryFact {
            id: fact_id,
            namespace: namespace.to_string(),
            content: content.to_string(),
            created_at: ts_to_rfc3339(created_at_ts)?,
            score: 0.0,
            source: source.map(|s| s.to_string()),
        })
    }

    /// Fused BM25 + vector search via RRF. Optional `namespace` restricts results.
    ///
    /// The returned `score` is the RRF score (higher is better), not cosine similarity.
    pub async fn search_facts(
        &self,
        query: &str,
        limit: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<MemoryFact>> {
        let svc = self.embedding_service.lock().await.clone();
        let embeddings = svc.embed(&[query]).await?;
        let results = self
            .store
            .fused_search(query, &embeddings[0], limit, namespace)
            .await?;
        let mut out = Vec::new();
        for (fact, score) in results {
            out.push(MemoryFact {
                id: fact.id,
                namespace: fact.namespace,
                content: fact.content,
                created_at: ts_to_rfc3339(fact.created_at)?,
                score,
                source: fact.source,
            });
        }
        Ok(out)
    }

    /// Update an existing fact in place. Returns false if the ID doesn't exist.
    pub async fn update_fact(
        &self,
        fact_id: &str,
        content: &str,
        source: Option<&str>,
    ) -> Result<MemoryFact> {
        let existing = self
            .store
            .get_fact(fact_id)
            .await?
            .ok_or_else(|| ServerError::NotFound(fact_id.to_string()))?;
        let svc = self.embedding_service.lock().await.clone();
        let embeddings = svc.embed(&[content]).await?;
        self.store
            .update_fact(fact_id, content, &embeddings[0], source)
            .await?;
        Ok(MemoryFact {
            id: fact_id.to_string(),
            namespace: existing.namespace,
            content: content.to_string(),
            created_at: ts_to_rfc3339(existing.created_at)?,
            score: 0.0,
            source: source.map(|s| s.to_string()),
        })
    }

    /// Delete a fact. Returns true if it existed.
    pub async fn delete_fact(&self, fact_id: &str) -> Result<bool> {
        self.store.delete_fact(fact_id).await
    }

    /// List facts in a namespace, most recent first.
    /// Optional `from`/`to` are RFC 3339 or Unix timestamp strings for date filtering.
    pub async fn list_facts(
        &self,
        namespace: &str,
        limit: usize,
        from_ts: Option<i64>,
        to_ts: Option<i64>,
    ) -> Result<Vec<MemoryFact>> {
        let facts = self
            .store
            .list_facts(namespace, limit, from_ts, to_ts)
            .await?;
        let mut out = Vec::new();
        for fact in facts {
            out.push(MemoryFact {
                id: fact.id,
                namespace: fact.namespace,
                content: fact.content,
                created_at: ts_to_rfc3339(fact.created_at)?,
                score: 0.0,
                source: fact.source,
            });
        }
        Ok(out)
    }

    /// Switch to a different embedding model for future operations.
    /// Re-embeds all existing facts with the new model.
    pub async fn switch_model(&self, new_model: EmbeddingModelType) -> Result<()> {
        let new_svc = EmbeddingService::new(new_model).await?;
        let old_svc = self.embedding_service.lock().await.clone();

        // Re-embed all facts with the new model
        let store = Arc::clone(&self.store);
        let new_dimension = new_model.dimensions();

        store
            .rebuild_vectors(new_dimension, |texts| {
                old_svc
                    .blocking_embed(texts)
                    .map_err(|e| ServerError::MemoryError(e.to_string()))
            })
            .await?;

        *self.embedding_service.lock().await = new_svc;
        Ok(())
    }

    // ── Error logging ─────────────────────────────────────────────────────────

    pub fn log_error(
        &self,
        id: &str,
        component: &str,
        severity: &str,
        message: &str,
        details: Option<&str>,
    ) -> Result<()> {
        let store = self.store.db();
        let db = store.lock().unwrap();
        db.log_error(id, component, severity, message, details)?;
        Ok(())
    }

    pub async fn get_recent_errors(
        &self,
        component: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, i64, String, String, String, Option<String>)>> {
        let store = self.store.db();
        let db = store.lock().unwrap();
        db.get_recent_errors(component, limit)
    }

    pub async fn resolve_error(&self, error_id: &str) -> Result<bool> {
        let store = self.store.db();
        let db = store.lock().unwrap();
        db.resolve_error(error_id)
    }
}

fn ts_to_rfc3339(ts: i64) -> Result<String> {
    chrono::Utc
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.to_rfc3339())
        .ok_or_else(|| ServerError::InvalidArgument("timestamp out of range".to_string()))
}
