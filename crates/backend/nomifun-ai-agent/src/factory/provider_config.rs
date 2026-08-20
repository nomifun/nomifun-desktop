//! Shared Chat capability resolver and one-shot completion support.

use std::path::Path;
use std::sync::Arc;

use nomi_config::config::{CliArgs, Config};
use nomi_providers::{LlmProvider, ProviderError, create_provider};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role};
use nomifun_api_types::{ModelTask, ModelTrait};
use nomifun_common::{AppError, ProviderId};
use nomifun_model_invoke::{
    AuthMaterial, AuthScheme, ModelInvokeService, ModelRef, ProtocolExecutorKind,
    protocol_task_descriptor,
};

use crate::types::NomiCompatOverrides;

use super::nomi::resolve_bedrock_config;

/// Image input is opt-in on the exact Chat capability. Runtime observations
/// may downgrade a declared vision model after an explicit upstream rejection,
/// but can never promote a capability that omitted `vision_input`.
pub(crate) fn capability_supports_image(
    provider_id: &str,
    model: &str,
    traits: &[ModelTrait],
) -> bool {
    traits.contains(&ModelTrait::VisionInput)
        && !nomifun_common::VisionUnsupportedRegistry::global()
            .is_unsupported(provider_id, model)
}

/// Intermediate result of resolving a provider DB row before building a full
/// `Config`. Used internally by both `resolve_provider_config` and the nomi
/// agent factory to avoid duplicating the load+decrypt+map+url logic.
pub(crate) struct ResolvedProviderFields {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
    pub compat_overrides: NomiCompatOverrides,
    pub bedrock_config: Option<nomi_config::config::BedrockConfig>,
    pub context_limit: Option<i64>,
    pub output_limit: Option<i64>,
}

fn invoke_error_to_app_error(error: nomifun_model_invoke::InvokeError) -> AppError {
    AppError::BadRequest(error.to_string())
}

/// Preserve the complete connection key ring for providers that rotate keys
/// on auth/rate-limit failures. Nomi's provider constructors accept the same
/// comma/newline-separated representation as provider settings; a newline is
/// used here because individual persisted keys are already trimmed.
fn agent_api_keys(auth: &AuthMaterial) -> Result<String, AppError> {
    let keys = auth.secrets();
    if keys.is_empty() {
        return Err(AppError::BadRequest(
            "selected Agent Chat connection carries no API keys".into(),
        ));
    }
    Ok(keys.join("\n"))
}

