// MCP Streamable HTTP transport (protocol version 2025-06-18)
//
// Spec: https://spec.modelcontextprotocol.io/specification/2025-06-18/basic/transports/
//
// POST /mcp  — client→server JSON-RPC (requests, notifications, responses)
// GET  /mcp  — server→client SSE push (405: we have no server-initiated messages)
// DELETE /mcp — explicit session teardown

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use ulid::Ulid;

use crate::api::oidc::OidcVerifier;
use crate::indexer::IndexService;
use crate::memory::service::MemoryService;
use crate::mcp::protocol::{Request, Response, PARSE_ERROR, INVALID_PARAMS};
use crate::mcp::server::handle;
use std::sync::Mutex as StdMutex;

/// Auth configuration, chosen at startup based on environment variables.
pub enum AuthConfig {
    /// No auth — bind localhost, check Origin header (default).
    LocalOnly,
    /// Simple shared bearer token — bind 0.0.0.0, skip Origin check.
    Bearer(String),
    /// OIDC JWT verification — bind 0.0.0.0, skip Origin check.
    Oidc(StdMutex<OidcVerifier>),
}

impl AuthConfig {
    /// Returns true when the server should bind to all interfaces.
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::LocalOnly)
    }

    /// Build auth config from MemoryConfig (synchronous subset — OIDC requires
    /// async initialisation and is handled in main.rs).
    pub fn from_config(config: &crate::config::MemoryConfig) -> Result<Self, String> {
        if config.oidc_issuer.is_some() {
            Err("OIDC requires async initialisation".to_string())
        } else if let Some(token) = config.auth_token.clone() {
            Ok(Self::Bearer(token))
        } else {
            Ok(Self::LocalOnly)
        }
    }
}

const PROTOCOL_VERSION: &str = "2025-06-18";

// ── session store ─────────────────────────────────────────────────────────────

pub struct SessionStore {
    sessions: Arc<Mutex<HashMap<String, Instant>>>,
    ttl: Duration,
}

impl SessionStore {
    pub fn new(ttl_hours: u64) -> Self {
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        let ttl = Duration::from_secs(ttl_hours * 3600);
        let sessions_clone = Arc::clone(&sessions);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let now = Instant::now();
                let mut guard = sessions_clone.lock().await;
                guard.retain(|_, created| now.duration_since(*created) < ttl);
            }
        });
        Self { sessions, ttl }
    }

    async fn create(&self) -> String {
        let id = Ulid::new().to_string();
        self.sessions.lock().await.insert(id.clone(), Instant::now());
        id
    }

    async fn is_valid(&self, id: &str) -> bool {
        let guard = self.sessions.lock().await;
        match guard.get(id) {
            Some(created) => Instant::now().duration_since(*created) < self.ttl,
            None => false,
        }
    }

    async fn remove(&self, id: &str) {
        self.sessions.lock().await.remove(id);
    }
}

// ── POST /mcp ─────────────────────────────────────────────────────────────────

pub async fn mcp_post(
    req: HttpRequest,
    body: web::Bytes,
    memory: web::Data<MemoryService>,
    indexer: web::Data<IndexService>,
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
            Some(id) if sessions.is_valid(id).await => {}
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

    let Some(mcp_resp) = handle(&mcp_req, &memory, &indexer).await else {
        return HttpResponse::Accepted().finish();
    };

    // 7. On successful initialize, issue a new session ID
    let mut builder = HttpResponse::Ok();
    builder.content_type("application/json");

    if method.as_str() == "initialize" && mcp_resp.error.is_none() {
        let sid = sessions.create().await;
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
        sessions.remove(&id).await;
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
            match verifier.lock().unwrap().verify(&token) {
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
    // Constant-time comparison regardless of length
    let max_len = token.len().max(expected.len());
    if max_len == 0 {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..max_len {
        let a = token.bytes().nth(i).unwrap_or(0);
        let b = expected.bytes().nth(i).unwrap_or(0);
        diff |= a ^ b;
    }
    diff == 0 && token.len() == expected.len()
}

/// Reject cross-origin requests (DNS rebinding protection) — used in LocalOnly mode.
/// Requests with no Origin header (e.g. curl, server-to-server) are always allowed.
fn check_origin(req: &HttpRequest) -> Option<HttpResponse> {
    let origin = req.headers().get("Origin")?.to_str().unwrap_or("").to_string();
    let parsed = url::Url::parse(&origin).ok()?;
    let host = parsed.host_str().unwrap_or("");
    match host {
        "localhost" | "127.0.0.1" | "::1" => None,
        _ => Some(HttpResponse::Forbidden().body(format!("Origin '{origin}' not allowed"))),
    }
}

fn header_str(req: &HttpRequest, name: &str) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
