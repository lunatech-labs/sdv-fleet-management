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
