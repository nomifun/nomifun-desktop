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
    pub miniapp_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedApp {
    name: String,
    description: String,
    assistant_message: String,
    html: String,
}

const SYSTEM: &str = r#"You create complete, polished, immediately usable Mini Apps inside NomiFun Desktop.
Return only a JSON object with exactly these fields: name, description, assistant_message, html.
Use the language of the user's request. The HTML must be a complete self-contained document with inline CSS and JavaScript. No external scripts, packages, frameworks, CDN, imports, images, network requests, popups or parent-window access. Use semantic HTML controls and CSS; use text or CSS for visual decoration. Implement the user's actual requested interactions, never a title-only template or fake success. If a feature cannot be implemented with the available browser and storage capabilities, explain the limitation honestly in assistant_message and provide the usable part without pretending external services are connected.
Persistent storage API is provided by the host: await window.nomi.storage.get(key) returns a JSON value or null; await window.nomi.storage.set(key,value); await window.nomi.storage.delete(key). These methods wait for the host to connect. Always use this API for persistent data. Never use localStorage, sessionStorage, indexedDB, cookies or fetch. In preview the same storage API is temporary; a saved app uses its own durable storage. Start with an empty actual user dataset, not fictitious personal records. You may show clearly labeled placeholders in an empty state.
Generated HTML must fit its container at widths >= 320px, use readable Chinese or English typography, proper button labels and keyboard interactions, and coherent restrained colors. Avoid horizontal page scrolling and fixed screen dimensions. Choose sensible defaults; do not ask the user to fill technical configuration forms.
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
        return Err(invalid("The model did not produce a complete HTML MiniApp"));
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
        .map_err(|_| invalid("The model did not return a valid MiniApp document"))?;
    validate_html(&app.html)?;
    app.name = app.name.trim().to_owned();
    if app.name.is_empty()
        || app.name.chars().count() > 120
        || app.description.chars().count() > 500
        || app.assistant_message.len() > 8000
    {
        return Err(invalid("The generated MiniApp metadata is invalid"));
    }
    Ok(app)
}

pub(super) async fn generate(
    State(state): State<MiniAppM1RouterState>,
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
                "This MiniApp is already being created".into(),
            ));
        }
        if draft.import.is_some() {
            return Err(invalid("Import this MiniApp before editing it"));
        }
        draft
    } else {
        let mut draft = Draft {
            id: uuid::Uuid::now_v7().to_string(),
            revision: 0,
            name: "".into(),
            description: "".into(),
            html: "".into(),
            service_source: None,
            messages: vec![],
            status: "generating".into(),
            error: None,
            miniapp_id: None,
            base_release_digest: None,
            base_source_digest: None,
            updated_at: nomifun_common::now_ms(),
            import: None,
        };
        if let Some(id) = &request.miniapp_id {
            let app = service
                .application
                .workshop(&owner, id)
                .await
                .map_err(application_error)?;
            let source = service
                .application
                .source_file(&owner, id, "ui/index.html")
                .await
                .map_err(application_error)?;
            draft.name = app.miniapp.display_name;
            draft.description = app.miniapp.description.unwrap_or_default();
            draft.html = source.content;
            draft.base_source_digest = Some(source.source_snapshot_digest);
            draft.miniapp_id = Some(id.clone());
            draft.base_release_digest = app.miniapp.releases.active.map(|r| r.release_digest);
            if matches!(app.miniapp.kind, MiniAppKindDto::Service) {
                draft.service_source = Some(
                    service
                        .application
                        .source_file(&owner, id, "service/main.mjs")
                        .await
                        .map_err(application_error)?
                        .content,
                );
            }
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
    draft.status = "generating".into();
    draft.error = None;
    service.put_draft(&owner, &mut draft).await?;
    let token = CancellationToken::new();
    let job_key = format!("{owner}:{}", draft.id);
    service
        .jobs
        .lock()
        .await
        .insert(job_key.clone(), token.clone());
    let worker = service.clone();
    let snapshot = draft.clone();
    tokio::spawn(async move {
        let generated = tokio::select! {
            _=token.cancelled()=>return,
            result=tokio::time::timeout(Duration::from_secs(240),worker.complete(&request,&snapshot))=>result.unwrap_or_else(|_|Err(AppError::Timeout("MiniApp generation timed out".into())))
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
                latest.html = app.html;
                latest.status = "ready".into();
                latest.error = None;
                latest.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: app.assistant_message,
                });
            }
            Err(error) => {
                tracing::warn!(error=%error,"MiniApp generation failed");
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
            tracing::error!(error=%error,"Could not retain MiniApp creation result");
        }
    });
    Ok(Json(ApiResponse::ok(draft)))
}

impl MiniAppProductService {
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
        let prompt=serde_json::to_string(&serde_json::json!({"conversation":draft.messages,"current_html":draft.html,"service_source_read_only":draft.service_source,"instruction":"Produce the complete functioning MiniApp; only the HTML surface may be edited when service_source_read_only is present."})).map_err(internal)?;
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
        Err(invalid("Could not generate the MiniApp"))
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
