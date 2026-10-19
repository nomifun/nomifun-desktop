//! Live Session process ownership and its actual release receipt. Retention
//! never proves quiescence, authorizes replay, or restores a process after restart.
use std::collections::BTreeSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::{AgentSessionId, DigestHex, OperationId, PrincipalRef, ResolvedSnapshotRef};

pub const SESSION_PROCESS_OWNER_RELEASED: &str = "session/process-owner-released";
pub const SESSION_PROCESS_REAPED: &str = "session/process-reaped";

/// The original native owner's complete terminal observation for one retained
/// process. It is neither a second ToolResult nor a whole-owner release.
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionProcessReapedV1 {
    pub retention:NativeSessionProcessRetention,
    /// The stable first-party process terminal wire value, validated by its
    /// original process type at the Store boundary. Bounded output is retained.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub terminal:Option<crate::StrictJsonValue>,
    /// An existing first-party process terminal is referenced, never copied
    /// into a second business ToolResult.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub terminal_ref:Option<crate::EventId>,
    /// Original bounded raw output and decoder state. This has no native
    /// process handle, input capability, or execution/replay authority.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub output_replay:Option<crate::StrictJsonValue>,
    /// Store-authored original frozen bytes in Session Payload chunks. Native
    /// output is observation data, never model/context budget or execution.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub output_archive:Option<SessionProcessOutputArchiveV1>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub component_identity:Option<crate::StrictJsonValue>,
}

impl SessionProcessReapedV1 {
    pub fn validate(&self)->Result<(),String> {
        self.retention.validate()?;
        if self.retention.processes.len()!=1 {return Err("process reap must identify exactly one original process".into());}
        if let Some(archive)=&self.output_archive {
            if self.terminal.is_some()||self.output_replay.is_some()||archive.chunks.is_empty()||archive.byte_len==0
                ||archive.chunks.iter().any(|id|id.as_ref().is_empty()) {
                return Err("process archive requires native terminal metadata and original Payload chunks".into());
            }
        } else if self.terminal.is_some()==self.terminal_ref.is_some() {return Err("process reap requires exactly one original terminal or terminal reference".into());}
        Ok(())
    }
}

#[derive(Clone,Debug,PartialEq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionProcessOutputArchiveV1 {
    /// Exact first-party terminal metadata, with its original cleanup report.
    /// The Store validates the stable native type without loading output.
    pub terminal:crate::StrictJsonValue,
    pub byte_len:u64,
    pub digest:DigestHex,
    pub chunks:Vec<crate::SessionPayloadId>,
}

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
