# rFMS coverage for the operator dashboard

This note lists what the operator dashboard reads, what the blueprint rFMS API returns today, and
what is missing. It follows the agreement in
[issue #69](https://github.com/eclipse-sdv-blueprints/fleet-management/issues/69) that the
dashboard consumes the rFMS API instead of reading InfluxDB directly.

All references point at `eclipse-sdv-blueprints/fleet-management` at commit `f2b8410`.

The gaps fall into three categories. The categories cost very different amounts of work, so the
distinction matters more than the field count.

- **(a) Server wiring.** The value reaches InfluxDB and `fms-server` does not return it. One file
  to change.
- **(b) Contract.** No field exists to carry the value. Four layers to change.
- **(c) Recording.** The contract and the code are complete and the CSV has no rows. One data file
  to change.

## What the dashboard reads

The read surface is small. `frontend/src/types.ts` defines the whole of it:

```ts
export interface VehicleRecord {
  vin: string
  brand: string
  model: string
  software_version: string
  latitude: number
  longitude: number
  last_seen: string
}

export interface PositionEvent {
  vin: string
  lat: number
  lon: number
}
```

Everything else in the dashboard is OTA campaign state. That is not rFMS data, and it arrives over
uProtocol Notifications and the hawkBit Management API.

| Dashboard field | Source today | Category |
|---|---|---|
| `vin` | `/rfms/vehicles`, works | none |
| `latitude`, `longitude` | `/rfms/vehiclepositions`, always null | (c) |
| `brand`, `model` | not available | (a) and (b) |
| `software_version` | not available from rFMS by design | see below |
| `last_seen` | `createdDateTime`, works | none |

## (a) Server wiring gaps

The data reaches InfluxDB. `fms-server` does not return it.

| Field | Where the value is lost |
|---|---|
| Every `/rfms/vehicles` field except `vin` | `influx_reader.rs:103` builds `VehicleObject::new(vin)`. `models/vehicle.rs:162` sets the other 19 fields to `None`. |
| The vehicle list itself | `get_vehicles` runs `schema.tagValues(tag: "vin")`. A vehicle that never sent telemetry is absent from the API. |
| `driver1Id` | `influx_reader.rs:382` returns `None`. The constants `FIELD_DRIVER1_ID` and `FIELD_DRIVER1_CARD_ISSUER` exist at `influx-client/src/lib.rs:32-33`, and `fms.proto` carries `DriverId driver1_id`. The writer populates them. Only the reader drops them. |
| `latestOnly` on `/rfms/vehiclepositions` | Issue #33. The parameter returns the latest value for each trigger instead of each vehicle. A polling dashboard hits this on every request. |
| Battery, EV, `accumulatedData`, `uptimeData`, `estimatedDistanceToEmpty` | `influx_reader.rs:354-395` returns `None` for all of them. The dashboard does not need them. Listed for completeness, and blocked by (c) as well. |

### Two bugs we found while testing

We built a recording with the full position signal set and ran it through the stack. Two defects
show up, and both affect every deployment, not only ours. We are happy to open issues and pull
requests for either.

**1. `positionDateTime` is wrong by a factor of 1000.** `influx-client/src/writer.rs:155` writes
epoch **seconds**:

```rust
builder = builder.field(crate::FIELD_POSITION_DATE_TIME, instant.seconds);
```

`fms-server/src/influx_reader.rs:73-76` reads them as **milliseconds**:

```rust
fn unpack_time(value: Option<&String>) -> Option<DateTime<Utc>> {
    let timestamp = unpack_value_i64(value)?;
    DateTime::from_timestamp_millis(timestamp)
}
```

Observed: InfluxDB holds `1767225609`, which is 2026-01-01T00:00:09Z. The API returns
`"positionDateTime": "1970-01-21T10:53:45.606Z"`. The fix is one function, but it also affects
`createdDateTime` parsing, so it needs a check of every `unpack_time` caller.

**2. `heading` and `altitude` are always dropped.** Both are `double` in VSS, and
`fms-forwarder/src/vehicle_abstraction/kuksa.rs:97` and `:105` convert them to integers:

```rust
.altitude = i32::try_from(value).ok();   // Vehicle.CurrentLocation.Altitude is a double
.heading  = u32::try_from(value).ok();   // Vehicle.CurrentLocation.Heading is a double
```

Latitude and longitude use `f64::try_from` at `:81` and `:89` and work correctly. Observed:
the CSV Provider logs `Update current value of Vehicle.CurrentLocation.Heading to 256`, and no
`heading` field ever reaches InfluxDB, while `latitude` and `longitude` do. The conversion returns
`None` for every value.

This matters to us because the map orients its markers by heading. It matters more generally
because `GnssPosition.heading` and `.altitude` are part of the rFMS contract and are silently
always absent.

`/rfms/vehicles` is the largest hole for an operator view. A map cannot label a marker with a bare
VIN. Note that the endpoint derives its list from InfluxDB tag values, so the vehicle list and the
telemetry history are the same thing today.

## (b) Contract gaps

No field exists to carry the value.

### `software_version` is not an rFMS gap

rFMS 4.0 has no software version field. `VehicleObject` has 20 fields and none of them is a
version. `fms.proto` has none, `influx-client` has no constant, and standard VSS has none.
`Vehicle.VersionVSS` is the version of the VSS model, not of the vehicle software.

Please do not add one. The dashboard takes the installed version from the OTA domain instead,
where it belongs: `OtaStatus.version` in the uProtocol Notification, and `installedDistributionSet`
from the hawkBit Management API. Our VSS overlay adds `Vehicle.SoftwareVersion` for in-vehicle
observability, which is what makes an install verifiable at the Databroker.

### `brand` and `model` through telemetry

VSS defines `Vehicle.VehicleIdentification.Brand` and `.Model`. Carrying them to the API needs four
changes: a field on `VehicleStatus` in `fms.proto`, a constant and a writer branch in
`influx-client`, a subscription in `fms-forwarder/src/vehicle_abstraction/vss.rs`, and rows in the
recording.

We suggest a static registry file instead, mounted into `fms-server`. Two reasons. First, rFMS
models `/rfms/vehicles` as an operator-side registry rather than as telemetry, so a registry
matches the specification. Second, it makes the vehicle list independent of whether telemetry
flows, which fixes the "vehicle absent until it reports" behaviour in the same change.

This is a design opinion, and it is the one point in this note where we would most like your view
before anyone writes code.

## (c) Recording gaps

The contract and the code are complete. `csv-provider/signalsFmsRecording.csv` has no rows.

### Position is the one that matters

The chain is fully wired:

- `fms-forwarder/src/vehicle_abstraction/vss.rs:58-62` declares the five signals.
- `vehicle_abstraction.rs:41-46` subscribes to them.
- `vehicle_abstraction/kuksa.rs:115` parses the timestamp as RFC3339.
- `influx-client/src/writer.rs:140-156` writes the fields.
- `fms-server/src/influx_reader.rs:176-197` builds `GnssPositionObject`.

The recording carries 15 signals and none of them is a location, so `gnssPosition` is `null` in
every response. Fixing `fms-server` does not fix this.

**One detail is easy to miss.** `influx_reader.rs:176` returns a position only when
`positionDateTime`, `longitude` and `latitude` are all present:

```rust
match (
    unpack_time(entry.get(influx_client::FIELD_POSITION_DATE_TIME)),
    unpack_value_f64(entry.get(influx_client::FIELD_LONGITUDE)),
    unpack_value_f64(entry.get(influx_client::FIELD_LATITUDE)),
) {
```

So a recording that carries latitude and longitude but no `Vehicle.CurrentLocation.Timestamp` still
produces `null`. The timestamp must be an RFC3339 string, because `kuksa.rs:118` parses it with
`chrono::DateTime::parse_from_rfc3339`. `heading`, `altitude` and `speed` are optional and degrade
cleanly.

We hit this ourselves. Our first per-vehicle recordings had latitude and longitude only, and the
map stayed empty.

### The remaining recording gaps

| Signal | Effect |
|---|---|
| `Vehicle.CurrentLocation.Speed` | Declared at `vss.rs:28`, no rows. Feeds `gnssPosition.speed`. |
| Driver identification | The recording has `Driver1.IsCardPresent` and `Driver1.WorkingState` but no card identity. Fixing the reader gap in (a) alone will not fully populate `driver1Id`. |
| Battery, EV, door and tell-tale signals | No rows. Consistent with a diesel truck recording. Out of scope for the dashboard. |

We have a generator that produces per-vehicle recordings with distinct routes, at
`csv-provider/generate_vehicle_recordings.py` in our repository. It uses the Python standard
library only. We are happy to contribute it, and it may be the natural mechanism for the per-vehicle
CSV files in your multi-vehicle branch.

## Summary

| Need | Category | Effort |
|---|---|---|
| Position on the map | (c) | One data file, plus a generator |
| Vehicle identity for map labels | (a) plus a design decision | One reader change, plus a registry file |
| `driver1Id` | (a) | One reader change, plus (c) for the card identity |
| `latestOnly` correctness | (a) | Issue #33 |
| `software_version` | none | Not an rFMS concern. Sourced from OTA. |

The position gap blocks the map. The identity gap blocks map labels. Neither needs a protobuf or
VSS change if the registry approach is acceptable.

## Two coordination asks

1. **Please push the three-vehicle branch as a draft**, even if it is incomplete. Our compose
   overlay and your branch both touch the vehicle services, and the per-vehicle recordings overlap
   directly with the position fix above. We would rather build on top of your branch than merge two
   independent rewrites of the same files later.
2. **Please confirm the OTA uProtocol addresses.** We use service id `D102` for the in-vehicle
   agent, `D103` for the backend orchestrator, and resource id `0x8001` for the status
   notification. Issue #65 is a second uProtocol effort, and two efforts picking service ids
   independently is how they end up colliding.
