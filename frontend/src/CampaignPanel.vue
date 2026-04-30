<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import type { Campaign, VehicleRecord, VehicleUpdateState } from './types'

const props = defineProps<{
  vehicles: Record<string, VehicleRecord>
  campaigns: Record<string, Campaign>
}>()

const emit = defineEmits<{ launched: [Campaign] }>()

const BACKEND = (import.meta.env.VITE_BACKEND_URL as string | undefined) ?? 'http://localhost:3000'

const versions = ref<string[]>([])
const selectedVersion = ref<string>('')
const selectedVins = ref<Set<string>>(new Set())
const launching = ref(false)
const errorMsg = ref<string | null>(null)

onMounted(async () => {
  // Preselect every known VIN so the operator just clicks Launch.
  for (const vin of Object.keys(props.vehicles)) selectedVins.value.add(vin)
  await loadVersions()
})

async function loadVersions() {
  try {
    const res = await fetch(`${BACKEND}/versions`)
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    const body = await res.json() as { versions: string[] }
    versions.value = body.versions
    if (body.versions.length > 0) selectedVersion.value = body.versions[0]
  } catch (e) {
    errorMsg.value = `Failed to load versions: ${(e as Error).message}`
  }
}

function toggleVin(vin: string) {
  if (selectedVins.value.has(vin)) selectedVins.value.delete(vin)
  else selectedVins.value.add(vin)
}

function toggleAll() {
  if (selectedVins.value.size === Object.keys(props.vehicles).length) {
    selectedVins.value.clear()
  } else {
    for (const vin of Object.keys(props.vehicles)) selectedVins.value.add(vin)
  }
}

async function launch() {
  errorMsg.value = null
  if (!selectedVersion.value || selectedVins.value.size === 0) return
  launching.value = true
  try {
    const res = await fetch(`${BACKEND}/campaigns`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        version: selectedVersion.value,
        vins: [...selectedVins.value],
      }),
    })
    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: `HTTP ${res.status}` }))
      throw new Error(body.error ?? `HTTP ${res.status}`)
    }
    // The /ws/campaigns stream only broadcasts vehicle transitions, not
    // creations. Merge the POST response in directly so the card appears
    // without waiting for a reconnect snapshot.
    const created = await res.json() as Campaign
    emit('launched', created)
  } catch (e) {
    errorMsg.value = `Launch failed: ${(e as Error).message}`
  } finally {
    launching.value = false
  }
}

function stateLabel(s: VehicleUpdateState): string {
  switch (s.state) {
    case 'COMPLETE': return `COMPLETE ${s.version}`
    case 'FAILED':   return `FAILED: ${s.error}`
    default:         return s.state
  }
}

function stateClass(s: VehicleUpdateState): string {
  return `chip chip-${s.state.toLowerCase()}`
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleTimeString()
}

const sortedCampaigns = computed(() =>
  Object.values(props.campaigns).sort((a, b) => b.created.localeCompare(a.created))
)

const allVins = computed(() => Object.keys(props.vehicles).sort())
const allSelected = computed(() =>
  allVins.value.length > 0 && selectedVins.value.size === allVins.value.length
)
</script>

