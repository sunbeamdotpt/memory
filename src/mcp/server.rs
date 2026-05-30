// MCP request dispatcher and tool implementations

use serde_json::{json, Value};
use crate::indexer::IndexService;
use crate::memory::service::MemoryService;
use crate::urn::{SourceUrn, schema_json, invalid_urn_response};
use chrono;
use chrono::TimeZone;
use crate::mcp::protocol::{
    Request, Response, ToolResult,
    METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL_ERROR, PARSE_ERROR,
};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Dispatch an incoming JSON-RPC request. Returns None for notifications
/// (which expect no response).
pub async fn handle(req: &Request, memory: &MemoryService, indexer: &IndexService) -> Option<Response> {
    if req.is_notification() {
        return None;
    }

    let id = req.id.clone().unwrap_or(Value::Null);

    let outcome = match req.method.as_str() {
        "initialize"  => Ok(initialize()),
        "ping"        => Ok(json!({})),
        "tools/list"  => Ok(tools_list()),
        "tools/call"  => tools_call(req.params.as_ref(), memory, indexer).await,
        other         => Err((METHOD_NOT_FOUND, format!("Unknown method: {other}"))),
    };

    Some(match outcome {
        Ok(v)          => Response::ok(id, v),
        Err((c, msg))  => Response::err(id, c, msg),
    })
}

// ── initialize ────────────────────────────────────────────────────────────────

fn initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "sunbeam-memory",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

// ── tools/list ────────────────────────────────────────────────────────────────

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "store_fact",
                "description": "Embed and store a piece of text in semantic memory. Returns the fact ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "Text to store"
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Logical grouping (e.g. 'code', 'docs', 'notes'). Defaults to 'default'."
                        },
                        "source": {
                            "type": "string",
                            "description": "Optional smem URN identifying where this content came from. Must be a valid urn:smem: URN if provided. Use build_source_urn to construct one. Example: urn:smem:code:fs:/path/to/file.rs#L10-L30"
                        }
                    },
                    "required": ["content"]
                }
            },
            {
                "name": "search_facts",
                "description": "Search semantic memory for content similar to a query. Returns ranked results with similarity scores.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results to return (default: 10)"
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Restrict search to a specific namespace"
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "update_fact",
                "description": "Update an existing fact in place, keeping the same ID. Re-embeds the new content and replaces the vector.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id":      { "type": "string", "description": "Fact ID to update" },
                        "content": { "type": "string", "description": "New text content" },
                        "source":  { "type": "string", "description": "New smem URN (optional)" }
                    },
                    "required": ["id", "content"]
                }
            },
            {
                "name": "delete_fact",
                "description": "Delete a stored fact by its ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Fact ID (as returned by store_fact or search_facts)"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "list_facts",
                "description": "List facts stored in a namespace, most recent first. Supports date range filtering.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "namespace": {
                            "type": "string",
                            "description": "Namespace to list (default: 'default')"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max facts to return (default: 50)"
                        },
                        "from": {
                            "type": "string",
                            "description": "Return only facts stored on or after this time. Accepts RFC 3339 (e.g. '2026-03-01T00:00:00Z') or Unix timestamp integer as string."
                        },
                        "to": {
                            "type": "string",
                            "description": "Return only facts stored on or before this time. Same format as 'from'."
                        }
                    }
                }
            },
            {
                "name": "build_source_urn",
                "description": "Build a valid smem URN from its components. Use this before calling store_fact with a source.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content_type": {
                            "type": "string",
                            "description": "Content type: code | doc | web | data | note | conf"
                        },
                        "origin": {
                            "type": "string",
                            "description": "Origin: git | fs | https | http | db | api | manual"
                        },
                        "locator": {
                            "type": "string",
                            "description": "Origin-specific locator. See describe_urn_schema for shapes."
                        },
                        "fragment": {
                            "type": "string",
                            "description": "Optional fragment: L42 (line), L10-L30 (range), or a slug anchor."
                        }
                    },
                    "required": ["content_type", "origin", "locator"]
                }
            },
            {
                "name": "parse_source_urn",
                "description": "Parse and validate a smem URN. Returns structured components or an error with the spec.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "urn": {
                            "type": "string",
                            "description": "The URN to parse, e.g. urn:smem:code:fs:/path/to/file.rs#L10"
                        }
                    },
                    "required": ["urn"]
                }
            },
            {
                "name": "describe_urn_schema",
                "description": "Return the full machine-readable smem URN taxonomy: content types, origins, locator shapes, and examples.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "add_watch_target",
                "description": "Add a file, directory, or git repository to the automatic indexing watch list. The indexer will scan it immediately and re-ingest when files change.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute filesystem path to watch"
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Namespace for ingested facts (default: 'default')"
                        },
                        "target_type": {
                            "type": "string",
                            "description": "Type: file | directory | git_repo. Auto-detected if omitted."
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "remove_watch_target",
                "description": "Remove a watch target by ID. Stops watching the path.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target_id": {
                            "type": "string",
                            "description": "Target ID as returned by add_watch_target"
                        }
                    },
                    "required": ["target_id"]
                }
            },
            {
                "name": "list_watch_targets",
                "description": "List all indexing watch targets with their current progress and git state.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "sync_watch_target",
                "description": "Force an immediate rescan and re-ingestion of a watch target.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target_id": {
                            "type": "string",
                            "description": "Target ID to sync"
                        }
                    },
                    "required": ["target_id"]
                }
            },
            {
                "name": "get_index_progress",
                "description": "Get detailed indexing progress for a specific target.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target_id": {
                            "type": "string",
                            "description": "Target ID"
                        }
                    },
                    "required": ["target_id"]
                }
            },
            {
                "name": "restore_stale_fact",
                "description": "Restore a stale (soft-deleted) fact so it appears in search again.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Fact ID to restore"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "get_recent_errors",
                "description": "Retrieve recent system errors logged by the indexer, API, or other components. Use this to diagnose failures.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "component": {
                            "type": "string",
                            "description": "Filter by component: indexer | api | mcp | watcher | extractor. Omit to see all."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max errors to return (default: 10)"
                        }
                    }
                }
            },
            {
                "name": "resolve_error",
                "description": "Mark a logged error as resolved so it no longer appears in get_recent_errors.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "error_id": {
                            "type": "string",
                            "description": "Error ID to resolve"
                        }
                    },
                    "required": ["error_id"]
                }
            }
        ]
    })
}

