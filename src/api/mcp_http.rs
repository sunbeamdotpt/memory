// MCP Streamable HTTP transport using rmcp's native implementation.
//
// Auth is enforced via an actix-web middleware so that rmcp handles the
// protocol layer and we never write raw JSON-RPC.

use actix_web::{
    body::BoxBody,
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    web, HttpResponse,
};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp_actix_web::transport::StreamableHttpService;
use std::sync::Arc;

use crate::indexer::IndexService;
use crate::memory::service::MemoryService;
use crate::mcp::server::SunbeamServer;

// ── auth configuration ────────────────────────────────────────────────────────

pub enum AuthConfig {
    /// No auth — bind localhost, check Origin header (default).
    LocalOnly,
    /// Simple shared bearer token — bind 0.0.0.0, skip Origin check.
    Bearer(String),
    /// OIDC JWT verification — bind 0.0.0.0, skip Origin check.
    Oidc(std::sync::Mutex<crate::api::oidc::OidcVerifier>),
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

// ── auth middleware ───────────────────────────────────────────────────────────

pub async fn auth_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let auth = req
        .app_data::<web::Data<AuthConfig>>()
        .expect("AuthConfig not configured");

    if let Some(response) = check_auth(req.request(), auth) {
        return Ok(req.into_response(response));
    }

    next.call(req).await
}

// ── MCP service factory ───────────────────────────────────────────────────────

pub type McpHttpService = StreamableHttpService<SunbeamServer, LocalSessionManager>;

pub fn build_mcp_service(memory: MemoryService, indexer: IndexService) -> McpHttpService {
    StreamableHttpService::builder()
        .service_factory(Arc::new(move || {
            Ok(SunbeamServer::new(memory.clone(), indexer.clone()))
        }))
        .session_manager(Arc::new(LocalSessionManager::default()))
        .stateful_mode(true)
        .build()
}

// ── auth helpers ──────────────────────────────────────────────────────────────

fn check_auth(req: &actix_web::HttpRequest, auth: &AuthConfig) -> Option<HttpResponse> {
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
fn extract_bearer(req: &actix_web::HttpRequest) -> Option<String> {
    let header = req.headers().get("Authorization")?.to_str().ok()?;
    header.strip_prefix("Bearer ").map(|t| t.to_string())
}

/// Check `Authorization: Bearer <token>` using constant-time comparison.
fn check_bearer(req: &actix_web::HttpRequest, expected: &str) -> bool {
    let Some(token) = extract_bearer(req) else { return false };
    // Constant-time comparison regardless of length
    let max_len = token.len().max(expected.len());
    if max_len == 0 {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..max_len {
        let a = token.as_bytes().get(i).copied().unwrap_or(0);
        let b = expected.as_bytes().get(i).copied().unwrap_or(0);
        diff |= a ^ b;
    }
    diff == 0 && token.len() == expected.len()
}

/// Reject cross-origin requests (DNS rebinding protection) — used in LocalOnly mode.
/// Requests with no Origin header (e.g. curl, server-to-server) are always allowed.
fn check_origin(req: &actix_web::HttpRequest) -> Option<HttpResponse> {
    let origin = req.headers().get("Origin")?.to_str().unwrap_or("").to_string();
    let parsed = url::Url::parse(&origin).ok()?;
    let host = parsed.host_str().unwrap_or("");
    match host {
        "localhost" | "127.0.0.1" | "::1" => None,
        _ => Some(HttpResponse::Forbidden().body(format!("Origin '{origin}' not allowed"))),
    }
}
