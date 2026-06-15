//! Protocol-agnostic core business logic.
//!
//! CoreService sits between the transport layers (MCP, ConnectRPC)
//! and the persistence layers (MemoryService, IndexService).
//! It handles validation, orchestration, and structured returns.
//! Transport-specific formatting (MCP text, protobuf, HTTP JSON) lives in
//! the respective adapters.

use crate::error::{Result, ServerError};
use crate::indexer::{IndexService, IngestionTarget, TargetProgress};
use crate::memory::service::{MemoryFact, MemoryService};
use crate::urn::SourceUrn;

/// Protocol-agnostic orchestration layer.
///
/// Holds [`MemoryService`] and [`IndexService`] and exposes validated,
/// structured operations that both MCP and ConnectRPC adapters delegate to.
#[derive(Clone)]
pub struct CoreService {
    memory: MemoryService,
    indexer: IndexService,
}

impl CoreService {
    /// Create a new `CoreService` from its constituent services.
    pub fn new(memory: MemoryService, indexer: IndexService) -> Self {
        Self { memory, indexer }
    }

    /// Access the underlying memory service.
    pub fn memory(&self) -> &MemoryService {
        &self.memory
    }

    /// Access the underlying indexer service.
    pub fn indexer(&self) -> &IndexService {
        &self.indexer
    }

    // ── Facts ──────────────────────────────────────────────────────────────────

    /// Embed and store a fact. Returns the created fact including its ULID.
    pub async fn store_fact(
        &self,
        content: &str,
        namespace: Option<&str>,
        source: Option<&str>,
    ) -> Result<MemoryFact> {
        if content.trim().is_empty() {
            return Err(ServerError::InvalidArgument(
                "`content` is required and must not be empty".to_string(),
            ));
        }
        let ns = namespace.unwrap_or("default");
        if let Some(s) = source {
            SourceUrn::parse(s)
                .map_err(|e| ServerError::InvalidArgument(format!("Invalid source URN: {e}")))?;
        }
        self.memory.add_fact(ns, content, source).await
    }

