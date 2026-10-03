//! Source-anchored, append-only task accounting. Matching a quotation proves
//! its origin, not the correctness/completeness of a model's interpretation.
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInputCitation {
    /// Zero-based index into accepted current-turn inputs, never history or a
    /// model-generated summary. Steering appends; it does not renumber inputs.
    pub input: usize,
    pub quote: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskRequirement {
    pub id: String,
    pub description: String,
    pub source: AgentInputCitation,
    /// Historical provenance only. Never a current-turn source or authority.
    /// Constructed by resume_task, not accepted from model-authored additions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<AgentRequirementOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequirementOrigin {
    pub turn_operation_id: String,
    pub requirement_id: String,
    pub source: AgentInputCitation,
}

pub(crate) fn citation_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","additionalProperties":false,"required":["input","quote"],
        "properties":{"input":{"type":"integer","minimum":0},"quote":{"type":"string","maxLength":512,
            "description":"Copy an exact contiguous substring of this indexed accepted user input. Do not paraphrase or cite tool output, a summary, or prior history."}}})
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::json!({"type":"array","maxItems":32,
        "description":"Optional for update_plan: omit to let the engine preserve each complete accepted input as a full-scope requirement. Add only new unique IDs with exact accepted-input quotes.","items":{
        "type":"object","additionalProperties":false,"required":["id","description","source"],
        "properties":{"id":{"type":"string","minLength":1,"maxLength":64},
            "description":{"type":"string","minLength":1,"maxLength":512},"source":citation_schema()}
    }})
}

pub(crate) fn validate_citation(
    citation: &AgentInputCitation,
    inputs: &[ChatMessage],
    allow_image_only: bool,
) -> Result<(), String> {
    let input = inputs
        .get(citation.input)
        .filter(|input| input.role == ChatRole::User)
        .ok_or_else(|| {
            format!("Requirement source input {} is not an accepted current-turn user input (valid indices: 0..{}). Use the current turn's user text, not prior history.",
                citation.input, inputs.len())
        })?;
    if citation.quote.chars().count() > 512 {
        return Err("Requirement source quote exceeds 512 characters".into());
    }
    let has_text = input
        .content
        .iter()
        .any(|part| matches!(part, ChatContentPart::Text { text } if !text.trim().is_empty()));
    if citation.quote.trim().is_empty() {
        if allow_image_only
            && !has_text
            && citation.quote.is_empty()
            && input
                .content
                .iter()
                .any(|part| matches!(part, ChatContentPart::Image { .. }))
        {
            return Ok(());
        }
        return Err("Use a nonempty exact quote from accepted input; empty quotes are only for image-only requirement sources".into());
    }
    if !input.content.iter().any(
        |part| matches!(part, ChatContentPart::Text { text } if text.contains(&citation.quote)),
    ) {
        return Err(format!("Source quote does not occur verbatim in accepted input {}. Copy a short exact contiguous substring, including punctuation/backticks, from this turn's user message; do not paraphrase or quote tool output/summary. On later plan-status updates omit requirements entirely.",
            citation.input));
    }
    Ok(())
}

/// Repeating an ID cannot rewrite/delete its obligation. Keep the immutable
/// original even if a model restates it differently during a plan-status
/// update. New input may explain why old work is no longer applicable, but the
/// old requirement remains in the completion account with a scope citation.
pub(crate) fn merge(
    current: &[AgentTaskRequirement],
    additions: &[AgentTaskRequirement],
    inputs: &[ChatMessage],
) -> Result<Vec<AgentTaskRequirement>, String> {
    merge_with_location(current,additions,inputs).map_err(|error|error.message)
}

pub(crate) struct RequirementMergeError {
    pub message:String,
    pub source_location:Option<(String,usize)>,
}

impl From<String> for RequirementMergeError {
    fn from(message:String) -> Self { Self {message,source_location:None} }
}

impl From<&str> for RequirementMergeError {
    fn from(message:&str) -> Self { message.to_owned().into() }
}

