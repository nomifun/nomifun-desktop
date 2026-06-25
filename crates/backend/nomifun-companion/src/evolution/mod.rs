//! 桌面伙伴自进化引擎（design §5）。
//!
//! 独立于轻量记忆学习器（`crate::learner`）的后台管线：从采集的工具调用事件里
//! 挖出重复多步套路（`miner`，确定性无 LLM），起草 + 评审成 SKILL.md（`prompt` +
//! `engine`，`one_shot_completion`），物化为待审草稿 + `create_skill` 建议卡。

pub mod conversation_transcript;
pub mod engine;
pub mod miner;
pub mod prompt;
pub mod transcript;

pub use conversation_transcript::ConversationTranscriptSource;
pub use engine::{EvolutionEngine, EvolveRun};
pub use miner::{mine_patterns, mine_reflection_candidates, tool_call_signature, MinedPattern};
pub use transcript::{
    render_transcript, NoopTranscriptSource, ToolTrace, TranscriptAnchor, TranscriptSource, TranscriptTurn, TurnRole,
};
