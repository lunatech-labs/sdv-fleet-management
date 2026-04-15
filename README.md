# SDV Fleet Management — v1 Demo

A client-facing demo showcasing Rust as a high-performance backend for vehicle fleet management, using Eclipse Kuksa to simulate a realistic fleet of 20 vehicles. Live GPS positions flow from per-vehicle Kuksa Databrokers → MQTT → Rust backend → browser map.

---

## Quickstart

```sh
git clone git@github.com:lunatech-labs/sdv-fleet-management.git
cd sdv-fleet-management

docker compose up
```

Then visit: `http://localhost:8080`.

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

# Swagger UI
open http://localhost:3000/docs

# WebSocket live stream (requires websocat: brew install websocat)
websocat ws://localhost:3000/ws/fleet
# {"vin":"VIN-0003","lat":48.8641,"lon":2.3318}
# {"vin":"VIN-0011","lat":48.8462,"lon":2.3204}
```

### Frontend

```sh
docker compose up --build
open http://localhost:8080
```

- 20 vehicle pins visible on the Paris map
- Pins move in real time (~1 Hz)
- Clicking a pin opens the drawer with VIN, brand, model, and software version

For local development without Docker:
```sh
cd frontend && npm install
VITE_BACKEND_URL=http://localhost:3000 npm run dev
```

---

## Port Reference

| Service | Host port |
|---|---|
| Mosquitto MQTT | 1883 |
| Kuksa Databroker VIN-0001–0020 | 55556–55575 |
| Rust backend | 3000 |
| Frontend | 8080 |