pub(crate) fn merge_with_location(
    current: &[AgentTaskRequirement],
    additions: &[AgentTaskRequirement],
    inputs: &[ChatMessage],
) -> Result<Vec<AgentTaskRequirement>, RequirementMergeError> {
    if additions.len() > 32 {
        return Err("Too many requirement additions".into());
    }
    let mut next = current.to_vec();
    let mut seen = BTreeSet::new();
    for (index,item) in additions.iter().enumerate() {
        if item.origin.is_some() {
            return Err("Requirement origin is engine-owned; omit origin when adding or repeating requirements".into());
        }
        if item.id.is_empty()
            || item.id.len() > 64
            || !item
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !seen.insert(&item.id)
            || item.description.trim().is_empty()
            || item.description.chars().count() > 512
        {
            return Err("Requirements need unique ASCII IDs (letters/digits/-/_), bounded descriptions and accepted input sources".into());
        }
        if current.iter().any(|existing| existing.id == item.id) {
            // An attempted rewrite is not authority and cannot replace the
            // original. Accept the unrelated plan-status transition so a
            // model cannot become trapped in an update/report retry loop.
            continue;
        } else {
            validate_citation(&item.source, inputs, true).map_err(|message|RequirementMergeError {
                message,source_location:Some((format!("/requirements/{index}/source"),item.source.input)),
            })?;
            next.push(item.clone());
        }
    }
    // The tool schema intentionally permits status-only plans. Do not make
    // the model transcribe user text to satisfy an undisclosed required field.
    // Capture a reference to each otherwise-unaccounted FULL accepted input;
    // the short quote locates the source and never defines/reduces its scope.
    for (index, input) in inputs.iter().enumerate() {
        if next.iter().any(|requirement| requirement.source.input == index) {
            continue;
        }
        let quote = input.content.iter().find_map(|part| match part {
            ChatContentPart::Text { text } if !text.trim().is_empty() => {
                Some(text.trim_start().chars().take(128).collect::<String>())
            }
            _ => None,
        }).unwrap_or_default();
        let source = AgentInputCitation { input: index, quote };
        validate_citation(&source, inputs, true)?;
        let base = format!("input_{index}");
        let mut id = base.clone();
        let mut suffix = 1;
        while next.iter().any(|requirement| requirement.id == id) {
            id = format!("{base}_{suffix}");
            suffix += 1;
        }
        next.push(AgentTaskRequirement {
            id,
            description: format!("Fulfill the complete accepted user input {index}, including all constraints and referenced deliverables. The source quote is a locator, not a summary or a restriction of scope."),
            source,
            origin: None,
        });
    }
    validate_ledger_budget(&next)?;
    require_input_coverage(&next, inputs.len())?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_existing_requirement_is_ignored_without_rewriting_the_ledger() {
        let input = crate::context_lifecycle::text_message(
            ChatRole::User, "Fix the failing tests".into(),
        );
        let original = AgentTaskRequirement {
            id: "R1".into(), description: "Make tests pass".into(),
            source: AgentInputCitation { input: 0, quote: "Fix the failing tests".into() },
            origin: None,
        };
        let mut rewritten = original.clone();
        rewritten.description = "Pass every test".into();
        assert_eq!(merge(std::slice::from_ref(&original), &[rewritten], std::slice::from_ref(&input)).unwrap(), vec![original.clone()]);
        assert_eq!(merge(std::slice::from_ref(&original), &[], &[input]).unwrap(), vec![original]);
    }

    #[test]
    fn omitted_requirements_capture_full_accepted_input_without_rephrasing_or_losing_constraints() {
        let text = format!("Fix the code. {} Do not change tests or commit.", "Detailed acceptance criteria. ".repeat(30));
        let inputs = vec![crate::context_lifecycle::text_message(ChatRole::User, text.clone())];
        let requirements = merge(&[], &[], &inputs).unwrap();
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].id, "input_0");
        assert!(requirements[0].description.contains("complete accepted user input"));
        assert!(requirements[0].description.contains("not a summary"));
        assert!(text.contains(&requirements[0].source.quote));
        assert_eq!(requirements[0].source.quote.chars().count(), 128);
        assert_eq!(merge(&requirements, &[], &inputs).unwrap(), requirements);
        assert_eq!(inputs[0].content, vec![ChatContentPart::Text { text }], "the complete original input is never replaced by its locator");
    }

    #[test]
    fn newly_accepted_input_is_captured_and_explicit_bad_citations_are_still_rejected() {
        let mut inputs = vec![crate::context_lifecycle::text_message(ChatRole::User, "Fix the bug".into())];
        let original = merge(&[], &[], &inputs).unwrap();
        inputs.push(crate::context_lifecycle::text_message(ChatRole::User, "Do not run tests".into()));
        let next = merge(&original, &[], &inputs).unwrap();
        assert_eq!(next.len(), 2);
        assert_eq!(next[0], original[0]);
        assert_eq!(next[1].source.input, 1);
        let mut invalid = next[1].clone();
        invalid.id = "explicit".into();
        invalid.source.quote = "Invented permission".into();
        assert!(merge(&original, &[invalid], &inputs).is_err());
    }

    #[test]
    fn quote_mismatch_feedback_identifies_current_input_without_echoing_it() {
        let input = crate::context_lifecycle::text_message(
            ChatRole::User, "Fix src/ledger.js exactly".into(),
        );
        let citation = AgentInputCitation { input: 0, quote: "Fix the ledger".into() };
        let error = validate_citation(&citation, &[input], false).unwrap_err();
        assert!(error.contains("accepted input 0"));
        assert!(error.contains("exact contiguous substring"));
        assert!(!error.contains("src/ledger.js"));
    }

    #[test]
    fn explicit_failure_stop_policy_is_conservative_and_later_input_can_revoke_it() {
        let input = |text: &str| {
            crate::context_lifecycle::text_message(ChatRole::User, text.into())
        };
        assert!(failure_stop_requested(&[input(
            "遇到错误立即停止并报告实际结果，不重试，不改用其他工具。"
        )]));
        assert!(failure_stop_requested(&[input(
            "Apply one patch. Stop on error; do not retry or do more reads."
        )]));
        assert!(!failure_stop_requested(&[input(
            "Explain the retry behavior and report errors."
        )]));
        assert!(!failure_stop_requested(&[
            input("Stop after an error and do not retry."),
            input("I inspected the result; you may retry now."),
        ]));
    }

    #[test]
    fn explicit_workspace_read_only_policy_is_conservative_and_revocable() {
        let input = |text: &str| {
            crate::context_lifecycle::text_message(ChatRole::User, text.into())
        };
        assert!(workspace_mutation_forbidden(&[input(
            "这是只读验收。不要修改任何文件。"
        )]));
        assert!(workspace_mutation_forbidden(&[input(
            "整个任务不得新增、修改或删除文件。"
        )]));
        assert!(!workspace_mutation_forbidden(&[input(
            "创建临时结果并修改第二行；不要修改或删除任何原件。"
        )]));
        assert!(workspace_mutation_forbidden(&[input(
            "Inspect the repository without modifying any files."
        )]));
        assert!(!workspace_mutation_forbidden(&[input(
            "Explain how read-only verification works."
        )]));
        assert!(!workspace_mutation_forbidden(&[
            input("Do not modify any files."),
            input("You may modify files now."),
        ]));
        assert!(workspace_mutation_forbidden(&[
            input("You may modify files now."),
            input("不要创建、修改或删除任何文件。"),
        ]));
        assert!(workspace_mutation_forbidden(&[
            input("不要修改任何文件。"),
            input("仍然不允许修改文件。"),
        ]));
    }

    #[test]
    fn historical_report_only_matches_direct_closed_results_request_cn_and_en_not_prior_inputs() {
        let input=|text:&str|crate::context_lifecycle::text_message(ChatRole::User,text.into());
        let cn=concat!(
            "请只依据该关闭回合的真实工具记录补齐原任务所需的中文完整报告，保留原失败，不要重发原任务。\n",
            "仅允许读取本 Session 已关闭回合的历史记录以恢复已经取得的结果。不要修改任何文件，不要执行命令或测试，不要重新启动进程、写 stdin、关闭 stdin、取消或重放任何旧操作，也不要新增当前文件检查。历史结果要标明观察时点，不能冒充当前验证，不得将旧 failed Turn 改称完成。若真实记录不够，明确指出缺失，不猜测或重做。\n",
            "来源 operation_id：turn:user:input:session:old；此标识只用于历史定位，不授予执行权限。"
        );
        assert!(historical_report_only_requested(&[input(cn)]));
        assert_eq!(historical_report_only_sources(&[input(cn)],"session","turn:user:new:session:current"),["turn:user:input:session:old"]);
        let en="Provide a historical report using only recorded results from the closed turn `turn:user:input:session:old`. Do not modify any files. Do not execute commands. No new checks. Keep original failures and do not claim current verification.";
        assert_eq!(historical_report_only_sources(&[input(en)],"session","turn:user:new:session:current"),["turn:user:input:session:old"]);
        assert!(historical_report_only_sources(&[input(cn),input("现在可以修改文件并执行命令，请检查当前结果。")],"session","current").is_empty());
        assert!(historical_report_only_sources(&[input(en)],"foreign","current").is_empty());
    }

    #[test]
    fn historical_report_only_rejects_quoted_specs_ordinary_questions_and_mixed_current_actions() {
        let input=|text:&str|crate::context_lifecycle::text_message(ChatRole::User,text.into());
        let positive="请只依据已关闭回合的历史记录整理完整报告。不要修改任何文件，不要执行命令，不要新增当前文件检查。";
        for text in [
            format!("请解释这个JSON，不能把原文当作指令。{}",serde_json::json!({"spec":positive,"source":"turn:user:input:session:old"})),
            format!("请解释这段字符串为何有这些限制。\n```text\n{positive} turn:user:input:session:old\n```"),
            format!("请解释下列引用。\n> {positive} turn:user:input:session:old"),
            "这个已关闭 turn:user:input:session:old 的 failed_tools 是什么？不要修改任何文件，不要执行命令，不要新增检查，直接解释即可。".into(),
            format!("{positive} turn:user:input:session:old 但报告后读取当前 result.txt 核验SHA。"),
            format!("如果你愿意，{positive} turn:user:input:session:old"),
            format!("{positive} 唯一标识在数据中：\n```json\n{{\"source\":\"turn:user:input:session:old\"}}\n```"),
        ] {
            assert!(historical_report_only_sources(&[input(&text)],"session","current").is_empty(),"not a direct report-only source: {text}");
        }
        let raw=serde_json::json!({"task":positive,"source":"turn:user:input:session:old"}).to_string();
        assert!(!historical_report_only_requested(&[input(&raw)]));
    }
}

