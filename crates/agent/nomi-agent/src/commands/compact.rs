use async_trait::async_trait;

use super::{CommandContext, CommandResult, SlashCommand};
use crate::compact::auto;
use nomi_types::compact::CompactTrigger;

pub struct CompactCommand;

#[async_trait]
impl SlashCommand for CompactCommand {
    fn name(&self) -> &str {
        "compact"
    }

    fn description(&self) -> &str {
        "Compress conversation context"
    }

    async fn execute(
        &self,
        ctx: &mut CommandContext<'_>,
        _args: &str,
    ) -> anyhow::Result<CommandResult> {
        if ctx.messages.len() <= 2 {
            ctx.output.emit_info("Context is already compact");
            return Ok(CommandResult::Continue);
        }

        // Reset circuit breaker — manual intent overrides protection
        ctx.compact_state.consecutive_failures = 0;

        let pre_tokens = ctx.compact_state.last_input_tokens;

        match auto::autocompact(
            ctx.provider.as_ref(),
            ctx.messages,
            ctx.model,
            ctx.compact_config,
            ctx.compact_state,
        )
        .await
        {
            Ok(result) => {
                let msgs_summarized = result.messages_summarized;
                *ctx.messages = result.messages;

                if let Some(boundary) = ctx.messages.first_mut() {
                    for block in &mut boundary.content {
                        if let nomi_types::message::ContentBlock::Text { text } = block
                            && text.starts_with(auto::BOUNDARY_PREFIX)
                        {
                            let metadata = nomi_types::compact::CompactMetadata {
                                trigger: CompactTrigger::Manual,
                                pre_compact_tokens: pre_tokens,
                                messages_summarized: msgs_summarized,
                            };
                            *text = format!(
                                "{}\n{}",
                                auto::BOUNDARY_PREFIX,
                                serde_json::to_string(&metadata)
                                    .expect("metadata serialization cannot fail")
                            );
                        }
                    }
                }

                ctx.output.emit_info(&format!(
                    "Context compacted: {}k → compact ({} messages summarized)",
                    pre_tokens / 1000,
                    msgs_summarized
                ));
            }
            Err(e) => {
                ctx.output.emit_warning(&format!("Compact failed: {}", e));
            }
        }

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
    use crate::compact::state::CompactState;
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
    async fn compact_already_compact_guard() {
        let provider: Arc<dyn LlmProvider> = Arc::new(NullProvider);
        let registry = CommandRegistry::new();
        let output = NullSink;
        let mut messages = vec![Message::new(
            Role::User,
            vec![ContentBlock::Text { text: "hi".into() }],
        )];
        let mut state = CompactState::new();
        let config = nomi_config::compact::CompactConfig::default();

        let mut ctx = CommandContext {
            messages: &mut messages,
            compact_state: &mut state,
            compact_config: &config,
            provider,
            model: "test-model",
            output: &output,
            registry: &registry,
        };

        let cmd = CompactCommand;
        let result = cmd.execute(&mut ctx, "").await.unwrap();
        assert_eq!(result, CommandResult::Continue);
        assert_eq!(ctx.messages.len(), 1);
    }

    #[tokio::test]
    async fn compact_resets_circuit_breaker() {
        let provider: Arc<dyn LlmProvider> = Arc::new(NullProvider);
        let registry = CommandRegistry::new();
        let output = NullSink;
        let mut messages: Vec<Message> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                };
                Message::new(
                    role,
                    vec![ContentBlock::Text {
                        text: format!("msg-{i}"),
                    }],
                )
            })
            .collect();
        let mut state = CompactState::new();
        state.consecutive_failures = 5;
        let config = nomi_config::compact::CompactConfig::default();

        let mut ctx = CommandContext {
            messages: &mut messages,
            compact_state: &mut state,
            compact_config: &config,
            provider,
            model: "test-model",
            output: &output,
            registry: &registry,
        };

        let cmd = CompactCommand;
        let _ = cmd.execute(&mut ctx, "").await;
        // Circuit breaker was reset to 0 before the call, then failure increments it
        assert!(ctx.compact_state.consecutive_failures <= 1);
    }
}
