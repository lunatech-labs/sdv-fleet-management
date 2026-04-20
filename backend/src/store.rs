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
        if let Some(v) = lat {
            record.latitude = v;
        }
        if let Some(v) = lon {
            record.longitude = v;
        }
        record.last_seen = Utc::now();
        Some((record.latitude, record.longitude))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::Store;
    use crate::models::VehicleRecord;

    fn make_record(vin: &str) -> VehicleRecord {
        VehicleRecord {
            vin: vin.to_string(),
            brand: "Acme".to_string(),
            model: "X1".to_string(),
            software_version: "1.0.0".to_string(),
            latitude: 0.0,
            longitude: 0.0,
            last_seen: Utc::now(),
        }
    }

    #[test]
    fn update_position_unknown_vin_returns_none() {
        let store = Store::new();
        assert!(store.update_position("UNKNOWN-VIN", Some(1.0), Some(2.0)).is_none());
    }

    #[test]
    fn update_position_known_vin_updates_last_seen() {
        let store = Store::new();
        let mut record = make_record("VIN-0001");
        let before = Utc::now();
        record.last_seen = before;
        store.insert(record);

        store.update_position("VIN-0001", Some(48.8), Some(2.3));

        let updated = store.get("VIN-0001").unwrap();
        assert!(updated.last_seen > before);
    }
}
