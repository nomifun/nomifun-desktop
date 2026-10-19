//! Installation-owned non-Agent authentication records. Reuse the preserved
//! key/value table without changing Agent schema, lineage or content.
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use nomifun_api_types::ConnectedDevice;
use nomifun_common::AppError;
use nomifun_db::IClientPreferenceRepository;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEVICE_AUTH_PREFERENCE_KEY: &str = "auth.devices.v1";
const MAX_DEVICES: usize = 128;
const ACTIVITY_PERSIST_INTERVAL_MS: i64 = 60_000;

#[derive(Clone, Serialize, Deserialize)]
struct DeviceRecord {
    user_id: String,
    #[serde(flatten)]
    device: ConnectedDevice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installation_key_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_id: Option<String>,
}

/// The exact credential generation persisted by one registration. Signing must
/// use this generation, rather than reading a newer concurrent registration.
#[derive(Debug, Clone)]
pub struct RegisteredDevice {
    pub device: ConnectedDevice,
    pub credential_id: String,
}

fn valid_hex_secret(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode_records(raw: &str) -> Result<BTreeMap<String, DeviceRecord>, AppError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|_| AppError::Internal("Device authentication records are invalid".into()))?;
    if !value.as_object().is_some_and(|records| records.values().all(|record| {
        record.as_object().is_some_and(|record| {
            record.contains_key("revoked_at_ms")
                && (record.contains_key("installation_key_hash") == record.contains_key("credential_id"))
                && (!record.contains_key("installation_key_hash")
                    || (record["installation_key_hash"].is_string() && record["credential_id"].is_string()))
        })
    })) {
        return Err(AppError::Internal("Device authentication records are incomplete".into()));
    }
    let records: BTreeMap<String, DeviceRecord> = serde_json::from_value(value)
        .map_err(|_| AppError::Internal("Device authentication records are invalid".into()))?;
    let mut identities = std::collections::BTreeSet::new();
    if records.len() > MAX_DEVICES || records.iter().any(|(id, record)| {
        id != &record.device.device_id || id.len() != 64
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
            || nomifun_common::UserId::parse(record.user_id.clone()).is_err()
            || record.device.device_name.is_empty()
            || match (&record.installation_key_hash, &record.credential_id) {
                (None, None) => false,
                (Some(identity), Some(credential)) => !valid_hex_secret(identity)
                    || !valid_hex_secret(credential)
                    || !identities.insert((record.user_id.clone(), identity.clone())),
                _ => true,
            }
    }) {
        return Err(AppError::Internal("Device authentication records are invalid".into()));
    }
    Ok(records)
}

/// One shared process owner serializes persistent mutations. The database's
/// startup lock prevents a second writable host for the same dataset.
#[derive(Clone)]
pub struct DeviceStore {
    records: Arc<RwLock<BTreeMap<String, DeviceRecord>>>,
    mutation: Arc<tokio::sync::Mutex<()>>,
    repo: Option<Arc<dyn IClientPreferenceRepository>>,
    invalidations: tokio::sync::watch::Sender<u64>,
}

impl Default for DeviceStore {
    fn default() -> Self {
        Self::from_records(BTreeMap::new(), None)
    }
}

impl DeviceStore {
    fn from_records(records: BTreeMap<String, DeviceRecord>, repo: Option<Arc<dyn IClientPreferenceRepository>>) -> Self {
        let (invalidations, _) = tokio::sync::watch::channel(0);
        Self {
            records: Arc::new(RwLock::new(records)),
            mutation: Arc::new(tokio::sync::Mutex::new(())),
            repo,
            invalidations,
        }
    }

