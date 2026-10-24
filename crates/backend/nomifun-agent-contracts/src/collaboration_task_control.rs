//! Source-scoped task controls. Versions identify the selected task before a
//! command is accepted; idempotency remains the original message command key.
use serde::{Deserialize,Serialize};

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationTaskControl {
    pub expected_execution_version:i64,
    pub expected_step_version:i64,
    pub action:AgentCollaborationTaskAction,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum AgentCollaborationTaskAction {
    Retry {},
    Restart {expected_attempt_id:Option<String>},
    Recheck {},
    Stop {},
    ReplaceWithExisting {replacement_step_id:String,expected_replacement_step_version:i64},
}

impl AgentCollaborationTaskControl {
    pub fn validate(&self)->Result<(),String> {
        if self.expected_execution_version<0||self.expected_step_version<0 {
            return Err("task control requires exact nonnegative versions".into());
        }
        if let AgentCollaborationTaskAction::Restart {expected_attempt_id:Some(attempt)}=&self.action
            && (attempt.trim().is_empty()||attempt.len()>256) {
            return Err("restart requires the exact selected Attempt identity".into());
        }
        if let AgentCollaborationTaskAction::ReplaceWithExisting {replacement_step_id,expected_replacement_step_version}=&self.action
            && (replacement_step_id.trim().is_empty()||replacement_step_id.len()>256||*expected_replacement_step_version<0) {
            return Err("replacement requires an exact existing task and version".into());
        }
        Ok(())
    }
}
