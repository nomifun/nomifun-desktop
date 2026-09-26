//! Host-owned execution allowances. These are not model tool arguments and
//! do not grant capabilities, effect replay, or a change of accepted scope.
use serde::{Deserialize, Serialize};

pub const MAX_NATIVE_APPROVED_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_NATIVE_APPROVED_JOURNAL_RECORDS: u64 = 240_000;
pub const MAX_NATIVE_APPROVED_PAYLOAD_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_NATIVE_APPROVED_REPLAY_BYTES: usize = 80 * 1024 * 1024;

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
