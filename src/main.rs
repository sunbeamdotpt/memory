use mcp_server::{
    config::MemoryConfig,
    memory::service::MemoryService,
    mcp::protocol::{Request, Response, PARSE_ERROR},
    mcp::server::handle,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // --http [PORT]  →  run HTTP REST server
    if let Some(pos) = args.iter().position(|a| a == "--http") {
        let port: u16 = args.get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(3456);
        run_http(port).await;
        return;
    }

    run_stdio().await;
}

// ── stdio MCP transport ───────────────────────────────────────────────────────

async fn run_stdio() {
    let config = MemoryConfig::from_env();

    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);

    let memory = init_memory(config).await;

    // Create file watcher and index service
    let watcher = mcp_server::indexer::IndexWatcher::new(event_tx).expect("failed to create file watcher");
    let indexer = mcp_server::indexer::IndexService::new(memory.clone(), event_rx, watcher);
    let indexer_for_mcp = indexer.clone();
    tokio::spawn(indexer.run());

    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                let response: Option<Response> = match serde_json::from_str::<Request>(&line) {
                    Ok(req)  => handle(&req, &memory, &indexer_for_mcp).await,
                    Err(err) => Some(Response::err(Value::Null, PARSE_ERROR, err.to_string())),
                };

                if let Some(resp) = response {
                    match serde_json::to_string(&resp) {
                        Ok(mut json) => {
                            json.push('\n');
                            let _ = stdout.write_all(json.as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                        Err(e) => {
                            let err_id = ulid::Ulid::new().to_string();
                            let _ = memory.log_error(&err_id, "mcp", "error", "failed to serialize response", Some(&e.to_string()));
                            let fallback = r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal serialization error"}}"#;
                            let _ = stdout.write_all(fallback.as_bytes()).await;
                            let _ = stdout.write_all(b"\n").await;
                            let _ = stdout.flush().await;
                        }
                    }
                }
            }
            Ok(None) => break, // EOF
            Err(e) => {
                let err_id = ulid::Ulid::new().to_string();
                let _ = memory.log_error(&err_id, "mcp", "error", "stdin read error", Some(&e.to_string()));
                let err_resp = Response::err(Value::Null, PARSE_ERROR, format!("stdin error: {e}"));
                if let Ok(mut json) = serde_json::to_string(&err_resp) {
                    json.push('\n');
                    let _ = stdout.write_all(json.as_bytes()).await;
                    let _ = stdout.flush().await;
                }
                break;
            }
        }
    }
}

// ── HTTP REST server ──────────────────────────────────────────────────────────

async fn run_http(port: u16) {
    use actix_web::{web, App, HttpServer};
    use mcp_server::api::config::configure_api;
    use mcp_server::api::mcp_http::{AuthConfig, SessionStore};
    use mcp_server::api::oidc::OidcVerifier;

    let config = MemoryConfig::from_env();
    eprintln!(
        "sunbeam-memory {}: loading model and opening store at {}…",
        env!("CARGO_PKG_VERSION"),
        config.base_dir
    );

    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);

    // Build auth config inside the tokio runtime
    let auth_config = if let Some(issuer) = config.oidc_issuer.clone() {
        eprintln!("  fetching OIDC JWKS from {issuer}…");
        match OidcVerifier::new(&issuer, config.oidc_audience.clone()).await {
            Ok(v) => {
                eprintln!("  OIDC ready (issuer: {issuer})");
                AuthConfig::Oidc(std::sync::Mutex::new(v))
            }
            Err(e) => {
                eprintln!("fatal: {e}");
                std::process::exit(1);
            }
        }
    } else if let Some(token) = config.auth_token.clone() {
        AuthConfig::Bearer(token.clone())
    } else {
        AuthConfig::LocalOnly
    };

    let bind_addr = if auth_config.is_remote() { "0.0.0.0" } else { "127.0.0.1" };

    match &auth_config {
        AuthConfig::LocalOnly => {
            eprintln!("sunbeam-memory ready (MCP HTTP on {bind_addr}:{port}, localhost only)");
            eprintln!("  MCP endpoint: http://127.0.0.1:{port}/mcp");
        }
        AuthConfig::Bearer(tok) => {
            eprintln!("sunbeam-memory ready (MCP HTTP on {bind_addr}:{port}, bearer auth)");
            eprintln!("  MCP endpoint: http://<your-host>:{port}/mcp");
            eprintln!("  Token:        {tok}");
        }
        AuthConfig::Oidc(_) => {
            eprintln!("sunbeam-memory ready (MCP HTTP on {bind_addr}:{port}, OIDC auth)");
            eprintln!("  MCP endpoint: http://<your-host>:{port}/mcp");
        }
    }

    let session_ttl = config.session_ttl_hours;
    let memory = init_memory(config).await;

    // Create file watcher and index service
    let watcher = mcp_server::indexer::IndexWatcher::new(event_tx).expect("failed to create file watcher");
    let indexer = mcp_server::indexer::IndexService::new(memory.clone(), event_rx, watcher);
    let indexer_data = web::Data::new(indexer.clone());
    tokio::spawn(indexer.run());

    let memory_data = web::Data::new(memory);
    let sessions = web::Data::new(SessionStore::new(session_ttl));
    let auth = web::Data::new(auth_config);

    HttpServer::new(move || {
        App::new()
            .app_data(memory_data.clone())
            .app_data(indexer_data.clone())
            .app_data(sessions.clone())
            .app_data(auth.clone())
            .configure(configure_api)
    })
    .bind((bind_addr, port))
    .unwrap_or_else(|e| { eprintln!("fatal: cannot bind {bind_addr}:{port}: {e}"); std::process::exit(1) })
    .run()
    .await
    .unwrap_or_else(|e| eprintln!("fatal: server error: {e}"));
}

// ── shared init ───────────────────────────────────────────────────────────────

async fn init_memory(config: MemoryConfig) -> MemoryService {
    match MemoryService::new(&config).await {
        Ok(svc) => svc,
        Err(e) => {
            eprintln!("fatal: failed to initialise memory service: {e}");
            std::process::exit(1);
        }
    }
}
