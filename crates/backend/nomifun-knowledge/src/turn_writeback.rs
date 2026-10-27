//! Turn-final knowledge write-back extraction.
//!
//! This module is pure prompt/parse logic. The service layer owns provider calls,
//! write policy, and disk writes.

use nomifun_api_types::KnowledgeMountInfo;
use nomifun_common::KnowledgeBaseId;
use serde::Deserialize;

use crate::context::WritebackEagerness;

const TOC_LINES_PER_BASE_MAX: usize = 24;
const MOUNT_CONTEXT_MAX_BYTES: usize = 16 * 1024;
const MOUNT_CONTEXT_PER_BASE_MAX_BYTES: usize = 8 * 1024;
const USER_TEXT_MAX_BYTES: usize = 20 * 1024;
const TRUNCATION_MARKER: &str = "\n[truncated]\n";

/// Strict-JSON contract for extracting durable knowledge from one completed turn.
///
/// The new-material-only rule is load-bearing: the service merges a candidate
/// append-only, so a candidate that restates an existing document would be
/// appended to that document instead of replacing it.
pub const TURN_WRITEBACK_SYSTEM: &str = "You are a knowledge-base curator for NomiFun. \
Read one completed conversation turn and decide whether anything should be proposed \
for knowledge-base write-back. Output ONLY a JSON object of this exact shape:\n\
{\"candidates\":[{\"kb_id\":\"...\",\"rel_path\":\"path/inside/base.md\",\"content\":\"markdown body\"}]}\n\
Rules:\n\
- Only write reusable knowledge that will help future sessions using one of the mounted bases.\n\
- Do not write one-off task status, transient debugging notes, chat narration, or facts already obvious from the current code/git state.\n\
- Do not write secrets, tokens, credentials, private identifiers, or sensitive personal data.\n\
- Ground every candidate in the provided turn. Never invent facts.\n\
- Pick exactly one mounted kb_id for each candidate.\n\
- Return at most 8 candidates.\n\
- rel_path must be a relative markdown path inside that base, never absolute.\n\
- content must be concise markdown, organized as a durable note.\n\
- content must be ONLY the new material to record, never a rewrite of an existing document.\n\
- If nothing qualifies, return {\"candidates\":[]}.";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnWritebackCandidate {
    pub kb_id: KnowledgeBaseId,
    pub rel_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnWritebackOutput {
    pub candidates: Vec<TurnWritebackCandidate>,
}

pub fn build_turn_writeback_prompt(
    mounts: &[KnowledgeMountInfo],
    eagerness: WritebackEagerness,
    user_text: &str,
    assistant_text: &str,
    limits: crate::autogen::KnowledgeWritebackLimits,
) -> String {
    let eagerness_label = match eagerness {
        WritebackEagerness::Manual => "manual",
        WritebackEagerness::Auto => "auto",
    };
    // Manual reaches this extractor only after the runtime has identified
    // explicit save/record intent, and the prompt repeats that gate as defense
    // in depth.
    let eagerness_rule = match eagerness {
        WritebackEagerness::Manual => {
            "Manual: the owner drives write-back. Return an EMPTY candidates array unless the user in THIS turn explicitly asked to record or save something; then extract only that."
        }
        WritebackEagerness::Auto => {
            "Auto: you decide, and the bar is high. Extract only durable, reusable, clearly-correct knowledge. Prefer an empty array over a marginal candidate, and never extract trivia, transient state, or anything the known_paths listing shows is already recorded."
        }
    };

    let mut prompt = String::new();
    prompt.push_str("Write-back extraction settings:\n");
    prompt.push_str(&format!("- eagerness: {eagerness_label}\n"));
    prompt.push_str(&format!("- rule: {eagerness_rule}\n\n"));
    prompt.push_str("Mounted knowledge bases:\n");
    let mount_start = prompt.len();
    let input_budget = limits.input_max_utf8_bytes.min(limits.input_max_tokens_estimate.saturating_mul(3));
    let available = input_budget.saturating_sub(TURN_WRITEBACK_SYSTEM.len() + prompt.len() + 128);
    // Complete identifiers always survive. Optional descriptions and TOCs
    // share only the remaining metadata allowance, so a verbose first base
    // cannot hide another selected base's identity.
    let headers = mounts.iter().map(|mount| format!("- kb_id: {}\n  name: {}\n", mount.knowledge_base_id, truncate_text(&mount.name, 128)))
        .collect::<Vec<_>>();
    let mut remaining_headers = headers.iter().map(String::len).sum::<usize>();
    let mount_budget = (available / 5).min(MOUNT_CONTEXT_MAX_BYTES).max(remaining_headers);
    for (index, (m, header)) in mounts.iter().zip(headers).enumerate() {
        prompt.push_str(&header);
        remaining_headers = remaining_headers.saturating_sub(header.len());
        let allowance = mount_budget.saturating_sub(prompt.len() - mount_start + remaining_headers)
            .checked_div(mounts.len() - index).unwrap_or(0).min(MOUNT_CONTEXT_PER_BASE_MAX_BYTES);
        let mut context = String::new();
        if !m.description.trim().is_empty() {
            context.push_str(&format!("  description: {}\n", one_line(&m.description)));
        }
        if let Some(summary) = m.summary.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            context.push_str(&format!("  summary: {}\n", one_line(summary)));
        }
        context.push_str(&format!("  mount_path: {}\n", one_line(&m.rel_path)));
        if !m.toc.is_empty() {
            context.push_str("  known_paths:\n");
            for line in m.toc.iter().take(TOC_LINES_PER_BASE_MAX) {
                let line = format!("    - {}\n", one_line(line));
                if context.len() + line.len() > MOUNT_CONTEXT_PER_BASE_MAX_BYTES {
                    context.push_str("    [Additional known paths omitted by the extraction input budget]\n");
                    break;
                }
                context.push_str(&line);
            }
        }
        let context = truncate_text(&context, allowance);
        prompt.push_str(&context);
        if !context.ends_with('\n') { prompt.push('\n'); }
    }
    prompt.push_str("\nCompleted turn:\n<user>\n");
    let user_budget = (available / 5).min(USER_TEXT_MAX_BYTES);
    prompt.push_str(&truncate_text(user_text, user_budget));
    prompt.push_str("\n</user>\n<assistant>\n");
    let assistant_budget = input_budget.saturating_sub(TURN_WRITEBACK_SYSTEM.len() + prompt.len() + "\n</assistant>\n".len());
    prompt.push_str(&truncate_text(assistant_text, assistant_budget));
    prompt.push_str("\n</assistant>\n");
    prompt
}

