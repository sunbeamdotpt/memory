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
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            base_dir: "./data/memory".to_string(),
            auth_token: None,
            oidc_issuer: None,
            oidc_audience: None,
        }
    }
}

impl MemoryConfig {
    pub fn from_env() -> Self {
        Self {
            base_dir: std::env::var("MCP_MEMORY_BASE_DIR")
                .unwrap_or_else(|_| "./data/memory".to_string()),
            auth_token: std::env::var("MCP_AUTH_TOKEN").ok(),
            oidc_issuer: std::env::var("MCP_OIDC_ISSUER").ok(),
            oidc_audience: std::env::var("MCP_OIDC_AUDIENCE").ok(),
        }
    }
}
