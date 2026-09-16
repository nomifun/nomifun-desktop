//! Conversation generation is a persisted media task, independent of a chat runtime.
use super::*;
use nomifun_creation::{CreationInput, CreativeCreationTask, CreativeTaskOwner, NewCreationTask};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitConversationCreation {
    pub preset_id: String,
    pub provider_id: String,
    pub model: String,
    pub capability: String,
    pub params: Value,
    #[serde(default)]
    pub inputs: Vec<CreationInput>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Serialize)]
pub struct ConversationCreationResponse {
    pub message_id: String,
    pub tasks: Vec<CreativeCreationTask>,
}

#[derive(Serialize)]
pub struct ConversationCreationPage {
    pub items: Vec<CreativeCreationTask>,
}

fn required_capability(operation: &str) -> Result<&'static str, AppError> {
    match operation {
        "t2i" => Ok("creation.image"),
        "i2i" | "inpaint" => Ok("creation.image_edit"),
        "t2v" | "i2v" => Ok("creation.video"),
        "music" => Ok("creation.music"),
        "tts" => Ok("creation.audio"),
        _ => Err(AppError::BadRequest(
            "Unsupported conversation generation task".into(),
        )),
    }
}

fn validate_next_turn_runtime_binding(current: &Value, incoming: &Value) -> Result<(), AppError> {
    let key = nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY;
    let parse = |extra: &Value| -> Result<Option<nomifun_api_types::RuntimeEngineBinding>, AppError> {
        extra.get(key).map(|raw| {
            let binding: nomifun_api_types::RuntimeEngineBinding = serde_json::from_value(raw.clone())
                .map_err(|error| AppError::BadRequest(format!("Invalid runtime binding: {error}")))?;
            binding.validate()?;
            Ok(binding)
        }).transpose()
    };
    let current = parse(current)?;
    if let Some(target) = parse(incoming)? {
        if current.as_ref() != Some(&target) {
            return Err(AppError::Conflict("This Agent uses a different runtime engine; start a new conversation with it from the Agent workbench".into()));
        }
    }
    Ok(())
}

