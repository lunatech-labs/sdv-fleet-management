use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    routing::{get, post},
    Router,
};
use chrono::Utc;
use rumqttc::AsyncClient;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
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
    pub mqtt_client: AsyncClient,
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
    let hawkbit_token = env::var("HAWKBIT_TOKEN").unwrap_or_else(|_| "demo".into());

    // ── Pre-populate store from vehicles.json ─────────────────────────────────
    let store = Store::new();
    let seeds: Vec<VehicleSeed> = serde_json::from_str(
        &fs::read_to_string(&vehicles_file)
            .unwrap_or_else(|_| panic!("cannot read {vehicles_file:?}")),
    )
    .expect("vehicles.json is not valid JSON");

    let vins: Vec<String> = seeds.iter().map(|s| s.vin.clone()).collect();
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
    let hawkbit = Arc::new(HawkbitClient::new(hawkbit_url, hawkbit_token));
    register_targets(&hawkbit, &vins).await;
    seed_distribution_sets(&hawkbit).await;

    // ── MQTT connect ─────────────────────────────────────────────────────────
    let (mqtt_client, eventloop) = mqtt::connect(&mqtt_host, mqtt_port).await;

    let campaigns = CampaignStore::new();

    let state = AppState {
        store: store.clone(),
        tx: tx.clone(),
        campaigns: campaigns.clone(),
        campaign_tx: campaign_tx.clone(),
        hawkbit: hawkbit.clone(),
        mqtt_client: mqtt_client.clone(),
    };

    // ── MQTT consumer (background task) ──────────────────────────────────────
    tokio::spawn(mqtt::run(
        eventloop,
        store,
        tx,
        campaigns.clone(),
        campaign_tx.clone(),
        hawkbit.clone(),
    ));

    // ── HawkBit rollout poll (background task) ───────────────────────────────
    tokio::spawn(poll_rollouts(hawkbit.clone(), campaigns, campaign_tx));

    // ── Axum router ───────────────────────────────────────────────────────────
    let app = build_router(state);

    info!("listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));

    axum::serve(listener, app).await.expect("server error");
}

// ── HawkBit startup + poll helpers ───────────────────────────────────────────

async fn register_targets(hawkbit: &HawkbitClient, vins: &[String]) {
    for vin in vins {
        if let Err(e) = hawkbit.register_target(vin).await {
            warn!("hawkbit target registration for {} failed: {}", vin, e);
        }
    }
    info!("hawkbit: registered {} targets", vins.len());
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

/// Every 5 seconds, reconcile the in-memory campaign store against HawkBit
/// rollout state and broadcast transitions.
async fn poll_rollouts(
    hawkbit: Arc<HawkbitClient>,
    campaigns: CampaignStore,
    campaign_tx: broadcast::Sender<CampaignEvent>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    // First tick fires immediately; skip it so we don't race startup registration.
    interval.tick().await;

    loop {
        interval.tick().await;
        for campaign in campaigns.all() {
            let Some(rollout_id) = campaign.rollout_id else {
                continue;
            };
            let targets = match hawkbit.poll_rollout(rollout_id).await {
                Ok(t) => t,
                Err(e) => {
                    if !e.is_unreachable() {
                        warn!("poll_rollout({}) failed: {}", rollout_id, e);
                    }
                    continue;
                }
            };
            for (vin, new_state) in targets {
                let previous = campaign.vehicles.get(&vin).cloned();
                if changed(previous.as_ref(), &new_state) {
                    if let Some(updated) =
                        campaigns.set_vehicle_state(&campaign.id, &vin, new_state)
                    {
                        let _ = campaign_tx.send(CampaignEvent {
                            campaign_id: campaign.id,
                            vin,
                            state: updated,
                        });
                    }
                }
            }
        }
    }
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
        ));
        // Build an MQTT client that isn't actually connected — the tests here
        // never exercise publishing, they only hit HTTP routes.
        let (mqtt_client, _el) =
            AsyncClient::new(rumqttc::MqttOptions::new("test", "127.0.0.1", 1), 1);
        AppState {
            store: Store::new(),
            tx,
            campaigns: CampaignStore::new(),
            campaign_tx,
            hawkbit,
            mqtt_client,
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
