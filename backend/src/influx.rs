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

//! Fleet position ingest from InfluxDB.
//!
//! Telemetry reaches the back end as uProtocol Publish messages, which the
//! blueprint's FMS Consumer already writes to InfluxDB. Rather than adding a
//! second subscriber to the vehicle status topic, we read the positions back
//! out of InfluxDB. That keeps this component out of the uProtocol data path
//! and avoids duplicating a component the blueprint already maintains.
//!
//! Field and measurement names mirror `components/influx-client/src/lib.rs` in
//! the blueprint; they are the contract between the FMS Consumer and us.

use std::time::Duration;

use tokio::sync::broadcast;
use tracing::{debug, error, warn};

use crate::{models::PositionEvent, store::Store};

const MEASUREMENT_SNAPSHOT: &str = "snapshot";
const FIELD_LATITUDE: &str = "latitude";
const FIELD_LONGITUDE: &str = "longitude";
const TAG_VIN: &str = "vin";

/// How far back each poll looks. Comfortably wider than the poll interval so a
/// slow write or a brief consumer stall does not produce a gap.
const LOOKBACK: &str = "-30s";

#[derive(Debug, Clone)]
pub struct InfluxConfig {
    pub uri: String,
    pub org: String,
    pub bucket: String,
    pub token: String,
    pub poll_interval: Duration,
}

impl InfluxConfig {
    /// Build the configuration from the environment.
    ///
    /// `INFLUXDB_TOKEN_FILE` takes precedence over `INFLUXDB_TOKEN`: the
    /// blueprint's InfluxDB init script writes the generated token to a file on
    /// a shared volume, which is how every other component picks it up.
    pub fn from_env() -> Result<Self, String> {
        let token = match std::env::var("INFLUXDB_TOKEN_FILE") {
            Ok(path) => std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read INFLUXDB_TOKEN_FILE {path}: {e}"))?
                .trim()
                .to_string(),
            Err(_) => std::env::var("INFLUXDB_TOKEN")
                .map_err(|_| "neither INFLUXDB_TOKEN_FILE nor INFLUXDB_TOKEN is set".to_string())?,
        };

        if token.is_empty() {
            return Err("InfluxDB token is empty".to_string());
        }

        Ok(Self {
            uri: std::env::var("INFLUXDB_URI").unwrap_or_else(|_| "http://influxdb:8086".into()),
            org: std::env::var("INFLUXDB_ORG").unwrap_or_else(|_| "sdv".into()),
            bucket: std::env::var("INFLUXDB_BUCKET").unwrap_or_else(|_| "demo".into()),
            token,
            poll_interval: Duration::from_millis(
                std::env::var("INFLUXDB_POLL_INTERVAL_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1000),
            ),
        })
    }
}

/// Latest known position for one vehicle.
#[derive(Debug, PartialEq)]
pub struct VehiclePosition {
    pub vin: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Flux query returning the most recent latitude and longitude per VIN.
///
/// Grouping by `vin` and `_field` before `last()` gives exactly one row per
/// signal per vehicle; the second `group` plus `pivot` then collapses the two
/// signals into a single row per vehicle.
fn position_query(bucket: &str) -> String {
    format!(
        r#"from(bucket: "{bucket}")
  |> range(start: {LOOKBACK})
  |> filter(fn: (r) => r._measurement == "{MEASUREMENT_SNAPSHOT}")
  |> filter(fn: (r) => r._field == "{FIELD_LATITUDE}" or r._field == "{FIELD_LONGITUDE}")
  |> group(columns: ["{TAG_VIN}", "_field"])
  |> last()
  |> keep(columns: ["{TAG_VIN}", "_field", "_value"])
  |> group(columns: ["{TAG_VIN}"])
  |> pivot(rowKey: ["{TAG_VIN}"], columnKey: ["_field"], valueColumn: "_value")"#
    )
}

/// Parse InfluxDB's annotated CSV response into one position per vehicle.
///
/// The response carries `#datatype`/`#group`/`#default` annotation lines, then a
/// header row, then the data. Column order is not guaranteed, so the header row
/// is used to locate the columns by name.
pub fn parse_positions(csv: &str) -> Vec<VehiclePosition> {
    let mut out = Vec::new();
    let mut columns: Option<Vec<String>> = None;

    for line in csv.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split(',').collect();

        // A row naming the vin column is a header row. Influx repeats the header
        // for each result table, so this can legitimately happen more than once.
        if fields.contains(&TAG_VIN) {
            columns = Some(fields.iter().map(|f| f.to_string()).collect());
            continue;
        }

        let Some(cols) = columns.as_ref() else {
            continue;
        };

        let value_of = |name: &str| {
            cols.iter()
                .position(|c| c == name)
                .and_then(|i| fields.get(i))
                .map(|v| v.trim())
        };

        let (Some(vin), Some(lat), Some(lon)) = (
            value_of(TAG_VIN),
            value_of(FIELD_LATITUDE),
            value_of(FIELD_LONGITUDE),
        ) else {
            continue;
        };

        let (Ok(latitude), Ok(longitude)) = (lat.parse::<f64>(), lon.parse::<f64>()) else {
            warn!("skipping row with unparsable position for {vin}: {lat},{lon}");
            continue;
        };

        if vin.is_empty() {
            continue;
        }

        out.push(VehiclePosition {
            vin: vin.to_string(),
            latitude,
            longitude,
        });
    }

