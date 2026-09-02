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

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    campaign::{Campaign, VehicleUpdateState},
    AppState,
};

// ── Request / response schemas ──────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCampaign {
    pub version: String,
    pub vins: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VersionsResponse {
    pub versions: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    pub error: String,
}

impl ApiError {
    fn new(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }
}

// ── POST /campaigns ─────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/campaigns",
    request_body = CreateCampaign,
    responses(
        (status = 200, description = "Campaign created", body = Campaign),
        (status = 400, description = "Invalid request",  body = ApiError),
        (status = 503, description = "HawkBit unreachable", body = ApiError),
    )
)]
pub async fn create_campaign(
    State(state): State<AppState>,
    Json(req): Json<CreateCampaign>,
) -> Result<Json<Campaign>, (StatusCode, Json<ApiError>)> {
    if req.version.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError::new("version is required")),
        ));
    }
    if req.vins.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError::new("vins must not be empty")),
        ));
    }
    for vin in &req.vins {
        if state.store.get(vin).is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(format!("unknown VIN: {}", vin))),
            ));
        }
    }

    // Look up the target distribution set.
    let ds_id = match state.hawkbit.find_version_id(&req.version).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(format!("unknown version: {}", req.version))),
            ));
        }
        Err(e) if e.is_unreachable() => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::new("hawkbit unreachable")),
            ));
        }
        Err(e) => {
            warn!("hawkbit lookup failed: {e}");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::new("hawkbit error")),
            ));
        }
    };

    let campaign_id = Uuid::new_v4();
    let rollout_name = format!("campaign-{}", campaign_id);
    let rollout_id = match state
        .hawkbit
        .create_rollout(&rollout_name, ds_id, &req.vins)
        .await
    {
        Ok(id) => Some(id),
        Err(e) if e.is_unreachable() => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::new("hawkbit unreachable")),
            ));
        }
        Err(e) => {
            warn!("hawkbit rollout creation failed: {e}");
            None
        }
    };

    // No MQTT fan-out any more: HawkBit dispatches the rollout to targets
    // and ota-agents pick it up through their DDI poll loop.

    // Seed the campaign store with every target in PENDING.
    let mut vehicles = HashMap::new();
    for vin in &req.vins {
        vehicles.insert(vin.clone(), VehicleUpdateState::Pending);
    }
    let campaign = Campaign {
        id: campaign_id,
        version: req.version,
        vehicles,
        created: Utc::now(),
        rollout_id,
        // Filled in lazily: hawkBit allocates an action per target only once
        // the rollout starts running.
        actions: HashMap::new(),
    };
    state.campaigns.insert(campaign.clone());

    info!(
        "campaign {} launched for {} vehicles",
        campaign_id,
        campaign.vehicles.len()
    );
    Ok(Json(campaign))
}

// ── GET /campaigns ──────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/campaigns",
    responses(
        (status = 200, description = "All campaigns", body = Vec<Campaign>),
    )
)]
pub async fn list_campaigns(State(state): State<AppState>) -> Json<Vec<Campaign>> {
    Json(state.campaigns.all())
}

// ── GET /campaigns/{id} ─────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/campaigns/{id}",
    params(("id" = Uuid, Path, description = "Campaign id")),
    responses(
        (status = 200, description = "Campaign", body = Campaign),
        (status = 404, description = "Campaign not found"),
    )
)]
pub async fn get_campaign(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Campaign>, StatusCode> {
    state
        .campaigns
        .get(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ── GET /versions ───────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/versions",
    responses(
        (status = 200, description = "Available versions",    body = VersionsResponse),
        (status = 503, description = "HawkBit unreachable",   body = ApiError),
    )
)]
pub async fn list_versions(
    State(state): State<AppState>,
) -> Result<Json<VersionsResponse>, (StatusCode, Json<ApiError>)> {
    match state.hawkbit.list_versions().await {
        Ok(versions) => Ok(Json(VersionsResponse { versions })),
        Err(e) if e.is_unreachable() => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError::new("hawkbit unreachable")),
        )),
        Err(e) => {
            warn!("hawkbit list_versions failed: {e}");
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::new("hawkbit error")),
            ))
        }
    }
}
