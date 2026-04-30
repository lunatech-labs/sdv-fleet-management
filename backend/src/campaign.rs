use std::{collections::HashMap, sync::Arc};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub type CampaignId = Uuid;

/// Per-vehicle progress within a campaign.
///
/// Serialised as `{ "state": "...", ... }` via the `tag = "state"` discriminator,
/// matching the shape documented in specs-v2.md §7.4.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "state")]
pub enum VehicleUpdateState {
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "DOWNLOADING")]
    Downloading,
    #[serde(rename = "INSTALLING")]
    Installing,
    #[serde(rename = "COMPLETE")]
    Complete { version: String },
    #[serde(rename = "FAILED")]
    Failed { error: String },
}

/// One OTA campaign as exposed to the dashboard.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Campaign {
    pub id: CampaignId,
    pub version: String,
    pub vehicles: HashMap<String, VehicleUpdateState>,
    pub created: DateTime<Utc>,
    /// HawkBit rollout identifier — kept so the background poll can reconcile
    /// state against HawkBit. Not serialised to the dashboard.
    #[serde(skip)]
    pub rollout_id: Option<u64>,
}

/// Event emitted whenever a single vehicle transitions state.
///
/// Broadcast over the `/ws/campaigns` socket after the campaign store has been
/// updated from a HawkBit response.
#[derive(Debug, Clone)]
pub struct CampaignEvent {
    pub campaign_id: CampaignId,
    pub vin: String,
    pub state: VehicleUpdateState,
}

/// Thread-safe in-memory campaign store.
#[derive(Clone, Default)]
pub struct CampaignStore(Arc<DashMap<CampaignId, Campaign>>);

impl CampaignStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, campaign: Campaign) {
        self.0.insert(campaign.id, campaign);
    }

    pub fn get(&self, id: &CampaignId) -> Option<Campaign> {
        self.0.get(id).map(|c| c.clone())
    }

    pub fn all(&self) -> Vec<Campaign> {
        self.0.iter().map(|c| c.clone()).collect()
    }

    /// Update a single vehicle's state inside a campaign. Returns the new state
    /// if the campaign and VIN exist, so the caller can broadcast a transition.
    pub fn set_vehicle_state(
        &self,
        campaign_id: &CampaignId,
        vin: &str,
        state: VehicleUpdateState,
    ) -> Option<VehicleUpdateState> {
        let mut campaign = self.0.get_mut(campaign_id)?;
        campaign.vehicles.insert(vin.to_string(), state.clone());
        Some(state)
    }
}
