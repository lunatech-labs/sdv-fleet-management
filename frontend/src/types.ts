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
