#!/bin/sh
# SPDX-FileCopyrightText: 2026 Contributors to the Eclipse Foundation
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0
#
# Write the runtime configuration the operator UI reads on startup. The nginx
# image runs every executable in /docker-entrypoint.d before starting the
# server, so this lands before the first request is served. Keeping the backend
# URL out of the bundle means one image works in any deployment.

set -eu

BACKEND_URL="${BACKEND_URL:-http://localhost:3000}"

cat >/usr/share/nginx/html/config.js <<EOF
window.__FLEET_CONFIG__ = { backendUrl: "${BACKEND_URL}" };
EOF

echo "fleet-config: backendUrl=${BACKEND_URL}"
