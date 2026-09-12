//! `ContextContributor` — the host-agnostic seam (design §3.5) that lets the
//! backend inject dynamic, per-turn context into the system prompt without the
//! engine hard-coding each source. The engine holds a list of contributors
//! (empty by default → behaviour byte-for-byte unchanged) and, at the start of
//! each turn, appends whatever they contribute to the system prompt.
//!
//! This is the foundation for turning "passive" platform features into "active"
//! injection (knowledge auto-RAG, inline memory, etc.) as registered
//! contributors rather than bespoke call-sites. It is purely additive: with no
//! contributors registered, `merge_pre_turn_context` returns the system prompt
//! unchanged.

use async_trait::async_trait;
use nomi_types::message::ContentBlock;

/// The server-owned facts for the user turn currently crossing the provider
/// boundary. Contributors receive text and attachment metadata, never image
/// bytes or model-supplied authority fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnContext {
    pub source_message_id: String,
    pub text: String,
    pub image_media_types: Vec<String>,
    pub cs_dialogue_id: Option<String>,
}

impl TurnContext {
    pub(crate) fn from_user_content(
        source_message_id: &str,
        content: &[ContentBlock],
        cs_dialogue_id: Option<String>,
    ) -> Self {
        let mut text = Vec::new();
        let mut image_media_types = Vec::new();
        for block in content {
            match block {
                ContentBlock::Text { text: value } => text.push(value.as_str()),
                ContentBlock::Image { media_type, .. } => {
                    image_media_types.push(media_type.clone());
                }
                _ => {}
            }
        }
        Self {
            source_message_id: source_message_id.to_owned(),
            text: text.join("\n"),
            image_media_types,
            cs_dialogue_id,
        }
    }
}

/// Stable host-context key populated by the customer-service transport. The
/// engine copies only this explicitly approved field into [`TurnContext`].
pub const CS_DIALOGUE_HOST_CONTEXT_KEY: &str = "cs_dialogue_id";

/// A source of dynamic per-turn context. Implementations live in the backend
/// (host) and are registered onto the engine; the engine stays host-agnostic.
#[async_trait]
pub trait ContextContributor: Send + Sync {
    /// Context to add to the system prompt for the upcoming turn, or `None` to
    /// contribute nothing this turn. Called once per turn before the model call.
    async fn pre_turn_context(&self) -> Option<String>;

    /// Context for the exact user turn currently being processed. Existing
    /// contributors remain source-compatible and keep their original behavior;
    /// TurnMiddleware adapters override this method when they need live input.
    async fn pre_turn_context_for_turn(&self, _turn: &TurnContext) -> Option<String> {
        self.pre_turn_context().await
    }

    /// Fallible host boundary for mandatory TurnMiddleware. Ordinary context
    /// contributors inherit the optional behavior above; policy middleware may
    /// override this method so a timeout/owner failure stops the provider call
    /// instead of silently continuing without required policy context.
    async fn pre_turn_context_for_turn_result(
        &self,
        turn: &TurnContext,
    ) -> Result<Option<String>, String> {
        Ok(self.pre_turn_context_for_turn(turn).await)
    }

    /// A short stable label for diagnostics/telemetry.
    fn label(&self) -> &str {
        "context_contributor"
    }
}

/// Append non-empty contributor contributions to `system`, each under a blank
/// line, in registration order. Empty / all-`None` → `system` returned
/// unchanged (the zero-contributor fast path the engine relies on). Pure so the
/// merge rule is unit-testable without an engine.
pub fn merge_pre_turn_context(system: String, contributions: Vec<String>) -> String {
    let mut out = system;
    for c in contributions {
        let trimmed = c.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(trimmed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_contributions_returns_system_unchanged() {
        let sys = "SYSTEM PROMPT".to_string();
        assert_eq!(merge_pre_turn_context(sys.clone(), vec![]), sys);
        // All-empty contributions are also a no-op.
        assert_eq!(
            merge_pre_turn_context(sys.clone(), vec!["".into(), "   ".into()]),
            sys
        );
    }

    #[test]
    fn appends_non_empty_contributions_in_order() {
        let out = merge_pre_turn_context(
            "BASE".to_string(),
            vec!["[KB] hit".into(), "".into(), "[memory] fact".into()],
        );
        assert_eq!(out, "BASE\n\n[KB] hit\n\n[memory] fact");
    }

    #[test]
    fn empty_system_with_one_contribution_has_no_leading_blank() {
        let out = merge_pre_turn_context(String::new(), vec!["only".into()]);
        assert_eq!(out, "only");
    }

    #[tokio::test]
    async fn trait_object_contributes_through_merge() {
        struct Fixed(&'static str);
        #[async_trait]
        impl ContextContributor for Fixed {
            async fn pre_turn_context(&self) -> Option<String> {
                Some(self.0.to_string())
            }
        }
        let contributors: Vec<Box<dyn ContextContributor>> =
            vec![Box::new(Fixed("alpha")), Box::new(Fixed("beta"))];
        let mut contributions = Vec::new();
        for c in &contributors {
            if let Some(s) = c.pre_turn_context().await {
                contributions.push(s);
            }
        }
        assert_eq!(merge_pre_turn_context("S".into(), contributions), "S\n\nalpha\n\nbeta");
    }

    #[test]
    fn turn_context_keeps_text_and_image_metadata_but_not_image_bytes() {
        let turn = TurnContext::from_user_content(
            "source-1",
            &[
                ContentBlock::Text {
                    text: "hello".to_owned(),
                },
                ContentBlock::Image {
                    media_type: "image/png".to_owned(),
                    data: "sensitive-base64".to_owned(),
                },
            ],
            Some("dialogue-1".to_owned()),
        );
        assert_eq!(turn.source_message_id, "source-1");
        assert_eq!(turn.text, "hello");
        assert_eq!(turn.image_media_types, ["image/png"]);
        assert_eq!(turn.cs_dialogue_id.as_deref(), Some("dialogue-1"));
        assert!(!format!("{turn:?}").contains("sensitive-base64"));
    }
}