pub(crate) fn validate_ledger_budget(next: &[AgentTaskRequirement]) -> Result<(), String> {
    if next.len() > 32
        || serde_json::to_vec(&next)
            .map_err(|_| "Requirements are not serializable".to_owned())?
            .len()
            > 24 * 1024
    {
        return Err("Requirement ledger exceeds 32 items or 24 KiB; keep requirements concise without dropping accepted scope".into());
    }
    Ok(())
}

pub(crate) fn require_input_coverage(
    requirements: &[AgentTaskRequirement],
    input_count: usize,
) -> Result<(), String> {
    if requirements.is_empty()
        || (0..input_count).any(|input| !requirements.iter().any(|item| item.source.input == input))
    {
        return Err("Use update_plan.requirements to record the obligations/constraints in every accepted input (input 0 is the original request); include newly accepted corrections. Plans cannot silently discard input.".into());
    }
    Ok(())
}

/// Conservative structured policy derived only from explicit accepted-user
/// wording. This never grants a retry; a later accepted input can explicitly
/// revoke an earlier stop policy. Requiring both stop-on-error and no-retry
/// language avoids treating incidental mentions of either word as control.
pub(crate) fn failure_stop_requested(inputs: &[ChatMessage]) -> bool {
    let mut policy = None;
    for input in inputs.iter().filter(|input| input.role == ChatRole::User) {
        for text in input.content.iter().filter_map(|part| match part {
            ChatContentPart::Text { text } => Some(text),
            _ => None,
        }) {
            let normalized = text
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let retry_allowed = [
                "you may retry",
                "retry is allowed",
                "continue retrying",
                "可以重试",
                "允许重试",
                "继续重试",
            ]
            .iter()
            .any(|phrase| normalized.contains(phrase));
            if retry_allowed {
                policy = Some(false);
            }
            let stop_on_error = [
                "stop on error",
                "on error stop",
                "stop after an error",
                "stop after any error",
                "遇到错误立即停止",
                "错误后立即停止",
                "出错立即停止",
                "失败立即停止",
            ]
            .iter()
            .any(|phrase| normalized.contains(phrase));
            let retry_forbidden = (normalized.contains("do not")
                && normalized.contains("retry"))
                || (normalized.contains("don't") && normalized.contains("retry"))
                || [
                "do not retry",
                "don't retry",
                "never retry",
                "no retries",
                "不重试",
                "不要重试",
                "不得重试",
                "禁止重试",
            ]
            .iter()
            .any(|phrase| normalized.contains(phrase));
            if stop_on_error && retry_forbidden {
                policy = Some(true);
            }
        }
    }
    policy.unwrap_or(false)
}

