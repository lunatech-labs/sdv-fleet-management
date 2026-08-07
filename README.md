# SDV Fleet Management — v2 Demo

A demo showcasing Rust as a high-performance backend for vehicle fleet management, built on the [Eclipse SDV Fleet Management blueprint](https://github.com/eclipse-sdv-blueprints/fleet-management). Telemetry flows through the blueprint's uProtocol pipeline: a Kuksa CSV Provider replays a recording into a per-vehicle Databroker, an FMS Forwarder publishes `VehicleStatus` over uProtocol, and the FMS Consumer writes it to InfluxDB, which this project's backend reads to drive a live browser map.

On top of that, this project adds over-the-air (OTA) software update campaigns powered by Eclipse HawkBit: create a rollout from the UI, watch vehicle markers change colour as updates progress, and track per-vehicle state in real time over WebSocket.

For a guided presentation walkthrough with screenshots and a screen recording, see [DEMO.md](DEMO.md).

---

## Quickstart

```sh
git clone git@github.com:lunatech-labs/sdv-fleet-management.git
cd sdv-fleet-management

# Copy .env.example to .env — set HAWKBIT_GATEWAY_TOKEN, HAWKBIT_USER, HAWKBIT_PASSWORD.
cp .env.example .env

docker compose up
```

Then visit `http://localhost:8080`.

HawkBit takes 60 to 120 seconds to become healthy on a cold start, and the backend waits for it. For first-run timing notes and a pre-demo checklist, see [DEMO.md](DEMO.md).

---

## Architecture

```
Browser (Vue 3 · port 8080)
    │  REST GET /fleet, /campaigns, /versions
    │  WebSocket /ws/fleet, /ws/campaigns
    ▼
Rust Backend (axum · port 3000)
    │  Flux query (positions)          │  REST Management API (rollouts)
    ▼                                  ▼
InfluxDB (port 8086)              Eclipse HawkBit (port 8083)
    ▲                                  ▲  DDI poll loop
    │  writes VehicleStatus            │
FMS Consumer                       OTA agents (×3, Rust)
    ▲                                  │  gRPC Set Vehicle.SoftwareVersion
    │  uProtocol Publish                │
    │  up://<VIN>/D100/1/D100          │
Zenoh router (port 7447)               │
    ▲                                  │
    │                                  │
┌───┴──────────────────────────────────┴──────────────┐
│  3 vehicles                                         │
│  CSV Provider → Kuksa Databroker (ports 55556–55558)│
│                 → FMS Forwarder                     │
└─────────────────────────────────────────────────────┘
```

Telemetry uses the blueprint's components unmodified (`fms-forwarder`, `fms-consumer`, pulled from GHCR). The OTA agents and the operator backend/UI are this project's own.

### Why the backend reads InfluxDB

The FMS Consumer already subscribes to the uProtocol vehicle status topic and writes everything to InfluxDB. Rather than adding a second subscriber to the same topic, the backend polls InfluxDB for the latest position per VIN. That keeps this project out of the uProtocol data path and avoids duplicating a component the blueprint maintains. See `backend/src/influx.rs`.

### Per-vehicle recordings

The blueprint ships two recordings and neither works alone for a multi-vehicle fleet: `signalsFmsRecording.csv` has the full FMS signal set but one fixed VIN and no position, and `signalsCovesaCvRecording.csv` has a GNSS track but no VIN. `csv-provider/generate_vehicle_recordings.py` merges them, giving each vehicle its own VIN and its own position track:

```sh
python3 csv-provider/generate_vehicle_recordings.py --vehicles 3
```

Outputs `csv-provider/vehicles/<VIN>.csv`, which docker-compose mounts into each CSV Provider. Re-run it after changing `seed/vehicles.json`.

### OTA state over uProtocol

The in-vehicle agent reports every transition (Pending, Downloading, Installing, Complete, Failed) to the back end as a uProtocol Notification, addressed from `up://<VIN>/D102/1/0` to `up://fms-ota-orchestrator/D103/1/0`. The contract is `proto/ota/v1/ota.proto`, shared by both crates.

The agent still drives HawkBit's DDI API over HTTP; only the reporting path runs over uProtocol. The HawkBit Management API poll remains as reconciliation. To confirm the notification path alone drives a rollout:

```sh
docker compose run --rm -e HAWKBIT_RECONCILE_ENABLED=false -p 3001:3000 backend
```

### Gateway token

The in-vehicle OTA agents authenticate to HawkBit's DDI API with a gateway token. It is deployment configuration (`HAWKBIT_GATEWAY_TOKEN`), shared by the agents and the backend, which provisions that exact value on HawkBit's DEFAULT tenant at startup. Both sides can therefore start in any order; agents retry until the backend has provisioned it.

---

## Docker Compose features

### Healthchecks and startup ordering

`influxdb`, `postgres`, and `hawkbit` expose healthchecks, and downstream services use `depends_on` with `condition: service_healthy` to enforce the startup sequence. HawkBit 1.1.0 does not expose the Spring actuator, so its readiness is probed against the Management API instead — an unauthenticated request answers `401` once the app is serving.

### YAML anchors and extension fields

The `x-databroker-defaults`, `x-csv-provider-defaults`, `x-forwarder-defaults`, and `x-ota-agent-defaults` blocks use Docker Compose's extension field convention (`x-` prefix). Each defines shared configuration once, and services merge it in with the YAML merge key (`<<: *ota-agent-defaults`). Growing the fleet means copying a handful of lines per vehicle rather than a full service definition.

### BuildKit deduplication

All OTA agent services share the same build (context `.`, dockerfile `ota-agent/Dockerfile`) and `image: ota-agent:local`. The context is the repository root so `proto/` is shared with the backend; `.dockerignore` keeps it small. BuildKit builds the image once even though every agent declares it. The `image:` tag is also what stops Docker trying to pull `ota-agent:local` from Docker Hub on first run.

```sh
docker compose build ota-agent-01
```

### Architecture note

`fms-consumer` is published upstream for `linux/amd64` only (`fms-forwarder` is the only multi-arch image), so it is pinned with `platform: linux/amd64` and runs under emulation on arm64 hosts.

---

## Testing

### Telemetry pipeline

```sh
docker compose up -d
```

```sh
# Databroker ports are listening
for port in 55556 55557 55558; do
  echo -n "port $port: " && nc -z localhost $port && echo "OK" || echo "FAIL"
done

# A databroker responds to gRPC (requires grpcurl: brew install grpcurl)
grpcurl -plaintext localhost:55556 list

# The CSV Provider is writing the VIN
grpcurl -plaintext \
  -d '{"entries": [{"path": "Vehicle.VehicleIdentification.VIN", "fields": ["FIELD_VALUE"]}]}' \
  localhost:55556 kuksa.val.v1.VAL/Get

# The FMS Consumer is writing to InfluxDB
docker compose logs fms-consumer | tail
```

Query the positions the backend reads, straight from InfluxDB:

```sh
TOKEN=$(docker compose exec -T influxdb cat /tmp/out/fms-demo.token | tr -d '\r\n')
curl -s -XPOST "http://127.0.0.1:8086/api/v2/query?org=sdv" \
  -H "Authorization: Token $TOKEN" \
  -H "Accept: application/csv" \
  -H "Content-Type: application/vnd.flux" \
  --data-binary 'from(bucket: "demo")
  |> range(start: -30s)
  |> filter(fn: (r) => r._measurement == "snapshot")
  |> filter(fn: (r) => r._field == "latitude" or r._field == "longitude")
  |> group(columns: ["vin", "_field"])
  |> last()
  |> keep(columns: ["vin", "_field", "_value"])
  |> group(columns: ["vin"])
  |> pivot(rowKey: ["vin"], columnKey: ["_field"], valueColumn: "_value")'
```

### Backend

```sh
curl http://localhost:3000/health
curl -s http://localhost:3000/fleet | jq '.[0]'
curl -s http://localhost:3000/vehicles/VIN-0001 | jq

# OTA campaigns
curl -s http://localhost:3000/versions | jq
curl -s http://localhost:3000/campaigns | jq
curl -s -X POST http://localhost:3000/campaigns \
  -H 'Content-Type: application/json' \
  -d '{"version":"2.0.0","vins":["VIN-0001","VIN-0002"]}' | jq
curl -s http://localhost:3000/campaigns/<id> | jq

# Swagger UI
open http://localhost:3000/docs

# WebSocket live streams (requires websocat: brew install websocat)
websocat ws://localhost:3000/ws/fleet
# {"vin":"VIN-0003","lat":48.8641,"lon":2.3318}

websocat ws://localhost:3000/ws/campaigns
# {"type":"transition","campaign_id":"...","vin":"VIN-0001","state":"INSTALLING"}
```

### Frontend

```sh
docker compose up --build
open http://localhost:8080
```

- Three vehicle pins visible on the Paris map
- Pins move in real time
- Clicking a pin opens the drawer with VIN, brand, model, and software version
- Campaign Panel lets you select a software version and target vehicles, then launch a rollout
- Vehicle markers change colour as OTA state progresses (Pending, Downloading, Installing, Complete, Failed)

For local development without Docker:
```sh
cd frontend && npm install
VITE_BACKEND_URL=http://localhost:3000 npm run dev
```

In a container the backend URL is not baked into the bundle: the entrypoint writes `/config.js` from `BACKEND_URL`, so one image works in any deployment. `VITE_BACKEND_URL` is only the `npm run dev` fallback.

---

## Contributing

### Backend (Rust)

```sh
cd backend
cargo fmt
cargo clippy -- -D warnings
cargo test
```

All three steps run automatically on every push to `main` via GitHub Actions (`.github/workflows/backend.yml`).

### Frontend (Vue 3 + TypeScript)

```sh
cd frontend
npm run lint
npm test
```

Both steps run automatically on every push to `main` via GitHub Actions (`.github/workflows/frontend.yml`).

### End-to-end tests (Playwright)

```sh
docker compose up -d          # full stack must be running

cd e2e
npm install
npm run install-browsers      # first time only — downloads Chromium
npm test
```

The suite reads the fleet size from the backend rather than hardcoding it, so it tracks whatever `docker-compose.yml` and `seed/vehicles.json` define.

Overrides (useful when the ports are remapped):

```sh
PLAYWRIGHT_BASE_URL=http://localhost:8080 \
PLAYWRIGHT_BACKEND_URL=http://localhost:3000 \
npm test
```

### Licensing

The project is licensed under Apache-2.0, matching the Eclipse SDV blueprint it targets. It was relicensed from EPL-2.0 for that reason; every contributor at the time of the change was a Lunatech employee.

Every source file carries an Apache-2.0 SPDX header. Files that cannot carry comments (JSON, CSV) use a REUSE-style `<filename>.license` sidecar instead. [NOTICE.md](NOTICE.md) records the content inherited from the blueprint and the third-party images the stack runs.

---

## Port Reference

| Service | Host port |
|---|---|
| Kuksa Databroker VIN-0001–0003 | 55556–55558 |
| Zenoh router | 7447 |
| InfluxDB | 8086 |
| Rust backend | 3000 |
| Eclipse HawkBit | 8083 |
| Frontend | 8080 |

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| Campaign panel shows an error | HawkBit is still initialising -- wait 60 to 120 seconds and retry |
| No pins on the map | Backend is not yet ready -- check `curl http://localhost:3000/health` |
| Pins are not moving | Check the telemetry chain: `docker compose logs fms-forwarder-01` then `docker compose logs fms-consumer` |
| All vehicles share one position | `csv-provider/vehicles/*.csv` may be stale -- re-run `generate_vehicle_recordings.py` |
| OTA agents log `401` from DDI | The backend has not provisioned the gateway token yet; agents retry automatically |
| Port conflict | Check nothing else is on ports 8080, 3000, 8086, 7447, or 8083 |
