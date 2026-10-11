//! Installation-owned non-Agent authentication records. Reuse the preserved
//! key/value table without changing Agent schema, lineage or content.
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use nomifun_api_types::ConnectedDevice;
use nomifun_common::AppError;
use nomifun_db::IClientPreferenceRepository;
use serde::{Deserialize, Serialize};

pub const DEVICE_AUTH_PREFERENCE_KEY: &str = "auth.devices.v1";
const MAX_DEVICES: usize = 128;
const ACTIVITY_PERSIST_INTERVAL_MS: i64 = 60_000;

#[derive(Clone, Serialize, Deserialize)]
struct DeviceRecord {
    user_id: String,
    #[serde(flatten)]
    device: ConnectedDevice,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{SqliteClientPreferenceRepository, init_database_memory};

    #[tokio::test]
    async fn device_revocation_survives_restart_and_agent_reset() {
        let database = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteClientPreferenceRepository::new(database.pool().clone()));
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let store = Arc::new(DeviceStore::load(repo.clone()).await.unwrap());
        let a = store.register(owner.as_str(), "Phone A").await.unwrap();
        let b = store.register(owner.as_str(), "Phone B").await.unwrap();
        let jwt = crate::JwtService::with_devices("persistent-secret".into(), store.clone());
        let a_token = jwt.sign_for_device(owner.as_str(), "admin", &a.device_id).unwrap();
        let b_token = jwt.sign_for_device(owner.as_str(), "admin", &b.device_id).unwrap();
        store.revoke(&a.device_id, owner.as_str()).await.unwrap();
        assert!(jwt.verify(&a_token).is_err());
        assert!(jwt.verify(&b_token).is_ok());
        nomifun_db::reset_agent_data(database.pool()).await.unwrap();
        let restarted = Arc::new(DeviceStore::load(repo.clone()).await.unwrap());
        let jwt = crate::JwtService::with_devices("persistent-secret".into(), restarted.clone());
        assert!(jwt.verify(&a_token).is_err());
        assert!(jwt.verify(&b_token).is_ok());
        assert_eq!(restarted.list(owner.as_str()).unwrap().len(), 2);
        let value = &repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap()[0].value;
        assert!(!value.contains(&a_token));
        assert!(!value.contains(&b_token));
    }

    #[tokio::test]
    async fn concurrent_registration_preserves_both_records_and_corrupt_state_fails_closed() {
        let database = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteClientPreferenceRepository::new(database.pool().clone()));
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let store = DeviceStore::load(repo.clone()).await.unwrap();
        let (a, b) = tokio::join!(store.register(owner.as_str(), "Phone A"), store.register(owner.as_str(), "Phone B"));
        let a = a.unwrap(); let b = b.unwrap();
        assert_ne!(a.device_id, b.device_id);
        assert_eq!(DeviceStore::load(repo.clone()).await.unwrap().list(owner.as_str()).unwrap().len(), 2);
        repo.upsert_batch(&[(DEVICE_AUTH_PREFERENCE_KEY, "{bad-json}")]).await.unwrap();
        let corrupted = DeviceStore::load(repo.clone()).await.unwrap();
        assert!(corrupted.list(owner.as_str()).unwrap().is_empty());
        assert!(!corrupted.is_active(&a.device_id, owner.as_str()));
        let fresh = corrupted.register(owner.as_str(), "Phone C").await.unwrap();
        let stored = &repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap()[0].value;
        assert!(serde_json::from_str::<serde_json::Value>(stored).is_ok());
        let reloaded = DeviceStore::load(repo.clone()).await.unwrap().list(owner.as_str()).unwrap();
        assert_eq!([fresh.device_id.as_str()], reloaded.iter().map(|d| d.device_id.as_str()).collect::<Vec<_>>().as_slice());
        // A partially written record must never turn a revoked device active
        // through serde's implicit default for an absent nullable field.
        let partial = serde_json::json!({a.device_id.clone(): {
            "user_id":owner.as_str(), "device_id":a.device_id,
            "device_name":"Phone A", "created_at_ms":1, "last_seen_at_ms":1
        }}).to_string();
        repo.upsert_batch(&[(DEVICE_AUTH_PREFERENCE_KEY, &partial)]).await.unwrap();
        let corrupted = DeviceStore::load(repo.clone()).await.unwrap();
        assert!(corrupted.list(owner.as_str()).unwrap().is_empty());
        assert!(!corrupted.is_active(&a.device_id, owner.as_str()));
        let fresh = corrupted.register(owner.as_str(), "Phone D").await.unwrap();
        let stored = &repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap()[0].value;
        assert!(serde_json::from_str::<serde_json::Value>(stored).is_ok());
        let reloaded = DeviceStore::load(repo).await.unwrap().list(owner.as_str()).unwrap();
        assert_eq!([fresh.device_id.as_str()], reloaded.iter().map(|d| d.device_id.as_str()).collect::<Vec<_>>().as_slice());
    }

    #[tokio::test]
    async fn revoke_all_marks_only_that_owners_active_devices_once() {
        let database = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteClientPreferenceRepository::new(database.pool().clone()));
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let other = nomifun_common::UserId::new();
        let store = DeviceStore::load(repo.clone()).await.unwrap();
        let a = store.register(owner.as_str(), "Phone A").await.unwrap();
        let b = store.register(owner.as_str(), "Phone B").await.unwrap();
        let foreign = store.register(other.as_str(), "Phone C").await.unwrap();
        store.revoke(&a.device_id, owner.as_str()).await.unwrap();
        assert_eq!(store.revoke_all(owner.as_str()).await.unwrap(), 1);
        assert!(store.is_active(&foreign.device_id, other.as_str()));
        assert_eq!(store.revoke_all(owner.as_str()).await.unwrap(), 0);
        let reloaded = DeviceStore::load(repo).await.unwrap();
        assert!(reloaded.list(owner.as_str()).unwrap().iter().all(|d| d.revoked_at_ms.is_some()));
        assert!(reloaded.is_active(&foreign.device_id, other.as_str()));
        assert!(!reloaded.is_active(&b.device_id, owner.as_str()));
    }
}