// ── tools/call ────────────────────────────────────────────────────────────────

async fn tools_call(
    params: Option<&Value>,
    memory: &MemoryService,
    indexer: &IndexService,
) -> Result<Value, (i32, String)> {
    let params = params.ok_or((INVALID_PARAMS, "Missing params".to_string()))?;

    let name = params["name"]
        .as_str()
        .ok_or((INVALID_PARAMS, "Missing tool name".to_string()))?;

    let args = &params["arguments"];

    let result = match name {
        "store_fact"        => tool_store_fact(args, memory).await,
        "update_fact"       => tool_update_fact(args, memory).await,
        "search_facts"      => tool_search_facts(args, memory).await,
        "delete_fact"       => tool_delete_fact(args, memory).await,
        "list_facts"        => tool_list_facts(args, memory).await,
        "build_source_urn"  => tool_build_source_urn(args),
        "parse_source_urn"  => tool_parse_source_urn(args),
        "describe_urn_schema" => tool_describe_urn_schema(),
        "add_watch_target"  => tool_add_watch_target(args, indexer).await,
        "remove_watch_target" => tool_remove_watch_target(args, indexer).await,
        "list_watch_targets" => tool_list_watch_targets(indexer).await,
        "sync_watch_target" => tool_sync_watch_target(args, indexer).await,
        "get_index_progress" => tool_get_index_progress(args, indexer).await,
        "restore_stale_fact" => tool_restore_stale_fact(args, memory).await,
        "get_recent_errors" => tool_get_recent_errors(args, memory).await,
        "resolve_error"     => tool_resolve_error(args, memory).await,
        other               => ToolResult::error(format!("Unknown tool: {other}")),
    };

    serde_json::to_value(result)
        .map_err(|e| (INTERNAL_ERROR, e.to_string()))
}

// ── individual tools ──────────────────────────────────────────────────────────

