<script setup lang="ts">
import { reactive, ref, computed, onMounted } from 'vue'
import MapView from './MapView.vue'
import VehicleDrawer from './VehicleDrawer.vue'
import FleetTable from './FleetTable.vue'
import { useFleetSocket } from './useFleetSocket'
import type { VehicleRecord, PositionEvent } from './types'

const BACKEND = (import.meta.env.VITE_BACKEND_URL as string | undefined) ?? 'http://localhost:3000'

const vehicles = reactive<Record<string, VehicleRecord>>({})
const selected = ref<VehicleRecord | null>(null)
const selectedVin = computed(() => selected.value?.vin ?? null)
const tableVisible = ref(false)

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
    <div class="map-panel">
      <MapView
        :vehicles="vehicles"
        @select="selected = $event"
      />
      <button
        class="toggle-btn"
        :title="tableVisible ? 'Hide table' : 'Show table'"
        @click="tableVisible = !tableVisible"
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          fill="currentColor"
          xmlns="http://www.w3.org/2000/svg"
        >
          <rect
            x="1"
            y="2"
            width="14"
            height="3"
            rx="1"
          />
          <rect
            x="1"
            y="6.5"
            width="14"
            height="2.5"
            rx="1"
          />
          <rect
            x="1"
            y="10.5"
            width="14"
            height="2.5"
            rx="1"
          />
        </svg>
      </button>
    </div>
    <FleetTable
      v-if="tableVisible"
      :vehicles="vehicles"
      :selected-vin="selectedVin"
    />
    <VehicleDrawer
      :vehicle="selected"
      @close="selected = null"
    />
  </div>
</template>

<style>
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html, body, #app { width: 100%; height: 100%; }
.app { display: flex; width: 100%; height: 100%; }
.map-panel { position: relative; flex: 1; min-width: 0; height: 100%; }
.toggle-btn {
  position: absolute;
  top: 10px;
  right: 10px;
  z-index: 1000;
  width: 32px;
  height: 32px;
  background: #fff;
  border: 1px solid #ccc;
  border-radius: 4px;
  cursor: pointer;
  font-size: 10px;
  box-shadow: 0 1px 4px rgba(0,0,0,0.2);
}
.toggle-btn:hover { background: #f0f0f0; }
</style>
