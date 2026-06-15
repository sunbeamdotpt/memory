//! ConnectRPC implementation of `sunbeam.memory.v1.MemoryService`.
//!
//! Thin adapter: validates inputs, delegates to [`CoreService`], and maps
//! results to buffa-generated request/response views.

use std::sync::Arc;

use connectrpc::{ConnectError, Context, ErrorCode};

use crate::core::service::CoreService;
use crate::error::ServerError;

// Generated proto types
use crate::connect::memory_proto::sunbeam::memory::v1::{
    AddWatchTargetRequestView, AddWatchTargetResponse, BuildSourceUrnRequestView,
    BuildSourceUrnResponse, DeleteFactRequestView, DeleteFactResponse,
    DescribeUrnSchemaRequestView, DescribeUrnSchemaResponse, ErrorEntry, Fact,
    GetIndexProgressRequestView, GetIndexProgressResponse, GetRecentErrorsRequestView,
    GetRecentErrorsResponse, HealthCheckRequestView, HealthCheckResponse, IndexProgress,
    ListFactsRequestView, ListFactsResponse, ListWatchTargetsRequestView, ListWatchTargetsResponse,
    MemoryService, ParseSourceUrnRequestView, ParseSourceUrnResponse, RemoveWatchTargetRequestView,
    RemoveWatchTargetResponse, ResolveErrorRequestView, ResolveErrorResponse,
    RestoreStaleFactRequestView, RestoreStaleFactResponse, SearchFactsRequestView,
    SearchFactsResponse, StoreFactRequestView, StoreFactResponse, SyncWatchTargetRequestView,
    SyncWatchTargetResponse, UpdateFactRequestView, UpdateFactResponse, WatchTarget,
};

/// ConnectRPC adapter for `sunbeam.memory.v1.MemoryService`.
///
/// Implements the generated [`MemoryService`] trait by delegating to
/// [`CoreService`]. All methods validate inputs, call the core, and map
/// [`ServerError`] to [`ConnectError`] with appropriate [`ErrorCode`]s.
#[derive(Clone)]
pub struct MemoryConnectService {
    core: Arc<CoreService>,
}

impl MemoryConnectService {
    /// Wrap a [`CoreService`] in an `Arc` for sharing across requests.
    pub fn new(core: CoreService) -> Self {
        Self {
            core: Arc::new(core),
        }
    }
}

// ── error mapping ─────────────────────────────────────────────────────────────

fn map_error(e: ServerError) -> ConnectError {
    match e {
        ServerError::InvalidArgument(msg) => ConnectError::new(ErrorCode::InvalidArgument, msg),
        ServerError::NotFound(msg) => ConnectError::new(ErrorCode::NotFound, msg),
        _ => ConnectError::new(ErrorCode::Internal, e.to_string()),
    }
}

// ── trait implementation ──────────────────────────────────────────────────────

impl MemoryService for MemoryConnectService {
    async fn store_fact(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<StoreFactRequestView<'static>>,
    ) -> Result<(StoreFactResponse, Context), ConnectError> {
        let namespace = if req.namespace.is_empty() {
            None
        } else {
            Some(req.namespace)
        };
        let source = req.source.map(|s| s.to_string());
        let fact = self
            .core
            .store_fact(req.content, namespace, source.as_deref())
            .await
            .map_err(map_error)?;

        let mut resp = StoreFactResponse::default();
        resp.fact.modify(|f| *f = (&fact).into());
        Ok((resp, ctx))
    }

    async fn search_facts(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<SearchFactsRequestView<'static>>,
    ) -> Result<(SearchFactsResponse, Context), ConnectError> {
        let limit = if req.limit == 0 {
            10
        } else {
            req.limit as usize
        };
        let namespace = req.namespace.map(|s| s.to_string());
        let results = self
            .core
            .search_facts(req.query, limit, namespace.as_deref())
            .await
            .map_err(map_error)?;

        let total = results.len() as u64;
        let facts: Vec<Fact> = results.iter().map(|f| f.into()).collect();

        let mut resp = SearchFactsResponse::default();
        resp.results = facts;
        resp.total = total;
        Ok((resp, ctx))
    }

