# Fleet Management Demo — v2 Specification

**Version:** 2.0 (draft)
**Status:** In progress
**Audience:** External clients and prospects
**Builds on:** specs.md (v1)

---

## 1. Overview

v2 extends the live fleet dashboard with OTA (over-the-air) software update campaigns powered by Eclipse HawkBit. An operator selects a set of vehicles and a target software version directly from the Vue dashboard, launches a campaign, and watches per-vehicle progress in real time. A configurable failure rate makes the demo resilient to the "happy path only" criticism and demonstrates rollback awareness.

The existing v1 data pipeline (Kuksa → MQTT → Rust backend → browser) is unchanged. v2 adds a second flow alongside it: HawkBit (campaign store) ↔ Rust backend (OTA bridge) ↔ MQTT ↔ ota-agent (per-vehicle simulator).

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
- Retry logic for offline ota-agents. If an agent is unreachable when a command is published, the vehicle remains in `PENDING` indefinitely. Operators must restart the affected container manually. This is a known demo limitation; `restart: on-failure` in docker-compose mitigates the common crash case.

---

## 3. Architecture

### 3.1 Component overview

```
Browser Dashboard
    │  REST  POST /campaigns, GET /campaigns/{id}
    │  REST  GET /versions
    │  WS    /ws/campaigns  (live per-vehicle status)
    │  REST  GET /fleet, GET /vehicles/{vin}        <- unchanged from v1
    │  WS    /ws/fleet                              <- unchanged from v1
    v
Rust Backend (axum)
    │  HawkBit Management API  <- register targets, create rollouts, report status
    │  MQTT publish  kuksa/{vin}/ota/command
    │  MQTT subscribe  kuksa/{vin}/ota/status       <- new subscription
    │  MQTT subscribe  kuksa/+/telemetry/#          <- unchanged from v1
    v
Eclipse Mosquitto (unchanged)
    ^  kuksa/{vin}/ota/status  (ota-agent -> backend)
    v  kuksa/{vin}/ota/command (backend -> ota-agent)
    │
┌───────────────────────────────────────────────────────┐
│  Vehicle fleet — 20 instances                         │
│                                                       │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────┐  │
│  │   Kuksa      │  │ kuksa2mqtt   │  │  ota-agent  │  │
│  │ Databroker   │<>│  sidecar     │  │  (new)      │  │
│  │ (gRPC)       │  │              │  │             │  │
│  └──────────────┘  └──────────────┘  └─────────────┘  │
│         × 20 vehicles (60 containers total)           │
└───────────────────────────────────────────────────────┘

Eclipse HawkBit (new)
    │  stores: targets (vehicles), distribution sets (versions), rollouts (campaigns)
    <- Management API calls from Rust backend
```

### 3.2 HawkBit concepts mapped to this demo

| HawkBit concept | Demo mapping |
|---|---|
| Target | One vehicle (keyed by VIN) |
| Distribution Set | A named software version (e.g. `fleet-fw:2.0.0`) |
| Rollout | One campaign (covers N vehicles) |
| Deployment | Per-vehicle assignment within a rollout |

### 3.3 Port allocation (additions)

| Service | Host port |
|---|---|
| Eclipse HawkBit | 8083 |
| All v1 services | unchanged |

---

## 4. New component: Eclipse HawkBit

HawkBit runs as a single Docker container using its standalone Spring Boot image with an H2 in-memory database (sufficient for demo use).

> **Note:** The H2 in-memory database means a HawkBit container restart loses all campaign history. This is expected behaviour in the demo environment.

**Responsibilities:**
- Persist vehicle targets (registered by the backend at startup)
- Persist distribution sets (software versions available to deploy, seeded at startup)
- Persist rollouts and track per-deployment status
- Expose the Management API consumed by the Rust backend

**Configuration:**
- Authentication: static bearer token set via `HAWKBIT_TOKEN` env var, shared between the HawkBit container and the backend. Fully disabled auth is avoided even in the demo environment to prevent accidental exposure if the compose stack is run on a non-loopback interface.
- Database: H2 in-memory (no separate DB container needed)
- Port: 8083

**Backend interactions:**
1. On startup: `PUT /rest/v1/targets/{vin}` for each of the 20 vehicles
2. On startup: seed three distribution sets (`1.5.0`, `2.0.0`, `2.1.0-beta`) if not already present
3. On `POST /campaigns`: create a Rollout targeting selected VINs against an existing Distribution Set
4. Background poll every 5 seconds: reconcile per-vehicle deployment state into the in-memory DashMap
5. On MQTT status received: `POST /rest/v1/targets/{vin}/deployments/{id}/feedback` to mark complete/failed in HawkBit, then update the DashMap from the HawkBit response

HawkBit is the authoritative state store. The DashMap is always updated from HawkBit responses, not directly from raw MQTT payloads, keeping a single write path and preventing divergence.

---

