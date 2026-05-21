import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { defineComponent, h } from 'vue'
import { mount } from '@vue/test-utils'
import { useCampaignSocket } from '../useCampaignSocket'
import type { Campaign } from '../types'

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

function mountWithComposable() {
  let socket!: ReturnType<typeof useCampaignSocket>
  const wrapper = mount(
    defineComponent({
      setup() {
        socket = useCampaignSocket()
        return () => h('div')
      },
    }),
  )
  return { wrapper, get socket() { return socket } }
}

function makeCampaign(id: string, vin = 'VIN-0001'): Campaign {
  return {
    id,
    version: '1.0.0',
    vehicles: { [vin]: { state: 'PENDING' } },
    created: '2024-01-01T00:00:00Z',
  }
}

function send(ws: MockWebSocket, data: unknown) {
  ws.onmessage?.({ data: JSON.stringify(data) })
}

describe('useCampaignSocket', () => {
  beforeEach(() => { MockWebSocket.instances = [] })
  afterEach(() => vi.useRealTimers())

  it('snapshot message populates campaigns reactively', () => {
    const { socket } = mountWithComposable()
    send(MockWebSocket.instances[0], {
      type: 'snapshot',
      campaigns: { 'camp-1': makeCampaign('camp-1') },
    })

    expect(socket.campaigns['camp-1']).toBeDefined()
    expect(socket.campaigns['camp-1'].version).toBe('1.0.0')
  })

  it('snapshot clears previously known campaigns', () => {
    const { socket } = mountWithComposable()
    const ws = MockWebSocket.instances[0]

    send(ws, { type: 'snapshot', campaigns: { 'camp-1': makeCampaign('camp-1') } })
    expect(Object.keys(socket.campaigns)).toHaveLength(1)

    send(ws, { type: 'snapshot', campaigns: { 'camp-2': makeCampaign('camp-2') } })
    expect(socket.campaigns['camp-1']).toBeUndefined()
    expect(socket.campaigns['camp-2']).toBeDefined()
  })

  it('transition message updates vehicle state', () => {
    const { socket } = mountWithComposable()
    const ws = MockWebSocket.instances[0]

    send(ws, { type: 'snapshot', campaigns: { 'camp-1': makeCampaign('camp-1') } })
    send(ws, { type: 'transition', campaign_id: 'camp-1', vin: 'VIN-0001', state: 'DOWNLOADING' })

    expect(socket.campaigns['camp-1'].vehicles['VIN-0001'].state).toBe('DOWNLOADING')
  })

  it('COMPLETE transition carries version', () => {
    const { socket } = mountWithComposable()
    const ws = MockWebSocket.instances[0]

    send(ws, { type: 'snapshot', campaigns: { 'camp-1': makeCampaign('camp-1') } })
    send(ws, {
      type: 'transition',
      campaign_id: 'camp-1',
      vin: 'VIN-0001',
      state: 'COMPLETE',
      version: '2.0.0',
    })

    const s = socket.campaigns['camp-1'].vehicles['VIN-0001']
    expect(s.state).toBe('COMPLETE')
    if (s.state === 'COMPLETE') expect(s.version).toBe('2.0.0')
  })

  it('FAILED transition carries error', () => {
    const { socket } = mountWithComposable()
    const ws = MockWebSocket.instances[0]

    send(ws, { type: 'snapshot', campaigns: { 'camp-1': makeCampaign('camp-1') } })
    send(ws, {
      type: 'transition',
      campaign_id: 'camp-1',
      vin: 'VIN-0001',
      state: 'FAILED',
      error: 'simulated failure',
    })

    const s = socket.campaigns['camp-1'].vehicles['VIN-0001']
    expect(s.state).toBe('FAILED')
    if (s.state === 'FAILED') expect(s.error).toBe('simulated failure')
  })

  it('transition for unknown campaign is silently ignored', () => {
    const { socket } = mountWithComposable()
    send(MockWebSocket.instances[0], {
      type: 'transition',
      campaign_id: 'no-such-campaign',
      vin: 'VIN-0001',
      state: 'DOWNLOADING',
    })
    expect(Object.keys(socket.campaigns)).toHaveLength(0)
  })

  it('calls onTransition handlers on state change', () => {
    const { socket } = mountWithComposable()
    const ws = MockWebSocket.instances[0]
    const handler = vi.fn()
    socket.onTransition(handler)

    send(ws, { type: 'snapshot', campaigns: { 'camp-1': makeCampaign('camp-1') } })
    send(ws, { type: 'transition', campaign_id: 'camp-1', vin: 'VIN-0001', state: 'INSTALLING' })

    expect(handler).toHaveBeenCalledWith('camp-1', 'VIN-0001', { state: 'INSTALLING' })
  })

  it('silently ignores invalid JSON', () => {
    const { socket } = mountWithComposable()
    expect(() =>
      MockWebSocket.instances[0].onmessage?.({ data: 'not-json' }),
    ).not.toThrow()
    expect(Object.keys(socket.campaigns)).toHaveLength(0)
  })

  it('reconnects 3 s after close', () => {
    vi.useFakeTimers()
    mountWithComposable()

    MockWebSocket.instances[0].onclose?.()
    expect(MockWebSocket.instances).toHaveLength(1)

    vi.advanceTimersByTime(3_000)
    expect(MockWebSocket.instances).toHaveLength(2)
  })

  it('does not reconnect after unmount', () => {
    vi.useFakeTimers()
    const { wrapper } = mountWithComposable()

    wrapper.unmount()
    MockWebSocket.instances[0].onclose?.()

    vi.advanceTimersByTime(3_000)
    expect(MockWebSocket.instances).toHaveLength(1)
  })

  it('onerror closes the socket', () => {
    mountWithComposable()
    const ws = MockWebSocket.instances[0]
    ws.onerror?.()
    expect(ws.close).toHaveBeenCalled()
  })
})
