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
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

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

/// Thin wrapper around a shared `reqwest::Client` with a base URL + Basic auth
/// credentials. HawkBit's Management API (`/rest/v1/...`) uses HTTP Basic, not
/// the DDI gateway token.
#[derive(Clone)]
pub struct HawkbitClient {
    http: Client,
    base: String,
    user: String,
    password: String,
}

impl HawkbitClient {
    pub fn new(base_url: String, user: String, password: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");
        Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            user,
            password,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
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

    // ── Gateway token (tenant-wide DDI provisioning) ─────────────────────────

    /// Enable gateway-token authentication on HawkBit's DEFAULT tenant and
    /// return the current token, generating one if the tenant doesn't have a
    /// key configured yet. Idempotent: safe to call on every backend start.
    ///
    /// With this enabled, any client presenting `Authorization: GatewayToken
    /// <key>` against `/DEFAULT/controller/v1/{vin}` will authenticate and —
    /// if the target doesn't yet exist — HawkBit auto-creates it under
    /// `CONTROLLER_PLUG_AND_PLAY`. This is what makes ota-agent self-
    /// registration work without admin credentials.
    pub async fn enable_gateway_token(&self) -> Result<String, HawkbitError> {
        self.put_config(
            "authentication.gatewaytoken.enabled",
            serde_json::json!(true),
        )
        .await?;

        let existing = self
            .get_config_string("authentication.gatewaytoken.key")
            .await?;
        if !existing.is_empty() {
            return Ok(existing);
        }

        let token = Uuid::new_v4().to_string();
        self.put_config("authentication.gatewaytoken.key", serde_json::json!(&token))
            .await?;
        Ok(token)
    }

    async fn get_config_string(&self, key: &str) -> Result<String, HawkbitError> {
        let resp = self
            .http
            .get(self.url(&format!("/rest/v1/system/configs/{}", key)))
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let cfg: SystemConfig = resp.json().await?;
        Ok(cfg.value.unwrap_or_default())
    }

    async fn put_config(&self, key: &str, value: serde_json::Value) -> Result<(), HawkbitError> {
        let resp = self
            .http
            .put(self.url(&format!("/rest/v1/system/configs/{}", key)))
            .basic_auth(&self.user, Some(&self.password))
            .json(&serde_json::json!({ "value": value }))
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
    }

    // ── Distribution Sets (= software versions) ──────────────────────────────

    /// Seed a distribution set if one with the same name + version does not yet
    /// exist, and attach one `os`-type software module so HawkBit considers it
    /// *complete* (a prerequisite for rollouts). Returns the distribution set id.
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
            .basic_auth(&self.user, Some(&self.password))
            .json(&body)
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let created: Vec<DistributionSet> = resp.json().await?;
        let ds_id =
            created
                .into_iter()
                .next()
                .map(|d| d.id)
                .ok_or_else(|| HawkbitError::Status {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: "empty distribution set creation response".into(),
                })?;

        // HawkBit requires a DS of type `os` to have an `os` software module
        // attached before it can be rolled out. Reuse one per (name, version).
        let module_id = self.ensure_software_module(name, version).await?;
        self.assign_module_to_ds(ds_id, module_id).await?;
        Ok(ds_id)
    }

    /// Create (or find) a software module of type `os` matching (name, version).
    async fn ensure_software_module(&self, name: &str, version: &str) -> Result<u64, HawkbitError> {
        let query = format!("name=={};version=={}", name, version);
        let resp = self
            .http
            .get(self.url("/rest/v1/softwaremodules"))
            .basic_auth(&self.user, Some(&self.password))
            .query(&[("q", query.as_str())])
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: SoftwareModulePage = resp.json().await?;
        if let Some(sm) = page.content.into_iter().next() {
            return Ok(sm.id);
        }

        let body = serde_json::json!([{ "name": name, "version": version, "type": "os" }]);
        let resp = self
            .http
            .post(self.url("/rest/v1/softwaremodules"))
            .basic_auth(&self.user, Some(&self.password))
            .json(&body)
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let created: Vec<SoftwareModule> = resp.json().await?;
        created
            .into_iter()
            .next()
            .map(|m| m.id)
            .ok_or_else(|| HawkbitError::Status {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: "empty software module creation response".into(),
            })
    }

