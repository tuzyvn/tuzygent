use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::prelude::*;
use anyhow::Context;
use clap::Parser;
use tracing::info;

use agent_server::{build_app, AppState, AuthState, ServerCli};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,agent_server=debug".into()),
        )
        .init();

    let cli = ServerCli::parse();
    anyhow::ensure!(
        !cli.api_token.trim().is_empty(),
        "TUZYGENT_API_TOKEN must not be empty"
    );
    let allowed_origins: Vec<axum::http::HeaderValue> = cli
        .allowed_origins
        .iter()
        .map(|origin| origin.parse())
        .collect::<Result<_, _>>()?;
    let auth = AuthState::new(Arc::new(cli.api_token), allowed_origins);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let bus = InterAgentBus::new(200);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));
    let vault = Arc::new(agent_engine::AccountVault::new());
    let storage_dir = std::path::Path::new(&cli.workspace).join("storage");
    std::fs::create_dir_all(&storage_dir)?;
    let daemon_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(storage_dir.join("daemon.lock"))?;
    daemon_lock
        .try_lock()
        .context("Another daemon owns this workspace's task store")?;
    let task_store = agent_runtime::SqliteTaskStore::new(&cli.workspace)
        .context("Failed to initialize durable task store")?;
    let task_store = Arc::new(task_store);
    let engines_config =
        EnginesConfig::load_hierarchical(Some(std::path::Path::new(&cli.workspace)));
    let sandbox_config = SandboxConfig::from_engines_config(&engines_config)
        .context("Invalid sandbox configuration")?;
    let task_service = Arc::new(agent_runtime::TaskService::new(
        task_store,
        &cli.workspace,
        4,
        None,
        Some(sandbox_config),
    ));
    task_service
        .recover_after_restart()
        .await
        .context("Failed to recover task state")?;

    let state = AppState {
        supervisor,
        bus,
        workspace: cli.workspace,
        vault,
        task_service,
        metrics: std::sync::Arc::new(agent_server::metrics::ResilientMetricsService::default()),
    };

    let app = build_app(state, auth);

    info!("🚀 AgentCore Daemon Server listening on http://{}", addr);
    info!(
        "🤖 OpenAI-Compatible Gateway active at http://{}/v1/chat/completions",
        addr
    );
    info!(
        "🌐 Web Account Management Dashboard active at http://{}/dashboard",
        addr
    );
    info!("📡 WebSocket stream active at ws://{}/api/v1/ws", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
