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

use std::{sync::Arc, time::Duration};

use clap::Parser;
use log::{info, warn};
use rand::Rng;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use tokio::{sync::Mutex, time};

mod databroker;
mod uprotocol;
use uprotocol::{status, OtaReporter, UpdateState};

// ── Config ───────────────────────────────────────────────────────────────────

/// In-vehicle OTA agent.
///
/// Polls the hawkBit DDI API over HTTP and reports every state transition to the
/// backend orchestrator as a uProtocol Notification.
#[derive(Debug, Parser)]
#[command(name = "ota-agent", about, version)]
struct Config {
    /// Vehicle identifier. Read from the Databroker when not given, so that
    /// every agent can share one configuration block.
    #[arg(long, env = "VEHICLE_VIN")]
    vin: Option<String>,

    #[arg(long, env = "HAWKBIT_GATEWAY_TOKEN")]
    gateway_token: String,

    #[arg(
        long,
        env = "HAWKBIT_URL",
        default_value = "http://hawkbit:8080",
        value_parser = trim_trailing_slash
    )]
    hawkbit_url: String,

    /// This agent's own uProtocol address. One authority per vehicle, so the
    /// back end can tell the agents apart. `{vin}` is replaced at startup.
    #[arg(long, env = "UP_SOURCE_URI", default_value = "up://{vin}/D102/1/0")]
    up_source_uri: String,

    /// The back end orchestrator that OTA notifications are addressed to.
    #[arg(
        long,
        env = "UP_DESTINATION_URI",
        default_value = "up://fms-ota-orchestrator/D103/1/0"
    )]
    up_destination_uri: String,

    #[arg(long, env = "ZENOH_CONFIG_PATH", default_value = "/zenoh-config.json5")]
    zenoh_config_path: String,

    #[arg(long, env = "KUKSA_HOST", default_value = "databroker")]
    kuksa_host: String,

    #[arg(long, env = "KUKSA_PORT", default_value_t = 55556)]
    kuksa_port: u16,

    /// Probability that a simulated install fails, in [0.0, 1.0].
    ///
    /// Defaults to 0 so that a default run is deterministic and the end-to-end
    /// test is not flaky. Set 0.2 for a demo that shows the failure path.
    #[arg(long, env = "FAILURE_RATE", default_value_t = 0.0)]
    failure_rate: f64,

    #[arg(long, env = "DOWNLOAD_DELAY_SECS", default_value_t = 5)]
    download_delay_secs: u64,

    #[arg(long, env = "INSTALL_DELAY_SECS", default_value_t = 3)]
    install_delay_secs: u64,

    #[arg(long, env = "DDI_POLL_SECS", default_value_t = 3)]
    poll_interval_secs: u64,
}

fn trim_trailing_slash(value: &str) -> Result<String, std::convert::Infallible> {
    Ok(value.trim_end_matches('/').to_string())
}

/// Runtime configuration, with the VIN resolved and substituted into the
/// source URI. Built from [`Config`] once the Databroker has answered.
struct Agent {
    vin: String,
    gateway_token: String,
    hawkbit_url: String,
    up_source_uri: String,
    up_destination_uri: String,
    zenoh_config_path: String,
    kuksa_host: String,
    kuksa_port: u16,
    failure_rate: f64,
    download_delay_secs: u64,
    install_delay_secs: u64,
    poll_interval_secs: u64,
}

impl Agent {
    fn new(cfg: Config, vin: String) -> Self {
        Self {
            up_source_uri: cfg.up_source_uri.replace("{vin}", &vin),
            vin,
            gateway_token: cfg.gateway_token,
            hawkbit_url: cfg.hawkbit_url,
            up_destination_uri: cfg.up_destination_uri,
            zenoh_config_path: cfg.zenoh_config_path,
            kuksa_host: cfg.kuksa_host,
            kuksa_port: cfg.kuksa_port,
            failure_rate: cfg.failure_rate.clamp(0.0, 1.0),
            download_delay_secs: cfg.download_delay_secs,
            install_delay_secs: cfg.install_delay_secs,
            poll_interval_secs: cfg.poll_interval_secs,
        }
    }
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
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

async fn run_deployment(
    cfg: Arc<Agent>,
    ddi: Arc<Ddi>,
    reporter: Option<Arc<OtaReporter>>,
    action_id: u64,
) {
    info!("[{}] action {action_id}: deployment picked up", cfg.vin);

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
        warn!(
            "[{}] action {action_id}: download feedback failed: {e}",
            cfg.vin
        );
    }
    notify(UpdateState::UPDATE_STATE_DOWNLOADING, version.clone(), None).await;
    time::sleep(Duration::from_secs(cfg.download_delay_secs)).await;

    // INSTALLING phase
    if let Err(e) = ddi
        .feedback(action_id, "downloaded", "none", "INSTALLING")
        .await
    {
        warn!(
            "[{}] action {action_id}: install feedback failed: {e}",
            cfg.vin
        );
    }
    notify(UpdateState::UPDATE_STATE_INSTALLING, version.clone(), None).await;
    time::sleep(Duration::from_secs(cfg.install_delay_secs)).await;

