//! WebSocket infrastructure for real-time status updates

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::time::interval;

use super::state::AppState;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// WebSocket upgrade handler at /ws
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut status_rx = state.status_tx.subscribe();
    let mut heartbeat = interval(HEARTBEAT_INTERVAL);

    // Send initial connection status
    let init_msg = serde_json::json!({
        "type": "connected",
        "message": "WebSocket connected"
    });
    let _ = sender.send(Message::Text(init_msg.to_string().into())).await;

    loop {
        tokio::select! {
            // Forward status events to client
            Ok(event) = status_rx.recv() => {
                if let Ok(json) = serde_json::to_string(&event) {
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
            // Send heartbeat pings
            _ = heartbeat.tick() => {
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
            // Handle incoming messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Pong(_))) => {
                        // Client is alive
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(_)) => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::debug!("WebSocket connection closed");
}
