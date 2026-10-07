use std::sync::Arc;

use nomifun_api_types::{ClientPreferencesResponse, UpdateClientPreferencesRequest};
use nomifun_common::AppError;
use nomifun_db::IClientPreferenceRepository;

/// Maximum allowed key length for client preferences.
const MAX_KEY_LENGTH: usize = 255;
/// System-owned and retired key prefixes. The generic PUT /api/settings/client
/// endpoint rejects them so a client cannot restore removed product settings:
/// - Retired Browser v1 keys have no supported write owner. They must not be
///   resurrected through the generic preferences API after their UI is removed.
const SYSTEM_RESERVED_PREFIXES: &[&str] = &["agent.browserUse", "browser.resourcePolicy"];

/// Business logic for client preferences (generic key-value store).
#[derive(Clone)]
pub struct ClientPrefService {
    repo: Arc<dyn IClientPreferenceRepository>,
}

impl ClientPrefService {
    pub fn new(repo: Arc<dyn IClientPreferenceRepository>) -> Self {
        Self { repo }
    }

    /// Get all client preferences, or only the specified keys.
    pub async fn get_preferences(&self, keys: Option<&[&str]>) -> Result<ClientPreferencesResponse, AppError> {
        let rows = match keys {
            Some(k) if !k.is_empty() => self.repo.get_by_keys(k).await,
            _ => self.repo.get_all().await,
        }
        .map_err(|e| AppError::Internal(format!("Failed to get preferences: {e}")))?;

        let mut map = ClientPreferencesResponse::new();
        for row in rows {
            let value: serde_json::Value =
                serde_json::from_str(&row.value).unwrap_or(serde_json::Value::String(row.value));
            map.insert(row.key, value);
        }
        Ok(map)
    }

    /// Batch update client preferences. Null values delete the key.
    pub async fn update_preferences(&self, req: UpdateClientPreferencesRequest) -> Result<(), AppError> {
        let mut upserts: Vec<(String, String)> = Vec::new();
        let mut deletes: Vec<String> = Vec::new();

        for (key, value) in req {
            validate_key(&key)?;
            if SYSTEM_RESERVED_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
            {
                return Err(AppError::Forbidden(format!(
                    "Preference key '{key}' is managed by the system"
                )));
            }

            if value.is_null() {
                deletes.push(key);
            } else {
                upserts.push((
                    key,
                    serde_json::to_string(&value)
                        .map_err(|e| AppError::Internal(format!("Failed to serialize value: {e}")))?,
                ));
            }
        }

        let entries: Vec<(&str, &str)> = upserts
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let keys: Vec<&str> = deletes.iter().map(String::as_str).collect();
        self.repo.update_batch(&entries, &keys).await?;

        Ok(())
    }
}

