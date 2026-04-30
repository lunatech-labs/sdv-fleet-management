import { test, expect, type Page, type Locator } from '@playwright/test'

const EXPECTED_FLEET_SIZE = 20
const MARKER = '.leaflet-marker-icon'
const PANEL = '.campaign-panel'
const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

// Default marker pin colour from MapView.vue PINS.default — anything else means
// the vehicle picked up a campaign state.
const DEFAULT_PIN_FILL = '#2a81cb'
const CAMPAIGN_FILLS = ['#9e9e9e', '#2a66d4', '#ff9800', '#2e8b57', '#b00020']

const MAP_CENTER = { lat: 48.8566, lon: 2.3522 }

interface VehicleRecord {
  vin: string
  brand: string
  model: string
  software_version: string
  latitude: number
  longitude: number
}

interface VersionsResponse { versions: string[] }

interface VehicleUpdateState {
  state: 'PENDING' | 'DOWNLOADING' | 'INSTALLING' | 'COMPLETE' | 'FAILED'
  version?: string
  error?: string
}

interface Campaign {
  id: string
  version: string
  vehicles: Record<string, VehicleUpdateState>
  created: string
}

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

function versionField(page: Page): Locator {
  return page.locator(`${PANEL} .field`).filter({ has: page.locator('label', { hasText: 'Version' }) })
}

function vehicleField(page: Page): Locator {
  return page.locator(`${PANEL} .field`).filter({ has: page.locator('label', { hasText: 'Vehicles' }) })
}

function vinCheckbox(page: Page, vin: string): Locator {
  return vehicleField(page)
    .locator('.vin-row')
    .filter({ hasText: vin })
    .locator('input[type="checkbox"]')
}

async function pinFill(marker: Locator): Promise<string | null> {
  return marker.locator('svg path').first().getAttribute('fill')
}

