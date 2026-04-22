# Fleet Management Demo, v2 Specification

**Version:** 2.1
**Status:** Implemented
**Audience:** External clients and prospects
**Builds on:** specs.md (v1)

---

## 1. Overview

v2 extends the live fleet dashboard with OTA (over-the-air) software update campaigns powered by Eclipse HawkBit. An operator selects a set of vehicles and a target software version directly from the Vue dashboard, launches a campaign, and watches per-vehicle progress in real time. A configurable failure rate makes the demo resilient to the "happy path only" criticism and demonstrates rollback awareness.

The existing v1 telemetry pipeline (Kuksa → MQTT → Rust backend → browser) is unchanged. v2 adds a second flow alongside it in which HawkBit is the authoritative source of OTA state: the Rust backend orchestrates campaigns through HawkBit's Management API, and each ota-agent talks directly to HawkBit's Direct Device Integration (DDI) API. MQTT is no longer on the OTA path; its only new role is to carry the retained gateway token that the backend provisions and publishes so agents can authenticate to DDI.

---

## 2. Goals

### v2 must demonstrate

- An operator launching an OTA campaign from the Vue dashboard (vehicle selection + target version)
- Per-vehicle state machine progressing in real time: `PENDING → DOWNLOADING → INSTALLING → COMPLETE / FAILED`
- Configurable failure rate (default 20%) so some vehicles fail and require attention
- `Vehicle.SoftwareVersion` updating in the vehicle drawer after a successful install
- HawkBit as the authoritative campaign store, visible to the backend via its Management API
- Clean REST surface for campaign management, documented in Swagger UI

### Out of scope for v2

- Real artifact download (update payloads are simulated)
- TLS / authentication (demo environment only)
- Rollback execution (failure is surfaced but recovery is manual)
- More than 20 vehicles
- Offline retry logic. `restart: on-failure` in docker-compose recovers crashed agents. HawkBit DDI polling tolerates transient HawkBit outages because every agent poll is stateless and the state machine only advances on a successful poll response.

---

## 3. Architecture

### 3.1 Component overview

```
Browser Dashboard
    │  REST  POST /campaigns, GET /campaigns/{id}, GET /versions
    │  WS    /ws/campaigns                          (live per-vehicle status)
    │  REST  GET /fleet, GET /vehicles/{vin}        (unchanged from v1)
    │  WS    /ws/fleet                              (unchanged from v1)
    v
Rust Backend (axum)
    │  HawkBit Management API (HTTP Basic admin:admin)
    │     * provision gateway token in tenant config
    │     * seed distribution sets + software modules
    │     * create rollouts
    │     * poll target actions + action-status history every 3 s
    │     * hydrate campaign store from rollouts on startup
    │  MQTT publish  fleet/gateway-token            (retained, on startup)
    │  MQTT subscribe  kuksa/+/telemetry/#          (unchanged from v1)
    v
Eclipse Mosquitto (unchanged)
    v  fleet/gateway-token  backend -> ota-agents (retained, QoS 1)
    │
┌───────────────────────────────────────────────────────────────┐
│  Vehicle fleet, 20 instances                                  │
│                                                               │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────┐  │
│  │   Kuksa      │  │ kuksa2mqtt   │  │  ota-agent          │  │
│  │ Databroker   │<>│  sidecar     │  │                     │  │
│  │ (gRPC)       │  │              │  │                     │  │
│  └──────────────┘  └──────────────┘  └─────────────────────┘  │
│         × 20 vehicles (60 containers total)                   │
└───────────────────────────────────────────────────────────────┘
                                                  │
                                                  │  HawkBit DDI API
                                                  │  Authorization: GatewayToken <uuid>
                                                  │    * GET /DEFAULT/controller/v1/{vin}
                                                  │      (self-register on first call,
                                                  │       discover pending deployment)
                                                  │    * GET .../deploymentBase/{id}
                                                  │    * POST .../deploymentBase/{id}/feedback
                                                  │    * POST .../cancelAction/{id}/feedback
                                                  v
Eclipse HawkBit (Spring Boot, Postgres-backed)
    stores: targets (VINs), distribution sets, software modules, rollouts,
            actions and action-status history
```