fn decode_records(raw: &str) -> Result<BTreeMap<String, DeviceRecord>, AppError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|_| AppError::Internal("Device authentication records are invalid".into()))?;
    if !value.as_object().is_some_and(|records| records.values().all(|record| {
        record.as_object().is_some_and(|record| record.contains_key("revoked_at_ms"))
    })) {
        return Err(AppError::Internal("Device authentication records are incomplete".into()));
    }
    let records: BTreeMap<String, DeviceRecord> = serde_json::from_value(value)
        .map_err(|_| AppError::Internal("Device authentication records are invalid".into()))?;
    if records.len() > MAX_DEVICES || records.iter().any(|(id, record)| {
        id != &record.device.device_id || id.len() != 64
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
            || nomifun_common::UserId::parse(record.user_id.clone()).is_err()
            || record.device.device_name.is_empty()
    }) {
        return Err(AppError::Internal("Device authentication records are invalid".into()));
    }
    Ok(records)
}

/// One shared process owner serializes persistent mutations. The database's
/// startup lock prevents a second writable host for the same dataset.
pub struct DeviceStore {
    records: RwLock<BTreeMap<String, DeviceRecord>>,
    mutation: tokio::sync::Mutex<()>,
    repo: Option<Arc<dyn IClientPreferenceRepository>>,
}

impl Default for DeviceStore {
    fn default() -> Self {
        Self {
            records: RwLock::new(BTreeMap::new()),
            mutation: tokio::sync::Mutex::new(()),
            repo: None,
        }
    }
}

impl DeviceStore {
    /// Only the repository read may fail startup. Unreadable stored records
    /// decode to an empty map instead: every previously issued device token is
    /// inactive (fail closed) and the next successful `register` overwrites the
    /// unreadable value with a valid one.
    pub async fn load(repo: Arc<dyn IClientPreferenceRepository>) -> Result<Self, AppError> {
        let rows = repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await?;
        let records: BTreeMap<String, DeviceRecord> = match rows.first() {
            Some(row) => match decode_records(&row.value) {
                Ok(records) => records,
                Err(error) => {
                    tracing::error!(%error, "stored device authentication records are unreadable; starting empty");
                    BTreeMap::new()
                }
            },
            None => BTreeMap::new(),
        };
        Ok(Self { records: RwLock::new(records), mutation: tokio::sync::Mutex::new(()), repo: Some(repo) })
    }

