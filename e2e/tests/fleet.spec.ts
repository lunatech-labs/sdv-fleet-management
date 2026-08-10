// SPDX-FileCopyrightText: 2026 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

import { test, expect, type Locator } from '@playwright/test'
import { fetchFleetSize } from './fleet-size'

let EXPECTED_FLEET_SIZE = 0
const MARKER = '.leaflet-marker-icon'
const DRAWER = '.drawer'
const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

interface VehicleRecord {
  vin: string
  brand: string
  model: string
  latitude: number
  longitude: number
}

const MAP_CENTER = { lat: 48.8566, lon: 2.3522 }

function indexClosestToCenter(fleet: VehicleRecord[]): number {
  let best = 0
  let bestDist = Infinity
  for (let i = 0; i < fleet.length; i++) {
    const dLat = fleet[i].latitude - MAP_CENTER.lat
    const dLon = fleet[i].longitude - MAP_CENTER.lon
    const dist = dLat * dLat + dLon * dLon
    if (dist < bestDist) { bestDist = dist; best = i }
  }
  return best
}

async function transform(marker: Locator): Promise<string> {
  return marker.evaluate(el => (el as HTMLElement).style.transform)
}

test.beforeAll(async () => {
  EXPECTED_FLEET_SIZE = await fetchFleetSize()
})

test.describe('SDV fleet map', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE, { timeout: 30_000 })
  })

  test('renders a marker for every vehicle in the fleet', async ({ page }) => {
    const markers = page.locator(MARKER)
    await expect(markers).toHaveCount(EXPECTED_FLEET_SIZE)
    for (let i = 0; i < EXPECTED_FLEET_SIZE; i++) {
      await expect(markers.nth(i)).toBeVisible()
    }
  })

  test('clicking a marker opens the drawer with the matching VIN', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    expect(fleet).toHaveLength(EXPECTED_FLEET_SIZE)

    // Markers are rendered in /fleet response order because App.vue inserts
    // vehicles into a reactive record keyed by VIN in that same order.
    // Pick the vehicle closest to map center to guarantee it's in the viewport.
    const idx = indexClosestToCenter(fleet)
    const expected = fleet[idx]

    await page.locator(MARKER).nth(idx).click({ force: true })

    const drawer = page.locator(DRAWER)
    await expect(drawer).toBeVisible()
    await expect(drawer.getByRole('heading')).toHaveText(`${expected.brand} ${expected.model}`)
    await expect(drawer.getByText(expected.vin, { exact: true })).toBeVisible()
  })

  test('marker position changes within 3 seconds', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const idx = indexClosestToCenter(fleet)
    const marker = page.locator(MARKER).nth(idx)
    const initial = await transform(marker)

    await expect
      .poll(() => transform(marker), {
        message: 'marker transform should change as MQTT position updates arrive',
        timeout: 3_000,
        intervals: [100, 150, 200],
      })
      .not.toBe(initial)
  })
})
