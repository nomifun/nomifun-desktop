//! Exact native parent commands. A parent reply remains Agent-authored
//! guidance; it cannot stand in for authenticated human consent.
use serde::{Deserialize,Serialize};
use crate::{AgentSessionId,OperationId,EventId,DigestHex,digest_bytes,digest_payload};

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="SCREAMING_SNAKE_CASE")]
pub enum AgentCollaborationCommandRejectCode {
    ChildRequiresInput,
    TargetStateChanged,
    CommandUnavailable,
    CommandRejected,
    TemporaryUnavailable,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentQuestionReplyReference {
    pub agent_session_id:AgentSessionId,
    pub turn_operation_id:OperationId,
    pub question_event_id:EventId,
    pub request_id:crate::chat_model::ToolCallId,
}

/// References retained by the existing DecisionInput outbox. This is Agent
/// provenance, never an authenticated human response or permission receipt.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentParentQuestionReply {
    pub question:AgentQuestionReplyReference,
    pub sender_session_id:AgentSessionId,
    pub sender_turn_operation_id:OperationId,
    pub sender_command_event_id:EventId,
}

pub fn parent_question_reply_content(content:&str,source:&AgentParentQuestionReply)->Result<String,String> {
    serde_json::to_string(&serde_json::json!({"kind":"parent_agent_question_reply","human_permission":false,
        "message":content,"source":source})).map_err(|error|error.to_string())
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum AgentCollaborationMessageIntent {
    Guidance {},
    QuestionReply {question:AgentQuestionReplyReference},
    TaskControl {control:crate::AgentCollaborationTaskControl},
}

impl Default for AgentCollaborationMessageIntent {
    fn default()->Self {Self::Guidance{}}
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationMessage {
    pub execution_id:String,
    pub step_id:String,
    pub content:String,
    pub idempotency_key:String,
    #[serde(default)]
    pub intent:AgentCollaborationMessageIntent,
}

impl AgentCollaborationMessage {
    pub fn validate(&self)->Result<(),String> {
        fn id(value:&str)->bool {!value.is_empty()&&value.len()<=256&&value.trim()==value&&!value.chars().any(char::is_control)}
        if !id(&self.execution_id)||!id(&self.step_id)||!id(&self.idempotency_key)||self.idempotency_key.len()>128
            ||self.content.trim().is_empty()||self.content.len()>64*1024 {return Err("collaboration message exceeds its bounded contract".into());}
        if let AgentCollaborationMessageIntent::QuestionReply {question}=&self.intent {
            if [question.agent_session_id.as_ref(),question.turn_operation_id.as_ref(),question.question_event_id.as_ref(),question.request_id.as_ref()]
                .iter().any(|value|!id(value)) {return Err("question reply has an invalid exact reference".into());}
        }
        if let AgentCollaborationMessageIntent::TaskControl {control}=&self.intent {control.validate()?;}
        Ok(())
    }

    /// Stable guidance digests retain their existing meaning. A reply binds
    /// its content AND exact question, so a reused key cannot answer a successor.
    pub fn content_digest(&self)->Result<DigestHex,String> {
        match &self.intent {
            AgentCollaborationMessageIntent::Guidance{}=>Ok(digest_bytes(self.content.as_bytes())),
            AgentCollaborationMessageIntent::QuestionReply {question}=>digest_payload(&serde_json::json!({
                "kind":"parent_question_reply","content":self.content,"question":question})).map_err(|error|error.to_string()),
            AgentCollaborationMessageIntent::TaskControl {control}=>digest_payload(&serde_json::json!({
                "kind":"parent_task_control","content":self.content,"control":control})).map_err(|error|error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_question_identity_is_part_of_the_original_reply_digest() {
        let mut command:AgentCollaborationMessage=serde_json::from_value(serde_json::json!({"execution_id":"execution","step_id":"step",
            "content":"Original answer","idempotency_key":"same-key","intent":{"kind":"question_reply","question":{
                "agent_session_id":"child","turn_operation_id":"child-turn","question_event_id":"question-1","request_id":"request-1"}}})).unwrap();
        assert!(command.validate().is_ok());let original=command.content_digest().unwrap();
        if let AgentCollaborationMessageIntent::QuestionReply{question}=&mut command.intent {question.question_event_id="question-2".into();}
        assert_ne!(command.content_digest().unwrap(),original,"the same key and text cannot silently answer a successor question");
    }
    #[test]
    fn ordinary_guidance_has_no_implicit_question_or_human_permission() {
        let command:AgentCollaborationMessage=serde_json::from_value(serde_json::json!({"execution_id":"execution","step_id":"step",
            "content":"Original guidance","idempotency_key":"same-key"})).unwrap();
        assert_eq!(command.intent,AgentCollaborationMessageIntent::Guidance{});
        assert_eq!(command.content_digest().unwrap(),digest_bytes(command.content.as_bytes()));
        assert!(serde_json::from_value::<AgentCollaborationMessageIntent>(serde_json::json!({"kind":"guidance","question":{}})).is_err());
    }

    #[test]
    fn task_control_binds_exact_versions_and_replacement_to_the_original_command() {
        let mut command:AgentCollaborationMessage=serde_json::from_value(serde_json::json!({"execution_id":"execution","step_id":"original",
            "content":"use verified replacement","idempotency_key":"same-key","intent":{"kind":"task_control","control":{
                "expected_execution_version":4,"expected_step_version":2,"action":{"kind":"replace_with_existing","replacement_step_id":"completed",
                    "expected_replacement_step_version":3}}}})).unwrap();
        command.validate().unwrap();let digest=command.content_digest().unwrap();
        if let AgentCollaborationMessageIntent::TaskControl {control}=&mut command.intent {control.expected_step_version+=1;}
        assert_ne!(digest,command.content_digest().unwrap());
        if let AgentCollaborationMessageIntent::TaskControl {control}=&mut command.intent {control.expected_step_version=-1;}
        assert!(command.validate().is_err());
    }
}
