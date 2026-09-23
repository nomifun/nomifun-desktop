//! Application-owned Wave 3 Creation capability host.
//!
//! The adapter is intentionally thin: Wave 3 owns the strict action DTOs,
//! `CreationService` owns validation, durable idempotency and task execution,
//! and task-aware routing selects an exact provider/model at admission.
//! The Tool returns the actual durable task status immediately after the row is written
//! by `ICreationTaskRepository::get_or_create_creative_task`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave3::{
    CreationAudioRequest, CreationImageEditRequest, CreationImageRequest, CreationMusicRequest,
    CreationTaskTarget, CreationTextRequest, CreationVideoRequest, Wave3CapabilityOperation,
    Wave3HostContext,
    Wave3HostPort, Wave3HostPortError, Wave3HostRequest,
};
use nomifun_common::AppError;
use nomifun_creation::{
    CreationInput, CreationInputKind, CreationService, CreativeTaskOwner,
    NewCreationTask,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct Wave3CreationHost {
    creation: Arc<CreationService>,
    invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    pool: nomifun_db::SqlitePool,
    owner_id: Arc<str>,
}

impl Wave3CreationHost {
    pub(crate) fn new(
        creation: Arc<CreationService>,
        invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
        pool: nomifun_db::SqlitePool,
        owner_id: Arc<str>,
    ) -> Self {
        Self { creation, invoke, pool, owner_id }
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
            if request.context.principal.principal_kind != "user" || request.context.principal.principal_id != self.owner_id.as_ref() {
                return Err(Wave3HostPortError::invalid_request("generation owner does not match the authenticated installation owner"));
            }
            let model_task = creation_model_task(request.context.action_id.as_ref())?;
            let creation_task_id = stable_creation_task_id(&request.context);
            let (owner, mut task) = map_creation_operation(request.operation)?;
            self.validate_task_owner(&request.context, &owner).await?;
            match self.creation.get_task(&creation_task_id).await {
                Ok(existing) => {
                    let owner_matches = match &owner {
                        CreativeTaskOwner::ConversationTurn { conversation_id, message_id } => existing.conversation_id.as_ref() == Some(conversation_id) && existing.message_id.as_ref() == Some(message_id),
                        CreativeTaskOwner::CanvasNode { canvas_id, node_id } => existing.canvas_id.as_ref() == Some(canvas_id) && existing.node_id.as_ref() == Some(node_id),
                        CreativeTaskOwner::TemplateStep { template_id, template_run_id, template_step_id } => existing.template_id.as_ref() == Some(template_id) && existing.template_run_id.as_ref() == Some(template_run_id) && existing.template_step_id.as_ref() == Some(template_step_id),
                    };
                    if !owner_matches || existing.capability != task.capability { return Err(Wave3HostPortError::invalid_request("generation replay owner or capability differs")); }
                    let request_matches = existing.params.as_object().is_some_and(|params| {
                        let unfrozen = params.iter().filter(|(key, _)| !key.starts_with("_nomifun_")).map(|(key, value)| (key.clone(), value.clone())).collect::<Map<_, _>>();
                        unfrozen == task.params
                    });
                    if !request_matches || existing.inputs.as_deref().unwrap_or_default() != task.inputs.as_slice() { return Err(Wave3HostPortError::invalid_request("generation replay input differs from the accepted task")); }

                    return Ok(StrictJsonValue(json!({"creation_task_id": existing.creation_task_id, "status": existing.status, "result_asset_ids": existing.result_asset_ids})));
                }
                Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(map_creation_error(error)),
            }
            let provider = self.resolve_model(model_task).await?;
            task.params.insert(
                "_nomifun_connection_config_ref".into(),
                Value::String(format!("provider:{}@{}", provider.provider_id, provider.config_revision)),
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

            Ok(StrictJsonValue(json!({
                "creation_task_id": persisted.creation_task_id,
                "status": persisted.status,
                "result_asset_ids": persisted.result_asset_ids,
            })))
        })
    }
}