## 5. New component: ota-agent

A new Rust binary (`ota-agent/`) running as a separate container per vehicle. It is the simulated ECU update client.

**Responsibilities:**
- Subscribe to `kuksa/{vin}/ota/command`
- On command received: run the simulated update state machine
- Publish state transitions to `kuksa/{vin}/ota/status`
- On `COMPLETE`: write the new version string to `Vehicle.SoftwareVersion` in the local Databroker via gRPC

**State machine:**

```
PENDING -> DOWNLOADING (delay: DOWNLOAD_DELAY_SECS) -> INSTALLING (delay: INSTALL_DELAY_SECS)
                                                     -> COMPLETE  (probability: 1 - FAILURE_RATE)
                                                     -> FAILED    (probability: FAILURE_RATE)
```

**Environment variables:**

| Variable | Default | Description |
|---|---|---|
| `VEHICLE_VIN` | required | VIN this agent manages |
| `MQTT_HOST` | `mosquitto` | MQTT broker hostname |
| `KUKSA_HOST` | required | Local Databroker hostname |
| `KUKSA_PORT` | `55555` | Local Databroker gRPC port |
| `FAILURE_RATE` | `0.2` | Probability (0.0–1.0) that an install fails |
| `DOWNLOAD_DELAY_SECS` | `5` | Simulated download duration |
| `INSTALL_DELAY_SECS` | `3` | Simulated install duration |

**MQTT message formats:**

Command (backend → ota-agent):
```json
{ "campaign_id": "uuid", "version": "2.0.0" }
```

Status (ota-agent → backend):
```json
{ "campaign_id": "uuid", "vin": "VIN-0001", "state": "DOWNLOADING" }
{ "campaign_id": "uuid", "vin": "VIN-0001", "state": "INSTALLING" }
{ "campaign_id": "uuid", "vin": "VIN-0001", "state": "COMPLETE", "version": "2.0.0" }
{ "campaign_id": "uuid", "vin": "VIN-0001", "state": "FAILED", "error": "simulated failure" }
```

---

## 6. MQTT topic additions

The two reserved topics from v1 are now active:

```
kuksa/{vin}/ota/command    backend -> ota-agent   (new, QoS 1)
kuksa/{vin}/ota/status     ota-agent -> backend   (new, QoS 1)
```

The backend subscribes to `kuksa/+/ota/status` with a single wildcard, mirroring the `kuksa/+/telemetry/#` pattern already in use.

**Known limitation:** commands are published at QoS 1 but are not retained. If an ota-agent is not connected when the command is published, the message is lost and the vehicle remains in `PENDING` indefinitely. Recovery requires manually restarting the affected container. The `restart: on-failure` compose policy mitigates the crash case but does not handle a clean startup race.

---

## 7. Backend changes

### 7.1 New module: `hawkbit.rs`

HawkBit Management API client. This module is the single point of write authority for deployment state.

Responsibilities:
- Register all 20 vehicles as targets on startup
- Seed distribution sets for `1.5.0`, `2.0.0`, `2.1.0-beta` on startup if not present
- `create_distribution_set(name, version) -> DistributionSetId`
- `create_rollout(dist_id, vins) -> RolloutId`
- `report_deployment_result(vin, deployment_id, success)` — called immediately on MQTT status receipt
- `poll_rollout_status(rollout_id) -> Vec<(Vin, DeploymentState)>` — called by a background task every 5 seconds; results are diffed against the DashMap and any transitions are broadcast on `/ws/campaigns`

### 7.2 New module: `campaign.rs`

In-memory campaign state store (`DashMap<CampaignId, Campaign>`). Always written from HawkBit poll results or HawkBit feedback responses, never directly from raw MQTT payloads.

```rust
struct Campaign {
    id:       Uuid,
    version:  String,
    vehicles: HashMap<String, VehicleUpdateState>,
    created:  DateTime<Utc>,
}

enum VehicleUpdateState {
    Pending,
    Downloading,
    Installing,
    Complete { version: String },
    Failed { error: String },
}
```

### 7.3 Updated module: `mqtt.rs`

- Add subscription to `kuksa/+/ota/status`
- On status message: call `hawkbit.rs` to report result, then update `campaign.rs` store from the HawkBit response, then broadcast the transition on `/ws/campaigns`

### 7.4 New REST endpoints

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

Proxies `GET /rest/v1/distributionsets` from HawkBit and returns version strings.

**`/ws/campaigns` WebSocket protocol:**

On connect, the backend immediately sends a full snapshot of all current campaign states. Subsequent messages are individual transition events. This ensures clients connecting mid-campaign have consistent state without a separate REST call.

```json
// Sent once on connect
{ "type": "snapshot", "campaigns": { "<campaign_id>": { } } }

// Sent on each state transition
{ "type": "transition", "campaign_id": "uuid", "vin": "VIN-0001", "state": "INSTALLING" }
```

