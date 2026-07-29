//! Row-level service + row → wire projection for the authoritative
//! `provider_models` entity (`/api/provider-models`).
//!
//! JSON parse failures on a row degrade to empty/None values with a
//! `tracing::warn!` instead of failing the whole listing — one bad row must
//! never take down `GET /api/providers` (same tolerance strategy as
//! `row_to_profile` in `model_profile.rs` uses for profile rows).

use std::sync::Arc;

use nomifun_api_types::{
    CreateProviderModelRequest, ModelHealthStatus, ModelTask, ModelTrait, ProfileSource,
    ProviderModelResponse, UpdateProviderModelRequest,
};
use nomifun_common::{AppError, ProviderId};
use nomifun_db::{
    IProviderModelRepository, IProviderRepository, NewProviderModel, ProviderModelRow,
    ProviderModelUpdate,
};

fn source_from_str(s: &str) -> ProfileSource {
    match s {
        "user" => ProfileSource::User,
        _ => ProfileSource::Inferred,
    }
}

/// CRUD over the row-level model catalog (`/api/provider-models`).
///
/// This service deliberately does NOT write back to the legacy
/// `providers.models` JSON column: since the Task 4 projection, every reader
/// of catalog membership (including `ProviderService::list`) projects from
/// `provider_models` rows, so a row created/deleted here is immediately
/// visible there. Dual-write only exists in the legacy→new direction to guard
/// against direct writers of the old column drifting.
#[derive(Clone)]
pub struct ProviderModelService {
    repo: Arc<dyn IProviderModelRepository>,
    provider_repo: Arc<dyn IProviderRepository>,
}

impl ProviderModelService {
    pub fn new(
        repo: Arc<dyn IProviderModelRepository>,
        provider_repo: Arc<dyn IProviderRepository>,
    ) -> Self {
        Self { repo, provider_repo }
    }

    /// All rows, or one provider's rows when `provider_id` is given.
    pub async fn list(
        &self,
        provider_id: Option<&str>,
    ) -> Result<Vec<ProviderModelResponse>, AppError> {
        let rows = match provider_id {
            Some(provider_id) => {
                let provider_id = ProviderId::parse(provider_id).map_err(|error| {
                    AppError::BadRequest(format!("invalid provider_id: {error}"))
                })?;
                self.repo.list_for_provider(provider_id.as_str()).await?
            }
            None => self.repo.list().await?,
        };
        rows.into_iter().map(row_to_model_response).collect()
    }

