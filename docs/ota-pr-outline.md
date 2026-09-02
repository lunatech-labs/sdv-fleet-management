# Upstream contribution outline

The pull request sequence for
[issue #69](https://github.com/eclipse-sdv-blueprints/fleet-management/issues/69), smallest first.

Every pull request stands alone and is reviewable on its own. Only one depends on the multi-vehicle
work. All paths below are in `eclipse-sdv-blueprints/fleet-management`.

## Sequence

| PR | Scope | Depends on |
|---|---|---|
| 1 | GNSS rows in the recording | nothing |
| 2 | rFMS identity and driver wiring | nothing |
| 3 | The OTA contract and the VSS overlay | nothing |
| 4 | The in-vehicle OTA agent | PR 3 |
| 5 | The optional hawkBit overlay | PR 4, and the multi-vehicle work |
| 6 | npm support in the license script | nothing |
| 7 | The fleet operations backend | PR 2, 3 |
| 8 | The operator dashboard | PR 6, 7 |

PRs 1, 2, 3 and 6 share no files. They can be open at the same time.

## PR 1: add GNSS rows to the FMS recording

`csv-provider/signalsFmsRecording.csv` holds 15 signals and none is a location, so `gnssPosition`
is `null` in every rFMS response. The rest of the chain is complete and wired.

Add the five `Vehicle.CurrentLocation.*` rows. The timestamp must be RFC3339, because the forwarder
parses it with `chrono::DateTime::parse_from_rfc3339`. Without it the reader returns no position at
all, whatever else is present.

We have a generator that produces per-vehicle tracks with distinct routes. It uses the Python
standard library only. It may be the natural mechanism for the per-vehicle files in the
multi-vehicle branch.

**Coordination.** This is the one file that collides with that branch.

## PR 2: populate vehicle identity and driver fields in the rFMS API

Close the server wiring gaps only. No protobuf change and no VSS change.

- `fms-server/src/influx_reader.rs`: populate `driver1_id`. The constants already exist and the
  writer already fills them. Only the reader drops them.
- Add a vehicle registry file, mounted into `fms-server`, for brand, model and type.

`/rfms/vehicles` returns the VIN and nothing else today, so an operator view cannot label a marker.
Routing brand and model through telemetry would touch four layers. A registry matches how rFMS
models the endpoint, and it makes the vehicle list independent of whether telemetry flows.

This is a design opinion. It is the point we would most like a view on before writing code.

Consider folding in issue #33 here. A polling dashboard hits `latestOnly` on every request.

## PR 3: add the OTA status contract and the software version overlay

The wire contract and the VSS signal, with no runtime code. This lets the addressing and the enum
be argued once, in isolation.

- A new `components/fms-ota-proto` crate holding `ota/v1/ota.proto`.
- `spec/overlay/ota.vspec` adding `Vehicle.SoftwareVersion`, which standard VSS does not define.

Put the contract in a new crate rather than in `fms-proto`. `fms-proto` is the rFMS wire contract.
Folding an unrelated domain into it would make the optional OTA overlay a mandatory dependency of
`fms-forwarder` and `fms-consumer`. Register it as a path dependency, not a workspace member, which
mirrors how `fms-proto` is wired today.

Addressing, which we would like confirmed: service id `D102` for the agent, `D103` for the backend
orchestrator, resource `0x8001` for the status notification. Upstream uses `D100` and `D101`. Issue
#65 informally uses `D200`.

## PR 4: add the in-vehicle OTA agent

A self-contained crate. Nothing runs yet, so it is reviewable on its own.

The agent polls the hawkBit DDI API over HTTP and sends a uProtocol Notification on every
transition. It reads its own VIN from the vehicle Databroker, so every agent shares one
configuration block and only the Databroker address differs. Simulated failures default to off.

It uses `kuksa-rust-sdk`, the client `fms-forwarder` already uses, so it adds no second gRPC stack.

**One cost to flag.** `kuksa-rust-sdk` build-depends on `protobuf-src`, which compiles protobuf
from source and needs a C++ toolchain. The `rust-musl-cross` builder already ships one, so the
existing Dockerfiles are unaffected.

## PR 5: add an optional hawkBit overlay

A new `fms-blueprint-compose-hawkbit.yaml` with postgres, hawkBit and one agent per vehicle. No
backend and no dashboard yet. At this point the flow is complete and observable through the hawkBit
UI and the existing Grafana dashboard.

Compose merges top-level keys across `-f` files, so the overlay adds its own services and volumes
and touches no existing file. Add `iot.hawkbit` to `.sdv-blueprint.json`.

The overlay is Zenoh-only by construction, because neither Hono transport carries cloud-to-vehicle
traffic. See [ddi-over-uprotocol.md](ddi-over-uprotocol.md).

**This is the only pull request that touches the multi-vehicle work.** The coupling is weak,
because the agent reads its VIN from the Databroker. If that branch has not landed, the overlay
ships one agent against the single `databroker` container, and a later pull request adds the other
two as pure additions.

## PR 6: vet npm dependencies with the Eclipse Dash tool

`.github/scripts/check-3rd-party-licenses.sh` reads `cargo tree` only. PR #66 already extended it
once, for Java. This adds a producer that reads `package-lock.json` and emits npm coordinates.

Both producers exclude development dependencies, because only distributed code needs a review.

We ran it. Across 429 distributed dependencies, 424 are approved and 5 are restricted. **No npm
dependency is restricted.** The five are Rust crates with permissive licenses that need a ticket
rather than a code change.

## PR 7: add the fleet operations backend

Campaign orchestration through the hawkBit Management API, the notification listener, WebSocket
fan-out, and fleet state read from the rFMS API rather than from InfluxDB.

Split into a read model and the campaign half if the diff grows past roughly 1500 lines.

**Software version does not come from rFMS.** rFMS 4.0 has no such field. It comes from the OTA
domain: the notification payload, and `installedDistributionSet` from hawkBit.

## PR 8: add the fleet map and campaign dashboard

The Vue 3 frontend at a top-level directory, not under `components/`, because that is the Cargo
workspace root and a `node_modules` tree inside it would confuse `cargo tree`.

Scoped to what Grafana does not do well: the live fleet map and the campaign panel. Grafana keeps
the telemetry dashboards.

## Not in this sequence

The end-to-end suite. Upstream has no e2e infrastructure, and image builds are disabled for pull
requests, so there is no image to test against. Worth discussing separately.
