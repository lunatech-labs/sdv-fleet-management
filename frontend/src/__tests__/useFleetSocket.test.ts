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

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { defineComponent, h } from 'vue'
import { mount } from '@vue/test-utils'
import { useFleetSocket } from '../useFleetSocket'

class MockWebSocket {
  static instances: MockWebSocket[] = []
  onmessage: ((ev: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  close = vi.fn()

  constructor(public url: string) {
    MockWebSocket.instances.push(this)
  }
}

vi.stubGlobal('WebSocket', MockWebSocket)

function mountWithComposable(onEvent: Parameters<typeof useFleetSocket>[0]) {
  return mount(defineComponent({
    setup() {
      useFleetSocket(onEvent)
      return () => h('div')
    },
  }))
}

describe('useFleetSocket', () => {
  beforeEach(() => { MockWebSocket.instances = [] })
  afterEach(() => vi.useRealTimers())

  it('calls onEvent with parsed message', () => {
    const onEvent = vi.fn()
    mountWithComposable(onEvent)

    MockWebSocket.instances[0].onmessage?.({ data: '{"vin":"VIN-0001","lat":1.0,"lon":2.0}' })

    expect(onEvent).toHaveBeenCalledWith({ vin: 'VIN-0001', lat: 1.0, lon: 2.0 })
  })

  it('silently ignores invalid JSON', () => {
    const onEvent = vi.fn()
    mountWithComposable(onEvent)

    expect(() => MockWebSocket.instances[0].onmessage?.({ data: 'not-json' })).not.toThrow()
    expect(onEvent).not.toHaveBeenCalled()
  })

  it('reconnects 3 s after close', () => {
    vi.useFakeTimers()
    mountWithComposable(vi.fn())

    MockWebSocket.instances[0].onclose?.()
    expect(MockWebSocket.instances).toHaveLength(1)

    vi.advanceTimersByTime(3_000)
    expect(MockWebSocket.instances).toHaveLength(2)
  })

  it('does not reconnect after unmount', () => {
    vi.useFakeTimers()
    const wrapper = mountWithComposable(vi.fn())

    wrapper.unmount()
    MockWebSocket.instances[0].onclose?.()

    vi.advanceTimersByTime(3_000)
    expect(MockWebSocket.instances).toHaveLength(1)
  })
})
