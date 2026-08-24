//! Provider-wire prompt fitting for image and video requests.
//!
//! Creation tasks keep the user's canonical prompt unchanged. This module only
//! derives the text placed on a provider request when an exact protocol limit
//! is known, or when the provider explicitly rejects the first attempt as too
//! long. That separation keeps prompt-library assets, canvas drafts, task
//! history and retry provenance lossless while preventing a provider-specific
//! ceiling from breaking every product surface that shares this invoke layer.

use crate::error::{InvokeError, InvokeErrorKind};
use crate::types::TaskRequest;

const STEPFUN_IMAGE_PROMPT_MAX_CHARS: usize = 512;
const GENERIC_MEDIA_RETRY_MAX_CHARS: usize = 512;
const OMITTED_SEPARATOR: &str = "\n…\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MediaPromptAdaptation {
    pub original_chars: usize,
    pub submitted_chars: usize,
    pub limit: usize,
}

/// Apply only an exact, documented protocol ceiling before the first request.
pub(crate) fn apply_known_media_prompt_limit(
    protocol: &str,
    model: &str,
    request: &mut TaskRequest,
) -> Option<MediaPromptAdaptation> {
    let limit = match (protocol, model) {
        // Step Image Edit 2 documents the same 512-character ceiling for its
        // text-to-image and image-edit endpoints. `stepfun.images` is the
        // exact persisted protocol for both StepFun and Step Plan connections.
        ("stepfun.images", "step-image-edit-2") => STEPFUN_IMAGE_PROMPT_MAX_CHARS,
        _ => return None,
    };
    adapt_media_request_prompt(request, limit)
}

/// After an upstream 400/422 explicitly reports a long prompt, derive one
/// bounded retry from the provider's stated limit (or a conservative media
/// fallback when the error omits the number). Never retries unrelated invalid
/// parameters or negative-prompt failures.
pub(crate) fn apply_media_prompt_length_retry(
    protocol: &str,
    model: &str,
    request: &mut TaskRequest,
    error: &InvokeError,
) -> Option<MediaPromptAdaptation> {
    if !is_positive_prompt_too_long(error) {
        return None;
    }
    let original_chars = media_prompt(request)?.chars().count();
    let limit = prompt_limit_from_error(&error.message)
        .or_else(|| known_prompt_limit(protocol, model))
        .unwrap_or_else(|| {
            GENERIC_MEDIA_RETRY_MAX_CHARS.min((original_chars / 2).max(1))
        })
        .min(original_chars.saturating_sub(1));
    (limit > 0).then_some(())?;
    adapt_media_request_prompt(request, limit)
}

fn known_prompt_limit(protocol: &str, model: &str) -> Option<usize> {
    match (protocol, model) {
        ("stepfun.images", "step-image-edit-2") => Some(STEPFUN_IMAGE_PROMPT_MAX_CHARS),
        _ => None,
    }
}

fn is_positive_prompt_too_long(error: &InvokeError) -> bool {
    if error.kind != InvokeErrorKind::InvalidParams
        || !matches!(error.http_status, Some(400 | 422))
    {
        return false;
    }
    let normalized = error.message.to_lowercase();
    if normalized.contains("negative_prompt") || normalized.contains("negative prompt") {
        return false;
    }
    normalized.contains("prompt_too_long")
        || ((normalized.contains("prompt") || normalized.contains("提示词"))
            && (normalized.contains("too long")
                || normalized.contains("exceed")
                || normalized.contains("超长")
                || normalized.contains("过长")
                || normalized.contains("太长")))
}

fn prompt_limit_from_error(message: &str) -> Option<usize> {
    let normalized = message.to_lowercase();
    if let Some(index) = normalized.find("between [").or_else(|| normalized.find("between (")) {
        let numbers = ascii_integers(&normalized[index..]);
        if let Some(limit) = numbers.get(1).copied().filter(|limit| *limit > 0) {
            return Some(limit);
        }
    }
    for marker in ["maximum", "max", "最多", "up to", "limit"] {
        let Some(index) = normalized.find(marker) else {
            continue;
        };
        if let Some(limit) = limit_near_marker(&normalized[index + marker.len()..]) {
            return Some(limit);
        }
    }
    None
}

