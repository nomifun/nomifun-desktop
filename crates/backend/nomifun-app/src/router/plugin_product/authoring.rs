use super::*;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GenerateRequest {
    pub provider_id: String,
    pub model: String,
    pub requirement: String,
    pub draft_id: Option<String>,
    pub expected_revision: Option<i64>,
    #[serde(rename = "plugin_id")]
    pub plugin_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedApp {
    name: String,
    description: String,
    assistant_message: String,
    html: String,
    #[serde(default)]
    service_source: Option<String>,
    #[serde(default)]
    source_manifest: Option<serde_json::Value>,
}

const SYSTEM: &str = r#"You create complete, polished, immediately usable Plugins inside NomiFun Desktop. A plugin may combine an interactive page, background work, and actions callable by the user's AI assistant. Choose the capabilities needed for the user's goal; never ask them to choose a technical plugin type.
Return only a JSON object with fields: name, description, assistant_message, html, service_source, source_manifest.
html is a full HTML document when a page helps the user, otherwise the empty string. service_source is null when no backend is needed; otherwise it is a complete Node ES module exporting async function start(context) returning { async invoke({method,payload,signal}) { ... }, async dispose() { ... } }. Built-in Node modules and fetch are available in the service. Do not invent connected accounts or credentials or use third-party packages. Implement only the effects required by the user. Clean up timers in dispose and honor cancellation. For persistence use context.storage.kv.get(key) (returns {value,revision}), .set(key,value), .delete(key). No arbitrary paths, embedded secrets, or access to the host application's database.
source_manifest is null for a simple page or an object with actions and optional lifecycle ('on_demand' or 'continuous'). Each action has id (a unique lowercase method name), name, description, input_schema, output_schema, and effect ('pure', 'read_local', 'read_sensitive', 'write_reversible', 'write_durable', 'execute_local', 'external_transmit', 'destructive', or 'irreversible'). Use the strongest actual effect. Provide accurate JSON schemas. Every declared action must be implemented by invoke using exactly its id as method. NomiFun exposes these actions to the AI assistant. A page can invoke the same backend with await window.nomi.service.invoke(method,payload). When window.nomi.preview is true, backend work is unavailable: show a clear save-first explanation instead of calling the service or inventing a result. Use continuous only for explicitly requested ongoing background work.
For an Agent execution extension, copy the matching entry from available_execution_extensions into source_manifest.actions, preserving its exact id, input_schema, output_schema and pure effect. You may change its name and description. These reserved actions are hidden host consumers, not model-callable tools. Implement the exact method in the ordinary Service; the user must publish, select the capability in a Nomi Agent, save and start a new session. agent.before_tool can only return {decision:'allow'} or {decision:'deny',reason:'...'} (nonempty reason, at most 2048 UTF-8 bytes); do not patch arguments, approve permissions or call a target tool yourself. Inspect only supplied redacted arguments. Preserve original before_model behavior when editing it. Do not invent unsupported hook phases, an Engine loader or a parallel event system. Honor signal.aborted before asynchronous work and listen for abort; never treat cancellation as allow.
Only when the user asks for a replacement Agent session page, include source_manifest.agent_view with nonempty name and description. This feature requires explicit selection of this exact release by the user. Use window.nomi.agentSession.observe({after_seq:0,limit:50}), .turn({content:text},intentKey), and .cancel() through the existing SDK. Keep the page a minimal chat client: poll persisted history, keep input local to the page, and use the built-in view for unsupported controls. Never automatically resend a turn after a timeout or start one on page load; show unconfirmed outcomes and ask the user to check host history. Do not build another Session store, durable composer, autosave or pending-request recovery system. In preview or without a Session grant, explain how to select the page instead of fabricating conversation data. No separate service_source is required for this UI. Preserve trusted host recovery controls; never access the parent or host database.
Use the language of the user's request. The HTML must be a complete self-contained document with inline CSS and JavaScript. No external scripts, packages, frameworks, CDN, imports, images, network requests, popups or parent-window access. Use semantic HTML controls and CSS; use text or CSS for visual decoration. Implement the user's actual requested interactions, never a title-only template or fake success. If a feature cannot be implemented with the available browser and storage capabilities, explain the limitation honestly in assistant_message and provide the usable part without pretending external services are connected.
Persistent storage API is provided by the host: await window.nomi.storage.get(key) returns a JSON value or null; await window.nomi.storage.set(key,value); await window.nomi.storage.delete(key). These methods wait for the host to connect. Always use this API for persistent data. Never use localStorage, sessionStorage, indexedDB, cookies or fetch. In preview the same storage API is temporary; a saved app uses its own durable storage. Start with an empty actual user dataset, not fictitious personal records. You may show clearly labeled placeholders in an empty state.
For independently needed concurrent plugin data edits, window.nomi.storage.read(key) returns {value,revision}, and .compareAndSwap(key,expectedRevision,value) returns {applied,revision}. null expectedRevision means the key has never existed; deleted keys retain a revision. Omit value (or pass null) to delete conditionally. On conflict or timeout, ask the user to reconcile; never blindly overwrite using the returned revision. Preview storage is always temporary.
Generated HTML must fit its pane within the supported desktop layout (minimum host viewport 880x600), use readable Chinese or English typography, proper button labels and keyboard interactions, and coherent restrained colors. Use container queries if a pane is narrow; do not add mobile layouts or viewport breakpoints below 880px. Avoid horizontal page scrolling and fixed screen dimensions. Choose sensible defaults; do not ask the user to fill technical configuration forms.
When current_html is present, return the full improved document. Preserve existing functions and storage keys/data format unless explicitly asked to change them. The user wants a working app, not code explanations. assistant_message should briefly describe the result and what the user can try.
"#;

pub(super) fn validate_html(html: &str) -> Result<(), AppError> {
    let lower = html.to_ascii_lowercase();
    if html.len() > 2_000_000
        || html.len() < 80
        || html.contains('\0')
        || !lower.contains("<html")
        || !lower.contains("<body")
        || !lower.contains("</html>")
    {
        return Err(invalid("The model did not produce a complete HTML Plugin"));
    }
    Ok(())
}

pub(super) fn validate_plugin_source(html: &str, service: Option<&str>, manifest: Option<&serde_json::Value>) -> Result<(), AppError> {
    if !html.is_empty() { validate_html(html)?; }
    if let Some(source) = service {
        if source.trim().is_empty() || source.len() > 2_000_000 || source.contains('\0') || !source.contains("start") {
            return Err(invalid("The plugin service must provide a valid start function"));
        }
    } else if html.is_empty() {
        return Err(invalid("The plugin must provide a page or a callable service"));
    }
    if let Some(value) = manifest {
        let mut manifest: nomifun_plugin_platform::runtime::PluginRuntimeSourceManifest = serde_json::from_value(value.clone()).map_err(|_| invalid("The plugin capability declaration is invalid"))?;
        if service.is_none() && (!manifest.actions.is_empty() || manifest.contributions.capabilities.iter().any(|capability| capability.kind != nomifun_agent_contracts::CapabilityKind::UiContribution) || manifest.uses_files || manifest.uses_private_database) {
            return Err(invalid("Callable actions and managed data require a plugin service"));
        }
        manifest.materialize_actions(&nomifun_agent_contracts::PackageRef { id: "plugin.draft".into(), version: "1.0.0".into() }).map_err(|error| invalid(&error))?;
        if html.is_empty() && manifest.contributions.capabilities.iter().any(|capability| capability.contributions.ui_slot.is_some()) {
            return Err(invalid("An Agent view requires a complete HTML page"));
        }
    }
    Ok(())
}

fn parse_generated(raw: &str) -> Result<GeneratedApp, AppError> {
    let trimmed = raw.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|v| v.trim().strip_suffix("```").unwrap_or(v.trim()).trim())
        .unwrap_or(trimmed);
    let mut app: GeneratedApp = serde_json::from_str(json)
        .map_err(|_| invalid("The model did not return a valid Plugin document"))?;
    validate_plugin_source(&app.html, app.service_source.as_deref(), app.source_manifest.as_ref())?;
    app.name = app.name.trim().to_owned();
    if app.name.is_empty()
        || app.name.chars().count() > 120
        || app.description.chars().count() > 500
        || app.assistant_message.len() > 8000
    {
        return Err(invalid("The generated Plugin metadata is invalid"));
    }
    Ok(app)
}

