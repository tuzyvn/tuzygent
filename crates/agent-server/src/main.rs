use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::prelude::*;
use axum::{
    http::{HeaderValue, Method},
    middleware,
    routing::{delete, get, post},
    Router,
};
use anyhow::Context;
use clap::Parser;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

pub mod auth;
pub mod dashboard_html;
pub mod gateway;
pub mod handlers;
pub mod state;

pub use auth::{require_auth, token_matches, AuthState};
pub use gateway::*;
pub use handlers::*;
pub use state::{AppState, ServerCli};

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
    let allowed_origins: Vec<HeaderValue> = cli
        .allowed_origins
        .iter()
        .map(|origin| origin.parse())
        .collect::<Result<_, _>>()?;
    let auth = AuthState {
        token: Arc::new(cli.api_token),
        allowed_origins: allowed_origins.clone(),
    };

    let bus = InterAgentBus::new(200);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));
    let vault = Arc::new(agent_engine::AccountVault::new());
    let task_store = match agent_runtime::SqliteTaskStore::new(&cli.workspace) {
        Ok(store) => store,
        Err(err) => {
            warn!(
                "Failed to initialize SQLite task store at {:?}: {err}, falling back to in-memory store",
                cli.workspace
            );
            agent_runtime::SqliteTaskStore::memory()
                .context("Failed to initialize fallback in-memory SQLite task store")?
        }
    };
    let task_store = Arc::new(task_store);
    let task_service = Arc::new(agent_runtime::TaskService::new(
        task_store,
        &cli.workspace,
        4,
        None,
        None,
    ));

    let state = AppState {
        supervisor,
        bus,
        workspace: cli.workspace,
        vault,
        task_service,
    };

    let app = build_app(state, auth);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    info!("🚀 AgentCore Daemon Server listening on http://{}", addr);
    info!("🤖 OpenAI-Compatible Gateway active at http://{}/v1/chat/completions", addr);
    info!("🌐 Web Account Management Dashboard active at http://{}/dashboard", addr);
    info!("📡 WebSocket stream active at ws://{}/api/v1/ws", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

pub fn build_app(state: AppState, auth: AuthState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(auth.allowed_origins.clone())
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);

    let gateway = Router::new()
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/models", get(list_models_handler))
        .route(
            "/api/v1/auth/accounts",
            get(list_accounts_handler).post(add_account_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}",
            delete(remove_account_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}/reset",
            post(reset_account_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}/import-local",
            post(import_local_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}/verify",
            post(verify_account_handler),
        );

    let api = Router::new()
        .route("/api/v1/engines", get(list_engines_handler))
        .route(
            "/api/v1/agents",
            get(list_agents_handler).post(create_agent_handler),
        )
        .route("/api/v1/tasks/run", post(run_task_handler))
        .route(
            "/api/v1/tasks",
            post(submit_task_handler).get(list_tasks_handler),
        )
        .route("/api/v1/tasks/{id}", get(get_task_handler))
        .route("/api/v1/tasks/{id}/cancel", post(cancel_task_handler))
        .route("/api/v1/tasks/{id}/approve", post(approve_task_handler))
        .route("/api/v1/ws", get(ws_handler))
        .merge(gateway)
        .route_layer(middleware::from_fn_with_state(auth, require_auth));

    Router::new()
        .route("/", get(dashboard_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/health", get(health_handler))
        .merge(api)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests;
