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
    /// HawkBit action id per VIN, learned the first time a vehicle reports.
    ///
    /// OTA notifications carry `(vin, action_id)` and no campaign, so this is
    /// the correlation key. It cannot be filled in at rollout creation because
    /// HawkBit allocates an action per target only once the rollout starts.
    #[serde(skip)]
    pub actions: HashMap<String, u64>,
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

    /// Find the campaign that owns a specific HawkBit action for a vehicle.
    ///
    /// This is an exact match against the cached `(vin, action_id)` binding. It
    /// returns `None` until the action has been bound with `bind_action`.
    pub fn campaign_for_action(&self, vin: &str, action_id: u64) -> Option<CampaignId> {
        self.0
            .iter()
            .find(|c| c.actions.get(vin) == Some(&action_id))
            .map(|c| c.id)
    }

    /// Find the campaign created from a given HawkBit rollout.
    pub fn campaign_for_rollout(&self, rollout_id: u64) -> Option<CampaignId> {
        self.0
            .iter()
            .find(|c| c.rollout_id == Some(rollout_id))
            .map(|c| c.id)
    }

    /// Return the action id already bound for a vehicle in a campaign.
    pub fn bound_action(&self, vin: &str) -> Option<u64> {
        self.0.iter().find_map(|c| c.actions.get(vin).copied())
    }

    /// Cache the HawkBit action id that belongs to a vehicle in a campaign.
    ///
    /// Returns false if the campaign does not exist or does not target the VIN,
    /// so a mis-resolved action cannot attach itself to an unrelated campaign.
    pub fn bind_action(&self, campaign_id: &CampaignId, vin: &str, action_id: u64) -> bool {
        let Some(mut campaign) = self.0.get_mut(campaign_id) else {
            return false;
        };
        if !campaign.vehicles.contains_key(vin) {
            return false;
        }
        campaign.actions.insert(vin.to_string(), action_id);
        true
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
            actions: HashMap::new(),
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
    fn campaign_for_action_returns_none_until_bound() {
        let store = CampaignStore::new();
        let c = make_campaign(&["VIN-0001"]);
        let id = c.id;
        store.insert(c);

        assert!(store.campaign_for_action("VIN-0001", 42).is_none());
        assert!(store.bind_action(&id, "VIN-0001", 42));
        assert_eq!(store.campaign_for_action("VIN-0001", 42), Some(id));
    }

    #[test]
    fn campaign_for_action_does_not_match_a_different_action() {
        let store = CampaignStore::new();
        let c = make_campaign(&["VIN-0001"]);
        let id = c.id;
        store.insert(c);
        store.bind_action(&id, "VIN-0001", 42);

        assert!(store.campaign_for_action("VIN-0001", 43).is_none());
    }

    #[test]
    fn overlapping_campaigns_stay_separate() {
        // The case the old most-recent-campaign guess got wrong: two campaigns
        // both targeting VIN-0001, each with its own hawkBit action.
        let store = CampaignStore::new();
        let first = make_campaign(&["VIN-0001", "VIN-0002"]);
        let second = make_campaign(&["VIN-0001", "VIN-0003"]);
        let (first_id, second_id) = (first.id, second.id);
        store.insert(first);
        store.insert(second);

        store.bind_action(&first_id, "VIN-0002", 10);
        store.bind_action(&second_id, "VIN-0003", 11);
        store.bind_action(&first_id, "VIN-0001", 12);

        assert_eq!(store.campaign_for_action("VIN-0001", 12), Some(first_id));
        assert_eq!(store.campaign_for_action("VIN-0002", 10), Some(first_id));
        assert_eq!(store.campaign_for_action("VIN-0003", 11), Some(second_id));
    }

    #[test]
    fn bind_action_rejects_a_vin_the_campaign_does_not_target() {
        let store = CampaignStore::new();
        let c = make_campaign(&["VIN-0001"]);
        let id = c.id;
        store.insert(c);

        assert!(!store.bind_action(&id, "VIN-9999", 42));
        assert!(store.campaign_for_action("VIN-9999", 42).is_none());
    }

    #[test]
    fn bind_action_rejects_an_unknown_campaign() {
        let store = CampaignStore::new();
        assert!(!store.bind_action(&Uuid::new_v4(), "VIN-0001", 42));
    }

    #[test]
    fn campaign_for_rollout_matches_the_rollout_id() {
        let store = CampaignStore::new();
        let mut c = make_campaign(&["VIN-0001"]);
        c.rollout_id = Some(7);
        let id = c.id;
        store.insert(c);

        assert_eq!(store.campaign_for_rollout(7), Some(id));
        assert!(store.campaign_for_rollout(8).is_none());
    }

    #[test]
    fn bound_action_reports_the_cached_id() {
        let store = CampaignStore::new();
        let c = make_campaign(&["VIN-0001"]);
        let id = c.id;
        store.insert(c);

        assert!(store.bound_action("VIN-0001").is_none());
        store.bind_action(&id, "VIN-0001", 42);
        assert_eq!(store.bound_action("VIN-0001"), Some(42));
    }

    #[test]
    fn set_vehicle_state_unknown_campaign_returns_none() {
        let store = CampaignStore::new();
        let result =
            store.set_vehicle_state(&Uuid::new_v4(), "VIN-0001", VehicleUpdateState::Downloading);
        assert!(result.is_none());
    }
}
