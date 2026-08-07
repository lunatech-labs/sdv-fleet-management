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

import { test, expect, type Page } from '@playwright/test'
import { fetchFleetSize } from './fleet-size'

let EXPECTED_FLEET_SIZE = 0
const MARKER = '.leaflet-marker-icon'
const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

interface VehicleRecord {
  vin: string
  brand: string
  model: string
  software_version: string
}

function filterGroup(page: Page) {
  return page.locator('.control-group').filter({ has: page.locator('label', { hasText: 'Filter' }) })
}

test.beforeAll(async () => {
  EXPECTED_FLEET_SIZE = await fetchFleetSize()
})

test.describe('map reflects table filters', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE, { timeout: 30_000 })
    await page.locator('.toggle-table').click()
    await expect(page.locator('.fleet-table')).toBeVisible()
  })

  test('brand filter reduces visible map markers', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const brand = fleet[0].brand
    const expected = fleet.filter(v => v.brand === brand).length

    const fg = filterGroup(page)
    await fg.locator('select').first().selectOption('brand')
    await fg.locator('select').nth(1).selectOption(brand)

    await expect(page.locator(MARKER)).toHaveCount(expected)
  })

  test('search query reduces visible map markers', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const brand = fleet[0].brand
    const expected = fleet.filter(v => v.brand.toLowerCase().includes(brand.toLowerCase())).length

    await page.locator('.search').fill(brand)

    await expect(page.locator(MARKER)).toHaveCount(expected)
  })

  test('reset button restores all markers on the map', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const brand = fleet[0].brand

    const fg = filterGroup(page)
    await fg.locator('select').first().selectOption('brand')
    await fg.locator('select').nth(1).selectOption(brand)
    await expect(page.locator(MARKER)).toHaveCount(fleet.filter(v => v.brand === brand).length)

    await page.locator('.reset-btn').click()
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE)
  })

  test('hiding the table restores all markers on the map', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const brand = fleet[0].brand

    const fg = filterGroup(page)
    await fg.locator('select').first().selectOption('brand')
    await fg.locator('select').nth(1).selectOption(brand)
    await expect(page.locator(MARKER)).toHaveCount(fleet.filter(v => v.brand === brand).length)

    await page.locator('.toggle-table').click()
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE)
  })

  test('group by does not reduce map markers', async ({ page }) => {
    const groupSelect = page
      .locator('.control-group')
      .filter({ has: page.locator('label', { hasText: 'Group' }) })
      .locator('select')

    await groupSelect.selectOption('brand')
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE)
  })
})
