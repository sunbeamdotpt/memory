#![deny(dead_code)]
#![deny(unused)]
#![deny(unused_mut)]
#![deny(clippy::missing_safety_doc)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
// for @siennathesane's sanity and to make it clear the scope of error handling. and because it's
// super fucking subtle and i'll miss it in code reviews sorry not sorry
#![deny(clippy::question_mark_used)]
// just keeps syntax consistent
#![deny(clippy::needless_borrow)]

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use rmcp::ServiceExt;
use rmcp::model::{PingRequest, ServerRequest};
use sunbeam_memory::{
    config::MemoryConfig,
    connect::{memory_proto::sunbeam::memory::v1::MemoryServiceExt, service::MemoryConnectService},
    core::service::CoreService,
    mcp::server::SunbeamServer,
    memory::service::MemoryService,
};

#[derive(Parser, Debug)]
#[command(name = "sunbeam-memory")]
#[command(about = "Personal semantic memory server for AI assistants")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the MCP HTTP server
    Http(HttpArgs),
}

#[derive(Parser, Debug)]
struct HttpArgs {
    /// Port to bind the HTTP server to
    #[arg(long, short, default_value = "3456")]
    port: u16,
    /// Interval in seconds between SSE keep-alive comments. Overrides
    /// `MCP_SSE_KEEPALIVE_SECONDS`. Set to 0 to disable.
    #[arg(long)]
    sse_keepalive_seconds: Option<u64>,
    /// Idle timeout in seconds for MCP Streamable HTTP sessions. Overrides
    /// `MCP_SESSION_KEEPALIVE_SECONDS`. Set to 0 to disable.
    #[arg(long)]
    session_keepalive_seconds: Option<u64>,
}

// ── stdio MCP transport ───────────────────────────────────────────────────────

async fn run_stdio() -> Result<()> {
    let config = MemoryConfig::from_env();
    let core = match init_core(config.clone()).await {
        Ok(c) => c,
        Err(e) => return Err(e),
    };
    let indexer = core.indexer().clone();
    tokio::spawn(indexer.run());

    let server = SunbeamServer::new(core);
    let service = match server
        .serve(rmcp::transport::stdio())
        .await
        .context("failed to start MCP service")
    {
        Ok(s) => s,
        Err(e) => return Err(e),
    };

    if config.stdio_keepalive_seconds > 0 {
        tracing::info!(
            "stdio keepalive enabled: ping every {}s",
            config.stdio_keepalive_seconds
        );
        let peer = service.peer().clone();
        let interval = Duration::from_secs(config.stdio_keepalive_seconds);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if peer.is_transport_closed() {
                    break;
                }
                let ping = ServerRequest::PingRequest(PingRequest::default());
                if let Err(e) = peer.send_request(ping).await {
                    tracing::debug!("stdio keepalive ping failed: {e}");
                    break;
                }
            }
        });
    }

    match service.waiting().await {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::Error::from(e)),
    }
}

// ── HTTP server (axum + ConnectRPC + MCP Streamable HTTP) ─────────────────────