pub fn parse_turn_writeback_output(raw: &str) -> Result<TurnWritebackOutput, String> {
    let slice = extract_json_object(raw).ok_or_else(|| "no JSON object found".to_owned())?;
    serde_json::from_str::<TurnWritebackOutput>(slice).map_err(|error| {
        // Serde's Display may echo an invalid field/value from the response.
        // Keep only locally authored structure diagnostics.
        format!("invalid turn writeback JSON: {:?} at line {}, column {}", error.classify(), error.line(), error.column())
    })
}

fn one_line(s: &str) -> String {
    truncate_text(s, 768).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_text(s: &str, max_bytes: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max_bytes {
        return trimmed.to_owned();
    }
    let mut out = String::new();
    let limit = max_bytes.saturating_sub(TRUNCATION_MARKER.len());
    let head_limit = limit / 2;
    for ch in trimmed.chars() {
        if out.len() + ch.len_utf8() > head_limit { break; }
        out.push(ch);
    }
    if max_bytes >= TRUNCATION_MARKER.len() { out.push_str(TRUNCATION_MARKER); }
    let tail_limit = max_bytes.saturating_sub(out.len());
    let mut tail_start = trimmed.len();
    for (start, _) in trimmed.char_indices().rev() {
        if trimmed.len() - start > tail_limit { break; }
        tail_start = start;
    }
    out.push_str(&trimmed[tail_start..]);
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tolerates_fences_and_surrounding_text() {
        let kb_id = KnowledgeBaseId::new();
        let raw = format!(
            "```json\n{{\"candidates\":[{{\"kb_id\":\"{kb_id}\",\"rel_path\":\"a.md\",\"content\":\"# A\"}}]}}\n```"
        );
        let out = parse_turn_writeback_output(&raw).unwrap();
        assert_eq!(out.candidates.len(), 1);
        assert_eq!(out.candidates[0].kb_id, kb_id);
        assert_eq!(out.candidates[0].rel_path, "a.md");
    }

    #[test]
    fn parse_rejects_noncanonical_knowledge_base_id() {
        let raw = r##"{"candidates":[{"kb_id":"kb_1","rel_path":"a.md","content":"# A"}]}"##;
        assert!(parse_turn_writeback_output(raw).is_err());
    }

    #[test]
    fn only_an_explicit_empty_candidates_array_means_no_candidate() {
        assert!(parse_turn_writeback_output(r#"{"candidates":[]}"#).unwrap().candidates.is_empty());
        let kb_id = KnowledgeBaseId::new();
        for raw in [
            "{}".to_owned(),
            r#"{"candidates":null}"#.to_owned(),
            r#"{"candidates":[],"result":"ignored"}"#.to_owned(),
            format!(r#"{{"candidates":[{{"kb_id":"{kb_id}","content":"body"}}]}}"#),
            format!(r#"{{"candidates":[{{"kb_id":"{kb_id}","rel_path":"a.md"}}]}}"#),
        ] {
            assert!(parse_turn_writeback_output(&raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn extraction_budget_preserves_a_normal_cjk_answer_and_bounds_large_mounts() {
        let mount = KnowledgeMountInfo {
            knowledge_base_id: KnowledgeBaseId::new(), name: "Python 基础".into(), description: "可复用教程".into(),
            rel_path: ".nomi/knowledge/python".into(), toc: vec![], summary: None, live_sources: vec![], remote_index_stale: false,
        };
        let answer = "知".repeat(5000);
        let prompt = build_turn_writeback_prompt(std::slice::from_ref(&mount), WritebackEagerness::Auto, "解释 Python 正则", &answer, Default::default());
        assert!(prompt.contains(&answer));
        assert!(!prompt.contains(TRUNCATION_MARKER));
        let mut large_mount = mount;
        large_mount.summary = Some("🙂".repeat(10_000));
        large_mount.toc = vec!["a".repeat(8192); TOC_LINES_PER_BASE_MAX];
        let prompt = build_turn_writeback_prompt(&vec![large_mount; 10], WritebackEagerness::Auto, &"🙂".repeat(100_000), &"知".repeat(100_000), Default::default());
        assert!(prompt.len() + TURN_WRITEBACK_SYSTEM.len() <= crate::autogen::KnowledgeWritebackLimits::default().input_max_utf8_bytes);
        assert!(prompt.ends_with("\n</assistant>\n"));
        assert!(prompt.contains(TRUNCATION_MARKER));
    }

    #[test]
    fn smaller_context_keeps_every_base_identity_and_answer_end_without_exhausting_output() {
        let first = KnowledgeMountInfo { knowledge_base_id: KnowledgeBaseId::new(), name: "First".into(), description: "d".repeat(5000),
            rel_path: ".nomi/knowledge/first".into(), toc: vec!["long path".repeat(1000); 24], summary: Some("summary".repeat(1000)), live_sources: vec![], remote_index_stale: false };
        let mut second = first.clone(); second.knowledge_base_id = KnowledgeBaseId::new(); second.name = "Second".into();
        let limits = crate::autogen::KnowledgeWritebackLimits::default().for_configuration(&crate::autogen::test_writeback_configuration(Some(8192), None)).unwrap();
        let answer = format!("Opening\n{}\nFinal durable conclusion", "知".repeat(12_000));
        let user = format!("{}\nPlease record the final conclusion", "长".repeat(12_000));
        let prompt = build_turn_writeback_prompt(&[first.clone(), second.clone()], WritebackEagerness::Manual, &user, &answer, limits);
        for base in [&first, &second] { assert!(prompt.contains(base.knowledge_base_id.as_str())); }
        assert!(prompt.len() + TURN_WRITEBACK_SYSTEM.len() <= limits.input_max_utf8_bytes);
        assert!(prompt.contains("Final durable conclusion"));
        assert!(prompt.contains("Please record the final conclusion"));
        assert!(prompt.contains(TRUNCATION_MARKER));
        assert_eq!(limits.output_max_tokens, 2048);
        let normal = "知".repeat(5000);
        let medium = crate::autogen::KnowledgeWritebackLimits::default().for_configuration(&crate::autogen::test_writeback_configuration(Some(16_384), None)).unwrap();
        assert!(build_turn_writeback_prompt(&[first,second], WritebackEagerness::Auto,"Remember the lesson", &normal, medium).contains(&normal));
    }

    /// The disposition has to reach the model, and the two dispositions must
    /// never render the same rule text — a silent collapse would hand Manual
    /// sessions the self-directed instruction.
    #[test]
    fn prompt_labels_disposition_and_keeps_manual_distinct_from_auto() {
        let mount = KnowledgeMountInfo {
            knowledge_base_id: nomifun_common::KnowledgeBaseId::new(),
            name: "Ops".into(),
            description: String::new(),
            rel_path: ".nomi/knowledge/Ops".into(),
            toc: vec!["runbook.md — Runbook".into()],
            summary: None,
            live_sources: Vec::new(),
            remote_index_stale: false,
        };

        let auto = build_turn_writeback_prompt(
            std::slice::from_ref(&mount),
            WritebackEagerness::Auto,
            "u",
            "a",
            Default::default(),
        );
        assert!(auto.contains("eagerness: auto"));
        assert!(auto.contains("runbook.md — Runbook"));
        assert!(!auto.contains("EMPTY candidates array"));

        let manual = build_turn_writeback_prompt(&[mount], WritebackEagerness::Manual, "u", "a", Default::default());
        assert!(manual.contains("eagerness: manual"));
        assert!(manual.contains("EMPTY candidates array"));
    }

}
