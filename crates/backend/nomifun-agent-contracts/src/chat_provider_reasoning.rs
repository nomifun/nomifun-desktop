//! Typed, protocol-bound continuation data. Never instructions or tool output.
//! Do not reinterpret an Anthropic signature as Responses encrypted_content.

use crate::DigestHex;
use serde::{Deserialize, Serialize};

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
}

// Debug output must not accidentally disclose signed or redacted contents.
impl std::fmt::Debug for ChatProviderReasoning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AnthropicThinking { .. } => "AnthropicThinking([private])",
            Self::AnthropicRedactedThinking { .. } => "AnthropicRedactedThinking([private])",
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
        };
        let origin = match self {
            Self::AnthropicThinking { route_digest, .. }
            | Self::AnthropicRedactedThinking { route_digest, .. } => route_digest,
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
    pub fn visible_text(&self) -> Option<&str> {
        match self {
            Self::AnthropicThinking { text, .. } if !text.is_empty() => Some(text),
            _ => None,
        }
    }
}
