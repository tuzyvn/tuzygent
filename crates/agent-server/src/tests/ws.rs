use std::{sync::Arc, time::Duration};

use agent_core::prelude::*;
use axum::{
    extract::{ws::WebSocketUpgrade, State},
    http::StatusCode,
    routing::get,
    Router,
};
use futures::StreamExt;
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    WebSocketStream,
};

use crate::{
    handlers::ws::{handle_socket, ws_handler},
    AppState,
};

fn state(bus: InterAgentBus) -> AppState {
    AppState {
        supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
        bus,
        workspace: "/tmp".into(),
        vault: Arc::new(agent_engine::AccountVault::new_with_dir(
            std::env::temp_dir().join(format!("ws_test_vault_{}", uuid::Uuid::new_v4().simple())),
        )),
        task_service: Arc::new(agent_runtime::TaskService::new(
            Arc::new(agent_runtime::SqliteTaskStore::memory().unwrap()),
            "/tmp",
            2,
            None,
            None,
        )),
        metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
    }
}

async fn connect(
    app: Router,
) -> (
    WebSocketStream<tokio::io::DuplexStream>,
    tokio::task::JoinHandle<()>,
) {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let service = TowerToHyperService::new(app);
    let server = tokio::spawn(async move {
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .with_upgrades()
            .await
            .unwrap();
    });
    let request = "ws://localhost/ws".into_client_request().unwrap();
    let (socket, _) = tokio_tungstenite::client_async(request, client_io)
        .await
        .expect("websocket upgrade should succeed");
    (socket, server)
}

#[tokio::test]
async fn ws_handler_accepts_upgrade_and_selects_tuzygent_protocol() {
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state(InterAgentBus::new(16)));
    let request = "ws://localhost/ws".into_client_request().unwrap();
    let mut request = request;
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "tuzygent".parse().unwrap());
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let service = TowerToHyperService::new(app);
    let server = tokio::spawn(async move {
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .with_upgrades()
            .await
            .unwrap();
    });
    let (socket, response) = tokio_tungstenite::client_async(request, client_io)
        .await
        .expect("websocket upgrade should succeed");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok()),
        Some("tuzygent")
    );

    drop(socket);
    server.abort();
}

#[tokio::test]
async fn handle_socket_sends_published_bus_events_as_json() {
    let bus = InterAgentBus::new(16);
    let app = Router::new()
        .route(
            "/ws",
            get(
                |ws: WebSocketUpgrade, State(state): State<AppState>| async move {
                    ws.on_upgrade(move |socket| handle_socket(socket, state))
                },
            ),
        )
        .with_state(state(bus.clone()));
    let (mut socket, server) = connect(app).await;

    let event = InterAgentEvent::new(
        AgentId::new("ws-test-agent"),
        None,
        "test.event",
        serde_json::json!({"answer": 42}),
    );
    let expected = serde_json::to_value(&event).unwrap();
    let publish_bus = bus.clone();
    let publish_event = event.clone();
    let publisher = tokio::spawn(async move {
        for _ in 0..20 {
            let _ = publish_bus.publish(publish_event.clone());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("event should arrive before timeout")
        .expect("socket should stay open")
        .expect("websocket frame should be valid");
    let Message::Text(text) = frame else {
        panic!("expected event text frame, got {frame:?}");
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        expected
    );

    publisher.abort();
    let _ = socket.close(None).await;
    server.abort();
}
