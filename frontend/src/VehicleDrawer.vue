<script setup lang="ts">
import { computed } from 'vue'
import type { Campaign, VehicleRecord, VehicleUpdateState } from './types'

const props = defineProps<{
  vehicle: VehicleRecord | null
  campaigns?: Record<string, Campaign>
}>()

defineEmits<{ close: [] }>()

// Latest campaign state for this vehicle, if any. Pick the most-recently
// created campaign that references this VIN.
const activeState = computed<VehicleUpdateState | null>(() => {
  if (!props.vehicle || !props.campaigns) return null
  const vin = props.vehicle.vin
  const candidates = Object.values(props.campaigns)
    .filter(c => c.vehicles[vin])
    .sort((a, b) => b.created.localeCompare(a.created))
  return candidates[0]?.vehicles[vin] ?? null
})

function stateLabel(s: VehicleUpdateState): string {
  switch (s.state) {
    case 'COMPLETE': return `COMPLETE ${s.version}`
    case 'FAILED':   return `FAILED: ${s.error}`
    default:         return s.state
  }
}
</script>

<template>
  <Transition name="drawer">
    <div
      v-if="vehicle"
      class="drawer"
    >
      <button
        class="close"
        @click="$emit('close')"
      >
        ✕
      </button>
      <h2>{{ vehicle.brand }} {{ vehicle.model }}</h2>
      <dl>
        <dt>VIN</dt>      <dd>{{ vehicle.vin }}</dd>
        <dt>Software</dt> <dd>{{ vehicle.software_version }}</dd>
        <dt>Lat</dt>      <dd>{{ vehicle.latitude.toFixed(5) }}</dd>
        <dt>Lon</dt>      <dd>{{ vehicle.longitude.toFixed(5) }}</dd>
        <template v-if="activeState">
          <dt>Update</dt>
          <dd>
            <span :class="['chip', `chip-${activeState.state.toLowerCase()}`]">
              {{ stateLabel(activeState) }}
            </span>
          </dd>
        </template>
      </dl>
    </div>
  </Transition>
</template>

<style scoped>
.drawer {
  position: fixed;
  top: 0; right: 0;
  width: 260px; height: 100%;
  background: #fff;
  box-shadow: -4px 0 20px rgba(0, 0, 0, 0.12);
  padding: 24px 20px;
  z-index: 1000;
}

.close {
  position: absolute;
  top: 14px; right: 14px;
  background: none; border: none;
  font-size: 18px; cursor: pointer; color: #888;
}
.close:hover { color: #333; }

h2 { font-size: 18px; margin-bottom: 20px; padding-right: 24px; }

dl {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 10px 16px;
  align-items: baseline;
}

dt {
  font-size: 11px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.06em;
  color: #999;
}

dd { font-size: 14px; font-family: monospace; color: #222; }

.chip {
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.04em;
  padding: 2px 6px;
  border-radius: 10px;
  color: #fff;
  font-family: inherit;
}
.chip-pending     { background: #9e9e9e; }
.chip-downloading { background: #2a66d4; }
.chip-installing  { background: #ff9800; }
.chip-complete    { background: #2e8b57; }
.chip-failed      { background: #b00020; }

.drawer-enter-active, .drawer-leave-active { transition: transform 0.2s ease; }
.drawer-enter-from, .drawer-leave-to       { transform: translateX(100%); }
</style>
