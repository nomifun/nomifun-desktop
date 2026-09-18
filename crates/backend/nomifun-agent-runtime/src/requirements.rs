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
        "properties":{"input":{"type":"integer","minimum":0},"quote":{"type":"string","maxLength":512}}})
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::json!({"type":"array","maxItems":32,"items":{
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
            "Requirement source must identify an accepted current-turn user input".to_owned()
        })?;
    if citation.quote.len() > 512 {
        return Err("Requirement source quote exceeds 512 UTF-8 bytes".into());
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
        return Err("Source quote does not occur in that accepted input; tool output, summaries and invented quotations are not user sources".into());
    }
    Ok(())
}

/// Repeating an unchanged ID is harmless; rewriting/deleting its obligation
/// is not. New input may explain why old work is no longer applicable, but the
/// old requirement remains in the completion account with a scope citation.
pub(crate) fn merge(
    current: &[AgentTaskRequirement],
    additions: &[AgentTaskRequirement],
    inputs: &[ChatMessage],
) -> Result<Vec<AgentTaskRequirement>, String> {
    if additions.len() > 32 {
        return Err("Too many requirement additions".into());
    }
    let mut next = current.to_vec();
    let mut seen = BTreeSet::new();
    for item in additions {
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
            || item.description.len() > 512
        {
            return Err("Requirements need unique ASCII IDs (letters/digits/-/_), bounded descriptions and accepted input sources".into());
        }
        validate_citation(&item.source, inputs, true)?;
        if let Some(existing) = current.iter().find(|existing| existing.id == item.id) {
            if existing.description != item.description || existing.source != item.source {
                return Err("Existing requirements are immutable; add a new requirement and account for scope changes explicitly at completion".into());
            }
        } else {
            next.push(item.clone());
        }
    }
    validate_ledger_budget(&next)?;
    require_input_coverage(&next, inputs.len())?;
    Ok(next)
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
