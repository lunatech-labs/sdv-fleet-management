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

// Record the walkthrough video that DEMO.md links.
//
// Playwright writes webm. The runner converts it to docs/screenshots/demo.mp4,
// because that is the path DEMO.md already references and mp4 plays in more
// places than webm.
//
// Pauses are deliberate. The file is watched by a person, not asserted on.

import { test, expect } from '@playwright/test'

const MARKER = '.leaflet-marker-icon'
const TILE = '.leaflet-tile-loaded'
const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

const BEAT = 1800

test.use({
  video: { mode: 'on', size: { width: 1280, height: 800 } },
  viewport: { width: 1280, height: 800 },
})

test('record the demo walkthrough', async ({ page }) => {
  const fleet = (await (await fetch(`${BACKEND_URL}/fleet`)).json()) as unknown[]

  // 1. The live map.
  await page.goto('/')
  await expect(page.locator(MARKER)).toHaveCount(fleet.length)
  await expect(page.locator(TILE).first()).toBeVisible()
  await page.waitForTimeout(BEAT * 2)

  // 2. The vehicle drawer.
  await page.locator(MARKER).first().click()
  await expect(page.locator('.drawer')).toBeVisible()
  await page.waitForTimeout(BEAT * 2)
  await page.locator('.drawer .close').click()

  // 3. The fleet table.
  await page.locator('.toggle-table').click()
  await expect(page.locator('.fleet-table')).toBeVisible()
  await page.waitForTimeout(BEAT * 2)

  // 4. The campaign panel, beside the table.
  await page.locator('.toggle-campaigns').click()
  await expect(page.locator('.campaign-panel')).toBeVisible()
  await page.waitForTimeout(BEAT)
  await page.locator('.toggle-table').click()
  await page.waitForTimeout(BEAT)

  // 5. Launch, then watch the rollout to the end.
  await page.locator('.campaign-panel .launch-btn').click()
  const card = page.locator('.campaign-panel .card').first()
  await expect(card).toBeVisible({ timeout: 20_000 })
  await expect(card.locator('text=/COMPLETE|FAILED/')).toHaveCount(fleet.length, {
    timeout: 90_000,
  })
  await page.waitForTimeout(BEAT)

  // 6. Back to the map, to show the pins settled on their final color.
  await page.locator('.toggle-campaigns').click()
  await page.waitForTimeout(BEAT * 2)
})
