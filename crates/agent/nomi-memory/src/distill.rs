//! Post-session memory distillation: hand a serializable transcript of a
//! finished work session to an LLM and turn its high-signal output into
//! file-based memory entries.
//!
//! This module holds only **pure, synchronous functions**: build the prompt,
//! parse the model JSON, write entries to disk, and parse citation filenames.
//! The LLM call itself and the origin/companion gating live in
//! `nomifun-ai-agent` (it owns the provider and the tokio runtime). Keeping
//! the file logic here makes it unit-testable without a live backend.

use std::path::Path;

use serde::Deserialize;

use crate::error::Result;
use crate::index::{append_index_entry, read_index};
use crate::paths::{ensure_memory_dir, memory_entrypoint};
use crate::store::write_memory;
use crate::types::{MemoryEntry, MemoryType};

/// One batch of distillation output (strict JSON). A no-op session yields an
/// empty `memories` array.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DistillOutput {
    #[serde(default)]
    pub memories: Vec<DistilledMemory>,
}

/// A single distilled memory candidate. `r#type` is validated against the
/// four memory types on apply; unrecognized values cause the entry to be
/// dropped.
#[derive(Debug, Clone, Deserialize)]
pub struct DistilledMemory {
    /// One of `user|feedback|project|reference` (invalid → entry dropped).
    pub r#type: String,
    /// Short name used to derive the filename.
    pub name: String,
    /// One-line hook for the MEMORY.md index entry.
    pub description: String,
    /// The memory body.
    pub content: String,
}

/// System prompt for distillation. Carries codex `stage_one_system`'s
/// high-signal + no-op-gate spirit, but its output contract directly produces
/// the four file-based memory types (no raw/summary intermediate stage), and
/// it forbids storing secrets (redaction is the second gate, applied by the
/// caller before write).
pub const DISTILL_SYSTEM: &str = r#"你是 nomi 的记忆蒸馏器。读完一段已结束的工作会话转写，提炼出对"未来会话"有持久价值的记忆。

只保留高信号记忆（满足才写，否则宁缺毋滥）：
- 稳定的用户偏好/操作习惯（用户反复要求或纠正的）
- 高杠杆的过程知识/失败护盾（symptom→cause→fix、关键路径/命令）
- 项目背景（代码/git 推断不出来的「为什么」）
- 外部系统指针（bug 在哪个看板、文档在哪）

绝不写：
- 能从当前代码/git 读出来的（结构、约定、文件路径、谁改了什么）
- 一次性任务细节、临时状态、当前会话上下文
- 已在 AGENTS.md 记录的东西
- 任何密钥/令牌/密码（即使会话里出现也不要复述）

No-op 闸门：先问「未来 agent 会因为这条记忆而表现更好吗？」若否，该条不写。
若整段会话无可留之物，返回 {"memories":[]}。

只输出一个 JSON 对象，无任何额外文字：
{"memories":[{"type":"user|feedback|project|reference","name":"短名","description":"一句话索引钩子","content":"正文"}]}"#;

/// Wrap a rendered transcript into the user-side prompt. `transcript` is
/// produced by the session engine from already-redacted messages.
pub fn build_distill_prompt(transcript: &str) -> String {
    format!(
        "以下是一段已结束的工作会话转写。请蒸馏记忆。\n\n<transcript>\n{transcript}\n</transcript>"
    )
}

/// Parse the model output (tolerant of ```json fences and surrounding prose).
pub fn parse_distill_output(raw: &str) -> std::result::Result<DistillOutput, String> {
    let slice = extract_json_object(raw).ok_or_else(|| "no JSON object found".to_string())?;
    serde_json::from_str::<DistillOutput>(slice).map_err(|e| e.to_string())
}

