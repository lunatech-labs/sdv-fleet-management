# hawkBit DDI over uProtocol

A sketch of the second iteration of the OTA transport, for
[issue #69](https://github.com/eclipse-sdv-blueprints/fleet-management/issues/69).

Its purpose is one check. `sophokles73` offered to add the cloud-to-vehicle direction to the Hono
uTransport. This document describes what a DDI bridge would need from that transport, so the two
can be compared before anyone writes code.

## Where iteration one stands

The agent polls the hawkBit DDI API over plain HTTP. Only the status reporting runs over uProtocol.

The agent makes four calls. Every call carries `Authorization: GatewayToken <token>` against tenant
`DEFAULT`.

| Purpose | Call |
|---|---|
| Poll for work | `GET /DEFAULT/controller/v1/{vin}` |
| Read the deployment | `GET /DEFAULT/controller/v1/{vin}/deploymentBase/{actionId}` |
| Report progress | `POST /DEFAULT/controller/v1/{vin}/deploymentBase/{actionId}/feedback` |
| Acknowledge a cancel | `POST /DEFAULT/controller/v1/{vin}/cancelAction/{actionId}/feedback` |

The poll returns `_links.deploymentBase` or `_links.cancelAction`. hawkBit appends a cache-busting
`?c=<n>` query, and the action id is the last path segment.

Status already travels as a uProtocol Notification, from `up://<VIN>/D102/1/8001` to
`up://fms-ota-orchestrator/D103/1/0`. The payload is `ota.v1.OtaStatus`.

## Why the poll is still HTTP

A bridge needs a cloud-to-vehicle path. Neither Hono transport provides one today.

- `up-transport-hono-mqtt` implements `send` only. It registers no listener, so it cannot receive.
- `up-transport-hono-kafka::send` returns `UNIMPLEMENTED`.

A bridge built now would work on Zenoh and fail on Hono. Keeping the poll on HTTP keeps both
transports working.

## The shape of iteration two

Replace the poll with two message flows. Keep hawkBit as the authority. The bridge holds no state.

### Cloud to vehicle: the deployment instruction

A new component sits in front of hawkBit DDI. It watches for work and sends a Notification to one
vehicle.

- Source: `up://fms-ota-bridge/D104/1/8002`
- Sink: `up://<VIN>/D102/1/0`
- Payload: a new `ota.v1.DeploymentInstruction`

The instruction carries what the agent reads from the poll and the deployment base today:

| Field | From |
|---|---|
| `action_id` | the trailing segment of `_links.deploymentBase` |
| `version` | `deployment.chunks[0].version` |
| `download_type`, `update_type` | `deployment.download`, `deployment.update` |
| `cancel` | set when hawkBit offers `_links.cancelAction` instead |

A Notification fits better than a Publish. A deployment addresses one vehicle, and uProtocol
Notifications carry a single sink.

### Vehicle to cloud: the feedback

The agent already sends `ota.v1.OtaStatus` on every transition. The bridge subscribes to it and
translates each message into the DDI feedback call the agent makes today.

The mapping is direct, because `UpdateState` was defined to mirror the DDI vocabulary:

| `UpdateState` | DDI `execution` | DDI `result.finished` |
|---|---|---|
| `PENDING` | none, this state is local | |
| `DOWNLOADING` | `download` | `none` |
| `INSTALLING` | `downloaded` | `none` |
| `COMPLETE` | `closed` | `success` |
| `FAILED` | `closed` | `failure` |

No new vehicle-to-cloud message is needed. The existing contract already carries `vin`,
`action_id`, `state`, `version` and `error`.

## What this needs from the transport

- A working `send` in the cloud-to-vehicle direction, with a sink of `resource_id == 0`.
- A `register_listener` on the in-vehicle side that accepts a sink filter.

Zenoh provides both today. Hono provides neither.

## Open points

1. **Retries.** DDI polling is naturally idempotent, because the agent re-reads the same
   deployment base. A pushed instruction is not. The bridge must re-send until the first status
   arrives, or the agent must request the current deployment on startup.
2. **Artifact download.** This demo simulates the download, so no bytes cross the link. A real
   deployment fetches an artifact over HTTP from hawkBit. The instruction would carry that URL, and
   the transport would stay a control channel.
3. **Bridge placement.** The bridge is a backend component. It needs the DDI gateway token, which
   today lives only in the vehicle.
