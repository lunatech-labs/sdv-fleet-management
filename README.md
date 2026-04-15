# SDV Fleet Management — v1 Demo

A client-facing demo showcasing Rust as a high-performance backend for vehicle fleet management, using Eclipse Kuksa to simulate a realistic fleet of 20 vehicles. Live GPS positions flow from per-vehicle Kuksa Databrokers → MQTT → Rust backend → browser map.

---

## Architecture

```
Browser (Vue 3)
    │  REST GET /fleet
    │  WebSocket /ws/fleet
    ▼
Rust Backend (axum · port 3000)
    │  MQTT subscribe: kuksa/+/telemetry/#
    ▼
Eclipse Mosquitto (port 1883)
    ▲
    │  2 dynamic signals/vehicle at 1 Hz (lat/lon)
    │
┌─────────────────────────────────────┐
│  20 vehicles                        │
│  Kuksa Databroker + kuksa2mqtt      │
│  sidecar (ports 55556–55575)        │
└─────────────────────────────────────┘
    ▲
    │  Seed script (Python, runs once)
```

---

## Docker Compose features

### Healthchecks and startup ordering

`mosquitto` exposes a healthcheck using `mosquitto_sub` to confirm the broker is accepting connections. Downstream services use `depends_on` with two different conditions to enforce the correct startup sequence:

- `condition: service_healthy` — waits for the target service's healthcheck to pass (used by sidecars and backend waiting on Mosquitto).
- `condition: service_completed_successfully` — waits for a one-shot container to exit with code 0 (used by sidecars waiting on the seed script).

This guarantees the order: Mosquitto → Databrokers → Seed → Sidecars.

### YAML anchors and extension fields

The `x-sidecar-defaults` block at the top of the file uses Docker Compose's extension field convention (`x-` prefix). It defines shared configuration — `image`, `depends_on`, and `restart` — once, and each sidecar service merges it in with the YAML merge key `<<: *sidecar-defaults`. This avoids repeating the same 8 lines across all 20 services.

### Single build, shared image

All 20 `kuksa2mqtt` sidecar services are identical except for their environment variables. Only `kuksa2mqtt-01` has a `build:` directive; the other 19 reference the same `image: kuksa2mqtt:local` tag. Docker builds the image once and all containers reuse it — avoiding the race condition that occurs when 20 parallel builds try to tar the same build context simultaneously.

To rebuild the sidecar image:
```sh
docker compose build kuksa2mqtt-01
```

### Mosquitto config volume

The Mosquitto broker mounts its config file from `./mosquitto/mosquitto.conf` as a read-only bind mount. This allows the broker configuration (anonymous access, port) to be version-controlled alongside the rest of the project without building a custom image.

---

## Todo

### Infrastructure

- [x] Write `docker-compose.yml`
  - [x] `mosquitto` service with config
  - [x] 20 `databroker-{01..20}` services (ports 55556–55575)
  - [x] `seed` service with `depends_on` + healthcheck gate
  - [x] 20 `kuksa2mqtt-{01..20}` sidecar services
  - [x] `backend` service
  - [x] `frontend` service
- [x] Write Mosquitto config (`mosquitto.conf`) — allow anonymous, listen on 1883

#### Testing the infrastructure

Spin up only the services that exist so far (Mosquitto + all Databrokers):

```sh
docker compose up mosquitto databroker-01 databroker-02 databroker-03 databroker-04 \
  databroker-05 databroker-06 databroker-07 databroker-08 databroker-09 databroker-10 \
  databroker-11 databroker-12 databroker-13 databroker-14 databroker-15 databroker-16 \
  databroker-17 databroker-18 databroker-19 databroker-20
```

**Check Mosquitto is up:**

```sh
# Subscribe to the system topic — should print broker stats every second
mosquitto_sub -h localhost -p 1883 -t '$SYS/#' -v
```

**Check a Databroker is up (requires `grpcurl`):**

```sh
# Should return a list of VSS signal entries (empty at this stage, before seed runs)
grpcurl -plaintext localhost:55556 list
```

**Check all 20 Databroker ports are listening:**

```sh
for port in $(seq 55556 55575); do
  echo -n "port $port: " && nc -z localhost $port && echo "OK" || echo "FAIL"
done
```

**Tear down:**

```sh
docker compose down
```

### Seed Script (`seed/`)

- [x] Create `seed/vehicles.json` — 20-vehicle dataset (VIN, brand, model, software version, initial lat/lon)
- [x] Write `seed/seed.py`
  - [x] Connect to each of the 20 Databrokers over gRPC (`kuksa-client`)
  - [x] Write 4 static VSS signals per vehicle: `Vehicle.VehicleIdentification.VIN`, `Brand`, `Model`, `Vehicle.SoftwareVersion`
  - [x] Write initial `Vehicle.CurrentLocation.Latitude` and `Longitude`
  - [x] Exit with code 0 on success
