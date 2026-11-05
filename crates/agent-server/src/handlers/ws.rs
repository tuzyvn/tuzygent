use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use tracing::info;

use crate::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.protocols(["tuzygent"])
        .on_upgrade(move |socket| handle_socket(socket, state))
}

pub async fn handle_socket(socket: WebSocket, state: AppState) {
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
