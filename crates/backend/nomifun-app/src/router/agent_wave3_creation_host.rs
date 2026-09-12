//! Application-owned Wave 3 Creation capability host.
//!
//! The adapter is intentionally thin: Wave 3 owns the strict action DTOs,
//! `CreationService` owns validation, durable idempotency and task execution,
//! and the frozen `generation_provider` binding owns provider/model selection.
//! No success result is emitted until the service has returned the row written
//! by `ICreationTaskRepository::get_or_create_creative_task`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave3::{
    CreationAudioRequest, CreationImageEditRequest, CreationImageRequest,
    CreationTaskTarget, CreationTextRequest, CreationVideoRequest,
    CreationWorkbenchKind, Wave3CapabilityOperation, Wave3HostContext,
    Wave3HostPort, Wave3HostPortError, Wave3HostRequest,
    generation_provider_selection,
};
use nomifun_common::AppError;
use nomifun_creation::{
    CreationInput, CreationInputKind, CreationService, CreativeTaskOwner,
    NewCreationTask, StandaloneWorkbenchKind,
};
use nomifun_workshop::WorkshopService;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const CREATION_POLL_INTERVAL: Duration = Duration::from_millis(200);
const CREATION_WAIT_LIMIT: Duration = Duration::from_secs(300);
const WAVE3_CREATION_FAILED: &str = "WAVE3_CREATION_FAILED";
const WAVE3_CREATION_CANCELED: &str = "WAVE3_CREATION_CANCELED";
const WAVE3_CREATION_TIMEOUT_CANCELED: &str = "WAVE3_CREATION_TIMEOUT_CANCELED";
const WAVE3_CREATION_OUTCOME_UNKNOWN: &str = "WAVE3_CREATION_OUTCOME_UNKNOWN";

#[derive(Clone)]
pub(crate) struct Wave3CreationHost {
    creation: Arc<CreationService>,
    workshop: Arc<WorkshopService>,
}

impl Wave3CreationHost {
    pub(crate) fn new(
        creation: Arc<CreationService>,
        workshop: Arc<WorkshopService>,
    ) -> Self {
        Self { creation, workshop }
    }

    /// Erase the concrete adapter for `Wave3OwnerBindings::with_creation`.
    pub(crate) fn into_host_port(self) -> Arc<dyn Wave3HostPort> {
        Arc::new(self)
    }
}

impl Wave3HostPort for Wave3CreationHost {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            request.validate()?;
            self.workshop
                .require_creative_studio_owner(&request.context.principal.principal_id)
                .await
                .map_err(map_creation_error)?;
            let provider = generation_provider_selection(&request.context)?;
            let creation_task_id = stable_creation_task_id(&request.context);
            let (owner, mut task) = map_creation_operation(request.operation)?;
            task.params.insert(
                "_nomifun_connection_config_ref".into(),
                Value::String(provider.connection_config_ref.as_ref().to_owned()),
            );
            task.params.insert(
                "_nomifun_provider_config_revision".into(),
                json!(provider.config_revision),
            );
            let persisted = self
                .creation
                .create_creative_task(
                    owner,
                    creation_task_id,
                    NewCreationTask {
                        provider_id: provider.provider_id,
                        model: provider.model,
                        capability: task.capability,
                        params: Value::Object(task.params),
                        inputs: task.inputs,
                    },
                )
                .await
                .map_err(map_creation_error)?;
            let persisted = self.wait_for_terminal(persisted).await?;

            Ok(StrictJsonValue(json!({
                "creation_task_id": persisted.creation_task_id,
                "status": persisted.status,
                "result_asset_ids": persisted.result_asset_ids,
            })))
        })
    }
}