- [x] Write `seed/Dockerfile`
- [ ] **TODO:** Upgrade `kuksa-client` to 0.5.0 — requires the databroker image to also be upgraded to a version that implements the `PublishValue` RPC. The two must be bumped together. Pinned to 0.4.3 for now.
- [ ] **TODO:** Set up a proper local Python environment for the seed (`requirements.txt` + venv or `uv`) so the script can be run and tested without rebuilding the Docker image.

#### Testing the seed

Start Mosquitto and the Databrokers, then run the seed container in isolation:

```sh
# 1. Bring up Mosquitto and all Databrokers
docker compose up -d mosquitto databroker-01 databroker-02 databroker-03 databroker-04 \
  databroker-05 databroker-06 databroker-07 databroker-08 databroker-09 databroker-10 \
  databroker-11 databroker-12 databroker-13 databroker-14 databroker-15 databroker-16 \
  databroker-17 databroker-18 databroker-19 databroker-20

# 2. Build and run the seed (follows startup logs, exits when done)
docker compose up --build seed
```

The seed should log `✓ seeded` for each vehicle and exit with code 0.

**Verify seeded signals with `grpcurl`:**

```sh
# Install grpcurl (macOS)
brew install grpcurl

# List all services on a databroker
grpcurl -plaintext localhost:55556 list

# List all RPC methods on the VAL service
grpcurl -plaintext localhost:55556 list kuksa.val.v1.VAL

# Describe the Get request message (shows available fields)
grpcurl -plaintext localhost:55556 describe kuksa.val.v1.GetRequest

# Read the VIN from databroker-01
grpcurl -plaintext \
  -d '{"entries": [{"path": "Vehicle.VehicleIdentification.VIN", "fields": ["FIELD_VALUE"]}]}' \
  localhost:55556 kuksa.val.v1.VAL/Get

# Read all five seeded signals from databroker-01
grpcurl -plaintext \
  -d '{"entries": [
    {"path": "Vehicle.VehicleIdentification.VIN",   "fields": ["FIELD_VALUE"]},
    {"path": "Vehicle.VehicleIdentification.Brand",  "fields": ["FIELD_VALUE"]},
    {"path": "Vehicle.VehicleIdentification.Model",  "fields": ["FIELD_VALUE"]},
    {"path": "Vehicle.CurrentLocation.Latitude",     "fields": ["FIELD_VALUE"]},
    {"path": "Vehicle.CurrentLocation.Longitude",    "fields": ["FIELD_VALUE"]}
  ]}' \
  localhost:55556 kuksa.val.v1.VAL/Get
```

Expected: responses containing `"string_value": "VIN-0001"` for the VIN, and numeric values for latitude/longitude.

**Tear down:**

```sh
docker compose down
```

### kuksa2mqtt Sidecar (`kuksa2mqtt/`)

- [x] Initialise Rust crate (`Cargo.toml`)
- [x] Write `src/main.rs`
  - [x] Read config from env: `KUKSA_HOST`, `KUKSA_PORT`, `MQTT_HOST`, `VEHICLE_VIN`
  - [x] Connect to local Databroker via gRPC
  - [x] Connect to Mosquitto via MQTT (`rumqttc`)
  - [x] GPS random walk loop (1 Hz, σ ≈ 0.0002°)
    - [x] Generate new lat/lon delta
    - [x] Write updated values to Databroker
    - [x] Publish to `kuksa/{vin}/telemetry/CurrentLocation/Latitude` and `Longitude`
- [x] Write `Dockerfile`

#### Testing the sidecar

Build and run the full stack up to and including the sidecars:

```sh
# Bring up infrastructure + seed + one sidecar to verify
docker compose up --build mosquitto databroker-01 seed kuksa2mqtt-01
```

**Verify MQTT messages are flowing (requires `mqtt-cli`):**

```sh
brew install mqtt-cli

# Subscribe to all telemetry for VIN-0001 — should see lat/lon messages every second
mqtt sub -h localhost -p 1883 --topic='kuksa/VIN-0001/telemetry/#'
```

Expected output (updating at 1 Hz):
```
kuksa/VIN-0001/telemetry/VehicleIdentification/VIN   VIN-0001
kuksa/VIN-0001/telemetry/VehicleIdentification/Brand Toyota
kuksa/VIN-0001/telemetry/VehicleIdentification/Model Camry
kuksa/VIN-0001/telemetry/CurrentLocation/Latitude    48.8571
kuksa/VIN-0001/telemetry/CurrentLocation/Longitude   2.3529
...
```

