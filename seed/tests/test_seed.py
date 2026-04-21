import pytest
from unittest.mock import MagicMock, patch, call
from kuksa_client.grpc import Datapoint

import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent.parent))

from seed import seed_vehicle


def make_mock_client():
    client = MagicMock()
    client.__enter__ = MagicMock(return_value=client)
    client.__exit__ = MagicMock(return_value=False)
    return client


@patch("seed.VSSClient")
def test_seed_vehicle_writes_correct_signals(mock_vss_client):
    client = make_mock_client()
    mock_vss_client.return_value = client

    vehicle = {
        "vin": "VIN-0001",
        "brand": "Toyota",
        "model": "Camry",
        "software_version": "1.0.0",
        "latitude": 48.8566,
        "longitude": 2.3522,
    }

    seed_vehicle(vehicle, 1)

    client.set_current_values.assert_called_once()
    written = client.set_current_values.call_args[0][0]

    assert "Vehicle.VehicleIdentification.VIN" in written
    assert "Vehicle.VehicleIdentification.Brand" in written
    assert "Vehicle.VehicleIdentification.Model" in written
    assert "Vehicle.CurrentLocation.Latitude" in written
    assert "Vehicle.CurrentLocation.Longitude" in written


@patch("seed.VSSClient")
def test_seed_vehicle_never_writes_software_version(mock_vss_client):
    client = make_mock_client()
    mock_vss_client.return_value = client

    vehicle = {
        "vin": "VIN-0001",
        "brand": "Toyota",
        "model": "Camry",
        "software_version": "1.0.0",
        "latitude": 48.8566,
        "longitude": 2.3522,
    }

    seed_vehicle(vehicle, 1)

    written = client.set_current_values.call_args[0][0]
    assert not any("SoftwareVersion" in key for key in written)


@patch("seed.VSSClient")
def test_seed_vehicle_connects_to_correct_databroker(mock_vss_client):
    client = make_mock_client()
    mock_vss_client.return_value = client

    vehicle = {
        "vin": "VIN-0007",
        "brand": "Audi",
        "model": "A4",
        "software_version": "3.0.0",
        "latitude": 48.8678,
        "longitude": 2.3156,
    }

    seed_vehicle(vehicle, 7)

    mock_vss_client.assert_called_once_with("databroker-07", 55555)


@patch("seed.time.sleep")
@patch("seed.VSSClient")
def test_seed_vehicle_retries_on_failure(mock_vss_client, mock_sleep):
    client = make_mock_client()
    client.set_current_values.side_effect = [Exception("not ready"), Exception("not ready"), None]
    mock_vss_client.return_value = client

    vehicle = {
        "vin": "VIN-0001",
        "brand": "Toyota",
        "model": "Camry",
        "software_version": "1.0.0",
        "latitude": 48.8566,
        "longitude": 2.3522,
    }

    seed_vehicle(vehicle, 1)

    assert client.set_current_values.call_count == 3
    assert mock_sleep.call_count == 2


@patch("seed.time.sleep")
@patch("seed.VSSClient")
def test_seed_vehicle_raises_after_max_retries(mock_vss_client, mock_sleep):
    client = make_mock_client()
    client.set_current_values.side_effect = Exception("not ready")
    mock_vss_client.return_value = client

    vehicle = {
        "vin": "VIN-0001",
        "brand": "Toyota",
        "model": "Camry",
        "software_version": "1.0.0",
        "latitude": 48.8566,
        "longitude": 2.3522,
    }

    with pytest.raises(RuntimeError, match="Could not seed VIN-0001"):
        seed_vehicle(vehicle, 1)
