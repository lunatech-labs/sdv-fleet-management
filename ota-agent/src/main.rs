use std::{env, sync::Arc, time::Duration};

use rand::Rng;
use reqwest::Client as HttpClient;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use tokio::{sync::Mutex, time};
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
    hawkbit_url: String,
    failure_rate: f64,
    download_delay_secs: u64,
    install_delay_secs: u64,
    poll_interval_secs: u64,
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
            hawkbit_url: env::var("HAWKBIT_URL")
                .unwrap_or_else(|_| "http://hawkbit:8080".into())
                .trim_end_matches('/')
                .to_string(),
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
            poll_interval_secs: env::var("DDI_POLL_SECS")
                .unwrap_or_else(|_| "3".into())
                .parse()
                .expect("DDI_POLL_SECS must be a positive integer"),
        }
    }
}

fn required(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("{} must be set", key))
}

// ── DDI DTOs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ControllerBase {
    #[serde(default, rename = "_links")]
    links: Option<ControllerLinks>,
}

#[derive(Debug, Deserialize)]
struct ControllerLinks {
    #[serde(default, rename = "deploymentBase")]
    deployment_base: Option<Link>,
    #[serde(default, rename = "cancelAction")]
    cancel_action: Option<Link>,
}

#[derive(Debug, Deserialize)]
struct Link {
    href: String,
}

#[derive(Debug, Deserialize)]
struct DeploymentBase {
    #[serde(default)]
    deployment: Option<DeploymentPayload>,
}

#[derive(Debug, Deserialize)]
struct DeploymentPayload {
    #[serde(default)]
    chunks: Vec<DeploymentChunk>,
}

#[derive(Debug, Deserialize)]
struct DeploymentChunk {
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

// ── HawkBit DDI loop ─────────────────────────────────────────────────────────

struct Ddi {
    http: HttpClient,
    base: String,
    vin: String,
    auth: String,
}

impl Ddi {
    fn new(base: &str, vin: &str, token: &str) -> Self {
        Self {
            http: HttpClient::new(),
            base: base.trim_end_matches('/').to_string(),
            vin: vin.to_string(),
            auth: format!("GatewayToken {}", token),
        }
    }

    fn poll_url(&self) -> String {
        format!("{}/DEFAULT/controller/v1/{}", self.base, self.vin)
    }

    /// Single DDI poll. Returns the work item HawkBit wants us to do next, if
    /// any. Cancel actions take priority — HawkBit won't surface a new
    /// deployment until an outstanding cancel is closed.
    async fn poll(&self) -> Result<Option<DdiWork>, reqwest::Error> {
        let resp = self
            .http
            .get(self.poll_url())
            .header("Authorization", &self.auth)
            .send()
            .await?
            .error_for_status()?;
        let base: ControllerBase = resp.json().await?;
        let Some(links) = base.links else {
            return Ok(None);
        };
        if let Some(id) = links
            .cancel_action
            .as_ref()
            .and_then(|l| last_path_segment(&l.href))
            .and_then(|s| s.parse::<u64>().ok())
        {
            return Ok(Some(DdiWork::Cancel(id)));
        }
        if let Some(id) = links
            .deployment_base
            .and_then(|l| last_path_segment(&l.href))
            .and_then(|s| s.parse::<u64>().ok())
        {
            return Ok(Some(DdiWork::Deploy(id)));
        }
        Ok(None)
    }

