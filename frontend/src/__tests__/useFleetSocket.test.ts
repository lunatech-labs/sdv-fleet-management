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