impl ConversationService {
    /// Caller holds the existing preparation gate, after replay and active-turn checks.
    pub(super) async fn apply_next_turn_preset(
        &self,
        user_id: &str,
        row: &ConversationRow,
        preset_id: &str,
        lease: &RuntimeBuildLease,
    ) -> Result<(), AppError> {
        if !self.execution_authority(user_id).controls_host() {
            return Err(AppError::Forbidden(
                "Agent selection requires the installation owner".into(),
            ));
        }
        if self.runtime_state.has_active_turn(&row.conversation_id) {
            return Err(AppError::Conflict(
                "The current conversation turn is still running".into(),
            ));
        }
        let resolver = self
            .product_agent_snapshot_resolver
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| AppError::Internal("Agent preset resolver is unavailable".into()))?;
        let model = row
            .model
            .as_deref()
            .map(parse_provider_with_model)
            .transpose()?;
        let current_snapshot: Option<AgentResolvedSnapshot> = row.agent_snapshot.as_deref()
            .map(serde_json::from_str).transpose().map_err(|error| AppError::Internal(format!("Invalid saved Agent snapshot: {error}")))?;
        let resolution = resolver
            .resolve_preset(user_id, preset_id, model.as_ref(), current_snapshot.as_ref().and_then(|snapshot| snapshot.canonical_binding.as_ref()))
            .await?;
        let snapshot = serde_json::to_string(&resolution.snapshot)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut extra: Value =
            serde_json::from_str(&row.extra).map_err(|e| AppError::Internal(e.to_string()))?;
        // Runtime identity belongs to the Conversation, not a refreshed Agent
        // revision. Reject a real engine switch before terminating its runtime.
        validate_next_turn_runtime_binding(&extra, &resolution.runtime_extra)?;
        let canonical_session_matches = resolution.runtime_extra.get("nomi_core_session")
            .is_none_or(|binding| extra.get("nomi_core_session") == Some(binding));
        if row.agent_snapshot.as_deref() == Some(snapshot.as_str()) && canonical_session_matches {
            return Ok(());
        }
        lease.ensure_active()?;
        Self::terminate_runtime_with_proof(
            &self.runtime_registry,
            &row.conversation_id,
            AgentKillReason::ConfigurationChanged,
            "next turn Agent selection",
        )
        .await?;
        lease.ensure_active()?;
        let object = extra
            .as_object_mut()
            .ok_or_else(|| AppError::Internal("Invalid conversation configuration".into()))?;
        for key in [
            "system_prompt",
            "allowed_tools",
            "enforce_tool_allowlist",
            "deferred_tools",
            "vision_input",
            "vision_on_demand",
            "chat_config_revision_digest",
            "citation_render",
            "runtime_profile",
            "mcp_capabilities",
            "browser_use",
            "computer_use",
            "nomi_core_session",
            "agent_name",
            "companion_memory_enabled",
            "companion_skills_enabled",
            "product_agent_target_kind",
            "product_agent_target_id",
            "product_agent_capabilities",
            "preset_rules",
            "preset_context",
            "preset_instructions_embedded",
        ] {
            object.remove(key);
        }
        if let Some(runtime) = resolution.runtime_extra.as_object() {
            // Keep the exact saved JSON value, even if an equal typed binding
            // arrives with another key order. SQLite protects that value.
            object.extend(runtime.iter().filter(|(key, _)| key.as_str() != nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY)
                .map(|(key, value)| (key.clone(), value.clone())));
        }
        let auto_inject = if object.get("product_agent_target_kind").is_none()
            && resolution.snapshot.canonical_binding.is_none()
        {
            self.skill_resolver.auto_inject_names().await
        } else {
            Vec::new()
        };
        object.insert(
            "skills".into(),
            json!(compute_initial_skills(
                &auto_inject,
                &resolution.snapshot.included_skills,
                &resolution.snapshot.excluded_auto_skills
            )),
        );
        self.conversation_repo
            .update(
                &row.conversation_id,
                &ConversationRowUpdate {
                    preset_id: Some(Some(resolution.snapshot.preset_id)),
                    preset_revision: Some(Some(resolution.snapshot.preset_revision)),
                    agent_snapshot: Some(Some(snapshot)),
                    extra: Some(extra.to_string()),
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await?;
        self.runtime_state
            .clear_knowledge_signature(&row.conversation_id);
        Ok(())
    }

    pub fn with_creation_service(&self, service: Arc<nomifun_creation::CreationService>) {
        *self
            .creation_service
            .write()
            .expect("creation service registration") = Some(service);
    }

    fn creation_engine(&self) -> Result<Arc<nomifun_creation::CreationService>, AppError> {
        self.creation_service
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| AppError::Internal("Generation service is unavailable".into()))
    }

    pub(super) async fn cancel_conversation_creations(&self, id: &str) -> Result<(), AppError> {
        let engine = self
            .creation_service
            .read()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(engine) = engine {
            engine.cancel_conversation_tasks(id).await?;
        }
        Ok(())
    }