/// Resolve Chat through the same task capability and connection resolver used
/// by multimodal, realtime and health paths. Protocol is the sole serializer
/// authority; platform never chooses a URL or silently substitutes a family.
pub(crate) async fn resolve_provider_fields(
    invoke: &ModelInvokeService,
    provider_id: &str,
    model: &str,
) -> Result<ResolvedProviderFields, AppError> {
    ProviderId::try_from(provider_id).map_err(|_| {
        AppError::BadRequest("provider_id must be a canonical ProviderId".to_owned())
    })?;
    if model.is_empty() || model.trim() != model {
        return Err(AppError::BadRequest(
            "model must be trimmed and non-empty".to_owned(),
        ));
    }

    let task = invoke
        .resolve_task_config(
            &ModelRef {
                provider_id: provider_id.to_owned(),
                model: model.to_owned(),
            },
            ModelTask::Chat,
        )
        .await
        .map_err(invoke_error_to_app_error)?;
    let descriptor = protocol_task_descriptor(&task.protocol, ModelTask::Chat).ok_or_else(|| {
        AppError::BadRequest(format!("Unsupported Chat protocol {:?}", task.protocol))
    })?;
    if descriptor.executor != ProtocolExecutorKind::Agent {
        return Err(AppError::BadRequest(format!(
            "Protocol {:?} is not an Agent Chat protocol",
            task.protocol
        )));
    }

    let (provider, api_key, base_url, bedrock_config) = match task.protocol.as_str() {
        "openai.chat_text" => {
            if task.connection.auth.scheme != AuthScheme::Bearer {
                return Err(AppError::BadRequest(
                    "openai.chat_text requires a bearer-auth connection".into(),
                ));
            }
            (
                "openai".to_owned(),
                agent_api_keys(&task.connection.auth)?,
                Some(task.http_endpoint().map_err(invoke_error_to_app_error)?),
                None,
            )
        }
        "anthropic.messages" => {
            if !matches!(
                &task.connection.auth.scheme,
                AuthScheme::HeaderKey(name) if name.eq_ignore_ascii_case("x-api-key")
            ) {
                return Err(AppError::BadRequest(
                    "anthropic.messages requires a header_key:x-api-key connection".into(),
                ));
            }
            (
                "anthropic".to_owned(),
                agent_api_keys(&task.connection.auth)?,
                Some(task.http_endpoint().map_err(invoke_error_to_app_error)?),
                None,
            )
        }
        "gemini.generate_text" => {
            if !matches!(
                &task.connection.auth.scheme,
                AuthScheme::HeaderKey(name) if name.eq_ignore_ascii_case("x-goog-api-key")
            ) {
                return Err(AppError::BadRequest(
                    "gemini.generate_text requires a header_key:x-goog-api-key connection".into(),
                ));
            }
            (
                "gemini".to_owned(),
                agent_api_keys(&task.connection.auth)?,
                Some(task.http_endpoint().map_err(invoke_error_to_app_error)?),
                None,
            )
        }
        "bedrock.anthropic_messages" => {
            if task.connection.auth.scheme != AuthScheme::Bedrock {
                return Err(AppError::BadRequest(
                    "bedrock.anthropic_messages requires a bedrock-auth connection".into(),
                ));
            }
            let bedrock = resolve_bedrock_config(
                task.bedrock_config.as_deref(),
                &task.connection.auth.credentials,
            )
            .ok_or_else(|| {
                AppError::BadRequest(
                    "bedrock.anthropic_messages requires a valid providers.bedrock_config".into(),
                )
            })?;
            ("bedrock".to_owned(), String::new(), None, Some(bedrock))
        }
        protocol => {
            return Err(AppError::BadRequest(format!(
                "Unsupported Chat protocol {protocol:?}; expected openai.chat_text, anthropic.messages, gemini.generate_text or bedrock.anthropic_messages"
            )));
        }
    };

    let mut provider_body = task
        .provider_params
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("Chat capability provider_params must be a JSON object".into()))?;
    let max_tokens_field = match provider_body.remove("max_tokens_field") {
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            Some(value.trim().to_owned())
        }
        Some(_) => {
            return Err(AppError::BadRequest(
                "Chat provider_params.max_tokens_field must be a non-empty string".into(),
            ));
        }
        None => None,
    };
    let require_reasoning_content = match provider_body.remove("require_reasoning_content") {
        Some(serde_json::Value::Bool(value)) => Some(value),
        Some(_) => {
            return Err(AppError::BadRequest(
                "Chat provider_params.require_reasoning_content must be a boolean".into(),
            ));
        }
        None => None,
    };

    let compat_overrides = NomiCompatOverrides {
        // The resolver passes Nomi a complete task endpoint.
        api_path: base_url.as_ref().map(|_| String::new()),
        supports_image: Some(capability_supports_image(
            provider_id,
            model,
            &task.traits,
        )),
        max_tokens_field,
        require_reasoning_content,
        extra_body: (!provider_body.is_empty()).then_some(provider_body),
    };

    Ok(ResolvedProviderFields {
        provider,
        api_key,
        model: task.model,
        base_url,
        compat_overrides,
        bedrock_config,
        context_limit: task.context_limit,
        output_limit: task.output_limit,
    })
}