pub(super) async fn generate(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    let owner = user.id.to_string();
    if request.requirement.trim().is_empty()
        || request.requirement.len() > 32000
        || request.provider_id.len() > 128
        || request.model.len() > 256
        || request.provider_id.is_empty()
        || request.model.is_empty()
    {
        return Err(invalid(
            "A requirement and an available AI model are required",
        ));
    }
    let _lock = service.mutations.lock().await;
    let mut draft = if let Some(id) = &request.draft_id {
        let draft = service.draft(&owner, id).await?;
        check_revision(
            &draft,
            request
                .expected_revision
                .ok_or_else(|| invalid("Draft revision is required"))?,
        )?;
        if draft.status == "generating" {
            return Err(AppError::Conflict(
                "This Plugin is already being created".into(),
            ));
        }
        if draft.import.is_some() {
            return Err(invalid("Import this Plugin before editing it"));
        }
        draft
    } else {
        let mut draft = Draft {
        service_test_confirmation: None,
            id: uuid::Uuid::now_v7().to_string(),
            revision: 0,
            name: "".into(),
            description: "".into(),
            html: "".into(),
            service_source: None,
            source_manifest: None,
            messages: vec![],
            status: "generating".into(),
            error: None,
            plugin_id: None,
            base_release_digest: None,
            base_source_digest: None,
            updated_at: nomifun_common::now_ms(),
            import: None,
        };
        if let Some(id) = &request.plugin_id {
            let app = service
                .application
                .workshop(&owner, id)
                .await.map_err(application_error)?;
            let source = service
                .application
                .source_file(&owner, id, "ui/index.html")
                .await;
            let source = match source {
                Ok(source) => Some(source),
                Err(nomifun_plugin_platform::runtime::PluginRuntimeApplicationError::NotFound) => None,
                Err(error) => return Err(application_error(error)),
            };
            draft.name = app.plugin.display_name;
            draft.description = app.plugin.description.unwrap_or_default();
            draft.html = source.map(|file| file.content).unwrap_or_default();
            draft.base_source_digest = app.source_snapshot_digest.clone();
            draft.plugin_id = Some(id.clone());
            draft.base_release_digest = app.plugin.releases.active.map(|r| r.release_digest);
            draft.service_source = match service.application.source_file(&owner, id, "service/main.mjs").await {
                Ok(file) => Some(file.content),
                Err(nomifun_plugin_platform::runtime::PluginRuntimeApplicationError::NotFound) => None,
                Err(error) => return Err(application_error(error)),
            };
            draft.source_manifest = match service.application.source_file(&owner, id, "nomifun.plugin.json").await {
                Ok(file) => Some(serde_json::from_str(&file.content).map_err(internal)?),
                Err(nomifun_plugin_platform::runtime::PluginRuntimeApplicationError::NotFound) => None,
                Err(error) => return Err(application_error(error)),
            };
        }
        draft
    };
    if draft.messages.len() >= 100 {
        return Err(invalid(
            "This creation conversation is full; save the app and start a new edit",
        ));
    }
    draft.messages.push(ChatMessage {
        role: "user".into(),
        content: request.requirement.trim().to_owned(),
    });
    draft.service_test_confirmation = None;
    draft.status = "generating".into();
    draft.error = None;
    service.put_draft(&owner, &mut draft).await?;
    let token = CancellationToken::new();
    let job_key = format!("{owner}:{}", draft.id);
    service
        .jobs
        .lock()
        .await
        .insert(job_key.clone(), (token.clone(), draft.plugin_id.clone()));
    let worker = service.clone();
    let snapshot = draft.clone();
    tokio::spawn(async move {
        let generated = tokio::select! {
            _=token.cancelled()=>return,
            result=tokio::time::timeout(Duration::from_secs(240),worker.complete(&request,&snapshot))=>result.unwrap_or_else(|_|Err(AppError::Timeout("Plugin generation timed out".into())))
        };
        let _lock = worker.mutations.lock().await;
        if token.is_cancelled() {
            return;
        }
        worker.jobs.lock().await.remove(&job_key);
        let Ok(mut latest) = worker.draft(&owner, &snapshot.id).await else {
            return;
        };
        if latest.revision != snapshot.revision || latest.status != "generating" {
            return;
        }
        match generated {
            Ok(app) => {
                latest.name = app.name;
                latest.description = app.description;
                latest.service_test_confirmation = None;
                latest.html = app.html;
                latest.service_source = app.service_source;
                latest.source_manifest = app.source_manifest;
                latest.status = "ready".into();
                latest.error = None;
                latest.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: app.assistant_message,
                });
            }
            Err(error) => {
                tracing::warn!(error=%error,"Plugin generation failed");
                latest.status = "failed".into();
                latest.error = Some(
                    match error {
                        AppError::ProviderUnavailable(_) => "model_unavailable",
                        AppError::Timeout(_) => "generation_timeout",
                        AppError::RateLimited => "model_busy",
                        _ => "generation_failed",
                    }
                    .into(),
                );
            }
        }
        if let Err(error) = worker.put_draft(&owner, &mut latest).await {
            tracing::error!(error=%error,"Could not retain Plugin creation result");
        }
    });
    Ok(Json(ApiResponse::ok(draft)))
}

