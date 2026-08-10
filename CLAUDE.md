# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Running the stack

```sh
# Full stack. HawkBit takes 60–120 s to become healthy on a cold start and the
# backend waits for it, so expect a slow first boot.
docker compose up

# Rebuild everything
docker compose up --build

# Rebuild only the OTA agent image (shared by all ota-agent-* services)
docker compose build ota-agent-01

# Stream logs for a specific service
docker compose logs -f backend
docker compose logs -f ota-agent-01
```

## Per-component development

**Backend** (`backend/` — Rust/axum):
```sh
cd backend
cargo check                         # fast type-check
cargo build                         # debug build
cargo test                          # needs INFLUXDB_TOKEN set to anything
RUST_LOG=backend=debug cargo run    # needs INFLUXDB_*, HAWKBIT_* env
```

**OTA agent** (`ota-agent/` — Rust/tonic):
```sh
cd ota-agent
cargo check
cargo build
# proto/kuksa/val/v1/ is compiled by build.rs via tonic-build (prost), and the
# shared ../proto/ota/v1/ota.proto via rust-protobuf. protobuf-compiler must be
# installed for the former: apt-get install protobuf-compiler
```

**Frontend** (`frontend/` — Vue 3/Vite):
```sh
cd frontend
npm install
VITE_BACKEND_URL=http://localhost:3000 npm run dev   # dev server on :5173
npm run build                                         # type-checks then bundles
```

**Per-vehicle recordings** (`csv-provider/`, Python, no deps):
```sh
python3 csv-provider/generate_vehicle_recordings.py --vehicles 3
```

## Architecture — data flow

Telemetry uses the Eclipse SDV Fleet Management blueprint's components unmodified,
pulled from GHCR. Only the OTA path and the operator backend/UI are ours.

```
CSV Provider (×3)  →  Kuksa Databroker (×3, gRPC, port 55556 in-container)
                              ↓
                      FMS Forwarder (×3)
                              ↓  uProtocol Publish, up://<VIN>/D100/1/D100
                      Zenoh router (port 7447)
                              ↓
                      FMS Consumer  →  InfluxDB (port 8086)
                                            ↓ Flux query, ~1 Hz
                      Rust backend (axum, port 3000)
                          ├── GET /fleet, /vehicles/:vin  →  DashMap store
                          └── WS  /ws/fleet               →  broadcast::Sender<PositionEvent>
                                            ↓
                                  Vue 3 frontend (port 8090)

OTA:  backend  →  HawkBit Management API (rollouts)  →  HawkBit (port 8083)
      ota-agent (×3)  →  HawkBit DDI poll loop  →  gRPC Set Vehicle.SoftwareVersion
      ota-agent (×3)  →  uProtocol Notification  →  backend
                         up://<VIN>/D102/1/0 -> up://fms-ota-orchestrator/D103/1/0
```

## Key design decisions

**The backend reads positions from InfluxDB, not from uProtocol.** The FMS Consumer already subscribes to the vehicle status topic and writes everything to InfluxDB, so `backend/src/influx.rs` polls InfluxDB rather than adding a second subscriber. Measurement and field names (`snapshot`, `latitude`, `longitude`, tag `vin`) are the contract with `influx-client` in the blueprint.

**Positions only broadcast on change.** The forwarder reports on a timer, so `influx::run` compares against the store and skips unchanged coordinates; otherwise a stationary vehicle would emit an event every poll.

**The gateway token is deployment config, not a runtime hand-off.** Both the backend and the agents read `HAWKBIT_GATEWAY_TOKEN`; the backend provisions that exact value on HawkBit's DEFAULT tenant at startup. This replaced a retained-MQTT broadcast and is why the stack no longer needs a broker. Agents may log `401` until the backend has provisioned it, and retry.

**OTA state reaches the back end as uProtocol Notifications.** The agent sends one on every transition (`ota-agent/src/uprotocol.rs`), addressed to the orchestrator; `backend/src/ota_listener.rs` receives them and drives `CampaignStore`. A Notification has a single distinct addressee, which suits a status report better than a Publish. The HawkBit Management API poll stays as reconciliation, repairing anything a dropped notification leaves stale and rehydrating after a restart. Set `HAWKBIT_RECONCILE_ENABLED=false` to verify the notification path alone drives a campaign to completion.

