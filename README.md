# SDV Fleet Management — v2 Demo

A client-facing demo showcasing Rust as a high-performance backend for vehicle fleet management, using Eclipse Kuksa to simulate a realistic fleet of 20 vehicles. Live GPS positions flow from per-vehicle Kuksa Databrokers → MQTT → Rust backend → browser map. V2 adds over-the-air (OTA) software update campaigns powered by Eclipse HawkBit: create a rollout from the UI, watch vehicle markers change colour as updates progress, and track per-vehicle state in real time via WebSocket.

---

## Quickstart

```sh
git clone git@github.com:lunatech-labs/sdv-fleet-management.git
cd sdv-fleet-management

# Copy .env.example to .env — set HAWKBIT_TOKEN, HAWKBIT_USER, HAWKBIT_PASSWORD.
cp .env.example .env

docker compose up
```

Then visit: `http://localhost:8080`.

> **First run note:** HawkBit takes 60 to 90 seconds to initialise on first boot. The frontend will display vehicles as soon as the backend is ready, but the Campaign panel requires HawkBit to be healthy before a rollout can be launched.

---

## Showcase

A guided walkthrough of the main features. All steps assume the full stack is running at `http://localhost:8080`.

### 1. Live fleet map

Open `http://localhost:8080`. You will see 20 vehicle pins spread across Paris, each moving independently at 1 Hz. Positions flow from 20 Kuksa Databrokers through MQTT to the Rust backend, then over a WebSocket to the browser — no polling.

### 2. Vehicle detail

Click any pin to open the side drawer. It shows the vehicle's VIN, manufacturer, model, and current software version. This data is loaded once via `GET /fleet` on page load; the drawer requires no additional request.

### 3. Fleet table and filters

Switch to the Fleet tab to see all 20 vehicles in a sortable table. Applying a filter also updates which pins are visible on the map, so both views stay in sync.

### 4. OTA update campaign

Open the Campaign panel (the update icon in the toolbar).

1. Select a target software version from the dropdown (populated by `GET /versions`).
2. Check the vehicles you want to update, or leave all 20 selected.
3. Click **Launch**. The campaign card appears immediately.

Each vehicle progresses through its own state machine:

```
Pending  →  Downloading  →  Installing  →  Complete
                                        ↘  Failed
```

A 20% simulated failure rate means some vehicles will fail, showing realistic error handling rather than a guaranteed happy path. State chips on the campaign card update live via WebSocket.

### 5. Updated software version

Once a vehicle reaches **Complete**, click its map pin. The software version in the drawer reflects the newly installed version, read from the campaign state.

### 6. API explorer

Visit `http://localhost:3000/docs` for the Swagger UI. All REST endpoints are documented and executable from the browser — useful for showing the clean API surface and the OpenAPI spec generated directly from the Rust source.

---

## Architecture

```
Browser (Vue 3)
    │  REST GET /fleet, /campaigns, /versions
    │  WebSocket /ws/fleet, /ws/campaigns
    ▼
Rust Backend (axum · port 3000)
    │  MQTT subscribe: kuksa/+/telemetry/#
    │  REST Management API
    ▼                         ▼
Eclipse Mosquitto         Eclipse HawkBit (port 8083)
(port 1883)                   │  DDI poll loop
    ▲                         ▼
    │  2 dynamic signals   OTA agents (×20, Rust)
    │  per vehicle at 1 Hz     │  gRPC Set SoftwareVersion
    │  (lat/lon)               │  MQTT publish ota/{vin}/state
    │                          │
┌──────────────────────────────────────┐
│  20 vehicles                         │
│  Kuksa Databroker + kuksa2mqtt       │
│  sidecar (ports 55556–55575)         │
└──────────────────────────────────────┘
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

### BuildKit deduplication

All 20 `kuksa2mqtt` sidecar services share the same `build: ./kuksa2mqtt` and `image: kuksa2mqtt:local` in the YAML anchor. Docker BuildKit is smart enough to build the image only once even though all 20 services declare it — subsequent services hit the cache immediately. The `image:` tag is also what prevents Docker from trying to pull `kuksa2mqtt:local` from Docker Hub on first run.

To rebuild the sidecar image:
```sh
docker compose build kuksa2mqtt-01
```

### Mosquitto config volume

The Mosquitto broker mounts its config file from `./mosquitto/mosquitto.conf` as a read-only bind mount. This allows the broker configuration (anonymous access, port) to be version-controlled alongside the rest of the project without building a custom image.

---

## Testing

### Infrastructure

```sh
docker compose up mosquitto databroker-01 ... databroker-20
```

```sh
# Mosquitto is accepting connections
mosquitto_sub -h localhost -p 1883 -t '$SYS/#' -v

