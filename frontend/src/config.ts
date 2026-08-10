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

/**
 * Where the operator UI finds the backend.
 *
 * Resolved at runtime from `/config.js`, which the container entrypoint writes
 * from the `BACKEND_URL` environment variable. Baking the URL in at build time
 * would pin the published image to a single deployment, so the build-time
 * `VITE_BACKEND_URL` is only a fallback for `npm run dev`.
 */
declare global {
  interface Window {
    __FLEET_CONFIG__?: { backendUrl?: string }
  }
}

export const BACKEND: string =
  window.__FLEET_CONFIG__?.backendUrl ||
  (import.meta.env.VITE_BACKEND_URL as string | undefined) ||
  'http://localhost:3000'

/** Same origin as {@link BACKEND}, with the scheme swapped for websockets. */
export const BACKEND_WS: string = BACKEND.replace(/^http/, 'ws')
