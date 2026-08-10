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
