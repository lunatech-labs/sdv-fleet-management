use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::{models::PositionEvent, AppState};

/// Upgrade to a WebSocket connection and stream PositionEvents at 1 Hz per vehicle.
#[utoipa::path(
    get,
    path = "/ws/fleet",
    responses(
        (status = 101, description = "WebSocket upgrade — streams PositionEvent JSON at ~1 Hz per vehicle"),
    )
)]
pub async fn ws_fleet(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    // Subscribe before the upgrade so no events are missed during the handshake.
    let rx = state.tx.subscribe();
    ws.on_upgrade(move |socket| handle_socket(socket, rx))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<PositionEvent>) {
    info!("WebSocket client connected");

    loop {
        tokio::select! {
            // Outbound: forward position events from the broadcast channel.
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = match serde_json::to_string(&event) {
                            Ok(s)  => s,
                            Err(e) => { warn!("serialisation error: {e}"); continue; }
                        };
                        debug!("sending to WebSocket client: {json}");
                        if socket.send(Message::Text(json)).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket receiver lagged by {n} messages — some events dropped");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Inbound: drain incoming frames so the connection doesn't stall.
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(_))  => {} // ping/pong/binary — ignore
                }
            }
        }
    }

    info!("WebSocket client disconnected");
}
