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

import { reactive, onUnmounted } from 'vue'
import type { Campaign, VehicleUpdateState, WsCampaignMessage } from './types'

import { BACKEND_WS } from './config'

const WS_URL = BACKEND_WS + '/ws/campaigns'

export interface CampaignSocket {
  campaigns: Record<string, Campaign>
  onTransition(handler: (campaignId: string, vin: string, state: VehicleUpdateState) => void): void
}

type TransitionMsg = Extract<WsCampaignMessage, { type: 'transition' }>

function extractState(msg: TransitionMsg): VehicleUpdateState {
  switch (msg.state) {
    case 'COMPLETE': return { state: 'COMPLETE', version: msg.version }
    case 'FAILED':   return { state: 'FAILED',   error:   msg.error }
    default:         return { state: msg.state }
  }
}

export function useCampaignSocket(): CampaignSocket {
  const campaigns = reactive<Record<string, Campaign>>({})
  const handlers: Array<(campaignId: string, vin: string, state: VehicleUpdateState) => void> = []

  let ws: WebSocket | null = null
  let stopped = false

  function connect() {
    if (stopped) return
    ws = new WebSocket(WS_URL)

    ws.onmessage = (ev: MessageEvent) => {
      let msg: WsCampaignMessage
      try { msg = JSON.parse(ev.data as string) as WsCampaignMessage } catch { return }

      if (msg.type === 'snapshot') {
        for (const key of Object.keys(campaigns)) delete campaigns[key]
        for (const [id, c] of Object.entries(msg.campaigns)) campaigns[id] = c
      } else if (msg.type === 'transition') {
        const c = campaigns[msg.campaign_id]
        if (!c) return
        const state = extractState(msg)
        c.vehicles[msg.vin] = state
        for (const h of handlers) h(msg.campaign_id, msg.vin, state)
      }
    }
    ws.onclose = () => { if (!stopped) setTimeout(connect, 3_000) }
    ws.onerror = () => ws?.close()
  }

  connect()

  onUnmounted(() => { stopped = true; ws?.close() })

  return {
    campaigns,
    onTransition(h) { handlers.push(h) },
  }
}