impl Wave3CreationHost {
    async fn wait_for_terminal(
        &self,
        mut task: nomifun_creation::CreationTask,
    ) -> Result<nomifun_creation::CreationTask, Wave3HostPortError> {
        let started = Instant::now();
        loop {
            match task.status.as_str() {
                "succeeded" => return Ok(task),
                "failed" => {
                    return Err(Wave3HostPortError::new(
                        WAVE3_CREATION_FAILED,
                        creation_failure_message(&task),
                    ));
                }
                "canceled" => {
                    return Err(Wave3HostPortError::new(
                        WAVE3_CREATION_CANCELED,
                        format!("creation task {} was canceled", task.creation_task_id),
                    ));
                }
                "queued" | "running" => {}
                status => {
                    return Err(Wave3HostPortError::new(
                        WAVE3_CREATION_OUTCOME_UNKNOWN,
                        format!(
                            "creation task {} has unknown status {status}",
                            task.creation_task_id
                        ),
                    ));
                }
            }
            if started.elapsed() >= CREATION_WAIT_LIMIT {
                return match self.creation.cancel_task(&task.creation_task_id).await {
                    Ok(canceled) if canceled.status == "canceled" => Err(Wave3HostPortError::new(
                        WAVE3_CREATION_TIMEOUT_CANCELED,
                        format!(
                            "creation task {} timed out and was durably canceled",
                            canceled.creation_task_id
                        ),
                    )),
                    Ok(completed) if completed.status == "succeeded" => Ok(completed),
                    Ok(completed) if completed.status == "failed" => Err(
                        Wave3HostPortError::new(WAVE3_CREATION_FAILED, creation_failure_message(&completed)),
                    ),
                    Ok(observed) => Err(Wave3HostPortError::new(
                        WAVE3_CREATION_OUTCOME_UNKNOWN,
                        format!(
                            "creation task {} timed out; cancellation returned {}. Observe the task before retrying",
                            observed.creation_task_id, observed.status
                        ),
                    )),
                    Err(error) => Err(Wave3HostPortError::new(
                        WAVE3_CREATION_OUTCOME_UNKNOWN,
                        format!(
                            "creation task {} timed out and cancellation could not be confirmed: {error}. Observe the task before retrying",
                            task.creation_task_id
                        ),
                    )),
                };
            }
            tokio::time::sleep(CREATION_POLL_INTERVAL).await;
            task = self
                .creation
                .get_task(&task.creation_task_id)
                .await
                .map_err(map_creation_error)?;
        }
    }
}

fn creation_failure_message(task: &nomifun_creation::CreationTask) -> String {
    task.error
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_else(|| format!("creation task {} failed", task.creation_task_id))
}

struct MappedCreationTask {
    capability: String,
    params: Map<String, Value>,
    inputs: Vec<CreationInput>,
}

fn map_creation_operation(
    operation: Wave3CapabilityOperation,
) -> Result<(CreativeTaskOwner, MappedCreationTask), Wave3HostPortError> {
    match operation {
        Wave3CapabilityOperation::CreationText(request) => {
            let owner = map_target(request.target.clone());
            Ok((owner, map_text(request)))
        }
        Wave3CapabilityOperation::CreationImage(request) => {
            let owner = map_target(request.target.clone());
            Ok((owner, map_image(request)))
        }
        Wave3CapabilityOperation::CreationImageEdit(request) => {
            let owner = map_target(request.target.clone());
            Ok((owner, map_image_edit(request)))
        }
        Wave3CapabilityOperation::CreationVideo(request) => {
            let owner = map_target(request.target.clone());
            Ok((owner, map_video(request)))
        }
        Wave3CapabilityOperation::CreationAudio(request) => {
            let owner = map_target(request.target.clone());
            Ok((owner, map_audio(request)))
        }
        other => Err(Wave3HostPortError::action_operation_mismatch(format!(
            "Creation host cannot execute {}",
            other.capability_id().as_ref()
        ))),
    }
}

fn map_target(target: CreationTaskTarget) -> CreativeTaskOwner {
    match target {
        CreationTaskTarget::CanvasNode { canvas_id, node_id } => {
            CreativeTaskOwner::CanvasNode { canvas_id, node_id }
        }
        CreationTaskTarget::StandaloneWorkbench { workbench_kind } => {
            CreativeTaskOwner::StandaloneWorkbench {
                workbench_kind: match workbench_kind {
                    CreationWorkbenchKind::Image => StandaloneWorkbenchKind::Image,
                    CreationWorkbenchKind::Video => StandaloneWorkbenchKind::Video,
                    CreationWorkbenchKind::Audio => StandaloneWorkbenchKind::Audio,
                },
            }
        }
        CreationTaskTarget::TemplateStep {
            template_id,
            template_run_id,
            template_step_id,
        } => CreativeTaskOwner::TemplateStep {
            template_id,
            template_run_id,
            template_step_id,
        },
    }
}

fn map_text(request: CreationTextRequest) -> MappedCreationTask {
    let mut params = Map::from_iter([
        ("prompt".to_owned(), Value::String(request.prompt)),
        ("max_tokens".to_owned(), json!(request.max_tokens)),
    ]);
    insert_optional(&mut params, "system", request.system);
    MappedCreationTask {
        capability: "text".to_owned(),
        params,
        inputs: Vec::new(),
    }
}

fn map_image(request: CreationImageRequest) -> MappedCreationTask {
    let mut params = Map::from_iter([
        ("prompt".to_owned(), Value::String(request.prompt)),
        ("count".to_owned(), json!(request.count)),
    ]);
    insert_optional(&mut params, "size", request.size);
    insert_optional(&mut params, "quality", request.quality);
    MappedCreationTask {
        capability: "t2i".to_owned(),
        params,
        inputs: Vec::new(),
    }
}