    pub(crate) fn invalidation_sender(&self) -> tokio::sync::watch::Sender<u64> {
        self.invalidations.clone()
    }

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
        Ok(Self::from_records(records, Some(repo)))
    }

    pub fn list(&self, user_id: &str) -> Result<Vec<ConnectedDevice>, AppError> {
        Ok(self.snapshot()?.into_values().filter(|r| r.user_id == user_id).map(|r| r.device).collect())
    }

    /// Both halves must match: an old token cannot become valid again when a
    /// revoked installation receives a fresh authorization.
    pub fn is_credential_active(&self, device_id: &str, user_id: &str, credential_id: Option<&str>) -> bool {
        self.records.read().ok().is_some_and(|records| records.get(device_id).is_some_and(|record| {
            record.user_id == user_id && record.device.revoked_at_ms.is_none()
                && record.credential_id.as_deref() == credential_id
        }))
    }

    fn validate_installation_key(installation_key: &str) -> Result<(), AppError> {
        if !valid_hex_secret(installation_key) {
            return Err(AppError::BadRequest("Installation key must contain 64 hexadecimal characters".into()));
        }
        Ok(())
    }

    pub fn validate_registration(name: &str, installation_key: &str) -> Result<(), AppError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(AppError::BadRequest("Device name must contain 1 to 80 visible characters".into()));
        }
        Self::validate_installation_key(installation_key)
    }

    pub async fn register(
        &self,
        user_id: &str,
        name: &str,
        installation_key: &str,
        previous_device_id: Option<&str>,
    ) -> Result<RegisteredDevice, AppError> {
        Self::validate_registration(name, installation_key)?;
        let identity = format!("{:x}", Sha256::digest(installation_key.to_ascii_lowercase().as_bytes()));
        let name = name.trim().to_owned();
        let user_id = user_id.to_owned();
        let previous_device_id = previous_device_id.map(str::to_owned);
        self.mutate(true, move |records| {
            // A legacy record may be claimed only after the caller proved that
            // exact active credential. Names and addresses are never identity.
            let previous = if let Some(id) = previous_device_id.as_deref() {
                let record = records.get(id).filter(|record| {
                    record.user_id == user_id && record.device.revoked_at_ms.is_none()
                }).ok_or_else(|| AppError::Forbidden("Previous device credential is no longer valid".into()))?;
                if record.installation_key_hash.as_deref().is_some_and(|known| known != identity) {
                    return Err(AppError::Forbidden("Previous device belongs to another installation".into()));
                }
                Some(id.to_owned())
            } else { None };
            let existing = records.iter().find(|(_, record)| {
                record.user_id == user_id && record.installation_key_hash.as_deref() == Some(identity.as_str())
            }).map(|(id, _)| id.clone());
            let device_id = existing.or_else(|| previous.clone());
            let now = nomifun_common::now_ms();
            let credential_id = crate::generate_random_hex_secret();
            let device = if let Some(id) = device_id {
                let record = records.get_mut(&id).expect("selected device exists in snapshot");
                record.device.device_name = name.to_owned();
                record.device.last_seen_at_ms = now.max(record.device.last_seen_at_ms);
                record.device.revoked_at_ms = None;
                record.installation_key_hash = Some(identity);
                record.credential_id = Some(credential_id.clone());
                record.device.clone()
            } else {
                // Reauthorization remains possible at the cap. Revoked records
                // carry no authority and may be pruned only when adding a device.
                if records.len() >= MAX_DEVICES {
                    records.retain(|_, record| record.device.revoked_at_ms.is_none());
                }
                if records.len() >= MAX_DEVICES {
                    return Err(AppError::Conflict("Remove a connected device before adding another".into()));
                }
                let device = ConnectedDevice {
                    device_id: crate::generate_random_hex_secret(),
                    device_name: name.to_owned(),
                    created_at_ms: now,
                    last_seen_at_ms: now,
                    revoked_at_ms: None,
                };
                records.insert(device.device_id.clone(), DeviceRecord {
                    user_id: user_id.to_owned(), device: device.clone(),
                    installation_key_hash: Some(identity), credential_id: Some(credential_id.clone()),
                });
                device
            };
            // If this installation already had a bound record, retire only the
            // additional legacy credential whose possession was proven explicitly.
            if let Some(previous) = previous.filter(|previous| previous != &device.device_id) {
                records.get_mut(&previous).expect("proven previous device exists").device.revoked_at_ms.get_or_insert(now);
            }
            Ok((RegisteredDevice { device, credential_id }, true))
        }).await
    }

    pub async fn revoke(&self, device_id: &str, user_id: &str) -> Result<ConnectedDevice, AppError> {
        let device_id = device_id.to_owned();
        let user_id = user_id.to_owned();
        self.mutate(true, move |records| {
            let record = records.get_mut(&device_id).filter(|r| r.user_id == user_id)
                .ok_or_else(|| AppError::NotFound("Connected device not found".into()))?;
            let changed = record.device.revoked_at_ms.is_none();
            record.device.revoked_at_ms.get_or_insert_with(nomifun_common::now_ms);
            Ok((record.device.clone(), changed))
        }).await
    }

    /// Self-removal and logout must fence the exact admitted generation so a
    /// stalled old request cannot revoke a later authorization of the device.
    pub async fn revoke_credential(&self, device_id: &str, user_id: &str, credential_id: Option<&str>) -> Result<ConnectedDevice, AppError> {
        let device_id = device_id.to_owned();
        let user_id = user_id.to_owned();
        let credential_id = credential_id.map(str::to_owned);
        self.mutate(true, move |records| {
            let record = records.get_mut(&device_id).filter(|record| {
                record.user_id == user_id && record.device.revoked_at_ms.is_none()
                    && record.credential_id == credential_id
            }).ok_or_else(|| AppError::Forbidden("Device credential revoked".into()))?;
            record.device.revoked_at_ms = Some(nomifun_common::now_ms());
            Ok((record.device.clone(), true))
        }).await
    }

    /// Revokes every still-active credential of one owner; used when the
    /// account secret rotates so the registry matches the dead tokens.
    pub async fn revoke_all(&self, user_id: &str) -> Result<usize, AppError> {
        let user_id = user_id.to_owned();
        self.mutate(true, move |records| {
            let now = nomifun_common::now_ms();
            let mut count = 0;
            for record in records.values_mut().filter(|record| record.user_id == user_id && record.device.revoked_at_ms.is_none()) {
                record.device.revoked_at_ms = Some(now);
                count += 1;
            }
            Ok((count, count > 0))
        }).await
    }

    pub async fn touch(&self, device_id: &str, user_id: &str, credential_id: Option<&str>) -> Result<(), AppError> {
        let device_id = device_id.to_owned();
        let user_id = user_id.to_owned();
        let credential_id = credential_id.map(str::to_owned);
        self.mutate(false, move |records| {
            let Some(record) = records.get_mut(&device_id).filter(|r| r.user_id == user_id && r.device.revoked_at_ms.is_none()
                && r.credential_id == credential_id) else { return Ok(((), false)); };
            let now = nomifun_common::now_ms();
            if now.saturating_sub(record.device.last_seen_at_ms) < ACTIVITY_PERSIST_INTERVAL_MS { return Ok(((), false)); }
            record.device.last_seen_at_ms = now;
            Ok(((), true))
        }).await
    }

    /// The task owns the entire serialized mutation. Dropping an HTTP request
    /// after SQLite commits cannot cancel publication or socket invalidation,
    /// and later mutations cannot snapshot stale state while it completes.
    async fn mutate<T: Send + 'static>(
        &self,
        invalidate: bool,
        change: impl FnOnce(&mut BTreeMap<String, DeviceRecord>) -> Result<(T, bool), AppError> + Send + 'static,
    ) -> Result<T, AppError> {
        let store = self.clone();
        tokio::spawn(async move {
            let _mutation = store.mutation.lock().await;
            let mut records = store.snapshot()?;
            let (result, changed) = change(&mut records)?;
            if changed {
                store.publish(records).await?;
                if invalidate {
                    store.invalidations.send_modify(|generation| *generation = generation.wrapping_add(1));
                }
            }
            Ok(result)
        }).await.map_err(|error| AppError::Internal(format!("Device authentication mutation failed: {error}")))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use nomifun_db::{DbError, SqliteClientPreferenceRepository, init_database_memory, models::ClientPreference};

    const KEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const KEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    async fn fixture() -> (nomifun_db::Database, Arc<SqliteClientPreferenceRepository>, DeviceStore, String) {
        let database = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteClientPreferenceRepository::new(database.pool().clone()));
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let store = DeviceStore::load(repo.clone()).await.unwrap();
        (database, repo, store, owner)
    }

    fn sign(jwt: &crate::JwtService, owner: &str, registered: &RegisteredDevice) -> String {
        jwt.sign_for_device(owner, "admin", &registered.device.device_id, Some(&registered.credential_id)).unwrap()
    }

    #[tokio::test]
    async fn repeat_authorization_reuses_installation_and_same_names_stay_independent() {
        let (_, repo, store, owner) = fixture().await;
        let first = store.register(&owner, " Phone ", KEY_A, None).await.unwrap();
        let second = store.register(&owner, "Renamed phone", &KEY_A.to_uppercase(), None).await.unwrap();
        assert_eq!(first.device.device_id, second.device.device_id);
        assert_eq!(first.device.created_at_ms, second.device.created_at_ms);
        assert!(second.device.last_seen_at_ms >= first.device.last_seen_at_ms);
        assert_eq!(second.device.device_name, "Renamed phone");
        assert_ne!(first.credential_id, second.credential_id);
        let other = store.register(&owner, "Renamed phone", KEY_B, None).await.unwrap();
        assert_ne!(second.device.device_id, other.device.device_id);
        assert_eq!(store.list(&owner).unwrap().len(), 2);
        let raw = &repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap()[0].value;
        assert!(!raw.contains(KEY_A));
        assert!(!raw.contains(KEY_B));
        assert!(DeviceStore::validate_installation_key("invalid").is_err());
    }

    #[tokio::test]
    async fn revoked_and_rotated_credentials_never_revive_or_renew() {
        let (_, _, store, owner) = fixture().await;
        let store = Arc::new(store);
        let jwt = crate::JwtService::with_devices("test-secret".into(), store.clone());
        let first = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        let token = sign(&jwt, &owner, &first);
        let payload = jwt.verify(&token).unwrap();
        let renewed = jwt.renew(&payload, "admin").unwrap();
        assert_eq!(jwt.verify(&renewed).unwrap().device_credential_id.as_deref(), Some(first.credential_id.as_str()));
        store.revoke(&first.device.device_id, &owner).await.unwrap();
        let second = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        assert_eq!(first.device.device_id, second.device.device_id);
        assert!(second.device.revoked_at_ms.is_none());
        assert!(jwt.verify(&token).is_err());
        assert!(jwt.verify(&renewed).is_err());
        assert!(jwt.renew(&payload, "admin").is_err());
        assert!(jwt.sign_for_device(&owner, "admin", &second.device.device_id, None).is_err());
        assert!(jwt.verify(&sign(&jwt, &owner, &second)).is_ok());
        assert!(store.revoke_credential(&first.device.device_id, &owner, Some(&first.credential_id)).await.is_err());
        assert!(jwt.verify(&sign(&jwt, &owner, &second)).is_ok());
        let current_token = sign(&jwt, &owner, &second);
        store.revoke_credential(&second.device.device_id, &owner, Some(&second.credential_id)).await.unwrap();
        assert!(jwt.verify(&current_token).is_err());
    }

    #[tokio::test]
    async fn concurrent_repair_has_one_record_and_survives_restart_and_agent_reset() {
        let (database, repo, store, owner) = fixture().await;
        let (a, b) = tokio::join!(store.register(&owner, "Phone", KEY_A, None), store.register(&owner, "Phone", KEY_A, None));
        let a = a.unwrap(); let b = b.unwrap();
        assert_eq!(a.device.device_id, b.device.device_id);
        assert_ne!(a.credential_id, b.credential_id);
        assert_eq!(store.list(&owner).unwrap().len(), 1);
        let current = if store.is_credential_active(&a.device.device_id, &owner, Some(&a.credential_id)) { &a } else { &b };
        let old = if current.credential_id == a.credential_id { &b } else { &a };
        nomifun_db::reset_agent_data(database.pool()).await.unwrap();
        let restarted = Arc::new(DeviceStore::load(repo).await.unwrap());
        let jwt = crate::JwtService::with_devices("persistent-secret".into(), restarted.clone());
        assert!(jwt.verify(&sign(&jwt, &owner, current)).is_ok());
        assert!(!restarted.is_credential_active(&old.device.device_id, &owner, Some(&old.credential_id)));
        let again = restarted.register(&owner, "Phone again", KEY_A, None).await.unwrap();
        assert_eq!(current.device.device_id, again.device.device_id);
        assert_eq!(restarted.list(&owner).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn independent_revocation_survives_restart_and_agent_reset() {
        let (database, repo, store, owner) = fixture().await;
        let store = Arc::new(store);
        let jwt = crate::JwtService::with_devices("persistent-secret".into(), store.clone());
        let a = store.register(&owner, "Phone A", KEY_A, None).await.unwrap();
        let b = store.register(&owner, "Phone B", KEY_B, None).await.unwrap();
        let a_token = sign(&jwt, &owner, &a);
        let b_token = sign(&jwt, &owner, &b);
        store.revoke(&a.device.device_id, &owner).await.unwrap();
        nomifun_db::reset_agent_data(database.pool()).await.unwrap();
        let restarted = Arc::new(DeviceStore::load(repo.clone()).await.unwrap());
        let jwt = crate::JwtService::with_devices("persistent-secret".into(), restarted.clone());
        assert!(jwt.verify(&a_token).is_err());
        assert!(jwt.verify(&b_token).is_ok());
        assert_eq!(restarted.list(&owner).unwrap().len(), 2);
        let raw = &repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap()[0].value;
        assert!(!raw.contains(&a_token));
        assert!(!raw.contains(&b_token));
    }

    #[tokio::test]
    async fn installation_can_reauthorize_at_capacity() {
        let store = DeviceStore::default();
        let owner = nomifun_common::UserId::new().into_string();
        let first = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        for i in 0..MAX_DEVICES - 1 {
            store.register(&owner, "Phone", &format!("{i:064x}"), None).await.unwrap();
        }
        let again = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        assert_eq!(first.device.device_id, again.device.device_id);
        assert_eq!(store.list(&owner).unwrap().len(), MAX_DEVICES);
        assert!(matches!(store.register(&owner, "Phone", KEY_B, None).await, Err(AppError::Conflict(_))));
    }

    #[tokio::test]
    async fn only_the_proven_legacy_record_is_claimed_and_its_old_token_dies() {
        let (_, repo, _, owner) = fixture().await;
        let old_id = crate::generate_random_hex_secret();
        let other_id = crate::generate_random_hex_secret();
        let legacy = |id: &str| serde_json::json!({"user_id":owner,"device_id":id,"device_name":"Phone","created_at_ms":1,"last_seen_at_ms":1,"revoked_at_ms":null});
        let raw = serde_json::json!({old_id.clone():legacy(&old_id),other_id.clone():legacy(&other_id)}).to_string();
        repo.upsert_batch(&[(DEVICE_AUTH_PREFERENCE_KEY, &raw)]).await.unwrap();
        let store = Arc::new(DeviceStore::load(repo.clone()).await.unwrap());
        let jwt = crate::JwtService::with_devices("legacy-secret".into(), store.clone());
        let old_token = jwt.sign_for_device(&owner, "admin", &old_id, None).unwrap();
        let other_token = jwt.sign_for_device(&owner, "admin", &other_id, None).unwrap();
        let claimed = store.register(&owner, "Phone", KEY_A, Some(&old_id)).await.unwrap();
        assert_eq!(claimed.device.device_id, old_id);
        assert_eq!(claimed.device.created_at_ms, 1);
        assert!(jwt.verify(&old_token).is_err());
        assert!(jwt.verify(&other_token).is_ok());
        assert!(store.register(&owner, "Phone", KEY_B, Some(&old_id)).await.is_err());
        let foreign = nomifun_common::UserId::new().into_string();
        assert!(store.register(&foreign, "Phone", KEY_A, Some(&old_id)).await.is_err());
        assert_eq!(store.list(&owner).unwrap().len(), 2);
        let bound = store.register(&owner, "Phone", KEY_A, Some(&other_id)).await.unwrap();
        assert_eq!(bound.device.device_id, old_id);
        assert!(jwt.verify(&other_token).is_err());
        let restarted = DeviceStore::load(repo).await.unwrap();
        assert_eq!(restarted.list(&owner).unwrap().iter().filter(|row| row.revoked_at_ms.is_none()).count(), 1);
    }

    #[tokio::test]
    async fn partial_or_corrupt_identity_fails_closed() {
        let (_, repo, store, owner) = fixture().await;
        let device = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        let rows = repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap();
        let mut raw: serde_json::Value = serde_json::from_str(&rows[0].value).unwrap();
        raw[&device.device.device_id].as_object_mut().unwrap().remove("credential_id");
        repo.upsert_batch(&[(DEVICE_AUTH_PREFERENCE_KEY, &raw.to_string())]).await.unwrap();
        let corrupt = DeviceStore::load(repo.clone()).await.unwrap();
        assert!(!corrupt.is_credential_active(&device.device.device_id, &owner, Some(&device.credential_id)));
        assert!(corrupt.list(&owner).unwrap().is_empty());
        let fresh = corrupt.register(&owner, "Phone", KEY_A, None).await.unwrap();
        assert_ne!(fresh.device.device_id, device.device.device_id);
        let rows = repo.get_by_keys(&[DEVICE_AUTH_PREFERENCE_KEY]).await.unwrap();
        let mut raw: serde_json::Value = serde_json::from_str(&rows[0].value).unwrap();
        raw[&fresh.device.device_id].as_object_mut().unwrap().remove("revoked_at_ms");
        repo.upsert_batch(&[(DEVICE_AUTH_PREFERENCE_KEY, &raw.to_string())]).await.unwrap();
        assert!(DeviceStore::load(repo).await.unwrap().list(&owner).unwrap().is_empty());
    }

    #[tokio::test]
    async fn stale_activity_cannot_touch_new_authorization_and_revoke_all_stays_owner_scoped() {
        let (_, _, store, owner) = fixture().await;
        let old = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        let current = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        let mut snapshot = store.snapshot().unwrap();
        snapshot.get_mut(&current.device.device_id).unwrap().device.last_seen_at_ms = 0;
        store.publish(snapshot).await.unwrap();
        store.touch(&old.device.device_id, &owner, Some(&old.credential_id)).await.unwrap();
        assert_eq!(store.list(&owner).unwrap()[0].last_seen_at_ms, 0);
        store.touch(&current.device.device_id, &owner, Some(&current.credential_id)).await.unwrap();
        assert!(store.list(&owner).unwrap()[0].last_seen_at_ms > 0);
        let foreign = nomifun_common::UserId::new().into_string();
        let other = store.register(&foreign, "Phone", KEY_A, None).await.unwrap();
        assert_eq!(store.revoke_all(&owner).await.unwrap(), 1);
        assert_eq!(store.revoke_all(&owner).await.unwrap(), 0);
        assert!(store.is_credential_active(&other.device.device_id, &foreign, Some(&other.credential_id)));
    }

    struct FailingWrites {
        inner: Arc<SqliteClientPreferenceRepository>,
        fail: AtomicBool,
    }

    #[async_trait::async_trait]
    impl IClientPreferenceRepository for FailingWrites {
        async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError> { self.inner.get_all().await }
        async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError> { self.inner.get_by_keys(keys).await }
        async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError> {
            if self.fail.load(Ordering::SeqCst) { return Err(DbError::Init("injected write failure".into())); }
            self.inner.upsert_batch(entries).await
        }
        async fn delete_keys(&self, keys: &[&str]) -> Result<(), DbError> { self.inner.delete_keys(keys).await }
    }

    struct CommitBarrier {
        inner: Arc<SqliteClientPreferenceRepository>,
        block: AtomicBool,
        committed: tokio::sync::Semaphore,
        release: tokio::sync::Semaphore,
    }

    #[async_trait::async_trait]
    impl IClientPreferenceRepository for CommitBarrier {
        async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError> { self.inner.get_all().await }
        async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError> { self.inner.get_by_keys(keys).await }
        async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError> {
            self.inner.upsert_batch(entries).await?;
            if self.block.swap(false, Ordering::SeqCst) {
                self.committed.add_permits(1);
                self.release.acquire().await.expect("test barrier remains open").forget();
            }
            Ok(())
        }
        async fn delete_keys(&self, keys: &[&str]) -> Result<(), DbError> { self.inner.delete_keys(keys).await }
    }

    #[tokio::test]
    async fn abandoned_request_finishes_committed_authorization_and_socket_invalidation() {
        for revoke in [false, true] {
            let (_, repo, _, owner) = fixture().await;
            let repo = Arc::new(CommitBarrier {
                inner: repo, block: AtomicBool::new(false),
                committed: tokio::sync::Semaphore::new(0), release: tokio::sync::Semaphore::new(0),
            });
            let store = Arc::new(DeviceStore::load(repo.clone()).await.unwrap());
            let jwt = crate::JwtService::with_devices("cancellation-secret".into(), store.clone());
            let first = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
            let token = sign(&jwt, &owner, &first);
            let mut invalidations = jwt.subscribe_invalidations();
            repo.block.store(true, Ordering::SeqCst);
            let request = tokio::spawn({
                let store = store.clone(); let owner = owner.clone();
                let device_id = first.device.device_id.clone();
                async move {
                    if revoke { store.revoke(&device_id, &owner).await.map(|_| ()) }
                    else { store.register(&owner, "Phone again", KEY_A, None).await.map(|_| ()) }
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), repo.committed.acquire()).await.unwrap().unwrap().forget();
            assert!(jwt.verify(&token).is_ok(), "publication is still held at the post-commit test barrier");
            assert!(!invalidations.has_changed().unwrap());
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            // A queued stale activity write must snapshot only after the owned
            // mutation publishes; it must never restore the old generation.
            let stale_activity = tokio::spawn({
                let store = store.clone(); let owner = owner.clone();
                let device_id = first.device.device_id.clone(); let credential_id = first.credential_id.clone();
                async move { store.touch(&device_id, &owner, Some(&credential_id)).await }
            });
            repo.release.add_permits(1);
            tokio::time::timeout(std::time::Duration::from_secs(2), invalidations.changed()).await.unwrap().unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2), stale_activity).await.unwrap().unwrap().unwrap();
            assert!(jwt.verify(&token).is_err());
            let restarted = Arc::new(DeviceStore::load(repo.inner.clone()).await.unwrap());
            assert_eq!(store.list(&owner).unwrap(), restarted.list(&owner).unwrap());
            let current_credential = store.snapshot().unwrap()[&first.device.device_id].credential_id.clone();
            assert_eq!(store.is_credential_active(&first.device.device_id, &owner, current_credential.as_deref()), !revoke);
            assert_eq!(restarted.is_credential_active(&first.device.device_id, &owner, current_credential.as_deref()), !revoke);
            let restarted_jwt = crate::JwtService::with_devices("cancellation-secret".into(), restarted);
            assert!(restarted_jwt.verify(&token).is_err());
            assert_eq!(*invalidations.borrow_and_update(), 2);
            assert!(!invalidations.has_changed().unwrap(), "one authorization mutation emits one invalidation");
        }
    }

    #[tokio::test]
    async fn failed_registration_and_revocation_publish_no_credential_changes() {
        let (_, repo, _, owner) = fixture().await;
        let repo = Arc::new(FailingWrites { inner: repo, fail: AtomicBool::new(false) });
        let store = Arc::new(DeviceStore::load(repo.clone()).await.unwrap());
        let jwt = crate::JwtService::with_devices("failure-secret".into(), store.clone());
        let first = store.register(&owner, "Phone", KEY_A, None).await.unwrap();
        let token = sign(&jwt, &owner, &first);
        let invalidations = jwt.subscribe_invalidations();
        repo.fail.store(true, Ordering::SeqCst);
        assert!(store.register(&owner, "Renamed", KEY_A, None).await.is_err());
        assert!(store.register(&owner, "Other", KEY_B, None).await.is_err());
        assert!(store.revoke_credential(&first.device.device_id, &owner, Some(&first.credential_id)).await.is_err());
        assert!(jwt.verify(&token).is_ok());
        assert!(!invalidations.has_changed().unwrap());
        assert_eq!(store.list(&owner).unwrap(), vec![first.device.clone()]);
        let restarted = DeviceStore::load(repo).await.unwrap();
        assert!(restarted.is_credential_active(&first.device.device_id, &owner, Some(&first.credential_id)));
        assert_eq!(restarted.list(&owner).unwrap(), vec![first.device]);
    }
}
