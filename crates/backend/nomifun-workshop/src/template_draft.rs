//! Minimal, stateless Template draft completion contract.
//!
//! This surface deliberately owns no Conversation, Skill, MCP, template, or
//! persistence lifecycle. The app supplies a runner that resolves one exact
//! managed Chat model and performs one provider completion. Tests replace that
//! runner without touching a live model.

use nomifun_common::{AppError, ProviderId};

/// Hard request limit, measured like the renderer's JavaScript strings.
pub const MAX_TEMPLATE_DRAFT_PROMPT_UTF16: usize = 20_000;
/// Provider model ids are managed natural keys, not free-form prompts.
pub const MAX_TEMPLATE_DRAFT_MODEL_UTF16: usize = 512;
/// One fixed wall-clock budget spanning config resolution and the provider
/// stream. The product schedules no retry or model failover; a selected
/// provider may still perform its existing bounded transport negotiation while
/// the downstream receiver remains live.
pub const TEMPLATE_DRAFT_TIMEOUT_SECS: u64 = 120;
/// A template draft is intentionally small and structurally bounded.
pub const TEMPLATE_DRAFT_MAX_TOKENS: u32 = 4_096;
/// Renderer limit for the JSON payload inside the canonical response fence.
pub const MAX_TEMPLATE_DRAFT_JSON_BYTES: usize = 262_144;
/// Hard local output budget: frontend JSON budget plus the only allowed
/// opening (` ```json\n `, 8 bytes) and closing (`\n````, 4 bytes) fences.
/// Unlike `max_tokens`, this bounds the UTF-8 bytes actually accumulated.
pub const MAX_TEMPLATE_DRAFT_RESPONSE_BYTES: usize = MAX_TEMPLATE_DRAFT_JSON_BYTES + 12;

/// Fixed instruction for the one-shot model. The model proposes only the
/// product-owned v1 draft artifact; it never owns ids, persistence, execution,
/// model binding, or media generation.
pub const TEMPLATE_DRAFT_SYSTEM_PROMPT: &str = r#"You design one minimal NomiFun Creative Studio template draft for manual review.

Return exactly one lowercase `json` fenced block and no text before or after it. The JSON must contain exactly this shape and no additional keys:
```json
{
  "kind": "nomifun.creative-studio.template-draft/v1",
  "summary": "short user-facing summary",
  "draft": {
    "mode": "single-image",
    "name": "template name",
    "description": "short description",
    "category": "short category",
    "promptTemplate": "Create a product poster for {{product_name}} featuring {{selling_points}}"
  }
}
```

Set draft.mode to exactly "single-image" or "multi-image-series".
For "single-image", promptTemplate may use only {{product_name}} and {{selling_points}}.
For "multi-image-series", promptTemplate may use only {{topic}}, {{style}}, and {{platform}}.
Use at least one allowed placeholder. Never nest placeholders or invent another placeholder.

Never include ids, revisions, timestamps, variables, visibility, tags, attachments, assets, or model bindings. Never save or run a template, call tools or another model, generate media, or claim that anything was persisted. The product will create private defaults only after the user reviews and explicitly applies the draft."#;

/// Validated request handed to the app-owned stateless completion runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateDraftRunRequest {
    pub provider_id: String,
    pub model: String,
    pub system_prompt: &'static str,
    pub user_text: String,
}

/// Small seam around the app's live Agent Chat provider resolver.
///
/// There is intentionally no tool/history/session/skill field here. The live
/// implementation can only perform the one text completion represented by
/// this value.
#[async_trait::async_trait]
pub trait TemplateDraftRunner: Send + Sync {
    async fn run(&self, request: TemplateDraftRunRequest) -> Result<String, AppError>;
}

/// Validate and canonicalize the client-owned part of one draft request.
pub fn template_draft_run_request(
    prompt: String,
    provider_id: String,
    model: String,
) -> Result<TemplateDraftRunRequest, AppError> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::BadRequest(
            "prompt must be a non-empty string".into(),
        ));
    }
    if prompt.encode_utf16().count() > MAX_TEMPLATE_DRAFT_PROMPT_UTF16 {
        return Err(AppError::BadRequest(format!(
            "prompt is too long (max {MAX_TEMPLATE_DRAFT_PROMPT_UTF16} UTF-16 code units)"
        )));
    }

    let provider_id = ProviderId::parse(provider_id)
        .map_err(|error| {
            AppError::BadRequest(format!(
                "model.providerId must be a canonical Provider UUIDv7: {error}"
            ))
        })?
        .into_string();
    if model.is_empty()
        || model.trim() != model
        || model.encode_utf16().count() > MAX_TEMPLATE_DRAFT_MODEL_UTF16
    {
        return Err(AppError::BadRequest(format!(
            "model.model must be a non-empty trimmed string no longer than {MAX_TEMPLATE_DRAFT_MODEL_UTF16} UTF-16 code units"
        )));
    }

    Ok(TemplateDraftRunRequest {
        provider_id,
        model,
        system_prompt: TEMPLATE_DRAFT_SYSTEM_PROMPT,
        user_text: prompt.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_validation_is_strict_and_uses_utf16_limits() {
        let provider_id = ProviderId::new().into_string();
        let request = template_draft_run_request(
            "  设计一组社交海报  ".into(),
            provider_id.clone(),
            "chat-model".into(),
        )
        .unwrap();
        assert_eq!(request.provider_id, provider_id);
        assert_eq!(request.model, "chat-model");
        assert_eq!(request.user_text, "设计一组社交海报");
        assert_eq!(request.system_prompt, TEMPLATE_DRAFT_SYSTEM_PROMPT);
        assert_eq!(TEMPLATE_DRAFT_TIMEOUT_SECS, 120);
        assert_eq!(MAX_TEMPLATE_DRAFT_JSON_BYTES, 262_144);
        assert_eq!(MAX_TEMPLATE_DRAFT_RESPONSE_BYTES, 262_156);
        assert_eq!(
            b"```json\n".len() + MAX_TEMPLATE_DRAFT_JSON_BYTES + b"\n```".len(),
            MAX_TEMPLATE_DRAFT_RESPONSE_BYTES
        );
        assert!(
            b"```json\n".len() + MAX_TEMPLATE_DRAFT_JSON_BYTES + 1 + b"\n```".len()
                > MAX_TEMPLATE_DRAFT_RESPONSE_BYTES
        );

        assert!(
            template_draft_run_request(" \r\n ".into(), ProviderId::new().into_string(), "m".into())
                .is_err()
        );
        assert!(
            template_draft_run_request(
                "😀".repeat(MAX_TEMPLATE_DRAFT_PROMPT_UTF16 / 2 + 1),
                ProviderId::new().into_string(),
                "m".into(),
            )
            .is_err()
        );
        assert!(
            template_draft_run_request("x".into(), "not-a-provider".into(), "m".into())
                .is_err()
        );
        assert!(
            template_draft_run_request(
                "x".into(),
                ProviderId::new().into_string(),
                " m ".into(),
            )
            .is_err()
        );
        assert!(
            template_draft_run_request(
                "x".into(),
                ProviderId::new().into_string(),
                "m".repeat(MAX_TEMPLATE_DRAFT_MODEL_UTF16 + 1),
            )
            .is_err()
        );
    }
}
