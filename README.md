# Fleet Orchestrator — v1 Demo

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

- [ ] Initialise Rust crate (`Cargo.toml`)
- [ ] Write `src/main.rs`
  - [ ] Read config from env: `KUKSA_HOST`, `KUKSA_PORT`, `MQTT_HOST`, `VEHICLE_VIN`
  - [ ] Connect to local Databroker via gRPC
  - [ ] Connect to Mosquitto via MQTT (`rumqttc`)
  - [ ] GPS random walk loop (1 Hz, σ ≈ 0.0002°)
    - [ ] Generate new lat/lon delta
    - [ ] Write updated values to Databroker
    - [ ] Publish to `kuksa/{vin}/telemetry/CurrentLocation/Latitude` and `Longitude`
- [ ] Write `Dockerfile`

### Rust Backend (`backend/`)

- [ ] Initialise Rust crate (`Cargo.toml`) with dependencies: `axum`, `rumqttc`, `tokio`, `dashmap`, `serde`/`serde_json`, `chrono`, `tower-http`, `utoipa`/`utoipa-swagger-ui`, `tracing`/`tracing-subscriber`
- [ ] `src/models.rs` — define `VehicleRecord` and `PositionEvent` with serde + utoipa derives
- [ ] `src/store.rs` — `DashMap<String, VehicleRecord>` wrapper
- [ ] `src/mqtt.rs`
  - [ ] Connect to Mosquitto on startup
  - [ ] Subscribe to `kuksa/+/telemetry/#`
  - [ ] Parse topic: extract VIN (segment 1) and signal name (trailing path)
  - [ ] Update `DashMap` on each message
  - [ ] Broadcast `PositionEvent` on lat/lon updates
- [ ] `src/api/fleet.rs`
  - [ ] `GET /fleet` — return all vehicle records
  - [ ] `GET /vehicles/{vin}` — return single vehicle record
  - [ ] `GET /health` — liveness check
- [ ] `src/api/ws.rs` — `WS /ws/fleet` — subscribe to broadcast, stream `PositionEvent` JSON
- [ ] `src/main.rs` — wire axum router, CORS middleware, OpenAPI/Swagger at `/docs`, start MQTT loop
- [ ] Write `Dockerfile`

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
