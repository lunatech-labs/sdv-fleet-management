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

- [ ] Write `docker-compose.yml`
  - [ ] `mosquitto` service with config
  - [ ] 20 `databroker-{01..20}` services (ports 55556–55575)
  - [ ] `seed` service with `depends_on` + healthcheck gate
  - [ ] 20 `kuksa2mqtt-{01..20}` sidecar services
  - [ ] `backend` service
  - [ ] `frontend` service
- [ ] Write Mosquitto config (`mosquitto.conf`) — allow anonymous, listen on 1883

### Seed Script (`seed/`)

- [ ] Create `seed/vehicles.json` — 20-vehicle dataset (VIN, brand, model, software version, initial lat/lon)
- [ ] Write `seed/seed.py`
  - [ ] Connect to each of the 20 Databrokers over gRPC (`kuksa-client`)
  - [ ] Write 4 static VSS signals per vehicle: `Vehicle.VehicleIdentification.VIN`, `Brand`, `Model`, `Vehicle.SoftwareVersion`
  - [ ] Write initial `Vehicle.CurrentLocation.Latitude` and `Longitude`
  - [ ] Exit with code 0 on success
- [ ] Write `seed/Dockerfile`

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