fn limit_near_marker(value: &str) -> Option<usize> {
    let digit_index = value.find(|ch: char| ch.is_ascii_digit())?;
    if digit_index > 48 {
        return None;
    }
    let qualifier = value[..digit_index].to_lowercase();
    if ["actual", "current", "received", "got", "has", "counted"]
        .iter()
        .any(|word| qualifier.contains(word))
    {
        return None;
    }
    ascii_integers(&value[digit_index..])
        .into_iter()
        .find(|limit| *limit > 0)
}

fn ascii_integers(value: &str) -> Vec<usize> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(number) = current.parse() {
                numbers.push(number);
            }
            current.clear();
        }
    }
    if !current.is_empty()
        && let Ok(number) = current.parse()
    {
        numbers.push(number);
    }
    numbers
}

fn media_prompt(request: &TaskRequest) -> Option<&str> {
    match request {
        TaskRequest::ImageGeneration(request) => Some(&request.prompt),
        TaskRequest::ImageEdit(request) => Some(&request.prompt),
        TaskRequest::VideoGeneration(request) => Some(&request.prompt),
        _ => None,
    }
}

fn media_prompt_mut(request: &mut TaskRequest) -> Option<&mut String> {
    match request {
        TaskRequest::ImageGeneration(request) => Some(&mut request.prompt),
        TaskRequest::ImageEdit(request) => Some(&mut request.prompt),
        TaskRequest::VideoGeneration(request) => Some(&mut request.prompt),
        _ => None,
    }
}

fn adapt_media_request_prompt(
    request: &mut TaskRequest,
    limit: usize,
) -> Option<MediaPromptAdaptation> {
    let prompt = media_prompt_mut(request)?;
    let original_chars = prompt.chars().count();
    let compacted = compact_prompt(prompt, limit)?;
    let submitted_chars = compacted.chars().count();
    *prompt = compacted;
    Some(MediaPromptAdaptation {
        original_chars,
        submitted_chars,
        limit,
    })
}

