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
    pub tx: broadcast::Sender<PositionEvent>,
}

// ── OpenAPI doc ───────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    paths(fleet::get_fleet, fleet::get_vehicle, fleet::health, ws::ws_fleet),
    components(schemas(VehicleRecord, PositionEvent)),
    info(
        title = "SDV Fleet Management API",
        version = "1.0.0",
        description = "Live vehicle telemetry — REST + WebSocket"
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
    let app = build_router(state);

    info!("listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));

    axum::serve(listener, app).await.expect("server error");
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
        AppState {
            store: Store::new(),
            tx,
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

        let (tx, _) = broadcast::channel(16);
        let state = AppState {
            store: Store::new(),
            tx: tx.clone(),
        };
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

        let (tx, _) = broadcast::channel(1);
        let state = AppState {
            store: Store::new(),
            tx,
        };
        let addr = spawn_test_server(state).await;

        let (mut ws, _) = connect_async(format!("ws://{addr}/ws/fleet"))
            .await
            .unwrap();

        ws.send(Message::Close(None)).await.unwrap();

        // Drain until the stream ends — server should echo the close and terminate.
        while let Some(msg) = ws.next().await {
            if matches!(msg, Ok(Message::Close(_)) | Err(_)) {
                break;
            }
        }
        // Reaching here without panic means the handler closed cleanly.
    }
}
