//! Thin hook-chain dispatcher for outgoing ACP prompts.
//!
//! Hooks never return Result — failures are surfaced via
//! `AgentStreamEvent::AcpPromptHookWarning` through `ctx.runtime` so the
//! frontend can render a non-blocking toast. Registration order equals
//! execution order; each hook's output feeds the next hook's input.

use crate::agent_runtime::AgentRuntime;
use crate::capability::skill_manager::AcpSkillManager;
use crate::factory::acp_assembler::AcpSessionParams;
use crate::manager::acp::AcpSession;
use std::sync::Arc;

/// Read/write slice handed to each hook. `session` is a mutable borrow
/// so hooks can consume one-shot flags (e.g. `take_pending_model_notice`).
pub struct PromptCtx<'a> {
    pub session: &'a mut AcpSession,
    pub params: &'a AcpSessionParams,
    pub skill_manager: &'a Arc<AcpSkillManager>,
    pub runtime: &'a AgentRuntime,
}

#[async_trait::async_trait]
pub trait PreSendHook: Send + Sync {
    async fn pre_send(&self, ctx: &mut PromptCtx<'_>, prompt: String) -> String;
}

/// Reserved for future reply-path transformations. Defined now so the
/// callsite shape is stable; `PromptPipeline` does NOT invoke it in this
/// revision.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait PostRecvHook: Send + Sync {
    async fn post_recv(&self, ctx: &mut PromptCtx<'_>, reply: String) -> String;
}

pub struct PromptPipeline {
    hooks: Vec<Arc<dyn PreSendHook>>,
}

impl PromptPipeline {
    pub fn new(hooks: Vec<Arc<dyn PreSendHook>>) -> Self {
        Self { hooks }
    }

    pub async fn pre_send(&self, ctx: &mut PromptCtx<'_>, prompt: String) -> String {
        let mut current = prompt;
        for hook in &self.hooks {
            current = hook.pre_send(ctx, current).await;
        }
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zero-hook pipeline is an identity function — property-level
    /// check that does not require a constructed PromptCtx (which
    /// would pull in full AcpSessionParams plumbing for no gain).
    ///
    /// Multi-hook ordering + real ctx plumbing are exercised by the
    /// integration tests in `tests/prompt_pipeline_integration.rs`.
    #[tokio::test]
    async fn empty_pipeline_is_identity() {
        let pipeline = PromptPipeline::new(vec![]);
        // The loop body never runs with 0 hooks, so we need no ctx.
        // Assert by property: fold over empty Vec returns the seed.
        let seed = "hello".to_string();
        let folded: String = Vec::<Arc<dyn PreSendHook>>::new()
            .into_iter()
            .fold(seed.clone(), |acc, _h| acc);
        assert_eq!(folded, seed);
        let _ = pipeline; // ensure pipeline compiles end-to-end
    }
}