fn map_image_edit(request: CreationImageEditRequest) -> MappedCreationTask {
    let uses_mask = request
        .inputs
        .iter()
        .any(|input| input.role.as_str() == "mask");
    let inputs = request
        .inputs
        .into_iter()
        .map(|input| CreationInput {
            asset_id: input.asset_id,
            kind: CreationInputKind::Image,
            role: input.role.as_str().to_owned(),
        })
        .collect();
    let mut params = Map::from_iter([
        ("prompt".to_owned(), Value::String(request.prompt)),
        ("count".to_owned(), json!(request.count)),
    ]);
    insert_optional(&mut params, "size", request.size);
    MappedCreationTask {
        capability: if uses_mask { "inpaint" } else { "i2i" }.to_owned(),
        params,
        inputs,
    }
}

fn map_video(request: CreationVideoRequest) -> MappedCreationTask {
    let has_input = request.first_frame_asset_id.is_some();
    let mut inputs = Vec::with_capacity(2);
    if let Some(asset_id) = request.first_frame_asset_id {
        inputs.push(CreationInput {
            asset_id,
            kind: CreationInputKind::Image,
            role: "first_frame".to_owned(),
        });
    }
    if let Some(asset_id) = request.last_frame_asset_id {
        inputs.push(CreationInput {
            asset_id,
            kind: CreationInputKind::Image,
            role: "last_frame".to_owned(),
        });
    }
    let mut params = Map::from_iter([(
        "prompt".to_owned(),
        Value::String(request.prompt),
    )]);
    if let Some(seconds) = request.seconds {
        params.insert("seconds".to_owned(), json!(seconds));
    }
    insert_optional(&mut params, "size", request.size);
    MappedCreationTask {
        capability: if has_input { "i2v" } else { "t2v" }.to_owned(),
        params,
        inputs,
    }
}

fn map_audio(request: CreationAudioRequest) -> MappedCreationTask {
    // CreationService's canonical prompt accessor feeds SpeechSynthesis.text.
    let mut params = Map::from_iter([(
        "prompt".to_owned(),
        Value::String(request.text),
    )]);
    insert_optional(&mut params, "voice", request.voice);
    insert_optional(&mut params, "format", request.format);
    MappedCreationTask {
        capability: "tts".to_owned(),
        params,
        inputs: Vec::new(),
    }
}

fn insert_optional(params: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        params.insert(key.to_owned(), Value::String(value));
    }
}