async fn tool_store_fact(args: &Value, memory: &MemoryService) -> ToolResult {
    let content = match args["content"].as_str() {
        Some(c) if !c.trim().is_empty() => c,
        _ => return ToolResult::error("`content` is required and must not be empty"),
    };
    let namespace = args["namespace"].as_str().unwrap_or("default");

    // Validate source URN if provided
    let source = args["source"].as_str();
    if let Some(s) = source {
        if let Err(e) = SourceUrn::parse(s) {
            return ToolResult::error(format!("Invalid source URN: {e}"));
        }
    }

    match memory.add_fact(namespace, content, source).await {
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
            ToolResult::text(format!(
                "Stored.\nID: {}\nNamespace: {}\nCreated: {}{}",
                fact.id, fact.namespace, fact.created_at, source_line
            ))
        }
        Err(e) => ToolResult::error(format!("Failed to store: {e}")),
    }
}

async fn tool_search_facts(args: &Value, memory: &MemoryService) -> ToolResult {
    let query = match args["query"].as_str() {
        Some(q) if !q.trim().is_empty() => q,
        _ => return ToolResult::error("`query` is required and must not be empty"),
    };
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let namespace = args["namespace"].as_str();

    match memory.search_facts(query, limit, namespace).await {
        Ok(results) if results.is_empty() => ToolResult::text("No results found."),
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
            ToolResult::text(out.trim_end())
        }
        Err(e) => ToolResult::error(format!("Search failed: {e}")),
    }
}

async fn tool_update_fact(args: &Value, memory: &MemoryService) -> ToolResult {
    let id = match args["id"].as_str() {
        Some(s) if !s.trim().is_empty() => s,
        _ => return ToolResult::error("`id` is required"),
    };
    let content = match args["content"].as_str() {
        Some(s) if !s.trim().is_empty() => s,
        _ => return ToolResult::error("`content` is required and must not be empty"),
    };
    let source = args["source"].as_str();
    if let Some(s) = source {
        if let Err(e) = SourceUrn::parse(s) {
            return ToolResult::error(format!("Invalid source URN: {e}"));
        }
    }
    match memory.update_fact(id, content, source).await {
        Ok(fact) => {
            let source_line = match &fact.source {
                Some(s) => {
                    let desc = SourceUrn::parse(s).map(|u| u.human_readable()).unwrap_or_else(|_| s.clone());
                    format!("\nSource: {s}  ({desc})")
                }
                None => String::new(),
            };
            ToolResult::text(format!(
                "Updated.\nID: {}\nNamespace: {}\nCreated: {}{}",
                fact.id, fact.namespace, fact.created_at, source_line
            ))
        }
        Err(e) => ToolResult::error(format!("Update failed: {e}")),
    }
}

async fn tool_delete_fact(args: &Value, memory: &MemoryService) -> ToolResult {
    let id = match args["id"].as_str() {
        Some(id) if !id.trim().is_empty() => id,
        _ => return ToolResult::error("`id` is required"),
    };

    match memory.delete_fact(id).await {
        Ok(true)  => ToolResult::text(format!("Deleted {id}.")),
        Ok(false) => ToolResult::text(format!("Fact {id} not found.")),
        Err(e)    => ToolResult::error(format!("Delete failed: {e}")),
    }
}

async fn tool_list_facts(args: &Value, memory: &MemoryService) -> ToolResult {
    let namespace = args["namespace"].as_str().unwrap_or("default");
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;
    let from_ts = parse_date_arg(args["from"].as_str());
    let to_ts = parse_date_arg(args["to"].as_str());

    match memory.list_facts(namespace, limit, from_ts, to_ts).await {
        Ok(facts) if facts.is_empty() => {
            ToolResult::text(format!("No facts in namespace '{namespace}'."))
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
            ToolResult::text(out.trim_end())
        }
        Err(e) => ToolResult::error(format!("List failed: {e}")),
    }
}

