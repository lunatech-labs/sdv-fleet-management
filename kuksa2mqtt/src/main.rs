use std::{env, time::Duration};

use rand::SeedableRng;
use rand_distr::{Distribution, Normal};
use rumqttc::{AsyncClient, MqttOptions, QoS};
use tokio::time;
use tonic::transport::Channel;
use tracing::{error, info, warn};

// Generated gRPC client from the kuksa proto files.
pub mod kuksa {
    pub mod val {
        pub mod v1 {
            tonic::include_proto!("kuksa.val.v1");
        }
    }
}

use kuksa::val::v1::{
    val_client::ValClient, DataEntry, Datapoint, EntryUpdate, Field, SetRequest,
};

// ── Config ───────────────────────────────────────────────────────────────────

struct Config {
    vin:        String,
    brand:      String,
    model:      String,
    lat:        f64,
    lon:        f64,
    kuksa_host: String,
    kuksa_port: u16,
    mqtt_host:  String,
    mqtt_port:  u16,
}

impl Config {
    fn from_env() -> Self {
        Self {
            vin:        required("VEHICLE_VIN"),
            brand:      required("VEHICLE_BRAND"),
            model:      required("VEHICLE_MODEL"),
            lat:        required("VEHICLE_LAT").parse().expect("VEHICLE_LAT must be a float"),
            lon:        required("VEHICLE_LON").parse().expect("VEHICLE_LON must be a float"),
            kuksa_host: env::var("KUKSA_HOST").unwrap_or_else(|_| "localhost".into()),
            kuksa_port: env::var("KUKSA_PORT")
                .unwrap_or_else(|_| "55556".into())
                .parse()
                .expect("KUKSA_PORT must be a valid port number"),
            mqtt_host:  env::var("MQTT_HOST").unwrap_or_else(|_| "localhost".into()),
            mqtt_port:  env::var("MQTT_PORT")
                .unwrap_or_else(|_| "1883".into())
                .parse()
                .expect("MQTT_PORT must be a valid port number"),
        }
    }
}

fn required(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("{} must be set", key))
}

// ── gRPC helpers ─────────────────────────────────────────────────────────────

async fn connect_databroker(host: &str, port: u16) -> ValClient<Channel> {
    let endpoint = format!("http://{}:{}", host, port);
    loop {
        match ValClient::connect(endpoint.clone()).await {
            Ok(client) => {
                info!("connected to databroker at {}", endpoint);
                return client;
            }
            Err(e) => {
                warn!("databroker not ready ({}), retrying in 2s…", e);
                time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn set_double(client: &mut ValClient<Channel>, path: &str, value: f64) {
    let req = SetRequest {
        updates: vec![EntryUpdate {
            entry: Some(DataEntry {
                path:            path.to_string(),
                value:           Some(Datapoint {
                    value: Some(kuksa::val::v1::datapoint::Value::DoubleValue(value)),
                }),
                actuator_target: None,
                metadata:        None,
            }),
            fields: vec![Field::Value as i32],
        }],
    };
    if let Err(e) = client.set(req).await {
        warn!("failed to set {} = {}: {}", path, value, e);
    }
}

// ── MQTT helpers ─────────────────────────────────────────────────────────────

async fn publish(client: &AsyncClient, vin: &str, signal_suffix: &str, payload: String) {
    let topic = format!("kuksa/{}/telemetry/{}", vin, signal_suffix);
    if let Err(e) = client.publish(&topic, QoS::AtLeastOnce, false, payload).await {
        warn!("MQTT publish failed on {}: {}", topic, e);
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kuksa2mqtt=info".into()),
        )
        .init();

    let cfg = Config::from_env();
    info!("starting sidecar for {} ({} {})", cfg.vin, cfg.brand, cfg.model);

    // ── Connect to MQTT ───────────────────────────────────────────────────────
    let mut mqtt_opts = MqttOptions::new(
        format!("kuksa2mqtt-{}", cfg.vin),
        &cfg.mqtt_host,
        cfg.mqtt_port,
    );
    mqtt_opts.set_keep_alive(Duration::from_secs(30));
    let (mqtt_client, mut eventloop) = AsyncClient::new(mqtt_opts, 64);

    // Drive the MQTT event loop in the background.
    tokio::spawn(async move {
        loop {
            if let Err(e) = eventloop.poll().await {
                error!("MQTT event loop error: {}", e);
                time::sleep(Duration::from_secs(2)).await;
            }
        }
    });

    // ── Connect to Kuksa Databroker ───────────────────────────────────────────
    let mut kuksa = connect_databroker(&cfg.kuksa_host, cfg.kuksa_port).await;

    // ── Publish static signals once ───────────────────────────────────────────
    publish(&mqtt_client, &cfg.vin, "VehicleIdentification/VIN",   cfg.vin.clone()).await;
    publish(&mqtt_client, &cfg.vin, "VehicleIdentification/Brand",  cfg.brand.clone()).await;
    publish(&mqtt_client, &cfg.vin, "VehicleIdentification/Model",  cfg.model.clone()).await;

    // ── GPS random walk loop at 1 Hz ─────────────────────────────────────────
    let normal = Normal::new(0.0_f64, 0.0002).expect("invalid normal distribution params");
    let mut rng = rand::rngs::SmallRng::from_entropy();
    let mut interval = time::interval(Duration::from_secs(1));
    let mut lat = cfg.lat;
    let mut lon = cfg.lon;

    loop {
        interval.tick().await;

        lat += normal.sample(&mut rng);
        lon += normal.sample(&mut rng);

        // Write updated position back to the databroker.
        set_double(&mut kuksa, "Vehicle.CurrentLocation.Latitude",  lat).await;
        set_double(&mut kuksa, "Vehicle.CurrentLocation.Longitude", lon).await;

        // Publish to MQTT.
        publish(&mqtt_client, &cfg.vin, "CurrentLocation/Latitude",  lat.to_string()).await;
        publish(&mqtt_client, &cfg.vin, "CurrentLocation/Longitude", lon.to_string()).await;
    }
}
