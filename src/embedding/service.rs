// Embedding Service — wraps fastembed with a process-wide model cache so each
// model is loaded from disk exactly once regardless of how many EmbeddingService
// instances are created.

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EmbeddingError {
    #[error("Model {0} not supported")]
    UnsupportedModel(String),

    #[error("Failed to load model: {0}")]
    LoadError(String),

    #[error("Embedding generation failed: {0}")]
    GenerationError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmbeddingModelType {
    BgeBaseEnglish,
    CodeBert,
    GraphCodeBert,
}

impl EmbeddingModelType {
    pub fn model_name(&self) -> &'static str {
        match self {
            EmbeddingModelType::BgeBaseEnglish => "bge-base-en-v1.5",
            EmbeddingModelType::CodeBert => "codebert",
            EmbeddingModelType::GraphCodeBert => "graphcodebert",
        }
    }

    pub fn dimensions(&self) -> usize {
        match self {
            EmbeddingModelType::BgeBaseEnglish => 768,
            EmbeddingModelType::CodeBert => 768, // AllMpnetBaseV2
            EmbeddingModelType::GraphCodeBert => 768, // NomicEmbedTextV1
        }
    }

    pub fn to_fastembed_model(&self) -> EmbeddingModel {
        match self {
            EmbeddingModelType::BgeBaseEnglish => EmbeddingModel::BGEBaseENV15,
            EmbeddingModelType::CodeBert => EmbeddingModel::AllMpnetBaseV2,
            EmbeddingModelType::GraphCodeBert => EmbeddingModel::NomicEmbedTextV1,
        }
    }
}

// ── Bounded in-memory cache for embeddings ────────────────────────────────────
// Prevents unbounded growth while still avoiding repeated inference for
// frequently seen texts. Eviction is FIFO once the capacity is reached.

struct BoundedCache<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
    capacity: usize,
}

impl<K: Eq + std::hash::Hash + Clone, V> BoundedCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn get<Q: Eq + std::hash::Hash + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q>,
    {
        self.map.get(key)
    }

    fn contains_key<Q: Eq + std::hash::Hash + ?Sized>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q>,
    {
        self.map.contains_key(key)
    }

    #[allow(clippy::map_entry)]
    fn insert(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            self.map.insert(key, value);
            return;
        }
        if self.order.len() >= self.capacity
            && let Some(old) = self.order.pop_front()
        {
            self.map.remove(&old);
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}

// ── Global model cache ────────────────────────────────────────────────────────
// Each model is wrapped in Arc<Mutex<TextEmbedding>> so that:
//   - Arc  → the same TextEmbedding allocation is shared across all
//            EmbeddingService instances that use the same model.
//   - Mutex → TextEmbedding::embed takes &mut self so we need exclusive access.
//             The lock is held only for the duration of the embed call, which
//             is CPU-bound and returns quickly.

type CachedModel = Arc<Mutex<TextEmbedding>>;

struct ModelCache {
    models: HashMap<EmbeddingModelType, CachedModel>,
}

impl ModelCache {
    fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    fn get_or_load(
        &mut self,
        model_type: EmbeddingModelType,
    ) -> Result<CachedModel, EmbeddingError> {
        if let Some(model) = self.models.get(&model_type) {
            return Ok(Arc::clone(model));
        }

        let cache_dir = crate::paths::model_cache_dir();
        std::fs::create_dir_all(&cache_dir).ok();
        let text_embedding = TextEmbedding::try_new(
            InitOptions::new(model_type.to_fastembed_model()).with_cache_dir(cache_dir),
        )
        .map_err(|e| EmbeddingError::LoadError(e.to_string()))?;

        let model = Arc::new(Mutex::new(text_embedding));
        self.models.insert(model_type, Arc::clone(&model));
        Ok(model)
    }
}

static MODEL_CACHE: Lazy<Mutex<ModelCache>> = Lazy::new(|| Mutex::new(ModelCache::new()));

// ── EmbeddingService ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct EmbeddingService {
    model: CachedModel,
    model_type: EmbeddingModelType,
    query_cache: Arc<Mutex<BoundedCache<String, Vec<f32>>>>,
}

impl EmbeddingService {
    /// Default maximum number of unique texts kept in the per-service embedding cache.
    const DEFAULT_CACHE_CAPACITY: usize = 1024;

    /// Obtain a service backed by the globally cached model. Loading from disk
    /// only happens the first time a given model type is requested.
    pub async fn new(model_type: EmbeddingModelType) -> Result<Self, EmbeddingError> {
        let model = MODEL_CACHE.lock().unwrap().get_or_load(model_type)?;
        Ok(Self {
            model,
            model_type,
            query_cache: Arc::new(Mutex::new(BoundedCache::new(Self::DEFAULT_CACHE_CAPACITY))),
        })
    }

    /// Generate embeddings using the cached model (runs in spawn_blocking).
    /// Identical texts are served from an in-memory cache to guarantee
    /// deterministic results across repeated calls.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        // Fast path: all texts cached
        {
            let cache = self.query_cache.lock().unwrap();
            if texts.iter().all(|t| cache.contains_key(*t)) {
                return Ok(texts
                    .iter()
                    .map(|t| cache.get(*t).unwrap().clone())
                    .collect());
            }
        }

        let model = self.model.clone();
        let texts: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let texts_for_cache = texts.clone();
        let embeddings = tokio::task::spawn_blocking(move || {
            let texts_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            model
                .lock()
                .unwrap()
                .embed(&texts_refs, None)
                .map_err(|e| EmbeddingError::GenerationError(e.to_string()))
        })
        .await
        .map_err(|e| EmbeddingError::GenerationError(format!("spawn_blocking failed: {e}")))??;

        // Populate cache
        {
            let mut cache = self.query_cache.lock().unwrap();
            for (text, embedding) in texts_for_cache.iter().zip(embeddings.iter()) {
                cache.insert(text.clone(), embedding.clone());
            }
        }

        Ok(embeddings)
    }

    /// Synchronous embed for use inside spawn_blocking.
    pub fn blocking_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.model
            .lock()
            .unwrap()
            .embed(texts, None)
            .map_err(|e| EmbeddingError::GenerationError(e.to_string()))
    }

    pub fn current_model(&self) -> EmbeddingModelType {
        self.model_type
    }

    pub fn dimensions(&self) -> usize {
        self.model_type.dimensions()
    }
}