    /// Create one catalog row.
    ///
    /// - the parent provider must exist (`NotFound`), and a duplicate
    ///   `(provider_id, model)` key is a `Conflict` (from the repository);
    /// - an empty `tasks` means "no explicit profile": tasks (and traits, when
    ///   not explicitly given) are seeded from
    ///   [`nomifun_api_types::derive_tasks_and_traits`] with
    ///   `source = inferred`; a non-empty `tasks` is stored as given with
    ///   `source = user`;
    /// - `sort_order` defaults to appending after the provider's catalog.
    pub async fn create(
        &self,
        req: CreateProviderModelRequest,
    ) -> Result<ProviderModelResponse, AppError> {
        let provider_id = ProviderId::parse(req.provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?
            .into_string();
        let model = req.model.trim();
        if model.is_empty() {
            return Err(AppError::BadRequest("model is required".into()));
        }
        if let Some(role) = req.connection_role.as_deref() {
            crate::provider_connection::validate_role(role)?;
        }
        let provider = self
            .provider_repo
            .find_by_id(&provider_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provider '{provider_id}' not found")))?;

        let (tasks, traits, source): (Vec<ModelTask>, Vec<ModelTrait>, &str) = if req
            .tasks
            .is_empty()
        {
            let (derived_tasks, derived_traits) =
                nomifun_api_types::derive_tasks_and_traits(&provider.platform, model);
            let traits = if req.traits.is_empty() { derived_traits } else { req.traits };
            (derived_tasks, traits, "inferred")
        } else {
            (req.tasks, req.traits, "user")
        };
        let tasks_json = serde_json::to_string(&tasks)
            .map_err(|e| AppError::Internal(format!("serialize tasks: {e}")))?;
        let traits_json = serde_json::to_string(&traits)
            .map_err(|e| AppError::Internal(format!("serialize traits: {e}")))?;
        let params_value = req.params.unwrap_or_else(|| serde_json::json!({}));
        let params_json = serde_json::to_string(&params_value)
            .map_err(|e| AppError::Internal(format!("serialize params: {e}")))?;

        let sort_order = match req.sort_order {
            Some(sort_order) => sort_order,
            None => next_sort_order(self.repo.as_ref(), &provider_id).await?,
        };

        let row = self
            .repo
            .create(
                &provider_id,
                &NewProviderModel {
                    model,
                    enabled: req.enabled,
                    sort_order,
                    tasks: &tasks_json,
                    traits: &traits_json,
                    protocol: req.protocol.as_deref(),
                    params: &params_json,
                    context_limit: req.context_limit,
                    description: req.description.as_deref(),
                    source,
                    health: None,
                },
            )
            .await?;

        // `NewProviderModel` has no connection_role member (inserts always
        // start with NULL); apply an explicitly requested role right after.
        let row = match req.connection_role.as_deref() {
            Some(role) => {
                self.repo
                    .update(
                        &provider_id,
                        model,
                        &ProviderModelUpdate {
                            connection_role: Some(Some(role)),
                            ..Default::default()
                        },
                    )
                    .await?
            }
            None => row,
        };
        row_to_model_response(row)
    }

    /// Partially update one row. Double-Option fields distinguish keep
    /// (absent) / clear (`null`) / set (value). An explicit `tasks` or
    /// `traits` update is a user profile edit, so it also flips
    /// `source = user` (making the stored profile authoritative over the
    /// name heuristic). A missing row is `NotFound` (from the repository).
    pub async fn update(
        &self,
        req: UpdateProviderModelRequest,
    ) -> Result<ProviderModelResponse, AppError> {
        let provider_id = ProviderId::parse(req.provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?
            .into_string();
        // Double-Option: Some(Some(role)) sets and must satisfy the same role
        // grammar as the connections API; Some(None) clears without validation.
        if let Some(Some(role)) = req.connection_role.as_ref() {
            crate::provider_connection::validate_role(role)?;
        }

        let tasks_json = req
            .tasks
            .as_ref()
            .map(|tasks| {
                serde_json::to_string(tasks)
                    .map_err(|e| AppError::Internal(format!("serialize tasks: {e}")))
            })
            .transpose()?;
        let traits_json = req
            .traits
            .as_ref()
            .map(|traits| {
                serde_json::to_string(traits)
                    .map_err(|e| AppError::Internal(format!("serialize traits: {e}")))
            })
            .transpose()?;
        let params_json = req
            .params
            .as_ref()
            .map(|params| {
                serde_json::to_string(params)
                    .map_err(|e| AppError::Internal(format!("serialize params: {e}")))
            })
            .transpose()?;

        let update = ProviderModelUpdate {
            enabled: req.enabled,
            sort_order: req.sort_order,
            tasks: tasks_json.as_deref(),
            traits: traits_json.as_deref(),
            protocol: req.protocol.as_ref().map(|v| v.as_deref()),
            connection_role: req.connection_role.as_ref().map(|v| v.as_deref()),
            params: params_json.as_deref(),
            context_limit: req.context_limit,
            description: req.description.as_ref().map(|v| v.as_deref()),
            source: (req.tasks.is_some() || req.traits.is_some()).then_some("user"),
        };
        let row = self.repo.update(&provider_id, &req.model, &update).await?;
        row_to_model_response(row)
    }

    /// Delete one row; returns whether a row was removed (same contract as
    /// `ModelProfileService::delete`).
    pub async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, AppError> {
        ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?;
        Ok(self.repo.delete(provider_id, model).await?)
    }
}

/// Next append position for a provider's catalog (shared with
/// `ModelProfileService::upsert`'s create path).
pub(crate) async fn next_sort_order(
    repo: &dyn IProviderModelRepository,
    provider_id: &str,
) -> Result<i64, AppError> {
    Ok(repo
        .list_for_provider(provider_id)
        .await?
        .iter()
        .map(|row| row.sort_order)
        .max()
        .map_or(0, |max| max + 1))
}

/// Convert one `provider_models` row into the wire DTO.
///
/// Only a non-canonical stored `provider_id` is a hard error (it indicates
/// corrupted identity, not a degraded field); malformed JSON columns degrade
/// gracefully: `tasks`/`traits` → empty vec, `params` → `null`, `health` →
/// absent — each with a warning.
pub(crate) fn row_to_model_response(row: ProviderModelRow) -> Result<ProviderModelResponse, AppError> {
    ProviderId::parse(&row.provider_id).map_err(|error| {
        AppError::Internal(format!(
            "stored provider_models.provider_id '{}' is not canonical: {error}",
            row.provider_id
        ))
    })?;

    let tasks: Vec<ModelTask> = serde_json::from_str(&row.tasks).unwrap_or_else(|error| {
        tracing::warn!(
            provider_id = %row.provider_id,
            model = %row.model,
            %error,
            "invalid provider_models.tasks JSON; degrading to empty tasks"
        );
        Vec::new()
    });
    let traits: Vec<ModelTrait> = serde_json::from_str(&row.traits).unwrap_or_else(|error| {
        tracing::warn!(
            provider_id = %row.provider_id,
            model = %row.model,
            %error,
            "invalid provider_models.traits JSON; degrading to empty traits"
        );
        Vec::new()
    });
    let params: serde_json::Value = serde_json::from_str(&row.params).unwrap_or_else(|error| {
        tracing::warn!(
            provider_id = %row.provider_id,
            model = %row.model,
            %error,
            "invalid provider_models.params JSON; degrading to null params"
        );
        serde_json::Value::Null
    });
    let health: Option<ModelHealthStatus> = row.health.as_deref().and_then(|json| {
        serde_json::from_str(json)
            .map_err(|error| {
                tracing::warn!(
                    provider_id = %row.provider_id,
                    model = %row.model,
                    %error,
                    "invalid provider_models.health JSON; dropping health entry"
                );
            })
            .ok()
    });

    Ok(ProviderModelResponse {
        provider_id: row.provider_id,
        model: row.model,
        enabled: row.enabled,
        sort_order: row.sort_order,
        tasks,
        traits,
        protocol: row.protocol,
        connection_role: row.connection_role,
        params,
        context_limit: row.context_limit,
        description: row.description,
        source: source_from_str(&row.source),
        health,
        health_checked_at: row.health_checked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROVIDER_ID: &str = "018f1234-5678-7abc-8def-012345678990";

    fn sample_row() -> ProviderModelRow {
        ProviderModelRow {
            id: 7,
            provider_id: PROVIDER_ID.into(),
            model: "gpt-4o".into(),
            enabled: true,
            sort_order: 2,
            tasks: r#"["chat"]"#.into(),
            traits: r#"["vision_input"]"#.into(),
            protocol: Some("openai".into()),
            connection_role: None,
            params: r#"{"temperature":0.5}"#.into(),
            context_limit: Some(128000),
            description: Some("desc".into()),
            source: "user".into(),
            health: Some(r#"{"status":"healthy","latency":320}"#.into()),
            health_checked_at: Some(123),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn projects_all_fields() {
        let resp = row_to_model_response(sample_row()).unwrap();
        assert_eq!(resp.provider_id, PROVIDER_ID);
        assert_eq!(resp.model, "gpt-4o");
        assert!(resp.enabled);
        assert_eq!(resp.sort_order, 2);
        assert_eq!(resp.tasks, vec![ModelTask::Chat]);
        assert_eq!(resp.traits, vec![ModelTrait::VisionInput]);
        assert_eq!(resp.protocol.as_deref(), Some("openai"));
        assert_eq!(resp.params["temperature"], 0.5);
        assert_eq!(resp.context_limit, Some(128000));
        assert_eq!(resp.description.as_deref(), Some("desc"));
        assert_eq!(resp.source, ProfileSource::User);
        assert_eq!(
            resp.health.as_ref().map(|h| h.status),
            Some(nomifun_api_types::HealthStatus::Healthy)
        );
        assert_eq!(resp.health_checked_at, Some(123));
    }

    #[test]
    fn bad_json_degrades_instead_of_failing() {
        let row = ProviderModelRow {
            tasks: "not-json".into(),
            traits: "{broken".into(),
            params: "###".into(),
            health: Some("oops".into()),
            ..sample_row()
        };
        let resp = row_to_model_response(row).unwrap();
        assert!(resp.tasks.is_empty());
        assert!(resp.traits.is_empty());
        assert_eq!(resp.params, serde_json::Value::Null);
        assert!(resp.health.is_none());
    }

    #[test]
    fn noncanonical_provider_id_is_an_error() {
        let row = ProviderModelRow {
            provider_id: "not-a-uuid".into(),
            ..sample_row()
        };
        assert!(row_to_model_response(row).is_err());
    }

    // -- ProviderModelService tests (real in-memory SQLite repositories) --

    use nomifun_api_types::{CreateProviderModelRequest, UpdateProviderModelRequest};
    use nomifun_db::{
        CreateProviderParams, IProviderRepository, SqliteProviderModelRepository,
        SqliteProviderRepository, init_database_memory,
    };

    async fn setup(platform: &str) -> (ProviderModelService, crate::ProviderService, String, nomifun_db::Database) {
        let db = init_database_memory().await.unwrap();
        let provider_repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let model_repo = Arc::new(SqliteProviderModelRepository::new(db.pool().clone()));
        let provider_id = nomifun_common::ProviderId::new().into_string();
        provider_repo
            .create(CreateProviderParams {
                provider_id: Some(&provider_id),
                platform,
                name: "Test Provider",
                base_url: "https://x.test/v1",
                api_key_encrypted: &nomifun_common::encrypt_string("sk-test", &[0x42; 32]).unwrap(),
                models: "[]",
                enabled: true,
                capabilities: "[]",
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
        let service = ProviderModelService::new(model_repo.clone(), provider_repo.clone());
        let provider_service =
            crate::ProviderService::new(provider_repo, model_repo, [0x42; 32]);
        (service, provider_service, provider_id, db)
    }

    fn create_req(provider_id: &str, model: &str) -> CreateProviderModelRequest {
        CreateProviderModelRequest {
            provider_id: provider_id.into(),
            model: model.into(),
            enabled: true,
            tasks: vec![],
            traits: vec![],
            protocol: None,
            connection_role: None,
            params: None,
            context_limit: None,
            description: None,
            sort_order: None,
        }
    }

    fn update_req(provider_id: &str, model: &str) -> UpdateProviderModelRequest {
        UpdateProviderModelRequest {
            provider_id: provider_id.into(),
            model: model.into(),
            enabled: None,
            sort_order: None,
            tasks: None,
            traits: None,
            protocol: None,
            connection_role: None,
            params: None,
            context_limit: None,
            description: None,
        }
    }

    #[tokio::test]
    async fn create_seeds_inferred_tasks_when_tasks_empty() {
        let (service, _, provider_id, _db) = setup("stepfun").await;
        let resp = service
            .create(create_req(&provider_id, "step-asr"))
            .await
            .unwrap();
        assert_eq!(
            resp.tasks,
            vec![ModelTask::SpeechRecognition],
            "empty tasks are seeded from the platform+name heuristic"
        );
        assert_eq!(resp.source, ProfileSource::Inferred);
        assert!(resp.enabled);
        assert_eq!(resp.sort_order, 0);
        assert_eq!(resp.params, serde_json::json!({}));
    }

    #[tokio::test]
    async fn create_with_explicit_tasks_is_user_source() {
        let (service, _, provider_id, _db) = setup("openai").await;
        let resp = service
            .create(CreateProviderModelRequest {
                tasks: vec![ModelTask::ImageGeneration],
                traits: vec![],
                context_limit: Some(64_000),
                description: Some("img".into()),
                protocol: Some("openai".into()),
                connection_role: Some("primary".into()),
                params: Some(serde_json::json!({"steps": 4})),
                ..create_req(&provider_id, "my-image-model")
            })
            .await
            .unwrap();
        assert_eq!(resp.tasks, vec![ModelTask::ImageGeneration]);
        assert_eq!(resp.source, ProfileSource::User);
        assert_eq!(resp.context_limit, Some(64_000));
        assert_eq!(resp.description.as_deref(), Some("img"));
        assert_eq!(resp.protocol.as_deref(), Some("openai"));
        assert_eq!(resp.connection_role.as_deref(), Some("primary"));
        assert_eq!(resp.params["steps"], 4);
    }

    #[tokio::test]
    async fn create_appends_after_existing_catalog_and_rejects_duplicates() {
        let (service, _, provider_id, _db) = setup("openai").await;
        let first = service.create(create_req(&provider_id, "gpt-4o")).await.unwrap();
        assert_eq!(first.sort_order, 0);
        let second = service.create(create_req(&provider_id, "gpt-4o-mini")).await.unwrap();
        assert_eq!(second.sort_order, 1, "default sort_order appends (max+1)");

        let err = service.create(create_req(&provider_id, "gpt-4o")).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "duplicate key is Conflict: {err:?}");
    }

    #[tokio::test]
    async fn create_for_missing_provider_is_not_found() {
        let (service, _, _, _db) = setup("openai").await;
        let ghost = nomifun_common::ProviderId::new().into_string();
        let err = service.create(create_req(&ghost, "gpt-4o")).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "missing provider is NotFound: {err:?}");
    }

    #[tokio::test]
    async fn created_model_appears_in_provider_service_list_projection() {
        let (service, provider_service, provider_id, _db) = setup("openai").await;
        service.create(create_req(&provider_id, "gpt-4o")).await.unwrap();

        let providers = provider_service.list().await.unwrap();
        let provider = providers.iter().find(|p| p.provider_id == provider_id).unwrap();
        assert!(
            provider.models.contains(&"gpt-4o".to_string()),
            "row create is immediately visible in the legacy models projection"
        );
    }

    #[tokio::test]
    async fn update_partial_description_keeps_tasks_and_source() {
        let (service, _, provider_id, _db) = setup("stepfun").await;
        service.create(create_req(&provider_id, "step-asr")).await.unwrap();

        let resp = service
            .update(UpdateProviderModelRequest {
                description: Some(Some("speech to text".into())),
                ..update_req(&provider_id, "step-asr")
            })
            .await
            .unwrap();
        assert_eq!(resp.description.as_deref(), Some("speech to text"));
        assert_eq!(
            resp.tasks,
            vec![ModelTask::SpeechRecognition],
            "partial update leaves tasks untouched"
        );
        assert_eq!(resp.source, ProfileSource::Inferred, "no tasks/traits edit keeps source");
    }

    #[tokio::test]
    async fn update_clears_context_limit_with_explicit_null() {
        let (service, _, provider_id, _db) = setup("openai").await;
        service
            .create(CreateProviderModelRequest {
                context_limit: Some(128_000),
                ..create_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap();

        let resp = service
            .update(UpdateProviderModelRequest {
                context_limit: Some(None),
                ..update_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap();
        assert_eq!(resp.context_limit, None, "Some(None) clears the column");
    }

    #[tokio::test]
    async fn update_tasks_flips_source_to_user() {
        let (service, _, provider_id, _db) = setup("stepfun").await;
        let created = service.create(create_req(&provider_id, "step-asr")).await.unwrap();
        assert_eq!(created.source, ProfileSource::Inferred);

        let resp = service
            .update(UpdateProviderModelRequest {
                tasks: Some(vec![ModelTask::Chat]),
                ..update_req(&provider_id, "step-asr")
            })
            .await
            .unwrap();
        assert_eq!(resp.tasks, vec![ModelTask::Chat]);
        assert_eq!(resp.source, ProfileSource::User, "explicit tasks edit becomes user profile");
    }

    #[tokio::test]
    async fn update_missing_row_is_not_found() {
        let (service, _, provider_id, _db) = setup("openai").await;
        let err = service
            .update(UpdateProviderModelRequest {
                enabled: Some(false),
                ..update_req(&provider_id, "ghost-model")
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "missing row is NotFound: {err:?}");
    }

    #[tokio::test]
    async fn delete_removes_row_from_provider_projection() {
        let (service, provider_service, provider_id, _db) = setup("openai").await;
        service.create(create_req(&provider_id, "gpt-4o")).await.unwrap();

        assert!(service.delete(&provider_id, "gpt-4o").await.unwrap());
        assert!(!service.delete(&provider_id, "gpt-4o").await.unwrap(), "second delete is false");

        let providers = provider_service.list().await.unwrap();
        let provider = providers.iter().find(|p| p.provider_id == provider_id).unwrap();
        assert!(
            !provider.models.contains(&"gpt-4o".to_string()),
            "deleted row disappears from ProviderService::list()'s models projection"
        );
        assert!(provider.models_detail.is_empty());
    }

    #[tokio::test]
    async fn create_rejects_invalid_connection_role_before_writing() {
        let (service, _, provider_id, _db) = setup("openai").await;
        let err = service
            .create(CreateProviderModelRequest {
                connection_role: Some("Bad Role!".into()),
                ..create_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::BadRequest(ref message)
                    if message == "role must match ^[a-z][a-z0-9_-]{0,31}$"
            ),
            "unexpected error: {err:?}"
        );
        assert!(
            service.list(Some(&provider_id)).await.unwrap().is_empty(),
            "validation must run before any write; no half-created row may remain"
        );
    }

    #[tokio::test]
    async fn create_rejects_reserved_default_connection_role() {
        let (service, _, provider_id, _db) = setup("openai").await;
        let err = service
            .create(CreateProviderModelRequest {
                connection_role: Some("default".into()),
                ..create_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::BadRequest(ref message)
                    if message
                        == "role 'default' is reserved: the provider's own base_url/api_key is the default connection"
            ),
            "unexpected error: {err:?}"
        );
    }

    #[tokio::test]
    async fn update_validates_connection_role_set_but_not_clear() {
        let (service, _, provider_id, _db) = setup("openai").await;
        service.create(create_req(&provider_id, "gpt-4o")).await.unwrap();

        let err = service
            .update(UpdateProviderModelRequest {
                connection_role: Some(Some("bad!".into())),
                ..update_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::BadRequest(ref message)
                    if message == "role must match ^[a-z][a-z0-9_-]{0,31}$"
            ),
            "unexpected error: {err:?}"
        );

        // A valid role is accepted (Some(Some(valid))).
        let resp = service
            .update(UpdateProviderModelRequest {
                connection_role: Some(Some("voice".into())),
                ..update_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap();
        assert_eq!(resp.connection_role.as_deref(), Some("voice"));

        // Clearing with Some(None) must not run role validation.
        let resp = service
            .update(UpdateProviderModelRequest {
                connection_role: Some(None),
                ..update_req(&provider_id, "gpt-4o")
            })
            .await
            .unwrap();
        assert_eq!(resp.connection_role, None, "Some(None) clears the column");
    }

    #[tokio::test]
    async fn list_filters_by_provider() {
        let (service, _, provider_id, db) = setup("openai").await;
        let provider_repo = SqliteProviderRepository::new(db.pool().clone());
        let other_id = nomifun_common::ProviderId::new().into_string();
        provider_repo
            .create(CreateProviderParams {
                provider_id: Some(&other_id),
                platform: "deepseek",
                name: "Other",
                base_url: "https://y.test/v1",
                api_key_encrypted: "enc",
                models: "[]",
                enabled: true,
                capabilities: "[]",
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
        service.create(create_req(&provider_id, "gpt-4o")).await.unwrap();
        service.create(create_req(&other_id, "deepseek-chat")).await.unwrap();

        assert_eq!(service.list(None).await.unwrap().len(), 2);
        let one = service.list(Some(&provider_id)).await.unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].model, "gpt-4o");
        assert!(service.list(Some("not-a-uuid")).await.is_err());
    }
}