/// Resolve the exact Chat capability into a base `Config` suitable for LLM
/// calls. Protocol selects the serializer and endpoint; provider identity does
/// not participate in transport inference.
///
/// The returned `Config` does NOT include session-specific settings (MCP
/// servers, session directory, session mode) — callers layer those on top.
pub async fn resolve_provider_config(
    invoke: &ModelInvokeService,
    provider_id: &str,
    model: &str,
    workspace: &Path,
) -> Result<Config, AppError> {
    let fields = resolve_provider_fields(
        invoke,
        provider_id,
        model,
    )
    .await?;

    let cli_args = CliArgs {
        provider: Some(fields.provider),
        api_key: Some(fields.api_key),
        base_url: fields.base_url,
        model: Some(fields.model),
        max_tokens: None,
        max_turns: None,
        system_prompt: None,
        profile: None,
        auto_approve: false,
        project_dir: Some(workspace.to_path_buf()),
    };

    let mut config =
        Config::resolve(&cli_args).map_err(|e| AppError::Internal(format!("Config resolve failed: {e}")))?;

    // Apply bedrock and compat post-assignments
    config.bedrock = fields.bedrock_config;

    if let Some(field) = fields.compat_overrides.max_tokens_field {
        config.compat.max_tokens_field = Some(field);
    }
    if let Some(path) = fields.compat_overrides.api_path {
        config.compat.api_path = Some(path);
    }
    if let Some(required) = fields.compat_overrides.require_reasoning_content {
        config.compat.require_reasoning_content = Some(required);
    }
    config.compat.extra_body = fields.compat_overrides.extra_body;
    // One-shot consumers include robot vision, so the same persisted Chat
    // trait must govern provider serialization here as in long-lived agents.
    config.compat.supports_image = fields.compat_overrides.supports_image;

    Ok(config)
}

/// Which stream channel a delta came from, so callers can route reasoning
/// (thinking) deltas separately from the visible text answer.
///
/// Used by [`streaming_completion_text_or_reasoning`]: `Text` = `LlmEvent::TextDelta`
/// (the visible answer — what the final assembled string is built from);
/// `Reasoning` = `LlmEvent::ThinkingDelta` (the model's readable reasoning,
/// fanned out for observability but NOT part of the returned text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    /// A `TextDelta` — the visible answer text (assembled into the return value).
    Text,
    /// A `ThinkingDelta` — the model's reasoning (forwarded, not assembled).
    Reasoning,
}

/// Perform a single-turn LLM completion and return the assembled text response.
///
/// Builds an `LlmRequest` from the given config, streams events from the
/// provider, and concatenates `TextDelta` events until `Done` is received.
/// Errors from the provider or the stream are mapped to `AppError::BadGateway`.
pub async fn one_shot_completion(
    cfg: &Config,
    system: &str,
    messages: Vec<Message>,
    max_tokens: u32,
) -> Result<String, AppError> {
    streaming_completion(cfg, system, messages, max_tokens, |_| {}).await
}

/// Like [`one_shot_completion`] but invokes `on_delta` for every text chunk
/// as it streams in, so callers can fan deltas out (e.g. over WebSocket)
/// while the full reply is still being assembled.
pub async fn streaming_completion(
    cfg: &Config,
    system: &str,
    messages: Vec<Message>,
    max_tokens: u32,
    on_delta: impl FnMut(&str) + Send,
) -> Result<String, AppError> {
    let provider: Arc<dyn LlmProvider> = create_provider(cfg);

    let request = LlmRequest {
        model: cfg.model.clone(),
        system: system.to_owned(),
        messages,
        tools: vec![],
        max_tokens: Some(max_tokens),
        thinking: None,
        reasoning_effort: None,
    };

    let rx = provider.stream(&request).await.map_err(provider_error_to_app_error)?;

    drain_text_response_with(rx, on_delta).await
}

/// Like [`streaming_completion`] but for the Agent Execution planner: the
/// `on_delta` callback ALSO receives a [`DeltaKind`] so a caller can fan out the
/// model's reasoning (`ThinkingDelta`) separately from the visible answer
/// (`TextDelta`); and when the model emits its answer ONLY in the reasoning
/// channel (empty `content`), the returned String falls back to the assembled
/// reasoning text (see [`drain_text_or_reasoning`]) so a requested JSON can
/// still be recovered. This DIVERGES from the visible-answer one-shot semantics
/// on purpose and must only be used where reasoning-as-answer is acceptable
/// (planning / re-plan).
pub async fn streaming_completion_text_or_reasoning(
    cfg: &Config,
    system: &str,
    messages: Vec<Message>,
    max_tokens: u32,
    on_delta: impl FnMut(DeltaKind, &str) + Send,
) -> Result<String, AppError> {
    let provider: Arc<dyn LlmProvider> = create_provider(cfg);

    let request = LlmRequest {
        model: cfg.model.clone(),
        system: system.to_owned(),
        messages,
        tools: vec![],
        max_tokens: Some(max_tokens),
        thinking: None,
        reasoning_effort: None,
    };

    let rx = provider.stream(&request).await.map_err(provider_error_to_app_error)?;

    drain_text_or_reasoning(rx, on_delta).await
}