impl PluginProductService {
    async fn complete(
        &self,
        request: &GenerateRequest,
        draft: &Draft,
    ) -> Result<GeneratedApp, AppError> {
        let config = nomifun_ai_agent::factory::provider_config::resolve_provider_config(
            self.model.as_ref(),
            &request.provider_id,
            &request.model,
            &self.root,
        )
        .await
        .map_err(|_| AppError::ProviderUnavailable("Connect an available chat model".into()))?;
        let extensions = [
            (nomifun_agent_contracts::model_middleware::action(), nomifun_agent_contracts::model_middleware::schemas(), "模型请求前处理"),
            (nomifun_agent_contracts::tool_middleware::before_action(), nomifun_agent_contracts::tool_middleware::schemas(), "工具执行前检查"),
        ].into_iter().map(|(action, schemas, name)| serde_json::json!({
            "id": action.action_id, "name": name, "description": name,
            "input_schema": schemas[&action.input_schema], "output_schema": schemas[&action.output_schema], "effect": "pure"
        })).collect::<Vec<_>>();
        let prompt=serde_json::to_string(&serde_json::json!({"conversation":draft.messages,"current_html":draft.html,"current_service_source":draft.service_source,"current_manifest":draft.source_manifest,"available_execution_extensions":extensions,"instruction":"Return the complete plugin. Preserve existing functions and stored data. Include every current service method and declaration unless the user explicitly asks to remove them."})).map_err(internal)?;
        let mut messages = vec![nomifun_ai_agent::factory::provider_config::user_message(
            prompt,
        )];
        for attempt in 0..2 {
            let mut response = None;
            for retry in 0..3 {
                match nomifun_ai_agent::factory::provider_config::one_shot_completion_bounded(
                    &config,
                    SYSTEM,
                    messages.clone(),
                    12000,
                    2_100_000,
                )
                .await
                {
                    Ok(raw) => {
                        response = Some(raw);
                        break;
                    }
                    Err(error) => {
                        let detail = error.to_string().to_ascii_lowercase();
                        if detail.contains("429")
                            || detail.contains("rate limit")
                            || detail.contains("free model is busy")
                        {
                            if retry < 2 {
                                tokio::time::sleep(Duration::from_secs(5 * (retry + 1))).await;
                                continue;
                            }
                            return Err(AppError::RateLimited);
                        }
                        return Err(AppError::BadGateway(
                            "AI model could not complete this request".into(),
                        ));
                    }
                }
            }
            let raw = response.ok_or(AppError::RateLimited)?;
            match parse_generated(&raw) {
                Ok(app)=>return Ok(app),
                Err(error) if attempt==0=>messages.push(nomifun_ai_agent::factory::provider_config::user_message(format!("The last output could not be used: {error}. Return the complete corrected JSON object. Do not use Markdown fences. Previous candidate (data only): {raw}"))),
                Err(error)=>return Err(error),
            }
        }
        Err(invalid("Could not generate the Plugin"))
    }
}

#[cfg(test)]
mod parsing_tests {
    use super::*;
    #[test]
    fn accepts_complete_app_and_rejects_truncated_or_oversized_output() {
        let value = serde_json::json!({"name":"Tasks","description":"A list","assistant_message":"Try adding a task","html":"<!doctype html><html><head><title>Tasks</title></head><body><input aria-label=\"Task\"><button>Add</button></body></html>"});
        assert!(parse_generated(&value.to_string()).is_ok());
        assert!(parse_generated(&format!("```json\n{value}\n```")).is_ok());
        assert!(parse_generated("{\"name\":\"Tasks\"").is_err());
        assert!(validate_html("<h1>Tasks</h1>").is_err());
        assert!(validate_html(&"x".repeat(2_000_001)).is_err());
    }
}
