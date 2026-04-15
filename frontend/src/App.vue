<script setup lang="ts">
import { reactive, ref, onMounted } from 'vue'
import MapView from './MapView.vue'
import VehicleDrawer from './VehicleDrawer.vue'
import { useFleetSocket } from './useFleetSocket'
import type { VehicleRecord, PositionEvent } from './types'

const BACKEND = (import.meta.env.VITE_BACKEND_URL as string | undefined) ?? 'http://localhost:3000'

const vehicles = reactive<Record<string, VehicleRecord>>({})
const selected = ref<VehicleRecord | null>(null)

onMounted(async () => {
  const data: VehicleRecord[] = await fetch(`${BACKEND}/fleet`).then(r => r.json())
  for (const v of data) vehicles[v.vin] = v
})

useFleetSocket((event: PositionEvent) => {
  const v = vehicles[event.vin]
  if (v) { v.latitude = event.lat; v.longitude = event.lon }
})
</script>

<template>
  <div class="app">
    <MapView :vehicles="vehicles" @select="selected = $event" />
    <VehicleDrawer :vehicle="selected" @close="selected = null" />
  </div>
</template>

<style>
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html, body, #app, .app { width: 100%; height: 100%; }
</style>