    async fn creation_conversation(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<ConversationRow, AppError> {
        parse_conv_id(id)?;
        let row = self
            .conversation_repo
            .get(id)
            .await?
            .filter(|row| row.user_id == user_id)
            .ok_or_else(|| AppError::NotFound(format!("Conversation {id} not found")))?;
        if !self.execution_authority(user_id).controls_host() {
            return Err(AppError::Forbidden(
                "Media generation requires the installation owner".into(),
            ));
        }
        Ok(row)
    }

    pub async fn submit_conversation_creation(
        &self,
        user_id: &str,
        id: &str,
        key: &str,
        mut request: SubmitConversationCreation,
    ) -> Result<ConversationCreationResponse, AppError> {
        let lease = self.begin_public_runtime_preparation(id, user_id)?;
        let cancellation = lease.cancellation_token();
        let _guard = self
            .runtime_state
            .acquire_preparation_gate(id, &cancellation)
            .await?;
        lease.ensure_active()?;
        let row = self.creation_conversation(user_id, id).await?;
        self.ensure_public_mutation_allowed(user_id, id).await?;
        nomifun_common::CreationTaskId::parse(key)
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        let required = required_capability(&request.capability)?;
        let engine = self.creation_engine()?;
        let original_request =
            serde_json::to_value(&request).map_err(|e| AppError::BadRequest(e.to_string()))?;
        // Replay the accepted immutable request before consulting mutable Agent
        // presets, provider catalogs or files that may since have been removed.
        match engine.get_task(key).await {
            Ok(task) => {
                if task.conversation_id.as_deref() != Some(id)
                    || task.message_id.as_deref() != Some(key)
                    || task.params.get("_nomifun_creation_request") != Some(&original_request)
                {
                    return Err(AppError::Conflict(
                        "This submission key already belongs to a different request".into(),
                    ));
                }
                return self.finish_creation_batch(user_id, id, key, task).await;
            }
            Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
        let resolver = self
            .product_agent_snapshot_resolver
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| AppError::Internal("Agent preset resolver is unavailable".into()))?;
        let model = row
            .model
            .as_deref()
            .map(parse_provider_with_model)
            .transpose()?;
        let resolution = resolver
            .resolve_preset(user_id, &request.preset_id, model.as_ref(), None)
            .await?;
        if !resolution
            .snapshot
            .enabled_capabilities
            .iter()
            .any(|cap| cap == required)
        {
            return Err(AppError::Forbidden(format!(
                "This Agent does not enable {required}"
            )));
        }
        let params = request.params.as_object_mut().ok_or_else(|| {
            AppError::BadRequest("Generation parameters must be an object".into())
        })?;
        if params.keys().any(|key| key.starts_with("_nomifun")) {
            return Err(AppError::BadRequest(
                "Generation metadata is server-owned".into(),
            ));
        }
        if params
            .get("prompt")
            .and_then(Value::as_str)
            .is_none_or(|prompt| prompt.trim().is_empty())
        {
            return Err(AppError::BadRequest("Describe the work to generate".into()));
        }
        let max_count = if matches!(request.capability.as_str(), "t2v" | "i2v") {
            8
        } else {
            10
        };
        let count = params
            .get("count")
            .map(|value| {
                value
                    .as_u64()
                    .filter(|n| (1..=max_count).contains(n))
                    .ok_or_else(|| {
                        AppError::BadRequest(format!(
                            "Generation count must be between 1 and {max_count}"
                        ))
                    })
            })
            .transpose()?
            .unwrap_or(1);
        let batch_size = if matches!(request.capability.as_str(), "t2v" | "i2v") {
            count
        } else {
            1
        };
        let batch_ids: Vec<String> = std::iter::once(key.to_owned())
            .chain((1..batch_size).map(|_| nomifun_common::CreationTaskId::new().into_string()))
            .collect();
        if batch_size > 1 {
            params.insert("count".into(), json!(1));
        }
        params.insert("_nomifun_creation_request".into(), original_request);
        params.insert("_nomifun_creation_batch".into(), json!(batch_ids));
        params.insert(
            "_nomifun_creation_agent".into(),
            serde_json::to_value(&resolution.snapshot)
                .map_err(|e| AppError::Internal(e.to_string()))?,
        );
        // These are the same explicit local-owner attachments accepted by the
        // desktop composer. Read once into the existing asset store; retries use
        // the persisted input IDs, never the original path again.
        let references = Self::import_creation_files(
            &engine,
            id,
            &request.files,
            &request.capability,
            &request.inputs,
            false,
        )
        .await?;
        request.inputs.extend(references);
        lease.ensure_active()?;
        let mut generation = NewCreationTask {
            provider_id: request.provider_id,
            model: request.model,
            capability: request.capability,
            params: request.params,
            inputs: request.inputs,
        };
        engine.capture_model_config(&mut generation).await?;
        let task = engine
            .create_creative_task(
                CreativeTaskOwner::ConversationTurn {
                    conversation_id: id.to_owned(),
                    message_id: key.to_owned(),
                },
                key.to_owned(),
                generation,
            )
            .await?;
        self.finish_creation_batch(user_id, id, key, task).await
    }

    async fn finish_creation_batch(
        &self,
        user_id: &str,
        id: &str,
        key: &str,
        root: nomifun_creation::CreationTask,
    ) -> Result<ConversationCreationResponse, AppError> {
        let engine = self.creation_engine()?;
        let ids: Vec<String> = serde_json::from_value(
            root.params
                .get("_nomifun_creation_batch")
                .cloned()
                .unwrap_or_else(|| json!([key])),
        )
        .map_err(|e| AppError::Internal(format!("Invalid persisted generation batch: {e}")))?;
        let mut tasks = vec![CreativeCreationTask::try_from(root.clone())?];
        for task_id in ids.into_iter().skip(1) {
            let task = engine
                .create_creative_task(
                    CreativeTaskOwner::ConversationTurn {
                        conversation_id: id.to_owned(),
                        message_id: key.to_owned(),
                    },
                    task_id,
                    NewCreationTask {
                        provider_id: root.provider_id.clone(),
                        model: root.model.clone(),
                        capability: root.capability.clone(),
                        params: root.params.clone(),
                        inputs: root.inputs.clone().unwrap_or_default(),
                    },
                )
                .await?;
            tasks.push(CreativeCreationTask::try_from(task)?);
        }
        self.broadcast_list_changed(user_id, id, "updated", None);
        self.user_events.send_to_user(
            user_id,
            WebSocketMessage::new(
                "conversation.creationChanged",
                json!({"conversation_id":id,"message_id":key,"tasks":&tasks}),
            ),
        );
        Ok(ConversationCreationResponse {
            message_id: key.to_owned(),
            tasks,
        })
    }

    async fn import_creation_files(
        engine: &Arc<nomifun_creation::CreationService>,
        id: &str,
        files: &[String],
        capability: &str,
        existing_inputs: &[CreationInput],
        in_library: bool,
    ) -> Result<Vec<CreationInput>, AppError> {
        use tokio::io::AsyncReadExt;
        if files.len() > 8 {
            return Err(AppError::BadRequest(
                "Attach up to 8 generation references".into(),
            ));
        }
        let mut prepared = Vec::new();
        let mut total = 0;
        for path in files {
            let path = nomifun_file::path_safety::validate_path_authority(
                path,
                &nomifun_file::PathAuthority::Unrestricted,
            )?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Reference")
                .to_owned();
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let mime = match extension.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                "mp4" | "m4v" => "video/mp4",
                "webm" => "video/webm",
                "mov" => "video/quicktime",
                "mp3" => "audio/mpeg",
                "wav" => "audio/wav",
                "ogg" => "audio/ogg",
                "flac" => "audio/flac",
                "m4a" => "audio/mp4",
                _ => {
                    return Err(AppError::BadRequest(format!(
                        "{name}: choose an image, video or audio file"
                    )));
                }
            };
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|e| AppError::BadRequest(format!("Cannot read {name}: {e}")))?;
            let mut bytes = Vec::new();
            file.take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            total += bytes.len();
            if bytes.len() > 64 * 1024 * 1024 || total > 256 * 1024 * 1024 {
                return Err(AppError::BadRequest(
                    "References exceed the supported upload size".into(),
                ));
            }
            let mime = nomifun_creation::validate_artifact_payload(&bytes, mime)
                .map_err(|e| AppError::BadRequest(format!("{name}: {}", e.message)))?;
            prepared.push((bytes, mime, name));
        }
        let mut inputs = Vec::new();
        let mut assign_first_frame = capability == "i2v"
            && prepared
                .iter()
                .filter(|(_, mime, _)| mime.starts_with("image/"))
                .count()
                == 1
            && existing_inputs
                .iter()
                .all(|input| input.role == "last_frame");
        for (bytes, mime, name) in prepared {
            let mut input = engine.import_reference(bytes,mime,json!({"source":"conversation_attachment","attachment_context":{"conversation_id":id},"title":name}),in_library).await?;
            input.role = match input.kind {
                nomifun_creation::CreationInputKind::Image if assign_first_frame => {
                    assign_first_frame = false;
                    "first_frame"
                }
                nomifun_creation::CreationInputKind::Video => "video",
                nomifun_creation::CreationInputKind::Audio => "audio",
                _ => "reference",
            }
            .into();
            inputs.push(input);
        }
        Ok(inputs)
    }

    /// The existing turn gate owns the message while these references are
    /// captured. Persist each completed import before starting the next one so
    /// an interrupted/retried turn does not re-read or re-import completed files.
    pub(super) async fn persist_ordinary_creation_references(
        &self,
        user_id: &str,
        row: &ConversationRow,
        message_id: &str,
        files: &[String],
    ) -> Result<(), AppError> {
        if files.is_empty()
            || !self.execution_authority(user_id).controls_host()
            || row.r#type != AgentType::Nomi.serde_name()
            || row.agent_snapshot.is_none()
            || !(row_agent_snapshot_has_capability(row, "creation.image_edit")?
                || row_agent_snapshot_has_capability(row, "creation.video")?)
        {
            return Ok(());
        }
        let images: Vec<_> = files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                std::path::Path::new(file)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "gif"
                        )
                    })
            })
            .collect();
        if images.is_empty() {
            return Ok(());
        }
        let engine = self.creation_engine()?;
        persist_image_references(
            self.conversation_repo.as_ref(),
            &engine,
            &row.conversation_id,
            message_id,
            images,
        )
        .await
    }

    pub async fn list_conversation_creations(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<ConversationCreationPage, AppError> {
        self.creation_conversation(user_id, id).await?;
        let items = self
            .creation_engine()?
            .list_conversation_tasks(id)
            .await?
            .into_iter()
            .map(CreativeCreationTask::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ConversationCreationPage { items })
    }

    pub async fn cancel_conversation_creation(
        &self,
        user_id: &str,
        id: &str,
        task_id: &str,
    ) -> Result<CreativeCreationTask, AppError> {
        self.creation_conversation(user_id, id).await?;
        self.ensure_public_mutation_allowed(user_id, id).await?;
        let engine = self.creation_engine()?;
        let task = engine.get_task(task_id).await?;
        if task.conversation_id.as_deref() != Some(id) {
            return Err(AppError::NotFound("Generation task not found".into()));
        }
        CreativeCreationTask::try_from(engine.cancel_task(task_id).await?)
    }
}

