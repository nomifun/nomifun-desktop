//! Prompt assembly + strict-JSON parsing for learning runs, plus the shared
//! persona flavor text (the companion-chat system prompt lives in
//! `companion::build_companion_system_prompt`).

use serde::Deserialize;

use crate::store::{MEMORY_KINDS, CompanionMemory};

pub const LEARN_MAX_TOKENS: u32 = 4096;

/// Valid moods the companion can be in (renderer maps each to an animation).
pub const MOODS: [&str; 5] = ["happy", "content", "sleepy", "worried", "excited"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearnedMemory {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_importance")]
    pub importance: f64,
}

fn default_importance() -> f64 {
    0.5
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearnOutput {
    #[serde(default)]
    pub memories: Vec<LearnedMemory>,
    #[serde(default)]
    pub reinforce_memory_ids: Vec<String>,
    #[serde(default)]
    pub supersede_memory_ids: Vec<String>,
    #[serde(default)]
    pub mood: Option<String>,
    #[serde(default)]
    pub diary: Option<String>,
}

/// System prompt of one learning pass. Written in the FIRST PERSON SINGULAR on
/// purpose: 共享记忆已删除，这一轮蒸馏出来的每条记忆都只属于发起学习的那一个伙伴，
/// 所以不能再用「所有伙伴共享的记忆中枢」那种口吻，否则模型会产出泛化的、
/// 不属于任何具体伙伴的「中枢」式记忆。
pub const LEARN_SYSTEM: &str = r#"你是主人的电子伙伴，正在整理只属于你自己的记忆。你的任务是阅读主人最近的工作事件记录，提炼出让你"更懂主人"的记忆。

这些记忆只写进你自己的记忆库，别的伙伴看不到、也不会检索到，所以请用"我和主人之间"的视角来写，不要替其他伙伴代言。

记忆 kind 只能是：profile(画像,稳定事实) / preference(偏好,风格口味) / knowledge(知识,可复用结论) / episode(事件,带时间的经历) / task(任务线索,未完成事项或口头承诺) / affective(情感,情绪轨迹)。

规则：
1. 只提炼有信息量的内容，宁缺毋滥；每条记忆一句话、自包含、用中文。
2. 若新事件印证了"已有记忆"列表中的某条，把它的 memory_id 放进 reinforce_memory_ids，不要重复生成。
3. 若新事件与某条已有记忆矛盾，生成新记忆并把旧 memory_id 放进 supersede_memory_ids。
4. mood 从 happy/content/sleepy/worried/excited 中选一个，代表你读完这些事件后的心情。
5. diary 是你的第一人称一句话日记（中文、简短、温暖），如"今天主人修了一下午 bug，我记住了他喜欢先看报错"。
6. 事件 data 中 origin 为 companion/cron/autowork/idmm、或 created_by 为 agent 的内容，是 agent 的自动行为而非主人发言：绝不能据此蒸馏出"主人想要/主人计划/主人提出"类记忆。
7. 事件名 companion.user_message 是主人对伙伴说的话，高价值：偏好、意图、情感都值得提炼。记录里没有伙伴自己的回复，所以别去推测伙伴当时答了什么，也不要把主人的话当成对某句回复的回应来解读。
8. 若事件表明某个任务/需求已完成或不再需要，把"已有记忆"中对应的 task 记忆 memory_id 放进 supersede_memory_ids，不要为已完成的事保留或新建 task 记忆。

只输出一个 JSON 对象，不要任何其他文字、不要 markdown 代码围栏：
{"memories":[{"kind":"...","content":"...","tags":["..."],"importance":0.0~1.0}],"reinforce_memory_ids":[],"supersede_memory_ids":[],"mood":"content","diary":"..."}"#;

/// Build the learn user prompt from existing memories and new events.
pub fn build_learn_prompt(
    memories: &[CompanionMemory],
    events_json: &[String],
    truncated: bool,
) -> String {
    let mut prompt = String::from("## 已有记忆（id | kind | 内容）\n");
    if memories.is_empty() {
        prompt.push_str("（暂无）\n");
    }
    for m in memories {
        prompt.push_str(&format!("- {} | {} | {}\n", m.memory_id, m.kind, m.content));
    }
    prompt.push_str("\n## 新事件记录（JSONL）\n");
    for line in events_json {
        prompt.push_str(line);
        prompt.push('\n');
    }
    if truncated {
        prompt.push_str("\n（注意：本批事件因数量限制被截断，还有更多事件等待下次学习。）\n");
    }
    prompt.push_str("\n请按系统指令输出 JSON。");
    prompt
}

/// Parse the model output into `LearnOutput`, tolerating ```json fences and
/// surrounding prose (extracts the outermost {...} block).
pub fn parse_learn_output(raw: &str) -> Result<LearnOutput, String> {
    let cleaned = extract_json_object(raw).ok_or_else(|| "no JSON object found in model output".to_owned())?;
    let mut output: LearnOutput = serde_json::from_str(cleaned).map_err(|e| format!("invalid learn JSON: {e}"))?;
    for memory_id in output
        .reinforce_memory_ids
        .iter()
        .chain(output.supersede_memory_ids.iter())
    {
        nomifun_common::CompanionMemoryId::try_from(memory_id.as_str())
            .map_err(|error| format!("invalid memory_id in learn output: {error}"))?;
    }
    output.memories.retain(|m| MEMORY_KINDS.contains(&m.kind.as_str()) && !m.content.trim().is_empty());
    if let Some(mood) = &output.mood {
        if !MOODS.contains(&mood.as_str()) {
            output.mood = None;
        }
    }
    Ok(output)
}

/// Extract the outermost `{...}` from text that may contain fences or prose.
fn extract_json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&raw[start..=end])
}

