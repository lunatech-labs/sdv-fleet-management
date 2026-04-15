use std::{env, fs, path::PathBuf};

use axum::{routing::get, Router};
use chrono::Utc;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::info;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod api;
mod models;
mod mqtt;
mod store;

use api::{fleet, ws};
use models::{PositionEvent, VehicleRecord, VehicleSeed};
use store::Store;

// ── Shared application state ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub tx:    broadcast::Sender<PositionEvent>,
}

// ── OpenAPI doc ───────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    paths(fleet::get_fleet, fleet::get_vehicle, fleet::health, ws::ws_fleet),
    components(schemas(VehicleRecord, PositionEvent)),
    info(
        title       = "SDV Fleet Management API",
        version     = "1.0.0",
        description = "Live vehicle telemetry — REST + WebSocket"
    )
)]
struct ApiDoc;

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

    // ── Pre-populate store from vehicles.json ─────────────────────────────────
    let store = Store::new();
    let seeds: Vec<VehicleSeed> = serde_json::from_str(
        &fs::read_to_string(&vehicles_file)
            .unwrap_or_else(|_| panic!("cannot read {vehicles_file:?}")),
    )
    .expect("vehicles.json is not valid JSON");

    for s in seeds {
        store.insert(VehicleRecord {
            vin:              s.vin,
            brand:            s.brand,
            model:            s.model,
            software_version: s.software_version,
            latitude:         s.latitude,
            longitude:        s.longitude,
            last_seen:        Utc::now(),
        });
    }
    info!("store pre-populated with {} vehicles", store.all().len());

    // ── Broadcast channel ─────────────────────────────────────────────────────
    let (tx, _) = broadcast::channel::<PositionEvent>(256);

    let state = AppState { store, tx };

    // ── MQTT consumer (background task) ──────────────────────────────────────
    tokio::spawn(mqtt::start(
        state.store.clone(),
        state.tx.clone(),
        mqtt_host,
        mqtt_port,
    ));

    // ── Axum router ───────────────────────────────────────────────────────────
    let app = Router::new()
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/health",        get(fleet::health))
        .route("/fleet",         get(fleet::get_fleet))
        .route("/vehicles/:vin", get(fleet::get_vehicle))
        .route("/ws/fleet",      get(ws::ws_fleet))
        .with_state(state)
        .layer(CorsLayer::permissive());

    info!("listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));

    axum::serve(listener, app).await.expect("server error");
}