    /// Close a cancel action so HawkBit moves on. We don't actually roll back
    /// anything — there's nothing real to undo.
    async fn ack_cancel(&self, action_id: u64) -> Result<(), reqwest::Error> {
        let url = format!(
            "{}/DEFAULT/controller/v1/{}/cancelAction/{}/feedback",
            self.base, self.vin, action_id
        );
        let body = serde_json::json!({
            "id": action_id.to_string(),
            "status": {
                "execution": "closed",
                "result":    { "finished": "success" },
            },
        });
        self.http
            .post(url)
            .header("Authorization", &self.auth)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Fetch deployment details (we only care about the target version in
    /// `deployment.chunks[0].version`).
    async fn fetch_version(&self, action_id: u64) -> Result<Option<String>, reqwest::Error> {
        let url = format!(
            "{}/DEFAULT/controller/v1/{}/deploymentBase/{}",
            self.base, self.vin, action_id
        );
        let resp = self
            .http
            .get(url)
            .header("Authorization", &self.auth)
            .send()
            .await?
            .error_for_status()?;
        let body: DeploymentBase = resp.json().await?;
        Ok(body
            .deployment
            .and_then(|d| d.chunks.into_iter().next())
            .map(|c| c.version))
    }

    /// Send feedback for an action. `execution` is typically `proceeding` or
    /// `closed`; `finished` is `none`, `success`, or `failure`; `message` is
    /// surfaced as the first entry of the status history's `messages` and is
    /// what the backend parses to distinguish DOWNLOADING from INSTALLING.
    async fn feedback(
        &self,
        action_id: u64,
        execution: &str,
        finished: &str,
        message: &str,
    ) -> Result<(), reqwest::Error> {
        let url = format!(
            "{}/DEFAULT/controller/v1/{}/deploymentBase/{}/feedback",
            self.base, self.vin, action_id
        );
        let body = serde_json::json!({
            "id": action_id.to_string(),
            "status": {
                "execution": execution,
                "result":    { "finished": finished },
                "details":   [ message ],
            },
        });
        self.http
            .post(url)
            .header("Authorization", &self.auth)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

enum DdiWork {
    Deploy(u64),
    Cancel(u64),
}

fn last_path_segment(url: &str) -> Option<String> {
    // HawkBit DDI links include a cache-busting `?c=...` query param —
    // strip it before taking the last segment.
    let no_query = url.split('?').next().unwrap_or(url);
    no_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(|s| s.to_string())
}

async fn run_deployment(cfg: Arc<Config>, ddi: Arc<Ddi>, action_id: u64) {
    info!(vin = %cfg.vin, action_id, "deployment picked up");

    // Find out which version we're pretending to install. If HawkBit can't
    // tell us, fall back to "unknown" — we still go through the motions so
    // the backend sees the action move to `finished`.
    let version = ddi
        .fetch_version(action_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".into());

    // DOWNLOADING phase
    if let Err(e) = ddi
        .feedback(action_id, "download", "none", "DOWNLOADING")
        .await
    {
        warn!(vin = %cfg.vin, action_id, "download feedback failed: {e}");
    }
    time::sleep(Duration::from_secs(cfg.download_delay_secs)).await;

    // INSTALLING phase
    if let Err(e) = ddi
        .feedback(action_id, "downloaded", "none", "INSTALLING")
        .await
    {
        warn!(vin = %cfg.vin, action_id, "install feedback failed: {e}");
    }
    time::sleep(Duration::from_secs(cfg.install_delay_secs)).await;

    // Terminal: success or (simulated) failure
    let failed = rand::thread_rng().gen_bool(cfg.failure_rate.clamp(0.0, 1.0));
    if failed {
        warn!(vin = %cfg.vin, action_id, "update failed (simulated)");
        if let Err(e) = ddi
            .feedback(action_id, "closed", "failure", "simulated failure")
            .await
        {
            warn!(vin = %cfg.vin, action_id, "failure feedback failed: {e}");
        }
    } else {
        let mut kuksa = connect_databroker(&cfg.kuksa_host, cfg.kuksa_port).await;
        set_string(&mut kuksa, "Vehicle.SoftwareVersion", version.clone()).await;
        info!(vin = %cfg.vin, action_id, %version, "update complete");
        if let Err(e) = ddi
            .feedback(action_id, "closed", "success", &format!("installed {}", version))
            .await
        {
            warn!(vin = %cfg.vin, action_id, "success feedback failed: {e}");
        }
    }
}

async fn ddi_loop(cfg: Arc<Config>, ddi: Arc<Ddi>) {
    let mut ticker = time::interval(Duration::from_secs(cfg.poll_interval_secs));
    // Track the action ids we've already started processing so a slow
    // state-machine run doesn't get kicked off twice by the next poll.
    let in_flight: Arc<Mutex<std::collections::HashSet<u64>>> = Arc::default();

    loop {
        ticker.tick().await;
        match ddi.poll().await {
            Ok(Some(DdiWork::Deploy(action_id))) => {
                let mut set = in_flight.lock().await;
                if set.insert(action_id) {
                    drop(set);
                    let cfg = cfg.clone();
                    let ddi = ddi.clone();
                    let in_flight = in_flight.clone();
                    tokio::spawn(async move {
                        run_deployment(cfg, ddi, action_id).await;
                        in_flight.lock().await.remove(&action_id);
                    });
                }
            }
            Ok(Some(DdiWork::Cancel(action_id))) => {
                info!(vin = %cfg.vin, action_id, "ack-ing cancel action");
                if let Err(e) = ddi.ack_cancel(action_id).await {
                    warn!(vin = %cfg.vin, action_id, "cancel ack failed: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!(vin = %cfg.vin, "DDI poll failed: {e}");
            }
        }
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

    // Sole MQTT role now: receive the retained gateway token from the backend.
    let token_topic = "fleet/gateway-token";
    if let Err(e) = mqtt_client.subscribe(token_topic, QoS::AtLeastOnce).await {
        error!("failed to subscribe to {}: {}", token_topic, e);
    }

    let mut ddi_started = false;

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(p))) if p.topic == token_topic => {
                if ddi_started {
                    continue;
                }
                let token = match std::str::from_utf8(&p.payload) {
                    Ok(s) if !s.is_empty() => s.to_string(),
                    Ok(_) => {
                        warn!("empty gateway token received");
                        continue;
                    }
                    Err(e) => {
                        warn!("invalid UTF-8 in gateway token: {}", e);
                        continue;
                    }
                };
                let ddi = Arc::new(Ddi::new(&cfg.hawkbit_url, &cfg.vin, &token));
                info!(vin = %cfg.vin, "gateway token received; starting DDI poll loop");
                let cfg_clone = cfg.clone();
                tokio::spawn(async move { ddi_loop(cfg_clone, ddi).await });
                ddi_started = true;
            }
            Ok(_) => {}
            Err(e) => {
                error!("MQTT event loop error: {}", e);
                time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