**Three key shifts from the earlier v2 draft:**

1. **HawkBit is now authoritative.** The backend's in-memory `CampaignStore` is a projection of HawkBit state. On startup it hydrates from HawkBit rollouts; during runtime it reconciles every 3 s from each target's action status. The backend never writes OTA state without first having read it from HawkBit.
2. **ota-agents use the DDI API directly** with a gateway token. MQTT no longer carries OTA commands or status. This matches HawkBit's intended device-provisioning workflow.
3. **Target self-registration.** The backend no longer pre-registers targets. Each ota-agent's first DDI poll authenticates with the gateway token and HawkBit auto-creates the target under `CONTROLLER_PLUG_AND_PLAY`.

### 3.2 HawkBit concepts mapped to this demo

| HawkBit concept | Demo mapping |
|---|---|
| Target | One vehicle (keyed by VIN). Auto-created by HawkBit on first DDI contact. |
| Software Module | An `os` module seeded per version. Required by HawkBit for a distribution set of type `os` to be "complete". |
| Distribution Set | A named software version (e.g. `fleet-fw:2.0.0`) with an attached `os` module. |
| Rollout | One campaign (covers N vehicles). Named `campaign-<uuid>` so the backend can round-trip the campaign id on hydration. |
| Action | HawkBit's per-target deployment record. `action.status` and the latest status-history entry's `messages` drive the backend's `VehicleUpdateState`. |

### 3.3 Port allocation (additions)

| Service | Host port |
|---|---|
| Eclipse HawkBit | 8083 |
| Postgres (HawkBit backend store) | not exposed (internal only) |
| All v1 services | unchanged |

---

## 4. New components: Eclipse HawkBit and Postgres

HawkBit runs as a single Docker container using the `hawkbit-update-server:0.3.0M7` image, backed by a Postgres container. The image is pinned because `latest` ships an H2 JDBC driver that has dropped `CALL IDENTITY()` support. A dedicated `postgres:16-alpine` service replaces the H2 in-memory DB.

> **Note on persistence:** the Postgres container has no volume mounted, so a `docker compose down` still wipes all campaign history. This is intentional for demo reproducibility. The container's own lifecycle survives `docker compose restart` without data loss, and that is enough for the backend's campaign hydration to recover state across restarts.

**Responsibilities:**
- Persist targets (auto-created on first DDI contact from each ota-agent).
- Persist software modules and distribution sets (seeded by the backend at startup).
- Persist rollouts and per-target actions, including the full action-status history.
- Expose the Management API consumed by the Rust backend.
- Expose the DDI API consumed by ota-agents.

