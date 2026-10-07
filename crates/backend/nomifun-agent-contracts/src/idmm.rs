//! Bounded, server-authored IDMM explanations in the canonical Session chain.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_IDMM_RATIONALE_CHARS: usize = 40;
pub const MAX_IDMM_RATIONALE_BYTES: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdmmDecisionSource {
    Rule,
    BypassModel,
    Recovery,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdmmDecisionModel {
    pub provider_id: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdmmQuestionRef {
    pub message_id: String,
    pub sequence: u64,
    pub fingerprint: String,
}

impl IdmmQuestionRef {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !canonical_uuidv7(&self.message_id) {
            return Err("IDMM question message_id must be a canonical UUIDv7");
        }
        if self.sequence == 0 || self.sequence > i64::MAX as u64 {
            return Err("IDMM question sequence must be a positive canonical sequence");
        }
        if self.fingerprint.len() != 64
            || !self.fingerprint.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("IDMM question fingerprint must be a lowercase SHA-256 digest");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdmmDecisionExplanation {
    pub intervention_id: String,
    pub source: IdmmDecisionSource,
    pub reason_code: String,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<IdmmDecisionModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<IdmmQuestionRef>,
}

impl IdmmDecisionExplanation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !canonical_uuidv7(&self.intervention_id) {
            return Err("IDMM intervention_id must be a canonical UUIDv7");
        }
        if self.reason_code.is_empty() || self.reason_code.len() > 80
            || !self.reason_code.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("IDMM reason_code must be a bounded machine code");
        }
        if self.rationale.trim().is_empty() || self.rationale.trim() != self.rationale
            || self.rationale.chars().count() > MAX_IDMM_RATIONALE_CHARS
            || self.rationale.len() > MAX_IDMM_RATIONALE_BYTES
            || self.rationale.chars().any(char::is_control)
        {
            return Err("IDMM rationale must be one bounded, nonempty line");
        }
        match (self.source, self.model.as_ref()) {
            (IdmmDecisionSource::BypassModel, Some(model)) => {
                if !canonical_uuidv7(&model.provider_id) || model.model.trim().is_empty()
                    || model.model.trim() != model.model || model.model.len() > 200
                    || model.model.chars().any(char::is_control)
                {
                    return Err("IDMM bypass model identity is invalid");
                }
            }
            (IdmmDecisionSource::BypassModel, None) => return Err("IDMM bypass explanation requires model identity"),
            (_, Some(_)) => return Err("only an IDMM bypass explanation may carry model identity"),
            (_, None) => {}
        }
        if let Some(question) = &self.question {
            question.validate()?;
        } else if self.source != IdmmDecisionSource::Recovery {
            return Err("IDMM rule and bypass explanations require the exact question reference");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdmmDecisionNoticeStatus {
    WaitingForHuman,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdmmDecisionNotice {
    pub decision: IdmmDecisionExplanation,
    pub status: IdmmDecisionNoticeStatus,
    pub created_at: i64,
}

impl IdmmDecisionNotice {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.decision.validate()?;
        if self.decision.question.is_none() {
            return Err("IDMM notice requires the exact question reference");
        }
        if self.created_at <= 0 {
            return Err("IDMM notice requires a positive creation time");
        }
        Ok(())
    }
}

fn canonical_uuidv7(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explanation() -> IdmmDecisionExplanation {
        IdmmDecisionExplanation {
            intervention_id: Uuid::now_v7().to_string(),
            source: IdmmDecisionSource::Rule,
            reason_code: "rule_selected_safe_option".into(),
            rationale: "优先选择推荐的可继续方案".into(),
            model: None,
            question: Some(IdmmQuestionRef {
                message_id: Uuid::now_v7().to_string(), sequence: 3, fingerprint: "a".repeat(64),
            }),
        }
    }

    #[test]
    fn explanation_enforces_unicode_bound_and_exact_identities() {
        let mut value = explanation();
        value.rationale = "😀".repeat(40);
        assert!(value.validate().is_ok());
        value.rationale.push('😀');
        assert!(value.validate().is_err());
        value.rationale = "确认继续".into();
        value.question.as_mut().unwrap().fingerprint = "A".repeat(64);
        assert!(value.validate().is_err());
        value.question.as_mut().unwrap().fingerprint = "a".repeat(64);
        value.question.as_mut().unwrap().sequence = 0;
        assert!(value.validate().is_err());
        value.question.as_mut().unwrap().sequence = 3;
        value.intervention_id = "0190f5fe-7c00-4a00-8000-000000000007".into();
        assert!(value.validate().is_err());
        value = explanation();
        value.question = None;
        assert!(value.validate().is_err());
        value.source = IdmmDecisionSource::Recovery;
        assert!(value.validate().is_ok());
    }

    #[test]
    fn bypass_model_identity_is_required_only_for_actual_bypass_decisions() {
        let mut value = explanation();
        value.source = IdmmDecisionSource::BypassModel;
        assert!(value.validate().is_err());
        value.model = Some(IdmmDecisionModel { provider_id: Uuid::now_v7().to_string(), model: "sidecar".into() });
        assert!(value.validate().is_ok());
        value.source = IdmmDecisionSource::Rule;
        assert!(value.validate().is_err());
    }

    #[test]
    fn notice_is_a_bounded_question_fact_without_confidence_or_unknown_fields() {
        let mut notice = IdmmDecisionNotice { decision: explanation(), status: IdmmDecisionNoticeStatus::WaitingForHuman, created_at: 1 };
        assert!(notice.validate().is_ok());
        let mut wire = serde_json::to_value(&notice.decision).unwrap();
        wire["confidence"] = serde_json::json!(0.99);
        assert!(serde_json::from_value::<IdmmDecisionExplanation>(wire).is_err());
        notice.decision.question = None;
        assert!(notice.validate().is_err());
    }
}
