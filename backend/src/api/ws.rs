use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use std::collections::HashMap;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use serde::Serialize;

use crate::{
    campaign::{Campaign, CampaignEvent, CampaignId, VehicleUpdateState},
    models::PositionEvent,
    AppState,
};

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
    ws.on_upgrade(move |socket| handle_socket_fleet(socket, rx))
}

async fn handle_socket_fleet(mut socket: WebSocket, mut rx: broadcast::Receiver<PositionEvent>) {
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

// ── WS /ws/campaigns ────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsCampaignMessage<'a> {
    Snapshot {
        campaigns: HashMap<CampaignId, Campaign>,
    },
    Transition {
        campaign_id: CampaignId,
        vin: &'a str,
        #[serde(flatten)]
        state: &'a VehicleUpdateState,
    },
}

#[utoipa::path(
    get,
    path = "/ws/campaigns",
    responses(
        (status = 101, description = "WebSocket upgrade — snapshot then per-vehicle transitions"),
    )
)]
pub async fn ws_campaigns(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let rx = state.campaign_tx.subscribe();
    ws.on_upgrade(move |socket| handle_socket_campaigns(socket, state, rx))
}

async fn handle_socket_campaigns(
    mut socket: WebSocket,
    state: AppState,
    mut rx: broadcast::Receiver<CampaignEvent>,
) {
    // Snapshot everything currently known so mid-campaign clients hydrate
    // without a separate REST call.
    let snapshot: HashMap<_, _> = state
        .campaigns
        .all()
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    let snapshot_msg = WsCampaignMessage::Snapshot {
        campaigns: snapshot,
    };
    match serde_json::to_string(&snapshot_msg) {
        Ok(s) => {
            if socket.send(Message::Text(s)).await.is_err() {
                return;
            }
        }
        Err(e) => warn!("snapshot serialisation failed: {e}"),
    }

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = WsCampaignMessage::Transition {
                            campaign_id: event.campaign_id,
                            vin:         &event.vin,
                            state:       &event.state,
                        };
                        let json = match serde_json::to_string(&msg) {
                            Ok(s)  => s,
                            Err(e) => { warn!("transition serialisation failed: {e}"); continue; }
                        };
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("campaign WebSocket receiver lagged by {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(_))  => {}
                }
            }
        }
    }
}
