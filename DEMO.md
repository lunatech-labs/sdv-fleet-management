# Demo Guide

A step-by-step reference for running the SDV Fleet Management demo live.

---

## Before the demo

Run these steps at least a few minutes before presenting.

**1. Copy the env file if you haven't already:**
```sh
cp .env.example .env
```

**2. Pre-pull images to avoid slow downloads during startup:**
```sh
docker compose pull
```

**3. Start the full stack:**
```sh
docker compose up --build
```

**4. Wait for HawkBit to be ready** (60 to 90 seconds on first boot). Watch for this log line:
```
hawkbit  | Started HawkbitServerApp
```

**5. Verify the stack is up:**
```sh
curl http://localhost:3000/health        # should return 200
open http://localhost:8080               # frontend
open http://localhost:3000/docs          # Swagger UI
```

Keep both browser tabs open before starting the walkthrough.

---

## Full walkthrough recording

A complete run-through of all features: live map, vehicle drawer, fleet table, OTA campaign launch, and real-time state updates. The file is stored at [`docs/screenshots/demo.mp4`](docs/screenshots/demo.mp4).

<video src="docs/screenshots/demo.mp4" controls width="100%"></video>

---

## Demo walkthrough

### 1. Live fleet map

Open `http://localhost:8080`.

20 vehicle pins are spread across Paris, each moving independently at 1 Hz. Green pins are vehicles with up-to-date software; red pins are vehicles that are out of date or have a campaign in progress.

![Live fleet map](docs/screenshots/main.png)

Positions flow from 20 Kuksa Databrokers through MQTT to the Rust backend, then over a WebSocket to the browser with no polling.

---

### 2. Vehicle detail drawer

Click any pin to open the detail drawer on the left. It shows the vehicle name, VIN, manufacturer, model, current software version, live coordinates, and OTA status chip.

![Vehicle detail drawer](docs/screenshots/car_modal.png)

The drawer requires no additional request -- all data is loaded once on page load via `GET /fleet`. The status chip reflects the latest campaign state for that vehicle in real time.

---

### 3. Fleet table

Click the Fleet icon in the top-right toolbar to open the Fleet panel. All 20 vehicles are listed in a sortable table with VIN, brand, model, software version, current coordinates, and last-seen timestamp.

![Fleet table](docs/screenshots/fleet.png)

Use the search box to filter by VIN, make, model, or software version. Filtering also updates which pins are visible on the map -- both views stay in sync.

---

### 4. Launching an OTA campaign

Click the Campaign icon in the toolbar to open the Campaigns panel.

1. Select a target software version from the dropdown (populated by `GET /versions`).
2. Check the vehicles to include, or leave all 20 selected.
3. Click **Launch**.

![Campaign setup](docs/screenshots/campaigns.png)

The new campaign card appears immediately below the launcher. Each vehicle progresses through its own state machine:

```
Pending  ->  Downloading  ->  Installing  ->  Complete
                                          ->  Failed
```

A 20% simulated failure rate is intentional -- it demonstrates realistic error handling rather than a guaranteed happy path. State chips on the campaign card update live via WebSocket.

---

### 5. Campaign in progress

Once a campaign is running you can watch state chips update in real time. Vehicles that already have an active download show a conflict state (shown in red at the top of the list below).

![Campaign deployment in progress](docs/screenshots/campaign_deployment.png)

Scroll through the campaign history to show completed rollouts alongside the new one.

Once a vehicle reaches **Complete**, click its map pin -- the software version in the drawer reflects the newly installed version, read from the campaign state.

---

### 6. Fleet and Campaigns side by side

Both panels can be open simultaneously. This is useful for showing how the fleet table and campaign state stay in sync -- a vehicle that completes its update also updates its software version in the fleet table.

![Fleet and Campaigns panels open together](docs/screenshots/campaigns+fleet.png)

---

### 7. API explorer

Switch to `http://localhost:3000/docs` (Swagger UI). All REST endpoints are documented and executable from the browser. This shows the clean API surface and the OpenAPI spec generated directly from the Rust source.

---

## Optional: live pipeline (for a technical audience)

Open a terminal alongside the browser to show raw data flowing through the stack.

**MQTT telemetry from a single vehicle:**
```sh
mqtt sub -h localhost -p 1883 --topic='kuksa/VIN-0001/telemetry/#'
```

**Backend REST:**
```sh
curl -s http://localhost:3000/fleet | jq '.[0]'
```

**WebSocket position stream:**
```sh
websocat ws://localhost:3000/ws/fleet
```

**WebSocket OTA state stream (launch a campaign first):**
```sh
websocat ws://localhost:3000/ws/campaigns
```

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| Campaign panel shows an error | HawkBit is still initialising -- wait 60 to 90 seconds and retry |
| No pins on the map | Backend is not yet ready -- check `curl http://localhost:3000/health` |
| Pins are not moving | Sidecars may not have started -- run `docker compose logs -f kuksa2mqtt-01` |
| Port conflict | Check nothing else is on ports 8080, 3000, 1883, or 8083 |