test.describe('Campaign panel', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.locator(MARKER)).toHaveCount(EXPECTED_FLEET_SIZE, { timeout: 30_000 })
  })

  // ── Toggle ────────────────────────────────────────────────────────────────
  test('panel is hidden by default', async ({ page }) => {
    await expect(page.locator(PANEL)).not.toBeVisible()
  })

  test('toggle button shows the panel', async ({ page }) => {
    await page.locator('.toggle-campaigns').click()
    await expect(page.locator(PANEL)).toBeVisible()
  })

  test('toggle button hides an open panel', async ({ page }) => {
    const btn = page.locator('.toggle-campaigns')
    await btn.click()
    await expect(page.locator(PANEL)).toBeVisible()
    await btn.click()
    await expect(page.locator(PANEL)).not.toBeVisible()
  })

  // ── When open ─────────────────────────────────────────────────────────────
  test.describe('when open', () => {
    test.beforeEach(async ({ page }) => {
      await page.locator('.toggle-campaigns').click()
      await expect(page.locator(PANEL)).toBeVisible()
    })

    test('header renders the title and a count badge', async ({ page }) => {
      const header = page.locator(`${PANEL} .panel-header`)
      await expect(header).toContainText('Campaigns')
      await expect(header.locator('.count')).toBeVisible()
      await expect(header.locator('.count')).toHaveText(/^\d+$/)
    })

    test('version dropdown is populated from GET /versions', async ({ page, request }) => {
      const versions = (await request.get(`${BACKEND_URL}/versions`).then(r => r.json())) as VersionsResponse
      const select = versionField(page).locator('select')
      // option list is fetched async on mount, give it a beat to populate
      await expect.poll(() => select.locator('option').count(), { timeout: 5_000 })
        .toBeGreaterThanOrEqual(versions.versions.length)
      const options = await select.locator('option').allTextContents()
      for (const v of versions.versions) {
        expect(options).toContain(v)
      }
    })

    test('vehicle list contains every fleet VIN', async ({ page }) => {
      const rows = vehicleField(page).locator('.vin-row')
      await expect(rows).toHaveCount(EXPECTED_FLEET_SIZE)
    })

    test('all vehicles are preselected on mount', async ({ page }) => {
      const checkboxes = vehicleField(page).locator('input[type="checkbox"]')
      await expect(checkboxes).toHaveCount(EXPECTED_FLEET_SIZE)
      const checked = await checkboxes.evaluateAll(els =>
        els.filter(e => (e as HTMLInputElement).checked).length,
      )
      expect(checked).toBe(EXPECTED_FLEET_SIZE)
    })

    test('Clear unselects every vehicle, All restores them', async ({ page }) => {
      const link = vehicleField(page).locator('.link')
      const checkboxes = vehicleField(page).locator('input[type="checkbox"]')
      const checkedCount = async () => checkboxes.evaluateAll(els =>
        els.filter(e => (e as HTMLInputElement).checked).length,
      )

      await expect(link).toHaveText('Clear')
      await link.click()
      await expect(link).toHaveText('All')
      expect(await checkedCount()).toBe(0)

      await link.click()
      await expect(link).toHaveText('Clear')
      expect(await checkedCount()).toBe(EXPECTED_FLEET_SIZE)
    })

    test('Launch button shows the selected vehicle count', async ({ page }) => {
      await expect(page.locator('.launch-btn')).toContainText(`Launch (${EXPECTED_FLEET_SIZE})`)
      const firstCheckbox = vehicleField(page).locator('input[type="checkbox"]').first()
      await firstCheckbox.click()
      await expect(page.locator('.launch-btn')).toContainText(`Launch (${EXPECTED_FLEET_SIZE - 1})`)
    })

    test('Launch button is disabled when no vehicle is selected', async ({ page }) => {
      await vehicleField(page).locator('.link').click() // Clear
      await expect(page.locator('.launch-btn')).toBeDisabled()
    })

    // ── UI launch flow ─────────────────────────────────────────────────────
    test('clicking Launch creates a campaign card with a PENDING chip', async ({ page, request }) => {
      const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
      const targetVin = fleet[0].vin

      // Restrict the campaign to a single VIN to keep the assertion tight.
      await vehicleField(page).locator('.link').click() // Clear
      await vinCheckbox(page, targetVin).click()
      await expect(page.locator('.launch-btn')).toContainText('Launch (1)')

      const versions = (await request.get(`${BACKEND_URL}/versions`).then(r => r.json())) as VersionsResponse
      const version = versions.versions[0]
      await versionField(page).locator('select').selectOption(version)

      const cardsBefore = await page.locator(`${PANEL} .card`).count()
      await page.locator('.launch-btn').click()

      // A new card appears (regardless of any cards left over from prior tests).
      await expect(page.locator(`${PANEL} .card`)).toHaveCount(cardsBefore + 1)

      const newCard = page.locator(`${PANEL} .card`).first()
      await expect(newCard.locator('.version')).toHaveText(version)

      const row = newCard.locator('.vehicles li').filter({ hasText: targetVin })
      await expect(row).toBeVisible()
      // PENDING is the initial state the backend seeds. Even if HawkBit polling
      // has already advanced it by the time we assert, the chip class is still
      // one of the campaign palette — never the default blue.
      await expect(row.locator('.chip')).toHaveClass(/chip-(pending|downloading|installing|complete|failed)/)
    })

    test('header count increments after launch', async ({ page, request }) => {
      const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
      const targetVin = fleet[0].vin

      const countBefore = parseInt(await page.locator(`${PANEL} .panel-header .count`).innerText(), 10)

      await vehicleField(page).locator('.link').click() // Clear
      await vinCheckbox(page, targetVin).click()
      await page.locator('.launch-btn').click()

      await expect(page.locator(`${PANEL} .panel-header .count`)).toHaveText(String(countBefore + 1))
    })
  })

  // ── Marker color reflects rollout state ─────────────────────────────────
  test('marker pin color switches to a campaign palette colour after launch', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const idx = indexClosestToCenter(fleet)
    const targetVin = fleet[idx].vin
    const marker = page.locator(MARKER).nth(idx)

    await page.locator('.toggle-campaigns').click()
    await expect(page.locator(PANEL)).toBeVisible()

    await vehicleField(page).locator('.link').click() // Clear
    await vinCheckbox(page, targetVin).click()
    await page.locator('.launch-btn').click()

    await expect.poll(() => pinFill(marker), {
      message: 'marker fill should switch to a campaign palette colour after launch',
      timeout: 10_000,
    }).not.toBe(DEFAULT_PIN_FILL)

    const fill = await pinFill(marker)
    expect(CAMPAIGN_FILLS).toContain(fill)
  })

  // ── Vehicle drawer reflects update state ────────────────────────────────
  test('drawer shows the Update chip for a vehicle in an active campaign', async ({ page, request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const idx = indexClosestToCenter(fleet)
    const targetVin = fleet[idx].vin

    await page.locator('.toggle-campaigns').click()
    await vehicleField(page).locator('.link').click() // Clear
    await vinCheckbox(page, targetVin).click()
    await page.locator('.launch-btn').click()

    // Close the panel first so the marker click area isn't covered.
    await page.locator('.toggle-campaigns').click()
    await expect(page.locator(PANEL)).not.toBeVisible()

    await page.locator(MARKER).nth(idx).click({ force: true })
    const drawer = page.locator('.drawer')
    await expect(drawer).toBeVisible()
    await expect(drawer.getByText('Update', { exact: true })).toBeVisible()
    await expect(drawer.locator('.chip')).toHaveClass(/chip-(pending|downloading|installing|complete|failed)/)
  })
})

