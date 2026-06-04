use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use sunbeam_memory::{
    config::MemoryConfig,
    core::service::CoreService,
    memory::service::MemoryService,
    mcp::server::SunbeamServer,
};
use rmcp::ServiceExt;

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
}

// ── stdio MCP transport ───────────────────────────────────────────────────────

async fn run_stdio() -> Result<()> {
    let config = MemoryConfig::from_env();

    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);

    let memory = init_memory(config).await?;

    let watcher =
        sunbeam_memory::indexer::IndexWatcher::new(event_tx).context("failed to create file watcher")?;
    let indexer = sunbeam_memory::indexer::IndexService::new(memory.clone(), event_rx, watcher);
    let core = CoreService::new(memory, indexer.clone());
    let server = SunbeamServer::new(core);
    tokio::spawn(indexer.run());

    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .context("failed to start MCP service")?;

    service.waiting().await?;
    Ok(())
}

// ── HTTP server (axum + g2v) ──────────────────────────────────────────────────

async fn run_http(args: HttpArgs) -> Result<()> {
    let config = MemoryConfig::from_env();
    tracing::info!(
        "sunbeam-memory {}: loading model and opening store at {}…",
        env!("CARGO_PKG_VERSION"),
        config.base_dir
    );

    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);

    let memory = init_memory(config).await?;

    let watcher =
        sunbeam_memory::indexer::IndexWatcher::new(event_tx).context("failed to create file watcher")?;
    let indexer = sunbeam_memory::indexer::IndexService::new(memory.clone(), event_rx, watcher);
    tokio::spawn(indexer.run());

    let bind_addr: std::net::SocketAddr = format!("127.0.0.1:{}", args.port).parse()
        .context("invalid bind address")?;

    tracing::info!("sunbeam-memory ready (MCP HTTP on {bind_addr})");
    tracing::info!("  MCP endpoint: http://{bind_addr}/mcp");

    // TODO: wire up axum router with MCP + ConnectRPC + health + metrics
    // This is a placeholder to allow compilation during the migration.
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("cannot bind")?;

    // Minimal axum app — will be replaced with full router in Phase 2
    let app = axum::Router::new()
        .route("/", axum::routing::get(|| async { "sunbeam-memory" }));

    axum::serve(listener, app)
        .await
        .context("server error")?;

    Ok(())
}

// ── shared init ───────────────────────────────────────────────────────────────

async fn init_memory(config: MemoryConfig) -> Result<MemoryService> {
    MemoryService::new(&config)
        .await
        .with_context(|| format!("failed to initialise memory service (base_dir: {})", config.base_dir))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Http(args)) => run_http(args).await,
        None => run_stdio().await,
    }
}
