pub mod clear;
pub mod compact;
pub mod help;
pub mod quit;

use std::sync::Arc;

use async_trait::async_trait;

use crate::compact::state::CompactState;
use crate::output::OutputSink;
use nomi_config::compact::CompactConfig;
use nomi_providers::LlmProvider;
use nomi_types::message::Message;

/// Result of executing a slash command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResult {
    /// Command handled, continue the REPL loop.
    Continue,
    /// Exit the REPL.
    Exit,
}

/// Context passed to slash commands during execution.
pub struct CommandContext<'a> {
    pub messages: &'a mut Vec<Message>,
    pub compact_state: &'a mut CompactState,
    pub compact_config: &'a CompactConfig,
    pub provider: Arc<dyn LlmProvider>,
    pub model: &'a str,
    pub output: &'a dyn OutputSink,
    pub registry: &'a CommandRegistry,
}

/// A slash command that can be executed in the REPL.
#[async_trait]
pub trait SlashCommand: Send + Sync {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn description(&self) -> &str;
    async fn execute(
        &self,
        ctx: &mut CommandContext<'_>,
        args: &str,
    ) -> anyhow::Result<CommandResult>;
}

/// Registry of all available slash commands.
pub struct CommandRegistry {
    commands: Vec<Box<dyn SlashCommand>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn register(&mut self, cmd: Box<dyn SlashCommand>) {
        self.commands.push(cmd);
    }

    pub fn find(&self, name: &str) -> Option<&dyn SlashCommand> {
        self.commands.iter().find_map(|cmd| {
            if cmd.name() == name || cmd.aliases().contains(&name) {
                Some(cmd.as_ref())
            } else {
                None
            }
        })
    }

    pub fn all(&self) -> &[Box<dyn SlashCommand>] {
        &self.commands
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the default registry with all built-in commands.
pub fn default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(Box::new(compact::CompactCommand));
    registry.register(Box::new(clear::ClearCommand));
    registry.register(Box::new(help::HelpCommand));
    registry.register(Box::new(quit::QuitCommand));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_find_by_name() {
        let registry = default_registry();
        assert!(registry.find("compact").is_some());
        assert!(registry.find("clear").is_some());
        assert!(registry.find("help").is_some());
        assert!(registry.find("quit").is_some());
    }

    #[test]
    fn registry_find_by_alias() {
        let registry = default_registry();
        assert!(registry.find("exit").is_some());
        let cmd = registry.find("exit").unwrap();
        assert_eq!(cmd.name(), "quit");
    }

    #[test]
    fn registry_find_unknown_returns_none() {
        let registry = default_registry();
        assert!(registry.find("nonexistent").is_none());
    }

    #[test]
    fn registry_all_returns_all_commands() {
        let registry = default_registry();
        assert_eq!(registry.all().len(), 4);
    }
}
