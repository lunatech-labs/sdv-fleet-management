// SPDX-FileCopyrightText: 2026 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use log::{debug, info, warn};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use utoipa::OpenApi;
use uuid::Uuid;

mod api;
mod campaign;
mod hawkbit;
mod models;
mod ota_listener;
mod rfms;
mod store;

use api::{campaigns, fleet, ws};
use campaign::{Campaign, CampaignEvent, CampaignStore, VehicleUpdateState};
use hawkbit::HawkbitClient;
use models::{PositionEvent, VehicleRecord, VehicleSeed};
use store::Store;

// ── Shared application state ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub tx: broadcast::Sender<PositionEvent>,
    pub campaigns: CampaignStore,
    pub campaign_tx: broadcast::Sender<CampaignEvent>,
    pub hawkbit: Arc<HawkbitClient>,
}

// ── OpenAPI doc ───────────────────────────────────────────────────────────────

/// Fleet operations backend.
///
/// Serves the operator dashboard, orchestrates OTA campaigns through the
/// hawkBit Management API, and reads fleet telemetry from the blueprint rFMS
/// API.
#[derive(Debug, Parser)]
#[command(name = "backend", about, version)]
struct Cli {
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:3000")]
    bind_addr: String,

    /// Vehicle registry. Supplies brand and model, which the rFMS API does not
    /// return today. See docs/rfms-coverage.md.
    #[arg(long, env = "VEHICLES_FILE", default_value = "vehicles.json")]
    vehicles_file: PathBuf,

    #[arg(long, env = "HAWKBIT_URL", default_value = "http://hawkbit:8080")]
    hawkbit_url: String,

    #[arg(long, env = "HAWKBIT_USER", default_value = "admin")]
    hawkbit_user: String,

    #[arg(long, env = "HAWKBIT_PASSWORD", default_value = "admin")]
    hawkbit_password: String,

    /// Shared with the agents, which authenticate to the DDI API with it.
    #[arg(long, env = "HAWKBIT_GATEWAY_TOKEN")]
    hawkbit_gateway_token: String,

    /// Reconcile campaign state against the hawkBit Management API.
    ///
    /// Set to false to demonstrate that the uProtocol notification path alone
    /// drives a rollout to completion.
    #[arg(
        long,
        env = "HAWKBIT_RECONCILE_ENABLED",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    hawkbit_reconcile_enabled: bool,
}

/// Serve the generated OpenAPI document.
async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(OpenApi)]
#[openapi(
    paths(
        fleet::get_fleet,
        fleet::get_vehicle,
        fleet::health,
        ws::ws_fleet,
        ws::ws_campaigns,
        campaigns::create_campaign,
        campaigns::list_campaigns,
        campaigns::get_campaign,
        campaigns::list_versions,
    ),
    components(schemas(
        VehicleRecord,
        PositionEvent,
        Campaign,
        VehicleUpdateState,
        campaigns::CreateCampaign,
        campaigns::VersionsResponse,
        campaigns::ApiError,
    )),
    info(
        title = "SDV Fleet Management API",
        version = "2.0.0",
        description = "Live vehicle telemetry + OTA campaigns"
    )
)]
struct ApiDoc;

