// Embedding Service — wraps fastembed with a process-wide model cache so each
// model is loaded from disk exactly once regardless of how many EmbeddingService
// instances are created.

use fastembed::{EmbeddingModel, TextEmbedding, InitOptions};
use thiserror::Error;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use once_cell::sync::Lazy;

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
            EmbeddingModelType::CodeBert => 768,    // AllMpnetBaseV2
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
        Self { models: HashMap::new() }
    }

    fn get_or_load(&mut self, model_type: EmbeddingModelType) -> Result<CachedModel, EmbeddingError> {
        if let Some(model) = self.models.get(&model_type) {
            return Ok(Arc::clone(model));
        }

        let text_embedding = TextEmbedding::try_new(
            InitOptions::new(model_type.to_fastembed_model())
        ).map_err(|e| EmbeddingError::LoadError(e.to_string()))?;

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
}

impl EmbeddingService {
    /// Obtain a service backed by the globally cached model. Loading from disk
    /// only happens the first time a given model type is requested.
    pub async fn new(model_type: EmbeddingModelType) -> Result<Self, EmbeddingError> {
        let model = MODEL_CACHE.lock().unwrap().get_or_load(model_type)?;
        Ok(Self { model, model_type })
    }

    /// Generate embeddings using the cached model.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
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
