// MCP request dispatcher and tool implementations

use serde_json::{json, Value};
use crate::memory::service::MemoryService;
use crate::urn::{SourceUrn, schema_json, invalid_urn_response, SPEC};
use chrono;
use crate::mcp::protocol::{
    Request, Response, ToolResult,
    METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL_ERROR,
};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Dispatch an incoming JSON-RPC request. Returns None for notifications
/// (which expect no response).
pub async fn handle(req: &Request, memory: &MemoryService) -> Option<Response> {
    if req.is_notification() {
        return None;
    }

    let id = req.id.clone().unwrap_or(Value::Null);

    let outcome = match req.method.as_str() {
        "initialize"  => Ok(initialize()),
        "ping"        => Ok(json!({})),
        "tools/list"  => Ok(tools_list()),
        "tools/call"  => tools_call(req.params.as_ref(), memory).await,
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
            }
        ]
    })
}

// ── tools/call ────────────────────────────────────────────────────────────────

async fn tools_call(
    params: Option<&Value>,
    memory: &MemoryService,
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
            return ToolResult::text(
                serde_json::to_string(&invalid_urn_response(s, &e)).unwrap_or_default()
            );
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
            return ToolResult::text(
                serde_json::to_string(&invalid_urn_response(s, &e)).unwrap_or_default()
            );
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
        Err(e) => ToolResult::text(
            serde_json::to_string(&serde_json::json!({
                "error": e.to_string(),
                "spec": SPEC,
            })).unwrap_or_default()
        ),
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

    ToolResult::text(serde_json::to_string_pretty(&result).unwrap_or_default())
}

fn tool_describe_urn_schema() -> ToolResult {
    ToolResult::text(serde_json::to_string_pretty(&schema_json()).unwrap_or_default())
}