// ── Router ────────────────────────────────────────────────────────────────────

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // The OpenAPI document is served as JSON. utoipa-swagger-ui is not
        // used: it vendors the Swagger UI web assets into the binary, which
        // is a large third-party licence surface for a demo component.
        .route("/api-docs/openapi.json", get(openapi))
        .route("/health", get(fleet::health))
        .route("/fleet", get(fleet::get_fleet))
        .route("/vehicles/:vin", get(fleet::get_vehicle))
        .route("/ws/fleet", get(ws::ws_fleet))
        .route(
            "/campaigns",
            post(campaigns::create_campaign).get(campaigns::list_campaigns),
        )
        .route("/campaigns/:id", get(campaigns::get_campaign))
        .route("/versions", get(campaigns::list_versions))
        .route("/ws/campaigns", get(ws::ws_campaigns))
        .with_state(state)
        .layer(CorsLayer::permissive())
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // ── Config from env ───────────────────────────────────────────────────────
    let rfms_config = rfms::RfmsConfig::from_env();

    let cli = Cli::parse();
    let vehicles_file = cli.vehicles_file.clone();
    let bind_addr = cli.bind_addr.clone();

    // ── Pre-populate store from vehicles.json ─────────────────────────────────
    let store = Store::new();
    let seeds: Vec<VehicleSeed> = serde_json::from_str(
        &fs::read_to_string(&vehicles_file)
            .unwrap_or_else(|_| panic!("cannot read {vehicles_file:?}")),
    )
    .expect("vehicles.json is not valid JSON");

    for s in seeds {
        store.insert(VehicleRecord {
            vin: s.vin,
            brand: s.brand,
            model: s.model,
            software_version: s.software_version,
            latitude: s.latitude,
            longitude: s.longitude,
            last_seen: Utc::now(),
        });
    }
    info!("store pre-populated with {} vehicles", store.all().len());

    // ── Broadcast channels ────────────────────────────────────────────────────
    let (tx, _) = broadcast::channel::<PositionEvent>(256);
    let (campaign_tx, _) = broadcast::channel::<CampaignEvent>(256);

    // ── HawkBit client + startup reconciliation ──────────────────────────────
    let hawkbit = Arc::new(HawkbitClient::new(
        cli.hawkbit_url.clone(),
        cli.hawkbit_user.clone(),
        cli.hawkbit_password.clone(),
    ));
    // Targets are no longer pre-registered from here. Each ota-agent self-
    // registers on first DDI contact, using the gateway token provisioned here.
    // The old `register_targets(..)` call was dropped.
    //
    // The token is a deployment-wide secret shared with the agents through
    // HAWKBIT_GATEWAY_TOKEN rather than broadcast at runtime, so both sides can
    // start in any order and the demo needs no MQTT broker.
    hawkbit
        .enable_gateway_token(&cli.hawkbit_gateway_token)
        .await
        .expect("failed to provision HawkBit gateway token");
    info!("hawkbit: gateway token ready");
    seed_distribution_sets(&hawkbit).await;

    let campaigns = CampaignStore::new();

    // Rehydrate the campaign store from HawkBit so restarts don't lose
    // history. poll_campaign_state will update each vehicle's state on its
    // first tick.
    hydrate_campaigns(&hawkbit, &campaigns).await;

    let state = AppState {
        store: store.clone(),
        tx: tx.clone(),
        campaigns: campaigns.clone(),
        campaign_tx: campaign_tx.clone(),
        hawkbit: hawkbit.clone(),
    };

    // ── Telemetry ingest (background task) ───────────────────────────────────
    // Positions come from the blueprint rFMS API, which FMS Server serves from
    // the InfluxDB that the FMS Consumer fills from the uProtocol vehicle status
    // topic. Consuming rFMS rather than the database keeps the dashboard
    // portable across backends and needs no database credentials. OTA state
    // flows HawkBit → backend via `poll_campaign_state` below.
    tokio::spawn(rfms::run(rfms_config, store, tx));

    // ── OTA notifications over uProtocol ─────────────────────────────────────
    // The transport must outlive this scope: dropping it deregisters the
    // listener. A failure here is not fatal — poll_campaign_state below still
    // reconciles against HawkBit, just with higher latency.
    let _ota_transport = match ota_listener::start(
        ota_listener::OtaListenerConfig::from_env(),
        campaigns.clone(),
        campaign_tx.clone(),
        hawkbit.clone(),
    )
    .await
    {
        Ok(transport) => Some(transport),
        Err(e) => {
            warn!("OTA notifications over uProtocol unavailable: {e}");
            None
        }
    };

    // ── HawkBit reconciliation (background task) ─────────────────────────────
    // Repairs anything a dropped uProtocol notification would leave stale, and
    // rehydrates state after a restart. Can be switched off to verify that the
    // notification path alone drives a campaign to completion.
    if cli.hawkbit_reconcile_enabled {
        tokio::spawn(poll_campaign_state(hawkbit.clone(), campaigns, campaign_tx));
    } else {
        warn!("HawkBit reconciliation disabled; campaign state comes only from uProtocol");
    }

    // ── Axum router ───────────────────────────────────────────────────────────
    let app = build_router(state);

    info!("listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));

    axum::serve(listener, app).await.expect("server error");
}

// ── HawkBit startup helpers ──────────────────────────────────────────────────