impl Wave3CreationHost {
    async fn resolve_model(&self, task: nomifun_api_types::ModelTask) -> Result<nomifun_model_invoke::ResolvedTaskConfig, Wave3HostPortError> {
        let preferences = nomifun_db::SqliteClientPreferenceRepository::new(self.pool.clone());
        let result = self
            .invoke
            .resolve_automatic_task_model(task, &preferences)
            .await;
        result.map_err(|error| Wave3HostPortError::new("GENERATION_MODEL_UNAVAILABLE", error.to_string()))
    }

    async fn validate_task_owner(&self, context: &Wave3HostContext, owner: &CreativeTaskOwner) -> Result<(), Wave3HostPortError> {
        if let CreativeTaskOwner::ConversationTurn { conversation_id, message_id } = owner {
            if conversation_id != context.agent_session_id.as_ref() {
                return Err(Wave3HostPortError::invalid_request("generation target must be the current conversation"));
            }
            let store = nomifun_agent_session::AgentSessionStore::from_pool(self.pool.clone())
                .await
                .map_err(|error| Wave3HostPortError::invalid_request(error.to_string()))?;
            let session_id = nomifun_agent_contracts::AgentSessionId::from(conversation_id.clone());
            let session = store
                .get_live_session(&session_id)
                .await
                .map_err(|error| Wave3HostPortError::invalid_request(error.to_string()))?;
            if session.owner_ref != context.principal {
                return Err(Wave3HostPortError::invalid_request(
                    "generation target belongs to another owner",
                ));
            }
            let head = store
                .head(&session_id)
                .await
                .map_err(|error| Wave3HostPortError::invalid_request(error.to_string()))?;
            let operation = head.active_turn_id.ok_or_else(|| {
                Wave3HostPortError::invalid_request(
                    "generation target has no active canonical Turn",
                )
            })?;
            let receipt = store
                .read_turn_receipt(
                    &session_id,
                    &nomifun_agent_contracts::OperationId::from(operation),
                )
                .await
                .map_err(|error| Wave3HostPortError::invalid_request(error.to_string()))?;
            let admitted_message = receipt.started_event.and_then(|event| match event.payload {
                nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => payload
                    .0
                    .get("source_message_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            });
            if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running
                || admitted_message.as_deref() != Some(message_id.as_str())
            {
                return Err(Wave3HostPortError::invalid_request(
                    "generation target must reference the admitted current user turn",
                ));
            }
        }
        Ok(())
    }
}

fn creation_model_task(action_id: &str) -> Result<nomifun_api_types::ModelTask, Wave3HostPortError> {
    use nomifun_api_types::ModelTask;
    Ok(match action_id {
        "creation.media/text" => ModelTask::Chat,
        "creation.media/image" => ModelTask::ImageGeneration,
        "creation.media/image_edit" => ModelTask::ImageEdit,
        "creation.media/video" => ModelTask::VideoGeneration,
        "creation.media/music" => ModelTask::MusicGeneration,
        "creation.media/audio" => ModelTask::SpeechSynthesis,
        _ => return Err(Wave3HostPortError::invalid_request("unsupported Creation action")),
    })
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
        Wave3CapabilityOperation::CreationMusic(request) => {
            let owner = map_target(request.target.clone());
            Ok((owner, map_music(request)))
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
        CreationTaskTarget::ConversationTurn { conversation_id, message_id } => CreativeTaskOwner::ConversationTurn { conversation_id, message_id },
        CreationTaskTarget::CanvasNode { canvas_id, node_id } => {
            CreativeTaskOwner::CanvasNode { canvas_id, node_id }
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
    insert_optional(&mut params, "quality", request.quality);
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
    insert_optional(&mut params, "resolution", request.resolution);
    params.insert("count".to_owned(), json!(request.count));
    MappedCreationTask {
        capability: if has_input { "i2v" } else { "t2v" }.to_owned(),
        params,
        inputs,
    }
}

fn map_music(request: CreationMusicRequest) -> MappedCreationTask {
    let mut params = Map::from_iter([("prompt".to_owned(), json!(request.prompt)), ("instrumental".to_owned(), json!(request.instrumental))]);
    insert_optional(&mut params, "lyrics", request.lyrics);
    insert_optional(&mut params, "format", request.format);
    MappedCreationTask { capability: "music".to_owned(), params, inputs: Vec::new() }
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
        context.action_id.as_ref(),
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
    use nomifun_agent_contracts::{ActionId, AgentSessionId, CapabilityId, CorrelationId, IdempotencyKey, OperationId, PrincipalRef, ResolvedSnapshotRef, ScopeKey};
    use nomifun_common::{UserId, generate_id};
    use nomifun_db::{SqliteCreationTaskRepository, init_database_memory_with_owner};
    use super::*;

    fn context(owner: &str) -> Wave3HostContext {
        Wave3HostContext {
            principal: PrincipalRef { principal_kind: "user".into(), principal_id: owner.into() },
            agent_session_id: AgentSessionId::from(generate_id()), operation_id: OperationId::from("effect"),
            idempotency_key: IdempotencyKey::from("stable-effect"), correlation_id: CorrelationId::from("turn"),
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: generate_id().into(), snapshot_digest: "a".repeat(64).into() },
            registry_generation: 1, capability_id: CapabilityId::from(nomifun_agent_domain_wave3::CREATION_MEDIA_MODULE_ID), action_id: ActionId::from("creation.media/image"),
            state_scope_key: ScopeKey::from("session:test"), resource_bindings: vec![],
        }
    }

    fn host(pool: nomifun_db::SqlitePool, owner: &str) -> Wave3CreationHost {
        let invoke = Arc::new(nomifun_model_invoke::ModelInvokeService::new(
            Arc::new(nomifun_db::SqliteProviderRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(pool.clone())),
            [0; 32], reqwest::Client::new(), nomifun_model_invoke::AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
        ));
        Wave3CreationHost::new(CreationService::new(Arc::new(SqliteCreationTaskRepository::new(pool.clone()))), invoke, pool, Arc::from(owner))
    }

    async fn configured_media_models(pool: &nomifun_db::SqlitePool) -> std::collections::BTreeMap<&'static str, (String, String)> {
        use nomifun_db::{CreateProviderParams, IClientPreferenceRepository, IProviderRepository, NewProviderModel, NewProviderModelCapability};
        let providers = nomifun_db::SqliteProviderRepository::new(pool.clone());
        let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":["host-test-key"]}"#, &[0; 32]).unwrap();
        let mut result = std::collections::BTreeMap::new();
        for (action, task, protocol, platform) in [
            ("creation.media/image", "image_generation", "openai.images", "openai"),
            ("creation.media/image_edit", "image_edit", "openai.images", "openai"),
            ("creation.media/video", "video_generation", "openai.videos", "openai"),
            ("creation.media/music", "music_generation", "minimax.music", "minimax"),
            ("creation.media/audio", "speech_synthesis", "openai.audio_speech", "openai"),
        ] {
            let provider_id = generate_id();
            let model = format!("{task}-route");
            providers.create(CreateProviderParams { provider_id: Some(&provider_id), platform, name: &model,
                    base_url: "https://example.com/v1", auth_scheme: "bearer", credentials_encrypted: &encrypted,
                    enabled: true, bedrock_config: None, sort_order: None,
                }, &NewProviderModel { model: &model, enabled: true, sort_order: 0, description: None,
                    capabilities: &[NewProviderModelCapability { task, protocol, traits: "[]", connection_role: "default", provider_params: "{}", ..Default::default() }],
                }, &[]).await.unwrap();
            let preference_key = match task {
                "image_generation" => "models.default.imageGeneration",
                "image_edit" => "models.default.imageEdit",
                "video_generation" => "models.default.videoGeneration",
                "music_generation" => "models.default.musicGeneration",
                "speech_synthesis" => "models.default.speechSynthesis",
                _ => unreachable!(),
            };
            let preference = serde_json::json!({
                "provider_id": &provider_id,
                "model": &model,
            })
            .to_string();
            nomifun_db::SqliteClientPreferenceRepository::new(pool.clone())
                .upsert_batch(&[(preference_key, preference.as_str())])
                .await
                .unwrap();
            result.insert(action, (provider_id, model));
        }
        result
    }

    #[tokio::test]
    async fn product_actions_resolve_their_configured_task_routes_without_provider_input() {
        use nomifun_db::IClientPreferenceRepository;

        let owner = UserId::new(); let database = init_database_memory_with_owner(owner.clone()).await.unwrap();
        let host = host(database.pool().clone(), owner.as_str());
        let models = configured_media_models(database.pool()).await;
        for (action, (provider_id, model)) in &models {
            let task = creation_model_task(action).unwrap();
            let mut input = json!({"target":{"kind":"canvas_node", "canvas_id":generate_id(), "node_id":generate_id()},
                "prompt":"create"});
            if *action == "creation.media/audio" { input.as_object_mut().unwrap().remove("prompt"); input["text"] = json!("speak"); }
            if *action == "creation.media/image_edit" { input["inputs"] = json!([{"asset_id":generate_id(),"role":"reference"}]); }
            nomifun_agent_domain_wave3::operation_from_input(
                &CapabilityId::from(nomifun_agent_domain_wave3::CREATION_MEDIA_MODULE_ID),
                &ActionId::from(*action),
                StrictJsonValue(input),
            ).unwrap();
            let resolved = host.resolve_model(task).await.unwrap();
            assert_eq!(resolved.provider_id, *provider_id);
            assert_eq!(resolved.model, *model);
            assert_eq!(resolved.task, task);
        }
        nomifun_db::SqliteClientPreferenceRepository::new(database.pool().clone())
            .delete_keys(&[
                "models.default.imageGeneration",
                "models.default.imageEdit",
                "models.default.videoGeneration",
                "models.default.musicGeneration",
                "models.default.speechSynthesis",
            ])
            .await
            .unwrap();
        for (action, (provider_id, model)) in &models {
            let resolved = host.resolve_model(creation_model_task(action).unwrap()).await.unwrap();
            assert_eq!(resolved.provider_id, *provider_id, "{action} should route without a default");
            assert_eq!(resolved.model, *model);
        }
    }

    #[tokio::test]
    async fn routed_model_is_frozen_on_the_task_and_replay_survives_route_retirement() {
        let owner = UserId::new(); let database = init_database_memory_with_owner(owner.clone()).await.unwrap();
        let host = host(database.pool().clone(), owner.as_str());
        let models = configured_media_models(database.pool()).await;
        let (provider_id, model) = &models["creation.media/image"];
        let canvas_id = generate_id(); let node_id = generate_id();
        let document = json!({"schema":"nomifun.creative-studio/v1", "projectId":canvas_id});
        sqlx::query("INSERT INTO creative_studio_projects(project_id,title,document_json,created_at,updated_at) VALUES (?,'Model selection',?,0,0)")
            .bind(&canvas_id).bind(document.to_string()).execute(database.pool()).await.unwrap();
        let input = json!({"target":{"kind":"canvas_node","canvas_id":canvas_id,"node_id":node_id},
            "prompt":"a cat", "count":1});
        let request = Wave3HostRequest { context: context(owner.as_str()), operation: nomifun_agent_domain_wave3::operation_from_input(
            &nomifun_agent_domain_wave3::CREATION_MEDIA_MODULE_ID.into(),
            &"creation.media/image".into(),
            StrictJsonValue(input.clone()),
        ).unwrap() };
        let receipt = host.invoke(request.clone()).await.unwrap();
        let task_id = receipt.0["creation_task_id"].as_str().unwrap();
        let task = host.creation.get_task(task_id).await.unwrap();
        assert_eq!(task.provider_id, *provider_id); assert_eq!(task.model, *model);
        assert!(task.params["_nomifun_provider_config_revision"].is_number());
        assert!(task.params.get("model_selection").is_none(), "selection is not a provider request parameter");
        sqlx::query("UPDATE providers SET enabled=0 WHERE provider_id=?").bind(provider_id).execute(database.pool()).await.unwrap();
        assert_eq!(host.invoke(request.clone()).await.unwrap().0["creation_task_id"], task_id, "accepted replays retain their frozen model after retirement");
        assert!(task.params.get("provider_id").is_none());
        assert!(task.params.get("model").is_none());
    }

    #[test]
    fn media_mapping_preserves_quality_resolution_count_and_music_semantics() {
        let target = CreationTaskTarget::ConversationTurn { conversation_id: generate_id(), message_id: generate_id() };
        let (_, image) = map_creation_operation(Wave3CapabilityOperation::CreationImageEdit(CreationImageEditRequest {
            target: target.clone(), prompt: "edit".into(), inputs: vec![], count: 2, size: None, quality: Some("high".into()),
        })).unwrap();
        assert_eq!(image.params["quality"], "high");
        let (_, video) = map_creation_operation(Wave3CapabilityOperation::CreationVideo(CreationVideoRequest {
            target: target.clone(), prompt: "video".into(), count: 1, size: Some("16:9".into()), resolution: Some("1080p".into()), seconds: Some(8), first_frame_asset_id: None, last_frame_asset_id: None,
        })).unwrap();
        assert_eq!(video.params["resolution"], "1080p"); assert_eq!(video.params["count"], 1);
        let (_, music) = map_creation_operation(Wave3CapabilityOperation::CreationMusic(CreationMusicRequest {
            target, prompt: "music".into(), lyrics: None, instrumental: true, format: Some("mp3".into()),
        })).unwrap();
        assert_eq!(music.capability, "music"); assert_eq!(music.params["instrumental"], true);
        assert_ne!(creation_model_task("creation.media/music").unwrap(), creation_model_task("creation.media/audio").unwrap());
    }

    #[tokio::test]
    async fn missing_generation_model_and_foreign_owner_fail_before_task_persistence() {
        let owner = UserId::new(); let database = init_database_memory_with_owner(owner.clone()).await.unwrap();
        let host = host(database.pool().clone(), owner.as_str());
        let request = Wave3HostRequest { context: context(owner.as_str()), operation: Wave3CapabilityOperation::CreationImage(CreationImageRequest {
            target: CreationTaskTarget::CanvasNode { canvas_id: generate_id(), node_id: generate_id() }, prompt: "image".into(), count: 1, size: None, quality: None,
        }) };
        let error = host.invoke(request.clone()).await.unwrap_err(); assert_eq!(error.code, "GENERATION_MODEL_UNAVAILABLE");
        let mut foreign = request; foreign.context.principal.principal_id = UserId::new().to_string();
        assert!(host.invoke(foreign).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks").fetch_one(database.pool()).await.unwrap(); assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn disabled_explicit_default_does_not_fall_back() {
        let owner = UserId::new(); let database = init_database_memory_with_owner(owner.clone()).await.unwrap();
        let host = host(database.pool().clone(), owner.as_str());
        let models = configured_media_models(database.pool()).await;
        let (provider_id, _) = &models["creation.media/image"];
        sqlx::query("UPDATE providers SET enabled = 0 WHERE provider_id = ?")
            .bind(provider_id)
            .execute(database.pool())
            .await
            .unwrap();
        let error = match host.resolve_model(nomifun_api_types::ModelTask::ImageGeneration).await {
            Ok(_) => panic!("disabled default must not resolve"),
            Err(error) => error,
        };
        assert_eq!(error.code, "GENERATION_MODEL_UNAVAILABLE");
    }

    #[tokio::test]
    async fn no_default_routes_across_multiple_compatible_models_in_priority_order() {
        use nomifun_db::{CreateProviderParams, IClientPreferenceRepository, IProviderModelRepository, IProviderRepository, NewProviderModel, NewProviderModelCapability};

        let owner = UserId::new(); let database = init_database_memory_with_owner(owner.clone()).await.unwrap();
        let host = host(database.pool().clone(), owner.as_str());
        let configured = configured_media_models(database.pool()).await;
        let preferences = nomifun_db::SqliteClientPreferenceRepository::new(database.pool().clone());
        preferences.delete_keys(&["models.default.imageGeneration"]).await.unwrap();
        let preferred_provider_id = generate_id();
        let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":["host-test-key"]}"#, &[0; 32]).unwrap();
        let (preferred_provider, _) = nomifun_db::SqliteProviderRepository::new(database.pool().clone())
            .create(
                CreateProviderParams { provider_id: Some(&preferred_provider_id), platform: "openai",
                    name: "preferred automatic image route", base_url: "https://example.com/v1",
                    auth_scheme: "bearer", credentials_encrypted: &encrypted, enabled: true,
                    bedrock_config: None, sort_order: Some(-10),
                },
                &NewProviderModel { model: "preferred-image", enabled: true, sort_order: 0,
                    description: None, capabilities: &[NewProviderModelCapability { task: "image_generation",
                        protocol: "openai.images", traits: "[]", connection_role: "default",
                        provider_params: "{}", ..Default::default() }],
                },
                &[],
            ).await.unwrap();
        nomifun_db::SqliteProviderModelRepository::new(database.pool().clone())
            .save(
                &preferred_provider_id,
                preferred_provider.config_revision,
                &NewProviderModel { model: "aaa-image", enabled: true, sort_order: 5,
                    description: None, capabilities: &[NewProviderModelCapability { task: "image_generation",
                        protocol: "openai.images", traits: "[]", connection_role: "default",
                        provider_params: "{}", ..Default::default() }],
                },
            ).await.unwrap();
        let selected = host.resolve_model(nomifun_api_types::ModelTask::ImageGeneration).await.unwrap();
        assert_eq!(selected.provider_id, preferred_provider_id);
        assert_eq!(selected.model, "preferred-image");

        let (original_provider_id, original_model) = &configured["creation.media/image"];
        sqlx::query("UPDATE provider_model_capabilities SET health = '{\"status\":\"healthy\"}' WHERE provider_id = ? AND model = ? AND task = 'image_generation'")
            .bind(original_provider_id).bind(original_model).execute(database.pool()).await.unwrap();
        let selected = host.resolve_model(nomifun_api_types::ModelTask::ImageGeneration).await.unwrap();
        assert_eq!(selected.provider_id, *original_provider_id, "healthy capability takes precedence over untested routes");
        sqlx::query("UPDATE provider_model_capabilities SET health = NULL WHERE provider_id = ? AND model = ? AND task = 'image_generation'")
            .bind(original_provider_id).bind(original_model).execute(database.pool()).await.unwrap();
        let explicit = json!({"provider_id": original_provider_id, "model": original_model}).to_string();
        preferences.upsert_batch(&[("models.default.imageGeneration", &explicit)]).await.unwrap();
        let selected = host.resolve_model(nomifun_api_types::ModelTask::ImageGeneration).await.unwrap();
        assert_eq!(selected.provider_id, *original_provider_id, "explicit default takes precedence");
    }

    #[tokio::test]
    async fn conversation_generation_cannot_target_another_session_or_unadmitted_message() {
        let owner = UserId::new(); let database = init_database_memory_with_owner(owner.clone()).await.unwrap();
        let host = host(database.pool().clone(), owner.as_str()); let context = context(owner.as_str());
        assert!(host.validate_task_owner(&context, &CreativeTaskOwner::ConversationTurn { conversation_id: generate_id(), message_id: generate_id() }).await.is_err());
        assert!(host.validate_task_owner(&context, &CreativeTaskOwner::ConversationTurn { conversation_id: context.agent_session_id.as_ref().to_owned(), message_id: generate_id() }).await.is_err());
    }
}