fn validate_key(key: &str) -> Result<(), AppError> {
    if key.is_empty() {
        return Err(AppError::BadRequest("Preference key must not be empty".into()));
    }
    if key.len() > MAX_KEY_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Preference key exceeds maximum length of {MAX_KEY_LENGTH} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{SqliteClientPreferenceRepository, init_database_memory};
    use serde_json::json;

    async fn setup() -> ClientPrefService {
        let db = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteClientPreferenceRepository::new(db.pool().clone()));
        ClientPrefService::new(repo)
    }

    #[test]
    fn validate_key_accepts_valid() {
        assert!(validate_key("theme").is_ok());
        assert!(validate_key("system.closeToTray").is_ok());
        assert!(validate_key("a").is_ok());
    }

    #[test]
    fn validate_key_rejects_empty() {
        assert!(validate_key("").is_err());
    }

    #[test]
    fn validate_key_rejects_too_long() {
        let long_key = "x".repeat(MAX_KEY_LENGTH + 1);
        assert!(validate_key(&long_key).is_err());
    }

    #[tokio::test]
    async fn update_rejects_retired_browser_preferences() {
        let svc = setup().await;
        for (key, value) in [
            ("agent.browserUse.displayMode", json!("external")),
            ("agent.browserUse.displayModeVersion", json!(2)),
            ("agent.browserUse", json!(true)),
            ("agent.browserUse.source", json!("managed")),
            ("agent.browserUse.fullPower", json!(true)),
            ("agent.browserUse.persistentLogin", json!(false)),
            ("browser.resourcePolicy", json!({"preset":"highConcurrency"})),
        ] {
            let mut req = UpdateClientPreferencesRequest::new();
            req.insert(key.into(), value);
            let err = svc.update_preferences(req).await.unwrap_err();
            assert_eq!(
                err.status_code(),
                axum::http::StatusCode::FORBIDDEN,
                "{key} has no supported browser settings owner"
            );
        }
        // Retired keys are not rewritten or migrated through the generic API.
        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("agent.browserUse.displayModeVersion".into(), json!(null));
        assert_eq!(
            svc.update_preferences(req).await.unwrap_err().status_code(),
            axum::http::StatusCode::FORBIDDEN
        );
        assert!(svc.get_preferences(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_empty_returns_empty_map() {
        let svc = setup().await;
        let prefs = svc.get_preferences(None).await.unwrap();
        assert!(prefs.is_empty());
    }

    #[tokio::test]
    async fn scalar_preferences_round_trip() {
        let svc = setup().await;
        let req: UpdateClientPreferencesRequest = serde_json::from_value(json!({
            "system.closeToTray": true, "companion.size": 360, "theme": "dark"
        })).unwrap();
        svc.update_preferences(req.clone()).await.unwrap();
        assert_eq!(svc.get_preferences(None).await.unwrap(), req);
    }

    #[tokio::test]
    async fn null_deletes_key() {
        let svc = setup().await;

        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("theme".into(), json!("dark"));
        svc.update_preferences(req).await.unwrap();

        let mut req2 = UpdateClientPreferencesRequest::new();
        req2.insert("theme".into(), json!(null));
        svc.update_preferences(req2).await.unwrap();

        let prefs = svc.get_preferences(None).await.unwrap();
        assert!(!prefs.contains_key("theme"));
    }

    #[tokio::test]
    async fn get_by_keys_filters() {
        let svc = setup().await;

        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("a".into(), json!(1));
        req.insert("b".into(), json!(2));
        req.insert("c".into(), json!(3));
        svc.update_preferences(req).await.unwrap();

        let prefs = svc.get_preferences(Some(&["a", "c"])).await.unwrap();
        assert_eq!(prefs.len(), 2);
        assert_eq!(prefs["a"], json!(1));
        assert_eq!(prefs["c"], json!(3));
    }

    #[tokio::test]
    async fn overwrite_existing_value() {
        let svc = setup().await;

        let mut req1 = UpdateClientPreferencesRequest::new();
        req1.insert("k".into(), json!("v1"));
        svc.update_preferences(req1).await.unwrap();

        let mut req2 = UpdateClientPreferencesRequest::new();
        req2.insert("k".into(), json!("v2"));
        svc.update_preferences(req2).await.unwrap();

        let prefs = svc.get_preferences(None).await.unwrap();
        assert_eq!(prefs["k"], json!("v2"));
    }

    #[tokio::test]
    async fn empty_key_rejected() {
        let svc = setup().await;
        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("".into(), json!(true));
        let err = svc.update_preferences(req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn long_key_rejected() {
        let svc = setup().await;
        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("x".repeat(256), json!(true));
        let err = svc.update_preferences(req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn batch_mixed_upsert_and_delete() {
        let svc = setup().await;

        let mut setup_req = UpdateClientPreferencesRequest::new();
        setup_req.insert("keep".into(), json!(1));
        setup_req.insert("remove".into(), json!(2));
        svc.update_preferences(setup_req).await.unwrap();

        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("remove".into(), json!(null));
        req.insert("new".into(), json!(3));
        svc.update_preferences(req).await.unwrap();

        let prefs = svc.get_preferences(None).await.unwrap();
        assert_eq!(prefs.len(), 2);
        assert_eq!(prefs["keep"], json!(1));
        assert_eq!(prefs["new"], json!(3));
    }

    #[tokio::test]
    async fn provider_reference_conflict_is_not_reported_as_internal_error() {
        let svc = setup().await;
        let mut req = UpdateClientPreferencesRequest::new();
        req.insert("theme".into(), json!("dark"));
        req.insert(
            "knowledge.autogenModel".into(),
            json!({
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000099",
                "model": "missing"
            }),
        );

        let error = svc.update_preferences(req).await.unwrap_err();
        assert_eq!(error.status_code(), axum::http::StatusCode::CONFLICT);
        assert!(
            svc.get_preferences(None).await.unwrap().is_empty(),
            "the generic endpoint must not partially persist a mixed batch"
        );
    }
}