**Bring up all 20 sidecars:**

```sh
docker compose up --build
```

### Rust Backend (`backend/`)

- [x] Initialise Rust crate (`Cargo.toml`) with dependencies: `axum`, `rumqttc`, `tokio`, `dashmap`, `serde`/`serde_json`, `chrono`, `tower-http`, `utoipa`/`utoipa-swagger-ui`, `tracing`/`tracing-subscriber`
- [x] `src/models.rs` — define `VehicleRecord` and `PositionEvent` with serde + utoipa derives
- [x] `src/store.rs` — `DashMap<String, VehicleRecord>` wrapper
- [x] `src/mqtt.rs`
  - [x] Connect to Mosquitto on startup
  - [x] Subscribe to `kuksa/+/telemetry/#`
  - [x] Parse topic: extract VIN (segment 1) and signal name (trailing path)
  - [x] Update `DashMap` on each message
  - [x] Broadcast `PositionEvent` on lat/lon updates
- [x] `src/api/fleet.rs`
  - [x] `GET /fleet` — return all vehicle records
  - [x] `GET /vehicles/{vin}` — return single vehicle record
  - [x] `GET /health` — liveness check
- [x] `src/api/ws.rs` — `WS /ws/fleet` — subscribe to broadcast, stream `PositionEvent` JSON
- [x] `src/main.rs` — wire axum router, CORS middleware, OpenAPI/Swagger at `/docs`, start MQTT loop
- [x] Write `Dockerfile`

#### Testing the backend

Start the full stack (infrastructure + seed + sidecars + backend):

```sh
docker compose up --build backend
```

**Health check:**

```sh
curl http://localhost:3000/health
# Expected: 200 OK
```

**Fleet endpoint — all 20 vehicles:**

```sh
curl -s http://localhost:3000/fleet | jq '.[0]'
# Expected: first vehicle record with vin, brand, model, software_version, latitude, longitude, last_seen
```

**Single vehicle:**

```sh
curl -s http://localhost:3000/vehicles/VIN-0001 | jq
# Expected: full VehicleRecord for VIN-0001
# 404 if the VIN doesn't exist
```

**Swagger UI:**

Open [http://localhost:3000/docs](http://localhost:3000/docs) in a browser — should render the interactive OpenAPI documentation.

**WebSocket — live position stream:**

```sh
# Requires websocat: brew install websocat
websocat ws://localhost:3000/ws/fleet
# Expected: a stream of JSON position events at ~1 Hz per vehicle:
# {"vin":"VIN-0003","lat":48.8641,"lon":2.3318}
# {"vin":"VIN-0011","lat":48.8462,"lon":2.3204}
# ...
```

### Frontend (`frontend/`)

- [ ] Scaffold Vue 3 project (`package.json`, Vite config)
- [ ] Install dependencies: `leaflet`, `@vue-leaflet/vue-leaflet`
- [ ] `src/useFleetSocket.ts` — WebSocket composable (connect, parse `PositionEvent`, expose reactive state)
- [ ] `src/MapView.vue`
  - [ ] Full-screen Leaflet map
  - [ ] Place 20 pins on initial `GET /fleet` load
  - [ ] Update pin positions on each WebSocket message
  - [ ] Emit pin-click event with vehicle data
- [ ] `src/VehicleDrawer.vue` — side drawer showing VIN, brand, model, software version on pin click
- [ ] `src/App.vue` — compose map + drawer, call `GET /fleet` on mount, open WebSocket
- [ ] (Optional) Fleet table view — tabular display of all 20 records
- [ ] Write `Dockerfile` (Vite build + static serve)

### End-to-end Validation

- [ ] `docker compose up` — all 43 services start cleanly
- [ ] Seed exits with code 0; sidecars start after seed completes
- [ ] Backend logs show MQTT messages arriving for all 20 VINs
- [ ] `GET http://localhost:3000/fleet` returns 20 vehicle records
- [ ] `GET http://localhost:3000/health` returns `200 OK`
- [ ] `GET http://localhost:3000/docs` renders Swagger UI
- [ ] Browser map shows 20 pins moving in real time
- [ ] Clicking a pin opens the drawer with correct metadata

---

## Port Reference

| Service | Host port |
|---|---|
| Mosquitto MQTT | 1883 |
| Kuksa Databroker VIN-0001–0020 | 55556–55575 |
| Rust backend | 3000 |
| Frontend | 8080 |
