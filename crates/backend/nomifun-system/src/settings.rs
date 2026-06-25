use std::sync::Arc;

use nomifun_api_types::{SystemSettingsResponse, UpdateSettingsRequest};
use nomifun_common::AppError;
use nomifun_db::ISettingsRepository;

/// Supported BCP 47 language codes.
const SUPPORTED_LANGUAGES: &[&str] = &["en-US", "zh-CN"];

/// Business logic for system settings (language, notifications, etc.).
#[derive(Clone)]
pub struct SettingsService {
    repo: Arc<dyn ISettingsRepository>,
}

impl SettingsService {
    pub fn new(repo: Arc<dyn ISettingsRepository>) -> Self {
        Self { repo }
    }

    /// Get current system settings, falling back to defaults if not yet persisted.
    pub async fn get_settings(&self) -> Result<SystemSettingsResponse, AppError> {
        let row = self
            .repo
            .get_settings()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to get settings: {e}")))?;

        Ok(
            row.map_or_else(SystemSettingsResponse::default, |s| SystemSettingsResponse {
                language: s.language,
                notification_enabled: s.notification_enabled,
                cron_notification_enabled: s.cron_notification_enabled,
                command_queue_enabled: s.command_queue_enabled,
                save_upload_to_workspace: s.save_upload_to_workspace,
            }),
        )
    }

    /// Partially update system settings. Only fields present in the request are changed.
    pub async fn update_settings(&self, req: UpdateSettingsRequest) -> Result<SystemSettingsResponse, AppError> {
        if let Some(ref lang) = req.language {
            validate_language(lang)?;
        }

        // Merge with current settings (or defaults)
        let current = self.get_settings().await?;

        let language = req.language.unwrap_or(current.language);
        let notification_enabled = req.notification_enabled.unwrap_or(current.notification_enabled);
        let cron_notification_enabled = req
            .cron_notification_enabled
            .unwrap_or(current.cron_notification_enabled);
        let command_queue_enabled = req.command_queue_enabled.unwrap_or(current.command_queue_enabled);
        let save_upload_to_workspace = req.save_upload_to_workspace.unwrap_or(current.save_upload_to_workspace);

        let row = self
            .repo
            .upsert_settings(
                &language,
                notification_enabled,
                cron_notification_enabled,
                command_queue_enabled,
                save_upload_to_workspace,
            )
            .await
            .map_err(|e| AppError::Internal(format!("Failed to update settings: {e}")))?;

        Ok(SystemSettingsResponse {
            language: row.language,
            notification_enabled: row.notification_enabled,
            cron_notification_enabled: row.cron_notification_enabled,
            command_queue_enabled: row.command_queue_enabled,
            save_upload_to_workspace: row.save_upload_to_workspace,
        })
    }
}

fn validate_language(lang: &str) -> Result<(), AppError> {
    if SUPPORTED_LANGUAGES.contains(&lang) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("Unsupported language code: '{lang}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{SqliteSettingsRepository, init_database_memory};

    async fn setup() -> SettingsService {
        let db = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteSettingsRepository::new(db.pool().clone()));
        // Leak the db handle so the pool stays alive for the test
        std::mem::forget(db);
        SettingsService::new(repo)
    }

    #[test]
    fn validate_language_accepts_supported() {
        assert!(validate_language("en-US").is_ok());
        assert!(validate_language("zh-CN").is_ok());
    }

    #[test]
    fn validate_language_rejects_unsupported() {
        for lang in [
            "invalid", "", "xx-YY", "zh-TW", "ja-JP", "ko-KR", "ru-RU", "tr-TR", "uk-UA", "fr-FR",
        ] {
            assert!(validate_language(lang).is_err(), "{lang} should be rejected");
        }
    }

    #[tokio::test]
    async fn get_settings_returns_defaults_when_empty() {
        let svc = setup().await;
        let settings = svc.get_settings().await.unwrap();
        assert_eq!(settings, SystemSettingsResponse::default());
    }

    #[tokio::test]
    async fn update_single_field() {
        let svc = setup().await;
        let req = UpdateSettingsRequest {
            language: Some("zh-CN".into()),
            ..Default::default()
        };
        let result = svc.update_settings(req).await.unwrap();
        assert_eq!(result.language, "zh-CN");
        // Other fields stay at defaults
        assert!(result.notification_enabled);
        assert!(!result.cron_notification_enabled);
    }

    #[tokio::test]
    async fn update_multiple_fields() {
        let svc = setup().await;
        let req = UpdateSettingsRequest {
            notification_enabled: Some(false),
            command_queue_enabled: Some(true),
            ..Default::default()
        };
        let result = svc.update_settings(req).await.unwrap();
        assert!(!result.notification_enabled);
        assert!(result.command_queue_enabled);
        assert_eq!(result.language, "en-US");
    }

    #[tokio::test]
    async fn update_empty_request_returns_current() {
        let svc = setup().await;
        let result = svc.update_settings(UpdateSettingsRequest::default()).await.unwrap();
        assert_eq!(result, SystemSettingsResponse::default());
    }

    #[tokio::test]
    async fn update_invalid_language_rejected() {
        let svc = setup().await;
        let req = UpdateSettingsRequest {
            language: Some("invalid-lang".into()),
            ..Default::default()
        };
        let err = svc.update_settings(req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_then_get_reflects_changes() {
        let svc = setup().await;
        svc.update_settings(UpdateSettingsRequest {
            language: Some("zh-CN".into()),
            save_upload_to_workspace: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();

        let settings = svc.get_settings().await.unwrap();
        assert_eq!(settings.language, "zh-CN");
        assert!(settings.save_upload_to_workspace);
    }
}
