import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import VehicleDrawer from '../VehicleDrawer.vue'
import type { VehicleRecord } from '../types'

const vehicle: VehicleRecord = {
  vin: 'VIN-0001',
  brand: 'Acme',
  model: 'X1',
  software_version: '1.0.0',
  latitude: 48.85341,
  longitude: 2.34880,
  last_seen: '2024-01-01T00:00:00Z',
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
})
