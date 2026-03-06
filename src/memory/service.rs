use crate::config::MemoryConfig;
use crate::error::Result;
use crate::semantic::store::SemanticStore;
use crate::embedding::service::{EmbeddingService, EmbeddingModelType};
use std::sync::Arc;
use chrono::TimeZone;

#[derive(Clone)]
pub struct MemoryService {
    store: Arc<SemanticStore>,
    embedding_service: Arc<std::sync::Mutex<EmbeddingService>>,
}

#[derive(Debug, Clone)]
pub struct MemoryFact {
    pub id: String,
    pub namespace: String,
    pub content: String,
    pub created_at: String,      // RFC 3339
    pub score: f32,              // cosine similarity; 0.0 when not from a search
    pub source: Option<String>,  // smem URN identifying where this fact came from
}

impl MemoryService {
    pub async fn new(config: &MemoryConfig) -> Result<Self> {
        let store = SemanticStore::new(&crate::semantic::SemanticConfig {
            base_dir: config.base_dir.clone(),
            dimension: 768,
            model_name: "bge-base-en-v1.5".to_string(),
        }).await?;
        let svc = EmbeddingService::new(EmbeddingModelType::BgeBaseEnglish).await?;
        Ok(Self {
            store: Arc::new(store),
            embedding_service: Arc::new(std::sync::Mutex::new(svc)),
        })
    }

    pub async fn new_with_model(config: &MemoryConfig, model_type: EmbeddingModelType) -> Result<Self> {
        let store = SemanticStore::new(&crate::semantic::SemanticConfig {
            base_dir: config.base_dir.clone(),
            dimension: 768,
            model_name: "bge-base-en-v1.5".to_string(),
        }).await?;
        let svc = EmbeddingService::new(model_type).await?;
        Ok(Self {
            store: Arc::new(store),
            embedding_service: Arc::new(std::sync::Mutex::new(svc)),
        })
    }

    pub fn get_store(&self) -> Arc<SemanticStore> {
        Arc::clone(&self.store)
    }

    pub fn current_model(&self) -> EmbeddingModelType {
        self.embedding_service.lock().unwrap().current_model()
    }

    /// Embed and store content. Returns the created fact.
    pub async fn add_fact(&self, namespace: &str, content: &str, source: Option<&str>) -> Result<MemoryFact> {
        let svc = self.embedding_service.lock().unwrap().clone();
        let embeddings = svc.embed(&[content]).await?;
        let (fact_id, created_at_ts) = self.store.add_fact(namespace, content, &embeddings[0], source).await?;
        Ok(MemoryFact {
            id: fact_id,
            namespace: namespace.to_string(),
            content: content.to_string(),
            created_at: ts_to_rfc3339(created_at_ts),
            score: 0.0,
            source: source.map(|s| s.to_string()),
        })
    }

    /// Semantic search. Optional `namespace` restricts results to one namespace.
    pub async fn search_facts(
        &self,
        query: &str,
        limit: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<MemoryFact>> {
        let svc = self.embedding_service.lock().unwrap().clone();
        let embeddings = svc.embed(&[query]).await?;
        let results = self.store.search(&embeddings[0], limit, namespace).await?;
        Ok(results.into_iter().map(|(fact, score)| MemoryFact {
            id: fact.id,
            namespace: fact.namespace,
            content: fact.content,
            created_at: ts_to_rfc3339(fact.created_at),
            score,
            source: fact.source,
        }).collect())
    }

    /// Update an existing fact in place. Returns false if the ID doesn't exist.
    pub async fn update_fact(&self, fact_id: &str, content: &str, source: Option<&str>) -> Result<MemoryFact> {
        let existing = self.store.get_fact(fact_id).await?
            .ok_or_else(|| crate::error::ServerError::NotFound(fact_id.to_string()))?;
        let svc = self.embedding_service.lock().unwrap().clone();
        let embeddings = svc.embed(&[content]).await?;
        self.store.update_fact(fact_id, content, &embeddings[0], source).await?;
        Ok(MemoryFact {
            id: fact_id.to_string(),
            namespace: existing.namespace,
            content: content.to_string(),
            created_at: ts_to_rfc3339(existing.created_at),
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
        let facts = self.store.list_facts(namespace, limit, from_ts, to_ts).await?;
        Ok(facts.into_iter().map(|fact| MemoryFact {
            id: fact.id,
            namespace: fact.namespace,
            content: fact.content,
            created_at: ts_to_rfc3339(fact.created_at),
            score: 0.0,
            source: fact.source,
        }).collect())
    }

    /// Switch to a different embedding model for future operations.
    pub async fn switch_model(&self, new_model: EmbeddingModelType) -> Result<()> {
        let new_svc = EmbeddingService::new(new_model).await?;
        *self.embedding_service.lock().unwrap() = new_svc;
        Ok(())
    }
}

fn ts_to_rfc3339(ts: i64) -> String {
    chrono::Utc
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}