# All 20 databroker ports are listening
for port in $(seq 55556 55575); do
  echo -n "port $port: " && nc -z localhost $port && echo "OK" || echo "FAIL"
done

# A databroker responds to gRPC (requires grpcurl: brew install grpcurl)
grpcurl -plaintext localhost:55556 list
```

### Seed

```sh
docker compose up -d mosquitto databroker-01 ... databroker-20
docker compose up --build seed
# Expected: "✓ seeded" for each vehicle, exits 0
```

**Unit tests** (no Docker needed):

```sh
cd seed
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
pytest tests/ -v
```

Verify seeded signals with `grpcurl`:

```sh
# Read the VIN from databroker-01
grpcurl -plaintext \
  -d '{"entries": [{"path": "Vehicle.VehicleIdentification.VIN", "fields": ["FIELD_VALUE"]}]}' \
  localhost:55556 kuksa.val.v1.VAL/Get

# Read all five seeded signals
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

### kuksa2mqtt sidecar

```sh
docker compose up --build mosquitto databroker-01 seed kuksa2mqtt-01
```

```sh
# MQTT messages flowing at 1 Hz (requires mqtt-cli: brew install mqtt-cli)
mqtt sub -h localhost -p 1883 --topic='kuksa/VIN-0001/telemetry/#'
```

Expected output:
```
kuksa/VIN-0001/telemetry/VehicleIdentification/VIN   VIN-0001
kuksa/VIN-0001/telemetry/VehicleIdentification/Brand Toyota
kuksa/VIN-0001/telemetry/VehicleIdentification/Model Camry
kuksa/VIN-0001/telemetry/CurrentLocation/Latitude    48.8571
kuksa/VIN-0001/telemetry/CurrentLocation/Longitude   2.3529
...
```

### Backend

```sh
docker compose up   # sidecars must be running for MQTT data to flow
```

```sh
curl http://localhost:3000/health
curl -s http://localhost:3000/fleet | jq '.[0]'
curl -s http://localhost:3000/vehicles/VIN-0001 | jq

# OTA campaigns
curl -s http://localhost:3000/versions | jq
curl -s http://localhost:3000/campaigns | jq
curl -s -X POST http://localhost:3000/campaigns \
  -H 'Content-Type: application/json' \
  -d '{"version":"1.1.0","vins":["VIN-0001","VIN-0002"]}' | jq
curl -s http://localhost:3000/campaigns/<id> | jq

# Swagger UI
open http://localhost:3000/docs

# WebSocket live streams (requires websocat: brew install websocat)
websocat ws://localhost:3000/ws/fleet
# {"vin":"VIN-0003","lat":48.8641,"lon":2.3318}

websocat ws://localhost:3000/ws/campaigns
# {"campaign_id":"...","vin":"VIN-0001","state":"Installing"}
```

### Frontend

```sh
docker compose up --build
open http://localhost:8080
```

- 20 vehicle pins visible on the Paris map
- Pins move in real time (~1 Hz)
- Clicking a pin opens the drawer with VIN, brand, model, and software version
- Campaign Panel lets you select a software version and target vehicles, then launch a rollout
- Vehicle markers change colour as OTA state progresses (Pending, Downloading, Installing, Succeeded, Failed)

For local development without Docker:
```sh
cd frontend && npm install
VITE_BACKEND_URL=http://localhost:3000 npm run dev
```

---

## Contributing

### Backend (Rust)

```sh
cd backend

# Format
cargo fmt

# Lint
cargo clippy -- -D warnings

# Test
cargo test
```

All three steps run automatically on every push to `main` via GitHub Actions (`.github/workflows/backend.yml`).

### Frontend (Vue 3 + TypeScript)

```sh
cd frontend

# Lint
npm run lint

# Unit tests
npm test
```

Runs automatically on every push to `main` via GitHub Actions (`.github/workflows/frontend.yml`).

### End-to-end tests (Playwright)

```sh
docker compose up -d          # full stack must be running

cd e2e
npm install
npm run install-browsers      # first time only — downloads Chromium
npm test
```

Overrides (useful when the ports are remapped):

```sh
PLAYWRIGHT_BASE_URL=http://localhost:8080 \
PLAYWRIGHT_BACKEND_URL=http://localhost:3000 \
npm test
```

---

## Port Reference

| Service | Host port |
|---|---|
| Mosquitto MQTT | 1883 |
| Kuksa Databroker VIN-0001–0020 | 55556–55575 |
| Rust backend | 3000 |
| Eclipse HawkBit | 8083 |
| Frontend | 8080 |