/// Convenience constructor for a user-role `Message` with a single text block.
pub fn user_message(text: impl Into<String>) -> Message {
    Message::new(Role::User, vec![ContentBlock::Text { text: text.into() }])
}

/// Drain an `LlmEvent` receiver, concatenating `TextDelta` payloads until
/// `Done` is received. Returns the assembled text or an error.
#[cfg(test)]
async fn drain_text_response(rx: tokio::sync::mpsc::Receiver<LlmEvent>) -> Result<String, AppError> {
    drain_text_response_with(rx, |_| {}).await
}

/// Drain variant that surfaces every text delta to `on_delta` as it arrives.
async fn drain_text_response_with(
    mut rx: tokio::sync::mpsc::Receiver<LlmEvent>,
    mut on_delta: impl FnMut(&str) + Send,
) -> Result<String, AppError> {
    let mut output = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            LlmEvent::TextDelta(delta) => {
                on_delta(&delta);
                output.push_str(&delta);
            }
            LlmEvent::Done { .. } => return Ok(output),
            LlmEvent::Error(msg) => {
                return Err(AppError::BadGateway(format!("LLM stream error: {msg}")));
            }
            // Ignore thinking deltas, tool use, and signatures for one-shot
            _ => {}
        }
    }

    // Channel closed without a Done event
    if output.is_empty() {
        Err(AppError::BadGateway(
            "LLM stream ended without producing a response".into(),
        ))
    } else {
        Ok(output)
    }
}

/// Drain variant for the Agent Execution planner: assembles `TextDelta` into the
/// primary buffer AND `ThinkingDelta` into a separate reasoning buffer (forwarding
/// both to `on_delta`, tagged). On `Done`, returns the text buffer when it has
/// content, otherwise FALLS BACK to the reasoning buffer.
///
/// Some OpenAI-compatible reasoning models (e.g. StepFun `step-*`) put their entire
/// answer — including a requested JSON — in the `reasoning_content` channel
/// (→ `ThinkingDelta`) and leave `content` (→ `TextDelta`) empty. The standard drain
/// then returns `Ok("")` on `Done`, so the planner saw an empty completion and fell
/// back to a single-task DAG (会话10). Returning the reasoning text on empty content
/// lets `extract_json_object` still recover the plan JSON. Kept separate from
/// [`drain_text_response_with`] so the visible-answer one-shot semantics are
/// unchanged.
async fn drain_text_or_reasoning(
    mut rx: tokio::sync::mpsc::Receiver<LlmEvent>,
    mut on_delta: impl FnMut(DeltaKind, &str) + Send,
) -> Result<String, AppError> {
    let mut text = String::new();
    let mut reasoning = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            LlmEvent::TextDelta(delta) => {
                on_delta(DeltaKind::Text, &delta);
                text.push_str(&delta);
            }
            LlmEvent::ThinkingDelta(delta) => {
                on_delta(DeltaKind::Reasoning, &delta);
                reasoning.push_str(&delta);
            }
            LlmEvent::Done { .. } => {
                return Ok(if text.trim().is_empty() { reasoning } else { text });
            }
            LlmEvent::Error(msg) => {
                return Err(AppError::BadGateway(format!("LLM stream error: {msg}")));
            }
            // Ignore tool use and thinking signatures.
            _ => {}
        }
    }

    // Channel closed without a Done event: prefer text, else reasoning.
    let out = if text.trim().is_empty() { reasoning } else { text };
    if out.is_empty() {
        Err(AppError::BadGateway(
            "LLM stream ended without producing a response".into(),
        ))
    } else {
        Ok(out)
    }
}

