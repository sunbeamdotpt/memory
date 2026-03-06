// MCP Streamable HTTP transport (protocol version 2025-06-18)
//
// Spec: https://spec.modelcontextprotocol.io/specification/2025-06-18/basic/transports/
//
// POST /mcp  — client→server JSON-RPC (requests, notifications, responses)
// GET  /mcp  — server→client SSE push (405: we have no server-initiated messages)
// DELETE /mcp — explicit session teardown

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Mutex;
use uuid::Uuid;

use crate::api::oidc::OidcVerifier;
use crate::memory::service::MemoryService;
use crate::mcp::protocol::{Request, Response, PARSE_ERROR, INVALID_PARAMS};
use crate::mcp::server::handle;

/// Auth configuration, chosen at startup based on environment variables.
pub enum AuthConfig {
    /// No auth — bind localhost, check Origin header (default).
    LocalOnly,
    /// Simple shared bearer token — bind 0.0.0.0, skip Origin check.
    Bearer(String),
    /// OIDC JWT verification — bind 0.0.0.0, skip Origin check.
    Oidc(OidcVerifier),
}

impl AuthConfig {
    /// Returns true when the server should bind to all interfaces.
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::LocalOnly)
    }
}

const PROTOCOL_VERSION: &str = "2025-06-18";

// ── session store ─────────────────────────────────────────────────────────────

pub struct SessionStore(Mutex<HashSet<String>>);

impl SessionStore {
    pub fn new() -> Self {
        Self(Mutex::new(HashSet::new()))
    }

    fn create(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.0.lock().unwrap().insert(id.clone());
        id
    }

    fn is_valid(&self, id: &str) -> bool {
        self.0.lock().unwrap().contains(id)
    }

    fn remove(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}

// ── POST /mcp ─────────────────────────────────────────────────────────────────

pub async fn mcp_post(
    req: HttpRequest,
    body: web::Bytes,
    memory: web::Data<MemoryService>,
    sessions: web::Data<SessionStore>,
    auth: web::Data<AuthConfig>,
) -> HttpResponse {
    // 1. Auth / origin check
    if let Some(err) = check_auth(&req, &auth) {
        return err;
    }

    // 2. Protocol version check (SHOULD per spec)
    if let Some(v) = req.headers().get("MCP-Protocol-Version") {
        if v.to_str().unwrap_or("") != PROTOCOL_VERSION {
            return HttpResponse::BadRequest()
                .body(format!("unsupported MCP-Protocol-Version; expected {PROTOCOL_VERSION}"));
        }
    }

    // 3. Parse body
    let text = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return HttpResponse::BadRequest().body("body must be UTF-8"),
    };
    let raw: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let resp = Response::err(Value::Null, PARSE_ERROR, e.to_string());
            return HttpResponse::BadRequest().json(resp);
        }
    };

    // 4. Classify the message
    let has_method = raw.get("method").is_some();
    let has_id = raw.get("id").filter(|v| !v.is_null()).is_some();
    let is_response = !has_method && (raw.get("result").is_some() || raw.get("error").is_some());
    let is_notification = has_method && !has_id;

    if is_response || is_notification {
        return HttpResponse::Accepted().finish();
    }

    if !has_method {
        let resp = Response::err(Value::Null, INVALID_PARAMS, "missing 'method' field".to_string());
        return HttpResponse::BadRequest().json(resp);
    }

    // 5. Session check
    let method = raw["method"].as_str().unwrap_or("").to_string();
    let session_id = header_str(&req, "Mcp-Session-Id");

    if method.as_str() != "initialize" {
        match session_id.as_deref() {
            Some(id) if sessions.is_valid(id) => {}
            Some(_) => return HttpResponse::NotFound().body("session not found or expired"),
            None => return HttpResponse::BadRequest().body("Mcp-Session-Id required"),
        }
    }

    // 6. Deserialise and dispatch
    let mcp_req: Request = match serde_json::from_value(raw) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response::err(Value::Null, PARSE_ERROR, e.to_string());
            return HttpResponse::BadRequest().json(resp);
        }
    };

    let Some(mcp_resp) = handle(&mcp_req, &memory).await else {
        return HttpResponse::Accepted().finish();
    };

    // 7. On successful initialize, issue a new session ID
    let mut builder = HttpResponse::Ok();
    builder.content_type("application/json");

    if method.as_str() == "initialize" && mcp_resp.error.is_none() {
        let sid = sessions.create();
        builder.insert_header(("Mcp-Session-Id", sid));
    }

    builder.json(mcp_resp)
}

