//! Typed, protocol-bound continuation data. Never instructions or tool output.
//! Do not reinterpret an Anthropic signature as Responses encrypted_content.

use crate::DigestHex;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

pub const MAX_THINKING_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_THINKING_OPAQUE_BYTES: usize = 256 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatProviderReasoning {
    AnthropicThinking {
        text: String,
        signature: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_digest: Option<DigestHex>,
    },
    AnthropicRedactedThinking {
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_digest: Option<DigestHex>,
    },
    /// Exact provider-produced raw Responses reasoning, including its item
    /// identity and native content. Never reconstruct it from a UI summary.
    OpenaiResponsesReasoning {
        item: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_digest: Option<DigestHex>,
    },
}

// Debug output must not accidentally disclose signed or redacted contents.
impl std::fmt::Debug for ChatProviderReasoning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AnthropicThinking { .. } => "AnthropicThinking([private])",
            Self::AnthropicRedactedThinking { .. } => "AnthropicRedactedThinking([private])",
            Self::OpenaiResponsesReasoning { .. } => "OpenaiResponsesReasoning([private])",
        })
    }
}

impl ChatProviderReasoning {
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid = match self {
            Self::AnthropicThinking {
                text, signature, ..
            } => {
                text.len() <= MAX_THINKING_TEXT_BYTES
                    && !signature.is_empty()
                    && signature.len() <= MAX_THINKING_OPAQUE_BYTES
            }
            Self::AnthropicRedactedThinking { data, .. } => {
                !data.is_empty() && data.len() <= MAX_THINKING_OPAQUE_BYTES
            }
            Self::OpenaiResponsesReasoning { item, .. } => validate_responses_reasoning_item(item),
        };
        let origin = match self {
            Self::AnthropicThinking { route_digest, .. }
            | Self::AnthropicRedactedThinking { route_digest, .. }
            | Self::OpenaiResponsesReasoning { route_digest, .. } => route_digest,
        };
        let valid_origin = origin.as_ref().is_none_or(|digest| {
            let value = digest.as_ref();
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
        (valid && valid_origin)
            .then_some(())
            .ok_or("invalid or oversized provider reasoning block")
    }

    /// The only portion eligible for a reasoning UI event. Empty signed text
    /// and redacted data have no visible text; neither gets a fabricated one.
    pub fn visible_text(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::AnthropicThinking { text, .. } if !text.is_empty() => Some(Cow::Borrowed(text)),
            Self::OpenaiResponsesReasoning { item, .. } => {
                let text = item.get("content")?.as_array()?.iter()
                    .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                    .collect::<String>();
                (!text.is_empty()).then_some(Cow::Owned(text))
            }
            _ => None,
        }
    }
}

fn validate_responses_reasoning_item(item: &serde_json::Value) -> bool {
    let Some(item) = item.as_object() else { return false };
    if item.get("type").and_then(serde_json::Value::as_str) != Some("reasoning")
        || !item.get("id").and_then(serde_json::Value::as_str).is_some_and(|id| {
            !id.trim().is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
        })
        || item.get("status").is_some_and(|status| !status.is_null() && status.as_str() != Some("completed"))
    { return false; }
    let Some(content) = item.get("content") else { return false };
    if !content.is_null() && !content.as_array().is_some_and(|content| {
        content.len() <= 128 && content.iter().all(|part| {
        part.get("type").and_then(serde_json::Value::as_str) == Some("reasoning_text")
            && part.get("text").is_some_and(serde_json::Value::is_string)
        })
    })
    { return false; }
    if item.get("summary").is_some_and(|summary| !summary.as_array().is_some_and(|parts| {
        parts.len() <= 128 && parts.iter().all(|part| {
            part.get("type").and_then(serde_json::Value::as_str) == Some("summary_text")
                && part.get("text").is_some_and(serde_json::Value::is_string)
        })
    })) { return false; }
    serde_json::to_vec(item).is_ok_and(|encoded| encoded.len() <= MAX_THINKING_TEXT_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_raw_responses_reasoning_is_bounded_private_and_serializable() {
        let item = json!({"id":"rs_raw", "type":"reasoning", "status":"completed", "summary":[],
            "content":[{"type":"reasoning_text","text":"first"},{"type":"reasoning_text","text":"second"}],
            "provider_extension":{"opaque":"preserved"}});
        let block = ChatProviderReasoning::OpenaiResponsesReasoning { item:item.clone(), route_digest:None };
        block.validate().unwrap();
        assert_eq!(block.visible_text().as_deref(), Some("firstsecond"));
        assert!(!format!("{block:?}").contains("first"));
        let roundtrip: ChatProviderReasoning = serde_json::from_value(serde_json::to_value(&block).unwrap()).unwrap();
        assert_eq!(block, roundtrip);
        let null_item = json!({"id":"rs_nullable", "type":"reasoning", "content":null,
            "summary":[],"encrypted_content":null,"status":null});
        let null_block = ChatProviderReasoning::OpenaiResponsesReasoning {item:null_item.clone(),route_digest:None};
        null_block.validate().unwrap();
        assert!(null_block.visible_text().is_none());
        let null_restored: ChatProviderReasoning = serde_json::from_value(serde_json::to_value(&null_block).unwrap()).unwrap();
        assert_eq!(null_restored,null_block);
        for invalid in [
            json!({"id":"rs_raw", "type":"function_call", "content":[{"type":"reasoning_text","text":"raw"}]}),
            json!({"id":"rs_raw", "type":"reasoning", "summary":[{"type":"summary_text","text":"summary only"}]}),
            json!({"id":"rs_raw", "type":"reasoning", "content":[{"type":"reasoning_text","text":"x".repeat(MAX_THINKING_TEXT_BYTES)}]}),
        ] {
            assert!(ChatProviderReasoning::OpenaiResponsesReasoning {item:invalid,route_digest:None}.validate().is_err());
        }
    }
}
