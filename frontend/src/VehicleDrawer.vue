<script setup lang="ts">
import type { VehicleRecord } from './types'

defineProps<{ vehicle: VehicleRecord | null }>()
defineEmits<{ close: [] }>()
</script>

<template>
  <Transition name="drawer">
    <div v-if="vehicle" class="drawer">
      <button class="close" @click="$emit('close')">✕</button>
      <h2>{{ vehicle.brand }} {{ vehicle.model }}</h2>
      <dl>
        <dt>VIN</dt>      <dd>{{ vehicle.vin }}</dd>
        <dt>Software</dt> <dd>{{ vehicle.software_version }}</dd>
        <dt>Lat</dt>      <dd>{{ vehicle.latitude.toFixed(5) }}</dd>
        <dt>Lon</dt>      <dd>{{ vehicle.longitude.toFixed(5) }}</dd>
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

.drawer-enter-active, .drawer-leave-active { transition: transform 0.2s ease; }
.drawer-enter-from, .drawer-leave-to       { transform: translateX(100%); }
</style>
