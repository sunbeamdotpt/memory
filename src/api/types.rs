// API Request/Response Types
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct FactRequest {
    pub namespace: Option<String>,
    pub content: String,
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub limit: Option<usize>,
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub limit: Option<usize>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FactResponse {
    pub id: String,
    pub namespace: String,
    pub content: String,
    pub created_at: String,
    pub score: Option<f32>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<FactResponse>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}
