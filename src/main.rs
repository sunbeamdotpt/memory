use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use mcp_server::{
    config::MemoryConfig,
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
        mcp_server::indexer::IndexWatcher::new(event_tx).context("failed to create file watcher")?;
    let indexer = mcp_server::indexer::IndexService::new(memory.clone(), event_rx, watcher);
    let server = SunbeamServer::new(memory, indexer.clone());
    tokio::spawn(indexer.run());

    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .context("failed to start MCP service")?;

    service.waiting().await?;
    Ok(())
}

// ── HTTP REST server ──────────────────────────────────────────────────────────

async fn run_http(args: HttpArgs) -> Result<()> {
    use actix_web::{web, App, HttpServer};
    use mcp_server::api::config::configure_api;
    use mcp_server::api::mcp_http::{AuthConfig, build_mcp_service};
    use mcp_server::api::oidc::OidcVerifier;

    let config = MemoryConfig::from_env();
    eprintln!(
        "sunbeam-memory {}: loading model and opening store at {}…",
        env!("CARGO_PKG_VERSION"),
        config.base_dir
    );

    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);

    let auth_config = if let Some(issuer) = config.oidc_issuer.clone() {
        eprintln!("  fetching OIDC JWKS from {issuer}…");
        let verifier = OidcVerifier::new(&issuer, config.oidc_audience.clone())
            .await
            .with_context(|| format!("failed to fetch JWKS from {issuer}"))?;
        eprintln!("  OIDC ready (issuer: {issuer})");
        AuthConfig::Oidc(std::sync::Mutex::new(verifier))
    } else if let Some(token) = config.auth_token.clone() {
        AuthConfig::Bearer(token.clone())
    } else {
        AuthConfig::LocalOnly
    };

    let bind_addr = if auth_config.is_remote() {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };

    match &auth_config {
        AuthConfig::LocalOnly => {
            eprintln!("sunbeam-memory ready (MCP HTTP on {bind_addr}:{}, localhost only)", args.port);
            eprintln!("  MCP endpoint: http://127.0.0.1:{}/mcp", args.port);
        }
        AuthConfig::Bearer(tok) => {
            eprintln!("sunbeam-memory ready (MCP HTTP on {bind_addr}:{}, bearer auth)", args.port);
            eprintln!("  MCP endpoint: http://<your-host>:{}/mcp", args.port);
            eprintln!("  Token:        {tok}");
        }
        AuthConfig::Oidc(_) => {
            eprintln!("sunbeam-memory ready (MCP HTTP on {bind_addr}:{}, OIDC auth)", args.port);
            eprintln!("  MCP endpoint: http://<your-host>:{}/mcp", args.port);
        }
    }

    let memory = init_memory(config).await?;

    let watcher =
        mcp_server::indexer::IndexWatcher::new(event_tx).context("failed to create file watcher")?;
    let indexer = mcp_server::indexer::IndexService::new(memory.clone(), event_rx, watcher);
    let indexer_data = web::Data::new(indexer.clone());
    let mcp_service = build_mcp_service(memory.clone(), indexer.clone());
    tokio::spawn(indexer.run());

    let memory_data = web::Data::new(memory);
    let auth = web::Data::new(auth_config);
    let configure = configure_api(mcp_service);

    HttpServer::new(move || {
        App::new()
            .app_data(memory_data.clone())
            .app_data(indexer_data.clone())
            .app_data(auth.clone())
            .configure(configure.clone())
    })
    .bind((bind_addr, args.port))
    .with_context(|| format!("cannot bind {bind_addr}:{}", args.port))?
    .run()
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
