// SPDX-FileCopyrightText: 2026 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

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
