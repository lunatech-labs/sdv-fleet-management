#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Contributors to the Eclipse Foundation
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0
"""Build one CSV Provider recording per vehicle.

The blueprint ships two recordings and neither is sufficient on its own:
``signalsFmsRecording.csv`` carries the full FMS signal set but a single fixed
VIN and no position, while ``signalsCovesaCvRecording.csv`` carries a GNSS track
but no VIN. This script merges them, giving each vehicle its own VIN and its own
position track so that a multi-vehicle fleet shows up as distinct vehicles.
Each position fix carries latitude, longitude, heading and an RFC3339
timestamp, which is the set fms-server needs to report a gnssPosition.

Run it from the repository root:

    python3 csv-provider/generate_vehicle_recordings.py

Outputs ``csv-provider/vehicles/<VIN>.csv``, which docker-compose bind-mounts
into that vehicle's CSV Provider as ``/dist/signals.csv``.
"""

import argparse
import csv
import datetime
import json
import math
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
REPO_ROOT = HERE.parent

FMS_RECORDING = HERE / "signalsFmsRecording.csv"
COVESA_RECORDING = HERE / "signalsCovesaCvRecording.csv"
VEHICLES_FILE = REPO_ROOT / "seed" / "vehicles.json"
OUT_DIR = HERE / "vehicles"

VIN_SIGNAL = "Vehicle.VehicleIdentification.VIN"
LAT_SIGNAL = "Vehicle.CurrentLocation.Latitude"
LON_SIGNAL = "Vehicle.CurrentLocation.Longitude"
HEADING_SIGNAL = "Vehicle.CurrentLocation.Heading"
TIMESTAMP_SIGNAL = "Vehicle.CurrentLocation.Timestamp"

FIELDNAMES = ["field", "signal", "value", "delay"]

# One position update every POSITION_EVERY rows of the FMS recording. The
# recording's 3989 rows carry ~896 s of delays, i.e. ~225 ms per row, so a fix
# every 4 rows works out at roughly 0.9 s. That keeps the map moving at about
# 1 Hz; going much sparser pushes the end-to-end latency (replay + forwarder +
# consumer + the backend's 1 s InfluxDB poll) past what the operator UI and the
# e2e "marker moves within 3 seconds" test expect.
POSITION_EVERY = 4

# fms-server only returns a gnssPosition when positionDateTime, longitude and
# latitude are all present (influx_reader.rs:176), so every fix must carry a
# timestamp or the rFMS API reports no position at all. fms-forwarder parses the
# value with chrono::DateTime::parse_from_rfc3339 (kuksa.rs:118), so it must be
# an RFC3339 string.
#
# A recording cannot know its own replay time, so the timestamps are historical
# and repeat on every loop. That is inherent to a replayed recording. Consumers
# order by the InfluxDB write time, not by this field.
TIMESTAMP_BASE = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
TIMESTAMP_STEP = datetime.timedelta(milliseconds=900)


def read_rows(path):
    with path.open(newline="") as handle:
        return list(csv.DictReader(handle))


def read_position_track(path):
    """Extract the ordered lat/lon pairs from a recording.

    The Covesa recording emits Latitude, Longitude and Altitude as consecutive
    rows per fix. We pair each latitude with the next longitude that follows it
    and ignore everything else.
    """
    track = []
    pending_lat = None
    for row in read_rows(path):
        if row["signal"] == LAT_SIGNAL:
            pending_lat = float(row["value"])
        elif row["signal"] == LON_SIGNAL and pending_lat is not None:
            track.append((pending_lat, float(row["value"])))
            pending_lat = None
    if not track:
        raise SystemExit(f"no position fixes found in {path}")
    return track


def rebase_track(track, start_lat, start_lon):
    """Translate a track so that it begins at the vehicle's seeded position.

    The recorded track is in Ontario; the seeded fleet sits around Paris. A plain
    translation keeps the shape of the drive while putting each vehicle at its
    own starting point, which is all the demo needs.
    """
    origin_lat, origin_lon = track[0]
    return [
        (lat - origin_lat + start_lat, lon - origin_lon + start_lon)
        for lat, lon in track
    ]


def bearing(start, end):
    """Return the initial great-circle bearing between two fixes, in degrees."""
    lat1, lon1 = math.radians(start[0]), math.radians(start[1])
    lat2, lon2 = math.radians(end[0]), math.radians(end[1])
    delta_lon = lon2 - lon1
    y = math.sin(delta_lon) * math.cos(lat2)
    x = math.cos(lat1) * math.sin(lat2) - math.sin(lat1) * math.cos(lat2) * math.cos(delta_lon)
    return int(round(math.degrees(math.atan2(y, x)))) % 360


def build_recording(fms_rows, track, vin):
    """Interleave position fixes into a copy of the FMS recording."""
    out = []
    fix_index = 0
    for i, row in enumerate(fms_rows):
        row = dict(row)
        if row["signal"] == VIN_SIGNAL:
            row["value"] = vin
        out.append(row)

        if i % POSITION_EVERY == POSITION_EVERY - 1:
            lat, lon = track[fix_index % len(track)]
            nxt = track[(fix_index + 1) % len(track)]
            instant = TIMESTAMP_BASE + fix_index * TIMESTAMP_STEP
            fix_index += 1
            # delay 0: the whole fix is published alongside the surrounding FMS
            # signals rather than adding to the replay's wall-clock length.
            out.append(
                {"field": "current", "signal": LAT_SIGNAL, "value": f"{lat:.7f}", "delay": "0"}
            )
            out.append(
                {"field": "current", "signal": LON_SIGNAL, "value": f"{lon:.7f}", "delay": "0"}
            )
            out.append(
                {
                    "field": "current",
                    "signal": HEADING_SIGNAL,
                    "value": str(bearing((lat, lon), nxt)),
                    "delay": "0",
                }
            )
            out.append(
                {
                    "field": "current",
                    "signal": TIMESTAMP_SIGNAL,
                    "value": instant.strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z",
                    "delay": "0",
                }
            )
    return out


def write_recording(path, rows):
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=FIELDNAMES)
        writer.writeheader()
        writer.writerows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--vehicles",
        type=int,
        default=3,
        help="number of vehicles to generate recordings for (default: 3)",
    )
    args = parser.parse_args()

    fleet = json.loads(VEHICLES_FILE.read_text())[: args.vehicles]
    if len(fleet) < args.vehicles:
        raise SystemExit(
            f"{VEHICLES_FILE} only defines {len(fleet)} vehicles, "
            f"cannot generate {args.vehicles}"
        )

    fms_rows = read_rows(FMS_RECORDING)
    track = read_position_track(COVESA_RECORDING)

    OUT_DIR.mkdir(exist_ok=True)
    for offset, vehicle in enumerate(fleet):
        vin = vehicle["vin"]
        # Stagger the starting point along the track so the vehicles do not all
        # drive in lockstep.
        rotation = (offset * len(track) // max(len(fleet), 1)) % len(track)
        rotated = track[rotation:] + track[:rotation]
        rebased = rebase_track(rotated, vehicle["latitude"], vehicle["longitude"])

        out_path = OUT_DIR / f"{vin}.csv"
        write_recording(out_path, build_recording(fms_rows, rebased, vin))
        print(f"wrote {out_path.relative_to(REPO_ROOT)} ({vin})")


if __name__ == "__main__":
    main()
