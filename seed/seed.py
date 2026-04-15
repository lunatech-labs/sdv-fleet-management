#!/usr/bin/env python3
"""
Seed script — writes static VSS signals and initial GPS positions into each
of the 20 Kuksa Databrokers. Retries each connection until the broker is ready.
Exits 0 on success, 1 on unrecoverable error.
"""

import json
import logging
import sys
import time
from pathlib import Path

from kuksa_client.grpc import Datapoint, VSSClient

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [seed] %(levelname)s %(message)s",
    datefmt="%H:%M:%S",
)
log = logging.getLogger(__name__)

VEHICLES_FILE = Path(__file__).parent / "vehicles.json"
RETRY_INTERVAL = 2   # seconds between connection attempts
MAX_RETRIES    = 30  # ~60 s total per vehicle

VSS_SIGNALS = [
    "Vehicle.VehicleIdentification.VIN",
    "Vehicle.VehicleIdentification.Brand",
    "Vehicle.VehicleIdentification.Model",
    "Vehicle.CurrentLocation.Latitude",
    "Vehicle.CurrentLocation.Longitude",
]


def seed_vehicle(vehicle: dict, index: int) -> None:
    """Connect to the databroker for this vehicle and write all six signals."""
    host = f"databroker-{index:02d}"
    port = 55555
    vin  = vehicle["vin"]

    values = {
        "Vehicle.VehicleIdentification.VIN":   Datapoint(vehicle["vin"]),
        "Vehicle.VehicleIdentification.Brand":  Datapoint(vehicle["brand"]),
        "Vehicle.VehicleIdentification.Model":  Datapoint(vehicle["model"]),
        "Vehicle.CurrentLocation.Latitude":     Datapoint(vehicle["latitude"]),
        "Vehicle.CurrentLocation.Longitude":    Datapoint(vehicle["longitude"]),
    }

    for attempt in range(1, MAX_RETRIES + 1):
        try:
            with VSSClient(host, port) as client:
                client.set_current_values(values)
            log.info("%s ✓ seeded via %s:%d", vin, host, port)
            return
        except Exception as exc:
            if attempt == MAX_RETRIES:
                raise RuntimeError(
                    f"Could not seed {vin} after {MAX_RETRIES} attempts: {exc}"
                ) from exc
            log.warning(
                "%s — %s:%d not ready (attempt %d/%d), retrying in %ds…",
                vin, host, port, attempt, MAX_RETRIES, RETRY_INTERVAL,
            )
            time.sleep(RETRY_INTERVAL)


def main() -> None:
    vehicles = json.loads(VEHICLES_FILE.read_text())
    log.info("Seeding %d vehicles…", len(vehicles))

    errors = []
    for i, vehicle in enumerate(vehicles, start=1):
        try:
            seed_vehicle(vehicle, i)
        except RuntimeError as exc:
            log.error(str(exc))
            errors.append(str(exc))

    if errors:
        log.error("Seed failed for %d vehicle(s).", len(errors))
        sys.exit(1)

    log.info("All vehicles seeded successfully.")


if __name__ == "__main__":
    main()