/// Write the distilled entries to disk: one memory file each, plus an index
/// line in MEMORY.md. Returns the number of entries written.
///
/// The caller guarantees `content` / `description` are already redacted.
/// Entries with an unknown type or an empty body are skipped, as are entries
/// whose description already appears in the index (lightweight dedup).
pub fn apply_distilled(dir: &Path, out: &DistillOutput) -> Result<usize> {
    if out.memories.is_empty() {
        return Ok(0);
    }
    ensure_memory_dir(dir)?;
    let entrypoint = memory_entrypoint(dir);
    // Read the index once; track descriptions added in this batch so two
    // candidates with the same description don't both get written.
    let mut index_snapshot = read_index(&entrypoint);

    let mut written = 0usize;
    for m in &out.memories {
        let Some(ty) = MemoryType::parse(&m.r#type) else {
            continue;
        };
        if m.content.trim().is_empty() {
            continue;
        }
        let desc = m.description.trim();
        if !desc.is_empty() && index_has_description(&index_snapshot, desc) {
            continue; // dedup: this hook is already in the index
        }

        let entry = MemoryEntry::build(&m.name, &m.description, ty, &m.content);
        let path = write_memory(dir, &entry)?;
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = if m.name.trim().is_empty() {
            filename.clone()
        } else {
            m.name.trim().to_owned()
        };
        append_index_entry(&entrypoint, &title, &filename, &m.description)?;
        // Keep the in-memory snapshot current so later candidates in this
        // same batch dedup against just-written entries too. Mirror the index
        // line format (`… \u{2014} <desc>`) so `index_has_description` matches.
        index_snapshot.push('\n');
        index_snapshot.push_str(&format!("- [{title}]({filename}) \u{2014} {}", m.description));
        written += 1;
    }
    Ok(written)
}