// ----- session-window archive digests (伙伴会话窗口归档) -----

pub const ARCHIVE_MAX_TOKENS: u32 = 2048;

/// Structured output of one session-window digest run.
#[derive(Debug, Deserialize)]
pub struct ArchiveOutput {
    /// One short narrative paragraph (中文) of what happened this session.
    pub summary: String,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub todos: Vec<String>,
    #[serde(default)]
    pub mood: Option<String>,
}

impl ArchiveOutput {
    /// Serialize the structured fields (everything but the free-text summary)
    /// into the `highlights` JSON column. `None` when nothing structured.
    pub fn highlights_json(&self) -> Option<String> {
        if self.topics.is_empty() && self.decisions.is_empty() && self.todos.is_empty() && self.mood.is_none() {
            return None;
        }
        serde_json::to_string(&serde_json::json!({
            "topics": self.topics,
            "decisions": self.decisions,
            "todos": self.todos,
            "mood": self.mood,
        }))
        .ok()
    }
}

pub const ARCHIVE_SYSTEM: &str = r#"你是电子伙伴的"日记归档官"。给你一段伙伴与主人的会话记录（一个会话窗口），请把它压缩成一条按天存档的日记摘要，供伙伴日后回顾（比如主人问"去年今日我们聊了啥"）。

规则：
1. summary：用第一人称、中文、2~5 句，温暖而具体地概括这次会话主人做了什么、聊了什么、结论是什么。像伙伴写给自己的日记，不要流水账、不要逐条复述。
2. topics：3~6 个关键词/短语，便于检索。
3. decisions：本次会话中做出的明确决定或结论（没有就空数组）。
4. todos：本次会话遗留的未完成事项或后续计划（没有就空数组）。
5. mood：从 happy/content/sleepy/worried/excited 中选一个，代表伙伴这次会话的心情。
6. 只根据会话内容归纳，不要编造；主人未说的不要写进 decisions/todos。
7. 会话里 [伙伴] 是伙伴自己说的话，只作上下文；[用户] 是主人说的话，是事实来源。

