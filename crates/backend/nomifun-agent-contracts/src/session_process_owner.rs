//! Live Session process ownership and its actual release receipt. Retention
//! never proves quiescence, authorizes replay, or restores a process after restart.
use std::collections::BTreeSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::{AgentSessionId, DigestHex, OperationId, PrincipalRef, ResolvedSnapshotRef};

pub const SESSION_PROCESS_OWNER_RELEASED: &str = "session/process-owner-released";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeRetainedProcess {
    pub process_id: String,
    pub pid: u32,
    pub creator_turn_id: OperationId,
    pub creator_call_id: OperationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub resource_binding_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeSessionProcessRetention {
    pub agent_session_id: AgentSessionId,
    pub owner_ref: PrincipalRef,
    pub owner_generation: String,
    pub processes: Vec<NativeRetainedProcess>,
}

impl NativeSessionProcessRetention {
    pub fn validate(&self) -> Result<(), String> {
        let uuid = |value:&str| uuid::Uuid::parse_str(value).is_ok_and(|id| id.get_version_num()==7 && id.to_string()==value);
        let bounded = |value:&str| !value.is_empty() && value.len()<=1024 && !value.chars().any(char::is_control);
        let digest = |value:&str| value.len()==64 && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
        let mut ids = BTreeSet::new();
        let mut creators = BTreeSet::new();
        if !uuid(self.agent_session_id.as_ref()) || !uuid(&self.owner_generation)
            || self.owner_ref.principal_kind!="user" || !bounded(&self.owner_ref.principal_id) || self.processes.len()>64
            || self.processes.iter().any(|process| !uuid(&process.process_id) || process.pid==0 || !ids.insert(&process.process_id)
                || !creators.insert((&process.creator_turn_id,&process.creator_call_id))
                || !bounded(process.creator_turn_id.as_ref()) || !bounded(process.creator_call_id.as_ref())
                || !bounded(process.resolved_snapshot_ref.snapshot_id.as_ref()) || !digest(process.resolved_snapshot_ref.snapshot_digest.as_ref())
                || !digest(process.resource_binding_digest.as_ref())) {
            return Err("invalid Session process owner retention proof".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionProcessOwnerReleasedV1 {
    pub retention: NativeSessionProcessRetention,
    pub reaped: bool,
}

impl SessionProcessOwnerReleasedV1 {
    pub fn validate(&self) -> Result<(), String> {
        self.retention.validate()?;
        if !self.reaped || self.retention.processes.is_empty() { return Err("Session process owner release requires exact reaped processes".into()); }
        Ok(())
    }
}