    // Terminal: success or (simulated) failure
    let failed = rand::thread_rng().gen_bool(cfg.failure_rate);
    if failed {
        warn!(
            "[{}] action {action_id}: update failed (simulated)",
            cfg.vin
        );
        if let Err(e) = ddi
            .feedback(action_id, "closed", "failure", "simulated failure")
            .await
        {
            warn!(
                "[{}] action {action_id}: failure feedback failed: {e}",
                cfg.vin
            );
        }
        notify(
            UpdateState::UPDATE_STATE_FAILED,
            version.clone(),
            Some("simulated failure".to_string()),
        )
        .await;
    } else {
        match databroker::connect(&cfg.kuksa_host, cfg.kuksa_port) {
            Ok(mut kuksa) => {
                if let Err(e) = databroker::set_software_version(&mut kuksa, &version).await {
                    warn!("[{}] {e}", cfg.vin);
                }
            }
            Err(e) => warn!("[{}] {e}", cfg.vin),
        }
        info!(
            "[{}] action {action_id}: update complete, version {version}",
            cfg.vin
        );
        if let Err(e) = ddi
            .feedback(
                action_id,
                "closed",
                "success",
                &format!("installed {}", version),
            )
            .await
        {
            warn!(
                "[{}] action {action_id}: success feedback failed: {e}",
                cfg.vin
            );
        }
        notify(UpdateState::UPDATE_STATE_COMPLETE, version.clone(), None).await;
    }
}

async fn ddi_loop(cfg: Arc<Agent>, ddi: Arc<Ddi>, reporter: Option<Arc<OtaReporter>>) {
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
                info!("[{}] action {action_id}: acknowledging cancel", cfg.vin);
                if let Err(e) = ddi.ack_cancel(action_id).await {
                    warn!(
                        "[{}] action {action_id}: cancel acknowledgement failed: {e}",
                        cfg.vin
                    );
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!("[{}] DDI poll failed: {e}", cfg.vin);
            }
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = Config::parse();

    // The VIN identifies this vehicle everywhere: to hawkBit as the controller
    // id, and in this agent's uProtocol authority. Prefer the Databroker, which
    // is the vehicle's own source of truth, and fall back to the argument.
    let vin = match cfg.vin.clone() {
        Some(vin) => vin,
        None => {
            let mut client = match databroker::connect(&cfg.kuksa_host, cfg.kuksa_port) {
                Ok(client) => client,
                Err(e) => {
                    log::error!("{e}");
                    std::process::exit(1);
                }
            };
            databroker::wait_for_vin(&mut client, Duration::from_secs(2)).await
        }
    };

    let cfg = Arc::new(Agent::new(cfg, vin));
    info!("starting ota-agent for {}", cfg.vin);

    // The gateway token is deployment configuration rather than something handed
    // over at runtime, so the agent can start polling DDI immediately and does
    // not depend on the backend having come up first. HawkBit auto-registers the
    // target on the first authenticated request.
    //
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
            info!(
                "[{}] uProtocol OTA reporting enabled from {}",
                cfg.vin, cfg.up_source_uri
            );
            Some(Arc::new(reporter))
        }
        Err(e) => {
            warn!("[{}] uProtocol OTA reporting disabled: {e}", cfg.vin);
            None
        }
    };

    let ddi = Arc::new(Ddi::new(&cfg.hawkbit_url, &cfg.vin, &cfg.gateway_token));
    ddi_loop(cfg, ddi, reporter).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::parse_from(["ota-agent", "--gateway-token", "t"])
    }

    #[test]
    fn failure_injection_is_off_by_default() {
        // A non-deterministic default would make the end-to-end test flaky.
        assert_eq!(config().failure_rate, 0.0);
    }

    #[test]
    fn vin_is_optional_so_it_can_come_from_the_databroker() {
        assert!(config().vin.is_none());
    }

    #[test]
    fn source_uri_substitutes_the_vin() {
        let agent = Agent::new(config(), "VIN-0001".to_string());
        assert_eq!(agent.up_source_uri, "up://VIN-0001/D102/1/0");
        assert_eq!(
            agent.up_destination_uri,
            "up://fms-ota-orchestrator/D103/1/0"
        );
    }

    #[test]
    fn an_explicit_source_uri_is_left_alone() {
        let cfg = Config::parse_from([
            "ota-agent",
            "--gateway-token",
            "t",
            "--up-source-uri",
            "up://custom/D102/1/0",
        ]);
        let agent = Agent::new(cfg, "VIN-0001".to_string());
        assert_eq!(agent.up_source_uri, "up://custom/D102/1/0");
    }

    #[test]
    fn failure_rate_is_clamped() {
        let cfg = Config::parse_from(["ota-agent", "--gateway-token", "t", "--failure-rate", "5"]);
        assert_eq!(Agent::new(cfg, "VIN-0001".into()).failure_rate, 1.0);
    }

    #[test]
    fn hawkbit_url_loses_a_trailing_slash() {
        let cfg = Config::parse_from([
            "ota-agent",
            "--gateway-token",
            "t",
            "--hawkbit-url",
            "http://hawkbit:8080/",
        ]);
        assert_eq!(cfg.hawkbit_url, "http://hawkbit:8080");
    }

    #[test]
    fn last_path_segment_strips_the_cache_buster() {
        // hawkBit appends ?c=<n> to the deploymentBase link on every poll.
        assert_eq!(
            last_path_segment(
                "http://hawkbit:8080/DEFAULT/controller/v1/VIN-0001/deploymentBase/7?c=1234"
            ),
            Some("7".to_string())
        );
    }

    #[test]
    fn last_path_segment_handles_a_bare_url() {
        assert_eq!(
            last_path_segment("http://hawkbit:8080/DEFAULT/controller/v1/VIN-0001/cancelAction/12"),
            Some("12".to_string())
        );
    }

    #[test]
    fn last_path_segment_ignores_a_trailing_slash() {
        assert_eq!(
            last_path_segment("http://hawkbit:8080/deploymentBase/7/"),
            Some("7".to_string())
        );
    }

    #[test]
    fn last_path_segment_rejects_an_empty_input() {
        assert_eq!(last_path_segment(""), None);
        assert_eq!(last_path_segment("/"), None);
    }
}
