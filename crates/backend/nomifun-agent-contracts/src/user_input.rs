//! A normal user-input handoff on the existing canonical Turn.
//! The request is not an answer, permission, or task completion evidence.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::chat_model::ToolCallId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct AgentUserInputRequest {
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub request_id: ToolCallId,
    pub question: String,
    /// Model-authored description of unfinished work, not a new obligation.
    pub remaining_work: String,
}

impl AgentUserInputRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.request_id.as_ref().trim().is_empty() || self.request_id.as_ref().len() > 256
            || [&self.question, &self.remaining_work].iter().any(|value|
                value.trim().is_empty() || value.len() > 16_384
                    || value.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t')))
        {
            return Err("User input request has an empty, invalid or oversized field".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_input_request_is_bounded_and_never_carries_an_answer() {
        let request = AgentUserInputRequest { request_id: "question-1".into(),
            question: "请提供姓名和爱好。".into(), remaining_work: "收到资料后生成 PPT。".into() };
        assert!(request.validate().is_ok());
        let mut invalid = request.clone();
        invalid.question = " ".into();
        assert!(invalid.validate().is_err());
        invalid.question = "x".repeat(16_385);
        assert!(invalid.validate().is_err());
        let mut value = serde_json::to_value(request).unwrap();
        value["answer"] = serde_json::json!("fabricated");
        assert!(serde_json::from_value::<AgentUserInputRequest>(value).is_err());
    }
}