/// Keep the opening task/subject instructions and the closing constraints.
/// Curated media prompts commonly place identity and scene setup first, then
/// global consistency/quality requirements last. A visible omission marker is
/// preferable to cutting at an arbitrary byte or silently keeping only one
/// side. The canonical prompt is never mutated; this output is wire-only.
fn compact_prompt(prompt: &str, limit: usize) -> Option<String> {
    let char_count = prompt.chars().count();
    if char_count <= limit {
        return None;
    }
    let separator_chars = OMITTED_SEPARATOR.chars().count();
    if limit <= separator_chars {
        return Some(prompt.chars().take(limit).collect());
    }
    let available = limit - separator_chars;
    let tail_chars = available / 4;
    let head_chars = available - tail_chars;
    let head = prompt.chars().take(head_chars).collect::<String>();
    let tail = prompt
        .chars()
        .skip(char_count.saturating_sub(tail_chars))
        .collect::<String>();
    Some(format!(
        "{}{}{}",
        head.trim_end(),
        OMITTED_SEPARATOR,
        tail.trim_start()
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{ImageEditRequest, ImageGenRequest, VideoGenRequest};

    fn image_request(prompt: String) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt,
            count: 1,
            size: None,
            quality: None,
            extra: json!({}),
        })
    }

    fn video_request(prompt: String) -> TaskRequest {
        TaskRequest::VideoGeneration(VideoGenRequest {
            prompt,
            seconds: None,
            size: None,
            inputs: vec![],
            extra: json!({}),
        })
    }

    #[test]
    fn documented_stepfun_limit_derives_wire_prompt_without_losing_both_ends() {
        let original = format!("START-{}-END", "角".repeat(900));
        let mut request = image_request(original.clone());
        let adaptation = apply_known_media_prompt_limit(
            "stepfun.images",
            "step-image-edit-2",
            &mut request,
        )
        .expect("long StepFun prompt is adapted");
        let prompt = media_prompt(&request).unwrap();

        assert_eq!(adaptation.original_chars, original.chars().count());
        assert!(adaptation.submitted_chars <= 512);
        assert_eq!(adaptation.limit, 512);
        assert!(prompt.starts_with("START-"));
        assert!(prompt.ends_with("-END"));
        assert!(prompt.contains(OMITTED_SEPARATOR));
        assert_eq!(original.chars().count(), 910, "canonical source remains available to caller");
    }

    #[test]
    fn short_or_unknown_protocol_prompt_is_not_changed_preflight() {
        let mut short = image_request("短提示词".into());
        assert_eq!(
            apply_known_media_prompt_limit("stepfun.images", "step-image-edit-2", &mut short),
            None
        );
        assert_eq!(media_prompt(&short), Some("短提示词"));

        let long = "x".repeat(900);
        let mut unknown = image_request(long.clone());
        assert_eq!(
            apply_known_media_prompt_limit("openai.images", "gpt-image-1", &mut unknown),
            None
        );
        assert_eq!(media_prompt(&unknown), Some(long.as_str()));
    }

    #[test]
    fn provider_reported_limit_adapts_video_for_one_retry() {
        let original = format!("opening {} closing", "scene ".repeat(300));
        let mut request = video_request(original);
        let error = InvokeError::new(
            InvokeErrorKind::InvalidParams,
            r#"provider returned 400 Bad Request: {"error":{"type":"prompt_too_long","message":"prompt length between [1,256]"}}"#,
        )
        .with_http_status(400);

        let adaptation = apply_media_prompt_length_retry(
            "custom.video",
            "video-model",
            &mut request,
            &error,
        )
        .expect("provider limit triggers one derived retry prompt");
        assert_eq!(adaptation.limit, 256);
        assert!(adaptation.submitted_chars <= 256);
        assert!(media_prompt(&request).unwrap().starts_with("opening"));
        assert!(media_prompt(&request).unwrap().ends_with("closing"));
    }

    #[test]
    fn prompt_error_without_a_stated_limit_uses_a_conservative_retry_budget() {
        let mut request = video_request("镜".repeat(1_200));
        let error = InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "provider returned 400: prompt too long; actual length is 1200",
        )
        .with_http_status(400);

        let adaptation = apply_media_prompt_length_retry(
            "custom.video",
            "video-model",
            &mut request,
            &error,
        )
        .expect("prompt-too-long error gets one conservative retry");
        assert_eq!(adaptation.limit, 512);
        assert!(adaptation.submitted_chars <= 512);
    }

    #[test]
    fn retry_ignores_unrelated_and_negative_prompt_errors() {
        let prompt = "x".repeat(900);
        for message in [
            "provider returned 400: invalid size",
            "provider returned 400: negative_prompt is too long, max 128",
        ] {
            let mut request = video_request(prompt.clone());
            let error = InvokeError::new(InvokeErrorKind::InvalidParams, message)
                .with_http_status(400);
            assert_eq!(
                apply_media_prompt_length_retry(
                    "custom.video",
                    "video-model",
                    &mut request,
                    &error,
                ),
                None
            );
            assert_eq!(media_prompt(&request), Some(prompt.as_str()));
        }
    }

    #[test]
    fn image_edit_uses_the_same_media_boundary_and_non_media_is_ignored() {
        let mut edit = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "修".repeat(700),
            count: 1,
            size: None,
            inputs: vec![],
            extra: json!({}),
        });
        assert!(
            apply_known_media_prompt_limit("stepfun.images", "step-image-edit-2", &mut edit)
                .is_some()
        );
        assert!(media_prompt(&edit).unwrap().chars().count() <= 512);

        let mut embedding = TaskRequest::Embedding(crate::types::EmbedRequest {
            inputs: vec!["x".repeat(900)],
            extra: json!({}),
        });
        assert_eq!(
            apply_known_media_prompt_limit(
                "stepfun.images",
                "step-image-edit-2",
                &mut embedding,
            ),
            None
        );
    }
}
