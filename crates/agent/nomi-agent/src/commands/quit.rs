use async_trait::async_trait;

use super::{CommandContext, CommandResult, SlashCommand};

pub struct QuitCommand;

#[async_trait]
impl SlashCommand for QuitCommand {
    fn name(&self) -> &str {
        "quit"
    }

    fn aliases(&self) -> &[&str] {
        &["exit"]
    }

    fn description(&self) -> &str {
        "Exit the REPL"
    }

    async fn execute(
        &self,
        _ctx: &mut CommandContext<'_>,
        _args: &str,
    ) -> anyhow::Result<CommandResult> {
        Ok(CommandResult::Exit)
    }
}
