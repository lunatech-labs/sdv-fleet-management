export const BACKEND_URL = process.env.PLAYWRIGHT_BACKEND_URL ?? 'http://localhost:3000'

/**
 * How many vehicles the stack is running.
 *
 * Read from the backend rather than hardcoded: the vehicle count is a property
 * of docker-compose.yml and seed/vehicles.json, and a stale constant here turns
 * every count assertion into a 30 second timeout instead of a clear failure.
 */
export async function fetchFleetSize(): Promise<number> {
  const response = await fetch(`${BACKEND_URL}/fleet`)
  if (!response.ok) {
    throw new Error(`GET ${BACKEND_URL}/fleet returned ${response.status}`)
  }
  const fleet = (await response.json()) as unknown[]
  if (fleet.length === 0) {
    throw new Error('backend reported an empty fleet')
  }
  return fleet.length
}
