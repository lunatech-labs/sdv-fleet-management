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

//! Fleet position ingest from the blueprint rFMS API.
//!
//! The blueprint FMS Consumer writes vehicle status to InfluxDB, and FMS Server
//! serves it as the rFMS API. This module reads that API instead of reading
//! InfluxDB directly.
//!
//! The reason is interoperability, not convenience. An operator dashboard that
//! speaks rFMS works against any backend that implements the specification, and
//! the telemetry owner does not have to hand out database credentials. It also
//! gives the rFMS endpoints a real consumer. Reading InfluxDB directly is
//! faster to write, and it couples this component to a schema that belongs to
//! the FMS Consumer.
//!
//! Two limits of the current blueprint implementation shape the code below.
//! `docs/rfms-coverage.md` records both in full.
//!
//! 1. `/rfms/vehicles` returns the VIN and nothing else, so vehicle identity
//!    comes from the local registry file. The join disappears when the
//!    blueprint fills those fields in.
//! 2. `latestOnly=true` returns the latest position for each trigger type
//!    rather than for each vehicle (upstream issue #33), so this module keeps
//!    the newest entry per VIN itself.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{models::PositionEvent, store::Store};

#[derive(Debug, Clone)]
pub struct RfmsConfig {
    pub base_uri: String,
    pub poll_interval: Duration,
}

