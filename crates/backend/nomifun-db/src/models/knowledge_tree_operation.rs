use nomifun_common::{KnowledgeBaseId, KnowledgeTreeOperationId, TimestampMs};
use serde::{Deserialize, Serialize};
use sqlx::{Row, sqlite::SqliteRow};

pub const KNOWLEDGE_TREE_OPERATION_STATE_PREPARED: &str = "prepared";
pub const KNOWLEDGE_TREE_OPERATION_STATE_FILESYSTEM_COMMITTED: &str =
    "filesystem_committed";
pub const KNOWLEDGE_TREE_OPERATION_STATE_COMMITTED: &str = "committed";
pub const KNOWLEDGE_TREE_OPERATION_STATE_NEEDS_RECOVERY: &str = "needs_recovery";

pub const KNOWLEDGE_TREE_EVENT_STATUS_NONE: &str = "none";
pub const KNOWLEDGE_TREE_EVENT_STATUS_PENDING: &str = "pending";
pub const KNOWLEDGE_TREE_EVENT_STATUS_PUBLISHED: &str = "published";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTreeOperationState {
    Prepared,
    FilesystemCommitted,
    Committed,
    NeedsRecovery,
}

impl KnowledgeTreeOperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => KNOWLEDGE_TREE_OPERATION_STATE_PREPARED,
            Self::FilesystemCommitted => KNOWLEDGE_TREE_OPERATION_STATE_FILESYSTEM_COMMITTED,
            Self::Committed => KNOWLEDGE_TREE_OPERATION_STATE_COMMITTED,
            Self::NeedsRecovery => KNOWLEDGE_TREE_OPERATION_STATE_NEEDS_RECOVERY,
        }
    }

    pub fn parse(value: &str) -> Result<Self, InvalidKnowledgeTreeOperationValue> {
        match value {
            KNOWLEDGE_TREE_OPERATION_STATE_PREPARED => Ok(Self::Prepared),
            KNOWLEDGE_TREE_OPERATION_STATE_FILESYSTEM_COMMITTED => {
                Ok(Self::FilesystemCommitted)
            }
            KNOWLEDGE_TREE_OPERATION_STATE_COMMITTED => Ok(Self::Committed),
            KNOWLEDGE_TREE_OPERATION_STATE_NEEDS_RECOVERY => Ok(Self::NeedsRecovery),
            _ => Err(InvalidKnowledgeTreeOperationValue {
                field: "state",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTreeEventStatus {
    None,
    Pending,
    Published,
}

impl KnowledgeTreeEventStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => KNOWLEDGE_TREE_EVENT_STATUS_NONE,
            Self::Pending => KNOWLEDGE_TREE_EVENT_STATUS_PENDING,
            Self::Published => KNOWLEDGE_TREE_EVENT_STATUS_PUBLISHED,
        }
    }

    pub fn parse(value: &str) -> Result<Self, InvalidKnowledgeTreeOperationValue> {
        match value {
            KNOWLEDGE_TREE_EVENT_STATUS_NONE => Ok(Self::None),
            KNOWLEDGE_TREE_EVENT_STATUS_PENDING => Ok(Self::Pending),
            KNOWLEDGE_TREE_EVENT_STATUS_PUBLISHED => Ok(Self::Published),
            _ => Err(InvalidKnowledgeTreeOperationValue {
                field: "event_status",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid knowledge-tree operation {field} value {value:?}")]
pub struct InvalidKnowledgeTreeOperationValue {
    field: &'static str,
    value: String,
}

/// Durable coordinator row for one idempotent tree mutation.
///
/// The same row is also the transactional outbox record. A committed
/// operation has one event payload whose `event_status` advances independently
/// from `pending` to `published` without changing the mutation receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeTreeOperationRow {
    /// SQLite-local technical row key.
    pub id: i64,
    pub operation_id: KnowledgeTreeOperationId,
    pub knowledge_base_id: KnowledgeBaseId,
    /// Client idempotency key, unique only within its knowledge base.
    pub request_id: String,
    /// Lowercase SHA-256 of the canonical command payload.
    pub fingerprint: String,
    pub source_rel_path: String,
    pub destination_rel_path: String,
    pub source_fs_identity: Option<String>,
    pub state: KnowledgeTreeOperationState,
    pub receipt_json: Option<String>,
    pub error_message: Option<String>,
    pub event_status: KnowledgeTreeEventStatus,
    pub event_payload_json: Option<String>,
    pub filesystem_committed_at: Option<TimestampMs>,
    pub committed_at: Option<TimestampMs>,
    pub event_published_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl KnowledgeTreeOperationRow {
    /// Every non-terminal row requires reconciliation after an unclean stop.
    /// `prepared` is included because a crash can happen after the filesystem
    /// rename but before the next journal marker reaches SQLite.
    pub fn requires_recovery(&self) -> bool {
        self.state != KnowledgeTreeOperationState::Committed
    }

    pub fn has_pending_event(&self) -> bool {
        self.state == KnowledgeTreeOperationState::Committed
            && self.event_status == KnowledgeTreeEventStatus::Pending
    }
}

impl<'row> sqlx::FromRow<'row, SqliteRow> for KnowledgeTreeOperationRow {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        fn decode<T>(value: String) -> Result<T, sqlx::Error>
        where
            T: TryFrom<String>,
            T::Error: std::error::Error + Send + Sync + 'static,
        {
            T::try_from(value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
        }

        let state: String = row.try_get("state")?;
        let event_status: String = row.try_get("event_status")?;
        Ok(Self {
            id: row.try_get("id")?,
            operation_id: decode(row.try_get("operation_id")?)?,
            knowledge_base_id: decode(row.try_get("knowledge_base_id")?)?,
            request_id: row.try_get("request_id")?,
            fingerprint: row.try_get("fingerprint")?,
            source_rel_path: row.try_get("source_rel_path")?,
            destination_rel_path: row.try_get("destination_rel_path")?,
            source_fs_identity: row.try_get("source_fs_identity")?,
            state: KnowledgeTreeOperationState::parse(&state)
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
            receipt_json: row.try_get("receipt_json")?,
            error_message: row.try_get("error_message")?,
            event_status: KnowledgeTreeEventStatus::parse(&event_status)
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
            event_payload_json: row.try_get("event_payload_json")?,
            filesystem_committed_at: row.try_get("filesystem_committed_at")?,
            committed_at: row.try_get("committed_at")?,
            event_published_at: row.try_get("event_published_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_state_values_are_stable() {
        for (value, state) in [
            ("prepared", KnowledgeTreeOperationState::Prepared),
            (
                "filesystem_committed",
                KnowledgeTreeOperationState::FilesystemCommitted,
            ),
            ("committed", KnowledgeTreeOperationState::Committed),
            (
                "needs_recovery",
                KnowledgeTreeOperationState::NeedsRecovery,
            ),
        ] {
            assert_eq!(KnowledgeTreeOperationState::parse(value).unwrap(), state);
            assert_eq!(state.as_str(), value);
        }
        for (value, status) in [
            ("none", KnowledgeTreeEventStatus::None),
            ("pending", KnowledgeTreeEventStatus::Pending),
            ("published", KnowledgeTreeEventStatus::Published),
        ] {
            assert_eq!(KnowledgeTreeEventStatus::parse(value).unwrap(), status);
            assert_eq!(status.as_str(), value);
        }
    }
}
