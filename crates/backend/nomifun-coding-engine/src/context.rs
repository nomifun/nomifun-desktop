//! Bounded Coding model-context assembly.
//!
//! The platform owns durable SessionEvent history. This module only turns a
//! bounded projection plus current input into the existing ChatModelInput
//! contract; it does not persist or invent a second conversation aggregate.

use nomifun_chat_model_broker::{
    ChatMessage, ChatModelInput, ChatToolChoice, ChatToolDefinition,
};
use serde::{Deserialize, Serialize};

use crate::agents_md::AgentsMdContext;
use crate::error::CodingEngineError;

const DEFAULT_MAX_CONTEXT_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_HISTORY_MESSAGES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingContextBudget {
    pub max_context_bytes: usize,
    pub max_history_messages: usize,
}

impl Default for CodingContextBudget {
    fn default() -> Self {
        Self {
            max_context_bytes: DEFAULT_MAX_CONTEXT_BYTES,
            max_history_messages: DEFAULT_MAX_HISTORY_MESSAGES,
        }
    }
}

impl CodingContextBudget {
    pub fn validate(self) -> Result<Self, CodingEngineError> {
        if self.max_context_bytes == 0
            || self.max_history_messages == 0
            || self.max_context_bytes > 64 * 1024 * 1024
        {
            return Err(CodingEngineError::ContextAssembly(
                "Coding context budget is invalid".to_owned(),
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingContextDiagnostics {
    pub dropped_history_messages: usize,
    pub warnings: Vec<String>,
}

pub struct CodingContextAssembler;

impl CodingContextAssembler {
    pub fn assemble(
        mut input: ChatModelInput,
        history: Vec<ChatMessage>,
        current_message: ChatMessage,
        agents: &AgentsMdContext,
        budget: CodingContextBudget,
    ) -> Result<(ChatModelInput, CodingContextDiagnostics), CodingEngineError> {
        let budget = budget.validate()?;
        let mut diagnostics = CodingContextDiagnostics::default();

        if !agents.combined.is_empty() {
            input.instructions.push(agents.combined.clone());
        }
        diagnostics.warnings.extend(agents.warnings.clone());

        let mut bounded_history = history;
        if bounded_history.len() > budget.max_history_messages {
            let drop_count = bounded_history.len() - budget.max_history_messages;
            bounded_history.drain(..drop_count);
            diagnostics.dropped_history_messages = drop_count;
            diagnostics
                .warnings
                .push("old history messages were dropped by the message-count limit".to_owned());
        }
        input.messages.clear();
        input.messages.extend(bounded_history);
        input.messages.push(current_message);

        input = shrink_to_budget(input, budget.max_context_bytes, &mut diagnostics)?;
        input.tool_choice = normalize_tool_choice(&input.tools, input.tool_choice)?;
        input
            .validate()
            .map_err(|error| CodingEngineError::ContextAssembly(error.to_string()))?;
        Ok((input, diagnostics))
    }
}

fn normalize_tool_choice(
    tools: &[ChatToolDefinition],
    choice: ChatToolChoice,
) -> Result<ChatToolChoice, CodingEngineError> {
    if tools.is_empty() {
        return Ok(ChatToolChoice::None);
    }
    if matches!(choice, ChatToolChoice::None) {
        return Ok(ChatToolChoice::Auto);
    }
    if let ChatToolChoice::Specific { name } = &choice
        && !tools.iter().any(|tool| tool.name == *name)
    {
        return Err(CodingEngineError::ContextAssembly(format!(
            "tool choice {name:?} is not present in the selected ToolPlan"
        )));
    }
    Ok(choice)
}

fn shrink_to_budget(
    mut input: ChatModelInput,
    max_bytes: usize,
    diagnostics: &mut CodingContextDiagnostics,
) -> Result<ChatModelInput, CodingEngineError> {
    loop {
        let encoded = serde_json::to_vec(&input).map_err(|error| {
            CodingEngineError::ContextAssembly(format!("context could not be serialized: {error}"))
        })?;
        if encoded.len() <= max_bytes {
            return Ok(input);
        }

        if input.messages.len() <= 1 {
            return Err(CodingEngineError::ContextTooLarge {
                limit: max_bytes,
                actual: encoded.len(),
            });
        }
        input.messages.remove(0);
        diagnostics.dropped_history_messages =
            diagnostics.dropped_history_messages.saturating_add(1);
        diagnostics
            .warnings
            .push("old history messages were dropped by the byte budget".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::agents_md::AgentsMdContext;
    use nomifun_chat_model_broker::{
        ChatContentPart, ChatMessage, ChatResponseFormat, PromptCachePolicy,
    };

    fn base_input() -> ChatModelInput {
        ChatModelInput {
            instructions: Vec::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: ChatToolChoice::None,
            max_output_tokens: Some(100),
            reasoning: None,
            prompt_cache: PromptCachePolicy::Disabled,
            response_format: ChatResponseFormat::Text,
            requested_output_modalities: BTreeSet::new(),
            provider_round_parent: None,
            preserve_native_responses_items: false,
            metadata: Default::default(),
        }
    }

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: nomifun_chat_model_broker::ChatRole::User,
            content: vec![ChatContentPart::Text {
                text: text.to_owned(),
            }],
            provider_round_id: None,
        }
    }

    #[test]
    fn assembles_agents_context_and_retains_current_message() {
        let (input, diagnostics) = CodingContextAssembler::assemble(
            base_input(),
            vec![user("old")],
            user("current"),
            &AgentsMdContext {
                combined: "workspace rules".to_owned(),
                ..Default::default()
            },
            CodingContextBudget::default(),
        )
        .unwrap();
        assert_eq!(input.messages.len(), 2);
        assert_eq!(input.messages[1], user("current"));
        assert_eq!(input.instructions, vec!["workspace rules"]);
        assert_eq!(diagnostics.dropped_history_messages, 0);
    }

    #[test]
    fn drops_old_history_before_failing_current_message() {
        let (input, diagnostics) = CodingContextAssembler::assemble(
            base_input(),
            vec![user(&"old".repeat(256)), user(&"older".repeat(256))],
            user("current"),
            &AgentsMdContext::default(),
            CodingContextBudget {
                max_context_bytes: 700,
                max_history_messages: 16,
            },
        )
        .unwrap();
        assert_eq!(input.messages.len(), 1);
        assert!(diagnostics.dropped_history_messages >= 2);
    }

    #[test]
    fn rejects_context_that_cannot_fit_current_message() {
        let error = CodingContextAssembler::assemble(
            base_input(),
            Vec::new(),
            user(&"x".repeat(1024)),
            &AgentsMdContext::default(),
            CodingContextBudget {
                max_context_bytes: 64,
                max_history_messages: 1,
            },
        )
        .unwrap_err();
        assert!(matches!(error, CodingEngineError::ContextTooLarge { .. }));
    }
}
