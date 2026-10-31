use agent_core::prelude::*;
use axum::{
    extract::Request,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    response::Response,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "agent-server")]
#[command(about = "AgentCore Standalone REST & WebSocket Daemon Server")]
struct ServerCli {
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,

    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    #[arg(short, long, default_value = ".")]
    workspace: String,

    #[arg(long, env = "TUZYGENT_API_TOKEN", hide_env_values = true)]
    api_token: String,

    #[arg(long, env = "TUZYGENT_ALLOWED_ORIGINS", value_delimiter = ',')]
    allowed_origins: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    supervisor: Arc<AgentSupervisor>,
    bus: InterAgentBus,
    workspace: String,
}

#[derive(Clone)]
struct AuthState {
    token: Arc<String>,
    allowed_origins: Vec<HeaderValue>,
}

fn token_matches(provided: &str, expected: &str) -> bool {
    provided.len() == expected.len()
        && provided
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn require_auth(State(auth): State<AuthState>, request: Request, next: Next) -> Response {
    // CORS middleware does not validate WebSocket origins.
    if let Some(origin) = request.headers().get("origin") {
        if !auth.allowed_origins.contains(origin) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    // Browser WebSocket clients cannot set Authorization; pass ["tuzygent", token].
    let ws_token = if request.uri().path() == "/api/v1/ws" {
        request
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| {
                let mut parts = h.split(',').map(str::trim);
                match (parts.next(), parts.next(), parts.next()) {
                    (Some("tuzygent"), Some(token), None) => Some(token),
                    _ => None,
                }
            })
    } else {
        None
    };
    if !bearer
        .or(ws_token)
        .is_some_and(|token| token_matches(token, &auth.token))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
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

    let state = AppState {
        supervisor,
        bus,
        workspace: cli.workspace,
    };

    let app = build_app(state, auth);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    info!("🚀 AgentCore Daemon Server listening on http://{}", addr);
    info!("📡 WebSocket stream active at ws://{}/api/v1/ws", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn build_app(state: AppState, auth: AuthState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(auth.allowed_origins.clone())
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);

    let api = Router::new()
        .route("/api/v1/engines", get(list_engines_handler))
        .route(
            "/api/v1/agents",
            get(list_agents_handler).post(create_agent_handler),
        )
        .route("/api/v1/tasks/run", post(run_task_handler))
        .route("/api/v1/ws", get(ws_handler))
        .route_layer(middleware::from_fn_with_state(auth, require_auth));
    Router::new()
        .route("/health", get(health_handler))
        .merge(api)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
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

    let compute_engine = match agent_engine::configured_engine(&config, engine_name, None) {
        Ok(engine) => engine,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":error.to_string()})),
            )
        }
    };

    let scratchpad = ScratchpadManager::new(&state.workspace);
    let durable = DurableMemoryStore::new(&state.workspace);
    let guardrails = GuardrailPolicy::default_enterprise();

    let sandbox_cfg = (|| -> anyhow::Result<SandboxConfig> {
        let mut config = SandboxConfig::from_engines_config(&config)?;
        if let Some(driver) = &payload.sandbox {
            config.driver = SandboxDriverKind::parse(driver)?;
        }
        Ok(config)
    })();
    let sandbox_cfg = match sandbox_cfg {
        Ok(config) => config,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error.to_string()})),
            )
        }
    };

    let ooda = AutonomousOodaLoop::new(
        compute_engine,
        guardrails,
        scratchpad,
        durable,
        &state.workspace,
    )
    .with_timeout_seconds(config.defaults.timeout_seconds)
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
    ws.protocols(["tuzygent"])
        .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut bus_rx = state.bus.subscribe();

    info!("🔌 WebSocket client connected to InterAgentBus stream");

    let mut send_task = tokio::spawn(async move {
        loop {
            let event = match bus_rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn app() -> Router {
        let bus = InterAgentBus::new(16);
        build_app(
            AppState {
                supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
                bus,
                workspace: "/tmp".into(),
            },
            AuthState {
                token: Arc::new("test-secret".into()),
                allowed_origins: vec!["https://dashboard.example".parse().unwrap()],
            },
        )
    }
    #[tokio::test]
    async fn health_is_public_and_every_api_route_requires_auth() {
        assert_eq!(
            app()
                .oneshot(
                    Request::builder()
                        .uri("/health")
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        for (method, path) in [
            ("GET", "/api/v1/engines"),
            ("GET", "/api/v1/agents"),
            ("POST", "/api/v1/agents"),
            ("POST", "/api/v1/tasks/run"),
            ("GET", "/api/v1/ws"),
        ] {
            let response = app()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {path}"
            );
        }
    }
    #[tokio::test]
    async fn token_and_origin_are_checked_independently() {
        for (token, origin, expected) in [
            ("test-secret", None, StatusCode::OK),
            ("wrong", None, StatusCode::UNAUTHORIZED),
            (
                "test-secret",
                Some("https://evil.example"),
                StatusCode::FORBIDDEN,
            ),
            (
                "test-secret",
                Some("https://dashboard.example"),
                StatusCode::OK,
            ),
        ] {
            let mut request = Request::builder()
                .uri("/api/v1/agents")
                .header("authorization", format!("Bearer {token}"));
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            assert_eq!(
                app()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                expected
            );
        }
    }
    #[tokio::test]
    async fn browser_preflight_is_allowed_only_for_configured_origins() {
        for origin in ["https://dashboard.example", "https://evil.example"] {
            let request = Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/tasks/run")
                .header("origin", origin)
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "authorization,content-type",
                )
                .body(Body::empty())
                .unwrap();
            let response = app().oneshot(request).await.unwrap();
            assert_eq!(
                response
                    .headers()
                    .contains_key("access-control-allow-origin"),
                origin == "https://dashboard.example"
            );
        }
    }
    #[tokio::test]
    async fn websocket_token_does_not_authenticate_rest() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agents")
                    .header("sec-websocket-protocol", "tuzygent, test-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ws")
                    .header("sec-websocket-protocol", "tuzygent, test-secret")
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    #[test]
    fn cli_has_no_short_option_conflicts() {
        use clap::CommandFactory;
        ServerCli::command().debug_assert();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn authenticated_task_delegates_to_codex_via_rest() {
        use std::os::unix::fs::PermissionsExt;
        let workspace =
            std::env::temp_dir().join(format!("tuzygent-rest-codex-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let binary = workspace.join("codex");
        std::fs::write(
            &binary,
            include_str!("../../agent-engine/tests/fixtures/codex.sh"),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            workspace.join("tuzygent.toml"),
            format!(
                r#"
            [defaults]
            sandbox_driver = "host"
            timeout_seconds = 5
            [engines.codex]
            kind = "codex"
            binary_path = {:?}
            [engines.codex.env]
            CODEX_TEST_CASE = "ooda"
        "#,
                binary.to_str().unwrap()
            ),
        )
        .unwrap();
        let bus = InterAgentBus::new(32);
        let app = build_app(
            AppState {
                supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
                bus,
                workspace: workspace.to_str().unwrap().into(),
            },
            AuthState {
                token: Arc::new("test-secret".into()),
                allowed_origins: vec![],
            },
        );
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/tasks/run")
            .header("authorization", "Bearer test-secret")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"agent_id":"dev","goal":"delegate task","engine":"codex"}"#,
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["engine_used"], "codex");
        assert_eq!(result["status"], "Completed");
        assert_eq!(result["subtasks_count"], 1);
        std::fs::remove_dir_all(workspace).unwrap();
    }
}
