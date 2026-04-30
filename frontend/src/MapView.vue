<script setup lang="ts">
import L from 'leaflet'
import { LMap, LTileLayer, LMarker, LTooltip } from '@vue-leaflet/vue-leaflet'
import type { Campaign, VehicleRecord, VehicleUpdateState } from './types'

const props = defineProps<{
  vehicles: Record<string, VehicleRecord>
  campaigns?: Record<string, Campaign>
}>()

defineEmits<{ select: [vehicle: VehicleRecord] }>()

// Classic Leaflet teardrop, redrawn as an inline SVG so we can tint it per
// vehicle state. Same shape and 25x41 footprint as the bundled marker PNG;
// anchor at the bottom tip so the pin "stands" on the map coordinate.
function makePin(color: string): L.DivIcon {
  const svg = `
    <svg xmlns="http://www.w3.org/2000/svg" width="25" height="41" viewBox="0 0 25 41">
      <path d="M12.5 0C5.6 0 0 5.6 0 12.5 0 21.875 12.5 41 12.5 41S25 21.875 25 12.5C25 5.6 19.4 0 12.5 0z"
            fill="${color}" stroke="#fff" stroke-width="1.5"/>
      <circle cx="12.5" cy="12.5" r="4.5" fill="#fff"/>
    </svg>`
  return L.divIcon({
    className:   'fleet-pin',
    html:        svg,
    iconSize:    [25, 41],
    iconAnchor:  [12, 41],
    tooltipAnchor: [0, -36],
  })
}

const PINS: Record<string, L.DivIcon> = {
  default:     makePin('#2a81cb'),  // Leaflet's familiar blue for vehicles not in any campaign
  PENDING:     makePin('#9e9e9e'),
  DOWNLOADING: makePin('#2a66d4'),
  INSTALLING:  makePin('#ff9800'),
  COMPLETE:    makePin('#2e8b57'),
  FAILED:      makePin('#b00020'),
}

function latestState(vin: string): VehicleUpdateState | null {
  if (!props.campaigns) return null
  const candidates = Object.values(props.campaigns)
    .filter(c => c.vehicles[vin])
    .sort((a, b) => b.created.localeCompare(a.created))
  return candidates[0]?.vehicles[vin] ?? null
}

// @vue-leaflet/vue-leaflet's `icon` prop is typed `Icon<IconOptions>`, which
// doesn't accept `DivIcon<DivIconOptions>` via TS generics even though
// `L.DivIcon extends L.Icon` at runtime. Returning the loose type lets the
// template compile; Leaflet itself happily accepts either.
function iconFor(vin: string): L.Icon {
  const state = latestState(vin)
  const pin = (state && PINS[state.state]) ?? PINS.default
  return pin as unknown as L.Icon
}
</script>

<template>
  <l-map
    :zoom="13"
    :center="[48.8566, 2.3522]"
    style="height: 100%; width: 100%;"
  >
    <l-tile-layer
      url="https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png"
      attribution="&copy; <a href='https://www.openstreetmap.org/copyright'>OpenStreetMap</a> contributors"
    />
    <l-marker
      v-for="vehicle in vehicles"
      :key="vehicle.vin"
      :lat-lng="[vehicle.latitude, vehicle.longitude]"
      :icon="iconFor(vehicle.vin)"
      @click="$emit('select', vehicle)"
    >
      <l-tooltip>{{ vehicle.vin }} {{ vehicle.brand }} {{ vehicle.model }}</l-tooltip>
    </l-marker>
  </l-map>
</template>

<style>
/* Strip leaflet's default divIcon backdrop so only the SVG teardrop shows. */
.fleet-pin {
  background: transparent !important;
  border: none !important;
}
.fleet-pin svg {
  display: block;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.45));
}
</style>
