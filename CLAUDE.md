# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Running the stack

```sh
# Full stack — always use this, not `docker compose up --build backend` etc.
# Sidecars are not in the backend's depends_on chain and won't start otherwise.
docker compose up

# Rebuild everything
docker compose up --build

# Rebuild only the sidecar image (shared by all 20 kuksa2mqtt-* services)
docker compose build kuksa2mqtt-01

# Stream logs for a specific service
docker compose logs -f backend
docker compose logs -f kuksa2mqtt-01
```

## Per-component development

**Backend** (`backend/` — Rust/axum):
```sh
cd backend
cargo check                         # fast type-check
cargo build                         # debug build
RUST_LOG=backend=debug cargo run    # run with debug logging (needs MQTT_HOST etc.)
```

**Sidecar** (`kuksa2mqtt/` — Rust/tonic):
```sh
cd kuksa2mqtt
cargo check
cargo build
# Proto files in proto/kuksa/val/v1/ are compiled by build.rs via tonic-build.
# protobuf-compiler must be installed: apt-get install protobuf-compiler
```

**Frontend** (`frontend/` — Vue 3/Vite):
```sh
cd frontend
npm install
VITE_BACKEND_URL=http://localhost:3000 npm run dev   # dev server on :5173
npm run build                                         # type-checks then bundles
```

**Seed** (`seed/` — Python): no local venv is configured yet. Run via Docker:
```sh
docker compose up --build seed
```

## Architecture — data flow

```
Kuksa Databrokers (×20, gRPC)
        ↑ Set (lat/lon at 1 Hz)
kuksa2mqtt sidecars (×20, Rust)
        ↓ MQTT publish  kuksa/{vin}/telemetry/{signal}
Eclipse Mosquitto (port 1883)
        ↓ MQTT subscribe  kuksa/+/telemetry/#
Rust backend (axum, port 3000)
    ├── GET /fleet, GET /vehicles/:vin  →  DashMap store
    └── WS  /ws/fleet                  →  broadcast::Sender<PositionEvent>
                                                ↓
                                        Vue 3 frontend (port 8080)
```

## Key design decisions

**Single build, shared sidecar image.** Only `kuksa2mqtt-01` has a `build:` directive in `docker-compose.yml`. The other 19 sidecars reference `image: kuksa2mqtt:local`. This prevents a parallel-build race condition on the Cargo.lock file. To rebuild: `docker compose build kuksa2mqtt-01`.

**Sidecar reads static vehicle data from env, not the databroker.** Brand, Model, and initial lat/lon come from environment variables (`VEHICLE_BRAND`, `VEHICLE_MODEL`, `VEHICLE_LAT`, `VEHICLE_LON`). The sidecar only *writes* to the databroker (GPS updates via gRPC `Set`). Attempting to read VSS attribute signals back via gRPC was unreliable with this databroker version.

**Backend store is pre-populated at startup.** `main.rs` reads `seed/vehicles.json` (mounted as a read-only volume) and inserts all 20 `VehicleRecord`s into the `DashMap` before MQTT starts. This ensures `GET /fleet` returns data immediately and that `update_position` in `mqtt.rs` always finds a matching VIN. `software_version` is never updated via MQTT — it comes only from this seed file.

**Broadcast channel fanout.** `AppState` holds a `broadcast::Sender<PositionEvent>`. `mqtt.rs` calls `tx.send()` on every lat or lon MQTT message (so twice per vehicle per second). `ws.rs` subscribes with `state.tx.subscribe()` *before* the WebSocket upgrade to avoid missing events during the handshake. `tokio::select!` in `handle_socket` must drain incoming WebSocket frames concurrently — without it, pings pile up and stall the connection.

**`Vehicle.SoftwareVersion` is a custom VSS extension.** It does not exist in the COVESA standard catalog. The databroker rejects writes to it. It is handled as a static field sourced from `vehicles.json` only.

## Port reference

| Service | Host port |
|---|---|
| Mosquitto MQTT | 1883 |
| Kuksa Databrokers (VIN-0001–0020) | 55556–55575 |
| Rust backend | 3000 |
| Frontend | 8080 |

## Verifying the pipeline manually

```sh
# MQTT data flowing from sidecars
mqtt sub -h localhost -p 1883 --topic='kuksa/VIN-0001/telemetry/#'

# Backend REST
curl -s http://localhost:3000/fleet | jq '.[0]'
curl http://localhost:3000/health

# Backend WebSocket
websocat ws://localhost:3000/ws/fleet

# Swagger UI
open http://localhost:3000/docs
```
