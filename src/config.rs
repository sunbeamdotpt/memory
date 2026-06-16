#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub base_dir: String,
    /// Simple bearer token auth. Set `MCP_AUTH_TOKEN` for single-user remote hosting.
    /// Mutually exclusive with `oidc_issuer` (OIDC takes priority if both are set).
    pub auth_token: Option<String>,
    /// OIDC issuer URL (e.g. `https://auth.example.com`). When set, the server fetches
    /// the JWKS at startup and validates JWT Bearer tokens on every request.
    /// Read from `MCP_OIDC_ISSUER`.
    pub oidc_issuer: Option<String>,
    /// Optional OIDC audience claim to validate. Read from `MCP_OIDC_AUDIENCE`.
    /// Leave unset to skip audience validation.
    pub oidc_audience: Option<String>,
    /// Interval in seconds between SSE keep-alive comments on Streamable HTTP
    /// connections. Defaults to 15 seconds; set to 0 to disable. Read from
    /// `MCP_SSE_KEEPALIVE_SECONDS`.
    pub sse_keepalive_seconds: u64,
    /// Idle timeout in seconds for MCP Streamable HTTP sessions. A session is
    /// closed after this duration without any activity. Defaults to 300 seconds
    /// (5 minutes); set to 0 to disable. Read from `MCP_SESSION_KEEPALIVE_SECONDS`.
    pub session_keepalive_seconds: u64,
    /// Interval in seconds between protocol-level `ping` requests sent over the
    /// stdio transport. Defaults to 30 seconds; set to 0 to disable. Read from
    /// `MCP_STDIO_KEEPALIVE_SECONDS`.
    pub stdio_keepalive_seconds: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            base_dir: crate::paths::data_dir()
                .join("memory")
                .to_string_lossy()
                .to_string(),
            auth_token: None,
            oidc_issuer: None,
            oidc_audience: None,
            sse_keepalive_seconds: 15,
            session_keepalive_seconds: 300,
            stdio_keepalive_seconds: 30,
        }
    }
}

impl MemoryConfig {
    pub fn from_env() -> Self {
        Self {
            base_dir: std::env::var("MCP_MEMORY_BASE_DIR").unwrap_or_else(|_| {
                crate::paths::data_dir()
                    .join("memory")
                    .to_string_lossy()
                    .to_string()
            }),
            auth_token: std::env::var("MCP_AUTH_TOKEN").ok(),
            oidc_issuer: std::env::var("MCP_OIDC_ISSUER").ok(),
            oidc_audience: std::env::var("MCP_OIDC_AUDIENCE").ok(),
            sse_keepalive_seconds: parse_env_u64("MCP_SSE_KEEPALIVE_SECONDS", 15),
            session_keepalive_seconds: parse_env_u64("MCP_SESSION_KEEPALIVE_SECONDS", 300),
            stdio_keepalive_seconds: parse_env_u64("MCP_STDIO_KEEPALIVE_SECONDS", 30),
        }
    }
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