// ── Backend API ─────────────────────────────────────────────────────────────
test.describe('Campaigns REST API', () => {
  test('GET /versions returns the seeded distribution-set versions', async ({ request }) => {
    const res = await request.get(`${BACKEND_URL}/versions`)
    expect(res.status()).toBe(200)
    const body = (await res.json()) as VersionsResponse
    expect(Array.isArray(body.versions)).toBe(true)
    expect(body.versions.length).toBeGreaterThan(0)
  })

  test('GET /campaigns returns an array', async ({ request }) => {
    const res = await request.get(`${BACKEND_URL}/campaigns`)
    expect(res.status()).toBe(200)
    const body = await res.json()
    expect(Array.isArray(body)).toBe(true)
  })

  test('POST /campaigns rejects an empty vins list', async ({ request }) => {
    const versions = (await request.get(`${BACKEND_URL}/versions`).then(r => r.json())) as VersionsResponse
    const res = await request.post(`${BACKEND_URL}/campaigns`, {
      data: { version: versions.versions[0], vins: [] },
    })
    expect(res.status()).toBe(400)
    const body = await res.json()
    expect(body.error).toBeTruthy()
  })

  test('POST /campaigns rejects an unknown VIN', async ({ request }) => {
    const versions = (await request.get(`${BACKEND_URL}/versions`).then(r => r.json())) as VersionsResponse
    const res = await request.post(`${BACKEND_URL}/campaigns`, {
      data: { version: versions.versions[0], vins: ['VIN-DOES-NOT-EXIST'] },
    })
    expect(res.status()).toBe(400)
    const body = await res.json()
    expect(body.error).toMatch(/unknown VIN/i)
  })

  test('POST /campaigns rejects an unknown version', async ({ request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const res = await request.post(`${BACKEND_URL}/campaigns`, {
      data: { version: 'nonexistent-version-xyz', vins: [fleet[0].vin] },
    })
    expect(res.status()).toBe(400)
    const body = await res.json()
    expect(body.error).toMatch(/unknown version/i)
  })

  test('POST /campaigns returns a campaign in PENDING for valid input', async ({ request }) => {
    const fleet: VehicleRecord[] = await request.get(`${BACKEND_URL}/fleet`).then(r => r.json())
    const versions = (await request.get(`${BACKEND_URL}/versions`).then(r => r.json())) as VersionsResponse

    const res = await request.post(`${BACKEND_URL}/campaigns`, {
      data: { version: versions.versions[0], vins: [fleet[0].vin] },
    })
    expect(res.status()).toBe(200)
    const c = (await res.json()) as Campaign
    expect(c.id).toMatch(/^[0-9a-f-]{36}$/)
    expect(c.version).toBe(versions.versions[0])
    expect(c.vehicles[fleet[0].vin]).toBeTruthy()
    expect(c.vehicles[fleet[0].vin].state).toBe('PENDING')

    // The new campaign is reachable via GET /campaigns/{id} too.
    const fetched = await request.get(`${BACKEND_URL}/campaigns/${c.id}`)
    expect(fetched.status()).toBe(200)
    const fetchedBody = (await fetched.json()) as Campaign
    expect(fetchedBody.id).toBe(c.id)
  })

  test('GET /campaigns/{id} returns 404 for an unknown id', async ({ request }) => {
    const res = await request.get(`${BACKEND_URL}/campaigns/00000000-0000-0000-0000-000000000000`)
    expect(res.status()).toBe(404)
  })
})