**The agent still speaks DDI over HTTP.** Only reporting runs over uProtocol. A full uProtocol-to-DDI bridge would need a cloud-to-vehicle channel, and upstream `up-transport-hono-kafka::send()` is `UNIMPLEMENTED` while the Hono MQTT transport only implements `send`, so a bridge would be Zenoh-only.

**The OTA contract is compiled twice, with different codegen.** `proto/ota/v1/ota.proto` is shared by both crates and compiled with rust-protobuf, because up-rust's `UPayload` conversions need `protobuf::Message`; the agent's Kuksa protos stay on prost/tonic for gRPC. Both crates therefore build from the repository root (see `.dockerignore`).

**Notifications are correlated to campaigns by VIN, not action id.** The agent reports `(vin, action_id)`; the dashboard is organised by campaign, and HawkBit action ids are not tracked. `CampaignStore::active_campaign_for_vin` picks the most recent non-terminal campaign containing that VIN.

**BuildKit deduplicates the OTA agent build.** All agents declare the same build (context `.`, dockerfile `ota-agent/Dockerfile`) and `image: ota-agent:local` via the shared YAML anchor. BuildKit builds once; the `image:` tag prevents Docker pulling `ota-agent:local` from Docker Hub.

**Backend store is pre-populated at startup.** `main.rs` reads `seed/vehicles.json` (mounted read-only) and inserts a `VehicleRecord` per vehicle before ingest starts, so `GET /fleet` returns data immediately and `update_position` always finds a matching VIN. Positions for a VIN not in that file are ignored. `software_version` comes only from this seed file.

**Broadcast channel fanout.** `AppState` holds a `broadcast::Sender<PositionEvent>`. `ws.rs` subscribes with `state.tx.subscribe()` *before* the WebSocket upgrade to avoid missing events during the handshake. `tokio::select!` in `handle_socket` must drain incoming WebSocket frames concurrently — without it, pings pile up and stall the connection.

**`Vehicle.SoftwareVersion` is a custom VSS extension.** It does not exist in the COVESA standard catalogue (`Vehicle.VersionVSS` is the VSS model version, not the vehicle's software). It is declared in `spec/overlay/ota.vspec` and applied to `spec/overlay/vss.json`, which the databrokers load as their metadata file. Without it the databroker rejects the agent's write with `404 not_found`. Note `set_string` in the agent inspects the response body, because the databroker reports this as a per-datapoint error rather than a gRPC error.

**`fms-consumer` is amd64-only upstream.** It is pinned with `platform: linux/amd64` and runs under emulation on arm64 hosts. `fms-forwarder` is the only multi-arch blueprint image.

**The project is Apache-2.0.** It was relicensed from EPL-2.0 so components can be contributed to the Eclipse SDV Fleet Management blueprint, which is Apache-2.0; every contributor at the time was a Lunatech employee. Every source file carries an Apache-2.0 SPDX header, and files that cannot carry comments use a `<filename>.license` sidecar. See [NOTICE.md](NOTICE.md) for the third-party content inherited from the blueprint.

## Port reference

| Service | Host port |
|---|---|
| Kuksa Databrokers (VIN-0001–0003) | 55556–55558 |
| Zenoh router | 7447 |
| InfluxDB | 8086 |
| Rust backend | 3000 |
| Eclipse HawkBit | 8083 |
| Frontend | 8090 (`FRONTEND_PORT`) |

## Verifying the pipeline manually

```sh
# Telemetry reaching InfluxDB
docker compose logs fms-consumer | tail

# Backend REST
curl -s http://localhost:3000/fleet | jq '.[0]'
curl http://localhost:3000/health

# Backend WebSocket
websocat ws://localhost:3000/ws/fleet

# Launch an OTA campaign
curl -s -X POST http://localhost:3000/campaigns \
  -H 'Content-Type: application/json' \
  -d '{"version":"2.0.0","vins":["VIN-0001","VIN-0002","VIN-0003"]}' | jq

# Swagger UI
open http://localhost:3000/docs
```