/// Parse a date string: RFC 3339 or raw Unix timestamp integer.
pub fn parse_ts(s: &str) -> Option<i64> {
    if let Ok(ts) = s.parse::<i64>() {
        return Some(ts);
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

fn parse_date_arg(s: Option<&str>) -> Option<i64> {
    parse_ts(s?)
}

fn tool_build_source_urn(args: &Value) -> ToolResult {
    let content_type = match args["content_type"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`content_type` is required"),
    };
    let origin = match args["origin"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`origin` is required"),
    };
    let locator = match args["locator"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`locator` is required"),
    };
    let fragment = args["fragment"].as_str();

    match SourceUrn::build(content_type, origin, locator, fragment) {
        Ok(urn) => ToolResult::text(urn),
        Err(e) => ToolResult::error(format!("Failed to build URN: {e}")),
    }
}

fn tool_parse_source_urn(args: &Value) -> ToolResult {
    let urn_str = match args["urn"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`urn` is required"),
    };

    let result = match SourceUrn::parse(urn_str) {
        Ok(urn) => urn.describe(),
        Err(e) => invalid_urn_response(urn_str, &e),
    };

    match serde_json::to_string_pretty(&result) {
        Ok(json) => ToolResult::text(json),
        Err(e) => ToolResult::error(format!("JSON serialization failed: {e}")),
    }
}

fn tool_describe_urn_schema() -> ToolResult {
    match serde_json::to_string_pretty(&schema_json()) {
        Ok(json) => ToolResult::text(json),
        Err(e) => ToolResult::error(format!("JSON serialization failed: {e}")),
    }
}

// ── indexer tools ─────────────────────────────────────────────────────────────

async fn tool_add_watch_target(args: &Value, indexer: &IndexService) -> ToolResult {
    let path = match args["path"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`path` is required"),
    };
    let namespace = args["namespace"].as_str();
    let target_type = args["target_type"].as_str();

    match indexer.add_target(path, namespace, target_type).await {
        Ok(ids) if ids.len() == 1 => {
            let id = &ids[0];
            let indexer_clone = indexer.clone();
            let id_clone = id.clone();
            tokio::spawn(async move {
                if let Err(e) = indexer_clone.sync_target(&id_clone).await {
                    let err_id = ulid::Ulid::new().to_string();
                    let _ = indexer_clone.memory().log_error(&err_id, "indexer", "error", &format!("initial sync failed for target {id_clone}"), Some(&e.to_string()));
                }
            });
            ToolResult::text(format!("Watch target added.\nID: {id}\nPath: {path}"))
        }
        Ok(ids) => {
            let count = ids.len();
            let indexer_clone = indexer.clone();
            let ids_for_sync = ids.clone();
            tokio::spawn(async move {
                for id in &ids_for_sync {
                    if let Err(e) = indexer_clone.sync_target(id).await {
                        let err_id = ulid::Ulid::new().to_string();
                        let _ = indexer_clone.memory().log_error(&err_id, "indexer", "error", &format!("initial sync failed for target {id}"), Some(&e.to_string()));
                    }
                }
            });
            ToolResult::text(format!(
                "Watch targets added from glob.\nCount: {count}\nPattern: {path}\nIDs: {}",
                ids.join(", ")
            ))
        }
        Err(e) => ToolResult::error(format!("Failed to add watch target: {e}")),
    }
}

async fn tool_remove_watch_target(args: &Value, indexer: &IndexService) -> ToolResult {
    let target_id = match args["target_id"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`target_id` is required"),
    };

    match indexer.remove_target(target_id).await {
        Ok(true) => ToolResult::text(format!("Removed watch target {target_id}.")),
        Ok(false) => ToolResult::text(format!("Watch target {target_id} not found.")),
        Err(e) => ToolResult::error(format!("Failed to remove watch target: {e}")),
    }
}

async fn tool_list_watch_targets(indexer: &IndexService) -> ToolResult {
    match indexer.list_targets().await {
        Ok(targets) if targets.is_empty() => ToolResult::text("No watch targets configured."),
        Ok(targets) => {
            let mut out = format!("{} watch target(s):\n\n", targets.len());
            for t in &targets {
                let status = if t.enabled { "enabled" } else { "disabled" };
                let git_info = match (&t.last_scan_git_branch, &t.last_scan_git_commit) {
                    (Some(b), Some(c)) => format!(" | branch: {b} | commit: {c}"),
                    (Some(b), None) => format!(" | branch: {b}"),
                    _ => String::new(),
                };
                let progress = indexer.progress().get(&t.id).map(|p| {
                    format!(" | pending: {} | processing: {} | completed: {} | failed: {}",
                        p.files_pending, p.files_processing, p.files_completed, p.files_failed)
                }).unwrap_or_default();
                out.push_str(&format!(
                    "id={}\npath: {}\ntype: {} | namespace: {} | status: {}{}{}\n\n",
                    t.id, t.path, t.target_type.as_str(), t.namespace, status, git_info, progress
                ));
            }
            ToolResult::text(out.trim_end())
        }
        Err(e) => ToolResult::error(format!("Failed to list watch targets: {e}")),
    }
}

async fn tool_sync_watch_target(args: &Value, indexer: &IndexService) -> ToolResult {
    let target_id = match args["target_id"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`target_id` is required"),
    };

    // Run sync in background so we don't block the MCP response
    let indexer_clone = indexer.clone();
    let id_clone = target_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = indexer_clone.sync_target(&id_clone).await {
            let err_id = ulid::Ulid::new().to_string();
            let _ = indexer_clone.memory().log_error(&err_id, "indexer", "error", &format!("sync failed for target {id_clone}"), Some(&e.to_string()));
        }
    });

    ToolResult::text(format!("Sync started for target {target_id}. Use list_watch_targets to check progress."))
}

