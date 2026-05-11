<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import type { VehicleRecord } from './types'

const props = defineProps<{
  vehicles: Record<string, VehicleRecord>
  selectedVin: string | null
}>()

const emit = defineEmits<{ filtered: [list: VehicleRecord[]] }>()

type GroupField = 'none' | 'brand' | 'model' | 'software_version'

const searchQuery = ref('')
const filterField = ref<GroupField>('none')
const filterValue = ref('')
const groupBy = ref<GroupField>('none')

watch(filterField, () => { filterValue.value = '' })

const isDirty = computed(() =>
  searchQuery.value.trim() !== '' || filterField.value !== 'none' || groupBy.value !== 'none'
)

function resetFilters() {
  searchQuery.value = ''
  filterField.value = 'none'
  filterValue.value = ''
  groupBy.value = 'none'
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleTimeString()
}

const allVehicles = computed(() => Object.values(props.vehicles))

const filterOptions = computed(() => {
  if (filterField.value === 'none') return []
  const field = filterField.value as keyof VehicleRecord
  return [...new Set(allVehicles.value.map(v => String(v[field])))].sort()
})

const filtered = computed(() => {
  let list = allVehicles.value

  if (searchQuery.value.trim()) {
    const q = searchQuery.value.toLowerCase()
    list = list.filter(v =>
      v.vin.toLowerCase().includes(q) ||
      v.brand.toLowerCase().includes(q) ||
      v.model.toLowerCase().includes(q) ||
      v.software_version.toLowerCase().includes(q)
    )
  }

  if (filterField.value !== 'none' && filterValue.value) {
    const field = filterField.value as keyof VehicleRecord
    list = list.filter(v => String(v[field]) === filterValue.value)
  }

  return list
})

watch(filtered, list => emit('filtered', list), { immediate: true })

const groups = computed(() => {
  if (groupBy.value === 'none') return null
  const field = groupBy.value as keyof VehicleRecord
  const map = new Map<string, VehicleRecord[]>()
  for (const v of filtered.value) {
    const key = String(v[field])
    if (!map.has(key)) map.set(key, [])
    map.get(key)!.push(v)
  }
  return [...map.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, items]) => ({ key, items }))
})
</script>

<template>
  <div class="fleet-table">
    <div class="panel-header">
      Fleet
      <span class="count">
        {{ filtered.length }}
        <template v-if="filtered.length !== allVehicles.length"> / {{ allVehicles.length }}</template>
      </span>
    </div>

    <div class="toolbar">
      <input
        v-model="searchQuery"
        class="search"
        type="search"
        placeholder="Search VIN, make, model, software..."
      >
      <div class="controls">
        <div class="control-group">
          <label>Filter</label>
          <select v-model="filterField">
            <option value="none">
              —
            </option>
            <option value="brand">
              Make
            </option>
            <option value="model">
              Model
            </option>
            <option value="software_version">
              Software
            </option>
          </select>
          <select
            v-if="filterField !== 'none'"
            v-model="filterValue"
          >
            <option value="">
              All
            </option>
            <option
              v-for="opt in filterOptions"
              :key="opt"
              :value="opt"
            >
              {{ opt }}
            </option>
          </select>
        </div>
        <div class="control-group">
          <label>Group</label>
          <select v-model="groupBy">
            <option value="none">
              —
            </option>
            <option value="brand">
              Make
            </option>
            <option value="model">
              Model
            </option>
            <option value="software_version">
              Software
            </option>
          </select>
        </div>
        <button
          v-if="isDirty"
          class="reset-btn"
          @click="resetFilters"
        >
          Reset
        </button>
      </div>
    </div>

    <div class="scroll">
      <table>
        <thead>
          <tr>
            <th>VIN</th>
            <th>Make</th>
            <th>Model</th>
            <th>Software</th>
            <th>Lat</th>
            <th>Lon</th>
            <th>Last seen</th>
          </tr>
        </thead>

        <tbody v-if="!groups">
          <tr
            v-for="v in filtered"
            :key="v.vin"
            :class="{ selected: v.vin === selectedVin }"
          >
            <td class="mono">
              {{ v.vin }}
            </td>
            <td>{{ v.brand }}</td>
            <td>{{ v.model }}</td>
            <td class="mono">
              {{ v.software_version }}
            </td>
            <td class="mono">
              {{ v.latitude.toFixed(4) }}
            </td>
            <td class="mono">
              {{ v.longitude.toFixed(4) }}
            </td>
            <td class="mono">
              {{ formatTime(v.last_seen) }}
            </td>
          </tr>
        </tbody>

        <template v-else>
          <tbody
            v-for="group in groups"
            :key="group.key"
          >
            <tr class="group-header">
              <td colspan="7">
                {{ group.key }}
                <span class="group-count">{{ group.items.length }}</span>
              </td>
            </tr>
            <tr
              v-for="v in group.items"
              :key="v.vin"
              :class="{ selected: v.vin === selectedVin }"
            >
              <td class="mono">
                {{ v.vin }}
              </td>
              <td>{{ v.brand }}</td>
              <td>{{ v.model }}</td>
              <td class="mono">
                {{ v.software_version }}
              </td>
              <td class="mono">
                {{ v.latitude.toFixed(4) }}
              </td>
              <td class="mono">
                {{ v.longitude.toFixed(4) }}
              </td>
              <td class="mono">
                {{ formatTime(v.last_seen) }}
              </td>
            </tr>
          </tbody>
        </template>
      </table>

      <div
        v-if="filtered.length === 0"
        class="empty"
      >
        No vehicles match.
      </div>
    </div>
  </div>
