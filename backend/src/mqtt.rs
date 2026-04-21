use std::{sync::Arc, time::Duration};

use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{
    campaign::{CampaignEvent, CampaignStore, VehicleUpdateState},
    hawkbit::HawkbitClient,
    models::PositionEvent,
    store::Store,
};

/// Publish an OTA command to `kuksa/{vin}/ota/command`. QoS 1, not retained —
/// if the ota-agent is offline at publish time, the command is lost and the
/// vehicle stays in `PENDING` (see specs-v2.md §6).
pub async fn publish_ota_command(
    client: &AsyncClient,
    vin: &str,
    campaign_id: Uuid,
    version: &str,
) {
    let topic = format!("kuksa/{}/ota/command", vin);
    let payload = serde_json::json!({
        "campaign_id": campaign_id,
        "version":     version,
    })
    .to_string();
    if let Err(e) = client
        .publish(&topic, QoS::AtLeastOnce, false, payload)
        .await
    {
        warn!("failed to publish {}: {}", topic, e);
    }
}

/// Build the async MQTT client + event loop and issue both v1 and v2 subscriptions.
///
/// Splitting this from [`run`] lets `main` hold onto the `AsyncClient` so HTTP
/// handlers (e.g. `POST /campaigns`) can publish `kuksa/{vin}/ota/command`.
pub async fn connect(mqtt_host: &str, mqtt_port: u16) -> (AsyncClient, EventLoop) {
    let mut opts = MqttOptions::new("fleet-backend", mqtt_host, mqtt_port);
    opts.set_keep_alive(Duration::from_secs(30));

    let (client, eventloop) = AsyncClient::new(opts, 256);

    client
        .subscribe("kuksa/+/telemetry/#", QoS::AtLeastOnce)
        .await
        .expect("failed to subscribe to kuksa telemetry");
    client
        .subscribe("kuksa/+/ota/status", QoS::AtLeastOnce)
        .await
        .expect("failed to subscribe to kuksa ota status");

    info!(
        "MQTT connected to {}:{}, subscribed to kuksa/+/telemetry/# and kuksa/+/ota/status",
        mqtt_host, mqtt_port
    );

    (client, eventloop)
}

