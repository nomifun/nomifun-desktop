use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nomifun_api_types::{
    ModelProfile, ModelProfileUpsertRequest, ModelTask, ModelTrait, ProfileSource,
};
use nomifun_common::{AppError, ProviderId};
use nomifun_db::{IProviderModelRepository, NewProviderModel, ProviderModelRow, ProviderModelUpdate};

/// Business logic for authoritative per-model capability profiles (the
/// multimodal model hub). Since migration 015 retired `model_profiles`, the
/// profile fields live on the authoritative `provider_models` rows; this
/// service projects those rows to the unchanged `/api/model-profiles*` wire
/// shape. CRUD only — "resolve models by capability" is composed at the route
/// layer from the provider list + these profiles.
#[derive(Clone)]
pub struct ModelProfileService {
    repo: Arc<dyn IProviderModelRepository>,
}

impl ModelProfileService {
    pub fn new(repo: Arc<dyn IProviderModelRepository>) -> Self {
        Self { repo }
    }

    /// All stored profiles across all providers.
    pub async fn list(&self) -> Result<Vec<ModelProfile>, AppError> {
        let rows = self.repo.list().await?;
        rows.iter().map(row_to_profile_view).collect()
    }

    /// Insert or replace one profile. `source` defaults to `User` (this is the
    /// user-edit endpoint), making the stored profile authoritative over the
    /// name heuristic.
    ///
    /// A model already present in the catalog gets a partial update of the
    /// profile columns only (enabled/sort_order/protocol/… are untouched); a
    /// model not yet in the catalog gets a fresh enabled row appended after
    /// the provider's current ordering.
    pub async fn upsert(&self, req: ModelProfileUpsertRequest) -> Result<ModelProfile, AppError> {
        let provider_id = ProviderId::parse(req.provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?
            .into_string();
        if req.model.trim().is_empty() {
            return Err(AppError::BadRequest("model is required".into()));
        }
        let model = req.model.trim();
        let tasks_json = serde_json::to_string(&req.tasks)
            .map_err(|e| AppError::Internal(format!("serialize tasks: {e}")))?;
        let traits_json = serde_json::to_string(&req.traits)
            .map_err(|e| AppError::Internal(format!("serialize traits: {e}")))?;
        let params_value = req.params.unwrap_or_else(|| serde_json::json!({}));
        let params_json = serde_json::to_string(&params_value)
            .map_err(|e| AppError::Internal(format!("serialize params: {e}")))?;
        let source = req.source.unwrap_or(ProfileSource::User);
        let source_str = source_to_str(source);

        let profile_update = ProviderModelUpdate {
            tasks: Some(&tasks_json),
            traits: Some(&traits_json),
            params: Some(&params_json),
            source: Some(source_str),
            ..Default::default()
        };
        let row = if self.repo.get(&provider_id, model).await?.is_some() {
            self.repo.update(&provider_id, model, &profile_update).await?
        } else {
            let next_sort = next_sort_order(self.repo.as_ref(), &provider_id).await?;
            match self
                .repo
                .create(
                    &provider_id,
                    &NewProviderModel {
                        model,
                        enabled: true,
                        sort_order: next_sort,
                        tasks: &tasks_json,
                        traits: &traits_json,
                        params: &params_json,
                        source: source_str,
                        ..Default::default()
                    },
                )
                .await
            {
                Ok(row) => row,
                // Lost a create race (e.g. a concurrent catalog sync inserted
                // the row): fall back to the update path so upsert semantics
                // hold. A missing provider also surfaces as Conflict and then
                // fails the update with NotFound/Conflict as before.
                Err(nomifun_db::DbError::Conflict(_))
                    if self.repo.get(&provider_id, model).await?.is_some() =>
                {
                    self.repo.update(&provider_id, model, &profile_update).await?
                }
                Err(error) => return Err(error.into()),
            }
        };
        row_to_profile_view(&row)
    }

    /// Delete one profile; returns whether a row was removed. On the
    /// converged store this removes the catalog row itself.
    pub async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, AppError> {
        ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?;
        Ok(self.repo.delete(provider_id, model).await?)
    }

    /// Atomically seed inferred profiles for newly discovered catalog models.
    /// Existing user/catalog rows are never overwritten; see
    /// [`seed_missing_inferred_profiles`].
    pub async fn seed_missing_inferred<S>(
        &self,
        provider_id: &str,
        platform: &str,
        models: &[S],
    ) -> Result<usize, AppError>
    where
        S: AsRef<str> + Sync,
    {
        seed_missing_inferred_profiles(self.repo.as_ref(), provider_id, platform, models).await
    }
}

/// Reconcile inferred capability profiles for a provider's catalog models.
///
/// Two passes over the given catalog list:
/// - models with no `provider_models` row are inserted (`insert_if_absent`)
///   with tasks/traits from [`nomifun_api_types::derive_tasks_and_traits`] and
///   `source = "inferred"`;
/// - existing rows that were created by membership dual-write and never
///   profiled (`tasks == "[]"` AND `source == "inferred"`) get their
///   tasks/traits backfilled from the same heuristic (source stays inferred).
///
/// The repository's atomic `insert_if_absent` primitive is intentional. A
/// refresh runs in the background and may race with a user editing the same
/// profile; a prior `list`/`get` followed by an unconditional upsert could
/// overwrite that newer user choice. The backfill pass only ever touches
/// unprofiled inferred rows, so a concurrent user edit (which sets
/// `source = "user"`) wins.
///
/// Returns the number of rows seeded or backfilled.
pub async fn seed_missing_inferred_profiles<S>(
    repo: &dyn IProviderModelRepository,
    provider_id: &str,
    platform: &str,
    models: &[S],
) -> Result<usize, AppError>
where
    S: AsRef<str> + Sync,
{
    let provider_id = ProviderId::parse(provider_id)
        .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?;

    let platform = platform.trim();
    let existing = repo.list_for_provider(provider_id.as_str()).await?;
    let mut next_sort = existing
        .iter()
        .map(|row| row.sort_order)
        .max()
        .map_or(0, |max| max + 1);
    let by_model: HashMap<&str, &ProviderModelRow> =
        existing.iter().map(|row| (row.model.as_str(), row)).collect();

    let mut seen = HashSet::new();
    let mut changed = 0usize;
    for raw_model in models {
        let model = raw_model.as_ref().trim();
        if model.is_empty() || !seen.insert(model.to_owned()) {
            continue;
        }
        let (tasks, traits) = nomifun_api_types::derive_tasks_and_traits(platform, model);
        let tasks_json = serde_json::to_string(&tasks)
            .map_err(|error| AppError::Internal(format!("serialize inferred tasks: {error}")))?;
        let traits_json = serde_json::to_string(&traits)
            .map_err(|error| AppError::Internal(format!("serialize inferred traits: {error}")))?;

        match by_model.get(model) {
            // Membership dual-write inserts rows with empty tasks; give those
            // (and only those) the heuristic profile. Anything a user or an
            // earlier seed already profiled is left untouched.
            Some(row) if row.tasks == "[]" && row.source == "inferred" => {
                repo.update(
                    provider_id.as_str(),
                    model,
                    &ProviderModelUpdate {
                        tasks: Some(&tasks_json),
                        traits: Some(&traits_json),
                        ..Default::default()
                    },
                )
                .await?;
                changed += 1;
            }
            Some(_) => {}
            None => {
                if repo
                    .insert_if_absent(
                        provider_id.as_str(),
                        &NewProviderModel {
                            model,
                            enabled: true,
                            sort_order: next_sort,
                            tasks: &tasks_json,
                            traits: &traits_json,
                            params: "{}",
                            source: "inferred",
                            ..Default::default()
                        },
                    )
                    .await?
                {
                    changed += 1;
                    next_sort += 1;
                }
            }
        }
    }
    Ok(changed)
}

/// Next append position for a provider's catalog.
async fn next_sort_order(
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

fn source_to_str(source: ProfileSource) -> &'static str {
    match source {
        ProfileSource::Inferred => "inferred",
        ProfileSource::User => "user",
    }
}

fn source_from_str(s: &str) -> ProfileSource {
    match s {
        "user" => ProfileSource::User,
        _ => ProfileSource::Inferred,
    }
}

/// Project an authoritative `provider_models` row to the wire [`ModelProfile`].
/// Malformed JSON degrades gracefully (empty tasks/traits, `{}` params) with a
/// warning instead of erroring, so one bad row never breaks the whole listing
/// (same tolerance strategy as the `ProviderModelResponse` projection). Only a
/// non-canonical stored `provider_id` is a hard error.
pub fn row_to_profile_view(row: &ProviderModelRow) -> Result<ModelProfile, AppError> {
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
            "invalid provider_models.params JSON; degrading to empty params"
        );
        serde_json::json!({})
    });
    Ok(ModelProfile {
        provider_id: row.provider_id.clone(),
        model: row.model.clone(),
        tasks,
        traits,
        params,
        source: source_from_str(&row.source),
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{
        CreateProviderParams, IProviderRepository, SqliteProviderModelRepository,
        SqliteProviderRepository, init_database_memory,
    };

    async fn seed_provider(db: &nomifun_db::Database, models_json: &str) -> String {
        let provider_repo = SqliteProviderRepository::new(db.pool().clone());
        let provider_id = ProviderId::new().into_string();
        provider_repo
            .create(CreateProviderParams {
                provider_id: Some(&provider_id),
                platform: "nomifun-free-model",
                name: "Managed",
                base_url: "http://127.0.0.1:1/v1",
                api_key_encrypted: "encrypted",
                models: models_json,
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
        provider_id
    }

    #[tokio::test]
    async fn inferred_seed_is_idempotent_and_preserves_user_profile() {
        let db = init_database_memory().await.unwrap();
        let provider_id = seed_provider(&db, "[]").await;
        let repo = SqliteProviderModelRepository::new(db.pool().clone());
        repo.create(
            &provider_id,
            &NewProviderModel {
                model: "big-pickle",
                enabled: true,
                sort_order: 0,
                tasks: r#"["chat"]"#,
                traits: r#"["vision_input"]"#,
                params: r#"{"owner":"user"}"#,
                source: "user",
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let models = vec![
            "big-pickle".to_string(),
            "deepseek-v4-flash-free".to_string(),
            "deepseek-v4-flash-free".to_string(),
            " ".to_string(),
        ];
        assert_eq!(
            seed_missing_inferred_profiles(&repo, &provider_id, "nomifun-free-model", &models)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            seed_missing_inferred_profiles(&repo, &provider_id, "nomifun-free-model", &models)
                .await
                .unwrap(),
            0
        );

        let user = repo.get(&provider_id, "big-pickle").await.unwrap().unwrap();
        assert_eq!(user.source, "user");
        assert_eq!(user.params, r#"{"owner":"user"}"#);
        assert_eq!(user.tasks, r#"["chat"]"#, "user profile is never re-derived");
        let inferred = repo
            .get(&provider_id, "deepseek-v4-flash-free")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inferred.source, "inferred");
        assert!(inferred.enabled);
        assert_eq!(inferred.sort_order, 1, "seeded row appends after the catalog");
        assert_ne!(inferred.tasks, "[]", "seeded row carries derived tasks");
    }

    #[tokio::test]
    async fn seed_backfills_unprofiled_dual_write_rows_only() {
        let db = init_database_memory().await.unwrap();
        // Membership dual-write on provider create leaves tasks='[]',
        // source='inferred' — exactly the shape the backfill pass targets.
        let provider_id = seed_provider(&db, r#"["gpt-4o","big-pickle"]"#).await;
        let repo = SqliteProviderModelRepository::new(db.pool().clone());
        // A user profiles one of the two models before the refresh runs.
        repo.update(
            &provider_id,
            "big-pickle",
            &ProviderModelUpdate {
                tasks: Some(r#"["image_generation"]"#),
                source: Some("user"),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let models = ["gpt-4o".to_string(), "big-pickle".to_string()];
        assert_eq!(
            seed_missing_inferred_profiles(&repo, &provider_id, "openai", &models)
                .await
                .unwrap(),
            1,
            "only the unprofiled inferred row is backfilled"
        );
        let gpt = repo.get(&provider_id, "gpt-4o").await.unwrap().unwrap();
        assert_eq!(gpt.tasks, r#"["chat"]"#);
        assert_eq!(gpt.traits, r#"["vision_input"]"#);
        assert_eq!(gpt.source, "inferred", "backfill keeps the source inferred");
        let user = repo.get(&provider_id, "big-pickle").await.unwrap().unwrap();
        assert_eq!(user.tasks, r#"["image_generation"]"#, "user profile untouched");

        assert_eq!(
            seed_missing_inferred_profiles(&repo, &provider_id, "openai", &models)
                .await
                .unwrap(),
            0,
            "backfill is idempotent"
        );
    }

    #[tokio::test]
    async fn upsert_updates_existing_row_and_creates_missing_row() {
        let db = init_database_memory().await.unwrap();
        let provider_id = seed_provider(&db, r#"["step-image-edit-2"]"#).await;
        let repo = std::sync::Arc::new(SqliteProviderModelRepository::new(db.pool().clone()));
        let service = ModelProfileService::new(repo.clone());

        // Existing catalog row → profile-columns-only update.
        let profile = service
            .upsert(ModelProfileUpsertRequest {
                provider_id: provider_id.clone(),
                model: "step-image-edit-2".into(),
                tasks: vec![ModelTask::ImageGeneration, ModelTask::ImageEdit],
                traits: vec![],
                params: Some(serde_json::json!({"steps": 8})),
                source: None,
            })
            .await
            .unwrap();
        assert_eq!(profile.source, ProfileSource::User);
        assert_eq!(profile.tasks, vec![ModelTask::ImageGeneration, ModelTask::ImageEdit]);
        let row = repo.get(&provider_id, "step-image-edit-2").await.unwrap().unwrap();
        assert_eq!(row.sort_order, 0, "update keeps catalog placement");
        assert!(row.enabled);

        // Unknown model → fresh enabled row appended after the catalog.
        let profile = service
            .upsert(ModelProfileUpsertRequest {
                provider_id: provider_id.clone(),
                model: "tts-new".into(),
                tasks: vec![ModelTask::SpeechSynthesis],
                traits: vec![],
                params: None,
                source: None,
            })
            .await
            .unwrap();
        assert_eq!(profile.params, serde_json::json!({}));
        let row = repo.get(&provider_id, "tts-new").await.unwrap().unwrap();
        assert!(row.enabled);
        assert_eq!(row.sort_order, 1);
        assert_eq!(row.source, "user");

        // Delete removes the row.
        assert!(service.delete(&provider_id, "tts-new").await.unwrap());
        assert!(!service.delete(&provider_id, "tts-new").await.unwrap());
    }

    #[test]
    fn profile_view_is_tolerant_of_bad_json() {
        let row = ProviderModelRow {
            id: 1,
            provider_id: "0190f5fe-7c00-7a00-8abc-012345678901".into(),
            model: "m".into(),
            enabled: true,
            sort_order: 0,
            tasks: "not-json".into(),
            traits: "{broken".into(),
            protocol: None,
            connection_role: None,
            params: "###".into(),
            context_limit: None,
            description: None,
            source: "user".into(),
            health: None,
            health_checked_at: None,
            created_at: 1,
            updated_at: 2,
        };
        let profile = row_to_profile_view(&row).unwrap();
        assert!(profile.tasks.is_empty());
        assert!(profile.traits.is_empty());
        assert_eq!(profile.params, serde_json::json!({}));
        assert_eq!(profile.source, ProfileSource::User);
    }
}
