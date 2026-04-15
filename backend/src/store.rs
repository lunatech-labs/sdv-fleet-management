use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;

use crate::models::VehicleRecord;

/// Thread-safe in-memory store for all vehicle records.
#[derive(Clone)]
pub struct Store(Arc<DashMap<String, VehicleRecord>>);

impl Store {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    pub fn insert(&self, record: VehicleRecord) {
        self.0.insert(record.vin.clone(), record);
    }

    pub fn get(&self, vin: &str) -> Option<VehicleRecord> {
        self.0.get(vin).map(|r| r.clone())
    }

    pub fn all(&self) -> Vec<VehicleRecord> {
        self.0.iter().map(|r| r.clone()).collect()
    }

    /// Update a string field on an existing record; no-op if VIN is unknown.
    pub fn update_string(&self, vin: &str, f: impl FnOnce(&mut VehicleRecord)) {
        if let Some(mut record) = self.0.get_mut(vin) {
            f(&mut record);
            record.last_seen = Utc::now();
        }
    }

    /// Update lat or lon; returns the new (lat, lon) if the record exists.
    pub fn update_position(
        &self,
        vin: &str,
        lat: Option<f64>,
        lon: Option<f64>,
    ) -> Option<(f64, f64)> {
        let mut record = self.0.get_mut(vin)?;
        if let Some(v) = lat { record.latitude  = v; }
        if let Some(v) = lon { record.longitude = v; }
        record.last_seen = Utc::now();
        Some((record.latitude, record.longitude))
    }
}
