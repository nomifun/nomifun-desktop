//! Shared helper that resolves a Provider DB row into a fully-configured
//! `nomi_config::config::Config`, and a standalone one-shot LLM completion
//! function for the IDMM sidecar.

use std::path::Path;
use std::sync::Arc;

use nomi_config::config::{CliArgs, Config};
use nomi_providers::{LlmProvider, ProviderError, create_provider};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role};
use nomifun_common::AppError;
use nomifun_db::IProviderRepository;

use crate::types::NomiCompatOverrides;

use super::nomi::{map_nomi_provider, resolve_bedrock_config, resolve_nomi_url_and_compat};

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
}

/// Load a provider row from the DB, decrypt its API key, map platform to nomi
/// provider name, and resolve base URL / compat / bedrock fields.
///
/// This is the shared extraction used by both the full `resolve_provider_config`
/// (which also calls `Config::resolve`) and the nomi factory `build()` (which
/// passes the pieces into `NomiResolvedConfig`).
pub(crate) async fn resolve_provider_fields(
    provider_repo: &Arc<dyn IProviderRepository>,
    encryption_key: &[u8; 32],
    provider_id: &str,
    model: &str,
) -> Result<ResolvedProviderFields, AppError> {
    let row = provider_repo
        .find_by_id(provider_id)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to load provider config: {e}")))?
        .ok_or_else(|| AppError::BadRequest(format!("Provider '{provider_id}' not found")))?;

    let api_key = nomifun_common::decrypt_string(&row.api_key_encrypted, encryption_key)?;

    let provider = map_nomi_provider(&row.platform, model, row.model_protocols.as_deref());

    let (base_url, compat_overrides) =
        resolve_nomi_url_and_compat(&row.platform, &row.base_url, &provider, row.is_full_url);

    let bedrock_config = if row.platform == "bedrock" {
        resolve_bedrock_config(row.bedrock_config.as_deref())
    } else {
        None
    };

    Ok(ResolvedProviderFields {
        provider,
        api_key,
        model: model.to_owned(),
        base_url,
        compat_overrides,
        bedrock_config,
        context_limit: row.context_limit,
    })
}

/// Resolve a provider DB row into a base `Config` suitable for LLM calls.
///
/// This performs: load provider row, decrypt API key, map platform to nomi
/// provider name, resolve base URL / compat overrides, build `CliArgs`,
/// call `Config::resolve`, then apply bedrock and compat post-assignments.
///
/// The returned `Config` does NOT include session-specific settings (MCP
/// servers, session directory, session mode) — callers layer those on top.
pub async fn resolve_provider_config(
    provider_repo: &Arc<dyn IProviderRepository>,
    encryption_key: &[u8; 32],
    provider_id: &str,
    model: &str,
    workspace: &Path,
) -> Result<Config, AppError> {
    let fields = resolve_provider_fields(provider_repo, encryption_key, provider_id, model).await?;

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

    Ok(config)
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
        max_tokens,
        thinking: None,
        reasoning_effort: None,
    };

    let rx = provider.stream(&request).await.map_err(provider_error_to_app_error)?;

    drain_text_response_with(rx, on_delta).await
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

fn provider_error_to_app_error(e: ProviderError) -> AppError {
    AppError::BadGateway(format!("LLM provider error: {e}"))
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
}
