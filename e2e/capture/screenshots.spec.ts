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

// Capture the screenshots that DEMO.md embeds.
//
// Run against a fully started stack:
//   cd e2e && npx playwright test --config=capture/playwright.capture.config.ts
//
// The file names match the ones DEMO.md already references, so the markdown
// needs no edit when the images are refreshed.

import { test, expect, type Page } from '@playwright/test'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const OUT = path.resolve(HERE, '../../docs/screenshots')

const MARKER = '.leaflet-marker-icon'
const TILE = '.leaflet-tile-loaded'
const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

async function fleetSize(): Promise<number> {
  const response = await fetch(`${BACKEND_URL}/fleet`)
  const fleet = (await response.json()) as unknown[]
  return fleet.length
}

/** Wait until the map is worth photographing: every pin placed, tiles drawn. */
async function mapReady(page: Page): Promise<number> {
  const expected = await fleetSize()
  await page.goto('/')
  await expect(page.locator(MARKER)).toHaveCount(expected)
  await expect(page.locator(TILE).first()).toBeVisible()
  // Let the tile grid finish drawing. A half-painted basemap looks broken.
  await page.waitForTimeout(2500)
  return expected
}

async function shot(page: Page, name: string): Promise<void> {
  await page.screenshot({ path: path.join(OUT, name) })
}

test('capture the demo screenshots', async ({ page }) => {
  const expected = await mapReady(page)
  expect(expected).toBeGreaterThan(0)

  // 1. The live fleet map.
  await shot(page, 'main.png')

  // 2. The vehicle detail drawer.
  await page.locator(MARKER).first().click()
  await expect(page.locator('.drawer')).toBeVisible()
  await shot(page, 'car_modal.png')
  await page.locator('.drawer .close').click()

  // 3. The fleet table.
  await page.locator('.toggle-table').click()
  await expect(page.locator('.fleet-table')).toBeVisible()
  await shot(page, 'fleet.png')

  // 6. Both panels open. Captured here, while the table is already up, so the
  //    campaign panel is opened once and photographed twice.
  await page.locator('.toggle-campaigns').click()
  await expect(page.locator('.campaign-panel')).toBeVisible()
  await shot(page, 'campaigns+fleet.png')

  // 4. The campaign panel on its own, before launch.
  await page.locator('.toggle-table').click()
  await expect(page.locator('.fleet-table')).toBeHidden()
  await shot(page, 'campaigns.png')

  // 5. A campaign in progress. Every VIN is preselected on mount, so a click on
  //    Launch starts a rollout across the whole fleet. The agents take about
  //    five seconds to download and three to install, so wait until at least one
  //    vehicle has moved off PENDING before photographing.
  await page.locator('.campaign-panel .launch-btn').click()
  await expect(page.locator('.campaign-panel .card')).toHaveCount(1, { timeout: 20_000 })
  await expect(
    page.locator('.campaign-panel .card').first().locator('text=/DOWNLOADING|INSTALLING|COMPLETE/'),
  ).toHaveCount(1, { timeout: 30_000 })
  await shot(page, 'campaign_deployment.png')
})
