#!/bin/bash
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
# Check third-party licences with the Eclipse Dash tool.
#
# The blueprint script covers Cargo only. This one adds npm, because the
# operator dashboard is the first JavaScript component in this stack. Both
# producers write into one list, which the Dash jar consumes in a single pass.
#
# `tail -n +2` drops the root package line. The blueprint script uses GNU
# `sed -n '2~1p'` for this, which BSD sed rejects, so this runs on macOS too.
#
# Usage:
#   scripts/check-3rd-party-licenses.sh [review-token]
#
# Without a token the run only reports. With a token it also opens Eclipse IP
# review requests for anything unvetted.

set -euo pipefail

deps_file=${DEPS_FILE:-"DEPS.txt"}
dash_jar=${DASH_JAR:-"/tmp/dash.jar"}
dash_summary=${DASH_SUMMARY:-"DASH_SUMMARY.txt"}
dash_url=${DASH_URL:-"https://repo.eclipse.org/service/rest/v1/search/assets/download?sort=version&repository=dash-maven2-releases&maven.groupId=org.eclipse.dash&maven.artifactId=org.eclipse.dash.licenses&maven.extension=jar"}
project=${PROJECT:-"automotive.uprotocol"}
token=${1:-}

repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"

: > "$deps_file"

# ── Cargo ────────────────────────────────────────────────────────────────────
# First-party crates are filtered out. They are not third-party dependencies.
for manifest in backend/Cargo.toml ota-agent/Cargo.toml; do
  if [[ -r "$manifest" ]]; then
    echo "collecting cargo dependencies from ${manifest}..."
    cargo tree --manifest-path "$manifest" -e no-build,no-dev --prefix none --no-dedupe --locked \
      | tail -n +2 \
      | grep -v '^[[:space:]]*$' \
      | grep -v '^backend ' \
      | grep -v '^ota-agent ' \
      | sed -E 's|([^ ]+) v([^ ]+).*|crate/cratesio/-/\1/\2|' \
      >> "$deps_file"
  fi
done

# ── npm ──────────────────────────────────────────────────────────────────────
# Dash coordinates are npm/npmjs/-/<name>/<version>, and npm/npmjs/@scope/<name>/<version>
# for scoped packages. Only the lockfile is read, so no install is required.
for lockfile in frontend/package-lock.json e2e/package-lock.json; do
  if [[ -r "$lockfile" ]]; then
    echo "collecting npm dependencies from ${lockfile}..."
    jq -r '
      (.packages // {})
      | to_entries[]
      | select(.key != "")
      | select(.value.version != null)
      | select((.value.link // false) | not)
      # Match the cargo side, which uses -e no-build,no-dev: only what is
      # distributed needs an Eclipse IP review.
      | select((.value.dev // false) | not)
      | (.key | sub("^.*node_modules/"; "")) as $name
      | if ($name | startswith("@"))
        then "npm/npmjs/" + ($name | split("/")[0]) + "/" + ($name | split("/")[1]) + "/" + .value.version
        else "npm/npmjs/-/" + $name + "/" + .value.version
        end
    ' "$lockfile" >> "$deps_file"
  fi
done

sort -u -o "$deps_file" "$deps_file"
echo "collected $(wc -l < "$deps_file") dependencies into ${deps_file}"

if [[ ! -r "$dash_jar" ]]; then
  echo "Eclipse Dash JAR file [${dash_jar}] not found, downloading latest version from Eclipse repo..."
  if ! command -v wget >/dev/null 2>&1; then
    echo "wget command not available on path"
    exit 127
  fi
  wget --quiet -O "$dash_jar" "$dash_url"
  echo "successfully downloaded Eclipse Dash JAR to ${dash_jar}"
fi

if ! jar tf "$dash_jar" >/dev/null 2>&1; then
  echo "Downloaded Eclipse Dash file [${dash_jar}] is not a valid JAR"
  exit 1
fi

args=(-jar "$dash_jar" -timeout 60 -batch 90 -summary "$dash_summary")
if [[ -n "$token" ]]; then
  args=("${args[@]}" -review -token "$token" -project "$project")
fi
args=("${args[@]}" "$deps_file")

echo "checking 3rd party licenses..."
java "${args[@]}"