<template>
  <div class="campaign-panel">
    <div class="panel-header">
      Campaigns
      <span class="count">{{ sortedCampaigns.length }}</span>
    </div>

    <div class="launcher">
      <div class="field">
        <label>Version</label>
        <select v-model="selectedVersion">
          <option
            v-if="versions.length === 0"
            value=""
            disabled
          >
            No versions available
          </option>
          <option
            v-for="v in versions"
            :key="v"
            :value="v"
          >
            {{ v }}
          </option>
        </select>
      </div>

      <div class="field">
        <label>
          Vehicles
          <button
            class="link"
            @click="toggleAll"
          >
            {{ allSelected ? 'Clear' : 'All' }}
          </button>
        </label>
        <div class="vin-list">
          <label
            v-for="vin in allVins"
            :key="vin"
            class="vin-row"
          >
            <input
              type="checkbox"
              :checked="selectedVins.has(vin)"
              @change="toggleVin(vin)"
            >
            <span class="mono">{{ vin }}</span>
          </label>
        </div>
      </div>

      <button
        class="launch-btn"
        :disabled="launching || !selectedVersion || selectedVins.size === 0"
        @click="launch"
      >
        {{ launching ? 'Launching...' : `Launch (${selectedVins.size})` }}
      </button>

      <div
        v-if="errorMsg"
        class="error"
      >
        {{ errorMsg }}
      </div>
    </div>

    <div class="scroll">
      <div
        v-if="sortedCampaigns.length === 0"
        class="empty"
      >
        No campaigns yet.
      </div>
      <div
        v-for="c in sortedCampaigns"
        :key="c.id"
        class="card"
      >
        <div class="card-header">
          <span class="version mono">{{ c.version }}</span>
          <span class="card-time">{{ formatTime(c.created) }}</span>
        </div>
        <div class="card-id mono">
          {{ c.id.slice(0, 8) }}
        </div>
        <ul class="vehicles">
          <li
            v-for="(s, vin) in c.vehicles"
            :key="vin"
          >
            <span class="mono">{{ vin }}</span>
            <span :class="stateClass(s)">{{ stateLabel(s) }}</span>
          </li>
        </ul>
      </div>
    </div>
  </div>
</template>

<style scoped>
.campaign-panel {
  display: flex;
  flex-direction: column;
  width: 380px;
  min-width: 360px;
  height: 100%;
  background: #fafafa;
  border-left: 1px solid #e0e0e0;
}

.panel-header {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 14px 16px;
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #555;
  border-bottom: 1px solid #e0e0e0;
  background: #fff;
}

.count {
  background: #e8e8e8;
  color: #666;
  border-radius: 10px;
  padding: 1px 7px;
  font-size: 11px;
}

.launcher {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  border-bottom: 1px solid #e0e0e0;
  background: #fff;
}

.field { display: flex; flex-direction: column; gap: 4px; }
.field label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: #999;
}

.field select {
  padding: 5px 7px;
  border: 1px solid #d0d0d0;
  border-radius: 4px;
  font-size: 12px;
  background: #fafafa;
  outline: none;
}
.field select:focus { border-color: #888; background: #fff; }

.link {
  background: none;
  border: none;
  color: #2a66d4;
  cursor: pointer;
  font-size: 10px;
  text-transform: uppercase;
  padding: 0;
}
.link:hover { text-decoration: underline; }

.vin-list {
  max-height: 140px;
  overflow-y: auto;
  border: 1px solid #e0e0e0;
  border-radius: 4px;
  background: #fafafa;
}

.vin-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
  cursor: pointer;
  text-transform: none;
  font-size: 12px;
  color: #222;
  letter-spacing: 0;
  font-weight: 400;
}
.vin-row:hover { background: #f0f0f0; }

.launch-btn {
  padding: 8px 12px;
  border: none;
  border-radius: 4px;
  background: #2a66d4;
  color: #fff;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.launch-btn:disabled { background: #bcbcbc; cursor: not-allowed; }
.launch-btn:not(:disabled):hover { background: #214f9f; }

.error {
  color: #b00020;
  font-size: 12px;
  padding: 4px 0;
}

.scroll { flex: 1; overflow-y: auto; padding: 12px 14px; }
.empty { text-align: center; color: #aaa; font-size: 13px; padding: 20px 0; }

.card {
  background: #fff;
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  padding: 10px 12px;
  margin-bottom: 10px;
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 2px;
}

.version { font-size: 13px; font-weight: 600; color: #222; }
.card-time { font-size: 11px; color: #999; }
.card-id { font-size: 10px; color: #aaa; margin-bottom: 8px; }

.vehicles {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.vehicles li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
}

.chip {
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.04em;
  padding: 2px 6px;
  border-radius: 10px;
  color: #fff;
}
.chip-pending     { background: #9e9e9e; }
.chip-downloading { background: #2a66d4; }
.chip-installing  { background: #ff9800; }
.chip-complete    { background: #2e8b57; }
.chip-failed      { background: #b00020; }

.mono { font-family: monospace; }
</style>