fn provider_error_to_app_error(e: ProviderError) -> AppError {
    AppError::BadGateway(format!("LLM provider error: {e}"))
}

#[cfg(test)]
mod image_override_tests {
    use super::*;

    #[test]
    fn absent_vision_trait_never_defaults_to_supported() {
        assert!(!capability_supports_image(
            "unlikely-prov-xyz",
            "unlikely-model",
            &[]
        ));
        assert!(capability_supports_image(
            "unlikely-prov-xyz",
            "unlikely-model",
            &[ModelTrait::VisionInput]
        ));
    }
}

#[cfg(test)]
mod provider_resolution_tests {
    use super::*;
    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderConnectionRepository,
        IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
        NewProviderModel, NewProviderModelCapability, SqliteProviderConnectionRepository,
        SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
        SqliteProviderRepository, init_database_memory,
    };
    use nomifun_model_invoke::{AdapterRegistry, default_adapters};

    const ENCRYPTION_KEY: [u8; 32] = [0x42; 32];

    struct ChatCase {
        provider_id: &'static str,
        protocol: &'static str,
        auth_scheme: &'static str,
        base_url: &'static str,
        base_url_override: Option<&'static str>,
        endpoint: Option<&'static str>,
        traits: &'static str,
        credentials: &'static str,
        provider_params: &'static str,
        bedrock_config: Option<&'static str>,
    }

    async fn resolve_case(case: ChatCase) -> ResolvedProviderFields {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_repo: Arc<dyn IProviderRepository> =
            Arc::new(SqliteProviderRepository::new(pool.clone()));
        let model_repo: Arc<dyn IProviderModelRepository> =
            Arc::new(SqliteProviderModelRepository::new(pool.clone()));
        let capability_repo: Arc<dyn IProviderModelCapabilityRepository> =
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
        let connection_repo: Arc<dyn IProviderConnectionRepository> =
            Arc::new(SqliteProviderConnectionRepository::new(pool));
        let encrypted_key = encrypt_string(case.credentials, &ENCRYPTION_KEY).unwrap();
        let capabilities = [NewProviderModelCapability {
            task: "chat",
            traits: case.traits,
            protocol: case.protocol,
            connection_role: "default",
            base_url_override: case.base_url_override,
            endpoint: case.endpoint,
            provider_params: case.provider_params,
            context_limit: Some(131_072),
            ..Default::default()
        }];
        provider_repo
            .create(
                CreateProviderParams {
                    provider_id: Some(case.provider_id),
                    // Deliberately unrelated: platform must never select the
                    // serializer, authentication or endpoint.
                    platform: "unrelated-platform",
                    name: "Protocol seam test",
                    base_url: case.base_url,
                    auth_scheme: case.auth_scheme,
                    credentials_encrypted: &encrypted_key,
                    enabled: true,
                    bedrock_config: case.bedrock_config,
                    sort_order: None,
                },
                &NewProviderModel {
                    model: "test-model",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .unwrap();
        let invoke = ModelInvokeService::new(
            provider_repo,
            model_repo,
            capability_repo,
            connection_repo,
            ENCRYPTION_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(default_adapters()),
        );
        resolve_provider_fields(&invoke, case.provider_id, "test-model")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn exact_openai_chat_capability_resolves_nomi_config() {
        let fields = resolve_case(ChatCase {
            provider_id: "0190f5fe-7c00-7a00-8000-000000000101",
            protocol: "openai.chat_text",
            auth_scheme: "bearer",
            base_url: "https://transport.example/root",
            base_url_override: Some("https://transport.example/openai-root"),
            endpoint: Some("/custom/chat"),
            traits: "[]",
            credentials: r#"{"api_keys":["test-secret","test-secret-2"]}"#,
            provider_params: r#"{"max_tokens_field":"max_completion_tokens","require_reasoning_content":true}"#,
            bedrock_config: None,
        })
        .await;
        assert_eq!(fields.provider, "openai");
        assert_eq!(fields.api_key, "test-secret\ntest-secret-2");
        assert_eq!(fields.base_url.as_deref(), Some("https://transport.example/openai-root/custom/chat"));
        assert_eq!(fields.compat_overrides.api_path.as_deref(), Some(""));
        assert_eq!(fields.compat_overrides.max_tokens_field.as_deref(), Some("max_completion_tokens"));
        assert_eq!(fields.compat_overrides.require_reasoning_content, Some(true));
        assert_eq!(fields.context_limit, Some(131_072));
        assert_eq!(fields.compat_overrides.supports_image, Some(false));
    }

    #[tokio::test]
    async fn vision_input_is_strictly_controlled_by_the_exact_chat_trait() {
        let fields = resolve_case(ChatCase {
            provider_id: "0190f5fe-7c00-7a00-8000-000000000105",
            protocol: "openai.chat_text",
            auth_scheme: "bearer",
            base_url: "https://transport.example/root",
            base_url_override: None,
            endpoint: Some("/chat/completions"),
            traits: r#"["vision_input"]"#,
            credentials: r#"{"api_keys":["test-secret"]}"#,
            provider_params: "{}",
            bedrock_config: None,
        })
        .await;
        assert_eq!(fields.compat_overrides.supports_image, Some(true));
    }

    #[tokio::test]
    async fn exact_anthropic_chat_capability_resolves_nomi_config() {
        let fields = resolve_case(ChatCase {
            provider_id: "0190f5fe-7c00-7a00-8000-000000000102",
            protocol: "anthropic.messages",
            auth_scheme: "header_key:x-api-key",
            base_url: "https://transport.example/root",
            base_url_override: Some("https://transport.example/anthropic-root"),
            endpoint: Some("/custom/messages?beta=true"),
            traits: "[]",
            credentials: r#"{"api_keys":["test-secret","test-secret-2"]}"#,
            provider_params: "{}",
            bedrock_config: None,
        })
        .await;
        assert_eq!(fields.provider, "anthropic");
        assert_eq!(fields.api_key, "test-secret\ntest-secret-2");
        assert_eq!(fields.base_url.as_deref(), Some("https://transport.example/anthropic-root/custom/messages?beta=true"));
        assert_eq!(fields.compat_overrides.api_path.as_deref(), Some(""));
        assert!(fields.bedrock_config.is_none());
    }

    #[tokio::test]
    async fn exact_gemini_chat_capability_resolves_nomi_config() {
        let fields = resolve_case(ChatCase {
            provider_id: "0190f5fe-7c00-7a00-8000-000000000103",
            protocol: "gemini.generate_text",
            auth_scheme: "header_key:x-goog-api-key",
            base_url: "https://transport.example/root",
            base_url_override: Some("https://transport.example/gemini-root"),
            endpoint: Some("/v1beta/models/{model}:streamGenerateContent?alt=sse"),
            traits: "[]",
            credentials: r#"{"api_keys":["test-secret","test-secret-2"]}"#,
            provider_params: "{}",
            bedrock_config: None,
        })
        .await;
        assert_eq!(fields.provider, "gemini");
        assert_eq!(fields.api_key, "test-secret\ntest-secret-2");
        assert_eq!(fields.base_url.as_deref(), Some("https://transport.example/gemini-root/v1beta/models/test-model:streamGenerateContent?alt=sse"));
        assert_eq!(fields.compat_overrides.api_path.as_deref(), Some(""));
        assert!(fields.bedrock_config.is_none());
    }

    #[tokio::test]
    async fn exact_bedrock_chat_capability_resolves_nomi_config() {
        let fields = resolve_case(ChatCase {
            provider_id: "0190f5fe-7c00-7a00-8000-000000000104",
            protocol: "bedrock.anthropic_messages",
            auth_scheme: "bedrock",
            // Bedrock is an SDK protocol and therefore owns no HTTP root.
            base_url: "",
            base_url_override: None,
            endpoint: None,
            traits: "[]",
            credentials: r#"{"access_key_id":"AKIATEST","secret_access_key":"bedrock-secret"}"#,
            provider_params: r#"{"top_k":11}"#,
            bedrock_config: Some(
                r#"{"auth_method":"accessKey","region":"us-east-1"}"#,
            ),
        })
        .await;
        assert_eq!(fields.provider, "bedrock");
        assert!(fields.api_key.is_empty());
        assert!(fields.base_url.is_none());
        let bedrock = fields.bedrock_config.unwrap();
        assert_eq!(bedrock.region.as_deref(), Some("us-east-1"));
        assert_eq!(bedrock.access_key_id.as_deref(), Some("AKIATEST"));
        assert_eq!(bedrock.secret_access_key.as_deref(), Some("bedrock-secret"));
        assert_eq!(
            fields.compat_overrides.extra_body.as_ref().unwrap()["top_k"],
            11
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_creates_correct_structure() {
        let msg = user_message("Hello, world!");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
        match &msg.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello, world!"),
            _ => panic!("Expected Text content block"),
        }
        assert!(msg.timestamp.is_none());
    }

    #[test]
    fn user_message_accepts_string() {
        let owned = String::from("test input");
        let msg = user_message(owned);
        match &msg.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "test input"),
            _ => panic!("Expected Text content block"),
        }
    }

    #[tokio::test]
    async fn drain_text_response_concatenates_deltas() {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tx.send(LlmEvent::TextDelta("Hello".into())).await.unwrap();
        tx.send(LlmEvent::TextDelta(", world!".into())).await.unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: nomi_types::message::TokenUsage::default(),
        })
        .await
        .unwrap();

        let result = drain_text_response(rx).await.unwrap();
        assert_eq!(result, "Hello, world!");
    }

    #[tokio::test]
    async fn drain_text_response_returns_error_on_llm_error_event() {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tx.send(LlmEvent::TextDelta("partial".into())).await.unwrap();
        tx.send(LlmEvent::Error("rate limited".into())).await.unwrap();

        let result = drain_text_response(rx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::BadGateway(_)));
    }

    #[tokio::test]
    async fn drain_text_response_returns_partial_on_channel_close() {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tx.send(LlmEvent::TextDelta("partial output".into())).await.unwrap();
        drop(tx); // close channel without Done

        let result = drain_text_response(rx).await.unwrap();
        assert_eq!(result, "partial output");
    }

    #[tokio::test]
    async fn drain_text_response_errors_on_empty_channel_close() {
        let (_tx, rx) = tokio::sync::mpsc::channel::<LlmEvent>(8);
        drop(_tx);

        let result = drain_text_response(rx).await;
        assert!(result.is_err());
    }

    #[test]
    fn provider_error_maps_to_bad_gateway() {
        let err = provider_error_to_app_error(ProviderError::Connection("timeout".into()));
        assert!(matches!(err, AppError::BadGateway(_)));
    }

    // 会话10 fix: a reasoning-only stream (Done with EMPTY content) — as StepFun
    // `step-*` produces — must return the REASONING text, not "", so the planner can
    // still recover the JSON. `drain_text_or_reasoning` falls back to reasoning.
    #[tokio::test]
    async fn drain_text_or_reasoning_falls_back_to_reasoning_on_empty_text() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tx.send(LlmEvent::ThinkingDelta(r#"{"tasks":[{"title":"A",""#.into())).await.unwrap();
        tx.send(LlmEvent::ThinkingDelta(r#"spec":"a","depends_on":[]}]}"#.into())).await.unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: nomi_types::message::TokenUsage::default(),
        })
        .await
        .unwrap();

        let result = drain_text_or_reasoning(rx, |_, _| {}).await.unwrap();
        assert_eq!(result, r#"{"tasks":[{"title":"A","spec":"a","depends_on":[]}]}"#);
    }

    // When BOTH text and reasoning are present, the text (visible answer) wins — the
    // reasoning fallback only kicks in for an EMPTY text buffer.
    #[tokio::test]
    async fn drain_text_or_reasoning_prefers_text_when_present() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tx.send(LlmEvent::ThinkingDelta("thinking…".into())).await.unwrap();
        tx.send(LlmEvent::TextDelta("real answer".into())).await.unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: nomi_types::message::TokenUsage::default(),
        })
        .await
        .unwrap();

        let result = drain_text_or_reasoning(rx, |_, _| {}).await.unwrap();
        assert_eq!(result, "real answer");
    }
}
