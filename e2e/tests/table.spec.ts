import { test, expect, type Page } from '@playwright/test'

const EXPECTED_FLEET_SIZE = 20
const MARKER = '.leaflet-marker-icon'
const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

interface VehicleRecord {
  vin: string
  brand: string
  model: string
  software_version: string
  latitude: number
  longitude: number
}

function dataRows(page: Page) {
  return page.locator('.fleet-table tbody tr:not(.group-header)')
}

function filterGroup(page: Page) {
  return page.locator('.control-group').filter({ has: page.locator('label', { hasText: 'Filter' }) })
}

function groupGroup(page: Page) {
  return page.locator('.control-group').filter({ has: page.locator('label', { hasText: 'Group' }) })
}

test.describe('Fleet table', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE, { timeout: 30_000 })
  })

  // Toggle
  test('is hidden by default', async ({ page }) => {
    await expect(page.locator('.fleet-table')).not.toBeVisible()
  })

  test('toggle button shows the table', async ({ page }) => {
    await page.locator('.toggle-btn').click()
    await expect(page.locator('.fleet-table')).toBeVisible()
  })

  test('toggle button hides an open table', async ({ page }) => {
    const btn = page.locator('.toggle-btn')
    await btn.click()
    await expect(page.locator('.fleet-table')).toBeVisible()
    await btn.click()
    await expect(page.locator('.fleet-table')).not.toBeVisible()
  })

  test.describe('when open', () => {
    test.beforeEach(async ({ page }) => {
      await page.locator('.toggle-btn').click()
      await expect(page.locator('.fleet-table')).toBeVisible()
    })

    // Content
    test('displays all 20 vehicles', async ({ page }) => {
      await expect(dataRows(page)).toHaveCount(EXPECTED_FLEET_SIZE)
    })

    test('highlights the table row of the selected map marker', async ({ page, request }) => {
      const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
      await page.locator(MARKER).first().click({ force: true })
      const selectedRow = page.locator('.fleet-table tbody tr.selected')
      await expect(selectedRow).toBeVisible()
      await expect(selectedRow.locator('td').first()).toHaveText(fleet[0].vin)
    })

    // Search
    test.describe('search', () => {
      test('filters rows by the query text', async ({ page, request }) => {
        const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
        await page.locator('.search').fill(fleet[0].vin)
        await expect(dataRows(page)).toHaveCount(1)
      })

      test('updates the count badge to show filtered / total', async ({ page, request }) => {
        const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
        await page.locator('.search').fill(fleet[0].vin)
        await expect(page.locator('.panel-header .count')).toHaveText(/1\s*\/\s*20/)
      })

      test('shows the empty state when no vehicles match', async ({ page }) => {
        await page.locator('.search').fill('DOES-NOT-EXIST-XYZ')
        await expect(dataRows(page)).toHaveCount(0)
        await expect(page.locator('.fleet-table .empty')).toBeVisible()
      })
    })

    // Filter
    test.describe('filter', () => {
      test('selecting a filter field reveals the value select', async ({ page }) => {
        const fg = filterGroup(page)
        await expect(fg.locator('select').nth(1)).not.toBeAttached()
        await fg.locator('select').first().selectOption('brand')
        await expect(fg.locator('select').nth(1)).toBeVisible()
      })

      test('filtering by a value narrows visible rows', async ({ page, request }) => {
        const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
        const brand = fleet[0].brand
        const expected = fleet.filter(v => v.brand === brand).length

        const fg = filterGroup(page)
        await fg.locator('select').first().selectOption('brand')
        await fg.locator('select').nth(1).selectOption(brand)

        await expect(dataRows(page)).toHaveCount(expected)
      })

      test('changing the filter field resets the selected value', async ({ page }) => {
        const fg = filterGroup(page)
        await fg.locator('select').first().selectOption('brand')
        const valueSelect = fg.locator('select').nth(1)
        const opts = await valueSelect.locator('option').allTextContents()
        if (opts.length > 1) await valueSelect.selectOption({ index: 1 })
        await fg.locator('select').first().selectOption('model')
        await expect(fg.locator('select').nth(1)).toHaveValue('')
      })
    })

    // Group by
    test.describe('group by', () => {
      test('renders group section headers', async ({ page }) => {
        await groupGroup(page).locator('select').selectOption('brand')
        await expect(page.locator('.fleet-table .group-header').first()).toBeVisible()
      })

      test('all vehicles remain visible across groups', async ({ page }) => {
        await groupGroup(page).locator('select').selectOption('brand')
        await expect(dataRows(page)).toHaveCount(EXPECTED_FLEET_SIZE)
      })

      test('group headers display the group name and vehicle count', async ({ page }) => {
        await groupGroup(page).locator('select').selectOption('brand')
        const firstHeader = page.locator('.fleet-table .group-header').first()
        await expect(firstHeader.locator('.group-count')).toBeVisible()
        await expect(firstHeader.locator('.group-count')).toHaveText(/^\d+$/)
      })
    })

    // Reset
    test.describe('reset', () => {
      test('reset button is not visible when no filters are active', async ({ page }) => {
        await expect(page.locator('.reset-btn')).not.toBeVisible()
      })

      test('reset button appears when the search is active', async ({ page }) => {
        await page.locator('.search').fill('VIN')
        await expect(page.locator('.reset-btn')).toBeVisible()
      })

      test('reset button appears when a filter is active', async ({ page }) => {
        await filterGroup(page).locator('select').first().selectOption('brand')
        await expect(page.locator('.reset-btn')).toBeVisible()
      })

      test('reset button appears when a grouping is active', async ({ page }) => {
        await groupGroup(page).locator('select').selectOption('brand')
        await expect(page.locator('.reset-btn')).toBeVisible()
      })

      test('clicking reset clears all filters and hides the button', async ({ page, request }) => {
        const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
        const brand = fleet[0].brand

        const fg = filterGroup(page)
        const gg = groupGroup(page)

        await page.locator('.search').fill('VIN')
        await fg.locator('select').first().selectOption('brand')
        await fg.locator('select').nth(1).selectOption(brand)
        await gg.locator('select').selectOption('brand')

        await page.locator('.reset-btn').click()

        await expect(page.locator('.search')).toHaveValue('')
        await expect(fg.locator('select').first()).toHaveValue('none')
        await expect(gg.locator('select')).toHaveValue('none')
        await expect(dataRows(page)).toHaveCount(EXPECTED_FLEET_SIZE)
        await expect(page.locator('.reset-btn')).not.toBeVisible()
      })
    })
  })
})