/// Rebuild the in-memory campaign store from HawkBit rollouts on startup.
/// Every rollout named `campaign-<uuid>` becomes a `Campaign` with all its
/// targets in `PENDING`; `poll_campaign_state` then reconciles each vehicle's
/// real state on its next tick. Individual-campaign failures are logged and
/// skipped so one bad rollout doesn't prevent the others from hydrating.
async fn hydrate_campaigns(hawkbit: &HawkbitClient, store: &CampaignStore) {
    let rollouts = match hawkbit.list_rollouts().await {
        Ok(r) => r,
        Err(e) => {
            warn!("skipping campaign hydration: list_rollouts failed: {e}");
            return;
        }
    };

    let mut rehydrated = 0usize;
    for r in rollouts {
        let Some(campaign_id) = parse_campaign_uuid(&r.name) else {
            continue;
        };
        let version = match hawkbit
            .distribution_set_version(r.distribution_set_id)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!("hydrate: DS lookup failed for rollout {}: {e}", r.id);
                continue;
            }
        };
        let vins = match hawkbit.rollout_target_vins(r.id).await {
            Ok(v) => v,
            Err(e) => {
                warn!("hydrate: target lookup failed for rollout {}: {e}", r.id);
                continue;
            }
        };

        let mut vehicles = std::collections::HashMap::new();
        for vin in vins {
            vehicles.insert(vin, VehicleUpdateState::Pending);
        }

        let created = DateTime::from_timestamp_millis(r.created_at).unwrap_or_else(Utc::now);
        store.insert(Campaign {
            id: campaign_id,
            version,
            vehicles,
            created,
            rollout_id: Some(r.id),
            actions: std::collections::HashMap::new(),
        });
        rehydrated += 1;
    }
    if rehydrated > 0 {
        info!("hydrated {} campaign(s) from HawkBit", rehydrated);
    }
}

fn parse_campaign_uuid(rollout_name: &str) -> Option<Uuid> {
    let id = rollout_name.strip_prefix("campaign-")?;
    Uuid::parse_str(id).ok()
}

async fn seed_distribution_sets(hawkbit: &HawkbitClient) {
    const SEED: &[(&str, &str)] = &[
        ("fleet-fw", "1.5.0"),
        ("fleet-fw", "2.0.0"),
        ("fleet-fw", "2.1.0-beta"),
    ];
    for (name, version) in SEED {
        match hawkbit.ensure_distribution_set(name, version).await {
            Ok(id) => info!("hawkbit: distribution set {name}:{version} -> id {id}"),
            Err(e) => warn!("hawkbit: failed to seed {name}:{version}: {e}"),
        }
    }
}

// ── DDI reconciliation ──────────────────────────────────────────────────────

/// Every 3s, walk every non-terminal vehicle in every campaign and reconcile
/// its state from HawkBit. HawkBit's per-action `status` handles PENDING /
/// COMPLETE / FAILED cleanly; distinguishing DOWNLOADING from INSTALLING
/// requires peeking at the latest status-history entry's `messages`, where
/// ota-agents include `"DOWNLOADING"` or `"INSTALLING"` as the first message.
async fn poll_campaign_state(
    hawkbit: Arc<HawkbitClient>,
    campaigns: CampaignStore,
    campaign_tx: broadcast::Sender<CampaignEvent>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.tick().await; // skip the immediate first tick

    loop {
        ticker.tick().await;
        for campaign in campaigns.all() {
            let Some(rollout_id) = campaign.rollout_id else {
                continue;
            };

            for (vin, prev) in &campaign.vehicles {
                if is_terminal(prev) {
                    continue;
                }
                let new_state =
                    match resolve_state(&hawkbit, vin, rollout_id, &campaign.version).await {
                        Some(s) => s,
                        None => continue,
                    };
                if !changed(Some(prev), &new_state) {
                    continue;
                }
                if let Some(updated) = campaigns.set_vehicle_state(&campaign.id, vin, new_state) {
                    let _ = campaign_tx.send(CampaignEvent {
                        campaign_id: campaign.id,
                        vin: vin.clone(),
                        state: updated,
                    });
                }
            }
        }
    }
}

