# Fleet Orchestrator — Concepts & References

A per-component learning guide for everyone who wants to understand what this project is built on and why.

---

## 1. Infrastructure (`docker-compose.yml`)

### YAML anchors and extension fields
Docker Compose supports standard YAML anchors (`&name`) and merge keys (`<<: *name`). The `x-` prefix marks a block as a Compose extension field, which Compose ignores at runtime — it exists only to be merged into real service definitions. This lets you DRY up repeated configuration (image, depends_on, restart) across many services.

- [YAML specification — anchors and aliases](https://yaml.org/spec/1.2-old/spec.html#id2765878)
- [Docker Compose — extension fields](https://docs.docker.com/compose/how-tos/extension-fields/)

### Healthchecks and startup ordering
`depends_on` supports two conditions beyond the default `service_started`:
- `service_healthy` — waits for the target's `healthcheck` to pass. Used for stateful services like databases or brokers.
- `service_completed_successfully` — waits for a one-shot container to exit 0. Used for init/seed jobs.

Without these conditions, Docker Compose starts services concurrently and race conditions cause spurious failures.

- [Docker Compose — `depends_on`](https://docs.docker.com/compose/how-tos/startup-order/)
- [Docker — HEALTHCHECK](https://docs.docker.com/reference/dockerfile/#healthcheck)

### Single build, shared image
When multiple services share the same build context, building them in parallel causes a race condition on the file system (multiple tars of the same directory at once). The fix: give only one service the `build:` directive and set a shared `image:` tag on all of them. Docker builds once; every container reuses the cached image layer.

---

## 2. Eclipse Mosquitto (MQTT broker)

### MQTT protocol
MQTT is a lightweight publish/subscribe messaging protocol designed for constrained devices. A central **broker** routes messages between publishers and subscribers. Clients never communicate directly.

- [MQTT specification (v3.1.1)](https://mqtt.org/mqtt-specification/)
- [Eclipse Mosquitto documentation](https://mosquitto.org/documentation/)

### Topic wildcards
MQTT topics are `/`-separated path strings. Two wildcards exist:
- `+` — matches exactly one level. `kuksa/+/telemetry` matches `kuksa/VIN-0001/telemetry` but not `kuksa/a/b/telemetry`.
- `#` — matches zero or more trailing levels. Must be the last character. `kuksa/+/telemetry/#` matches all signals for any VIN.

### QoS levels
| Level | Name | Guarantee |
|---|---|---|
| 0 | At most once | Fire and forget — possible loss |
| 1 | At least once | Guaranteed delivery, possible duplicates |
| 2 | Exactly once | Guaranteed, no duplicates — highest overhead |

For telemetry at 1 Hz, QoS 1 is the right trade-off: guaranteed delivery without the two-phase handshake of QoS 2.

---

## 3. Eclipse Kuksa Databroker

### What is Kuksa?
Eclipse Kuksa is an open-source vehicle abstraction layer. The **Databroker** is a gRPC server that stores and serves VSS signals. It acts as a single source of truth for vehicle state.

- [Eclipse Kuksa project](https://eclipse-kuksa.github.io/kuksa.val/)
- [Kuksa Databroker API reference](https://github.com/eclipse-kuksa/kuksa-databroker/tree/main/proto/kuksa/val/v1)

### Vehicle Signal Specification (VSS)
VSS is a COVESA standard that defines a hierarchical tree of named signals for vehicle data — `Vehicle.CurrentLocation.Latitude`, `Vehicle.VehicleIdentification.VIN`, etc. Each signal has a defined type, unit, and access mode (sensor, actuator, attribute).

- [COVESA VSS specification](https://covesa.github.io/vehicle_signal_specification/)
- [VSS signal catalog on GitHub](https://github.com/COVESA/vehicle_signal_specification)

### gRPC and Protocol Buffers
The Databroker exposes a gRPC API defined in `.proto` files. gRPC uses Protocol Buffers (protobuf) as its serialisation format — a strongly-typed, binary, schema-first alternative to JSON. In Rust, `tonic` is the gRPC framework and `prost` handles protobuf encoding/decoding.

- [gRPC concepts](https://grpc.io/docs/what-is-grpc/core-concepts/)
- [Protocol Buffers language guide](https://protobuf.dev/programming-guides/proto3/)
- [tonic — Rust gRPC](https://github.com/hyperium/tonic)
- [prost — Rust protobuf](https://github.com/tokio-rs/prost)

---

## 4. Seed script (`seed/`)

### kuksa-client (Python)
The official Python library for interacting with Kuksa Databroker. `VSSClient` wraps the gRPC connection and exposes high-level methods like `set_current_values`, which takes a dict of VSS path → `Datapoint`.

- [kuksa-client on PyPI](https://pypi.org/project/kuksa-client/)
- [kuksa-client API docs](https://github.com/eclipse-kuksa/kuksa-python-sdk)

### One-shot containers
A seed container runs once, does its work, and exits 0. In Compose, set `restart: "no"` so it is not restarted after success, and use `condition: service_completed_successfully` in downstream services' `depends_on`. This is the canonical pattern for database migrations and data seeding.

---

## 5. kuksa2mqtt sidecar (`kuksa2mqtt/`)

### Tokio async runtime
Tokio is Rust's most widely used async runtime. It provides `async/await` scheduling, I/O primitives, timers, and task spawning. `#[tokio::main]` wraps the entry point in a multi-threaded executor.

- [Tokio tutorial](https://tokio.rs/tokio/tutorial)
- [Tokio API docs](https://docs.rs/tokio)

### rumqttc (async MQTT client)
`rumqttc` provides an async MQTT client split into two parts: `AsyncClient` (the API you call to publish/subscribe) and `EventLoop` (the I/O driver that must be polled continuously). The two communicate over an internal channel, so the event loop must run in a separate task for publishes to actually be sent.

- [rumqttc on crates.io](https://crates.io/crates/rumqttc)
- [rumqttc examples](https://github.com/bytebeamio/rumqtt/tree/main/rumqttc/examples)

### GPS random walk
A random walk models GPS drift by adding a small Gaussian-distributed delta at each time step. Using `Normal(μ=0, σ=0.0002°)` gives a standard deviation of roughly 22 metres per step at Paris's latitude — realistic for a slow-moving vehicle.

- [rand_distr — Normal distribution](https://docs.rs/rand_distr/latest/rand_distr/struct.Normal.html)
- [Random walk — Wikipedia](https://en.wikipedia.org/wiki/Random_walk)

---

## 6. Rust Backend (`backend/`)

### axum
axum is a Tokio-native web framework built on top of `tower` and `hyper`. Routes are composable, middleware is applied as `Layer`s, and shared state is injected via the `State` extractor. Handler functions are ordinary async functions; axum derives `IntoResponse` automatically for common return types.

- [axum documentation](https://docs.rs/axum)
- [axum WebSocket example](https://github.com/tokio-rs/axum/tree/main/examples/websockets)

### tokio::sync::broadcast
A broadcast channel delivers every message to every active receiver. The sender (`Sender<T>`) clones the value for each receiver; if a receiver falls behind, it gets a `RecvError::Lagged` error rather than blocking the sender. This is the right primitive for a pub/sub fanout (one MQTT stream → N WebSocket clients).

- [tokio broadcast docs](https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html)

### tokio::select!
`select!` polls multiple async expressions concurrently and proceeds with the first one that resolves. In the WebSocket handler it is essential: without it, incoming frames (pings, close) are never read, causing the connection to stall. With it, the loop handles both outbound events and inbound frames in a single task.

- [tokio::select! docs](https://docs.rs/tokio/latest/tokio/macro.select.html)

### DashMap
`DashMap<K, V>` is a concurrent hash map with fine-grained sharding — it allows multiple readers and writers to operate simultaneously without a global lock. It implements the same API as `HashMap` but is `Send + Sync`, making it safe to share across Tokio tasks via `Arc` (or directly, since `DashMap` is already `Arc`-backed).

- [DashMap on crates.io](https://crates.io/crates/dashmap)

### utoipa (OpenAPI)
`utoipa` generates an OpenAPI 3 document from Rust types and handler annotations at compile time. `#[utoipa::path(...)]` annotates handlers; `#[derive(ToSchema)]` annotates models; `#[derive(OpenApi)]` ties them together. `utoipa-swagger-ui` serves the interactive UI.

- [utoipa docs](https://docs.rs/utoipa)
- [utoipa-swagger-ui](https://docs.rs/utoipa-swagger-ui)

### tower-http — CORS
The frontend (port 8080) calls the backend (port 3000) — a cross-origin request. Browsers block these unless the server responds with the correct `Access-Control-Allow-*` headers. `CorsLayer::permissive()` adds permissive CORS headers for all origins, suitable for a local demo.

- [tower-http CorsLayer docs](https://docs.rs/tower-http/latest/tower_http/cors/index.html)
- [MDN — CORS](https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS)

---

## 7. Vue 3 Frontend (`frontend/`)

### Vue 3 Composition API
The Composition API (`<script setup>`, `ref`, `reactive`, `onMounted`, composables) replaces the Options API for organising component logic. `reactive()` creates a deeply reactive proxy over a plain object; mutations to nested properties trigger re-renders. `ref()` wraps a primitive in a `.value` accessor that is also reactive.

- [Vue 3 Composition API docs](https://vuejs.org/guide/extras/composition-api-faq)
- [Vue 3 reactivity fundamentals](https://vuejs.org/guide/essentials/reactivity-fundamentals)

### Composables
A composable is a function that encapsulates stateful logic using Vue's reactivity APIs and lifecycle hooks. `useFleetSocket` is a composable: it opens a WebSocket, registers `onUnmounted` to clean up, and calls back into the component when events arrive — without the component knowing anything about the WebSocket lifecycle.

- [Vue 3 — composables guide](https://vuejs.org/guide/reusability/composables)

### Leaflet + @vue-leaflet/vue-leaflet
Leaflet is the standard JavaScript mapping library. `@vue-leaflet/vue-leaflet` wraps it as Vue 3 components (`<l-map>`, `<l-tile-layer>`, `<l-marker>`, etc.). Marker positions update reactively when the `:lat-lng` prop changes.

Bundlers (Vite, webpack) break Leaflet's default marker icons because the CSS references image URLs that don't resolve after bundling. The fix is to manually import the PNGs and call `L.Icon.Default.mergeOptions()` before mounting the app.

- [Leaflet documentation](https://leafletjs.com/reference.html)
- [@vue-leaflet/vue-leaflet](https://github.com/vue-leaflet/vue-leaflet)
- [OpenStreetMap tile usage policy](https://operations.osmfoundation.org/policies/tiles/)

### Vite
Vite is a build tool that serves files over native ES modules during development (instant HMR) and bundles with Rollup for production. `import.meta.env` exposes environment variables prefixed with `VITE_`. The `vite/client` type reference (in `vite-env.d.ts`) provides TypeScript types for `import.meta.env` and for static asset imports (`.png`, `.svg`, etc.).

- [Vite documentation](https://vitejs.dev/guide/)
- [Vite — env variables](https://vitejs.dev/guide/env-and-mode)

### WebSocket in the browser
The browser's native `WebSocket` API is event-driven. Unlike HTTP, the connection stays open and either side can push messages. Key events: `onopen`, `onmessage`, `onclose`, `onerror`. A reconnect loop (`setTimeout(connect, 3000)` on close) makes the client resilient to backend restarts.

- [MDN — WebSocket API](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket)
