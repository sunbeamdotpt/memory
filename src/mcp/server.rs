// MCP server implementation using rmcp

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, Content, ErrorData, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler,
};
use rmcp::handler::server::wrapper::Parameters;
use chrono::TimeZone;

use crate::indexer::IndexService;
use crate::memory::service::MemoryService;
use crate::urn::{SourceUrn, schema_json, invalid_urn_response};

// ── parameter structs ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StoreFactParams {
    /// Text to store
    pub content: String,
    /// Logical grouping (e.g. 'code', 'docs', 'notes'). Defaults to 'default'.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Optional smem URN identifying where this content came from.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchFactsParams {
    /// Search query
    pub query: String,
    /// Max results to return (default: 10)
    #[serde(default)]
    pub limit: Option<u64>,
    /// Restrict search to a specific namespace
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateFactParams {
    /// Fact ID to update
    pub id: String,
    /// New text content
    pub content: String,
    /// New smem URN (optional)
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteFactParams {
    /// Fact ID (as returned by store_fact or search_facts)
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListFactsParams {
    /// Namespace to list (default: 'default')
    #[serde(default)]
    pub namespace: Option<String>,
    /// Max facts to return (default: 50)
    #[serde(default)]
    pub limit: Option<u64>,
    /// Return only facts stored on or after this time.
    #[serde(default)]
    pub from: Option<String>,
    /// Return only facts stored on or before this time.
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BuildSourceUrnParams {
    /// Content type: code | doc | web | data | note | conf
    pub content_type: String,
    /// Origin: git | fs | https | http | db | api | manual
    pub origin: String,
    /// Origin-specific locator.
    pub locator: String,
    /// Optional fragment: L42 (line), L10-L30 (range), or a slug anchor.
    #[serde(default)]
    pub fragment: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ParseSourceUrnParams {
    /// The URN to parse
    pub urn: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddWatchTargetParams {
    /// Absolute filesystem path to watch
    pub path: String,
    /// Namespace for ingested facts (default: 'default')
    #[serde(default)]
    pub namespace: Option<String>,
    /// Type: file | directory | git_repo. Auto-detected if omitted.
    #[serde(default)]
    pub target_type: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveWatchTargetParams {
    /// Target ID as returned by add_watch_target
    pub target_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SyncWatchTargetParams {
    /// Target ID to sync
    pub target_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetIndexProgressParams {
    /// Target ID
    pub target_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RestoreStaleFactParams {
    /// Fact ID to restore
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetRecentErrorsParams {
    /// Filter by component: indexer | api | mcp | watcher | extractor. Omit to see all.
    #[serde(default)]
    pub component: Option<String>,
    /// Max errors to return (default: 10)
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResolveErrorParams {
    /// Error ID to resolve
    pub error_id: String,
}

// ── server struct ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SunbeamServer {
    memory: MemoryService,
    indexer: IndexService,
    tool_router: ToolRouter<Self>,
}

impl SunbeamServer {
    pub fn new(memory: MemoryService, indexer: IndexService) -> Self {
        Self {
            memory,
            indexer,
            tool_router: Self::tool_router(),
        }
    }
}

// ── tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl SunbeamServer {
    #[tool(name = "store_fact", description = "Embed and store a piece of text in semantic memory. Returns the fact ID.")]
    pub async fn store_fact(&self, Parameters(params): Parameters<StoreFactParams>) -> Result<CallToolResult, ErrorData> {
        let content = params.content;
        if content.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`content` is required and must not be empty")]));
        }
        let namespace = params.namespace.as_deref().unwrap_or("default");

        if let Some(s) = &params.source {
            if let Err(e) = SourceUrn::parse(s) {
                return Ok(CallToolResult::error(vec![Content::text(format!("Invalid source URN: {e}"))]));
            }
        }

        match self.memory.add_fact(namespace, &content, params.source.as_deref()).await {
            Ok(fact) => {
                let source_line = match &fact.source {
                    Some(s) => {
                        let desc = SourceUrn::parse(s)
                            .map(|u| u.human_readable())
                            .unwrap_or_else(|_| s.clone());
                        format!("\nSource: {s}  ({desc})")
                    }
                    None => String::new(),
                };
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Stored.\nID: {}\nNamespace: {}\nCreated: {}{}",
                    fact.id, fact.namespace, fact.created_at, source_line
                ))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to store: {e}"))])),
        }
    }

    #[tool(name = "search_facts", description = "Search semantic memory for content similar to a query. Returns ranked results with similarity scores.")]
    pub async fn search_facts(&self, Parameters(params): Parameters<SearchFactsParams>) -> Result<CallToolResult, ErrorData> {
        let query = params.query;
        if query.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`query` is required and must not be empty")]));
        }
        let limit = params.limit.unwrap_or(10) as usize;
        let namespace = params.namespace.as_deref();

        match self.memory.search_facts(&query, limit, namespace).await {
            Ok(results) if results.is_empty() => Ok(CallToolResult::success(vec![Content::text("No results found.")])),
            Ok(results) => {
                let mut out = format!("Found {} result(s):\n\n", results.len());
                for (i, f) in results.iter().enumerate() {
                    out.push_str(&format!(
                        "{}. [{}] score={:.3}  id={}  created={}\n   {}",
                        i + 1, f.namespace, f.score, f.id, f.created_at, f.content
                    ));
                    if let Some(s) = &f.source {
                        let desc = SourceUrn::parse(s)
                            .map(|u| u.human_readable())
                            .unwrap_or_else(|_| s.clone());
                        out.push_str(&format!("\n   source: {s}  ({desc})"));
                    }
                    out.push_str("\n\n");
                }
                Ok(CallToolResult::success(vec![Content::text(out.trim_end())]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Search failed: {e}"))])),
        }
    }

    #[tool(name = "update_fact", description = "Update an existing fact in place, keeping the same ID. Re-embeds the new content and replaces the vector.")]
    pub async fn update_fact(&self, Parameters(params): Parameters<UpdateFactParams>) -> Result<CallToolResult, ErrorData> {
        let id = params.id;
        if id.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`id` is required")]));
        }
        let content = params.content;
        if content.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`content` is required and must not be empty")]));
        }
        if let Some(s) = &params.source {
            if let Err(e) = SourceUrn::parse(s) {
                return Ok(CallToolResult::error(vec![Content::text(format!("Invalid source URN: {e}"))]));
            }
        }
        match self.memory.update_fact(&id, &content, params.source.as_deref()).await {
            Ok(fact) => {
                let source_line = match &fact.source {
                    Some(s) => {
                        let desc = SourceUrn::parse(s).map(|u| u.human_readable()).unwrap_or_else(|_| s.clone());
                        format!("\nSource: {s}  ({desc})")
                    }
                    None => String::new(),
                };
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Updated.\nID: {}\nNamespace: {}\nCreated: {}{}",
                    fact.id, fact.namespace, fact.created_at, source_line
                ))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Update failed: {e}"))])),
        }
    }

    #[tool(name = "delete_fact", description = "Delete a stored fact by its ID.")]
    pub async fn delete_fact(&self, Parameters(params): Parameters<DeleteFactParams>) -> Result<CallToolResult, ErrorData> {
        let id = params.id;
        if id.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`id` is required")]));
        }
        match self.memory.delete_fact(&id).await {
            Ok(true) => Ok(CallToolResult::success(vec![Content::text(format!("Deleted {id}."))])),
            Ok(false) => Ok(CallToolResult::success(vec![Content::text(format!("Fact {id} not found."))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Delete failed: {e}"))])),
        }
    }

    #[tool(name = "list_facts", description = "List facts stored in a namespace, most recent first. Supports date range filtering.")]
    pub async fn list_facts(&self, Parameters(params): Parameters<ListFactsParams>) -> Result<CallToolResult, ErrorData> {
        let namespace = params.namespace.as_deref().unwrap_or("default");
        let limit = params.limit.unwrap_or(50) as usize;
        let from_ts = params.from.as_deref().and_then(parse_ts);
        let to_ts = params.to.as_deref().and_then(parse_ts);

        match self.memory.list_facts(namespace, limit, from_ts, to_ts).await {
            Ok(facts) if facts.is_empty() => {
                Ok(CallToolResult::success(vec![Content::text(format!("No facts in namespace '{namespace}'."))]))
            }
            Ok(facts) => {
                let mut out = format!("{} fact(s) in '{namespace}':\n\n", facts.len());
                for f in &facts {
                    out.push_str(&format!("id={}\ncreated: {}\n{}", f.id, f.created_at, f.content));
                    if let Some(s) = &f.source {
                        let desc = SourceUrn::parse(s)
                            .map(|u| u.human_readable())
                            .unwrap_or_else(|_| s.clone());
                        out.push_str(&format!("\nsource: {s}  ({desc})"));
                    }
                    out.push_str("\n\n");
                }
                Ok(CallToolResult::success(vec![Content::text(out.trim_end())]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("List failed: {e}"))])),
        }
    }

    #[tool(name = "build_source_urn", description = "Build a valid smem URN from its components. Use this before calling store_fact with a source.")]
    pub async fn build_source_urn(&self, Parameters(params): Parameters<BuildSourceUrnParams>) -> Result<CallToolResult, ErrorData> {
        let content_type = params.content_type;
        if content_type.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`content_type` is required")]));
        }
        let origin = params.origin;
        if origin.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`origin` is required")]));
        }
        let locator = params.locator;
        if locator.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`locator` is required")]));
        }
        match SourceUrn::build(&content_type, &origin, &locator, params.fragment.as_deref()) {
            Ok(urn) => Ok(CallToolResult::success(vec![Content::text(urn)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to build URN: {e}"))])),
        }
    }

    #[tool(name = "parse_source_urn", description = "Parse and validate a smem URN. Returns structured components or an error with the spec.")]
    pub async fn parse_source_urn(&self, Parameters(params): Parameters<ParseSourceUrnParams>) -> Result<CallToolResult, ErrorData> {
        let urn_str = params.urn;
        if urn_str.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`urn` is required")]));
        }
        let result = match SourceUrn::parse(&urn_str) {
            Ok(urn) => urn.describe(),
            Err(e) => invalid_urn_response(&urn_str, &e),
        };
        match serde_json::to_string_pretty(&result) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("JSON serialization failed: {e}"))])),
        }
    }

    #[tool(name = "describe_urn_schema", description = "Return the full machine-readable smem URN taxonomy: content types, origins, locator shapes, and examples.")]
    pub async fn describe_urn_schema(&self) -> Result<CallToolResult, ErrorData> {
        match serde_json::to_string_pretty(&schema_json()) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("JSON serialization failed: {e}"))])),
        }
    }

    #[tool(name = "add_watch_target", description = "Add a file, directory, or git repository to the automatic indexing watch list. The indexer will scan it immediately and re-ingest when files change.")]
    pub async fn add_watch_target(&self, Parameters(params): Parameters<AddWatchTargetParams>) -> Result<CallToolResult, ErrorData> {
        let path = params.path;
        if path.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`path` is required")]));
        }
        let namespace = params.namespace.as_deref();
        let target_type = params.target_type.as_deref();

        match self.indexer.add_target(&path, namespace, target_type).await {
            Ok(ids) if ids.len() == 1 => {
                let id = &ids[0];
                let indexer_clone = self.indexer.clone();
                let id_clone = id.clone();
                tokio::spawn(async move {
                    if let Err(e) = indexer_clone.sync_target(&id_clone).await {
                        let err_id = ulid::Ulid::new().to_string();
                        let _ = indexer_clone.memory().log_error(&err_id, "indexer", "error", &format!("initial sync failed for target {id_clone}"), Some(&e.to_string()));
                    }
                });
                Ok(CallToolResult::success(vec![Content::text(format!("Watch target added.\nID: {id}\nPath: {path}"))]))
            }
            Ok(ids) => {
                let count = ids.len();
                let indexer_clone = self.indexer.clone();
                let ids_for_sync = ids.clone();
                tokio::spawn(async move {
                    for id in &ids_for_sync {
                        if let Err(e) = indexer_clone.sync_target(id).await {
                            let err_id = ulid::Ulid::new().to_string();
                            let _ = indexer_clone.memory().log_error(&err_id, "indexer", "error", &format!("initial sync failed for target {id}"), Some(&e.to_string()));
                        }
                    }
                });
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Watch targets added from glob.\nCount: {count}\nPattern: {path}\nIDs: {}",
                    ids.join(", ")
                ))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to add watch target: {e}"))])),
        }
    }

    #[tool(name = "remove_watch_target", description = "Remove a watch target by ID. Stops watching the path.")]
    pub async fn remove_watch_target(&self, Parameters(params): Parameters<RemoveWatchTargetParams>) -> Result<CallToolResult, ErrorData> {
        let target_id = params.target_id;
        if target_id.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`target_id` is required")]));
        }
        match self.indexer.remove_target(&target_id).await {
            Ok(true) => Ok(CallToolResult::success(vec![Content::text(format!("Removed watch target {target_id}."))])),
            Ok(false) => Ok(CallToolResult::success(vec![Content::text(format!("Watch target {target_id} not found."))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to remove watch target: {e}"))])),
        }
    }

    #[tool(name = "list_watch_targets", description = "List all indexing watch targets with their current progress and git state.")]
    pub async fn list_watch_targets(&self) -> Result<CallToolResult, ErrorData> {
        match self.indexer.list_targets().await {
            Ok(targets) if targets.is_empty() => Ok(CallToolResult::success(vec![Content::text("No watch targets configured.")])),
            Ok(targets) => {
                let mut out = format!("{} watch target(s):\n\n", targets.len());
                for t in &targets {
                    let status = if t.enabled { "enabled" } else { "disabled" };
                    let git_info = match (&t.last_scan_git_branch, &t.last_scan_git_commit) {
                        (Some(b), Some(c)) => format!(" | branch: {b} | commit: {c}"),
                        (Some(b), None) => format!(" | branch: {b}"),
                        _ => String::new(),
                    };
                    let progress = self.indexer.progress().get(&t.id).map(|p| {
                        format!(" | pending: {} | processing: {} | completed: {} | failed: {}",
                            p.files_pending, p.files_processing, p.files_completed, p.files_failed)
                    }).unwrap_or_default();
                    out.push_str(&format!(
                        "id={}\npath: {}\ntype: {} | namespace: {} | status: {}{}{}\n\n",
                        t.id, t.path, t.target_type.as_str(), t.namespace, status, git_info, progress
                    ));
                }
                Ok(CallToolResult::success(vec![Content::text(out.trim_end())]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to list watch targets: {e}"))])),
        }
    }

    #[tool(name = "sync_watch_target", description = "Force an immediate rescan and re-ingestion of a watch target.")]
    pub async fn sync_watch_target(&self, Parameters(params): Parameters<SyncWatchTargetParams>) -> Result<CallToolResult, ErrorData> {
        let target_id = params.target_id;
        if target_id.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`target_id` is required")]));
        }
        let indexer_clone = self.indexer.clone();
        let id_clone = target_id.clone();
        tokio::spawn(async move {
            if let Err(e) = indexer_clone.sync_target(&id_clone).await {
                let err_id = ulid::Ulid::new().to_string();
                let _ = indexer_clone.memory().log_error(&err_id, "indexer", "error", &format!("sync failed for target {id_clone}"), Some(&e.to_string()));
            }
        });
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Sync started for target {target_id}. Use list_watch_targets to check progress."
        ))]))
    }

    #[tool(name = "get_index_progress", description = "Get detailed indexing progress for a specific target.")]
    pub async fn get_index_progress(&self, Parameters(params): Parameters<GetIndexProgressParams>) -> Result<CallToolResult, ErrorData> {
        let target_id = params.target_id;
        if target_id.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`target_id` is required")]));
        }
        match self.indexer.progress().get(&target_id) {
            Some(p) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Progress for target {target_id}:\ntotal: {}\npending: {}\nprocessing: {}\ncompleted: {}\nfailed: {}\ncurrent_file: {:?}\nlast_error: {:?}",
                p.files_total, p.files_pending, p.files_processing, p.files_completed, p.files_failed,
                p.current_file, p.last_error
            ))])),
            None => Ok(CallToolResult::success(vec![Content::text(format!("No progress data for target {target_id}."))])),
        }
    }

    #[tool(name = "restore_stale_fact", description = "Restore a stale (soft-deleted) fact so it appears in search again.")]
    pub async fn restore_stale_fact(&self, Parameters(params): Parameters<RestoreStaleFactParams>) -> Result<CallToolResult, ErrorData> {
        let id = params.id;
        if id.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`id` is required")]));
        }
        let store = self.memory.get_store();
        let db = store.db();
        let db_guard = db.lock().unwrap();
        match db_guard.restore_fact(&id) {
            Ok(true) => Ok(CallToolResult::success(vec![Content::text(format!("Restored fact {id}."))])),
            Ok(false) => Ok(CallToolResult::success(vec![Content::text(format!("Fact {id} not found."))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Restore failed: {e}"))])),
        }
    }

    #[tool(name = "get_recent_errors", description = "Retrieve recent system errors logged by the indexer, API, or other components. Use this to diagnose failures.")]
    pub async fn get_recent_errors(&self, Parameters(params): Parameters<GetRecentErrorsParams>) -> Result<CallToolResult, ErrorData> {
        let component = params.component.as_deref();
        let limit = params.limit.unwrap_or(10) as usize;
        match self.memory.get_recent_errors(component, limit).await {
            Ok(errors) if errors.is_empty() => Ok(CallToolResult::success(vec![Content::text("No unresolved errors.")])),
            Ok(errors) => {
                let lines: Vec<String> = errors.into_iter().map(|(id, ts, comp, sev, msg, details)| {
                    let dt = chrono::Utc.timestamp_opt(ts, 0).single()
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_else(|| ts.to_string());
                    let detail_line = details.map(|d| format!("\n  details: {d}")).unwrap_or_default();
                    format!("[{dt}] [{sev}] {comp}\n  id: {id}\n  msg: {msg}{detail_line}")
                }).collect();
                Ok(CallToolResult::success(vec![Content::text(format!("Recent errors:\n\n{}", lines.join("\n\n")))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to fetch errors: {e}"))])),
        }
    }

    #[tool(name = "resolve_error", description = "Mark a logged error as resolved so it no longer appears in get_recent_errors.")]
    pub async fn resolve_error(&self, Parameters(params): Parameters<ResolveErrorParams>) -> Result<CallToolResult, ErrorData> {
        let error_id = params.error_id;
        if error_id.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("`error_id` is required")]));
        }
        match self.memory.resolve_error(&error_id).await {
            Ok(true) => Ok(CallToolResult::success(vec![Content::text(format!("Resolved error {error_id}."))])),
            Ok(false) => Ok(CallToolResult::success(vec![Content::text(format!("Error {error_id} not found."))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("Failed to resolve error: {e}"))])),
        }
    }
}

// ── ServerHandler ─────────────────────────────────────────────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SunbeamServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_server_info(Implementation::new(
                "sunbeam-memory",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Personal semantic memory server for AI assistants.")
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