**Configuration:**
- **Management API authentication:** HTTP Basic. Credentials are driven from `HAWKBIT_USER` / `HAWKBIT_PASSWORD` in `.env`, default `admin:admin`. The same pair is injected into the HawkBit container via `HAWKBIT_SERVER_IM_USERS_0_*` Spring env vars with a `{noop}` password prefix so the backend and HawkBit cannot drift.
- **DDI authentication:** gateway-token, provisioned dynamically by the backend on startup. The backend PUTs `authentication.gatewaytoken.enabled=true` and, if the tenant does not already have a key, a freshly generated UUID into `authentication.gatewaytoken.key`. This value is then broadcast to all ota-agents as a retained MQTT message (see §6).
- **Database:** Postgres. Both the DB and HawkBit bind user/password to `hawkbit:hawkbit` (internal only, not exposed on the host).
- **Healthcheck:** `wget --spider http://localhost:8080/UI/login`. The `/actuator/health` path returns 404 on this HawkBit version; `/UI/login` returns 200 as soon as the embedded Tomcat is ready.
- **Port:** 8083 (mapped to HawkBit's internal 8080).

**Backend interactions at startup:**
1. Provision the gateway token (`PUT /rest/v1/system/configs/authentication.gatewaytoken.{enabled,key}`).
2. Seed three distribution sets (`1.5.0`, `2.0.0`, `2.1.0-beta`) and, for each, an `os`-type software module attached via `POST .../distributionsets/{id}/assignedSM`. A distribution set of type `os` without an attached module is rejected by the rollout creation endpoint as "incomplete".
3. Hydrate the `CampaignStore`: list all rollouts, parse `campaign-<uuid>` names, resolve each rollout's target VINs and distribution-set version, and insert a placeholder `Campaign` with every vehicle in `PENDING`. The reconcile loop fills in real states on its next tick.

**Backend interactions at runtime:**
- On `POST /campaigns`: create a rollout via `POST /rest/v1/rollouts`, wait for HawkBit to promote it from `creating` to `ready` (short polling loop), then start it with `POST /rest/v1/rollouts/{id}/start`.
- Every 3 seconds, for each non-terminal vehicle in each campaign: `GET /rest/v1/targets/{vin}/actions` to find the action matching the campaign's rollout id, then `GET .../actions/{id}/status?sort=id:DESC&limit=1` to read the latest status-history entry. The `action.status` plus the entry's `messages[0]` are mapped to the `VehicleUpdateState`. Terminal states are never downgraded.

HawkBit is the single source of truth. The backend's in-memory `CampaignStore` is a cache. No write to the store happens without a read from HawkBit having produced it, which keeps the data path cycle-free.

---

## 5. New component: ota-agent

A Rust binary (`ota-agent/`) running as a separate container per vehicle. It is the simulated ECU update client. In v2 as shipped, the agent talks to HawkBit's DDI API over HTTP and to its local Kuksa Databroker over gRPC. Its MQTT role is minimal: it subscribes to one retained topic to learn the gateway token, then proceeds entirely over HTTP.

**Bootstrap flow:**

1. Connect to MQTT and subscribe to `fleet/gateway-token` (retained, QoS 1).
2. On receiving the token, start the DDI poll loop. Subsequent retained-message redeliveries are ignored.
3. The very first DDI poll both self-registers the target (HawkBit auto-creates it) and proves the token works. No explicit registration call is made.

**DDI poll loop (every `DDI_POLL_SECS`, default 3 s):**

```
GET /DEFAULT/controller/v1/{vin}   Authorization: GatewayToken <token>
│
├── _links.cancelAction present      -> POST .../cancelAction/{id}/feedback
│                                        execution=closed, finished=success
│                                        (HawkBit won't dispatch new deployments
│                                         while a cancel is outstanding)
│
├── _links.deploymentBase present    -> spawn state machine for that action id
│                                        (skip if already in-flight)
│
└── nothing                         -> sleep, poll again
```

**State machine (per deployment action):**

```
GET  .../deploymentBase/{id}                                     # fetch target version
POST .../deploymentBase/{id}/feedback execution=proceeding       # message: "DOWNLOADING"
sleep DOWNLOAD_DELAY_SECS
POST .../deploymentBase/{id}/feedback execution=proceeding       # message: "INSTALLING"
sleep INSTALL_DELAY_SECS
roll FAILURE_RATE:
  success:
    gRPC Set Vehicle.SoftwareVersion = <version> on local databroker
    POST .../deploymentBase/{id}/feedback execution=closed, finished=success
  failure:
    POST .../deploymentBase/{id}/feedback execution=closed, finished=failure
                                                                 # message: "simulated failure"
```

The `DOWNLOADING` / `INSTALLING` / `simulated failure` strings in the feedback `details[]` array are what the backend parses out of the action-status history to distinguish DOWNLOADING from INSTALLING inside HawkBit's single `running` status and to surface the failure reason.

**Environment variables:**

| Variable | Default | Description |
|---|---|---|
| `VEHICLE_VIN` | required | VIN this agent manages |
| `MQTT_HOST` | `mosquitto` | MQTT broker hostname (gateway-token delivery only) |
| `KUKSA_HOST` | required | Local Databroker hostname |
| `KUKSA_PORT` | `55555` | Local Databroker gRPC port |
| `HAWKBIT_URL` | `http://hawkbit:8080` | HawkBit base URL for DDI calls |
| `FAILURE_RATE` | `0.2` | Probability (0.0 to 1.0) that an install fails |
| `DOWNLOAD_DELAY_SECS` | `5` | Simulated download duration |
| `INSTALL_DELAY_SECS` | `3` | Simulated install duration |
| `DDI_POLL_SECS` | `3` | DDI poll interval (overrides HawkBit's suggested cadence) |

No `HAWKBIT_TOKEN` env var is needed on the agent: the token is received over MQTT at runtime, which lets an admin rotate it without restarting every agent container.

---

## 6. MQTT topic additions

Only one new topic is added in v2:

```
fleet/gateway-token     backend -> ota-agents    (retained, QoS 1)
```

The `kuksa/{vin}/ota/command` and `kuksa/{vin}/ota/status` topics from the earlier draft are not used. OTA dispatch and feedback travel over HawkBit's DDI API, not MQTT.

The retained flag is load-bearing. Agents can start before the backend, or the backend can restart, without losing the token: the broker re-delivers the retained message to each new subscriber. If an admin rotates the token (by wiping HawkBit's tenant config), a backend restart re-provisions a new token and republishes, overwriting the retained message.

---

## 7. Backend changes

### 7.1 New module: `hawkbit.rs`

HawkBit Management API client. Exposes only read + orchestration operations; there is no Management-API feedback path (feedback lives on DDI).

Responsibilities:
- **Gateway-token provisioning:** `enable_gateway_token()` PUTs `authentication.gatewaytoken.enabled=true` and a freshly generated UUID into `authentication.gatewaytoken.key` if the tenant doesn't already have one. Idempotent.
- **Distribution set + software module seeding:** `ensure_distribution_set(name, version)` creates the DS if missing and attaches a matching `os` software module so HawkBit marks it complete.
- **Rollout creation:** `create_rollout(name, ds_id, vins)` posts the rollout, polls its status for up to 5 s until it reaches `ready`, then starts it.
- **Action readback:** `list_target_actions(vin)` and `latest_action_status(vin, action_id)` feed the reconciliation loop in `main.rs`.
- **Hydration readback:** `list_rollouts()`, `distribution_set_version(ds_id)`, `rollout_target_vins(rollout_id)` feed the startup hydrator.

Notably absent:
- No `register_target(vin)`. Targets self-register through DDI.
- No `report_feedback(...)`. Feedback is a DDI operation, not a Management API one.
- No `poll_rollout(rollout_id) -> per-VIN state`. The reconciliation path is per-target via `list_target_actions` so that terminal states on a per-vehicle basis are easy to short-circuit.

### 7.2 New module: `campaign.rs`

In-memory `CampaignStore` backed by `Arc<DashMap<CampaignId, Campaign>>`. Source of truth for the dashboard WS; projection of HawkBit for the write path.

```rust
pub struct Campaign {
    pub id:          Uuid,
    pub version:     String,
    pub vehicles:    HashMap<String, VehicleUpdateState>,
    pub created:     DateTime<Utc>,
    pub rollout_id:  Option<u64>,   // link back to HawkBit for reconciliation
}

pub enum VehicleUpdateState {
    Pending,
    Downloading,
    Installing,
    Complete { version: String },
    Failed   { error: String },
}
```

`set_vehicle_state(campaign_id, vin, state)` is the only mutation method. It is called from exactly one place: the `poll_campaign_state` task in `main.rs`.

### 7.3 Updated module: `mqtt.rs`

The backend's MQTT role is now limited to telemetry plus the one-shot gateway-token announcement.

- Subscriptions: `kuksa/+/telemetry/#` only.
- `publish_gateway_token(client, token)` posts a retained QoS 1 message to `fleet/gateway-token` at startup.
- No OTA command publish, no OTA status subscription, no `handle_ota_status` path.

### 7.4 New task in `main.rs`: `poll_campaign_state`

A background task that runs every 3 s and reconciles every non-terminal vehicle in every active campaign against HawkBit.

Pseudocode:

```rust
for campaign in campaigns.all() {
    let rollout_id = campaign.rollout_id?;
    for (vin, prev) in &campaign.vehicles {
        if is_terminal(prev) { continue; }               // Complete / Failed are sticky
        let action = hawkbit
            .list_target_actions(vin).await?
            .into_iter().find(|a| a.rollout == Some(rollout_id))?;
        let new_state = match action.status {
            "finished"         => Complete { version: campaign.version.clone() },
            "error" | "canceled" => Failed { error: latest_message_or("update failed") },
            "running"          => match latest_message(vin, action.id).await {
                starts_with("INSTALLING") => Installing,
                starts_with("DOWNLOADING") => Downloading,
                _                          => Pending,     // just the initial "Initiated by Rollout..."
            },
            _                  => Pending,
        };
        if discriminant(prev) != discriminant(new_state) {
            campaigns.set_vehicle_state(&campaign.id, vin, new_state);
            campaign_tx.send(CampaignEvent { campaign_id, vin, state });
        }
    }
}
```

### 7.5 New helper in `main.rs`: `hydrate_campaigns`

On startup, after distribution sets are seeded, the backend rebuilds the `CampaignStore` from HawkBit so restarts don't drop campaign history:

1. `list_rollouts()` from HawkBit.
2. For each rollout whose name starts with `campaign-<uuid>`, parse the UUID.
3. Look up the distribution-set version via `distribution_set_version(ds_id)`.
4. Enumerate target VINs via `rollout_target_vins(rollout_id)`.
5. Insert a `Campaign` with every vehicle in `Pending` and the rollout id attached.

The first `poll_campaign_state` tick (3 s after startup) then promotes those vehicles to their real state. This two-phase approach keeps hydration simple (no per-vehicle HTTP fan-out at startup) while still converging within a few seconds.

### 7.6 New REST endpoints

| Method | Path | Description |
|---|---|---|
| `POST` | `/campaigns` | Create and launch a new OTA campaign |
| `GET` | `/campaigns` | List all campaigns with summary status |
| `GET` | `/campaigns/{id}` | Full per-vehicle status for one campaign |
| `GET` | `/versions` | List available software versions from HawkBit |
| `WS` | `/ws/campaigns` | Live stream of `VehicleUpdateState` transitions |

**`POST /campaigns` request body:**
```json
{ "version": "2.0.0", "vins": ["VIN-0001", "VIN-0003"] }
```

**`POST /campaigns` error responses:**

| Status | Condition |
|---|---|
| `400` | Missing or invalid `version` / `vins` field; unknown VIN |
| `503` | HawkBit unreachable at campaign creation time |

Error body: `{ "error": "<human-readable message>" }`

**`GET /campaigns/{id}` response shape:**
```json
{
  "id": "uuid",
  "version": "2.0.0",
  "created": "2026-04-20T10:00:00Z",
  "vehicles": {
    "VIN-0001": { "state": "COMPLETE", "version": "2.0.0" },
    "VIN-0003": { "state": "DOWNLOADING" }
  }
}
```

**`GET /versions` response shape:**
```json
{ "versions": ["1.5.0", "2.0.0", "2.1.0-beta"] }
```

Proxies HawkBit's distribution-set listing and returns version strings.

**`/ws/campaigns` WebSocket protocol:**

On connect, the backend immediately sends a full snapshot of all current campaign states. Subsequent messages are individual per-vehicle transition events driven by the `poll_campaign_state` reconciliation loop. This guarantees mid-campaign clients have consistent state without a separate REST call.

```json
// Sent once on connect
{ "type": "snapshot", "campaigns": { "<campaign_id>": { } } }

// Sent on each per-vehicle state transition
{ "type": "transition", "campaign_id": "uuid", "vin": "VIN-0001", "state": "INSTALLING" }
```

Campaign creation is not currently broadcast over the WS. The frontend merges the `POST /campaigns` response into its local reactive map so the creating tab gets the card immediately. Other already-open tabs see it on their next reconnect snapshot. Fixing this to broadcast a `Created` event would require promoting `CampaignEvent` from a struct to an enum, which is left as a small follow-up.

### 7.7 Updated data model

`VehicleRecord.software_version` is already present and static in v1. On a successful OTA, the ota-agent writes the new version to its local Kuksa Databroker via gRPC. The kuksa2mqtt sidecar no longer streams this attribute to the backend in v2 (it was only ever a static seed signal), so the backend does not auto-update `VehicleRecord.software_version` from telemetry. If the frontend needs the new version displayed immediately, it should read it from the matching `Campaign.vehicles[vin]` entry when the state reaches `Complete { version: ... }`. The current Vue drawer does this.

### 7.8 New crates

| Crate | Role |
|---|---|
| `reqwest` | HTTP client for HawkBit Management API and DDI (also pulled in by ota-agent with `default-features = false` + `rustls-tls` to keep the Docker builder free of OpenSSL) |
| `uuid` | Campaign id and gateway token generation |

---

## 8. Frontend changes

### 8.1 New panel: `CampaignPanel.vue`

Opens as a second right-side drawer next to the existing `FleetTable`, toggled from the map via a separate icon button. Two sub-sections:

**Campaign launcher:**
- Version selector populated by `GET /versions` on mount.
- VIN checkboxes for every known vehicle, pre-checked by default.
- Launch button calling `POST /campaigns`. On success the created campaign is emitted upward and merged into the composable's reactive map immediately so the new card appears without waiting for a WS reconnect.

**Active campaigns list:**
- One card per campaign: target version, launch time, per-vehicle state chips updated live via `/ws/campaigns`.

### 8.2 Updated view: `VehicleDrawer.vue`

- `software_version` from v1 still displayed.
- New "Update" row shown when the vehicle is part of an active campaign. Displays a colour-coded chip reflecting the latest `VehicleUpdateState` from the reactive `campaigns` map.

### 8.3 New composable: `useCampaignSocket.ts`

Mirrors `useFleetSocket.ts`. Opens `/ws/campaigns`; on `snapshot` it replaces the reactive `Record<campaignId, Campaign>`; on `transition` it merges into the matching campaign's `vehicles[vin]` entry. Reconnects with exponential backoff.

### 8.4 New types

`frontend/src/types.ts` gains `VehicleUpdateState`, `Campaign`, and a `WsCampaignMessage` discriminated union mirroring the Rust enum shapes.

---

## 9. docker-compose additions

```yaml
x-ota-agent-defaults: &ota-agent-defaults
  build: ./ota-agent
  image: ota-agent:local
  depends_on:
    mosquitto:
      condition: service_healthy
    hawkbit:
      condition: service_healthy   # self-registration requires DDI reachable
  restart: on-failure

services:

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB:       hawkbit
      POSTGRES_USER:     hawkbit
      POSTGRES_PASSWORD: hawkbit
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U hawkbit -d hawkbit"]
      interval: 5s
      timeout: 3s
      retries: 10

  hawkbit:
    image: hawkbit/hawkbit-update-server:0.3.0M7
    ports:
      - "8083:8080"
    environment:
      SPRING_DATASOURCE_URL:               jdbc:postgresql://postgres:5432/hawkbit
      SPRING_DATASOURCE_USERNAME:          hawkbit
      SPRING_DATASOURCE_PASSWORD:          hawkbit
      SPRING_DATASOURCE_DRIVER_CLASS_NAME: org.postgresql.Driver
      SPRING_JPA_DATABASE:                 POSTGRESQL
      HAWKBIT_SERVER_IM_USERS_0_USERNAME:    ${HAWKBIT_USER:-admin}
      HAWKBIT_SERVER_IM_USERS_0_PASSWORD:    "{noop}${HAWKBIT_PASSWORD:-admin}"
      HAWKBIT_SERVER_IM_USERS_0_PERMISSIONS: ALL
    depends_on:
      postgres:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://localhost:8080/UI/login"]
      interval: 10s
      timeout: 5s
      retries: 10
      start_period: 60s

  ota-agent-01:
    <<: *ota-agent-defaults
    environment:
      VEHICLE_VIN:         VIN-0001
      MQTT_HOST:           mosquitto
      KUKSA_HOST:          databroker-01
      FAILURE_RATE:        "0.2"
      DOWNLOAD_DELAY_SECS: "5"
      INSTALL_DELAY_SECS:  "3"
  # × 20
```

`ota-agent` follows the same `build` + `image` extension-field pattern as `kuksa2mqtt`. Declaring both keys together causes BuildKit to build the image once and tag it as `ota-agent:local`, which is then reused across all 20 instances without redundant builds or a Docker Hub pull. Both Dockerfiles use BuildKit-native cache mounts to avoid re-downloading and re-compiling crates on every build.

**Backend service additions:**
- `depends_on: hawkbit: { condition: service_healthy }` so the backend only boots once HawkBit is ready to accept Management API calls.
- Env: `HAWKBIT_URL`, `HAWKBIT_USER`, `HAWKBIT_PASSWORD`, driven from `.env` so the backend and HawkBit never disagree on the admin credentials.

**`.env.example` at the repo root** supplies:
- `HAWKBIT_USER`, `HAWKBIT_PASSWORD` for the Management API (default `admin`/`admin`).
- `HAWKBIT_TOKEN` is documented in the example file but is not actually used now that the backend provisions the DDI gateway token dynamically. The variable is kept only so operators who deliberately want to pin a fixed token can PUT it into the tenant config out of band.

---

## 10. Repository structure additions

```
sdv-fleet-management/
├── ota-agent/                   (new)
│   ├── Dockerfile               # BuildKit-native; mirrors kuksa2mqtt/Dockerfile
│   ├── Cargo.toml               # reqwest + rustls-tls (no openssl / pkg-config)
│   └── src/
│       └── main.rs              # MQTT gateway-token listener
│                                #   + DDI poll loop
│                                #   + per-deployment state machine
│                                #   + cancel-action handling
│                                #   + gRPC Databroker writer on COMPLETE
├── backend/
│   └── src/
│       ├── hawkbit.rs           (new; Management API client)
│       ├── campaign.rs          (new; in-memory CampaignStore)
│       ├── mqtt.rs              (updated: telemetry + retained gateway-token)
│       ├── api/campaigns.rs     (new; REST + WS handlers)
│       └── main.rs              (updated: poll_campaign_state +
│                                 hydrate_campaigns tasks)
└── frontend/
    └── src/
        ├── CampaignPanel.vue    (new)
        ├── VehicleDrawer.vue    (updated; update-state chip)
        ├── App.vue              (updated; campaign-panel toggle)
        ├── types.ts             (updated; Campaign/VehicleUpdateState)
        └── useCampaignSocket.ts (new)
```

---

## 11. Open decisions

All previous open decisions are resolved:

- **Available versions list.** Resolved: `GET /versions` endpoint backed by HawkBit distribution sets seeded at startup (`1.5.0`, `2.0.0`, `2.1.0-beta`). Frontend version selector calls this on mount.
- **Campaign broadcast granularity.** Resolved: snapshot on connect, then individual per-vehicle transitions. See §7.6 for the full WebSocket protocol. Campaign-creation events are not yet on the WS; the creating tab merges locally from the POST response (see §7.6 for the follow-up).
- **HawkBit authentication.** Resolved in two pieces:
  - *Management API* uses HTTP Basic, with credentials driven from `.env` into both the backend and the HawkBit container so they cannot drift.
  - *DDI API* uses a gateway token. The backend provisions it at startup via `PUT /rest/v1/system/configs/authentication.gatewaytoken.*`, then broadcasts it to ota-agents as a retained MQTT message. No static token in the demo environment.

**New non-blocking follow-ups identified during implementation:**

- Broadcast campaign-creation events on `/ws/campaigns` so that all open tabs see new campaigns without waiting for a reconnect (requires converting `CampaignEvent` from struct to enum).
- Extend rollback UX: `VehicleUpdateState::Failed` is visible in the dashboard but no retry action is wired up.