async fn run_http(args: HttpArgs) -> Result<()> {
    let mut config = MemoryConfig::from_env();
    if let Some(seconds) = args.sse_keepalive_seconds {
        config.sse_keepalive_seconds = seconds;
    }
    if let Some(seconds) = args.session_keepalive_seconds {
        config.session_keepalive_seconds = seconds;
    }
    tracing::info!(
        "sunbeam-memory {}: loading model and opening store at {}…",
        env!("CARGO_PKG_VERSION"),
        config.base_dir
    );

    let core = match init_core(config.clone()).await {
        Ok(c) => c,
        Err(e) => return Err(e),
    };
    let indexer = core.indexer().clone();
    tokio::spawn(indexer.run());

    let bind_addr: std::net::SocketAddr = match format!("127.0.0.1:{}", args.port)
        .parse()
        .context("invalid bind address")
    {
        Ok(a) => a,
        Err(e) => return Err(e),
    };

    let app = build_http_app(core, config);

    tracing::info!("sunbeam-memory ready (HTTP on {bind_addr})");
    tracing::info!("  ConnectRPC base: http://{bind_addr}");
    tracing::info!("  MCP endpoint:    http://{bind_addr}/mcp");

    let listener = match tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("cannot bind")
    {
        Ok(l) => l,
        Err(e) => return Err(e),
    };

    match axum::serve(listener, app).await.context("server error") {
        Ok(()) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Build the axum application for HTTP mode.
///
/// Exposes the ConnectRPC `MemoryService` routes, an MCP Streamable HTTP
/// endpoint at `/mcp`, and a simple root handler.
pub fn build_http_app(core: CoreService, config: MemoryConfig) -> axum::Router {
    // ConnectRPC adapter serves the generated MemoryService routes.
    let connect_service = Arc::new(MemoryConnectService::new(core.clone()));
    let connect_router = connect_service.register(connectrpc::Router::new());

    // MCP Streamable HTTP transport at /mcp, backed by the same tool server
    // that stdio mode uses.
    let mcp_core = core.clone();
    let mut session_manager =
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default();
    session_manager.session_config.keep_alive = (config.session_keepalive_seconds > 0)
        .then(|| std::time::Duration::from_secs(config.session_keepalive_seconds));
    let session_manager = Arc::new(session_manager);

    let mut mcp_config =
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default();
    mcp_config.sse_keep_alive = (config.sse_keepalive_seconds > 0)
        .then(|| std::time::Duration::from_secs(config.sse_keepalive_seconds));

    let mcp_service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || Ok::<_, std::io::Error>(SunbeamServer::new(mcp_core.clone())),
        session_manager,
        mcp_config,
    );

    connect_router
        .into_axum_router()
        .route("/", axum::routing::get(|| async { "sunbeam-memory" }))
        .route_service("/mcp", mcp_service)
}

// ── shared init ───────────────────────────────────────────────────────────────

pub(crate) async fn init_core(config: MemoryConfig) -> Result<CoreService> {
    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);

    let memory = match MemoryService::new(&config).await.with_context(|| {
        format!(
            "failed to initialise memory service (base_dir: {})",
            config.base_dir
        )
    }) {
        Ok(m) => m,
        Err(e) => return Err(e),
    };

    let watcher = match sunbeam_memory::indexer::IndexWatcher::new(event_tx)
        .context("failed to create file watcher")
    {
        Ok(w) => w,
        Err(e) => return Err(e),
    };
    let indexer = sunbeam_memory::indexer::IndexService::new(memory.clone(), event_rx, watcher);

    Ok(CoreService::new(memory, indexer))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Http(args)) => run_http(args).await,
        None => run_stdio().await,
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    use sunbeam_memory::{
        config::MemoryConfig,
        core::service::CoreService,
        indexer::{IndexService, IndexWatcher},
        memory::service::MemoryService,
    };

    use super::build_http_app;

    async fn setup() -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            base_dir: dir.path().to_str().unwrap().to_string(),
            ..Default::default()
        };
        let memory = MemoryService::new(&config).await.unwrap();
        let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
        let watcher = IndexWatcher::new(dummy_tx).unwrap();
        let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
        let core = CoreService::new(memory, indexer);
        (build_http_app(core, config), dir)
    }

    fn uint64_value(v: &serde_json::Value) -> u64 {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0)
    }

    async fn send_json(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (http::StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn body_text(resp: axum::response::Response<Body>) -> String {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn test_root_handler() {
        let (app, _dir) = setup().await;
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(body_text(resp).await, "sunbeam-memory");
    }

    #[tokio::test]
    async fn test_health_check() {
        let (app, _dir) = setup().await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/HealthCheck",
            json!({}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn test_store_fact() {
        let (app, _dir) = setup().await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "hello world", "namespace": "docs"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(!json["fact"]["id"].as_str().unwrap().is_empty());
        assert_eq!(json["fact"]["namespace"], "docs");
    }

    #[tokio::test]
    async fn test_store_fact_invalid_argument() {
        let (app, _dir) = setup().await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "   "}),
        )
        .await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert!(json["code"].as_str().unwrap().contains("invalid_argument"));
    }

    #[tokio::test]
    async fn test_search_facts() {
        let (app, _dir) = setup().await;
        send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "Rust programming language"}),
        )
        .await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/SearchFacts",
            json!({"query": "Rust", "limit": 5}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) > 0);
    }

    #[tokio::test]
    async fn test_update_and_delete_fact() {
        let (app, _dir) = setup().await;
        let (_, store) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "initial content"}),
        )
        .await;
        let id = store["fact"]["id"].as_str().unwrap();

        let (status, update) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/UpdateFact",
            json!({"id": id, "content": "updated content"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(update["fact"]["content"], "updated content");

        let (status, delete) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/DeleteFact",
            json!({"id": id}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(delete["deleted"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_list_facts() {
        let (app, _dir) = setup().await;
        send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "fact one", "namespace": "ns1"}),
        )
        .await;
        send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "fact two", "namespace": "ns1"}),
        )
        .await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ListFacts",
            json!({"namespace": "ns1"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) >= 2);
    }

    #[tokio::test]
    async fn test_urn_tools() {
        let (app, _dir) = setup().await;
        let (status, build) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/BuildSourceUrn",
            json!({"content_type": "code", "origin": "fs", "locator": "/tmp/main.rs", "fragment": "L10-L20"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        let urn = build["urn"].as_str().unwrap();
        assert!(urn.contains("urn:smem:code:fs:/tmp/main.rs#L10-L20"));

        let (status, parse) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ParseSourceUrn",
            json!({"urn": urn}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(parse["valid"].as_bool().unwrap());

        let (status, schema) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/DescribeUrnSchema",
            json!({}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(
            schema["schemaJson"]
                .as_str()
                .unwrap()
                .contains("content_types")
        );
    }

    #[tokio::test]
    async fn test_watch_target_lifecycle() {
        let (app, dir) = setup().await;
        let path = dir.path().to_str().unwrap();
        let (status, add) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/AddWatchTarget",
            json!({"path": path, "namespace": "default", "target_type": "directory"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(add["targetIds"].as_array().unwrap().len(), 1);
        let target_id = add["targetIds"][0].as_str().unwrap();

        let (status, list) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ListWatchTargets",
            json!({}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&list["total"]) > 0);

        let (status, _progress) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/GetIndexProgress",
            json!({"target_id": target_id}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let (status, sync) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/SyncWatchTarget",
            json!({"target_id": target_id}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(sync["started"].as_bool().unwrap());

        let (status, remove) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/RemoveWatchTarget",
            json!({"target_id": target_id}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(remove["removed"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_errors_api() {
        let (app, _dir) = setup().await;
        let (status, list) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/GetRecentErrors",
            json!({}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(
            list.get("errors")
                .map(|e| e.as_array().unwrap().is_empty())
                .unwrap_or(true)
        );

        let (status, resolved) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ResolveError",
            json!({"error_id": "01ABCDEF0123456789ABCDEF01"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(
            !resolved
                .get("resolved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn test_mcp_initialize_streamable_http() {
        let (app, _dir) = setup().await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1.0"}
            }
        });
        let req = Request::builder()
            .method("POST")
            .uri("http://127.0.0.1/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
        let text = body_text(resp).await;
        assert!(text.contains("sunbeam-memory"));
        assert!(text.contains("\"result\""));
    }

    #[tokio::test]
    async fn test_mcp_requires_both_accept_mime_types() {
        let (app, _dir) = setup().await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1.0"}
            }
        });
        let req = Request::builder()
            .method("POST")
            .uri("http://127.0.0.1/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_ACCEPTABLE);
    }

    #[tokio::test]
    async fn test_init_core() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            base_dir: dir.path().to_str().unwrap().to_string(),
            ..Default::default()
        };
        let core = crate::init_core(config).await.unwrap();
        // Ensure the returned service is fully usable.
        let _ = core.memory().embedding_service();
    }

    #[tokio::test]
    async fn test_update_fact_not_found() {
        let (app, _dir) = setup().await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/UpdateFact",
            json!({"id": "01ABCDEF0123456789ABCDEF01", "content": "updated"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::NOT_FOUND);
        assert!(json["code"].as_str().unwrap().contains("not_found"));
    }

    #[tokio::test]
    async fn test_search_facts_default_limit() {
        let (app, _dir) = setup().await;
        for i in 0..3 {
            send_json(
                &app,
                "POST",
                "/sunbeam.memory.v1.MemoryService/StoreFact",
                json!({"content": format!("default limit fact {}", i), "namespace": "limits"}),
            )
            .await;
        }
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/SearchFacts",
            json!({"query": "default limit", "limit": 0}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) >= 3);
    }

    #[tokio::test]
    async fn test_list_facts_default_limit() {
        let (app, _dir) = setup().await;
        for i in 0..3 {
            send_json(
                &app,
                "POST",
                "/sunbeam.memory.v1.MemoryService/StoreFact",
                json!({"content": format!("list default fact {}", i), "namespace": "listlimits"}),
            )
            .await;
        }
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ListFacts",
            json!({"namespace": "listlimits", "limit": 0}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) >= 3);
    }

    #[tokio::test]
    async fn test_get_recent_errors_and_index_progress() {
        let (app, _dir) = setup().await;

        // Trigger a background indexer error so GetRecentErrors has something to return.
        send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/SyncWatchTarget",
            json!({"target_id": "01ABCDEF0123456789ABCDEF01"}),
        )
        .await;

        let mut found = false;
        for _ in 0..50 {
            let (status, json) = send_json(
                &app,
                "POST",
                "/sunbeam.memory.v1.MemoryService/GetRecentErrors",
                json!({"limit": 0}),
            )
            .await;
            assert_eq!(status, http::StatusCode::OK);
            if json["errors"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false)
            {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(found, "expected a logged indexer error");

        // Call GetIndexProgress; conversion is exercised when progress is Some.
        let (status, _json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/GetIndexProgress",
            json!({"target_id": "01ABCDEF0123456789ABCDEF01"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_restore_stale_fact_handler() {
        let (app, _dir) = setup().await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/RestoreStaleFact",
            json!({"id": "01ABCDEF0123456789ABCDEF01"}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        let restored = json
            .get("restored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(!restored);
    }

    #[tokio::test]
    async fn test_init_core_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file at the base_dir path so the store cannot create a directory there.
        let bad_path = dir.path().join("not_a_dir");
        std::fs::write(&bad_path, "").unwrap();
        let config = MemoryConfig {
            base_dir: bad_path.to_str().unwrap().to_string(),
            ..Default::default()
        };
        let result = crate::init_core(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_facts_default_namespace() {
        let (app, _dir) = setup().await;
        send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/StoreFact",
            json!({"content": "namespace default test"}),
        )
        .await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ListFacts",
            json!({"namespace": ""}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) >= 1);
    }

    #[tokio::test]
    async fn test_search_facts_nonzero_limit() {
        let (app, _dir) = setup().await;
        for i in 0..3 {
            send_json(
                &app,
                "POST",
                "/sunbeam.memory.v1.MemoryService/StoreFact",
                json!({"content": format!("limit fact {}", i), "namespace": "limitns"}),
            )
            .await;
        }
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/SearchFacts",
            json!({"query": "limit fact", "limit": 2}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) >= 1);
    }

    #[tokio::test]
    async fn test_list_facts_nonzero_limit() {
        let (app, _dir) = setup().await;
        for i in 0..3 {
            send_json(
                &app,
                "POST",
                "/sunbeam.memory.v1.MemoryService/StoreFact",
                json!({"content": format!("list limit fact {}", i), "namespace": "listns"}),
            )
            .await;
        }
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/ListFacts",
            json!({"namespace": "listns", "limit": 2}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(uint64_value(&json["total"]) >= 1);
    }

    #[tokio::test]
    async fn test_add_watch_target_default_namespace() {
        let (app, dir) = setup().await;
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/AddWatchTarget",
            json!({"path": dir.path().to_str().unwrap(), "namespace": ""}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(!json["targetIds"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_build_http_app_with_disabled_keepalive() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            base_dir: dir.path().to_str().unwrap().to_string(),
            sse_keepalive_seconds: 0,
            session_keepalive_seconds: 0,
            ..MemoryConfig::default()
        };
        let memory = MemoryService::new(&config).await.unwrap();
        let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
        let watcher = IndexWatcher::new(dummy_tx).unwrap();
        let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
        let core = CoreService::new(memory, indexer);
        // Should build without panicking and a health check should still work.
        let app = build_http_app(core, config);
        let (status, json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/HealthCheck",
            json!({}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_build_http_app_with_custom_keepalive() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            base_dir: dir.path().to_str().unwrap().to_string(),
            sse_keepalive_seconds: 60,
            session_keepalive_seconds: 3600,
            ..MemoryConfig::default()
        };
        let memory = MemoryService::new(&config).await.unwrap();
        let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
        let watcher = IndexWatcher::new(dummy_tx).unwrap();
        let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
        let core = CoreService::new(memory, indexer);
        let app = build_http_app(core, config);
        let (status, _json) = send_json(
            &app,
            "POST",
            "/sunbeam.memory.v1.MemoryService/HealthCheck",
            json!({}),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
    }

    #[test]
    fn test_http_args_keepalive_flags() {
        use clap::Parser;

        let args = super::HttpArgs::parse_from(["sunbeam-memory", "--sse-keepalive-seconds", "60"]);
        assert_eq!(args.sse_keepalive_seconds, Some(60));
        assert_eq!(args.session_keepalive_seconds, None);

        let args =
            super::HttpArgs::parse_from(["sunbeam-memory", "--session-keepalive-seconds", "3600"]);
        assert_eq!(args.session_keepalive_seconds, Some(3600));
    }
}
