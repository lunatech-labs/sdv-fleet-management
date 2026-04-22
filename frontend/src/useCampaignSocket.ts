import { reactive, onUnmounted } from 'vue'
import type { Campaign, VehicleUpdateState, WsCampaignMessage } from './types'

const BACKEND = (import.meta.env.VITE_BACKEND_URL as string | undefined) ?? 'http://localhost:3000'
const WS_URL  = BACKEND.replace(/^http/, 'ws') + '/ws/campaigns'

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
