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

//! Reporting OTA state transitions over uProtocol.
//!
//! Every transition is sent as a uProtocol Notification addressed to the back
//! end orchestrator. A Notification has a single distinct addressee, which fits
//! a status report better than a Publish would, and it is what the blueprint
//! maintainers suggested for this direction of traffic.
//!
//! The agent still drives HawkBit's DDI API over HTTP; only the reporting path
//! runs over uProtocol. That keeps the flow working on both the Zenoh and Hono
//! transports, whereas a full uProtocol-to-DDI bridge would need a
//! cloud-to-vehicle channel that does not exist upstream yet.

use std::{str::FromStr, sync::Arc, time::SystemTime};

use log::{debug, warn};
use up_rust::{
    communication::{CallOptions, Notifier, SimpleNotifier, UPayload},
    LocalUriProvider, StaticUriProvider, UTransport, UUri,
};
use up_transport_zenoh::UPTransportZenoh;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/ota_proto/mod.rs"));
}

pub use proto::ota::{OtaStatus, UpdateState};

/// Resource identifier this agent notifies from.
///
/// uProtocol reserves 0x8000..=0xFFFE for topic-like resources, which is what a
/// notification origin has to be.
const OTA_STATUS_RESOURCE_ID: u16 = 0x8001;

/// Publishes OTA state transitions to the back end.
pub struct OtaReporter {
    notifier: SimpleNotifier,
    destination: UUri,
}

impl OtaReporter {
    /// Connect to the Zenoh transport and prepare a notifier.
    ///
    /// `source` is this agent's own address (one authority per vehicle, so the
    /// back end can tell them apart); `destination` is the orchestrator.
    pub async fn connect(
        source_uri: &str,
        destination_uri: &str,
        zenoh_config_path: &str,
    ) -> Result<Self, String> {
        let source = UUri::from_str(source_uri).map_err(|e| format!("bad source URI: {e}"))?;
        let destination =
            UUri::from_str(destination_uri).map_err(|e| format!("bad destination URI: {e}"))?;

        let uri_provider = Arc::new(
            StaticUriProvider::try_from(&source)
                .map_err(|e| format!("cannot build URI provider: {e}"))?,
        );

        let zenoh_config = zenoh::Config::from_file(zenoh_config_path)
            .map_err(|e| format!("cannot read zenoh config {zenoh_config_path}: {e}"))?;
        let transport: Arc<dyn UTransport> =
            UPTransportZenoh::new(zenoh_config, uri_provider.get_source_uri())
                .await
                .map(Arc::new)
                .map_err(|e| format!("cannot open zenoh transport: {e}"))?;

        Ok(Self {
            notifier: SimpleNotifier::new(transport, uri_provider),
            destination,
        })
    }

    /// Report one state transition.
    ///
    /// Failures are logged rather than propagated: the DDI feedback the agent
    /// sends alongside this is the authoritative record, and the back end
    /// reconciles against HawkBit anyway, so a dropped notification must not
    /// stall the update.
    pub async fn report(&self, status: OtaStatus) {
        let vin = status.vin.clone();
        let state = status.state.enum_value_or_default();

        let payload = match UPayload::try_from_protobuf(status) {
            Ok(payload) => payload,
            Err(e) => {
                warn!("[{vin}] cannot encode OTA status: {e}");
                return;
            }
        };

        match self
            .notifier
            .notify(
                OTA_STATUS_RESOURCE_ID,
                &self.destination,
                CallOptions::for_notification(None, None, None),
                Some(payload),
            )
            .await
        {
            Ok(()) => debug!("[{vin}] OTA status notified: {state:?}"),
            Err(e) => warn!("[{vin}] OTA status notification failed for {state:?}: {e}"),
        }
    }
}

/// Build a status message stamped with the current time.
pub fn status(
    vin: &str,
    action_id: u64,
    state: UpdateState,
    version: &str,
    error: Option<&str>,
) -> OtaStatus {
    let mut status = OtaStatus::new();
    status.vin = vin.to_string();
    status.action_id = action_id;
    status.state = state.into();
    status.version = version.to_string();
    status.error = error.unwrap_or_default().to_string();
    status.timestamp_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    status
}
