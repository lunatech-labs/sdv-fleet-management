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

export type VehicleUpdateState =
  | { state: 'PENDING' }
  | { state: 'DOWNLOADING' }
  | { state: 'INSTALLING' }
  | { state: 'COMPLETE', version: string }
  | { state: 'FAILED', error: string }

export interface Campaign {
  id: string
  version: string
  vehicles: Record<string, VehicleUpdateState>
  created: string
}

export type WsCampaignMessage =
  | { type: 'snapshot', campaigns: Record<string, Campaign> }
  | { type: 'transition', campaign_id: string, vin: string } & VehicleUpdateState
