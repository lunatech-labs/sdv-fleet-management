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

//! Kuksa Databroker access.
//!
//! This uses `kuksa-rust-sdk`, the same client the blueprint FMS Forwarder
//! uses. An earlier version vendored the Kuksa protos and generated its own
//! gRPC stubs with tonic and prost, which meant the binary carried two protobuf
//! stacks: prost for Kuksa and rust-protobuf for the uProtocol payload, because
//! `up-rust` requires `protobuf::Message`. The SDK removes that duplication.

use std::time::Duration;

use kuksa_rust_sdk::kuksa::common::ClientTraitV2;
use kuksa_rust_sdk::kuksa::val::v2::KuksaClientV2;
use kuksa_rust_sdk::v2_proto;
use log::{debug, warn};
use tokio::time;

pub const VIN_SIGNAL: &str = "Vehicle.VehicleIdentification.VIN";
pub const SOFTWARE_VERSION_SIGNAL: &str = "Vehicle.SoftwareVersion";

/// Connect to a vehicle Databroker. Does not perform any I/O yet.
pub fn connect(host: &str, port: u16) -> Result<KuksaClientV2, String> {
    let uri = format!("http://{host}:{port}")
        .parse()
        .map_err(|e| format!("invalid Databroker URI for {host}:{port}: {e}"))?;
    Ok(KuksaClientV2::new(uri))
}

fn as_string(datapoint: Option<v2_proto::Datapoint>) -> Option<String> {
    match datapoint?.value?.typed_value? {
        v2_proto::value::TypedValue::String(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Read one string signal, or `None` when it is unset or not a string.
pub async fn read_string(client: &mut KuksaClientV2, path: &str) -> Option<String> {
    match client.get_value(path.to_string()).await {
        Ok(datapoint) => as_string(datapoint),
        Err(e) => {
            debug!("cannot read {path}: {e:?}");
            None
        }
    }
}

/// Wait for the vehicle's own VIN to appear in its Databroker.
///
/// The VIN is a signal like any other, and the CSV Provider publishes it near
/// the start of the recording. Reading it here rather than taking it from an
/// environment variable means every agent is configured identically and only
/// the Databroker it dials differs.
pub async fn wait_for_vin(client: &mut KuksaClientV2, retry: Duration) -> String {
    loop {
        if let Some(vin) = read_string(client, VIN_SIGNAL).await {
            return vin;
        }
        warn!(
            "{VIN_SIGNAL} is not available yet, retrying in {}s",
            retry.as_secs()
        );
        time::sleep(retry).await;
    }
}

/// Record an installed software version on the vehicle.
///
/// `Vehicle.SoftwareVersion` is not part of standard VSS. It comes from the
/// overlay in `spec/overlay/ota.vspec`, and it is what makes an install
/// verifiable at the Databroker rather than only in hawkBit.
pub async fn set_software_version(client: &mut KuksaClientV2, version: &str) -> Result<(), String> {
    let value = v2_proto::Value {
        typed_value: Some(v2_proto::value::TypedValue::String(version.to_string())),
    };
    client
        .publish_value(SOFTWARE_VERSION_SIGNAL.to_string(), value)
        .await
        .map_err(|e| format!("cannot set {SOFTWARE_VERSION_SIGNAL} to {version}: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_datapoint(value: &str) -> Option<v2_proto::Datapoint> {
        Some(v2_proto::Datapoint {
            timestamp: None,
            value: Some(v2_proto::Value {
                typed_value: Some(v2_proto::value::TypedValue::String(value.to_string())),
            }),
        })
    }

    #[test]
    fn reads_a_string_datapoint() {
        assert_eq!(
            as_string(string_datapoint("VIN-0001")),
            Some("VIN-0001".to_string())
        );
    }

    #[test]
    fn treats_an_empty_string_as_unset() {
        // The Databroker answers with an empty value before the CSV Provider
        // has published the VIN.
        assert_eq!(as_string(string_datapoint("")), None);
    }

    #[test]
    fn ignores_a_non_string_datapoint() {
        let datapoint = Some(v2_proto::Datapoint {
            timestamp: None,
            value: Some(v2_proto::Value {
                typed_value: Some(v2_proto::value::TypedValue::Bool(true)),
            }),
        });
        assert_eq!(as_string(datapoint), None);
    }

    #[test]
    fn ignores_a_missing_datapoint() {
        assert_eq!(as_string(None), None);
        assert_eq!(
            as_string(Some(v2_proto::Datapoint {
                timestamp: None,
                value: None,
            })),
            None
        );
    }

    #[test]
    fn connect_rejects_a_bad_host() {
        assert!(connect("not a host", 55556).is_err());
    }
}