    async fn update_fact(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<UpdateFactRequestView<'static>>,
    ) -> Result<(UpdateFactResponse, Context), ConnectError> {
        let source = req.source.map(|s| s.to_string());
        let fact = self
            .core
            .update_fact(req.id, req.content, source.as_deref())
            .await
            .map_err(map_error)?;

        let mut resp = UpdateFactResponse::default();
        resp.fact.modify(|f| *f = (&fact).into());
        Ok((resp, ctx))
    }

    async fn delete_fact(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<DeleteFactRequestView<'static>>,
    ) -> Result<(DeleteFactResponse, Context), ConnectError> {
        let deleted = self.core.delete_fact(req.id).await.map_err(map_error)?;

        let mut resp = DeleteFactResponse::default();
        resp.deleted = deleted;
        Ok((resp, ctx))
    }

    async fn list_facts(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<ListFactsRequestView<'static>>,
    ) -> Result<(ListFactsResponse, Context), ConnectError> {
        let namespace = if req.namespace.is_empty() {
            "default"
        } else {
            req.namespace
        };
        let limit = if req.limit == 0 {
            50
        } else {
            req.limit as usize
        };
        let from = req.from.as_deref();
        let to = req.to.as_deref();
        let facts = self
            .core
            .list_facts(namespace, limit, from, to)
            .await
            .map_err(map_error)?;

        let total = facts.len() as u64;
        let items: Vec<Fact> = facts.iter().map(|f| f.into()).collect();

        let mut resp = ListFactsResponse::default();
        resp.facts = items;
        resp.total = total;
        Ok((resp, ctx))
    }

    async fn add_watch_target(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<AddWatchTargetRequestView<'static>>,
    ) -> Result<(AddWatchTargetResponse, Context), ConnectError> {
        let namespace = if req.namespace.is_empty() {
            None
        } else {
            Some(req.namespace)
        };
        let target_type = req.target_type;
        let ids = self
            .core
            .add_watch_target(req.path, namespace, target_type)
            .await
            .map_err(map_error)?;

        let count = ids.len() as u64;
        let mut resp = AddWatchTargetResponse::default();
        resp.target_ids = ids;
        resp.count = count;
        Ok((resp, ctx))
    }

    async fn remove_watch_target(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<RemoveWatchTargetRequestView<'static>>,
    ) -> Result<(RemoveWatchTargetResponse, Context), ConnectError> {
        let removed = self
            .core
            .remove_watch_target(req.target_id)
            .await
            .map_err(map_error)?;

        let mut resp = RemoveWatchTargetResponse::default();
        resp.removed = removed;
        Ok((resp, ctx))
    }

    async fn list_watch_targets(
        &self,
        ctx: Context,
        _req: buffa::view::OwnedView<ListWatchTargetsRequestView<'static>>,
    ) -> Result<(ListWatchTargetsResponse, Context), ConnectError> {
        let targets = self.core.list_watch_targets().await.map_err(map_error)?;

        let total = targets.len() as u64;
        let items: Vec<WatchTarget> = targets.into_iter().map(|t| t.into()).collect();
        let mut resp = ListWatchTargetsResponse::default();
        resp.targets = items;
        resp.total = total;
        Ok((resp, ctx))
    }

    async fn sync_watch_target(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<SyncWatchTargetRequestView<'static>>,
    ) -> Result<(SyncWatchTargetResponse, Context), ConnectError> {
        self.core
            .sync_watch_target(req.target_id)
            .map_err(map_error)?;

        let mut resp = SyncWatchTargetResponse::default();
        resp.started = true;
        Ok((resp, ctx))
    }