</template>

<style scoped>
.fleet-table {
  display: flex;
  flex-direction: column;
  width: 540px;
  min-width: 520px;
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

.toolbar {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border-bottom: 1px solid #e0e0e0;
  background: #fff;
}

.search {
  width: 100%;
  padding: 6px 8px;
  border: 1px solid #d0d0d0;
  border-radius: 4px;
  font-size: 12px;
  outline: none;
  background: #fafafa;
}
.search:focus { border-color: #888; background: #fff; }

.controls {
  display: flex;
  align-items: center;
  gap: 12px;
}

.reset-btn {
  padding: 3px 9px;
  font-size: 11px;
  border: 1px solid #d0d0d0;
  border-radius: 4px;
  background: #fff;
  color: #666;
  cursor: pointer;
}
.reset-btn:hover { background: #f5f5f5; color: #333; }

.control-group {
  display: flex;
  align-items: center;
  gap: 5px;
  flex-wrap: wrap;
}

.control-group label {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: #999;
}

.control-group select {
  padding: 3px 5px;
  border: 1px solid #d0d0d0;
  border-radius: 4px;
  font-size: 12px;
  background: #fafafa;
  cursor: pointer;
  outline: none;
}
.control-group select:focus { border-color: #888; }

.scroll {
  overflow: auto;
  flex: 1;
}

table {
  min-width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

thead th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: #f0f0f0;
  padding: 8px 10px;
  text-align: left;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: #888;
  border-bottom: 1px solid #ddd;
  white-space: nowrap;
}

tbody tr {
  border-bottom: 1px solid #ebebeb;
}

tbody tr.selected { background: #e0eaff; }

td {
  padding: 9px 10px;
  color: #222;
  white-space: nowrap;
}

.mono { font-family: monospace; }

.group-header td {
  padding: 6px 10px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #666;
  background: #f4f4f4;
  border-top: 1px solid #ddd;
  border-bottom: 1px solid #ddd;
}

.group-count {
  margin-left: 6px;
  background: #e0e0e0;
  color: #777;
  border-radius: 8px;
  padding: 0 6px;
  font-size: 10px;
  font-weight: 400;
  text-transform: none;
  letter-spacing: 0;
}

.empty {
  padding: 24px;
  text-align: center;
  font-size: 13px;
  color: #aaa;
}
</style>
