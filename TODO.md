# TODO

## Seed

- [ ] Upgrade `kuksa-client` to 0.5.0 — requires the databroker image to also be upgraded to a version that implements the `PublishValue` RPC. The two must be bumped together. Pinned to 0.4.3 for now.
- [ ] Set up `requirements.txt` + local venv (or `uv`) so the seed script can be run and tested without rebuilding the Docker image.
- [ ] Add `pytest` suite with a mocked `VSSClient`: assert correct signals are written and `Vehicle.SoftwareVersion` is never attempted.

## Backend

- [x] Extract `build_router(state: AppState)` from `main()` to enable in-process axum testing via `tower::ServiceExt::oneshot`.
- [x] Integration tests for REST endpoints: `GET /fleet` (empty store), `GET /vehicles/:vin` (404), `GET /health` (200).
- [x] WebSocket integration tests: event forwarding, client-initiated close.
- [x] GitHub Actions CI: `cargo fmt --check`, `cargo clippy`, `cargo test` on push to `main`.
- [ ] Unit tests for `Store`: `update_position` returns `None` for unknown VIN, `last_seen` is updated on write.

## Frontend

- [ ] Add Vitest + Vue Test Utils: test `useFleetSocket` reconnect logic and message parsing, `VehicleDrawer` renders/hides correctly.
- [ ] (Optional) Fleet table view — tabular display of all 20 vehicle records alongside the map.

## E2E

- [ ] Playwright test against the full Docker stack: 20 markers visible, pin click opens drawer with correct VIN, marker position changes within 3 s.

## V2

### Infrastructure

- [ ] Add HawkBit service to `docker-compose.yml` (port 8083, H2 in-memory DB, `HAWKBIT_TOKEN` env var, healthcheck)
- [ ] Add `x-ota-agent-defaults` anchor and 20 `ota-agent` services to `docker-compose.yml`
- [ ] Add `hawkbit: { condition: service_healthy }` dependency to backend service
- [ ] Create `.env` file at repo root with `HAWKBIT_TOKEN`

### ota-agent (new component)

- [ ] Create `ota-agent/Cargo.toml` and `ota-agent/Dockerfile` (mirrors `kuksa2mqtt` BuildKit pattern)
- [ ] Implement `ota-agent/src/main.rs`: subscribe to `kuksa/{vin}/ota/command`, run state machine (`PENDING -> DOWNLOADING -> INSTALLING -> COMPLETE/FAILED`), publish transitions to `kuksa/{vin}/ota/status`, write new version to Databroker via gRPC on `COMPLETE`

### Backend

- [ ] Add `reqwest` and `uuid` crates to `backend/Cargo.toml`
- [ ] Add `backend/src/hawkbit.rs`: HawkBit Management API client (register targets, seed distribution sets, create rollout, report deployment result, poll rollout status)
- [ ] Add `backend/src/campaign.rs`: `DashMap<CampaignId, Campaign>` store with `VehicleUpdateState` enum
- [ ] Update `backend/src/mqtt.rs`: subscribe to `kuksa/+/ota/status`, publish `kuksa/{vin}/ota/command`, wire HawkBit feedback and campaign store updates
- [ ] Update `backend/src/main.rs`: HawkBit startup registration, distribution set seeding, 5s background poll task, new route registration, `HAWKBIT_URL`/`HAWKBIT_TOKEN` env vars
- [ ] Add `POST /campaigns`, `GET /campaigns`, `GET /campaigns/{id}`, `GET /versions` REST endpoints
- [ ] Add `WS /ws/campaigns` endpoint (snapshot on connect, then individual transition events)

### Frontend

- [ ] Add `frontend/src/useCampaignSocket.ts`: WebSocket composable for `/ws/campaigns`, snapshot hydration, transition merge by `(campaignId, vin)`, auto-reconnect
- [ ] Add `frontend/src/CampaignPanel.vue`: version selector, vehicle checkboxes (all pre-selected), launch button, live campaign cards with per-vehicle state chips
- [ ] Update `frontend/src/App.vue`: add Campaign tab alongside the map
- [ ] Update `frontend/src/VehicleDrawer.vue`: active update state badge, version field updates on `COMPLETE`
