//! Native tools for a summoned-companion work session (spec §设计 B2/B3).
//!
//! A summon loads one companion's memories **read-only** into an ordinary work
//! conversation: the session reuses `RecallMemoriesTool` (companion_tools.rs)
//! over a read-only sink, and write-back is confirmation-only through
//! `propose_companion_memory` — the agent proposes a candidate memory, the
//! backend turns it into a suggestion card, and nothing enters the memory
//! store until the owner accepts. `save_memory` is never registered in a
//! summoned session.
//!
//! Backend seams follow the `companion_tools.rs` pattern: `nomifun-companion`
//! injects concrete sinks; other hosts pass `None` and none of this exists.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::companion_tools::COMPANION_MEMORY_KINDS;
use crate::context_contributor::ContextContributor;

/// Backend seam for confirmation-style memory write-back from a summoned work
/// session. Implemented by `nomifun-companion` (writes a `companion_suggestions`
/// card, never `companion_memories`).
#[async_trait]
pub trait SummonProposalSink: Send + Sync {
    /// Submit one candidate memory (kind + content + why it is worth keeping).
    /// Returns a human-readable confirmation the model can quote.
    /// `conversation_id` is recorded as provenance on the suggestion card.
    async fn propose(
        &self,
        conversation_id: &str,
        kind: &str,
        content: &str,
        reason: &str,
    ) -> Result<String, String>;
}

/// Backend seam for the per-turn live memory-snapshot section of a summoned
/// session. Implementations resolve the summoned `memory_ids` against the
/// live store each turn (budgeted), so edits to a memory propagate naturally.
/// `None` = nothing to inject this turn (empty selection or resolver failure —
/// implementations log their own errors; a snapshot must never fail the turn).
#[async_trait]
pub trait SummonContextSink: Send + Sync {
    async fn resolve_context(&self) -> Option<String>;
}

/// `propose_companion_memory` — confirmation-style memory write-back.
pub struct ProposeCompanionMemoryTool {
    sink: Arc<dyn SummonProposalSink>,
    /// The conversation this tool instance serves — recorded as provenance.
    conversation_id: String,
}

impl ProposeCompanionMemoryTool {
    pub fn new(sink: Arc<dyn SummonProposalSink>, conversation_id: impl Into<String>) -> Self {
        Self {
            sink,
            conversation_id: conversation_id.into(),
        }
    }
}

#[async_trait]
impl Tool for ProposeCompanionMemoryTool {
    fn name(&self) -> &str {
        "propose_companion_memory"
    }

    fn description(&self) -> &str {
        "向被召唤伙伴的记忆库【提议】一条长期记忆（不会直接写入：生成建议卡，主人确认后才入库）。\
         仅在发现长期有价值的事实/偏好/约定时使用，宁缺毋滥；一句话自包含。kind 取值：\
         profile(稳定画像)/preference(偏好)/knowledge(可复用结论)/episode(带时间的经历)/\
         task(待办线索)/affective(情感)。"
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": COMPANION_MEMORY_KINDS},
                "content": {"type": "string", "description": "一句话记忆内容（中文，自包含）"},
                "reason": {"type": "string", "description": "为什么值得长期记住（给主人看的理由）"}
            },
            "required": ["kind", "content", "reason"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let kind = input.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
        let reason = input.get("reason").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !COMPANION_MEMORY_KINDS.contains(&kind) {
            return ToolResult {
                content: format!("kind 必须是 {COMPANION_MEMORY_KINDS:?} 之一"),
                is_error: true,
                images: Vec::new(),
            };
        }
        if content.is_empty() {
            return ToolResult {
                content: "content 不能为空".into(),
                is_error: true,
                images: Vec::new(),
            };
        }
        if reason.is_empty() {
            return ToolResult {
                content: "reason 不能为空（说明为什么值得长期记住）".into(),
                is_error: true,
                images: Vec::new(),
            };
        }
        match self
            .sink
            .propose(&self.conversation_id, kind, content, reason)
            .await
        {
            Ok(out) => ToolResult {
                content: out,
                is_error: false,
                images: Vec::new(),
            },
            Err(e) => ToolResult {
                content: e,
                is_error: true,
                images: Vec::new(),
            },
        }
    }

    fn category(&self) -> ToolCategory {
        // Writes only a suggestion card in the companion's own suggestion box
        // (never memories or user files) — Info, same rationale as save_memory.
        ToolCategory::Info
    }
}

/// Per-turn live memory-snapshot injection for a summoned session (spec §B2:
/// memories are re-resolved from the store each turn under a budget; the
/// conversation row never copies memory content). Empty resolution → `None`
/// (engine's zero-cost no-op path, same as `CompanionSkillContributor`).
pub struct SummonContextContributor {
    sink: Arc<dyn SummonContextSink>,
}

impl SummonContextContributor {
    pub fn new(sink: Arc<dyn SummonContextSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl ContextContributor for SummonContextContributor {
    async fn pre_turn_context(&self) -> Option<String> {
        self.sink.resolve_context().await.filter(|s| !s.trim().is_empty())
    }

    fn label(&self) -> &str {
        "companion_summon"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingProposalSink {
        proposals: Mutex<Vec<(String, String, String, String)>>,
    }

    #[async_trait]
    impl SummonProposalSink for RecordingProposalSink {
        async fn propose(
            &self,
            conversation_id: &str,
            kind: &str,
            content: &str,
            reason: &str,
        ) -> Result<String, String> {
            self.proposals.lock().unwrap().push((
                conversation_id.into(),
                kind.into(),
                content.into(),
                reason.into(),
            ));
            Ok("已生成建议卡".into())
        }
    }

    #[tokio::test]
    async fn propose_validates_kind_content_and_reason() {
        let sink = Arc::new(RecordingProposalSink { proposals: Mutex::new(vec![]) });
        let tool = ProposeCompanionMemoryTool::new(sink.clone(), "conv_s");
        assert!(tool.execute(json!({"kind": "bogus", "content": "x", "reason": "y"})).await.is_error);
        assert!(tool.execute(json!({"kind": "task", "content": "  ", "reason": "y"})).await.is_error);
        assert!(tool.execute(json!({"kind": "task", "content": "x", "reason": ""})).await.is_error);
        let ok = tool
            .execute(json!({"kind": "preference", "content": "主人喜欢 TDD", "reason": "多次强调"}))
            .await;
        assert!(!ok.is_error);
        let proposals = sink.proposals.lock().unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].0, "conv_s", "provenance conversation stamped");
        assert_eq!(proposals[0].1, "preference");
    }

    struct FixedContextSink(Option<String>);

    #[async_trait]
    impl SummonContextSink for FixedContextSink {
        async fn resolve_context(&self) -> Option<String> {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn summon_contributor_is_noop_when_empty() {
        let c = SummonContextContributor::new(Arc::new(FixedContextSink(None)));
        assert!(c.pre_turn_context().await.is_none());
        let blank = SummonContextContributor::new(Arc::new(FixedContextSink(Some("  ".into()))));
        assert!(blank.pre_turn_context().await.is_none());
    }

    #[tokio::test]
    async fn summon_contributor_passes_snapshot_through() {
        let c = SummonContextContributor::new(Arc::new(FixedContextSink(Some(
            "## 召唤的伙伴记忆（只读参考）\n- [preference] 深烘焙".into(),
        ))));
        let out = c.pre_turn_context().await.unwrap();
        assert!(out.contains("召唤的伙伴记忆"));
        assert_eq!(c.label(), "companion_summon");
    }
}
