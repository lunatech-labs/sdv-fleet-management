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

import { onUnmounted } from 'vue'
import type { PositionEvent } from './types'

import { BACKEND_WS } from './config'

const WS_URL = BACKEND_WS + '/ws/fleet'

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