async fn persist_image_references(
    repository: &dyn nomifun_db::IConversationRepository,
    engine: &Arc<nomifun_creation::CreationService>,
    conversation_id: &str,
    message_id: &str,
    images: Vec<(usize, &String)>,
) -> Result<(), AppError> {
    use nomifun_creation::{ConversationCreationReference, CreationInputKind};
    let message = repository
        .get_message(conversation_id, message_id)
        .await?
        .filter(|message| message.position.as_deref() == Some("right") && message.r#type == "text")
        .ok_or_else(|| AppError::NotFound("Reference source user message not found".into()))?;
    let mut content: Value = serde_json::from_str(&message.content)
        .map_err(|error| AppError::Internal(format!("Invalid user message: {error}")))?;
    let mut references: Vec<ConversationCreationReference> = content
        .get("creation_references")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| AppError::Internal(format!("Invalid creation references: {error}")))?
        .unwrap_or_default();
    for (file_index, file) in images {
        let file_name = std::path::Path::new(file)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Reference")
            .to_owned();
        if let Some(reference) = references
            .iter()
            .find(|reference| reference.file_index == file_index)
        {
            if reference.file_name != file_name || reference.kind != CreationInputKind::Image {
                return Err(AppError::Conflict(
                    "Stored creation reference does not match this attachment".into(),
                ));
            }
            continue;
        }
        let input = ConversationService::import_creation_files(
            engine,
            conversation_id,
            std::slice::from_ref(file),
            "i2i",
            &[],
            false,
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal("Image reference was not imported".into()))?;
        references.push(ConversationCreationReference {
            asset_id: input.asset_id,
            file_name,
            file_index,
            kind: input.kind,
        });
        references.sort_by_key(|reference| reference.file_index);
        content["creation_references"] = serde_json::to_value(&references)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        repository
            .update_message(
                message_id,
                &nomifun_db::MessageRowUpdate {
                    content: Some(content.to_string()),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_creation::{
        AssetSink, CreationError, PersistAsset, TaskArtifactIssue, TaskArtifactManifest,
        TaskArtifactReconcileReport,
    };
    use nomifun_db::{
        IConversationRepository, SqliteConversationRepository, SqliteCreationTaskRepository, sqlx,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct ReferenceSink(Mutex<Vec<(String, PersistAsset)>>);

    #[async_trait::async_trait]
    impl AssetSink for ReferenceSink {
        async fn persist(&self, asset: PersistAsset) -> Result<String, CreationError> {
            let id = nomifun_common::WorkshopAssetId::new().into_string();
            self.0.lock().unwrap().push((id.clone(), asset));
            Ok(id)
        }
        async fn rollback(&self, _: &[String]) -> Result<(), CreationError> {
            unreachable!()
        }
        async fn verify_task_artifacts(
            &self,
            _: &[TaskArtifactManifest],
        ) -> Result<Vec<TaskArtifactIssue>, CreationError> {
            unreachable!()
        }
        async fn reconcile_task_artifacts(
            &self,
            _: &[TaskArtifactManifest],
        ) -> Result<TaskArtifactReconcileReport, CreationError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn ordinary_creation_references_preserve_order_input_and_partial_retry_without_entering_library()
     {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let conversation = nomifun_common::ConversationId::new().into_string();
        let message = nomifun_common::MessageId::new().into_string();
        let owner = nomifun_common::UserId::new().into_string();
        sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,extra,created_at,updated_at) VALUES (?,?,'Reference test','nomi','{}',0,0)")
            .bind(&conversation).bind(&owner).execute(db.pool()).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.gif");
        let third = directory.path().join("third.gif");
        let files = vec![
            first.to_string_lossy().into_owned(),
            directory
                .path()
                .join("notes.txt")
                .to_string_lossy()
                .into_owned(),
            third.to_string_lossy().into_owned(),
        ];
        let original = json!({"content":"把第一张图的颜色应用到第二张图片", "files":files,
            "interaction":{"surface":"desktop"}, "agent_snapshot":{"preset_name":"通用助理"}});
        sqlx::query("INSERT INTO messages (message_id,conversation_id,msg_id,type,content,position,status,created_at) VALUES (?,?,?,'text',?,'right','finish',0)")
            .bind(&message).bind(&conversation).bind(&message).bind(original.to_string()).execute(db.pool()).await.unwrap();
        // A complete 1x1 GIF; the production importer fully decodes these bytes.
        let gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00\x3b";
        tokio::fs::write(&first, gif).await.unwrap();
        let repository = SqliteConversationRepository::new(db.pool().clone());
        let task_repository = Arc::new(SqliteCreationTaskRepository::new(db.pool().clone()));
        let sink = Arc::new(ReferenceSink::default());
        let engine = nomifun_creation::CreationService::builder(task_repository)
            .with_asset_sink(sink.clone())
            .build();
        let inputs = || vec![(0, &files[0]), (2, &files[2])];
        assert!(
            persist_image_references(&repository, &engine, &conversation, &message, inputs())
                .await
                .is_err()
        );
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        let partial = engine
            .conversation_message_creation_references(&conversation, &message)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(partial.as_array().unwrap().len(), 1);
        // The first file is no longer available. The retry must use its durable ID.
        tokio::fs::remove_file(&first).await.unwrap();
        tokio::fs::write(&third, gif).await.unwrap();
        persist_image_references(&repository, &engine, &conversation, &message, inputs())
            .await
            .unwrap();
        tokio::fs::remove_file(&third).await.unwrap();
        persist_image_references(&repository, &engine, &conversation, &message, inputs())
            .await
            .unwrap();
        let expected_references = {
            let captures = sink.0.lock().unwrap();
            assert_eq!(
                captures.len(),
                2,
                "completed references must never be reimported"
            );
            assert!(
                captures
                    .iter()
                    .all(|(_, asset)| !asset.in_library && asset.mime == "image/gif")
            );
            json!([
                {"asset_id":captures[0].0,"file_name":"first.gif","file_index":0,"kind":"image"},
                {"asset_id":captures[1].0,"file_name":"third.gif","file_index":2,"kind":"image"}
            ])
        };
        let references = engine
            .conversation_message_creation_references(&conversation, &message)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(references, expected_references);
        let mut stored: Value = serde_json::from_str(
            &repository
                .get_message(&conversation, &message)
                .await
                .unwrap()
                .unwrap()
                .content,
        )
        .unwrap();
        stored
            .as_object_mut()
            .unwrap()
            .remove("creation_references");
        assert_eq!(
            stored, original,
            "text, files, interaction and Agent snapshot are untouched"
        );
        assert!(
            engine
                .conversation_message_creation_references(
                    &nomifun_common::ConversationId::new().into_string(),
                    &message
                )
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            engine
                .conversation_message_creation_references(
                    &conversation,
                    &nomifun_common::MessageId::new().into_string()
                )
                .await
                .unwrap()
                .is_none()
        );
    }
}
