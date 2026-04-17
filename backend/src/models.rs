use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Full vehicle record served by GET /fleet and GET /vehicles/{vin}.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VehicleRecord {
    pub vin: String,
    pub brand: String,
    pub model: String,
    pub software_version: String,
    pub latitude: f64,
    pub longitude: f64,
    pub last_seen: DateTime<Utc>,
}

/// Real-time position update pushed over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionEvent {
    pub vin: String,
    pub lat: f64,
    pub lon: f64,
}

/// Minimal vehicle entry in vehicles.json (used to pre-populate static fields).
#[derive(Debug, Deserialize)]
pub struct VehicleSeed {
    pub vin: String,
    pub brand: String,
    pub model: String,
    pub software_version: String,
    pub latitude: f64,
    pub longitude: f64,
}
