use nomifun_common::{
    KnowledgeBaseId, KnowledgeEntryId, KnowledgeSourceId, KnowledgeSourceItemId, TimestampMs,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, sqlite::SqliteRow};

macro_rules! persisted_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub fn parse(value: &str) -> Result<Self, InvalidKnowledgeSourceValue> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => Err(InvalidKnowledgeSourceValue {
                        kind: stringify!($name),
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

persisted_enum! {
    /// Adapter kind of a normalized source aggregate.
    pub enum KnowledgeSourceKind {
        Url => "url",
    }
}

persisted_enum! {
    /// How source items participate in a knowledge base.
    pub enum KnowledgeSourceMode {
        Live => "live",
        Snapshot => "snapshot",
    }
}

persisted_enum! {
    /// Lifecycle shared by source aggregates and their independently managed
    /// items. Removed rows are tombstones retained for provenance and undo.
    pub enum KnowledgeSourceState {
        Active => "active",
        Paused => "paused",
        Removed => "removed",
    }
}

persisted_enum! {
    /// Durable per-item synchronization state. Lifecycle and synchronization
    /// are intentionally separate: a paused/removed item retains its last
    /// observed sync result.
    pub enum KnowledgeSourceItemSyncStatus {
        Pending => "pending",
        Syncing => "syncing",
        Synced => "synced",
        Failed => "failed",
        Conflicted => "conflicted",
        Missing => "missing",
    }
}

persisted_enum! {
    /// Relationship between one stable entry and its originating source item.
    /// Only `managed` grants the source authority over the body; detached and
    /// copy entries retain provenance while remaining user-editable.
    pub enum KnowledgeEntryProvenanceRelationship {
        Managed => "managed",
        Detached => "detached",
        Copy => "copy",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid persisted knowledge-source {kind} value {value:?}")]
pub struct InvalidKnowledgeSourceValue {
    kind: &'static str,
    value: String,
}

fn decode_id<T>(value: String) -> Result<T, sqlx::Error>
where
    T: TryFrom<String>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    T::try_from(value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn decode_optional_id<T>(value: Option<String>) -> Result<Option<T>, sqlx::Error>
where
    T: TryFrom<String>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    value.map(decode_id).transpose()
}

fn decode_enum<T>(
    value: String,
    parse: impl FnOnce(&str) -> Result<T, InvalidKnowledgeSourceValue>,
) -> Result<T, sqlx::Error> {
    parse(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

/// Durable normalized source aggregate. It owns source configuration, not a
/// filesystem locator; `default_parent_entry_id` is only the placement policy
/// for newly captured documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSourceRow {
    pub id: i64,
    pub knowledge_source_id: KnowledgeSourceId,
    pub knowledge_base_id: KnowledgeBaseId,
    pub kind: KnowledgeSourceKind,
    pub mode: KnowledgeSourceMode,
    pub state: KnowledgeSourceState,
    pub revision: i64,
    pub default_parent_entry_id: Option<KnowledgeEntryId>,
    pub removed_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl KnowledgeSourceRow {
    pub fn is_removed(&self) -> bool {
        self.state == KnowledgeSourceState::Removed
    }
}

impl<'row> sqlx::FromRow<'row, SqliteRow> for KnowledgeSourceRow {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            knowledge_source_id: decode_id(row.try_get("knowledge_source_id")?)?,
            knowledge_base_id: decode_id(row.try_get("knowledge_base_id")?)?,
            kind: decode_enum(row.try_get("kind")?, KnowledgeSourceKind::parse)?,
            mode: decode_enum(row.try_get("mode")?, KnowledgeSourceMode::parse)?,
            state: decode_enum(row.try_get("state")?, KnowledgeSourceState::parse)?,
            revision: row.try_get("revision")?,
            default_parent_entry_id: decode_optional_id(
                row.try_get("default_parent_entry_id")?,
            )?,
            removed_at: row.try_get("removed_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// One independently refreshable item in a source aggregate. URL identity and
/// synchronization state remain stable while its managed document moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSourceItemRow {
    pub id: i64,
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub knowledge_source_id: KnowledgeSourceId,
    pub requested_url: String,
    pub normalized_url: String,
    pub final_url: Option<String>,
    pub rendered: bool,
    pub title: Option<String>,
    pub ordinal: i64,
    pub state: KnowledgeSourceState,
    pub sync_status: KnowledgeSourceItemSyncStatus,
    pub revision: i64,
    pub etag: Option<String>,
    pub http_last_modified: Option<String>,
    pub last_attempt_at: Option<TimestampMs>,
    pub last_success_at: Option<TimestampMs>,
    pub last_error: Option<String>,
    pub last_published_hash: Option<String>,
    /// Publication intent persisted before touching the managed document. A
    /// non-null hash means recovery must compare the filesystem with this exact
    /// prepared payload before committing or failing the sync.
    pub pending_published_hash: Option<String>,
    pub pending_final_url: Option<String>,
    pub pending_title: Option<String>,
    pub pending_publication_at: Option<TimestampMs>,
    pub removed_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl KnowledgeSourceItemRow {
    pub fn is_removed(&self) -> bool {
        self.state == KnowledgeSourceState::Removed
    }
}

impl<'row> sqlx::FromRow<'row, SqliteRow> for KnowledgeSourceItemRow {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            knowledge_source_item_id: decode_id(row.try_get("knowledge_source_item_id")?)?,
            knowledge_source_id: decode_id(row.try_get("knowledge_source_id")?)?,
            requested_url: row.try_get("requested_url")?,
            normalized_url: row.try_get("normalized_url")?,
            final_url: row.try_get("final_url")?,
            rendered: row.try_get("rendered")?,
            title: row.try_get("title")?,
            ordinal: row.try_get("ordinal")?,
            state: decode_enum(row.try_get("state")?, KnowledgeSourceState::parse)?,
            sync_status: decode_enum(
                row.try_get("sync_status")?,
                KnowledgeSourceItemSyncStatus::parse,
            )?,
            revision: row.try_get("revision")?,
            etag: row.try_get("etag")?,
            http_last_modified: row.try_get("http_last_modified")?,
            last_attempt_at: row.try_get("last_attempt_at")?,
            last_success_at: row.try_get("last_success_at")?,
            last_error: row.try_get("last_error")?,
            last_published_hash: row.try_get("last_published_hash")?,
            pending_published_hash: row.try_get("pending_published_hash")?,
            pending_final_url: row.try_get("pending_final_url")?,
            pending_title: row.try_get("pending_title")?,
            pending_publication_at: row.try_get("pending_publication_at")?,
            removed_at: row.try_get("removed_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// Durable source lineage for one stable entry. The row deliberately survives
/// path changes; entry IDs referenced here are logical so projection repair can
/// temporarily remove/recreate an entry without destroying source history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeEntryProvenanceRow {
    pub id: i64,
    pub knowledge_entry_id: KnowledgeEntryId,
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub relationship: KnowledgeEntryProvenanceRelationship,
    pub derived_from_entry_id: Option<KnowledgeEntryId>,
    pub revision: i64,
    pub detached_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl<'row> sqlx::FromRow<'row, SqliteRow> for KnowledgeEntryProvenanceRow {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            knowledge_entry_id: decode_id(row.try_get("knowledge_entry_id")?)?,
            knowledge_source_item_id: decode_id(row.try_get("knowledge_source_item_id")?)?,
            relationship: decode_enum(
                row.try_get("relationship")?,
                KnowledgeEntryProvenanceRelationship::parse,
            )?,
            derived_from_entry_id: decode_optional_id(row.try_get("derived_from_entry_id")?)?,
            revision: row.try_get("revision")?,
            detached_at: row.try_get("detached_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_values_are_stable() {
        assert_eq!(KnowledgeSourceKind::Url.as_str(), "url");
        assert_eq!(KnowledgeSourceMode::Snapshot.as_str(), "snapshot");
        assert_eq!(KnowledgeSourceState::Paused.as_str(), "paused");
        assert_eq!(KnowledgeSourceItemSyncStatus::Conflicted.as_str(), "conflicted");
        assert_eq!(
            KnowledgeEntryProvenanceRelationship::Detached.as_str(),
            "detached"
        );
        assert!(KnowledgeSourceItemSyncStatus::parse("unknown").is_err());
    }
}
