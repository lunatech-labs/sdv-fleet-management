<script setup lang="ts">
import { LMap, LTileLayer, LMarker, LTooltip } from '@vue-leaflet/vue-leaflet'
import type { VehicleRecord } from './types'

defineProps<{ vehicles: Record<string, VehicleRecord> }>()
defineEmits<{ select: [vehicle: VehicleRecord] }>()
</script>

<template>
  <l-map :zoom="13" :center="[48.8566, 2.3522]" style="height: 100%; width: 100%;">
    <l-tile-layer
      url="https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png"
      attribution="&copy; <a href='https://www.openstreetmap.org/copyright'>OpenStreetMap</a> contributors"
    />
    <l-marker
      v-for="vehicle in vehicles"
      :key="vehicle.vin"
      :lat-lng="[vehicle.latitude, vehicle.longitude]"
      @click="$emit('select', vehicle)"
    >
      <l-tooltip>{{ vehicle.vin }} — {{ vehicle.brand }} {{ vehicle.model }}</l-tooltip>
    </l-marker>
  </l-map>
</template>