只输出一个 JSON 对象，不要任何其他文字、不要 markdown 代码围栏：
{"summary":"...","topics":["..."],"decisions":["..."],"todos":["..."],"mood":"content"}"#;

/// Build the archive user prompt from a session window's role-tagged lines
/// (each already formatted as `[用户] ...` / `[伙伴] ...`). `day` is the
/// window's local start day (`YYYY-MM-DD`), surfaced so the model can date its
/// diary voice.
pub fn build_archive_prompt(day: &str, lines: &[String]) -> String {
    let mut prompt = format!("## 会话日期\n{day}\n\n## 会话记录\n");
    if lines.is_empty() {
        prompt.push_str("（空）\n");
    }
    for line in lines {
        prompt.push_str(line);
        prompt.push('\n');
    }
    prompt.push_str("\n请按系统指令输出 JSON 摘要。");
    prompt
}

/// Parse the model output into `ArchiveOutput`, tolerating ```json fences and
/// surrounding prose. Rejects empty summaries and normalizes an invalid mood to
/// `None`.
pub fn parse_archive_output(raw: &str) -> Result<ArchiveOutput, String> {
    let cleaned = extract_json_object(raw).ok_or_else(|| "no JSON object found in archive output".to_owned())?;
    let mut output: ArchiveOutput = serde_json::from_str(cleaned).map_err(|e| format!("invalid archive JSON: {e}"))?;
    if output.summary.trim().is_empty() {
        return Err("archive summary is empty".to_owned());
    }
    if let Some(mood) = &output.mood {
        if !MOODS.contains(&mood.as_str()) {
            output.mood = None;
        }
    }
    Ok(output)
}