    async fn get_index_progress(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<GetIndexProgressRequestView<'static>>,
    ) -> Result<(GetIndexProgressResponse, Context), ConnectError> {
        let mut resp = GetIndexProgressResponse::default();
        if let Some(p) = self.core.get_index_progress(req.target_id) {
            resp.progress = IndexProgress::from(p).into();
        }
        Ok((resp, ctx))
    }

    async fn restore_stale_fact(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<RestoreStaleFactRequestView<'static>>,
    ) -> Result<(RestoreStaleFactResponse, Context), ConnectError> {
        let restored = self.core.restore_stale_fact(req.id).map_err(map_error)?;

        let mut resp = RestoreStaleFactResponse::default();
        resp.restored = restored;
        Ok((resp, ctx))
    }

    async fn build_source_urn(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<BuildSourceUrnRequestView<'static>>,
    ) -> Result<(BuildSourceUrnResponse, Context), ConnectError> {
        let urn = self
            .core
            .build_source_urn(req.content_type, req.origin, req.locator, req.fragment)
            .map_err(map_error)?;

        let mut resp = BuildSourceUrnResponse::default();
        resp.urn = urn;
        Ok((resp, ctx))
    }

    async fn parse_source_urn(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<ParseSourceUrnRequestView<'static>>,
    ) -> Result<(ParseSourceUrnResponse, Context), ConnectError> {
        let json = self.core.parse_source_urn(req.urn).map_err(map_error)?;

        let mut resp = ParseSourceUrnResponse::default();
        resp.valid = json.get("valid").and_then(|v| v.as_bool()).unwrap_or(false);
        resp.content_type = json
            .get("content_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        resp.origin = json
            .get("origin")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        resp.locator = json
            .get("locator")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        resp.fragment = json
            .get("fragment")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        resp.human_readable = json
            .get("human_readable")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        resp.error = json
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok((resp, ctx))
    }

    async fn describe_urn_schema(
        &self,
        ctx: Context,
        _req: buffa::view::OwnedView<DescribeUrnSchemaRequestView<'static>>,
    ) -> Result<(DescribeUrnSchemaResponse, Context), ConnectError> {
        let json = self.core.describe_urn_schema().map_err(map_error)?;

        let mut resp = DescribeUrnSchemaResponse::default();
        resp.schema_json = json.to_string();
        Ok((resp, ctx))
    }

    async fn get_recent_errors(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<GetRecentErrorsRequestView<'static>>,
    ) -> Result<(GetRecentErrorsResponse, Context), ConnectError> {
        let component = req.component;
        let limit = if req.limit == 0 {
            10
        } else {
            req.limit as usize
        };
        let entries = self
            .core
            .get_recent_errors(component, limit)
            .await
            .map_err(map_error)?;

        let items: Vec<ErrorEntry> = entries.into_iter().map(|e| e.into()).collect();
        let mut resp = GetRecentErrorsResponse::default();
        resp.errors = items;
        Ok((resp, ctx))
    }

    async fn resolve_error(
        &self,
        ctx: Context,
        req: buffa::view::OwnedView<ResolveErrorRequestView<'static>>,
    ) -> Result<(ResolveErrorResponse, Context), ConnectError> {
        let resolved = self
            .core
            .resolve_error(req.error_id)
            .await
            .map_err(map_error)?;

        let mut resp = ResolveErrorResponse::default();
        resp.resolved = resolved;
        Ok((resp, ctx))
    }

    async fn health_check(
        &self,
        ctx: Context,
        _req: buffa::view::OwnedView<HealthCheckRequestView<'static>>,
    ) -> Result<(HealthCheckResponse, Context), ConnectError> {
        let mut resp = HealthCheckResponse::default();
        resp.status = "ok".to_string();
        resp.version = env!("CARGO_PKG_VERSION").to_string();
        Ok((resp, ctx))
    }
}

// ── helpers: domain → proto ───────────────────────────────────────────────────

impl From<&crate::memory::service::MemoryFact> for Fact {
    fn from(f: &crate::memory::service::MemoryFact) -> Self {
        Self {
            id: f.id.clone(),
            namespace: f.namespace.clone(),
            content: f.content.clone(),
            created_at: f.created_at.clone(),
            score: f.score,
            source: f.source.clone(),
            __buffa_unknown_fields: buffa::UnknownFields::default(),
            __buffa_cached_size: buffa::__private::CachedSize::default(),
        }
    }
}

impl From<crate::indexer::IngestionTarget> for WatchTarget {
    fn from(t: crate::indexer::IngestionTarget) -> Self {
        Self {
            id: t.id,
            path: t.path,
            target_type: t.target_type.as_str().to_string(),
            namespace: t.namespace,
            enabled: t.enabled,
            last_scan_git_branch: t.last_scan_git_branch,
            last_scan_git_commit: t.last_scan_git_commit,
            __buffa_unknown_fields: buffa::UnknownFields::default(),
            __buffa_cached_size: buffa::__private::CachedSize::default(),
        }
    }
}

impl From<crate::indexer::TargetProgress> for IndexProgress {
    fn from(p: crate::indexer::TargetProgress) -> Self {
        Self {
            files_total: p.files_total as u64,
            files_pending: p.files_pending as u64,
            files_processing: p.files_processing as u64,
            files_completed: p.files_completed as u64,
            files_failed: p.files_failed as u64,
            current_file: p.current_file,
            last_error: p.last_error,
            __buffa_unknown_fields: buffa::UnknownFields::default(),
            __buffa_cached_size: buffa::__private::CachedSize::default(),
        }
    }
}

impl From<crate::core::service::ErrorEntry> for ErrorEntry {
    fn from(e: crate::core::service::ErrorEntry) -> Self {
        Self {
            error_id: e.error_id,
            timestamp: e.timestamp,
            component: e.component,
            severity: e.severity,
            message: e.message,
            details: e.details,
            __buffa_unknown_fields: buffa::UnknownFields::default(),
            __buffa_cached_size: buffa::__private::CachedSize::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_error_codes() {
        let not_found = map_error(ServerError::NotFound("missing".to_string()));
        assert_eq!(not_found.code, ErrorCode::NotFound);

        let invalid = map_error(ServerError::InvalidArgument("bad".to_string()));
        assert_eq!(invalid.code, ErrorCode::InvalidArgument);

        let internal = map_error(ServerError::DatabaseError("boom".to_string()));
        assert_eq!(internal.code, ErrorCode::Internal);
    }

    #[test]
    fn index_progress_conversion() {
        let progress = crate::indexer::TargetProgress {
            files_total: 10,
            files_pending: 5,
            files_processing: 2,
            files_completed: 3,
            files_failed: 1,
            current_file: Some("main.rs".to_string()),
            last_error: Some("oops".to_string()),
        };
        let proto = IndexProgress::from(progress);
        assert_eq!(proto.files_total, 10);
        assert_eq!(proto.files_pending, 5);
        assert_eq!(proto.files_processing, 2);
        assert_eq!(proto.files_completed, 3);
        assert_eq!(proto.files_failed, 1);
        assert_eq!(proto.current_file.as_deref(), Some("main.rs"));
        assert_eq!(proto.last_error.as_deref(), Some("oops"));
    }

    #[test]
    fn error_entry_conversion() {
        let entry = crate::core::service::ErrorEntry {
            error_id: "err-1".to_string(),
            timestamp: 1234567890,
            component: "test".to_string(),
            severity: "warn".to_string(),
            message: "hello".to_string(),
            details: Some("details".to_string()),
        };
        let proto = ErrorEntry::from(entry);
        assert_eq!(proto.error_id, "err-1");
        assert_eq!(proto.timestamp, 1234567890);
        assert_eq!(proto.component, "test");
        assert_eq!(proto.severity, "warn");
        assert_eq!(proto.message, "hello");
        assert_eq!(proto.details.as_deref(), Some("details"));
    }
}
