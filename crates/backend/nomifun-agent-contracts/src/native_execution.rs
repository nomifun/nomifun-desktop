//! Host-owned execution allowances. These are not model tool arguments and
//! do not grant capabilities, effect replay, or a change of accepted scope.
use serde::{Deserialize, Serialize};

/// Canonical native owner evidence for a nonterminal pause. A checkpoint by
/// itself grants no capacity release or resume authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePauseState {
    pub revision: u64,
    pub reason: String,
    pub checkpoint_revision: u64,
    pub checkpoint_digest: Option<crate::DigestHex>,
    pub execution_fence: u64,
    pub cleanup_proven: bool,
    pub paused_at_ms: i64,
}

pub const MAX_NATIVE_APPROVED_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_NATIVE_APPROVED_JOURNAL_RECORDS: u64 = 240_000;
pub const MAX_NATIVE_APPROVED_PAYLOAD_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_NATIVE_APPROVED_REPLAY_BYTES: usize = 80 * 1024 * 1024;

/// Canonical owner-authored process-tree barriers. A quiescent receipt proves
/// retained tree cleanup, never command success or effect rollback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", deny_unknown_fields)]
pub enum NativeProcessWitness {
    #[serde(rename = "host_process_dispatch")]
    Dispatch {
        operation_id: String,
        ordinal: u16,
        /// The original Session process scope, captured before native dispatch.
        /// Older barriers without this binding never authorize a new reap path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_owner: Option<NativeProcessOwnerIdentity>,
    },
    #[serde(rename = "host_process_quiescent")]
    Quiescent { operation_id: String, ordinal: u16, process_count: usize },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProcessOwnerIdentity {
    pub owner_generation: String,
    pub resource_binding_digest: crate::DigestHex,
}

impl NativeProcessWitness {
    pub fn validate(&self) -> Result<(), &'static str> {
        let (operation, ordinal) = match self {
            Self::Dispatch { operation_id, ordinal, process_owner } => {
                if let Some(owner) = process_owner {
                    if !uuid::Uuid::parse_str(&owner.owner_generation).is_ok_and(|id|
                        id.get_version_num() == 7 && id.to_string() == owner.owner_generation)
                        || owner.resource_binding_digest.as_ref().len() != 64
                        || !owner.resource_binding_digest.as_ref().bytes().all(|byte|
                            byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
                        return Err("invalid original native process owner binding");
                    }
                }
                (operation_id, *ordinal)
            },
            Self::Quiescent { operation_id, ordinal, process_count } => {
                if *process_count > 512 { return Err("native process barrier exceeds the scope budget"); }
                (operation_id, *ordinal)
            }
        };
        if operation.is_empty() || operation.len() > 1024 || operation.chars().any(char::is_control)
            || ordinal == 0 || ordinal > 512 {
            return Err("invalid native process barrier identity");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeExecutionBudget {
    pub revision: u64,
    pub journal_bytes: u64,
    pub journal_records: u64,
    pub session_payload_bytes: u64,
}

impl Default for NativeExecutionBudget {
    fn default() -> Self {
        Self { revision: 0, journal_bytes: 16 * 1024 * 1024, journal_records: 60_000,
            session_payload_bytes: 16 * 1024 * 1024 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBudgetIncrease {
    #[serde(default)]
    pub additional_segments: u16,
    #[serde(default)]
    pub additional_journal_mib: u16,
    #[serde(default)]
    pub additional_payload_mib: u16,
    /// The owner explicitly acknowledges retrying a stalled/correction loop.
    /// This is never inferred from a model reply or a recovered checkpoint.
    #[serde(default)]
    pub retry_stall_guards: bool,
}

impl NativeExecutionBudget {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(16 * 1024 * 1024..=MAX_NATIVE_APPROVED_JOURNAL_BYTES).contains(&self.journal_bytes)
            || !(60_000..=MAX_NATIVE_APPROVED_JOURNAL_RECORDS).contains(&self.journal_records)
            || !(16 * 1024 * 1024..=MAX_NATIVE_APPROVED_PAYLOAD_BYTES).contains(&self.session_payload_bytes) {
            return Err("execution allowance exceeds the compiled host safety ceiling");
        }
        Ok(())
    }

    pub fn increased(&self, grant: &NativeBudgetIncrease) -> Result<Self, &'static str> {
        self.validate()?;
        if grant.additional_segments > 16 || grant.additional_journal_mib > 16 || grant.additional_payload_mib > 32 {
            return Err("one authorization exceeds its bounded additional allowance");
        }
        let mut next = self.clone();
        next.revision = next.revision.checked_add(1).ok_or("execution allowance revision exhausted")?;
        next.journal_bytes = next.journal_bytes.checked_add(u64::from(grant.additional_journal_mib) * 1024 * 1024).ok_or("journal allowance overflow")?;
        next.journal_records = next.journal_records.checked_add(u64::from(grant.additional_journal_mib) * 2800).ok_or("journal record allowance overflow")?;
        next.session_payload_bytes = next.session_payload_bytes.checked_add(u64::from(grant.additional_payload_mib) * 1024 * 1024).ok_or("payload allowance overflow")?;
        next.validate()?;
        Ok(next)
    }
}