// ── GET /mcp ──────────────────────────────────────────────────────────────────

pub async fn mcp_get(req: HttpRequest, auth: web::Data<AuthConfig>) -> HttpResponse {
    if let Some(err) = check_auth(&req, &auth) {
        return err;
    }
    HttpResponse::MethodNotAllowed()
        .insert_header(("Allow", "POST, DELETE"))
        .finish()
}

// ── DELETE /mcp ───────────────────────────────────────────────────────────────

pub async fn mcp_delete(
    req: HttpRequest,
    sessions: web::Data<SessionStore>,
    auth: web::Data<AuthConfig>,
) -> HttpResponse {
    if let Some(err) = check_auth(&req, &auth) {
        return err;
    }
    if let Some(id) = header_str(&req, "Mcp-Session-Id") {
        sessions.remove(&id);
    }
    HttpResponse::Ok().finish()
}

// ── auth helpers ──────────────────────────────────────────────────────────────

/// Returns `Some(error response)` if the request fails auth, `None` if it passes.
fn check_auth(req: &HttpRequest, auth: &AuthConfig) -> Option<HttpResponse> {
    match auth {
        AuthConfig::LocalOnly => check_origin(req),

        AuthConfig::Bearer(expected) => {
            if check_bearer(req, expected) {
                None
            } else {
                Some(
                    HttpResponse::Unauthorized()
                        .insert_header(("WWW-Authenticate", "Bearer"))
                        .body("invalid or missing Authorization header"),
                )
            }
        }

        AuthConfig::Oidc(verifier) => {
            let token = match extract_bearer(req) {
                Some(t) => t,
                None => {
                    return Some(
                        HttpResponse::Unauthorized()
                            .insert_header(("WWW-Authenticate", "Bearer"))
                            .body("Authorization: Bearer <token> required"),
                    )
                }
            };
            match verifier.verify(&token) {
                Ok(()) => None,
                Err(e) => Some(
                    HttpResponse::Unauthorized()
                        .insert_header(("WWW-Authenticate", "Bearer"))
                        .body(format!("token rejected: {e}")),
                ),
            }
        }
    }
}

/// Extract the raw token string from `Authorization: Bearer <token>`.
fn extract_bearer(req: &HttpRequest) -> Option<String> {
    let header = req.headers().get("Authorization")?.to_str().ok()?;
    header.strip_prefix("Bearer ").map(|t| t.to_string())
}

/// Check `Authorization: Bearer <token>` using constant-time comparison.
fn check_bearer(req: &HttpRequest, expected: &str) -> bool {
    let Some(token) = extract_bearer(req) else { return false };
    if token.len() != expected.len() {
        return false;
    }
    token.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Reject cross-origin requests (DNS rebinding protection) — used in LocalOnly mode.
/// Requests with no Origin header (e.g. curl, server-to-server) are always allowed.
fn check_origin(req: &HttpRequest) -> Option<HttpResponse> {
    let origin = req.headers().get("Origin")?.to_str().unwrap_or("").to_string();
    if ["localhost", "127.0.0.1", "::1"].iter().any(|h| origin.contains(h)) {
        None
    } else {
        Some(HttpResponse::Forbidden().body(format!("Origin '{origin}' not allowed")))
    }
}

fn header_str(req: &HttpRequest, name: &str) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