    async fn assign_module_to_ds(&self, ds_id: u64, module_id: u64) -> Result<(), HawkbitError> {
        let body = serde_json::json!([{ "id": module_id }]);
        let resp = self
            .http
            .post(self.url(&format!("/rest/v1/distributionsets/{}/assignedSM", ds_id)))
            .basic_auth(&self.user, Some(&self.password))
            .json(&body)
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
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
            .basic_auth(&self.user, Some(&self.password))
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
            .basic_auth(&self.user, Some(&self.password))
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
            .basic_auth(&self.user, Some(&self.password))
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
            .basic_auth(&self.user, Some(&self.password))
            .json(&body)
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let rollout: Rollout = resp.json().await?;

        // HawkBit creates the rollout in status `creating` and asynchronously
        // moves it to `ready`. Calling /start before `ready` returns 400. Poll
        // briefly, then fire /start.
        self.wait_for_rollout_ready(rollout.id).await;

        let start = self
            .http
            .post(self.url(&format!("/rest/v1/rollouts/{}/start", rollout.id)))
            .basic_auth(&self.user, Some(&self.password))
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

    /// Poll until a rollout's status is `ready` (or give up after ~5s). Best
    /// effort: on error or timeout the caller's /start will surface the real
    /// status.
    async fn wait_for_rollout_ready(&self, rollout_id: u64) {
        for _ in 0..10 {
            let resp = self
                .http
                .get(self.url(&format!("/rest/v1/rollouts/{}", rollout_id)))
                .basic_auth(&self.user, Some(&self.password))
                .send()
                .await;
            if let Ok(r) = resp {
                if let Ok(json) = r.json::<serde_json::Value>().await {
                    if json.get("status").and_then(|s| s.as_str()) == Some("ready") {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // ── Campaign hydration (on backend startup) ──────────────────────────────

    /// List all rollouts HawkBit knows about. Used at startup to rebuild the
    /// in-memory campaign store from HawkBit (the source of truth).
    pub async fn list_rollouts(&self) -> Result<Vec<RolloutSummary>, HawkbitError> {
        let resp = self
            .http
            .get(self.url("/rest/v1/rollouts"))
            .basic_auth(&self.user, Some(&self.password))
            .query(&[("limit", "500")])
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: RolloutPage = resp.json().await?;
        Ok(page.content)
    }

    /// Fetch the version string for a distribution set id.
    pub async fn distribution_set_version(&self, ds_id: u64) -> Result<String, HawkbitError> {
        let resp = self
            .http
            .get(self.url(&format!("/rest/v1/distributionsets/{}", ds_id)))
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let ds: DistributionSet = resp.json().await?;
        Ok(ds.version)
    }

    /// List the controller ids (VINs) targeted by a rollout. Our rollouts have
    /// a single deploy group, so this is one extra call to get the group id
    /// plus one to enumerate its targets.
    pub async fn rollout_target_vins(&self, rollout_id: u64) -> Result<Vec<String>, HawkbitError> {
        let resp = self
            .http
            .get(self.url(&format!("/rest/v1/rollouts/{}/deploygroups", rollout_id)))
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let groups: DeployGroupPage = resp.json().await?;

        let mut out = Vec::new();
        for g in groups.content {
            let resp = self
                .http
                .get(self.url(&format!(
                    "/rest/v1/rollouts/{}/deploygroups/{}/targets",
                    rollout_id, g.id
                )))
                .basic_auth(&self.user, Some(&self.password))
                .query(&[("limit", "500")])
                .send()
                .await?;
            let resp = Self::check(resp).await?;
            let page: TargetIdPage = resp.json().await?;
            for t in page.content {
                out.push(t.controller_id);
            }
        }
        Ok(out)
    }

    // ── Per-target action state (read-side of the DDI loop) ──────────────────

    /// Fetch actions for a target, most recent first.
    pub async fn list_target_actions(&self, vin: &str) -> Result<Vec<TargetAction>, HawkbitError> {
        let resp = self
            .http
            .get(self.url(&format!("/rest/v1/targets/{}/actions", vin)))
            .basic_auth(&self.user, Some(&self.password))
            .query(&[("sort", "id:DESC"), ("limit", "20")])
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: TargetActionPage = resp.json().await?;
        Ok(page.content)
    }

    /// Fetch the most recent status-history entry for an action. Used to
    /// distinguish DOWNLOADING vs INSTALLING inside HawkBit's coarse `running`
    /// state via the device-reported messages.
    pub async fn latest_action_status(
        &self,
        vin: &str,
        action_id: u64,
    ) -> Result<Option<ActionStatusEntry>, HawkbitError> {
        let resp = self
            .http
            .get(self.url(&format!(
                "/rest/v1/targets/{}/actions/{}/status",
                vin, action_id
            )))
            .basic_auth(&self.user, Some(&self.password))
            .query(&[("sort", "id:DESC"), ("limit", "1")])
            .send()
            .await?;
        let resp = Self::check(resp).await?;
        let page: ActionStatusPage = resp.json().await?;
        Ok(page.content.into_iter().next())
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
pub struct RolloutSummary {
    pub id: u64,
    pub name: String,
    #[serde(rename = "distributionSetId")]
    pub distribution_set_id: u64,
    /// Epoch-ms of creation.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
struct RolloutPage {
    content: Vec<RolloutSummary>,
}

#[derive(Debug, Deserialize)]
struct DeployGroupSummary {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct DeployGroupPage {
    content: Vec<DeployGroupSummary>,
}

#[derive(Debug, Deserialize)]
struct TargetId {
    #[serde(rename = "controllerId")]
    controller_id: String,
}

#[derive(Debug, Deserialize)]
struct TargetIdPage {
    content: Vec<TargetId>,
}

#[derive(Debug, Deserialize)]
struct SystemConfig {
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SoftwareModule {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct SoftwareModulePage {
    content: Vec<SoftwareModule>,
}

#[derive(Debug, Deserialize)]
pub struct TargetAction {
    pub id: u64,
    /// HawkBit action status: `pending`, `running`, `finished`, `error`,
    /// `canceling`, `canceled`, `scheduled`, `retrieved`, `warning`.
    pub status: String,
    /// Numeric id of the rollout this action came from, if any. Used to match
    /// an action to a Campaign (`Campaign.rollout_id`).
    #[serde(default)]
    pub rollout: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TargetActionPage {
    content: Vec<TargetAction>,
}

#[derive(Debug, Deserialize)]
pub struct ActionStatusEntry {
    /// Status entry type as reported by the device feedback or by HawkBit
    /// itself (e.g. `running`, `finished`, `error`, `download`, `downloaded`,
    /// `scheduled`, `canceled`).
    #[allow(dead_code)]
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub messages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ActionStatusPage {
    content: Vec<ActionStatusEntry>,
}
