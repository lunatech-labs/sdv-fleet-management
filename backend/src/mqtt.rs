use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::models::PositionEvent;
use crate::store::Store;

pub async fn start(
    store: Store,
    tx: broadcast::Sender<PositionEvent>,
    mqtt_host: String,
    mqtt_port: u16,
) {
    let mut opts = MqttOptions::new("fleet-backend", &mqtt_host, mqtt_port);
    opts.set_keep_alive(Duration::from_secs(30));

    let (client, mut eventloop) = AsyncClient::new(opts, 256);

    client
        .subscribe("kuksa/+/telemetry/#", QoS::AtLeastOnce)
        .await
        .expect("failed to subscribe to kuksa telemetry");

    info!(
        "MQTT connected to {}:{}, subscribed to kuksa/+/telemetry/#",
        mqtt_host, mqtt_port
    );

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) => {
                debug!("MQTT publish received on topic: {}", p.topic);
                let topic = p.topic.as_str();
                let payload = match std::str::from_utf8(&p.payload) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                // Topic shape: kuksa/{vin}/telemetry/{signal_path}
                let parts: Vec<&str> = topic.splitn(4, '/').collect();
                if parts.len() < 4 {
                    continue;
                }
                let vin = parts[1];
                let signal = parts[3];

                handle_signal(&store, &tx, vin, signal, payload);
            }
            Ok(_) => {}
            Err(e) => {
                error!("MQTT error: {e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

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