    /// Semantic search by meaning (cosine similarity + BM25 fusion).
    pub async fn search_facts(
        &self,
        query: &str,
        limit: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<MemoryFact>> {
        if query.trim().is_empty() {
            return Err(ServerError::InvalidArgument(
                "`query` is required and must not be empty".to_string(),
            ));
        }
        self.memory.search_facts(query, limit, namespace).await
    }

    /// Update an existing fact, re-embedding the new content.
    pub async fn update_fact(
        &self,
        id: &str,
        content: &str,
        source: Option<&str>,
    ) -> Result<MemoryFact> {
        if id.trim().is_empty() {
            return Err(ServerError::InvalidArgument("`id` is required".to_string()));
        }
        if content.trim().is_empty() {
            return Err(ServerError::InvalidArgument(
                "`content` is required and must not be empty".to_string(),
            ));
        }
        if let Some(s) = source {
            SourceUrn::parse(s)
                .map_err(|e| ServerError::InvalidArgument(format!("Invalid source URN: {e}")))?;
        }
        self.memory.update_fact(id, content, source).await
    }

    /// Delete a fact by ID. Returns `true` if it existed.
    pub async fn delete_fact(&self, id: &str) -> Result<bool> {
        if id.trim().is_empty() {
            return Err(ServerError::InvalidArgument("`id` is required".to_string()));
        }
        self.memory.delete_fact(id).await
    }

    /// List facts in a namespace, most recent first.
    pub async fn list_facts(
        &self,
        namespace: &str,
        limit: usize,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<MemoryFact>> {
        let from_ts = from.and_then(parse_ts);
        let to_ts = to.and_then(parse_ts);
        self.memory
            .list_facts(namespace, limit, from_ts, to_ts)
            .await
    }

    // ── Indexer ────────────────────────────────────────────────────────────────

    /// Add a filesystem path to the watch list and trigger an initial sync.
    pub async fn add_watch_target(
        &self,
        path: &str,
        namespace: Option<&str>,
        target_type: Option<&str>,
    ) -> Result<Vec<String>> {
        if path.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`path` is required".to_string(),
            ));
        }
        let ids = self
            .indexer
            .add_target(path, namespace, target_type)
            .await?;
        // Spawn initial sync in background
        let indexer = self.indexer.clone();
        let ids_clone = ids.clone();
        tokio::spawn(async move {
            for id in &ids_clone {
                if let Err(e) = indexer.sync_target(id).await {
                    let err_id = ulid::Ulid::new().to_string();
                    let _ = indexer.memory().log_error(
                        &err_id,
                        "indexer",
                        "error",
                        &format!("initial sync failed for target {id}"),
                        Some(&e.to_string()),
                    );
                }
            }
        });
        Ok(ids)
    }

    /// Remove a watch target by ID.
    pub async fn remove_watch_target(&self, target_id: &str) -> Result<bool> {
        if target_id.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`target_id` is required".to_string(),
            ));
        }
        self.indexer.remove_target(target_id).await
    }

    /// List all active watch targets.
    pub async fn list_watch_targets(&self) -> Result<Vec<IngestionTarget>> {
        self.indexer.list_targets().await
    }

    /// Trigger an asynchronous sync of a watch target.
    pub fn sync_watch_target(&self, target_id: &str) -> Result<()> {
        if target_id.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`target_id` is required".to_string(),
            ));
        }
        let indexer = self.indexer.clone();
        let id = target_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = indexer.sync_target(&id).await {
                let err_id = ulid::Ulid::new().to_string();
                let _ = indexer.memory().log_error(
                    &err_id,
                    "indexer",
                    "error",
                    &format!("sync failed for target {id}"),
                    Some(&e.to_string()),
                );
            }
        });
        Ok(())
    }

    /// Get ingestion progress for a specific target, if available.
    pub fn get_index_progress(&self, target_id: &str) -> Option<TargetProgress> {
        self.indexer.progress().get(target_id)
    }

    /// Restore a soft-deleted (stale) fact.
    pub fn restore_stale_fact(&self, id: &str) -> Result<bool> {
        if id.is_empty() {
            return Err(ServerError::InvalidArgument("`id` is required".to_string()));
        }
        let store = self.memory.get_store();
        let db = store.db();
        let db_guard = db.lock().unwrap();
        db_guard.restore_fact(id)
    }

    // ── URN ────────────────────────────────────────────────────────────────────

    /// Build a valid smem URN from its components.
    pub fn build_source_urn(
        &self,
        content_type: &str,
        origin: &str,
        locator: &str,
        fragment: Option<&str>,
    ) -> Result<String> {
        if content_type.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`content_type` is required".to_string(),
            ));
        }
        if origin.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`origin` is required".to_string(),
            ));
        }
        if locator.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`locator` is required".to_string(),
            ));
        }
        SourceUrn::build(content_type, origin, locator, fragment)
            .map_err(|e| ServerError::InvalidArgument(e.to_string()))
    }

    /// Parse a smem URN and return structured JSON describing its components.
    pub fn parse_source_urn(&self, urn: &str) -> Result<serde_json::Value> {
        if urn.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`urn` is required".to_string(),
            ));
        }
        let result = match SourceUrn::parse(urn) {
            Ok(u) => u.describe(),
            Err(e) => crate::urn::invalid_urn_response(urn, &e),
        };
        Ok(result)
    }

    /// Return the full machine-readable URN taxonomy as JSON.
    pub fn describe_urn_schema(&self) -> Result<serde_json::Value> {
        Ok(crate::urn::schema_json())
    }

    // ── Observability ──────────────────────────────────────────────────────────

    /// Retrieve recent error log entries.
    pub async fn get_recent_errors(
        &self,
        component: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ErrorEntry>> {
        let rows = self.memory.get_recent_errors(component, limit).await?;
        Ok(rows
            .into_iter()
            .map(|(id, ts, comp, sev, msg, details)| ErrorEntry {
                error_id: id,
                timestamp: ts,
                component: comp,
                severity: sev,
                message: msg,
                details,
            })
            .collect())
    }

    /// Mark an error log entry as resolved.
    pub async fn resolve_error(&self, error_id: &str) -> Result<bool> {
        if error_id.is_empty() {
            return Err(ServerError::InvalidArgument(
                "`error_id` is required".to_string(),
            ));
        }
        self.memory.resolve_error(error_id).await
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Parse a date string: RFC 3339 or raw Unix timestamp integer.
pub fn parse_ts(s: &str) -> Option<i64> {
    if let Ok(ts) = s.parse::<i64>() {
        return Some(ts);
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// A single error log entry.
#[derive(Debug, Clone)]
pub struct ErrorEntry {
    pub error_id: String,
    pub timestamp: i64,
    pub component: String,
    pub severity: String,
    pub message: String,
    pub details: Option<String>,
}