async fn resolve_state(
    hawkbit: &HawkbitClient,
    vin: &str,
    rollout_id: u64,
    campaign_version: &str,
) -> Option<VehicleUpdateState> {
    let actions = match hawkbit.list_target_actions(vin).await {
        Ok(a) => a,
        Err(e) => {
            if !e.is_unreachable() {
                warn!("list_target_actions({vin}) failed: {e}");
            }
            return None;
        }
    };
    let action = actions
        .into_iter()
        .find(|a| a.rollout == Some(rollout_id))?;

    // Always read the latest status entry so we can log it, even in branches
    // that don't need it for the state mapping.
    let latest = latest_message(hawkbit, vin, action.id).await;
    debug!(
        "resolve_state vin={} rollout={} action_id={} status={} latest_msg={:?}",
        vin, rollout_id, action.id, action.status, latest
    );

    // HawkBit reports `retrieved` (target has picked up the action) and
    // `running` for in-progress actions, depending on version and whether the
    // device has posted any `proceeding` feedback yet. Both need to be mapped
    // via the message to distinguish DOWNLOADING from INSTALLING.
    //
    // There is a small window (~60ms) around the terminal feedback where the
    // device has already posted `installed X.Y.Z` but HawkBit hasn't flipped
    // action.status from `retrieved` to `finished` yet. Checking the message
    // first avoids a transient Pending flash in the dashboard.
    let state = match latest.as_deref() {
        Some(msg) if msg.starts_with("installed ") => VehicleUpdateState::Complete {
            version: campaign_version.to_string(),
        },
        _ => match action.status.as_str() {
            "finished" => VehicleUpdateState::Complete {
                version: campaign_version.to_string(),
            },
            "error" | "canceled" => VehicleUpdateState::Failed {
                error: latest.clone().unwrap_or_else(|| "update failed".into()),
            },
            "running" | "retrieved" => match latest.as_deref() {
                Some(msg) if msg.starts_with("INSTALLING") => VehicleUpdateState::Installing,
                Some(msg) if msg.starts_with("DOWNLOADING") => VehicleUpdateState::Downloading,
                _ => VehicleUpdateState::Pending,
            },
            _ => VehicleUpdateState::Pending,
        },
    };
    info!("resolve_state vin={} -> {:?}", vin, state);
    Some(state)
}

async fn latest_message(hawkbit: &HawkbitClient, vin: &str, action_id: u64) -> Option<String> {
    let entry = hawkbit.latest_action_status(vin, action_id).await.ok()??;
    entry.messages.into_iter().next()
}

fn is_terminal(state: &VehicleUpdateState) -> bool {
    matches!(
        state,
        VehicleUpdateState::Complete { .. } | VehicleUpdateState::Failed { .. }
    )
}

