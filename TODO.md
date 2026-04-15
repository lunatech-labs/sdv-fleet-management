# TODO

## Seed

- [ ] Upgrade `kuksa-client` to 0.5.0 — requires the databroker image to also be upgraded to a version that implements the `PublishValue` RPC. The two must be bumped together. Pinned to 0.4.3 for now.
- [ ] Set up `requirements.txt` + local venv (or `uv`) so the seed script can be run and tested without rebuilding the Docker image.
- [ ] Add `pytest` suite with a mocked `VSSClient`: assert correct signals are written and `Vehicle.SoftwareVersion` is never attempted.

## Backend

- [ ] Extract `build_router(state: AppState)` from `main()` to enable in-process axum testing via `tower::ServiceExt::oneshot`.
- [ ] Unit tests for `Store`: `update_position` returns `None` for unknown VIN, `last_seen` is updated on write.
- [ ] Integration tests for REST endpoints: `GET /fleet` (empty store), `GET /vehicles/:vin` (404), `GET /health` (200).

## Frontend

- [ ] Add Vitest + Vue Test Utils: test `useFleetSocket` reconnect logic and message parsing, `VehicleDrawer` renders/hides correctly.
- [ ] (Optional) Fleet table view — tabular display of all 20 vehicle records alongside the map.

## E2E

- [ ] Playwright test against the full Docker stack: 20 markers visible, pin click opens drawer with correct VIN, marker position changes within 3 s.
