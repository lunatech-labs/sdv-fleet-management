use std::{env, sync::Arc, time::Duration};

use rand::Rng;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use serde_json::json;
use tokio::time;
use tonic::transport::Channel;
use tracing::{error, info, warn};

pub mod kuksa {
    pub mod val {
        pub mod v1 {
            tonic::include_proto!("kuksa.val.v1");
        }
    }
}

use kuksa::val::v1::{val_client::ValClient, DataEntry, Datapoint, EntryUpdate, Field, SetRequest};

// ── Config ───────────────────────────────────────────────────────────────────

struct Config {
    vin: String,
    mqtt_host: String,
    mqtt_port: u16,
    kuksa_host: String,
    kuksa_port: u16,
    failure_rate: f64,
    download_delay_secs: u64,
    install_delay_secs: u64,
}

impl Config {
    fn from_env() -> Self {
        Self {
            vin: required("VEHICLE_VIN"),
            mqtt_host: env::var("MQTT_HOST").unwrap_or_else(|_| "localhost".into()),
            mqtt_port: env::var("MQTT_PORT")
                .unwrap_or_else(|_| "1883".into())
                .parse()
                .expect("MQTT_PORT must be a valid port number"),
            kuksa_host: required("KUKSA_HOST"),
            kuksa_port: env::var("KUKSA_PORT")
                .unwrap_or_else(|_| "55555".into())
                .parse()
                .expect("KUKSA_PORT must be a valid port number"),
            failure_rate: env::var("FAILURE_RATE")
                .unwrap_or_else(|_| "0.2".into())
                .parse()
                .expect("FAILURE_RATE must be a float in [0.0, 1.0]"),
            download_delay_secs: env::var("DOWNLOAD_DELAY_SECS")
                .unwrap_or_else(|_| "5".into())
                .parse()
                .expect("DOWNLOAD_DELAY_SECS must be a positive integer"),
            install_delay_secs: env::var("INSTALL_DELAY_SECS")
                .unwrap_or_else(|_| "3".into())
                .parse()
                .expect("INSTALL_DELAY_SECS must be a positive integer"),
        }
    }
}

fn required(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("{} must be set", key))
}

// ── Command / state payloads ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Command {
    campaign_id: String,
    version: String,
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

async fn set_string(client: &mut ValClient<Channel>, path: &str, value: String) {
    let req = SetRequest {
        updates: vec![EntryUpdate {
            entry: Some(DataEntry {
                path: path.to_string(),
                value: Some(Datapoint {
                    value: Some(kuksa::val::v1::datapoint::Value::StringValue(value.clone())),
                }),
                actuator_target: None,
                metadata: None,
            }),
            fields: vec![Field::Value as i32],
        }],
    };
    if let Err(e) = client.set(req).await {
        warn!("failed to set {} = {}: {}", path, value, e);
    }
}

// ── MQTT helpers ─────────────────────────────────────────────────────────────

async fn publish_status(client: &AsyncClient, vin: &str, payload: String) {
    let topic = format!("kuksa/{}/ota/status", vin);
    if let Err(e) = client
        .publish(&topic, QoS::AtLeastOnce, false, payload)
        .await
    {
        warn!("MQTT publish failed on {}: {}", topic, e);
    }
}

// ── State machine ────────────────────────────────────────────────────────────

async fn run_state_machine(cfg: Arc<Config>, mqtt: Arc<AsyncClient>, command: Command) {
    let Command {
        campaign_id,
        version,
    } = command;
    info!(
        vin = %cfg.vin,
        campaign_id,
        version,
        "command received, starting update",
    );

    // DOWNLOADING
    publish_status(
        &mqtt,
        &cfg.vin,
        json!({
            "campaign_id": campaign_id,
            "vin":         cfg.vin,
            "state":       "DOWNLOADING",
        })
        .to_string(),
    )
    .await;
    time::sleep(Duration::from_secs(cfg.download_delay_secs)).await;

    // INSTALLING
    publish_status(
        &mqtt,
        &cfg.vin,
        json!({
            "campaign_id": campaign_id,
            "vin":         cfg.vin,
            "state":       "INSTALLING",
        })
        .to_string(),
    )
    .await;
    time::sleep(Duration::from_secs(cfg.install_delay_secs)).await;

    // COMPLETE or FAILED
    let failed = rand::thread_rng().gen_bool(cfg.failure_rate.clamp(0.0, 1.0));
    if failed {
        info!(vin = %cfg.vin, campaign_id, "update failed (simulated)");
        publish_status(
            &mqtt,
            &cfg.vin,
            json!({
                "campaign_id": campaign_id,
                "vin":         cfg.vin,
                "state":       "FAILED",
                "error":       "simulated failure",
            })
            .to_string(),
        )
        .await;
    } else {
        // Write new version to the local Databroker.
        let mut kuksa = connect_databroker(&cfg.kuksa_host, cfg.kuksa_port).await;
        set_string(&mut kuksa, "Vehicle.SoftwareVersion", version.clone()).await;

        info!(vin = %cfg.vin, campaign_id, version, "update complete");
        publish_status(
            &mqtt,
            &cfg.vin,
            json!({
                "campaign_id": campaign_id,
                "vin":         cfg.vin,
                "state":       "COMPLETE",
                "version":     version,
            })
            .to_string(),
        )
        .await;
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ota_agent=info".into()),
        )
        .init();

    let cfg = Arc::new(Config::from_env());
    info!("starting ota-agent for {}", cfg.vin);

    let mut mqtt_opts = MqttOptions::new(
        format!("ota-agent-{}", cfg.vin),
        &cfg.mqtt_host,
        cfg.mqtt_port,
    );
    mqtt_opts.set_keep_alive(Duration::from_secs(30));
    let (mqtt_client, mut eventloop) = AsyncClient::new(mqtt_opts, 64);
    let mqtt_client = Arc::new(mqtt_client);

    let cmd_topic = format!("kuksa/{}/ota/command", cfg.vin);
    if let Err(e) = mqtt_client.subscribe(&cmd_topic, QoS::AtLeastOnce).await {
        error!("failed to subscribe to {}: {}", cmd_topic, e);
    }

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(p))) if p.topic == cmd_topic => {
                let payload = match std::str::from_utf8(&p.payload) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("invalid UTF-8 in command payload: {}", e);
                        continue;
                    }
                };
                let command: Command = match serde_json::from_str(payload) {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("failed to parse command {:?}: {}", payload, e);
                        continue;
                    }
                };

                let cfg = cfg.clone();
                let mqtt = mqtt_client.clone();
                tokio::spawn(async move { run_state_machine(cfg, mqtt, command).await });
            }
            Ok(_) => {}
            Err(e) => {
                error!("MQTT event loop error: {}", e);
                time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
