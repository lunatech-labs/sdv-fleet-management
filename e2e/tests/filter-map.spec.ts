import { test, expect, type Page } from '@playwright/test'

const EXPECTED_FLEET_SIZE = 20
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

test.describe('map reflects table filters', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE, { timeout: 30_000 })
    await page.locator('.toggle-btn').click()
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

    await page.locator('.toggle-btn').click()
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
