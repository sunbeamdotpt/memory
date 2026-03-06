// JSON-RPC 2.0 + MCP wire types

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Incoming ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent on notifications; present on requests.
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl Request {
    /// True when this is a notification (no id, no response expected).
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

// ── Outgoing ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// Standard JSON-RPC error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }

    pub fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError { code, message: message.into(), data: None }),
        }
    }
}

// ── MCP tool response content ─────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: &'static str, // always "text"
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub content: Vec<TextContent>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![TextContent { kind: "text", text: text.into() }],
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![TextContent { kind: "text", text: message.into() }],
            is_error: true,
        }
    }
}
