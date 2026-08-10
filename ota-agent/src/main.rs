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

use std::{env, sync::Arc, time::Duration};

use rand::Rng;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use tokio::{sync::Mutex, time};
use tonic::transport::Channel;
use tracing::{info, warn};

pub mod kuksa {
    pub mod val {
        pub mod v1 {
            tonic::include_proto!("kuksa.val.v1");
        }
    }
}

use kuksa::val::v1::{val_client::ValClient, DataEntry, Datapoint, EntryUpdate, Field, SetRequest};

mod uprotocol;
use uprotocol::{status, OtaReporter, UpdateState};

// ── Config ───────────────────────────────────────────────────────────────────

struct Config {
    vin: String,
    gateway_token: String,
    /// This agent's own uProtocol address. One authority per vehicle, so the
    /// back end can tell the agents apart.
    up_source_uri: String,
    /// The back end orchestrator that OTA notifications are addressed to.
    up_destination_uri: String,
    zenoh_config_path: String,
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
            gateway_token: required("HAWKBIT_GATEWAY_TOKEN"),
            up_source_uri: env::var("UP_SOURCE_URI")
                .unwrap_or_else(|_| format!("up://{}/D102/1/0", required("VEHICLE_VIN"))),
            up_destination_uri: env::var("UP_DESTINATION_URI")
                .unwrap_or_else(|_| "up://fms-ota-orchestrator/D103/1/0".into()),
            zenoh_config_path: env::var("ZENOH_CONFIG_PATH")
                .unwrap_or_else(|_| "/zenoh-config.json5".into()),
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
    // The Databroker reports per-datapoint problems (unknown path, wrong type)
    // in the response body rather than as a gRPC error, so a bare Ok(..) is not
    // enough to conclude the value landed.
    match client.set(req).await {
        Err(e) => warn!("failed to set {} = {}: {}", path, value, e),
        Ok(resp) => {
            let resp = resp.into_inner();
            if let Some(error) = resp.error {
                warn!("databroker rejected {} = {}: {:?}", path, value, error);
            }
            for entry_error in resp.errors {
                warn!(
                    "databroker rejected {} = {}: {:?}",
                    entry_error.path, value, entry_error.error
                );
            }
        }
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

async fn run_deployment(
    cfg: Arc<Config>,
    ddi: Arc<Ddi>,
    reporter: Option<Arc<OtaReporter>>,
    action_id: u64,
) {
    info!(vin = %cfg.vin, action_id, "deployment picked up");

    // Notify alongside each DDI feedback rather than instead of it: DDI remains
    // the authoritative record HawkBit acts on, and the notification is what
    // gives the back end its low-latency view.
    let notify = |state: UpdateState, version: String, error: Option<String>| {
        let reporter = reporter.clone();
        let vin = cfg.vin.clone();
        async move {
            if let Some(reporter) = reporter {
                reporter
                    .report(status(&vin, action_id, state, &version, error.as_deref()))
                    .await;
            }
        }
    };

    notify(UpdateState::UPDATE_STATE_PENDING, String::new(), None).await;

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
    notify(UpdateState::UPDATE_STATE_DOWNLOADING, version.clone(), None).await;
    time::sleep(Duration::from_secs(cfg.download_delay_secs)).await;

    // INSTALLING phase
    if let Err(e) = ddi
        .feedback(action_id, "downloaded", "none", "INSTALLING")
        .await
    {
        warn!(vin = %cfg.vin, action_id, "install feedback failed: {e}");
    }
    notify(UpdateState::UPDATE_STATE_INSTALLING, version.clone(), None).await;
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
        notify(
            UpdateState::UPDATE_STATE_FAILED,
            version.clone(),
            Some("simulated failure".to_string()),
        )
        .await;
    } else {
        let mut kuksa = connect_databroker(&cfg.kuksa_host, cfg.kuksa_port).await;
        set_string(&mut kuksa, "Vehicle.SoftwareVersion", version.clone()).await;
        info!(vin = %cfg.vin, action_id, %version, "update complete");
        if let Err(e) = ddi
            .feedback(
                action_id,
                "closed",
                "success",
                &format!("installed {}", version),
            )
            .await
        {
            warn!(vin = %cfg.vin, action_id, "success feedback failed: {e}");
        }
        notify(UpdateState::UPDATE_STATE_COMPLETE, version.clone(), None).await;
    }
}

async fn ddi_loop(cfg: Arc<Config>, ddi: Arc<Ddi>, reporter: Option<Arc<OtaReporter>>) {
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
                    let reporter = reporter.clone();
                    let in_flight = in_flight.clone();
                    tokio::spawn(async move {
                        run_deployment(cfg, ddi, reporter, action_id).await;
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

    // The gateway token is deployment configuration rather than something handed
    // over at runtime, so the agent can start polling DDI immediately and does
    // not depend on the backend having come up first. HawkBit auto-registers the
    // target on the first authenticated request.
    // A transport failure must not stop the agent doing OTA work: DDI is the
    // authoritative path and the back end reconciles against HawkBit, so we
    // degrade to DDI-only reporting rather than refusing to start.
    let reporter = match OtaReporter::connect(
        &cfg.up_source_uri,
        &cfg.up_destination_uri,
        &cfg.zenoh_config_path,
    )
    .await
    {
        Ok(reporter) => {
            info!(vin = %cfg.vin, source = %cfg.up_source_uri, "uProtocol OTA reporting enabled");
            Some(Arc::new(reporter))
        }
        Err(e) => {
            warn!(vin = %cfg.vin, "uProtocol OTA reporting disabled: {e}");
            None
        }
    };

    let ddi = Arc::new(Ddi::new(&cfg.hawkbit_url, &cfg.vin, &cfg.gateway_token));
    ddi_loop(cfg, ddi, reporter).await;
}
