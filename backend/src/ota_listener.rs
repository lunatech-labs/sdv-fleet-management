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

//! Receives OTA state transitions from the in-vehicle agents over uProtocol.
//!
//! Each agent sends a Notification on every transition, addressed to this
//! component. That gives the dashboard a low-latency view of a rollout without
//! waiting for the HawkBit Management API poll in `poll_campaign_state`, which
//! stays in place as reconciliation: it repairs anything a dropped notification
//! would otherwise leave stale, and it is what rehydrates state after a restart.

use std::{str::FromStr, sync::Arc};

use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use up_rust::{
    LocalUriProvider, StaticUriProvider, UListener, UMessage, UTransport, UUri, UUriError,
};
use up_transport_zenoh::UPTransportZenoh;

use crate::campaign::{CampaignEvent, CampaignStore, VehicleUpdateState};

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/ota_proto/mod.rs"));
}

pub use proto::ota::{OtaStatus, UpdateState};

/// Translate the wire enum into the state the dashboard consumes.
///
/// Returns `None` for states that carry no dashboard meaning, so an unknown or
/// unset value is ignored rather than being coerced into a real transition.
pub fn to_vehicle_state(status: &OtaStatus) -> Option<VehicleUpdateState> {
    match status.state.enum_value_or_default() {
        UpdateState::UPDATE_STATE_PENDING => Some(VehicleUpdateState::Pending),
        UpdateState::UPDATE_STATE_DOWNLOADING => Some(VehicleUpdateState::Downloading),
        UpdateState::UPDATE_STATE_INSTALLING => Some(VehicleUpdateState::Installing),
        UpdateState::UPDATE_STATE_COMPLETE => Some(VehicleUpdateState::Complete {
            version: status.version.clone(),
        }),
        UpdateState::UPDATE_STATE_FAILED => Some(VehicleUpdateState::Failed {
            error: if status.error.is_empty() {
                "update failed".to_string()
            } else {
                status.error.clone()
            },
        }),
        UpdateState::UPDATE_STATE_UNSPECIFIED => None,
    }
}

struct OtaStatusListener {
    campaigns: CampaignStore,
    campaign_tx: broadcast::Sender<CampaignEvent>,
}

#[async_trait::async_trait]
impl UListener for OtaStatusListener {
    async fn on_receive(&self, message: UMessage) {
        let status = match message.extract_protobuf::<OtaStatus>() {
            Ok(status) => status,
            Err(e) => {
                warn!("ignoring OTA notification with invalid payload: {e}");
                return;
            }
        };

        let Some(state) = to_vehicle_state(&status) else {
            debug!(vin = %status.vin, "ignoring OTA notification with unspecified state");
            return;
        };

        // The agent reports (vin, action_id); the dashboard is organised by
        // campaign. HawkBit action ids are not tracked here, so the vehicle's
        // one in-flight campaign is the correlation key. A vehicle can only be
        // in one active campaign at a time, which the API enforces on create.
        let Some(campaign_id) = self.campaigns.active_campaign_for_vin(&status.vin) else {
            debug!(
                vin = %status.vin,
                action_id = status.action_id,
                "OTA notification for a vehicle with no active campaign"
            );
            return;
        };

        if let Some(state) = self
            .campaigns
            .set_vehicle_state(&campaign_id, &status.vin, state)
        {
            debug!(vin = %status.vin, ?state, "OTA state from uProtocol notification");
            let _ = self.campaign_tx.send(CampaignEvent {
                campaign_id,
                vin: status.vin,
                state,
            });
        }
    }
}

/// Configuration for the OTA notification listener.
pub struct OtaListenerConfig {
    /// This component's own uProtocol address; agents address notifications here.
    pub uri: String,
    pub zenoh_config_path: String,
}

impl OtaListenerConfig {
    pub fn from_env() -> Self {
        Self {
            uri: std::env::var("UP_URI")
                .unwrap_or_else(|_| "up://fms-ota-orchestrator/D103/1/0".into()),
            zenoh_config_path: std::env::var("ZENOH_CONFIG_PATH")
                .unwrap_or_else(|_| "/zenoh-config.json5".into()),
        }
    }
}

/// Open the transport and register the OTA notification listener.
///
/// Returns the transport, which the caller must keep alive: dropping it
/// deregisters the listener.
pub async fn start(
    config: OtaListenerConfig,
    campaigns: CampaignStore,
    campaign_tx: broadcast::Sender<CampaignEvent>,
) -> Result<Arc<dyn UTransport>, String> {
    let uri = UUri::from_str(&config.uri).map_err(|e: UUriError| format!("bad UP_URI: {e}"))?;
    let uri_provider = Arc::new(
        StaticUriProvider::try_from(&uri).map_err(|e| format!("cannot build URI provider: {e}"))?,
    );

    let zenoh_config = zenoh::Config::from_file(&config.zenoh_config_path)
        .map_err(|e| format!("cannot read zenoh config {}: {e}", config.zenoh_config_path))?;
    let transport: Arc<dyn UTransport> =
        UPTransportZenoh::new(zenoh_config, uri_provider.get_source_uri())
            .await
            .map(Arc::new)
            .map_err(|e| format!("cannot open zenoh transport: {e}"))?;

    let listener = Arc::new(OtaStatusListener {
        campaigns,
        campaign_tx,
    });

    // Any authority may notify us (one per vehicle), so filter on the sink:
    // this component's own address.
    let source_filter = UUri::any();
    transport
        .register_listener(&source_filter, Some(&uri), listener)
        .await
        .map_err(|e| format!("cannot register OTA listener: {e}"))?;

    info!(uri = %config.uri, "listening for OTA notifications over uProtocol");
    Ok(transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_with(state: UpdateState) -> OtaStatus {
        let mut status = OtaStatus::new();
        status.vin = "VIN-0001".into();
        status.action_id = 7;
        status.state = state.into();
        status.version = "2.0.0".into();
        status
    }

    #[test]
    fn maps_in_progress_states() {
        assert!(matches!(
            to_vehicle_state(&status_with(UpdateState::UPDATE_STATE_PENDING)),
            Some(VehicleUpdateState::Pending)
        ));
        assert!(matches!(
            to_vehicle_state(&status_with(UpdateState::UPDATE_STATE_DOWNLOADING)),
            Some(VehicleUpdateState::Downloading)
        ));
        assert!(matches!(
            to_vehicle_state(&status_with(UpdateState::UPDATE_STATE_INSTALLING)),
            Some(VehicleUpdateState::Installing)
        ));
    }

    #[test]
    fn complete_carries_the_version() {
        let state = to_vehicle_state(&status_with(UpdateState::UPDATE_STATE_COMPLETE));
        match state {
            Some(VehicleUpdateState::Complete { version }) => assert_eq!(version, "2.0.0"),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn failed_carries_the_error() {
        let mut status = status_with(UpdateState::UPDATE_STATE_FAILED);
        status.error = "disk full".into();
        match to_vehicle_state(&status) {
            Some(VehicleUpdateState::Failed { error }) => assert_eq!(error, "disk full"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn failed_without_detail_gets_a_placeholder() {
        match to_vehicle_state(&status_with(UpdateState::UPDATE_STATE_FAILED)) {
            Some(VehicleUpdateState::Failed { error }) => assert_eq!(error, "update failed"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn unspecified_state_is_ignored() {
        assert!(to_vehicle_state(&status_with(UpdateState::UPDATE_STATE_UNSPECIFIED)).is_none());
    }
}
