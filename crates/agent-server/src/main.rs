use agent_core::prelude::*;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "agent-server")]
#[command(about = "AgentCore Standalone REST & WebSocket Daemon Server")]
struct ServerCli {
    #[arg(short, long, default_value = "0.0.0.0")]
    host: String,

    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    #[arg(short, long, default_value = ".")]
    workspace: String,
}

#[derive(Clone)]
struct AppState {
    supervisor: Arc<AgentSupervisor>,
    bus: InterAgentBus,
    workspace: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateAgentReq {
    id: String,
    name: String,
    description: Option<String>,
    autonomy: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunTaskReq {
    agent_id: String,
    goal: String,
    engine: Option<String>,
    sandbox: Option<String>,
    persona_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    timestamp: chrono::DateTime<chrono::Utc>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,agent_server=debug".into()),
        )
        .init();

    let cli = ServerCli::parse();

    let bus = InterAgentBus::new(200);
    let supervisor = Arc::new(AgentSupervisor::new(bus.clone()));

    let state = AppState {
        supervisor,
        bus,
        workspace: cli.workspace,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/v1/engines", get(list_engines_handler))
        .route(
            "/api/v1/agents",
            get(list_agents_handler).post(create_agent_handler),
        )
        .route("/api/v1/tasks/run", post(run_task_handler))
        .route("/api/v1/ws", get(ws_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    info!("🚀 AgentCore Daemon Server listening on http://{}", addr);
    info!("📡 WebSocket stream active at ws://{}/api/v1/ws", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: "0.1.0",
        timestamp: chrono::Utc::now(),
    })
}

async fn list_agents_handler(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.supervisor.list_agents().await;
    let list: Vec<serde_json::Value> = agents
        .into_iter()
        .map(|(identity, agent_state)| {
            serde_json::json!({
                "id": identity.id.to_string(),
                "name": identity.name,
                "description": identity.description,
                "autonomy_level": format!("{:?}", identity.autonomy_level),
                "state": format!("{:?}", agent_state),
                "created_at": identity.created_at,
            })
        })
        .collect();

    Json(serde_json::json!({ "agents": list }))
}

async fn create_agent_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateAgentReq>,
) -> impl IntoResponse {
    let level = match payload.autonomy.as_deref().unwrap_or("full") {
        "supervised" => AutonomyLevel::Supervised,
        "shadow" => AutonomyLevel::ShadowMode,
        _ => AutonomyLevel::Full,
    };

    let identity = AgentIdentity {
        id: AgentId::from_str(&payload.id),
        name: payload.name,
        description: payload
            .description
            .unwrap_or_else(|| "Autonomous Digital Employee".to_string()),
        autonomy_level: level,
        created_at: chrono::Utc::now(),
    };

    state.supervisor.register_agent(identity.clone()).await;
    info!("✨ Registered agent via REST: {}", identity.id);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "agent_id": identity.id.to_string(),
            "name": identity.name,
        })),
    )
}

async fn list_engines_handler(State(state): State<AppState>) -> impl IntoResponse {
    let ws_path = std::path::Path::new(&state.workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let detected_driver = SandboxConfig::detect_best_driver();

    Json(serde_json::json!({
        "default_engine": config.defaults.engine,
        "default_timeout_seconds": config.defaults.timeout_seconds,
        "sandbox_driver_configured": config.defaults.sandbox_driver,
        "sandbox_driver_detected": detected_driver.to_string(),
        "engines": config.engines,
        "security": {
            "allowed_env_vars": config.security.allowed_env_vars,
            "mount_credentials": config.security.mount_credentials,
        }
    }))
}

async fn run_task_handler(
    State(state): State<AppState>,
    Json(payload): Json<RunTaskReq>,
) -> impl IntoResponse {
    let ws_path = std::path::Path::new(&state.workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let engine_name = payload.engine.as_deref().unwrap_or(&config.defaults.engine);

    let compute_engine: Arc<dyn ComputeEngine> = match engine_name {
        "claude" | "claude_code" => {
            let single = config.engines.get("claude");
            Arc::new(ClaudeCodeEngine::new(
                single.and_then(|s| s.binary_path.clone()),
                single.and_then(|s| s.model.clone()),
            ))
        }
        "omniroute" | "local" => Arc::new(OpenAiCompatibleEngine::new(
            "http://127.0.0.1:20128/v1",
            "omniroute",
            "local",
        )),
        "antigravity" | "agy" => {
            let single = config.engines.get("antigravity");
            Arc::new(AntigravityEngine::new(
                single.and_then(|s| s.binary_path.clone()),
                single.and_then(|s| s.model.clone()),
            ))
        }
        other => {
            if let Some(single) = config.engines.get(other) {
                Arc::new(GenericCliEngine::new(
                    other.to_string(),
                    single
                        .binary_path
                        .clone()
                        .unwrap_or_else(|| other.to_string()),
                    single.args.clone(),
                    single.env.clone(),
                    single.output_parser.clone(),
                ))
            } else {
                Arc::new(AntigravityEngine::new(None, None))
            }
        }
    };

    let scratchpad = ScratchpadManager::new(&state.workspace);
    let durable = DurableMemoryStore::new(&state.workspace);
    let guardrails = GuardrailPolicy::default_enterprise();

    let mut sandbox_cfg = SandboxConfig::default();
    if let Some(ref sb) = payload.sandbox {
        match sb.to_lowercase().as_str() {
            "docker" => sandbox_cfg.driver = SandboxDriverKind::Docker,
            "bubblewrap" | "bwrap" => sandbox_cfg.driver = SandboxDriverKind::Bubblewrap,
            "host" => sandbox_cfg.driver = SandboxDriverKind::Host,
            _ => {}
        }
    }

    let ooda = AutonomousOodaLoop::new(
        compute_engine,
        guardrails,
        scratchpad,
        durable,
        &state.workspace,
    )
    .with_sandbox_config(sandbox_cfg)
    .with_bus(state.bus.clone());

    let mut task = Task::new(&payload.goal, &payload.goal, Priority::High);
    task.assigned_agent = Some(AgentId::from_str(&payload.agent_id));

    match ooda.run_goal(&mut task).await {
        Ok(final_status) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": format!("{:?}", final_status),
                "task_id": task.id.to_string(),
                "goal": payload.goal,
                "agent_id": payload.agent_id,
                "engine_used": engine_name,
                "subtasks_count": task.subtasks.len(),
            })),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "failed",
                "error": err.to_string(),
            })),
        ),
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut bus_rx = state.bus.subscribe();

    info!("🔌 WebSocket client connected to InterAgentBus stream");

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = bus_rx.recv().await {
            if let Ok(json_str) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json_str.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };

    info!("🔌 WebSocket client disconnected");
}
