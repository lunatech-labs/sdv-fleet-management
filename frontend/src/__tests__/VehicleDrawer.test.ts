import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import VehicleDrawer from '../VehicleDrawer.vue'
import type { Campaign, VehicleRecord } from '../types'

const vehicle: VehicleRecord = {
  vin: 'VIN-0001',
  brand: 'Acme',
  model: 'X1',
  software_version: '1.0.0',
  latitude: 48.85341,
  longitude: 2.34880,
  last_seen: '2024-01-01T00:00:00Z',
}

function makeCampaign(id: string, state: Campaign['vehicles'][string], created = '2024-01-01T00:00:00Z'): Record<string, Campaign> {
  return {
    [id]: {
      id,
      version: '2.0.0',
      vehicles: { 'VIN-0001': state },
      created,
    },
  }
}

describe('VehicleDrawer', () => {
  it('is hidden when vehicle is null', () => {
    const wrapper = mount(VehicleDrawer, { props: { vehicle: null } })
    expect(wrapper.find('.drawer').exists()).toBe(false)
  })

  it('renders vehicle data when vehicle is provided', () => {
    const wrapper = mount(VehicleDrawer, { props: { vehicle } })
    expect(wrapper.find('.drawer').exists()).toBe(true)
    expect(wrapper.text()).toContain('VIN-0001')
    expect(wrapper.text()).toContain('Acme X1')
    expect(wrapper.text()).toContain('1.0.0')
  })

  it('emits close when close button is clicked', async () => {
    const wrapper = mount(VehicleDrawer, { props: { vehicle } })
    await wrapper.find('.close').trigger('click')
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  // ── activeState / chip visibility ─────────────────────────────────────────

  it('shows no chip when campaigns prop is undefined', () => {
    const wrapper = mount(VehicleDrawer, { props: { vehicle } })
    expect(wrapper.find('.chip').exists()).toBe(false)
  })

  it('shows no chip when vehicle has no matching campaign', () => {
    const campaigns: Record<string, Campaign> = {
      'camp-1': {
        id: 'camp-1',
        version: '2.0.0',
        vehicles: { 'OTHER-VIN': { state: 'PENDING' } },
        created: '2024-01-01T00:00:00Z',
      },
    }
    const wrapper = mount(VehicleDrawer, { props: { vehicle, campaigns } })
    expect(wrapper.find('.chip').exists()).toBe(false)
  })

  it('shows chip-pending for PENDING state', () => {
    const wrapper = mount(VehicleDrawer, {
      props: { vehicle, campaigns: makeCampaign('c1', { state: 'PENDING' }) },
    })
    const chip = wrapper.find('.chip')
    expect(chip.exists()).toBe(true)
    expect(chip.classes()).toContain('chip-pending')
    expect(chip.text()).toBe('PENDING')
  })

  it('shows chip-downloading for DOWNLOADING state', () => {
    const wrapper = mount(VehicleDrawer, {
      props: { vehicle, campaigns: makeCampaign('c1', { state: 'DOWNLOADING' }) },
    })
    expect(wrapper.find('.chip').classes()).toContain('chip-downloading')
  })

  it('shows chip-installing for INSTALLING state', () => {
    const wrapper = mount(VehicleDrawer, {
      props: { vehicle, campaigns: makeCampaign('c1', { state: 'INSTALLING' }) },
    })
    expect(wrapper.find('.chip').classes()).toContain('chip-installing')
  })

  it('shows chip-complete with version in label for COMPLETE state', () => {
    const wrapper = mount(VehicleDrawer, {
      props: {
        vehicle,
        campaigns: makeCampaign('c1', { state: 'COMPLETE', version: '2.0.0' }),
      },
    })
    const chip = wrapper.find('.chip')
    expect(chip.classes()).toContain('chip-complete')
    expect(chip.text()).toContain('COMPLETE')
    expect(chip.text()).toContain('2.0.0')
  })

  it('shows chip-failed with error in label for FAILED state', () => {
    const wrapper = mount(VehicleDrawer, {
      props: {
        vehicle,
        campaigns: makeCampaign('c1', { state: 'FAILED', error: 'network timeout' }),
      },
    })
    const chip = wrapper.find('.chip')
    expect(chip.classes()).toContain('chip-failed')
    expect(chip.text()).toContain('FAILED')
    expect(chip.text()).toContain('network timeout')
  })

  it('picks the most recently created campaign for the vehicle', () => {
    const campaigns: Record<string, Campaign> = {
      'camp-old': {
        id: 'camp-old',
        version: '1.0.0',
        vehicles: { 'VIN-0001': { state: 'COMPLETE', version: '1.0.0' } },
        created: '2023-01-01T00:00:00Z',
      },
      'camp-new': {
        id: 'camp-new',
        version: '2.0.0',
        vehicles: { 'VIN-0001': { state: 'INSTALLING' } },
        created: '2024-06-01T00:00:00Z',
      },
    }
    const wrapper = mount(VehicleDrawer, { props: { vehicle, campaigns } })
    expect(wrapper.find('.chip').classes()).toContain('chip-installing')
  })
})