impl RfmsConfig {
    pub fn from_env() -> Self {
        Self {
            base_uri: std::env::var("FMS_SERVER_URI")
                .unwrap_or_else(|_| "http://fms-server:8081".into()),
            poll_interval: Duration::from_millis(
                std::env::var("FLEET_POLL_INTERVAL_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1000),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// rFMS response types. Only the fields this dashboard reads are modelled.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct VehiclePositionResponse {
    #[serde(rename = "vehiclePositionResponse")]
    response: VehiclePositions,
}

#[derive(Debug, Deserialize)]
struct VehiclePositions {
    #[serde(rename = "vehiclePositions")]
    #[serde(default)]
    positions: Vec<VehiclePositionObject>,
}

#[derive(Debug, Deserialize)]
struct VehiclePositionObject {
    vin: String,
    #[serde(rename = "createdDateTime")]
    created_date_time: DateTime<Utc>,
    #[serde(rename = "gnssPosition")]
    gnss_position: Option<GnssPosition>,
}

#[derive(Debug, Deserialize)]
struct GnssPosition {
    latitude: f64,
    longitude: f64,
}

#[derive(Debug, Deserialize)]
struct VehicleResponse {
    #[serde(rename = "vehicleResponse")]
    response: Vehicles,
}

#[derive(Debug, Deserialize)]
struct Vehicles {
    #[serde(default)]
    vehicles: Vec<VehicleObject>,
}

#[derive(Debug, Deserialize)]
struct VehicleObject {
    vin: String,
}

/// Latest known position for one vehicle.
#[derive(Debug, PartialEq)]
pub struct VehiclePosition {
    pub vin: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Reduce an rFMS position response to one position per vehicle.
///
/// Entries without a `gnssPosition` are skipped. The blueprint omits the
/// position unless latitude, longitude and the position timestamp are all
/// present, so a recording without `Vehicle.CurrentLocation.Timestamp` yields
/// nothing here.
///
/// Where a VIN appears more than once, the newest `createdDateTime` wins. This
/// is the client-side workaround for upstream issue #33.
pub fn latest_per_vehicle(body: &str) -> Result<Vec<VehiclePosition>, String> {
    let parsed: VehiclePositionResponse =
        serde_json::from_str(body).map_err(|e| format!("cannot parse rFMS positions: {e}"))?;

    let mut newest: HashMap<String, (DateTime<Utc>, VehiclePosition)> = HashMap::new();

    for entry in parsed.response.positions {
        let Some(gnss) = entry.gnss_position else {
            continue;
        };

        let candidate = VehiclePosition {
            vin: entry.vin.clone(),
            latitude: gnss.latitude,
            longitude: gnss.longitude,
        };

        match newest.get(&entry.vin) {
            Some((seen, _)) if *seen >= entry.created_date_time => {}
            _ => {
                newest.insert(entry.vin, (entry.created_date_time, candidate));
            }
        }
    }

    let mut out: Vec<VehiclePosition> = newest.into_values().map(|(_, p)| p).collect();
    out.sort_by(|a, b| a.vin.cmp(&b.vin));
    Ok(out)
}

/// Extract the VIN list from an rFMS vehicles response.
pub fn vehicle_vins(body: &str) -> Result<Vec<String>, String> {
    let parsed: VehicleResponse =
        serde_json::from_str(body).map_err(|e| format!("cannot parse rFMS vehicles: {e}"))?;
    Ok(parsed
        .response
        .vehicles
        .into_iter()
        .map(|v| v.vin)
        .collect())
}

async fn get(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("rFMS request to {url} failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("rFMS request to {url} returned {status}: {body}"));
    }

    response
        .text()
        .await
        .map_err(|e| format!("cannot read rFMS response from {url}: {e}"))
}

/// Log how the rFMS vehicle list compares with the local registry.
///
/// This runs once at startup. It is a diagnostic, not a gate: the dashboard
/// serves the registry regardless, because a vehicle that has not reported yet
/// is absent from `/rfms/vehicles`.
async fn report_vehicle_coverage(client: &reqwest::Client, base: &str, store: &Store) {
    let url = format!("{base}/rfms/vehicles");
    match get(client, &url).await.and_then(|b| vehicle_vins(&b)) {
        Ok(vins) => {
            let known: Vec<String> = store.all().into_iter().map(|v| v.vin).collect();
            let missing: Vec<&String> = known.iter().filter(|v| !vins.contains(v)).collect();
            info!(
                "rFMS reports {} vehicle(s); the registry holds {}",
                vins.len(),
                known.len()
            );
            if !missing.is_empty() {
                warn!("no rFMS telemetry yet for: {missing:?}");
            }
        }
        Err(e) => warn!("cannot read the rFMS vehicle list: {e}"),
    }
}

/// Poll the rFMS API forever, applying position changes to the store and
/// broadcasting them to connected websocket clients.
///
/// Only actual movement is broadcast. The FMS Forwarder reports on a timer, so
/// a stationary vehicle would otherwise generate an event on every poll.
pub async fn run(config: RfmsConfig, store: Store, tx: broadcast::Sender<PositionEvent>) {
    let client = reqwest::Client::new();
    let base = config.base_uri.trim_end_matches('/').to_string();
    let positions_url = format!("{base}/rfms/vehiclepositions?latestOnly=true");

    report_vehicle_coverage(&client, &base, &store).await;

    let mut ticker = tokio::time::interval(config.poll_interval);

    loop {
        ticker.tick().await;

        let positions = match get(&client, &positions_url)
            .await
            .and_then(|body| latest_per_vehicle(&body))
        {
            Ok(positions) => positions,
            Err(e) => {
                error!("{e}");
                continue;
            }
        };

        for position in positions {
            let Some(current) = store.get(&position.vin) else {
                // The fleet comes from the registry. A VIN we do not know about
                // means the rFMS API holds data for a vehicle we are not
                // tracking, which is not something we should invent a record for.
                debug!("ignoring position for untracked VIN {}", position.vin);
                continue;
            };

            if current.latitude == position.latitude && current.longitude == position.longitude {
                continue;
            }

            if let Some((lat, lon)) = store.update_position(
                &position.vin,
                Some(position.latitude),
                Some(position.longitude),
            ) {
                debug!(
                    "broadcasting PositionEvent for {}: {lat},{lon}",
                    position.vin
                );
                let _ = tx.send(PositionEvent {
                    vin: position.vin,
                    lat,
                    lon,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_VEHICLES: &str = r#"{
      "moreDataAvailable": false,
      "vehiclePositionResponse": {
        "vehiclePositions": [
          {
            "vin": "VIN-0001",
            "createdDateTime": "2026-09-02T12:55:58.875Z",
            "gnssPosition": { "latitude": 48.8563482, "longitude": 2.3513531 }
          },
          {
            "vin": "VIN-0002",
            "createdDateTime": "2026-09-02T12:55:59.100Z",
            "gnssPosition": { "latitude": 48.8601, "longitude": 2.3488 }
          }
        ]
      }
    }"#;

    #[test]
    fn reads_one_position_per_vehicle() {
        let positions = latest_per_vehicle(TWO_VEHICLES).unwrap();
        assert_eq!(positions.len(), 2);
        assert_eq!(
            positions[0],
            VehiclePosition {
                vin: "VIN-0001".to_string(),
                latitude: 48.8563482,
                longitude: 2.3513531,
            }
        );
        assert_eq!(positions[1].vin, "VIN-0002");
    }

    #[test]
    fn keeps_the_newest_entry_per_vin() {
        // Upstream issue #33: latestOnly returns the latest position for each
        // trigger type, so the same VIN arrives more than once.
        let body = r#"{
          "vehiclePositionResponse": {
            "vehiclePositions": [
              {
                "vin": "VIN-0001",
                "createdDateTime": "2026-09-02T12:00:00Z",
                "gnssPosition": { "latitude": 1.0, "longitude": 1.0 }
              },
              {
                "vin": "VIN-0001",
                "createdDateTime": "2026-09-02T12:00:05Z",
                "gnssPosition": { "latitude": 2.0, "longitude": 2.0 }
              },
              {
                "vin": "VIN-0001",
                "createdDateTime": "2026-09-02T12:00:02Z",
                "gnssPosition": { "latitude": 3.0, "longitude": 3.0 }
              }
            ]
          }
        }"#;
        let positions = latest_per_vehicle(body).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].latitude, 2.0);
    }

    #[test]
    fn skips_entries_without_a_position() {
        // What the blueprint returns when the recording carries no
        // Vehicle.CurrentLocation.Timestamp row.
        let body = r#"{
          "vehiclePositionResponse": {
            "vehiclePositions": [
              {
                "vin": "VIN-0001",
                "createdDateTime": "2026-09-02T12:00:00Z",
                "wheelBasedSpeed": 2.9
              }
            ]
          }
        }"#;
        assert!(latest_per_vehicle(body).unwrap().is_empty());
    }

    #[test]
    fn tolerates_an_empty_response() {
        let body = r#"{"vehiclePositionResponse": {}}"#;
        assert!(latest_per_vehicle(body).unwrap().is_empty());
    }

    #[test]
    fn rejects_a_malformed_response() {
        assert!(latest_per_vehicle("not json").is_err());
    }

    #[test]
    fn reads_the_vehicle_list() {
        // The blueprint returns the VIN and nothing else. See
        // docs/rfms-coverage.md.
        let body = r#"{
          "moreDataAvailable": false,
          "vehicleResponse": {
            "vehicles": [{ "vin": "VIN-0001" }, { "vin": "VIN-0002" }]
          }
        }"#;
        assert_eq!(vehicle_vins(body).unwrap(), vec!["VIN-0001", "VIN-0002"]);
    }

    #[test]
    fn config_defaults_to_the_compose_service_name() {
        let config = RfmsConfig::from_env();
        assert!(config.base_uri.contains("fms-server") || std::env::var("FMS_SERVER_URI").is_ok());
    }
}