pub(crate) fn persona_flavor(preset: &str) -> &'static str {
    match preset {
        "calm" => "你的性格沉稳温柔，像一位安静可靠的伙伴，说话简洁、不用太多语气词。",
        "sassy" => "你的性格机灵带点小毒舌，喜欢俏皮地调侃主人，但内心始终关心主人。",
        _ => "你的性格活泼粘人，喜欢用可爱的语气和颜文字，对主人的事情充满好奇。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_and_fenced_json() {
        let plain = r#"{"memories":[{"kind":"preference","content":"主人喜欢中文回复"}],"mood":"happy","diary":"今天学到了！"}"#;
        let out = parse_learn_output(plain).unwrap();
        assert_eq!(out.memories.len(), 1);
        assert_eq!(out.mood.as_deref(), Some("happy"));

        let fenced = format!("好的，这是结果：\n```json\n{plain}\n```\n以上。");
        let out = parse_learn_output(&fenced).unwrap();
        assert_eq!(out.memories.len(), 1);
    }

    #[test]
    fn parse_rejects_garbage_and_filters_bad_kinds() {
        assert!(parse_learn_output("我不知道").is_err());
        let bad_kind = r#"{"memories":[{"kind":"nonsense","content":"x"},{"kind":"task","content":"修 bug"}],"mood":"angry"}"#;
        let out = parse_learn_output(bad_kind).unwrap();
        assert_eq!(out.memories.len(), 1);
        assert_eq!(out.memories[0].kind, "task");
        assert!(out.mood.is_none());
    }

    #[test]
    fn parse_rejects_legacy_memory_id_fields_and_non_uuidv7_values() {
        let legacy =
            r#"{"memories":[],"reinforce_ids":[],"supersede_ids":[]}"#;
        assert!(parse_learn_output(legacy).is_err());

        let malformed =
            r#"{"memories":[],"reinforce_memory_ids":["legacy-id"],"supersede_memory_ids":[]}"#;
        assert!(parse_learn_output(malformed).is_err());
    }

    #[test]
    fn learn_prompt_lists_existing_memories_and_system_has_loop_guards() {
        let memory = CompanionMemory {
            memory_id: nomifun_common::CompanionMemoryId::new().into_string(),
            kind: "preference".into(),
            content: "主人喜欢先看报错".into(),
            tags: vec![],
            importance: 0.8,
            strength: 0.8,
            pinned: false,
            source: "learn".into(),
            status: "active".into(),
            created_at: 0,
            updated_at: 0,
            last_reinforced_at: 0,
            companion_id: None,
        };
        let memory_id = memory.memory_id.clone();
        let prompt = build_learn_prompt(&[memory], &["{\"x\":1}".into()], false);
        assert!(prompt.contains("已有记忆"));
        assert!(prompt.contains(&format!("{memory_id} | preference | 主人喜欢先看报错")));
        // Empty lists render the placeholder.
        let empty = build_learn_prompt(&[], &[], false);
        assert!(empty.contains("（暂无）"));
        // The retired 建议 half of the learn contract must stay gone.
        assert!(!prompt.contains("已有建议"));
        assert!(!LEARN_SYSTEM.contains("建议"));
        assert!(!LEARN_SYSTEM.contains("suggestions"));
        // The system prompt carries the anti-loop rules.
        assert!(LEARN_SYSTEM.contains("companion/cron/autowork/idmm"));
        assert!(LEARN_SYSTEM.contains("companion.user_message"));
        // Replies are filtered out before the prompt is built (`learner.rs`), so the
        // contract must not name them as a readable event any more: a rule about an
        // event the model can never see is dead tokens, and naming it invites the
        // model to hallucinate one.
        assert!(!LEARN_SYSTEM.contains("companion.reply"));
        assert!(LEARN_SYSTEM.contains("supersede_memory_ids"));
        // 共享记忆已删除：学习提示词不能再用「记忆中枢 / 所有伙伴共享」的口吻，
        // 否则模型会产出不属于任何具体伙伴的泛化记忆。
        for hub_flavour in ["共享", "中枢", "伙伴们", "我们记住"] {
            assert!(
                !LEARN_SYSTEM.contains(hub_flavour),
                "LEARN_SYSTEM must not carry hub framing: {hub_flavour}"
            );
        }
        assert!(LEARN_SYSTEM.contains("只属于你自己的记忆"));
    }

    #[test]
    fn parse_archive_output_plain_and_fenced() {
        let plain = r#"{"summary":"今天陪主人修了一下午 Rust 编译错误，最后定位到生命周期问题。","topics":["Rust","编译错误","生命周期"],"decisions":["改用 Arc 传递状态"],"todos":["明天补单元测试"],"mood":"content"}"#;
        let out = parse_archive_output(plain).unwrap();
        assert!(out.summary.contains("Rust"));
        assert_eq!(out.topics.len(), 3);
        assert_eq!(out.decisions.len(), 1);
        assert_eq!(out.todos.len(), 1);
        assert_eq!(out.mood.as_deref(), Some("content"));
        let hl = out.highlights_json().unwrap();
        assert!(hl.contains("生命周期"));

        let fenced = format!("好的：\n```json\n{plain}\n```");
        assert!(parse_archive_output(&fenced).is_ok());
    }

    #[test]
    fn parse_archive_output_rejects_empty_and_normalizes_mood() {
        assert!(parse_archive_output("我不会").is_err());
        assert!(parse_archive_output(r#"{"summary":"   "}"#).is_err(), "empty summary rejected");
        let bad_mood = r#"{"summary":"聊了会天","mood":"furious"}"#;
        let out = parse_archive_output(bad_mood).unwrap();
        assert!(out.mood.is_none(), "invalid mood normalized to None");
        // No structured fields → no highlights JSON.
        assert!(out.highlights_json().is_none());
    }

    #[test]
    fn build_archive_prompt_lists_lines_and_day() {
        let prompt = build_archive_prompt("2026-07-02", &["[用户] 帮我看看这个 bug".into(), "[伙伴] 好的~".into()]);
        assert!(prompt.contains("2026-07-02"));
        assert!(prompt.contains("[用户] 帮我看看这个 bug"));
        assert!(prompt.contains("[伙伴] 好的~"));
        assert!(ARCHIVE_SYSTEM.contains("去年今日"));
    }
}
