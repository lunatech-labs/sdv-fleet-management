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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    fn make_campaign(vins: &[&str]) -> Campaign {
        let mut vehicles = HashMap::new();
        for vin in vins {
            vehicles.insert(vin.to_string(), VehicleUpdateState::Pending);
        }
        Campaign {
            id: Uuid::new_v4(),
            version: "1.0.0".into(),
            vehicles,
            created: Utc::now(),
            rollout_id: None,
        }
    }

    #[test]
    fn insert_and_get_round_trips() {
        let store = CampaignStore::new();
        let c = make_campaign(&["VIN-0001"]);
        let id = c.id;
        store.insert(c);
        let fetched = store.get(&id).unwrap();
        assert_eq!(fetched.id, id);
        assert_eq!(fetched.version, "1.0.0");
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let store = CampaignStore::new();
        assert!(store.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn all_returns_all_inserted_campaigns() {
        let store = CampaignStore::new();
        store.insert(make_campaign(&["VIN-0001"]));
        store.insert(make_campaign(&["VIN-0002"]));
        assert_eq!(store.all().len(), 2);
    }

    #[test]
    fn set_vehicle_state_updates_state_and_returns_new_state() {
        let store = CampaignStore::new();
        let c = make_campaign(&["VIN-0001"]);
        let id = c.id;
        store.insert(c);

        let result = store.set_vehicle_state(&id, "VIN-0001", VehicleUpdateState::Downloading);
        assert!(matches!(result, Some(VehicleUpdateState::Downloading)));

        let fetched = store.get(&id).unwrap();
        assert!(matches!(
            fetched.vehicles["VIN-0001"],
            VehicleUpdateState::Downloading
        ));
    }

    #[test]
    fn set_vehicle_state_unknown_campaign_returns_none() {
        let store = CampaignStore::new();
        let result =
            store.set_vehicle_state(&Uuid::new_v4(), "VIN-0001", VehicleUpdateState::Downloading);
        assert!(result.is_none());
    }
}