/// Creation's public HTTP contract requires a UUIDv7 idempotency key, while
/// AgentPlatform keys are opaque strings scoped by principal/session/action.
/// Convert that complete stable scope into a deterministic RFC UUID carrying
/// the v7 marker. A replay maps to the same row; a changed payload under the
/// same key is still rejected by CreationService's request fingerprint.
fn stable_creation_task_id(context: &Wave3HostContext) -> String {
    let mut digest = Sha256::new();
    for part in [
        context.principal.principal_kind.as_str(),
        context.principal.principal_id.as_str(),
        context.agent_session_id.as_ref(),
        context.capability_id.as_ref(),
        context.idempotency_key.as_ref(),
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn map_creation_error(error: AppError) -> Wave3HostPortError {
    Wave3HostPortError::new(error.error_code(), error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, ConnectionConfigRef, CorrelationId,
        IdempotencyKey, OperationId, PrincipalRef, ResolvedSnapshotRef,
        ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
        TypedResourceBinding,
    };
    use nomifun_common::{ProviderId, UserId, generate_id};
    use nomifun_db::{
        SqliteCreationTaskRepository, SqliteWorkshopRepository,
        init_database_memory_with_owner,
    };

    use super::*;

    async fn seed_image_provider(database: &nomifun_db::Database) -> String {
        let provider_id = ProviderId::new().into_string();
        sqlx::query(
            "INSERT INTO providers \
                (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                 created_at, updated_at) \
             VALUES (?, 'openai', 'Wave 3 Creation Test', 'https://example.invalid', \
                 'bearer', '', 1, 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("seed provider");
        sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
             VALUES (?, 'image-model-v1', 1, 0, NULL, 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("seed model");
        sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, \
                 provider_params, created_at, updated_at) \
             VALUES (?, 'image-model-v1', 'image_generation', '[]', \
                 'openai.images', 'default', '{}', 0, 0)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .expect("seed capability");
        provider_id
    }

    fn image_request(
        owner_id: &str,
        provider_id: &str,
        config_revision: i64,
        idempotency_key: &str,
    ) -> Wave3HostRequest {
        Wave3HostRequest {
            context: Wave3HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                agent_session_id: AgentSessionId::from(generate_id()),
                operation_id: OperationId::from("wave3-creation-operation"),
                idempotency_key: IdempotencyKey::from(idempotency_key.to_owned()),
                correlation_id: CorrelationId::from("wave3-creation-correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "wave3-creation-snapshot".into(),
                    snapshot_digest: "a".repeat(64).into(),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from("creation.image"),
                action_id: ActionId::from("creation.image.invoke"),
                state_scope_key: ScopeKey::from("session:wave3-creation"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("generation-provider"),
                    resource_kind: ResourceKind::from("generation_provider"),
                    resource_id: ResourceId::from(provider_id.to_owned()),
                    owner_id: owner_id.to_owned(),
                    operations: BTreeSet::from(["image".to_owned()]),
                    connection_config_ref: Some(ConnectionConfigRef::from(format!(
                        "provider:{provider_id}@{config_revision}"
                    ))),
                    typed_parameters: BTreeMap::from([(
                        "model.creation.image".to_owned(),
                        "image-model-v1".to_owned(),
                    )]),
                }],
            },
            operation: Wave3CapabilityOperation::CreationImage(CreationImageRequest {
                target: CreationTaskTarget::StandaloneWorkbench {
                    workbench_kind: CreationWorkbenchKind::Image,
                },
                prompt: "a real persisted image task".to_owned(),
                count: 1,
                size: Some("1024x1024".to_owned()),
                quality: None,
            }),
        }
    }

    #[tokio::test]
    async fn image_creation_returns_only_after_real_repository_insert_and_replays_one_row() {
        let owner_id = UserId::new();
        let database = init_database_memory_with_owner(owner_id.clone())
            .await
            .expect("database");
        let provider_id = seed_image_provider(&database).await;
        let config_revision: i64 =
            sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
                .bind(&provider_id)
                .fetch_one(database.pool())
                .await
                .expect("provider revision");
        let repo = Arc::new(SqliteCreationTaskRepository::new(database.pool().clone()));
        let data_dir = tempfile::tempdir().expect("data dir");
        let workshop = WorkshopService::start(
            data_dir.path(),
            Arc::new(SqliteWorkshopRepository::new(database.pool().clone())),
        );
        let host = Wave3CreationHost::new(CreationService::new(repo), workshop);
        let session_id = AgentSessionId::from(generate_id());
        let mut request = image_request(
            owner_id.as_str(),
            &provider_id,
            config_revision,
            "opaque-agent-effect-key",
        );
        request.context.agent_session_id = session_id.clone();
        let task_id = stable_creation_task_id(&request.context);

        let first = host
            .invoke(request.clone())
            .await
            .expect_err("unconfigured provider must be a non-success Tool result");
        let retry = host.invoke(request).await.expect_err("idempotent failed replay");
        assert_eq!(first.code, WAVE3_CREATION_FAILED);
        assert_eq!(retry.code, first.code);
        let row: (String, String, String, String, String) = sqlx::query_as(
            "SELECT creation_task_id, provider_id, model, capability, workbench_kind \
             FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(&task_id)
        .fetch_one(database.pool())
        .await
        .expect("repository side effect");
        assert_eq!(row.1, provider_id);
        assert_eq!(row.2, "image-model-v1");
        assert_eq!(row.3, "t2i");
        assert_eq!(row.4, "image");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(&row.0)
        .fetch_one(database.pool())
        .await
        .expect("task count");
        assert_eq!(count, 1, "retry must not manufacture a second task");
    }

    #[tokio::test]
    async fn missing_bound_model_fails_before_any_repository_side_effect() {
        let owner_id = UserId::new();
        let database = init_database_memory_with_owner(owner_id.clone())
            .await
            .expect("database");
        let provider_id = seed_image_provider(&database).await;
        let config_revision: i64 =
            sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
                .bind(&provider_id)
                .fetch_one(database.pool())
                .await
                .expect("provider revision");
        let repo = Arc::new(SqliteCreationTaskRepository::new(database.pool().clone()));
        let data_dir = tempfile::tempdir().expect("data dir");
        let workshop = WorkshopService::start(
            data_dir.path(),
            Arc::new(SqliteWorkshopRepository::new(database.pool().clone())),
        );
        let host = Wave3CreationHost::new(CreationService::new(repo), workshop);
        let mut request = image_request(
            owner_id.as_str(),
            &provider_id,
            config_revision,
            "missing-model-key",
        );
        request.context.resource_bindings[0].typed_parameters.clear();

        let error = host.invoke(request).await.expect_err("model is required");
        assert_eq!(error.code, "WAVE3_RESOURCE_BINDING_INVALID");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks")
            .fetch_one(database.pool())
            .await
            .expect("task count");
        assert_eq!(count, 0);
    }
}
