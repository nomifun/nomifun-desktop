//! Read-only collaboration records over the original Execution and Session facts.
use serde::{Deserialize, Serialize};
pub use nomifun_common::{DelegationProcessRecord, DelegationRecordAnchor, DelegationRecordKind, DelegationRecordSource};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRecordsQuery {
    /// Exact loaded accepted-request window. This is not a lifecycle watermark.
    pub source_message_ids: Option<String>,
    pub after_session_seq: Option<i64>,
    pub through_session_seq: Option<i64>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct DelegationRecordsResponse {
    pub records: Vec<DelegationProcessRecord>,
    pub through_session_seq: i64,
    pub next_cursor: Option<String>,
}
