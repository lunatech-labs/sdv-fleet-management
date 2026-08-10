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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Full vehicle record served by GET /fleet and GET /vehicles/{vin}.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VehicleRecord {
    pub vin: String,
    pub brand: String,
    pub model: String,
    pub software_version: String,
    pub latitude: f64,
    pub longitude: f64,
    pub last_seen: DateTime<Utc>,
}

/// Real-time position update pushed over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionEvent {
    pub vin: String,
    pub lat: f64,
    pub lon: f64,
}

/// Minimal vehicle entry in vehicles.json (used to pre-populate static fields).
#[derive(Debug, Deserialize)]
pub struct VehicleSeed {
    pub vin: String,
    pub brand: String,
    pub model: String,
    pub software_version: String,
    pub latitude: f64,
    pub longitude: f64,
}