### 7.5 Updated data model

`VehicleRecord.software_version` is already present and static in v1. After a successful OTA, the backend updates this field in the `DashMap` so `GET /fleet` and `GET /vehicles/{vin}` reflect the new version immediately without a restart.

### 7.6 New crates

| Crate | Role |
|---|---|
| `reqwest` | HTTP client for HawkBit Management API calls |
| `uuid` | Campaign and deployment ID generation |

---

## 8. Frontend changes

### 8.1 New view: Campaign panel

A new tab alongside the map with two sub-sections:

**Campaign launcher:**
- Version selector — dropdown populated by `GET /versions` on mount
- Vehicle selector — checkboxes for all 20 vehicles (pre-select all by default)
- Launch button — calls `POST /campaigns`

**Active campaigns list:**
- One card per campaign: target version, launch time, aggregate progress bar (X / N complete)
- Per-vehicle state chips inside each card, updated live via WebSocket

### 8.2 Updated view: Vehicle detail drawer

- `software_version` already displayed in v1; remains the same field
- New: active update state badge if the vehicle is part of a running campaign (e.g. "Installing 2.0.0...")
- Version field updates to the new version on `COMPLETE`

### 8.3 New composable: `useCampaignSocket.ts`

Mirrors `useFleetSocket.ts`. Opens `/ws/campaigns` and waits for the initial `snapshot` message to hydrate the reactive `Map<campaignId, Campaign>`. Subsequent `transition` messages are merged in by `(campaignId, vin)` key. If the socket drops and reconnects, the snapshot message re-hydrates state cleanly without requiring a page reload. No separate `GET /campaigns/{id}` call is needed on connect.

### 8.4 New component: `CampaignPanel.vue`

Campaign creation form + live status grid. Consumes `useCampaignSocket`.

---

## 9. docker-compose additions

```yaml
# build + image together: BuildKit deduplicates the build across all 20 agents
# and tags the result as ota-agent:local, mirroring the kuksa2mqtt pattern.
x-ota-agent-defaults: &ota-agent-defaults
  build: ./ota-agent
  image: ota-agent:local
  depends_on:
    mosquitto:
      condition: service_healthy
  restart: on-failure

services:

  hawkbit:
    image: hawkbit/hawkbit-update-server:latest
    ports:
      - "8083:8080"
    environment:
      SPRING_DATASOURCE_URL: jdbc:h2:mem:hawkbit
      HAWKBIT_TOKEN: ${HAWKBIT_TOKEN}
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/management/health"]
      interval: 10s
      timeout: 5s
      retries: 10

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

`ota-agent` follows the same `build` + `image` extension-field pattern as `kuksa2mqtt`. Declaring both keys together causes BuildKit to build the image once and tag it as `ota-agent:local`, which is then reused across all 20 instances without redundant builds or a Docker Hub pull. Both Dockerfiles use BuildKit-native cache mounts (`--mount=type=cache,target=/usr/local/cargo/registry` and `--mount=type=cache,target=/app/target`) to avoid re-downloading and re-compiling crates on every build. BuildKit is activated at the CLI level with `DOCKER_BUILDKIT=1` (or `COMPOSE_DOCKER_CLI_BUILD=1` for older compose versions) — the repo `README` should document this as a build prerequisite, consistent with the existing `kuksa2mqtt` note.

The backend service gains `hawkbit: { condition: service_healthy }` in its `depends_on` and `HAWKBIT_TOKEN: ${HAWKBIT_TOKEN}` in its environment.

A `.env` file at the repo root supplies `HAWKBIT_TOKEN` for local runs. The value is arbitrary for the demo but must match in both services.

---

## 10. Repository structure additions

```
fleet-demo/
├── ota-agent/                   <- new
│   ├── Dockerfile               # BuildKit-native; mirrors kuksa2mqtt/Dockerfile exactly
│   ├── Cargo.toml
│   └── src/
│       └── main.rs              # MQTT subscriber + state machine + gRPC writer
├── backend/
│   └── src/
│       ├── hawkbit.rs           <- new
│       ├── campaign.rs          <- new
│       └── mqtt.rs              <- updated (ota/status subscription + publish)
└── frontend/
    └── src/
        ├── CampaignPanel.vue    <- new
        └── useCampaignSocket.ts <- new
```

---

## 11. Open decisions

All previous open decisions are resolved:

- **Available versions list** — resolved: `GET /versions` endpoint backed by HawkBit distribution sets seeded at startup (`1.5.0`, `2.0.0`, `2.1.0-beta`). Frontend version selector calls this on mount.
- **Campaign broadcast granularity** — resolved: snapshot on connect, then individual transitions. See section 7.4 for the full WebSocket protocol.
- **HawkBit authentication** — resolved: static bearer token via `HAWKBIT_TOKEN` env var shared between the backend and HawkBit. Fully disabled auth is not used even in the demo environment.
