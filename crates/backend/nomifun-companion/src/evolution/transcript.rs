//! 重水合原语（design 2026-06-23 采集接缝重构）。
//!
//! 技能起草需要"真实做法"，但采集器只存候选索引（工具形状 + 锚点，无内容）。
//! 内容的**唯一事实源**是会话库（`nomifun-conversation` 的 messages 表，永久 durable）。
//! 起草时按 [`TranscriptAnchor`] 定向拉取"那一段"转录，脱敏后喂给 drafter，**用完即弃，
//! 绝不落 companion 库**。会话被删 → `window` 返回 `None`，调用方降级回工具名步骤。

use async_trait::async_trait;
use nomifun_common::AppError;

/// 转录窗口的定位锚：一个代表性会话 + 时间区间（call_id 辅助精确命中）。
/// `conversation_id` 为空或时间区间无法定位 → 无法重水合（降级）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptAnchor {
    /// 代表性会话（wire 形式 conversation_id，采集器即以字符串存）。
    pub conversation_id: String,
    /// 窗口首/末工具调用的采集 ts（毫秒）；用于在会话里框出"那一段"。
    pub start_ts: i64,
    pub end_ts: i64,
    /// 窗口前后各额外保留的轮数（给 drafter 上下文）。
    pub pad_turns: usize,
    /// 窗口内工具 call_id（精确命中辅助；时间区间为主）。
    pub call_ids: Vec<String>,
}

/// 一条转录消息的角色（由 messages.type+position 推导：right=user, left=assistant）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRole {
    User,
    Assistant,
    Tool,
}

/// 工具调用痕迹（名 + 参数 + 结果摘要），**调用方负责脱敏后再放入**。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolTrace {
    pub name: String,
    pub args: Option<String>,
    pub result: Option<String>,
}

/// 一条转录消息（内容应已脱敏）。
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptTurn {
    pub role: TurnRole,
    pub text: String,
    pub tool: Option<ToolTrace>,
}

impl TranscriptTurn {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: TurnRole::User, text: text.into(), tool: None }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: TurnRole::Assistant, text: text.into(), tool: None }
    }
    pub fn tool(name: impl Into<String>, args: Option<String>, result: Option<String>) -> Self {
        Self {
            role: TurnRole::Tool,
            text: String::new(),
            tool: Some(ToolTrace { name: name.into(), args, result }),
        }
    }
}

/// 只读的转录来源：按 [`TranscriptAnchor`] 拉取相关窗口。
///
/// 实现者（P-D 的 `ConversationTranscriptSource`）走会话库仓储层
/// `get_messages(conv_id)`（user 无关），直接使用 canonical conversation ID，
/// 读 full content（非 compact），脱敏后返回。会话已删/无法定位 → `Ok(None)`。
#[async_trait]
pub trait TranscriptSource: Send + Sync {
    async fn window(&self, anchor: &TranscriptAnchor) -> Result<Option<Vec<TranscriptTurn>>, AppError>;
}

/// 兜底源：永远返回 `None`（会话库未装配前 / 测试）。drafter 据此降级回工具名步骤。
pub struct NoopTranscriptSource;

#[async_trait]
impl TranscriptSource for NoopTranscriptSource {
    async fn window(&self, _anchor: &TranscriptAnchor) -> Result<Option<Vec<TranscriptTurn>>, AppError> {
        Ok(None)
    }
}

/// 单行字符截断（与 collector 同风格，末尾省略号）。
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// 把转录窗口渲染成喂给 drafter 的文本行（每条消息一行；工具带参/果摘要）。
/// 每行截断到 `max_chars_per_line`；空白消息丢弃。
pub fn render_transcript(turns: &[TranscriptTurn], max_chars_per_line: usize) -> Vec<String> {
    let mut out = Vec::new();
    for turn in turns {
        let line = match (&turn.role, &turn.tool) {
            (TurnRole::Tool, Some(t)) => {
                let mut s = format!("工具 {}", t.name);
                if let Some(a) = t.args.as_deref().filter(|a| !a.trim().is_empty()) {
                    s.push_str(&format!("({})", clip(a, max_chars_per_line / 2)));
                }
                if let Some(r) = t.result.as_deref().filter(|r| !r.trim().is_empty()) {
                    s.push_str(&format!(" → {}", clip(r, max_chars_per_line / 2)));
                }
                s
            }
            (TurnRole::User, _) => {
                let t = turn.text.trim();
                if t.is_empty() {
                    continue;
                }
                format!("用户：{}", clip(t, max_chars_per_line))
            }
            (TurnRole::Assistant, _) => {
                let t = turn.text.trim();
                if t.is_empty() {
                    continue;
                }
                format!("助手：{}", clip(t, max_chars_per_line))
            }
            (TurnRole::Tool, None) => continue,
        };
        out.push(line);
    }
    out
}

#[cfg(test)]
pub(crate) mod test_util {
    //! Shared test stub. Constructed by `engine.rs` tests (P-C); the
    //! `dead_code` allow keeps the P-A-only build clean before that lands.
    #![allow(dead_code)]
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// 测试桩：返回预置窗口，并记录被请求的锚（验证三路确实经重水合）。
    pub(crate) struct StubTranscript {
        pub turns: Option<Vec<TranscriptTurn>>,
        pub seen: Arc<Mutex<Vec<TranscriptAnchor>>>,
    }

    impl StubTranscript {
        pub fn with(turns: Vec<TranscriptTurn>) -> Self {
            Self { turns: Some(turns), seen: Arc::new(Mutex::new(Vec::new())) }
        }
        pub fn missing() -> Self {
            Self { turns: None, seen: Arc::new(Mutex::new(Vec::new())) }
        }
    }

    #[async_trait]
    impl TranscriptSource for StubTranscript {
        async fn window(&self, anchor: &TranscriptAnchor) -> Result<Option<Vec<TranscriptTurn>>, AppError> {
            self.seen.lock().await.push(anchor.clone());
            Ok(self.turns.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_user_assistant_tool_lines_and_drops_empty() {
        let turns = vec![
            TranscriptTurn::user("帮我把这批图压缩"),
            TranscriptTurn::assistant("   "), // 空白 → 丢弃
            TranscriptTurn::tool("imagemin", Some("dir=assets".into()), Some("压了 12 张".into())),
            TranscriptTurn::assistant("已完成"),
        ];
        let lines = render_transcript(&turns, 200);
        assert_eq!(lines.len(), 3, "空白助手行应被丢弃: {lines:?}");
        assert!(lines[0].starts_with("用户："));
        assert!(lines[1].starts_with("工具 imagemin"));
        assert!(lines[1].contains("dir=assets"));
        assert!(lines[1].contains("→ 压了 12 张"));
        assert!(lines[2].starts_with("助手："));
    }

    #[test]
    fn clips_long_lines() {
        let long = "x".repeat(500);
        let lines = render_transcript(&[TranscriptTurn::user(long)], 100);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].chars().count() <= "用户：".chars().count() + 100 + 1);
        assert!(lines[0].ends_with('…'));
    }

    #[tokio::test]
    async fn noop_source_returns_none() {
        let src = NoopTranscriptSource;
        let got = src.window(&TranscriptAnchor::default()).await.unwrap();
        assert!(got.is_none());
    }
}
