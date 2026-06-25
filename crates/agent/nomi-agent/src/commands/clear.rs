use async_trait::async_trait;

use super::{CommandContext, CommandResult, SlashCommand};
use crate::compact::state::CompactState;

pub struct ClearCommand;

#[async_trait]
impl SlashCommand for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }

    fn description(&self) -> &str {
        "Clear conversation history"
    }

    async fn execute(
        &self,
        ctx: &mut CommandContext<'_>,
        _args: &str,
    ) -> anyhow::Result<CommandResult> {
        ctx.messages.clear();
        *ctx.compact_state = CompactState::new();
        ctx.output.emit_info("Conversation cleared");
        Ok(CommandResult::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomi_providers::{LlmProvider, ProviderError};
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomi_types::message::{ContentBlock, Message, Role};

    use super::*;
    use crate::commands::{CommandContext, CommandRegistry};
    use crate::output::null_sink::NullSink;

    struct NullProvider;
    #[async_trait::async_trait]
    impl LlmProvider for NullProvider {
        async fn stream(
            &self,
            _: &LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }
    }

    #[tokio::test]
    async fn clear_empties_messages() {
        let provider: Arc<dyn LlmProvider> = Arc::new(NullProvider);
        let registry = CommandRegistry::new();
        let output = NullSink;
        let mut messages = vec![
            Message::new(
                Role::User,
                vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            ),
            Message::new(
                Role::Assistant,
                vec![ContentBlock::Text { text: "hi".into() }],
            ),
        ];
        let mut state = CompactState::new();
        state.last_input_tokens = 5000;
        state.consecutive_failures = 2;
        let config = nomi_config::compact::CompactConfig::default();

        let mut ctx = CommandContext {
            messages: &mut messages,
            compact_state: &mut state,
            compact_config: &config,
            provider,
            model: "test",
            output: &output,
            registry: &registry,
        };

        let cmd = ClearCommand;
        let result = cmd.execute(&mut ctx, "").await.unwrap();

        assert_eq!(result, CommandResult::Continue);
        assert!(ctx.messages.is_empty());
        assert_eq!(ctx.compact_state.last_input_tokens, 0);
        assert_eq!(ctx.compact_state.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn clear_on_empty_messages() {
        let provider: Arc<dyn LlmProvider> = Arc::new(NullProvider);
        let registry = CommandRegistry::new();
        let output = NullSink;
        let mut messages: Vec<Message> = vec![];
        let mut state = CompactState::new();
        let config = nomi_config::compact::CompactConfig::default();

        let mut ctx = CommandContext {
            messages: &mut messages,
            compact_state: &mut state,
            compact_config: &config,
            provider,
            model: "test",
            output: &output,
            registry: &registry,
        };

        let cmd = ClearCommand;
        let result = cmd.execute(&mut ctx, "").await.unwrap();
        assert_eq!(result, CommandResult::Continue);
        assert!(ctx.messages.is_empty());
    }
}
