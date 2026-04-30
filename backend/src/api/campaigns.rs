use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
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