async fn tool_get_index_progress(args: &Value, indexer: &IndexService) -> ToolResult {
    let target_id = match args["target_id"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`target_id` is required"),
    };

    match indexer.progress().get(target_id) {
        Some(p) => ToolResult::text(format!(
            "Progress for target {target_id}:\ntotal: {}\npending: {}\nprocessing: {}\ncompleted: {}\nfailed: {}\ncurrent_file: {:?}\nlast_error: {:?}",
            p.files_total, p.files_pending, p.files_processing, p.files_completed, p.files_failed,
            p.current_file, p.last_error
        )),
        None => ToolResult::text(format!("No progress data for target {target_id}.")),
    }
}

async fn tool_restore_stale_fact(args: &Value, memory: &MemoryService) -> ToolResult {
    let id = match args["id"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`id` is required"),
    };

    let store = memory.get_store();
    let db = store.db();
    let db_guard = db.lock().unwrap();
    match db_guard.restore_fact(id) {
        Ok(true) => ToolResult::text(format!("Restored fact {id}.")),
        Ok(false) => ToolResult::text(format!("Fact {id} not found.")),
        Err(e) => ToolResult::error(format!("Restore failed: {e}")),
    }
}

async fn tool_get_recent_errors(args: &Value, memory: &MemoryService) -> ToolResult {
    let component = args["component"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;

    match memory.get_recent_errors(component, limit).await {
        Ok(errors) if errors.is_empty() => ToolResult::text("No unresolved errors.".to_string()),
        Ok(errors) => {
            let lines: Vec<String> = errors.into_iter().map(|(id, ts, comp, sev, msg, details)| {
                let dt = chrono::Utc.timestamp_opt(ts, 0).single()
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_else(|| ts.to_string());
                let detail_line = details.map(|d| format!("\n  details: {d}")).unwrap_or_default();
                format!("[{dt}] [{sev}] {comp}\n  id: {id}\n  msg: {msg}{detail_line}")
            }).collect();
            ToolResult::text(format!("Recent errors:\n\n{}", lines.join("\n\n")))
        }
        Err(e) => ToolResult::error(format!("Failed to fetch errors: {e}")),
    }
}

async fn tool_resolve_error(args: &Value, memory: &MemoryService) -> ToolResult {
    let error_id = match args["error_id"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return ToolResult::error("`error_id` is required"),
    };

    match memory.resolve_error(error_id).await {
        Ok(true) => ToolResult::text(format!("Resolved error {error_id}.")),
        Ok(false) => ToolResult::text(format!("Error {error_id} not found.")),
        Err(e) => ToolResult::error(format!("Failed to resolve error: {e}")),
    }
}

/// Process a single MCP JSON-RPC line and return the JSON response string.
/// Used by the stdio transport and exposed for testing.
pub async fn process_mcp_line(line: &str, memory: &MemoryService, indexer: &IndexService) -> Option<String> {
    let response: Option<Response> = match serde_json::from_str::<Request>(line) {
        Ok(req) => handle(&req, memory, indexer).await,
        Err(err) => Some(Response::err(Value::Null, PARSE_ERROR, err.to_string())),
    };
    response.and_then(|resp| serde_json::to_string(&resp).ok())
}
