use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use crate::{models::VehicleRecord, AppState};

/// Return all vehicle records sorted by VIN.
#[utoipa::path(
    get,
    path = "/fleet",
    responses(
        (status = 200, description = "All vehicle records", body = Vec<VehicleRecord>)
    )
)]
pub async fn get_fleet(State(state): State<AppState>) -> Json<Vec<VehicleRecord>> {
    let mut vehicles = state.store.all();
    vehicles.sort_by(|a, b| a.vin.cmp(&b.vin));
    Json(vehicles)
}

/// Return a single vehicle record by VIN.
#[utoipa::path(
    get,
    path = "/vehicles/{vin}",
    params(("vin" = String, Path, description = "Vehicle identifier e.g. VIN-0001")),
    responses(
        (status = 200, description = "Vehicle record",   body = VehicleRecord),
        (status = 404, description = "Vehicle not found")
    )
)]
pub async fn get_vehicle(
    State(state): State<AppState>,
    Path(vin): Path<String>,
) -> Result<Json<VehicleRecord>, StatusCode> {
    state.store.get(&vin).map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// Liveness probe.
#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "OK"))
)]
pub async fn health() -> StatusCode {
    StatusCode::OK
}
