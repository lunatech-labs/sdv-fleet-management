use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::{debug, info, warn};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod api;
mod campaign;
mod hawkbit;
mod models;
mod mqtt;
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
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()))
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=info,tower_http=info".into()),
        )
        .init();

    // ── Config from env ───────────────────────────────────────────────────────
    let mqtt_host = env::var("MQTT_HOST").unwrap_or_else(|_| "localhost".into());
    let mqtt_port: u16 = env::var("MQTT_PORT")
        .unwrap_or_else(|_| "1883".into())
        .parse()
        .expect("MQTT_PORT must be a valid port number");

    let vehicles_file = env::var("VEHICLES_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("vehicles.json"));

    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());

    let hawkbit_url = env::var("HAWKBIT_URL").unwrap_or_else(|_| "http://hawkbit:8080".into());
    let hawkbit_user = env::var("HAWKBIT_USER").unwrap_or_else(|_| "admin".into());
    let hawkbit_password = env::var("HAWKBIT_PASSWORD").unwrap_or_else(|_| "admin".into());

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
        hawkbit_url,
        hawkbit_user,
        hawkbit_password,
    ));
    // Targets are no longer pre-registered from here. Each ota-agent self-
    // registers on first DDI contact, using the gateway token propagated
    // below. The old `register_targets(..)` call was dropped.
    let gateway_token = hawkbit
        .enable_gateway_token()
        .await
        .expect("failed to provision HawkBit gateway token");
    info!("hawkbit: gateway token ready");
    seed_distribution_sets(&hawkbit).await;

    // ── MQTT connect ─────────────────────────────────────────────────────────
    let (mqtt_client, eventloop) = mqtt::connect(&mqtt_host, mqtt_port).await;

    // Propagate the gateway token to every ota-agent over MQTT (retained, so
    // agents joining later pick it up immediately).
    mqtt::publish_gateway_token(&mqtt_client, &gateway_token).await;

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

    // ── MQTT consumer (background task) ──────────────────────────────────────
    // MQTT only carries telemetry now. OTA state flows HawkBit → backend via
    // `poll_campaign_state` below, not via MQTT status messages.
    tokio::spawn(mqtt::run(eventloop, store, tx));

    // ── HawkBit DDI reconciliation (background task) ─────────────────────────
    tokio::spawn(poll_campaign_state(
        hawkbit.clone(),
        campaigns,
        campaign_tx,
    ));

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
        let version = match hawkbit.distribution_set_version(r.distribution_set_id).await {
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
                let new_state = match resolve_state(&hawkbit, vin, rollout_id, &campaign.version)
                    .await
                {
                    Some(s) => s,
                    None => continue,
                };
                if !changed(Some(prev), &new_state) {
                    continue;
                }
                if let Some(updated) =
                    campaigns.set_vehicle_state(&campaign.id, vin, new_state)
                {
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
    let action = actions.into_iter().find(|a| a.rollout == Some(rollout_id))?;

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