/// Parse the filenames cited inside a `<nomi-mem-citation>` block in the
/// assistant's final text. Each non-empty line inside the block contributes
/// the token before its first `|` (or the whole trimmed line if there is no
/// `|`). Returns the cited filenames in order, de-duplicated.
///
/// A missing block, an empty block, or stray text yields an empty vec.
pub fn parse_citation_filenames(text: &str) -> Vec<String> {
    const OPEN: &str = "<nomi-mem-citation>";
    const CLOSE: &str = "</nomi-mem-citation>";

    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Support more than one block, just in case.
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after_open = &rest[start + OPEN.len()..];
        let Some(end) = after_open.find(CLOSE) else {
            break;
        };
        let block = &after_open[..end];
        for line in block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let token = line.split('|').next().unwrap_or(line).trim();
            if token.is_empty() {
                continue;
            }
            if seen.insert(token.to_owned()) {
                out.push(token.to_owned());
            }
        }
        rest = &after_open[end + CLOSE.len()..];
    }
    out
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract the first balanced top-level `{...}` JSON object from `raw`,
/// tolerating ```json fences and surrounding prose. String-literal aware so
/// braces inside JSON strings don't throw off the balance count.
fn extract_json_object(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let start = raw.find('{')?;

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;

    for i in start..bytes.len() {
        let c = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether the index text already contains an entry with the given
/// description hook. Matches the `\u{2014} <desc>` tail that
/// `append_index_entry` writes, so substring collisions on shorter
/// descriptions are avoided.
fn index_has_description(index: &str, description: &str) -> bool {
    let desc = description.trim();
    if desc.is_empty() {
        return false;
    }
    let needle = format!("\u{2014} {desc}");
    index.lines().any(|line| line.trim_end().ends_with(&needle))
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ENTRYPOINT_NAME;

    // -- parse_distill_output ------------------------------------------------

    #[test]
    fn parse_plain_json() {
        let raw = r#"{"memories":[{"type":"user","name":"role","description":"d","content":"c"}]}"#;
        let out = parse_distill_output(raw).unwrap();
        assert_eq!(out.memories.len(), 1);
        assert_eq!(out.memories[0].r#type, "user");
        assert_eq!(out.memories[0].name, "role");
    }

    #[test]
    fn parse_json_fenced() {
        let raw = "```json\n{\"memories\":[{\"type\":\"feedback\",\"name\":\"n\",\"description\":\"d\",\"content\":\"c\"}]}\n```";
        let out = parse_distill_output(raw).unwrap();
        assert_eq!(out.memories.len(), 1);
        assert_eq!(out.memories[0].r#type, "feedback");
    }

    #[test]
    fn parse_with_surrounding_prose() {
        let raw = "Here is the distilled output:\n{\"memories\":[]}\nThat's all.";
        let out = parse_distill_output(raw).unwrap();
        assert!(out.memories.is_empty());
    }

    #[test]
    fn parse_empty_memories_is_noop() {
        let out = parse_distill_output(r#"{"memories":[]}"#).unwrap();
        assert!(out.memories.is_empty());
    }

    #[test]
    fn parse_missing_memories_defaults_empty() {
        let out = parse_distill_output(r#"{}"#).unwrap();
        assert!(out.memories.is_empty());
    }

    #[test]
    fn parse_braces_inside_string_dont_confuse_extractor() {
        let raw = r#"{"memories":[{"type":"project","name":"n","description":"uses {braces}","content":"a } b { c"}]}"#;
        let out = parse_distill_output(raw).unwrap();
        assert_eq!(out.memories.len(), 1);
        assert_eq!(out.memories[0].content, "a } b { c");
    }

    #[test]
    fn parse_no_json_errors() {
        let err = parse_distill_output("no json here").unwrap_err();
        assert!(err.contains("no JSON object"));
    }

    #[test]
    fn parse_malformed_json_errors() {
        let err = parse_distill_output(r#"{"memories": [ broken"#).unwrap_err();
        assert!(!err.is_empty());
    }

    // -- build_distill_prompt ------------------------------------------------

    #[test]
    fn build_prompt_wraps_transcript() {
        let p = build_distill_prompt("[user] hi\n[assistant] hello");
        assert!(p.contains("<transcript>"));
        assert!(p.contains("</transcript>"));
        assert!(p.contains("[user] hi"));
    }

    // -- apply_distilled -----------------------------------------------------

    #[test]
    fn apply_writes_files_and_index() {
        let tmp = tempfile::tempdir().unwrap();
        let out = DistillOutput {
            memories: vec![
                DistilledMemory {
                    r#type: "user".into(),
                    name: "role".into(),
                    description: "senior Go engineer".into(),
                    content: "User has deep Go expertise.".into(),
                },
                DistilledMemory {
                    r#type: "feedback".into(),
                    name: "testing".into(),
                    description: "integration tests hit a real DB".into(),
                    content: "Do not mock the database.".into(),
                },
            ],
        };
        let written = apply_distilled(tmp.path(), &out).unwrap();
        assert_eq!(written, 2);
        assert!(tmp.path().join("user_role.md").exists());
        assert!(tmp.path().join("feedback_testing.md").exists());

        let index = std::fs::read_to_string(tmp.path().join(ENTRYPOINT_NAME)).unwrap();
        assert!(index.contains("user_role.md"));
        assert!(index.contains("senior Go engineer"));
        assert!(index.contains("feedback_testing.md"));
    }

    #[test]
    fn apply_skips_invalid_type() {
        let tmp = tempfile::tempdir().unwrap();
        let out = DistillOutput {
            memories: vec![DistilledMemory {
                r#type: "nonsense".into(),
                name: "x".into(),
                description: "d".into(),
                content: "c".into(),
            }],
        };
        let written = apply_distilled(tmp.path(), &out).unwrap();
        assert_eq!(written, 0);
    }

    #[test]
    fn apply_skips_empty_content() {
        let tmp = tempfile::tempdir().unwrap();
        let out = DistillOutput {
            memories: vec![DistilledMemory {
                r#type: "project".into(),
                name: "x".into(),
                description: "d".into(),
                content: "   ".into(),
            }],
        };
        let written = apply_distilled(tmp.path(), &out).unwrap();
        assert_eq!(written, 0);
    }

    #[test]
    fn apply_dedup_skips_existing_description() {
        let tmp = tempfile::tempdir().unwrap();
        // Pre-seed the index with a matching description hook.
        ensure_memory_dir(tmp.path()).unwrap();
        append_index_entry(
            &memory_entrypoint(tmp.path()),
            "Role",
            "user_role.md",
            "senior Go engineer",
        )
        .unwrap();

        let out = DistillOutput {
            memories: vec![DistilledMemory {
                r#type: "user".into(),
                name: "role2".into(),
                description: "senior Go engineer".into(),
                content: "dup".into(),
            }],
        };
        let written = apply_distilled(tmp.path(), &out).unwrap();
        assert_eq!(written, 0, "duplicate description should be skipped");
    }

    #[test]
    fn apply_dedup_within_same_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let out = DistillOutput {
            memories: vec![
                DistilledMemory {
                    r#type: "user".into(),
                    name: "a".into(),
                    description: "same hook".into(),
                    content: "first".into(),
                },
                DistilledMemory {
                    r#type: "user".into(),
                    name: "b".into(),
                    description: "same hook".into(),
                    content: "second".into(),
                },
            ],
        };
        let written = apply_distilled(tmp.path(), &out).unwrap();
        assert_eq!(written, 1, "second entry with same hook deduped in-batch");
    }

    #[test]
    fn apply_empty_output_is_noop_no_dir_created() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("memory");
        let written = apply_distilled(&target, &DistillOutput::default()).unwrap();
        assert_eq!(written, 0);
        assert!(!target.exists(), "no-op must not create the memory dir");
    }

    // -- parse_citation_filenames --------------------------------------------

    #[test]
    fn citation_parses_multiple_lines() {
        let text = "Here is my answer.\n\n<nomi-mem-citation>\nuser_role.md|note=[adjusted for Go expertise]\nfeedback_testing.md|note=[no DB mocks]\n</nomi-mem-citation>";
        let files = parse_citation_filenames(text);
        assert_eq!(files, vec!["user_role.md", "feedback_testing.md"]);
    }

    #[test]
    fn citation_empty_block_yields_nothing() {
        let text = "answer\n<nomi-mem-citation>\n\n</nomi-mem-citation>";
        assert!(parse_citation_filenames(text).is_empty());
    }

    #[test]
    fn citation_no_block_yields_nothing() {
        assert!(parse_citation_filenames("just a plain answer").is_empty());
    }

    #[test]
    fn citation_line_without_pipe_uses_whole_token() {
        let text = "<nomi-mem-citation>\nproject_status.md\n</nomi-mem-citation>";
        assert_eq!(parse_citation_filenames(text), vec!["project_status.md"]);
    }

    #[test]
    fn citation_dedups_repeated_filenames() {
        let text = "<nomi-mem-citation>\nuser_role.md|note=[a]\nuser_role.md|note=[b]\n</nomi-mem-citation>";
        assert_eq!(parse_citation_filenames(text), vec!["user_role.md"]);
    }

    #[test]
    fn citation_unterminated_block_yields_nothing() {
        let text = "<nomi-mem-citation>\nuser_role.md|note=[x]\n(no close tag)";
        assert!(parse_citation_filenames(text).is_empty());
    }

    // -- index_has_description -----------------------------------------------

    #[test]
    fn index_has_description_matches_tail_only() {
        let index = "- [Role](user_role.md) \u{2014} senior Go engineer\n";
        assert!(index_has_description(index, "senior Go engineer"));
        // A shorter substring of the hook must not match (tail anchored).
        assert!(!index_has_description(index, "senior Go"));
        assert!(!index_has_description(index, "absent hook"));
    }

    #[test]
    fn index_has_description_empty_is_false() {
        assert!(!index_has_description("- [A](a.md) \u{2014} x\n", "   "));
    }

    // -- extract_json_object -------------------------------------------------

    #[test]
    fn extract_first_balanced_object() {
        assert_eq!(extract_json_object("x {\"a\":1} y"), Some("{\"a\":1}"));
        assert_eq!(extract_json_object("{\"a\":{\"b\":2}}"), Some("{\"a\":{\"b\":2}}"));
        assert_eq!(extract_json_object("no braces"), None);
    }
}
