use std::time::Duration;

use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::{models::PositionEvent, store::Store};

/// Broadcast the HawkBit gateway token to every ota-agent. Published with the
/// MQTT retained flag so agents that connect later (or reconnect) pick it up
/// from the broker without needing the backend to re-announce. Without this
/// token, agents can't authenticate to HawkBit's DDI API and can't self-
/// register.
pub async fn publish_gateway_token(client: &AsyncClient, token: &str) {
    if let Err(e) = client
        .publish("fleet/gateway-token", QoS::AtLeastOnce, true, token)
        .await
    {
        warn!("failed to publish fleet/gateway-token: {e}");
    }
}

/// Build the async MQTT client + event loop. The backend subscribes only to
/// telemetry (lat/lon) now — OTA dispatch and feedback happen over HawkBit's
/// DDI API directly between HawkBit and the ota-agent.
pub async fn connect(mqtt_host: &str, mqtt_port: u16) -> (AsyncClient, EventLoop) {
    let mut opts = MqttOptions::new("fleet-backend", mqtt_host, mqtt_port);
    opts.set_keep_alive(Duration::from_secs(30));

    let (client, eventloop) = AsyncClient::new(opts, 256);

    client
        .subscribe("kuksa/+/telemetry/#", QoS::AtLeastOnce)
        .await
        .expect("failed to subscribe to kuksa telemetry");

    info!(
        "MQTT connected to {}:{}, subscribed to kuksa/+/telemetry/#",
        mqtt_host, mqtt_port
    );

    (client, eventloop)
}

/// Drive the MQTT event loop forever, dispatching telemetry messages.
pub async fn run(
    mut eventloop: EventLoop,
    store: Store,
    tx: broadcast::Sender<PositionEvent>,
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
                    if parts[1] == "telemetry" {
                        handle_signal(&store, &tx, vin, parts[2], payload);
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

// ── Telemetry ────────────────────────────────────────────────────────────────

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
