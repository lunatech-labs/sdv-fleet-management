//! Minimal Eclipse HawkBit Management API client.
//!
//! HawkBit is the authoritative store for OTA campaign state. This module owns
//! every request that reads or writes that state. The endpoints, payload shapes,
//! and response field names here follow the public HawkBit REST documentation
//! but have not been pinned to a specific HawkBit version — the stack uses the
//! `hawkbit/hawkbit-update-server:latest` image, so minor drift should be
//! reconciled at integration time.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::campaign::VehicleUpdateState;

#[derive(Debug, thiserror::Error)]
pub enum HawkbitError {
    #[error("HawkBit returned status {status}: {body}")]
    Status { status: StatusCode, body: String },
    #[error("HawkBit request failed: {0}")]
    Request(#[from] reqwest::Error),
}

impl HawkbitError {
    pub fn is_unreachable(&self) -> bool {
        matches!(
            self,
            HawkbitError::Request(e) if e.is_connect() || e.is_timeout(),
        )
    }
}

/// Thin wrapper around a shared `reqwest::Client` with a base URL + bearer token.
#[derive(Clone)]
pub struct HawkbitClient {
    http: Client,
    base: String,
    token: String,
}

impl HawkbitClient {
    pub fn new(base_url: String, token: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");
        Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response, HawkbitError> {
        if resp.status().is_success() {
            Ok(resp)
        } else {
            Err(HawkbitError::Status {
                status: resp.status(),
                body: resp.text().await.unwrap_or_default(),
            })
        }
    }

    // ── Targets ──────────────────────────────────────────────────────────────

    /// Register a single vehicle as a HawkBit target (`POST /rest/v1/targets`).
    /// Idempotent: treats `409 Conflict` as success.
    pub async fn register_target(&self, vin: &str) -> Result<(), HawkbitError> {
        let body = serde_json::json!([{ "controllerId": vin, "name": vin }]);
        let resp = self
            .http
            .post(self.url("/rest/v1/targets"))
            .header("Authorization", self.auth())
            .json(&body)
            .send()
            .await?;
        if resp.status() == StatusCode::CONFLICT {
            return Ok(());
        }
        Self::check(resp).await?;
        Ok(())
    }

    // ── Distribution Sets (= software versions) ──────────────────────────────

    /// Seed a distribution set if one with the same name + version does not yet
    /// exist. Returns the distribution set id.
    pub async fn ensure_distribution_set(
        &self,
        name: &str,
        version: &str,
    ) -> Result<u64, HawkbitError> {
        if let Some(existing) = self.find_distribution_set(name, version).await? {
            return Ok(existing);
        }

        let body = serde_json::json!([{
            "name": name,
            "version": version,
            "type": "os",
            "requiredMigrationStep": false,
        }]);
        let resp = self
            .http
            .post(self.url("/rest/v1/distributionsets"))
            .header("Authorization", self.auth())
            .json(&body)
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let created: Vec<DistributionSet> = resp.json().await?;
        created
            .into_iter()
            .next()
            .map(|d| d.id)
            .ok_or_else(|| HawkbitError::Status {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: "empty distribution set creation response".into(),
            })
    }

    async fn find_distribution_set(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<u64>, HawkbitError> {
        let query = format!("name=={};version=={}", name, version);
        let resp = self
            .http
            .get(self.url("/rest/v1/distributionsets"))
            .header("Authorization", self.auth())
            .query(&[("q", query.as_str())])
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: DistributionSetPage = resp.json().await?;
        Ok(page.content.into_iter().next().map(|d| d.id))
    }

    /// List all distribution set versions. Used by `GET /versions`.
    pub async fn list_versions(&self) -> Result<Vec<String>, HawkbitError> {
        let resp = self
            .http
            .get(self.url("/rest/v1/distributionsets"))
            .header("Authorization", self.auth())
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: DistributionSetPage = resp.json().await?;
        Ok(page.content.into_iter().map(|d| d.version).collect())
    }

    pub async fn find_version_id(&self, version: &str) -> Result<Option<u64>, HawkbitError> {
        let resp = self
            .http
            .get(self.url("/rest/v1/distributionsets"))
            .header("Authorization", self.auth())
            .query(&[("q", format!("version=={}", version).as_str())])
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: DistributionSetPage = resp.json().await?;
        Ok(page.content.into_iter().next().map(|d| d.id))
    }

    // ── Rollouts (= campaigns) ───────────────────────────────────────────────

    /// Create a HawkBit rollout targeting the given VINs against a distribution
    /// set. One deployment group covers all targets — this is a demo, not a
    /// staged rollout.
    pub async fn create_rollout(
        &self,
        name: &str,
        distribution_set_id: u64,
        vins: &[String],
    ) -> Result<u64, HawkbitError> {
        let target_filter = vins
            .iter()
            .map(|v| format!("controllerId=={}", v))
            .collect::<Vec<_>>()
            .join(",");
        let body = serde_json::json!({
            "name": name,
            "distributionSetId": distribution_set_id,
            "targetFilterQuery": target_filter,
            "amountGroups": 1,
        });
        let resp = self
            .http
            .post(self.url("/rest/v1/rollouts"))
            .header("Authorization", self.auth())
            .json(&body)
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let rollout: Rollout = resp.json().await?;

        // HawkBit rollouts are created READY and need an explicit start.
        let start = self
            .http
            .post(self.url(&format!("/rest/v1/rollouts/{}/start", rollout.id)))
            .header("Authorization", self.auth())
            .send()
            .await?;
        if !start.status().is_success() {
            warn!(
                "rollout {} start returned {}: proceeding anyway",
                rollout.id,
                start.status()
            );
        }
        info!(
            "hawkbit rollout {} created for {} vehicles",
            rollout.id,
            vins.len()
        );
        Ok(rollout.id)
    }

    /// Poll a rollout's deployment group for per-target action state.
    /// Returns one entry per target with its current `VehicleUpdateState`.
    pub async fn poll_rollout(
        &self,
        rollout_id: u64,
    ) -> Result<Vec<(String, VehicleUpdateState)>, HawkbitError> {
        let resp = self
            .http
            .get(self.url(&format!("/rest/v1/rollouts/{}/deploygroups", rollout_id)))
            .header("Authorization", self.auth())
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let groups: DeployGroupPage = resp.json().await?;

        let mut out = Vec::new();
        for group in groups.content {
            let resp = self
                .http
                .get(self.url(&format!(
                    "/rest/v1/rollouts/{}/deploygroups/{}/targets",
                    rollout_id, group.id
                )))
                .header("Authorization", self.auth())
                .send()
                .await?;
            let resp = Self::check(resp).await?;
            let targets: TargetPage = resp.json().await?;
            for t in targets.content {
                let state = action_to_state(&t);
                out.push((t.controller_id, state));
            }
        }
        Ok(out)
    }

    // ── Feedback ─────────────────────────────────────────────────────────────

    /// Record a completed / failed deployment. Called as soon as the matching
    /// MQTT status arrives so HawkBit reflects reality without waiting for the
    /// 5s poll cycle.
    pub async fn report_feedback(
        &self,
        vin: &str,
        action_id: u64,
        success: bool,
    ) -> Result<(), HawkbitError> {
        let finished = if success { "success" } else { "failure" };
        let body = serde_json::json!({
            "id": action_id.to_string(),
            "status": {
                "execution": "closed",
                "result": { "finished": finished },
            },
        });
        let resp = self
            .http
            .post(self.url(&format!(
                "/rest/v1/targets/{}/actions/{}/feedback",
                vin, action_id
            )))
            .header("Authorization", self.auth())
            .json(&body)
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
    }
}

// ── HawkBit DTOs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DistributionSet {
    id: u64,
    #[allow(dead_code)]
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct DistributionSetPage {
    content: Vec<DistributionSet>,
}

#[derive(Debug, Deserialize)]
struct Rollout {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct DeployGroup {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct DeployGroupPage {
    content: Vec<DeployGroup>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Target {
    #[serde(rename = "controllerId")]
    controller_id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "updateStatus")]
    update_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TargetPage {
    content: Vec<Target>,
}

/// Map a HawkBit target's rollout state to our `VehicleUpdateState`.
/// HawkBit reports `pending`, `running`, `finished`, `error`, `cancelled`.
fn action_to_state(t: &Target) -> VehicleUpdateState {
    let s = t
        .status
        .as_deref()
        .or(t.update_status.as_deref())
        .unwrap_or("pending");
    match s {
        "finished" => VehicleUpdateState::Complete {
            version: String::new(),
        },
        "error" | "cancelled" => VehicleUpdateState::Failed {
            error: format!("hawkbit reported {}", s),
        },
        "running" => VehicleUpdateState::Installing,
        _ => VehicleUpdateState::Pending,
    }
}
