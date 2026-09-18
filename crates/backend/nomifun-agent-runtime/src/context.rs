//! Bounded Nomi model-context assembly.
//!
//! The platform owns durable SessionEvent history. This module only turns a
//! bounded projection plus current input into the existing ChatModelInput
//! contract; it does not persist or invent a second conversation aggregate.

use nomifun_chat_model_broker::{
    ChatMessage, ChatModelInput, ChatRole, ChatToolChoice, ChatToolDefinition,
};
use serde::{Deserialize, Serialize};

use crate::agents_md::AgentsMdContext;
use crate::error::AgentEngineError;

const DEFAULT_MAX_CONTEXT_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_HISTORY_MESSAGES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentContextBudget {
    pub max_context_bytes: usize,
    pub max_history_messages: usize,
}

impl Default for AgentContextBudget {
    fn default() -> Self {
        Self {
            max_context_bytes: DEFAULT_MAX_CONTEXT_BYTES,
            max_history_messages: DEFAULT_MAX_HISTORY_MESSAGES,
        }
    }
}

impl AgentContextBudget {
    pub fn validate(self) -> Result<Self, AgentEngineError> {
        if self.max_context_bytes == 0
            || self.max_history_messages == 0
            || self.max_context_bytes > 64 * 1024 * 1024
        {
            return Err(AgentEngineError::ContextAssembly(
                "Nomi context budget is invalid".to_owned(),
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentContextDiagnostics {
    pub dropped_history_messages: usize,
    pub warnings: Vec<String>,
}

pub struct AgentContextAssembler;

impl AgentContextAssembler {
    pub fn assemble(
        mut input: ChatModelInput,
        history: Vec<ChatMessage>,
        current_message: ChatMessage,
        agents: &AgentsMdContext,
        budget: AgentContextBudget,
    ) -> Result<(ChatModelInput, AgentContextDiagnostics), AgentEngineError> {
        let budget = budget.validate()?;
        let mut diagnostics = AgentContextDiagnostics::default();

        if !agents.combined.is_empty() {
            input.instructions.push(agents.combined.clone());
        }
        diagnostics.warnings.extend(agents.warnings.clone());

        let mut bounded_history = history;
        let orphan_prefix = bounded_history.iter().position(|message| message.role == ChatRole::User)
            .unwrap_or(bounded_history.len());
        if orphan_prefix > 0 {
            bounded_history.drain(..orphan_prefix);
            diagnostics.dropped_history_messages = orphan_prefix;
            diagnostics.warnings.push("incomplete leading historical turn was omitted".into());
        }
        if bounded_history.len() > budget.max_history_messages {
            let mut drop_count = bounded_history.len() - budget.max_history_messages;
            // Never retain a tool result/assistant fragment whose user turn
            // was removed by truncation. Drop complete historical turns.
            while drop_count < bounded_history.len()
                && bounded_history[drop_count].role != ChatRole::User {
                drop_count += 1;
            }
            bounded_history.drain(..drop_count);
            diagnostics.dropped_history_messages += drop_count;
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
            .map_err(|error| AgentEngineError::ContextAssembly(error.to_string()))?;
        Ok((input, diagnostics))
    }
}

fn normalize_tool_choice(
    tools: &[ChatToolDefinition],
    choice: ChatToolChoice,
) -> Result<ChatToolChoice, AgentEngineError> {
    if tools.is_empty() {
        return Ok(ChatToolChoice::None);
    }
    if matches!(choice, ChatToolChoice::None) {
        return Ok(ChatToolChoice::Auto);
    }
    if let ChatToolChoice::Specific { name } = &choice
        && !tools.iter().any(|tool| tool.name == *name)
    {
        return Err(AgentEngineError::ContextAssembly(format!(
            "tool choice {name:?} is not present in the selected ToolPlan"
        )));
    }
    Ok(choice)
}

fn shrink_to_budget(
    mut input: ChatModelInput,
    max_bytes: usize,
    diagnostics: &mut AgentContextDiagnostics,
) -> Result<ChatModelInput, AgentEngineError> {
    loop {
        let encoded = serde_json::to_vec(&input).map_err(|error| {
            AgentEngineError::ContextAssembly(format!("context could not be serialized: {error}"))
        })?;
        if encoded.len() <= max_bytes {
            return Ok(input);
        }

        if input.messages.len() <= 1 {
            return Err(AgentEngineError::ContextTooLarge {
                limit: max_bytes,
                actual: encoded.len(),
            });
        }
        let drop_count = input.messages.iter().enumerate().skip(1)
            .find(|(_, message)| message.role == ChatRole::User)
            .map(|(index, _)| index)
            .unwrap_or(input.messages.len() - 1);
        input.messages.drain(..drop_count);
        diagnostics.dropped_history_messages =
            diagnostics.dropped_history_messages.saturating_add(drop_count);
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
        let (input, diagnostics) = AgentContextAssembler::assemble(
            base_input(),
            vec![user("old")],
            user("current"),
            &AgentsMdContext {
                combined: "workspace rules".to_owned(),
                ..Default::default()
            },
            AgentContextBudget::default(),
        )
        .unwrap();
        assert_eq!(input.messages.len(), 2);
        assert_eq!(input.messages[1], user("current"));
        assert_eq!(input.instructions, vec!["workspace rules"]);
        assert_eq!(diagnostics.dropped_history_messages, 0);
    }

    #[test]
    fn drops_old_history_before_failing_current_message() {
        let (input, diagnostics) = AgentContextAssembler::assemble(
            base_input(),
            vec![user(&"old".repeat(256)), user(&"older".repeat(256))],
            user("current"),
            &AgentsMdContext::default(),
            AgentContextBudget {
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
        let error = AgentContextAssembler::assemble(
            base_input(),
            Vec::new(),
            user(&"x".repeat(1024)),
            &AgentsMdContext::default(),
            AgentContextBudget {
                max_context_bytes: 64,
                max_history_messages: 1,
            },
        )
        .unwrap_err();
        assert!(matches!(error, AgentEngineError::ContextTooLarge { .. }));
    }

    #[test]
    fn count_budget_drops_complete_turn_instead_of_orphaning_assistant_output() {
        let mut assistant = user("old answer");
        assistant.role = ChatRole::Assistant;
        let current = user("current");
        let (input, diagnostics) = AgentContextAssembler::assemble(
            base_input(), vec![user("old question"), assistant, user("recent question")],
            current.clone(), &AgentsMdContext::default(),
            AgentContextBudget { max_context_bytes: 4096, max_history_messages: 2 },
        ).unwrap();
        assert_eq!(input.messages, vec![user("recent question"), current]);
        assert_eq!(diagnostics.dropped_history_messages, 2);
    }
}
