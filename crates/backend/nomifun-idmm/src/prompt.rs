//! Sidecar prompt assembly and strict-JSON decision parsing. Kept pure so it can
//! be unit-tested without a provider. NEVER log the assembled prompt or context
//! in production-visible logs (it contains user data).

use nomifun_api_types::{DecisionStrategy, Tendency};

use crate::signal::StallClass;

/// System prompt establishing the sidecar's role and strict output contract.
/// `action` covers both the option/permission path (`answer_choice`) and the
/// open-question free-text path (`answer_text`).
pub const SIDECAR_SYSTEM: &str = "You are a supervisory co-pilot. Your ONLY job is to unblock and steer a stalled \
agent session. Respond with STRICT JSON only — no prose, no code fences:\n\
{\"action\":\"retry|send_text|answer_choice|answer_text|wait|stop\",\"text\":\"\",\"wait_secs\":0,\"confidence\":0.0,\"reason\":\"\"}\n\
Field meanings: retry = re-run/continue the current step; send_text = inject the given text as a nudge or \
instruction; answer_choice = answer a pending option/permission decision with the given option/value; answer_text = \
answer an OPEN-ENDED question with a concise free-text reply; wait = do nothing for wait_secs; stop = give up and ask \
the human (reason required). confidence is 0..1.\n\
Hard rules: obey the DECISION POLICY exactly. Never propose destructive actions unless the SAFETY block allows them. \
Prefer the smallest action that unblocks the session.";

/// Render the human-readable decision-policy block from a [`DecisionStrategy`]:
/// the tendency, the on-blocked behavior, the never-destructive guard, and the
/// optional user freeform policy. Threaded into both the option and open-question
/// prompts (plan D5/D6) so the model is bound by the same structured guardrails
/// that drive the rule tier.
fn policy_block(strat: &DecisionStrategy) -> String {
    let tendency = match strat.tendency {
        Tendency::Conservative => "conservative (prefer the safest option; ask/halt when unsure)",
        Tendency::Balanced => "balanced",
        Tendency::Aggressive => "aggressive (keep work moving; decide decisively when safe)",
    };
    let freeform = strat.freeform_policy.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let freeform = freeform.unwrap_or(
        "(none provided — act conservatively; prefer recommended/default options and avoid irreversible actions)",
    );
    format!(
        "tendency={tendency}\non_blocked={:?}\nfreeform_policy:\n{freeform}",
        strat.on_blocked,
    )
}

/// Build the user message for an OPTION / PERMISSION decision or a fault/idle
/// stall: policy + safety + stall + context blocks.
pub fn build_user_prompt(strat: &DecisionStrategy, class: StallClass, detail: &str, context: &str) -> String {
    let never_destructive = strat.categories.option_decision.never_destructive;
    format!(
        "DECISION POLICY (obey strictly):\n{}\n\n\
         SAFETY: allow_destructive={}\n\n\
         STALL: class={} detail={}\n\n\
         RECENT CONTEXT (most recent last; may be truncated):\n{}",
        policy_block(strat),
        !never_destructive,
        class.as_str(),
        detail,
        context,
    )
}

/// Build the user message for an OPEN-ended question (纯问答, D6). Asks for a
/// concise free-text answer bounded by `max_answer_chars`, constrained by the
/// strategy's tendency / freeform policy. The model must reply with
/// `action=answer_text`.
pub fn build_open_question_prompt(
    strat: &DecisionStrategy,
    question: &str,
    context: &str,
    max_answer_chars: u32,
) -> String {
    format!(
        "DECISION POLICY (obey strictly):\n{}\n\n\
         TASK: The agent asked the user an OPEN-ENDED question and is blocked waiting for a reply. \
         Answer it on the user's behalf so the work continues. Be concise (≤{max_answer_chars} characters), \
         decisive per the tendency above, and never commit to irreversible/destructive actions. \
         Reply with action=answer_text and put your answer in `text`. If you cannot answer safely, use action=stop.\n\n\
         OPEN QUESTION:\n{question}\n\n\
         RECENT CONTEXT (most recent last; may be truncated):\n{context}",
        policy_block(strat),
    )
}

/// The strict JSON decision the sidecar must return.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct SidecarDecision {
    pub action: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub wait_secs: u64,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub reason: String,
}

/// Parse the model's reply into a decision, tolerating ```json fences and
/// surrounding prose by extracting the outermost `{ … }` span.
pub fn parse_decision(raw: &str) -> Option<SidecarDecision> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{DecisionStrategy, Tendency};

    #[test]
    fn parse_decision_plain() {
        let d = parse_decision(r#"{"action":"retry","confidence":0.9,"reason":"transient 500"}"#).unwrap();
        assert_eq!(d.action, "retry");
        assert_eq!(d.confidence, 0.9);
        assert_eq!(d.reason, "transient 500");
    }

    #[test]
    fn parse_decision_with_fence_and_prose() {
        let raw = "Here is my decision:\n```json\n{\"action\":\"answer_choice\",\"text\":\"1\"}\n```\nDone.";
        let d = parse_decision(raw).unwrap();
        assert_eq!(d.action, "answer_choice");
        assert_eq!(d.text, "1");
    }

    #[test]
    fn parse_decision_answer_text_open_question() {
        let d = parse_decision(r#"{"action":"answer_text","text":"用 LRU 缓存","confidence":0.8}"#).unwrap();
        assert_eq!(d.action, "answer_text");
        assert_eq!(d.text, "用 LRU 缓存");
    }

    #[test]
    fn parse_decision_garbage_is_none() {
        assert!(parse_decision("I cannot help with that.").is_none());
        assert!(parse_decision("").is_none());
    }

    #[test]
    fn build_user_prompt_includes_policy_and_stall() {
        let strat = DecisionStrategy {
            freeform_policy: Some("never delete data".into()),
            ..Default::default()
        };
        let p = build_user_prompt(&strat, StallClass::ProviderError, "http 500", "...");
        assert!(p.contains("never delete data"));
        assert!(p.contains("class=provider_error"));
        // never_destructive defaults true → allow_destructive=false.
        assert!(p.contains("allow_destructive=false"));
    }

    #[test]
    fn build_user_prompt_handles_empty_freeform() {
        let strat = DecisionStrategy::default();
        let p = build_user_prompt(&strat, StallClass::Idle, "no output 90s", "ctx");
        assert!(p.contains("act conservatively"));
    }

    #[test]
    fn build_user_prompt_tendency_aggressive_reflected() {
        let strat = DecisionStrategy {
            tendency: Tendency::Aggressive,
            ..Default::default()
        };
        let p = build_user_prompt(&strat, StallClass::Decision, "pick one", "ctx");
        assert!(p.contains("aggressive"));
    }

    #[test]
    fn build_open_question_prompt_bounds_and_instructs() {
        let strat = DecisionStrategy::default();
        let p = build_open_question_prompt(&strat, "你希望缓存怎么设计？", "ctx", 600);
        assert!(p.contains("OPEN QUESTION"));
        assert!(p.contains("你希望缓存怎么设计"));
        assert!(p.contains("answer_text"));
        assert!(p.contains("600"));
    }
}