fn changed(previous: Option<&VehicleUpdateState>, new: &VehicleUpdateState) -> bool {
    match (previous, new) {
        (None, _) => true,
        (Some(a), b) => std::mem::discriminant(a) != std::mem::discriminant(b),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn make_vehicle(vin: &str) -> VehicleRecord {
        VehicleRecord {
            vin: vin.to_string(),
            brand: "Acme".to_string(),
            model: "X1".to_string(),
            software_version: "1.0.0".to_string(),
            latitude: 48.85,
            longitude: 2.35,
            last_seen: Utc::now(),
        }
    }

    fn test_state() -> AppState {
        let (tx, _) = broadcast::channel(1);
        let (campaign_tx, _) = broadcast::channel(1);
        let hawkbit = Arc::new(HawkbitClient::new(
            "http://127.0.0.1:1".into(),
            "test".into(),
            "test".into(),
        ));
        AppState {
            store: Store::new(),
            tx,
            campaigns: CampaignStore::new(),
            campaign_tx,
            hawkbit,
        }
    }

    async fn spawn_test_server(state: AppState) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await.unwrap() });
        addr
    }

    #[tokio::test]
    async fn health_returns_200() {
        let response = build_router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn fleet_empty_store_returns_empty_array() {
        let response = build_router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/fleet")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"[]");
    }

    #[tokio::test]
    async fn get_vehicle_unknown_vin_returns_404() {
        let response = build_router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/vehicles/UNKNOWN")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ws_forwards_position_event() {
        use futures_util::StreamExt;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let state = test_state();
        let tx = state.tx.clone();
        let addr = spawn_test_server(state).await;

        let (mut ws, _) = connect_async(format!("ws://{addr}/ws/fleet"))
            .await
            .unwrap();

        let event = PositionEvent {
            vin: "VIN-TEST".into(),
            lat: 48.85,
            lon: 2.35,
        };
        tx.send(event).unwrap();

        let msg = ws.next().await.unwrap().unwrap();
        let text = match msg {
            Message::Text(t) => t,
            other => panic!("expected text frame, got {other:?}"),
        };
        let received: PositionEvent = serde_json::from_str(&text).unwrap();
        assert_eq!(received.vin, "VIN-TEST");
        assert_eq!(received.lat, 48.85);
        assert_eq!(received.lon, 2.35);
    }

    #[tokio::test]
    async fn ws_handles_client_close() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let state = test_state();
        let addr = spawn_test_server(state).await;

        let (mut ws, _) = connect_async(format!("ws://{addr}/ws/fleet"))
            .await
            .unwrap();

        ws.send(Message::Close(None)).await.unwrap();

        while let Some(msg) = ws.next().await {
            if matches!(msg, Ok(Message::Close(_)) | Err(_)) {
                break;
            }
        }
    }

    #[tokio::test]
    async fn campaigns_empty_store_returns_empty_array() {
        let response = build_router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/campaigns")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"[]");
    }

    #[tokio::test]
    async fn get_campaign_unknown_id_returns_404() {
        let response = build_router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/campaigns/00000000-0000-0000-0000-000000000000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // ── Pure-helper unit tests ────────────────────────────────────────────────

    #[test]
    fn is_terminal_complete_and_failed_are_terminal() {
        assert!(is_terminal(&VehicleUpdateState::Complete {
            version: "1.0".into()
        }));
        assert!(is_terminal(&VehicleUpdateState::Failed {
            error: "oops".into()
        }));
    }

    #[test]
    fn is_terminal_non_terminal_states_return_false() {
        assert!(!is_terminal(&VehicleUpdateState::Pending));
        assert!(!is_terminal(&VehicleUpdateState::Downloading));
        assert!(!is_terminal(&VehicleUpdateState::Installing));
    }

    #[test]
    fn changed_none_previous_is_always_true() {
        assert!(changed(None, &VehicleUpdateState::Pending));
        assert!(changed(
            None,
            &VehicleUpdateState::Complete {
                version: "1.0".into()
            }
        ));
    }

    #[test]
    fn changed_same_discriminant_is_false() {
        assert!(!changed(
            Some(&VehicleUpdateState::Pending),
            &VehicleUpdateState::Pending
        ));
        assert!(!changed(
            Some(&VehicleUpdateState::Downloading),
            &VehicleUpdateState::Downloading
        ));
    }

    #[test]
    fn changed_different_discriminant_is_true() {
        assert!(changed(
            Some(&VehicleUpdateState::Pending),
            &VehicleUpdateState::Downloading
        ));
        assert!(changed(
            Some(&VehicleUpdateState::Downloading),
            &VehicleUpdateState::Complete {
                version: "1.0".into()
            }
        ));
    }

    #[test]
    fn parse_campaign_uuid_valid_name() {
        let id = Uuid::new_v4();
        let name = format!("campaign-{id}");
        assert_eq!(parse_campaign_uuid(&name), Some(id));
    }

    #[test]
    fn parse_campaign_uuid_no_prefix_returns_none() {
        assert!(parse_campaign_uuid("not-a-campaign").is_none());
        assert!(parse_campaign_uuid("").is_none());
    }

    #[test]
    fn parse_campaign_uuid_invalid_uuid_returns_none() {
        assert!(parse_campaign_uuid("campaign-not-a-uuid").is_none());
    }

    // ── REST handler tests (populated state) ─────────────────────────────────

    #[tokio::test]
    async fn fleet_returns_vehicles_sorted_by_vin() {
        let state = test_state();
        state.store.insert(make_vehicle("VIN-0002"));
        state.store.insert(make_vehicle("VIN-0001"));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri("/fleet")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let vehicles: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(vehicles[0]["vin"], "VIN-0001");
        assert_eq!(vehicles[1]["vin"], "VIN-0002");
    }

    #[tokio::test]
    async fn get_vehicle_known_vin_returns_200_with_data() {
        let state = test_state();
        state.store.insert(make_vehicle("VIN-0001"));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri("/vehicles/VIN-0001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["vin"], "VIN-0001");
        assert_eq!(v["brand"], "Acme");
    }

    #[tokio::test]
    async fn create_campaign_empty_version_returns_400() {
        let state = test_state();
        state.store.insert(make_vehicle("VIN-0001"));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/campaigns")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"version":"","vins":["VIN-0001"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err["error"].as_str().unwrap().contains("version"));
    }

    #[tokio::test]
    async fn create_campaign_whitespace_version_returns_400() {
        let state = test_state();
        state.store.insert(make_vehicle("VIN-0001"));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/campaigns")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"version":"   ","vins":["VIN-0001"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_campaign_empty_vins_returns_400() {
        let state = test_state();

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/campaigns")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"version":"1.0.0","vins":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn create_campaign_unknown_vin_returns_400() {
        let state = test_state(); // empty store — any VIN is unknown

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/campaigns")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"version":"1.0.0","vins":["VIN-UNKNOWN"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("unknown vin"));
    }

    #[tokio::test]
    async fn ws_campaigns_sends_snapshot_on_connect() {
        use futures_util::StreamExt;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let state = test_state();
        let addr = spawn_test_server(state).await;

        let (mut ws, _) = connect_async(format!("ws://{addr}/ws/campaigns"))
            .await
            .unwrap();

        let msg = ws.next().await.unwrap().unwrap();
        let text = match msg {
            Message::Text(t) => t,
            other => panic!("expected text frame, got {other:?}"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["type"], "snapshot");
        assert!(parsed["campaigns"].is_object());
    }
}
