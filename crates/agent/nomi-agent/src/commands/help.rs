use async_trait::async_trait;

use super::{CommandContext, CommandResult, SlashCommand};

pub struct HelpCommand;

#[async_trait]
impl SlashCommand for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }

    fn description(&self) -> &str {
        "List available commands"
    }

    async fn execute(
        &self,
        ctx: &mut CommandContext<'_>,
        _args: &str,
    ) -> anyhow::Result<CommandResult> {
        let mut entries: Vec<(&str, &str)> = ctx
            .registry
            .all()
            .iter()
            .map(|cmd| (cmd.name(), cmd.description()))
            .collect();
        entries.sort_by_key(|(name, _)| *name);

        let mut output = String::from("Available commands:\n");
        for (name, desc) in entries {
            output.push_str(&format!("  /{} — {}\n", name, desc));
        }
        ctx.output.emit_info(output.trim_end());
        Ok(CommandResult::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nomi_providers::{LlmProvider, ProviderError};
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomi_types::message::Message;

    use super::*;
    use crate::commands::{CommandContext, default_registry};
    use crate::compact::state::CompactState;
    use crate::output::OutputSink;

    struct CaptureSink {
        messages: Mutex<Vec<String>>,
    }

    impl CaptureSink {
        fn new() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
            }
        }

        fn captured(&self) -> Vec<String> {
            self.messages.lock().unwrap().clone()
        }
    }

    impl OutputSink for CaptureSink {
        fn emit_text_delta(&self, _: &str, _: &str) {}
        fn emit_thinking(&self, _: &str, _: &str) {}
        fn emit_tool_call(&self, _: &str, _: &str, _: &str) {}
        fn emit_tool_result(&self, _: &str, _: &str, _: bool, _: &str) {}
        fn emit_stream_start(&self, _: &str) {}
        fn emit_stream_end(&self, _: &str, _: usize, _: u64, _: u64, _: u64, _: u64) {}
        fn emit_error(&self, _: &str) {}
        fn emit_info(&self, msg: &str) {
            self.messages.lock().unwrap().push(msg.to_string());
        }
    }

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
    async fn help_lists_all_commands() {
        let provider: Arc<dyn LlmProvider> = Arc::new(NullProvider);
        let registry = default_registry();
        let output = CaptureSink::new();
        let mut messages: Vec<Message> = Vec::new();
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

        let cmd = HelpCommand;
        let result = cmd.execute(&mut ctx, "").await.unwrap();
        assert_eq!(result, CommandResult::Continue);

        let captured = output.captured();
        assert_eq!(captured.len(), 1);
        let help_text = &captured[0];
        assert!(help_text.contains("/clear"));
        assert!(help_text.contains("/compact"));
        assert!(help_text.contains("/help"));
        assert!(help_text.contains("/quit"));
    }

    #[tokio::test]
    async fn help_output_is_sorted() {
        let provider: Arc<dyn LlmProvider> = Arc::new(NullProvider);
        let registry = default_registry();
        let output = CaptureSink::new();
        let mut messages: Vec<Message> = Vec::new();
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

        let cmd = HelpCommand;
        cmd.execute(&mut ctx, "").await.unwrap();

        let help_text = &output.captured()[0];
        let clear_pos = help_text.find("/clear").unwrap();
        let compact_pos = help_text.find("/compact").unwrap();
        let help_pos = help_text.find("/help").unwrap();
        let quit_pos = help_text.find("/quit").unwrap();

        assert!(clear_pos < compact_pos);
        assert!(compact_pos < help_pos);
        assert!(help_pos < quit_pos);
    }
}