    pub fn is_active(&self, device_id: &str, user_id: &str) -> bool {
        self.records.read().ok().and_then(|records| records.get(device_id).cloned())
            .is_some_and(|record| record.user_id == user_id && record.device.revoked_at_ms.is_none())
    }

    pub fn list(&self, user_id: &str) -> Result<Vec<ConnectedDevice>, AppError> {
        Ok(self.snapshot()?.into_values().filter(|r| r.user_id == user_id).map(|r| r.device).collect())
    }

    pub async fn register(&self, user_id: &str, name: &str) -> Result<ConnectedDevice, AppError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(AppError::BadRequest("Device name must contain 1 to 80 visible characters".into()));
        }
        let _mutation = self.mutation.lock().await;
        let mut records = self.snapshot()?;
        // Revoked device records are not an authority for any future credential.
        // They may be pruned when adding a device to keep storage bounded.
        if records.len() >= MAX_DEVICES {
            records.retain(|_, record| record.device.revoked_at_ms.is_none());
        }
        if records.len() >= MAX_DEVICES {
            return Err(AppError::Conflict("Remove a connected device before adding another".into()));
        }
        let now = nomifun_common::now_ms();
        let device = ConnectedDevice {
            device_id: crate::generate_random_hex_secret(),
            device_name: name.to_owned(),
            created_at_ms: now,
            last_seen_at_ms: now,
            revoked_at_ms: None,
        };
        records.insert(device.device_id.clone(), DeviceRecord { user_id: user_id.to_owned(), device: device.clone() });
        self.publish(records).await?;
        Ok(device)
    }

    pub async fn revoke(&self, device_id: &str, user_id: &str) -> Result<ConnectedDevice, AppError> {
        let _mutation = self.mutation.lock().await;
        let mut records = self.snapshot()?;
        let record = records.get_mut(device_id).filter(|r| r.user_id == user_id)
            .ok_or_else(|| AppError::NotFound("Connected device not found".into()))?;
        record.device.revoked_at_ms.get_or_insert_with(nomifun_common::now_ms);
        let device = record.device.clone();
        self.publish(records).await?;
        Ok(device)
    }

    /// Revokes every still-active credential of one owner; used when the
    /// account secret rotates so the registry matches the dead tokens.
    pub async fn revoke_all(&self, user_id: &str) -> Result<usize, AppError> {
        let _mutation = self.mutation.lock().await;
        let mut records = self.snapshot()?;
        let now = nomifun_common::now_ms();
        let mut count = 0;
        for record in records.values_mut().filter(|record| record.user_id == user_id && record.device.revoked_at_ms.is_none()) {
            record.device.revoked_at_ms = Some(now);
            count += 1;
        }
        if count > 0 {
            self.publish(records).await?;
        }
        Ok(count)
    }

    pub async fn touch(&self, device_id: &str, user_id: &str) -> Result<(), AppError> {
        let _mutation = self.mutation.lock().await;
        let mut records = self.snapshot()?;
        let Some(record) = records.get_mut(device_id).filter(|r| r.user_id == user_id && r.device.revoked_at_ms.is_none()) else { return Ok(()); };
        let now = nomifun_common::now_ms();
        if now.saturating_sub(record.device.last_seen_at_ms) < ACTIVITY_PERSIST_INTERVAL_MS { return Ok(()); }
        record.device.last_seen_at_ms = now;
        self.publish(records).await
    }

    fn snapshot(&self) -> Result<BTreeMap<String, DeviceRecord>, AppError> {
        self.records.read().map(|r| r.clone()).map_err(|_| AppError::Internal("Device authentication state unavailable".into()))
    }

    async fn publish(&self, records: BTreeMap<String, DeviceRecord>) -> Result<(), AppError> {
        if let Some(repo) = &self.repo {
            let value = serde_json::to_string(&records).map_err(|_| AppError::Internal("Device authentication serialization failed".into()))?;
            repo.upsert_batch(&[(DEVICE_AUTH_PREFERENCE_KEY, &value)]).await?;
        }
        *self.records.write().map_err(|_| AppError::Internal("Device authentication state unavailable".into()))? = records;
        Ok(())
    }
}
