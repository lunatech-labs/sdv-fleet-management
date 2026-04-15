import { onUnmounted } from 'vue'
import type { PositionEvent } from './types'

const BACKEND = (import.meta.env.VITE_BACKEND_URL as string | undefined) ?? 'http://localhost:3000'
const WS_URL  = BACKEND.replace(/^http/, 'ws') + '/ws/fleet'

export function useFleetSocket(onEvent: (event: PositionEvent) => void): void {
  let ws: WebSocket | null = null
  let stopped = false

  function connect() {
    if (stopped) return
    ws = new WebSocket(WS_URL)

    ws.onmessage = (ev: MessageEvent) => {
      try { onEvent(JSON.parse(ev.data as string) as PositionEvent) } catch { /* ignore */ }
    }
    ws.onclose = () => { if (!stopped) setTimeout(connect, 3_000) }
    ws.onerror = () => ws?.close()
  }

  connect()

  onUnmounted(() => { stopped = true; ws?.close() })
}