/// Drive the MQTT event loop forever, dispatching telemetry and OTA messages.
pub async fn run(
    mut eventloop: EventLoop,
    store: Store,
    tx: broadcast::Sender<PositionEvent>,
    campaigns: CampaignStore,
    campaign_tx: broadcast::Sender<CampaignEvent>,
    hawkbit: Arc<HawkbitClient>,
) {
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let topic = p.topic.as_str();
                let payload = match std::str::from_utf8(&p.payload) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                if let Some(rest) = topic.strip_prefix("kuksa/") {
                    let parts: Vec<&str> = rest.splitn(3, '/').collect();
                    if parts.len() < 3 {
                        continue;
                    }
                    let vin = parts[0];
                    match parts[1] {
                        "telemetry" => handle_signal(&store, &tx, vin, parts[2], payload),
                        "ota" if parts[2] == "status" => {
                            handle_ota_status(
                                &store,
                                &campaigns,
                                &campaign_tx,
                                hawkbit.clone(),
                                vin,
                                payload,
                            )
                            .await
                        }
                        _ => {}
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                error!("MQTT error: {e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

// ── Telemetry (v1) ───────────────────────────────────────────────────────────

fn handle_signal(
    store: &Store,
    tx: &broadcast::Sender<PositionEvent>,
    vin: &str,
    signal: &str,
    value: String,
) {
    match signal {
        "VehicleIdentification/VIN" => {
            store.update_string(vin, |r| r.vin = value);
        }
        "VehicleIdentification/Brand" => {
            store.update_string(vin, |r| r.brand = value);
        }
        "VehicleIdentification/Model" => {
            store.update_string(vin, |r| r.model = value);
        }
        "CurrentLocation/Latitude" => {
            let lat: f64 = match value.parse() {
                Ok(v) => v,
                Err(_) => {
                    warn!("bad latitude for {vin}: {value}");
                    return;
                }
            };
            if let Some((new_lat, lon)) = store.update_position(vin, Some(lat), None) {
                debug!("broadcasting PositionEvent for {vin}: lat={new_lat}, lon={lon}");
                let _ = tx.send(PositionEvent {
                    vin: vin.to_string(),
                    lat: new_lat,
                    lon,
                });
            } else {
                warn!("received latitude for unknown VIN: {vin}");
            }
        }
        "CurrentLocation/Longitude" => {
            let lon: f64 = match value.parse() {
                Ok(v) => v,
                Err(_) => {
                    warn!("bad longitude for {vin}: {value}");
                    return;
                }
            };
            if let Some((lat, new_lon)) = store.update_position(vin, None, Some(lon)) {
                debug!("broadcasting PositionEvent for {vin}: lat={lat}, lon={new_lon}");
                let _ = tx.send(PositionEvent {
                    vin: vin.to_string(),
                    lat,
                    lon: new_lon,
                });
            } else {
                warn!("received longitude for unknown VIN: {vin}");
            }
        }
        _ => {}
    }
}

// ── OTA status (v2) ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OtaStatus {
    campaign_id: Uuid,
    vin: String,
    state: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

async fn handle_ota_status(
    store: &Store,
    campaigns: &CampaignStore,
    campaign_tx: &broadcast::Sender<CampaignEvent>,
    hawkbit: Arc<HawkbitClient>,
    vin: &str,
    payload: String,
) {
    let status: OtaStatus = match serde_json::from_str(&payload) {
        Ok(s) => s,
        Err(e) => {
            warn!("bad ota/status payload for {vin}: {e}");
            return;
        }
    };

    let state = match status.state.as_str() {
        "PENDING" => VehicleUpdateState::Pending,
        "DOWNLOADING" => VehicleUpdateState::Downloading,
        "INSTALLING" => VehicleUpdateState::Installing,
        "COMPLETE" => VehicleUpdateState::Complete {
            version: status.version.clone().unwrap_or_default(),
        },
        "FAILED" => VehicleUpdateState::Failed {
            error: status.error.clone().unwrap_or_else(|| "unknown".into()),
        },
        other => {
            warn!("unknown ota state for {vin}: {other}");
            return;
        }
    };

    // Spec §4 / §7.3 say the DashMap should be updated *from the HawkBit
    // response* after feedback, keeping a single write path. In practice the
    // feedback endpoint returns no per-vehicle state body, so we update the
    // store from the MQTT payload directly and rely on the 5 s `poll_rollouts`
    // task in main.rs to reconcile against HawkBit if they ever diverge.
    //
    // Report terminal states back to HawkBit so it reflects reality without
    // waiting for the 5s poll cycle. The deployment/action id isn't carried on
    // the MQTT status payload yet; skip the feedback call if we can't match one.
    if matches!(
        state,
        VehicleUpdateState::Complete { .. } | VehicleUpdateState::Failed { .. }
    ) {
        if let Err(e) = hawkbit.report_feedback(&status.vin, 0, true).await {
            if e.is_unreachable() {
                warn!(
                    "hawkbit unreachable during feedback for {}: {}",
                    status.vin, e
                );
            } else {
                warn!("hawkbit feedback failed for {}: {}", status.vin, e);
            }
        }
    }

    // Update the campaign store.
    if let Some(new_state) =
        campaigns.set_vehicle_state(&status.campaign_id, &status.vin, state.clone())
    {
        // If the update completed successfully, also reflect the version in
        // the fleet store so GET /fleet shows the new SoftwareVersion.
        if let VehicleUpdateState::Complete { version } = &new_state {
            let v = version.clone();
            store.update_string(&status.vin, |r| r.software_version = v);
        }

        let _ = campaign_tx.send(CampaignEvent {
            campaign_id: status.campaign_id,
            vin: status.vin,
            state: new_state,
        });
    } else {
        debug!(
            "received ota status for unknown campaign {} / vin {}",
            status.campaign_id, status.vin
        );
    }
}