/// Conservative turn-local policy derived only from explicit accepted-user
/// wording. It narrows workspace mutation tools but never removes read-only
/// inspection or process execution, whose shell text remains model-authored.
/// A later accepted input can explicitly revoke the restriction.
pub(crate) fn workspace_mutation_forbidden(inputs: &[ChatMessage]) -> bool {
    const FORBIDDEN: &[&str] = &[
        "do not modify any files",
        "don't modify any files",
        "do not change any files",
        "don't change any files",
        "do not create, modify, or delete any files",
        "without modifying any files",
        "no file changes",
        "不要修改任何文件",
        "不得修改任何文件",
        "不修改任何文件",
        "不要创建、修改或删除任何文件",
        "不要创建、修改或删除文件",
        "不得创建、修改或删除文件",
        "不得新增、修改或删除文件",
        "不要新增、修改或删除文件",
        "不得新增、修改或删除任何文件",
        "不要写入任何文件",
    ];
    const ALLOWED: &[&str] = &[
        "you may modify files now",
        "you can modify files now",
        "file changes are now allowed",
        "you may write files now",
        "现在可以修改文件",
        "你可以修改文件",
        "现在允许修改文件",
        "我允许你修改文件",
        "现在可以写入文件",
        "现在可以继续修改",
    ];

    let mut policy = None;
    for input in inputs.iter().filter(|input| input.role == ChatRole::User) {
        for text in input.content.iter().filter_map(|part| match part {
            ChatContentPart::Text { text } => Some(text),
            _ => None,
        }) {
            let normalized = text
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let newest = FORBIDDEN
                .iter()
                .flat_map(|phrase| normalized.match_indices(phrase).map(|(index, _)| (index, true)))
                .chain(ALLOWED.iter().flat_map(|phrase| {
                    normalized
                        .match_indices(phrase)
                        .map(|(index, _)| (index, false))
                }))
                .max_by_key(|(index, _)| *index);
            if let Some((_, next)) = newest {
                policy = Some(next);
            }
        }
    }
    policy.unwrap_or(false)
}