    out
}

async fn query_positions(
    client: &reqwest::Client,
    config: &InfluxConfig,
) -> Result<Vec<VehiclePosition>, String> {
    let response = client
        .post(format!("{}/api/v2/query", config.uri.trim_end_matches('/')))
        .query(&[("org", config.org.as_str())])
        .header("Authorization", format!("Token {}", config.token))
        .header("Accept", "application/csv")
        .header("Content-Type", "application/vnd.flux")
        .body(position_query(&config.bucket))
        .send()
        .await
        .map_err(|e| format!("InfluxDB query failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("InfluxDB query returned {status}: {body}"));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("cannot read InfluxDB response: {e}"))?;

    Ok(parse_positions(&body))
}

/// Poll InfluxDB forever, applying position changes to the store and
/// broadcasting them to connected websocket clients.
///
/// Only actual movement is broadcast. The FMS Forwarder reports on a timer, so
/// a stationary vehicle would otherwise generate an event on every poll.
pub async fn run(config: InfluxConfig, store: Store, tx: broadcast::Sender<PositionEvent>) {
    let client = reqwest::Client::new();
    let mut ticker = tokio::time::interval(config.poll_interval);

    loop {
        ticker.tick().await;

        let positions = match query_positions(&client, &config).await {
            Ok(positions) => positions,
            Err(e) => {
                error!("{e}");
                continue;
            }
        };

        for position in positions {
            let Some(current) = store.get(&position.vin) else {
                // The fleet is seeded from vehicles.json; a VIN we do not know
                // about means InfluxDB holds data for a vehicle we are not
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

    #[test]
    fn parse_positions_reads_annotated_csv() {
        let csv = "\
#datatype,string,long,string,double,double\r
#group,false,false,true,false,false\r
#default,_result,,,,\r
,result,table,vin,latitude,longitude\r
,,0,VIN-0001,48.8566,2.3522\r
,,1,VIN-0002,48.8601,2.3488\r
";
        let positions = parse_positions(csv);
        assert_eq!(positions.len(), 2);
        assert_eq!(
            positions[0],
            VehiclePosition {
                vin: "VIN-0001".to_string(),
                latitude: 48.8566,
                longitude: 2.3522,
            }
        );
        assert_eq!(positions[1].vin, "VIN-0002");
    }

    #[test]
    fn parse_positions_tolerates_column_reordering() {
        let csv = ",result,table,longitude,vin,latitude\n,,0,2.3522,VIN-0001,48.8566\n";
        let positions = parse_positions(csv);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].latitude, 48.8566);
        assert_eq!(positions[0].longitude, 2.3522);
    }

    #[test]
    fn parse_positions_handles_repeated_headers() {
        let csv = "\
,result,table,vin,latitude,longitude
,,0,VIN-0001,48.8566,2.3522

,result,table,vin,latitude,longitude
,,1,VIN-0002,48.8601,2.3488
";
        let positions = parse_positions(csv);
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[1].vin, "VIN-0002");
    }

    #[test]
    fn parse_positions_skips_unparsable_rows() {
        let csv = ",result,table,vin,latitude,longitude\n,,0,VIN-0001,not-a-number,2.3522\n";
        assert!(parse_positions(csv).is_empty());
    }

    #[test]
    fn parse_positions_returns_empty_for_empty_result() {
        assert!(parse_positions("").is_empty());
        assert!(parse_positions("#datatype,string\n").is_empty());
    }

    #[test]
    fn position_query_targets_the_configured_bucket() {
        let query = position_query("demo");
        assert!(query.contains(r#"from(bucket: "demo")"#));
        assert!(query.contains(MEASUREMENT_SNAPSHOT));
        assert!(query.contains(FIELD_LATITUDE));
        assert!(query.contains(FIELD_LONGITUDE));
    }
}