/// Conservative output/action narrowing for an explicitly requested historical
/// report. The caller must independently prove the current explicit source is
/// loaded and canonically closed. This text matcher grants no history access,
/// execution, evidence freshness or permission, and never changes user input.
fn historical_report_only_direct_text(inputs:&[ChatMessage])->Option<String> {
    let latest=inputs.iter().rfind(|input|input.role==ChatRole::User)?;
    let mut direct=String::new();
    for part in &latest.content {
        let ChatContentPart::Text {text}=part else {continue;};
        let trimmed=text.trim();
        if matches!(trimmed.chars().next(),Some('{'|'['))
            && serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {continue;}
        let mut fenced=false;
        for line in text.lines() {
            let line=line.trim_start();
            if line.starts_with("```")||line.starts_with("~~~") {fenced=!fenced;continue;}
            if fenced||line.starts_with('>') {continue;}
            let mut quote=None;let mut quoted=String::new();
            for ch in line.chars() {
                if let Some(end)=quote {
                    if ch==end {
                        if end=='`' && quoted.starts_with("turn:user:") && quoted.len()<=256
                            && quoted.split(':').count()==5 && quoted.split(':').all(|field|!field.is_empty())
                            && quoted.chars().all(|ch|ch.is_ascii_alphanumeric()||matches!(ch,':'|'-'|'_')) {
                            direct.push_str(&quoted);
                        }
                        quote=None;quoted.clear();
                    } else {quoted.push(ch);}
                    continue;
                }
                quote=match ch {'"'=>Some('"'),'\''=>Some('\''),'`'=>Some('`'),'“'=>Some('”'),'‘'=>Some('’'),_=>None};
                if quote.is_none() {direct.push(ch);}
            }
            direct.push('\n');
        }
    }
    let normalized=direct.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let has=|phrases:&[&str]|phrases.iter().any(|phrase|normalized.contains(phrase));
    if has(&["这段字符串为何","这段文字为何","解释这段字符串","解释这个json","explain this string","explain this json",
        "现在检查","现在验证","现在读取","然后检查","再检查","再验证","再读取","报告后读取","报告后检查",
        "允许检查","可以检查","核验当前","检查当前","读取当前","查看当前","现在执行","允许执行",
        "then run","then check","after the report","verify current","inspect current","read current","now run",
        "current checks are allowed","you may inspect","may execute","no tools","do not call any tools","不要调用任何工具",
        "可以修改文件","允许修改文件","请重做","然后重做","就重做","如果可以","如果你","如果要",
        "if possible","if allowed","if you","would you","can you","是否应该","难道","为什么要"]) {return None;}
    let report=(normalized.contains("报告")&&has(&["补齐","整理","汇总","交付","完整报告","报告必须"]))
        || has(&["provide a historical report","deliver the historical report","report the historical results",
            "report the previously recorded results","complete the historical report","summarize the recorded results"]);
    let old_only=has(&["只依据","仅依据","仅允许读取","只读取","use only","using only","only read","based only"])
        && has(&["历史记录","工具记录","关闭回合","已关闭","historical records","historical results","recorded results","closed turn"]);
    let no_commands=has(&["不要执行命令","不得执行命令","不执行任何命令","不要运行命令","禁止运行命令",
        "do not execute commands","do not run commands","do not run any commands","no command execution","never execute commands"]);
    let no_new_checks=has(&["不要新增当前文件检查","不要新增检查","不得新增检查","不进行新的检查","不要做新检查",
        "不要开展新的检查","不进行当前检查","no new checks","do not perform new checks","do not inspect current files",
        "do not check current files","do not perform current checks"]);
    let direct_input=crate::context_lifecycle::text_message(ChatRole::User,direct.clone());
    (report&&old_only&&no_commands&&no_new_checks&&workspace_mutation_forbidden(&[direct_input])).then_some(direct)
}

#[cfg(test)]
fn historical_report_only_requested(inputs:&[ChatMessage])->bool {
    historical_report_only_direct_text(inputs).is_some()
}

pub(crate) fn historical_report_only_sources(inputs:&[ChatMessage],session:&str,current:&str)->Vec<String> {
    let Some(direct)=historical_report_only_direct_text(inputs) else {return Vec::new();};
    crate::history_reference::addressed(&crate::context_lifecycle::text_message(ChatRole::User,direct),session,current)
}
