//! Runtime assertions for the clean v3 database lineage.
//!
//! Product tables use local `INTEGER PRIMARY KEY AUTOINCREMENT` row identities.
//! Cross-boundary identities live in explicitly named columns, while logical
//! links replace SQLite foreign keys. Physical indexes are selected separately
//! from query workloads instead of being required for every relationship.
//! This module is the executable registry for those contracts and provides a
//! read-only orphan-audit skeleton.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::{Row, SqlitePool};

use crate::error::DbError;

/// The customer-service notes full-text index.
///
/// External-content FTS5 over `cs_notes.search_text` (migration 035), the
/// lexical half of note recall. It is a virtual table, so it carries NONE of
/// the row-key invariants the product tables below do — see
/// [`FTS_SHADOW_TABLES`].
pub(crate) const CS_NOTES_FTS_TABLE: &str = "cs_notes_fts";

/// Shadow tables SQLite materializes for [`CS_NOTES_FTS_TABLE`].
///
/// These belong to the v3 baseline table SET (so the registry stays an exact
/// equality check and a stray table is still caught), but they are EXEMPT from
/// the per-table structural asserts because their shape is owned by SQLite,
/// not by this repository: `cs_notes_fts` has no primary key at all, `_config`
/// keys on `k` and is WITHOUT ROWID, `_data`/`_docsize` declare
/// `id INTEGER PRIMARY KEY` without AUTOINCREMENT, and `_idx` uses a composite
/// `(segid, term)` key. Four of the five would fail
/// [`require_autoincrement_primary_key`]. Same treatment as the companion
/// store's FTS baseline (`nomifun-companion/src/store.rs:709-717`).
pub(crate) const FTS_SHADOW_TABLES: &[&str] = &[
    "cs_notes_fts_config",
    "cs_notes_fts_data",
    "cs_notes_fts_docsize",
    "cs_notes_fts_idx",
];

/// UARC Agent Store tables intentionally use canonical TEXT identities,
/// physical foreign keys and composite fact keys. They belong to the exact
/// database table set but not to the legacy v3 AUTOINCREMENT/logical-link
/// contract applied to `PRODUCT_TABLES`.
pub(crate) const CANONICAL_AGENT_STORE_TABLES: &[&str] = &[
    "schema_metadata",
    "agent_preset_templates",
    "agent_presets",
    "agent_preset_revisions",
    "agent_preset_contribution_locks",
    "agent_bindings",
    "agent_runtime_snapshots",
    "agent_sessions",
    "agent_deletion_audits",
    "agent_turns",
    "agent_session_resources",
    "agent_payloads",
    "agent_events",
    "agent_effects",
    "agent_session_heads",
    "agent_messages",
];

/// Unified Plugin Core tables use canonical TEXT identities, physical
/// ownership foreign keys and composite keys. Plugin business data is outside
/// the core database in generation DataRoots.
pub(crate) const CANONICAL_PLUGIN_TABLES: &[&str] = &[
    "plugins",
    "plugin_artifacts",
    "plugin_drafts",
    "plugin_credential_bindings",
    "plugin_grants",
    "plugin_library_state",
    "plugin_mutations",
];

/// Hard ceiling for product-owned SQLite B-trees on one table.
///
/// `PRAGMA index_list` includes UNIQUE auto-indexes, so this is a physical
/// storage/write-amplification budget rather than a count of handwritten
/// `CREATE INDEX` statements. FTS shadow tables are SQLite-owned and excluded.
const MAX_INDEXES_PER_TABLE: usize = 5;

pub(crate) const PRODUCT_TABLES: &[&str] = &[
    "agent_execution_attempts",
    "agent_execution_events",
    "agent_execution_participants",
    "agent_execution_step_dependencies",
    "agent_execution_steps",
    "agent_execution_template_participants",
    "agent_execution_templates",
    "agent_executions",
    "agent_metadata",
    "attachments",
    "channel_inbound_receipts",
    "channel_pairing_codes",
    "channel_pending_prompts",
    "channel_plugins",
    "channel_session_bindings",
    "channel_sessions",
    "channel_users",
    "client_preferences",
    "conversation_execution_links",
    "creation_tasks",
    "creative_studio_agent_proposal_receipts",
    "creative_studio_agent_sessions",
    "creative_studio_projects",
    "creative_studio_template_runs",
    "creative_studio_templates",
    "cron_job_runs",
    "cron_run_reservations",
    "cron_jobs",
    "cs_agent_capability_receipts",
    "cs_agents",
    "cs_audit_events",
    "cs_channel_bindings",
    "cs_dialogues",
    "cs_handoffs",
    "cs_messages",
    "cs_notes",
    "installation_identity",
    "installation_role_bindings",
    "instance_access_token",
    "knowledge_bases",
    "knowledge_binding_bases",
    "knowledge_bindings",
    "knowledge_entries",
    "knowledge_entry_provenance",
    "knowledge_source_items",
    "knowledge_sources",
    "knowledge_tags",
    "knowledge_tree_operations",
    "mcp_servers",
    "nomi_remote_events",
    "nomi_remote_sessions",
    "nomi_wave1_memory_action_receipts",
    "nomi_wave4_action_receipts",
    "product_agent_selections",
    "oauth_tokens",
    "provider_connections",
    "provider_model_capabilities",
    "provider_models",
    "providers",
    "remote_bindings",
    "requirement_display_sequence",
    "requirement_pre_effect_abandon_guards",
    "requirement_tags",
    "requirements",
    "skill_tags",
    "ssh_hosts",
    "system_settings",
    "tag_settings",
    "terminal_scrollback",
    "terminal_sessions",
    "terminal_turn_admissions",
    "users",
    "webhooks",
    "workshop_assets"
];

/// Business columns that carry a bare canonical UUIDv7 for every populated row.
const UUIDV7_BUSINESS_COLUMNS: &[(&str, &str)] = &[
    ("agent_execution_attempts", "attempt_id"),
    ("agent_execution_participants", "participant_id"),
    ("agent_execution_steps", "step_id"),
    (
        "agent_execution_template_participants",
        "template_participant_id",
    ),
    ("agent_execution_templates", "execution_template_id"),
    ("agent_executions", "execution_id"),
    ("agent_metadata", "agent_id"),
    ("attachments", "attachment_id"),
    ("channel_plugins", "channel_plugin_id"),
    ("channel_pending_prompts", "prompt_id"),
    ("channel_sessions", "channel_session_id"),
    ("channel_users", "channel_user_id"),
    ("creation_tasks", "creation_task_id"),
    ("creative_studio_agent_sessions", "session_id"),
    ("creative_studio_projects", "project_id"),
    ("creative_studio_template_runs", "template_run_id"),
    ("creative_studio_templates", "template_id"),
    ("cron_job_runs", "cron_job_run_id"),
    ("cron_run_reservations", "cron_job_run_id"),
    ("cron_jobs", "cron_job_id"),
    (
        "cs_agent_capability_receipts",
        "cs_agent_capability_receipt_id",
    ),
    ("cs_agents", "cs_agent_id"),
    ("cs_dialogues", "cs_dialogue_id"),
    ("cs_handoffs", "cs_handoff_id"),
    ("cs_messages", "cs_message_id"),
    ("cs_notes", "cs_note_id"),
    ("knowledge_bases", "knowledge_base_id"),
    ("knowledge_bindings", "knowledge_binding_id"),
    ("knowledge_entries", "knowledge_entry_id"),
    ("knowledge_source_items", "knowledge_source_item_id"),
    ("knowledge_sources", "knowledge_source_id"),
    ("knowledge_tree_operations", "operation_id"),
    ("mcp_servers", "mcp_server_id"),
    ("plugins", "plugin_id"),
    ("plugin_drafts", "draft_id"),
    ("plugin_mutations", "mutation_id"),
    ("nomi_remote_events", "event_id"),
    ("nomi_remote_sessions", "agent_session_id"),
    ("remote_bindings", "remote_binding_id"),
    ("provider_connections", "connection_id"),
    ("providers", "provider_id"),
    ("requirements", "requirement_id"),
    ("ssh_hosts", "ssh_host_id"),
    ("terminal_sessions", "terminal_id"),
    ("terminal_turn_admissions", "turn_token"),
    ("users", "user_id"),
    ("webhooks", "webhook_id"),
    ("workshop_assets", "asset_id")
];

/// Canonical UUIDv7 values owned by a managed side store rather than a
/// relational entity row in SQLite.
const UUIDV7_MANAGED_VALUE_COLUMNS: &[(&str, &str)] = &[
    ("creation_tasks", "node_id"),
    ("creation_tasks", "template_step_id"),
];

/// `_id` columns that are identities, operation tokens, platform handles, or
/// opaque remote handles rather than relational links. Every other physical
/// `_id` column must be present in [`LOGICAL_REFERENCES`].
const NON_REFERENCE_ID_COLUMNS: &[(&str, &str)] = &[
    ("installation_role_bindings", "role_id"),
    ("agent_metadata", "agent_id"),
    ("agent_metadata", "yolo_id"),
    ("agent_execution_attempts", "attempt_id"),
    ("agent_execution_participants", "participant_id"),
    ("agent_execution_steps", "step_id"),
    (
        "agent_execution_template_participants",
        "template_participant_id",
    ),
    ("agent_execution_templates", "execution_template_id"),
    ("agent_executions", "execution_id"),
    ("attachments", "attachment_id"),
    ("channel_inbound_receipts", "chat_id"),
    ("channel_inbound_receipts", "channel_plugin_scope_id"),
    ("channel_inbound_receipts", "conversation_scope_id"),
    ("channel_inbound_receipts", "message_scope_id"),
    ("channel_inbound_receipts", "provider_event_id"),
    ("channel_inbound_receipts", "user_scope_id"),
    ("channel_pairing_codes", "platform_user_id"),
    ("channel_pending_prompts", "prompt_id"),
    ("channel_pending_prompts", "chat_id"),
    ("channel_plugins", "channel_plugin_id"),
    ("channel_session_bindings", "chat_id"),
    ("channel_sessions", "channel_session_id"),
    ("channel_sessions", "chat_id"),
    ("channel_users", "channel_user_id"),
    ("channel_users", "platform_user_id"),
    ("cron_job_runs", "cron_job_run_id"),
    ("cron_run_reservations", "cron_job_run_id"),
    ("cron_jobs", "cron_job_id"),
    ("creation_tasks", "creation_task_id"),
    ("creation_tasks", "node_id"),
    ("creation_tasks", "remote_task_id"),
    ("creation_tasks", "template_step_id"),
    ("creative_studio_agent_sessions", "session_id"),
    ("creative_studio_projects", "project_id"),
    ("creative_studio_template_runs", "template_run_id"),
    ("creative_studio_templates", "template_id"),
    (
        "cs_agent_capability_receipts",
        "cs_agent_capability_receipt_id",
    ),
    ("cs_agent_capability_receipts", "capability_id"),
    ("cs_agent_capability_receipts", "owner_user_id"),
    ("cs_agent_capability_receipts", "cs_agent_id"),
    ("cs_agents", "cs_agent_id"),
    ("cs_dialogues", "cs_dialogue_id"),
    ("cs_dialogues", "chat_id"),
    ("cs_handoffs", "cs_handoff_id"),
    ("cs_handoffs", "cs_agent_id"),
    ("cs_handoffs", "cs_dialogue_id"),
    ("cs_handoffs", "requested_by"),
    ("cs_handoffs", "claimed_by"),
    ("cs_handoffs", "updated_by"),
    ("cs_messages", "cs_message_id"),
    ("cs_notes", "cs_note_id"),
    ("knowledge_bases", "knowledge_base_id"),
    ("knowledge_bindings", "knowledge_binding_id"),
    ("knowledge_entries", "knowledge_entry_id"),
    ("knowledge_source_items", "knowledge_source_item_id"),
    ("knowledge_sources", "knowledge_source_id"),
    ("knowledge_tree_operations", "operation_id"),
    ("knowledge_tree_operations", "request_id"),
    ("mcp_servers", "mcp_server_id"),
    ("plugins", "plugin_id"),
    ("plugins", "package_id"),
    ("plugin_drafts", "draft_id"),
    ("plugin_mutations", "mutation_id"),
    ("nomi_wave1_memory_action_receipts", "capability_id"),
    ("nomi_wave1_memory_action_receipts", "process_lease_id"),
    ("nomi_wave4_action_receipts", "capability_id"),
    ("nomi_wave4_action_receipts", "process_lease_id"),
    ("product_agent_selections", "target_id"),
    ("nomi_remote_events", "event_id"),
    ("plugin_artifacts", "package_id"),
    ("remote_bindings", "remote_binding_id"),
    ("provider_connections", "connection_id"),
    ("providers", "provider_id"),
    ("requirements", "requirement_id"),
    ("ssh_hosts", "ssh_host_id"),
    ("terminal_sessions", "terminal_id"),
    ("terminal_turn_admissions", "turn_token"),
    ("users", "user_id"),
    ("webhooks", "webhook_id"),
    ("workshop_assets", "asset_id")
];

const PARTIAL_UNIQUE_INDEXES: &[PartialUniqueIndexContract] = &[
    PartialUniqueIndexContract {
        index_name: "uq_requirements_active_conversation_owner",
        table: "requirements",
        columns: &["owner_conversation_id"],
        predicate: "status = 'in_progress' AND owner_conversation_id IS NOT NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_requirements_active_terminal_owner",
        table: "requirements",
        columns: &["owner_terminal_id"],
        predicate: "status = 'in_progress' AND owner_terminal_id IS NOT NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_entries_live_rel_path",
        table: "knowledge_entries",
        columns: &["knowledge_base_id", "rel_path"],
        predicate: "deleted_at IS NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_entries_live_portable_path",
        table: "knowledge_entries",
        columns: &["knowledge_base_id", "portable_rel_path"],
        predicate: "deleted_at IS NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_sources_live_kind",
        table: "knowledge_sources",
        columns: &["knowledge_base_id", "kind"],
        predicate: "state <> 'removed'",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_source_items_live_normalized_url",
        table: "knowledge_source_items",
        columns: &["knowledge_source_id", "normalized_url"],
        predicate: "state <> 'removed'",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_source_items_live_ordinal",
        table: "knowledge_source_items",
        columns: &["knowledge_source_id", "ordinal"],
        predicate: "state <> 'removed'",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_entry_provenance_managed_source_item",
        table: "knowledge_entry_provenance",
        columns: &["knowledge_source_item_id"],
        predicate: "relationship = 'managed'",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_bindings_target_workpath",
        table: "knowledge_bindings",
        columns: &["target_workpath"],
        predicate: "target_kind = 'workpath' AND target_workpath IS NOT NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_bindings_target_conversation_id",
        table: "knowledge_bindings",
        columns: &["target_conversation_id"],
        predicate: "target_kind = 'conversation' AND target_conversation_id IS NOT NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_bindings_target_terminal_id",
        table: "knowledge_bindings",
        columns: &["target_terminal_id"],
        predicate: "target_kind = 'terminal' AND target_terminal_id IS NOT NULL",
    },
    PartialUniqueIndexContract {
        index_name: "uq_knowledge_bindings_target_companion_id",
        table: "knowledge_bindings",
        columns: &["target_companion_id"],
        predicate: "target_kind = 'companion' AND target_companion_id IS NOT NULL",
    },
];

macro_rules! unique_key {
    ($table:literal, $($column:literal),+ $(,)?) => {
        UniqueKeyContract {
            table: $table,
            columns: &[$($column),+],
        }
    };
}

/// Exact non-partial keys named by production `ON CONFLICT(column, ...)`
/// clauses. These are write semantics, not optional read accelerators: SQLite
/// rejects the statement at prepare time when the matching UNIQUE key is
/// absent. Keep this registry separate from the physical index budget so an
/// index-pruning pass cannot silently turn a valid upsert into a runtime 500.
const UPSERT_CONFLICT_KEYS: &[UniqueKeyContract] = &[
    unique_key!("agent_bindings", "target_kind", "target_id"),
    unique_key!("agent_messages", "session_id", "projection_id"),
    unique_key!("agent_metadata", "agent_id"),
    unique_key!("channel_inbound_receipts", "operation_key"),
    unique_key!(
        "channel_users",
        "platform_user_id",
        "platform_type",
        "channel_plugin_id"
    ),
    unique_key!("client_preferences", "key"),
    unique_key!("creation_tasks", "creation_task_id"),
    unique_key!(
        "creative_studio_agent_sessions",
        "owner_id",
        "project_id",
        "session_id"
    ),
    unique_key!("creative_studio_template_runs", "template_run_id"),
    unique_key!(
        "cs_dialogues",
        "channel_plugin_id",
        "channel_user_id",
        "chat_id"
    ),
    unique_key!("installation_role_bindings", "role_id"),
    unique_key!("instance_access_token", "singleton_key"),
    unique_key!("knowledge_entries", "knowledge_entry_id"),
    unique_key!(
        "knowledge_tree_operations",
        "knowledge_base_id",
        "request_id"
    ),
    unique_key!("oauth_tokens", "server_url"),
    unique_key!("plugin_artifacts", "artifact_digest"),
    unique_key!("plugins", "owner_user_id", "package_id"),
    unique_key!("plugin_credential_bindings", "owner_user_id", "plugin_id", "slot"),
    unique_key!("plugin_grants", "owner_user_id", "plugin_id", "permission"),
    unique_key!("plugin_library_state", "owner_user_id", "plugin_id"),
    unique_key!("plugin_mutations", "owner_user_id", "plugin_id"),
    unique_key!(
        "product_agent_selections",
        "owner_user_id",
        "target_kind",
        "target_id"
    ),
    unique_key!("provider_connections", "provider_id", "role"),
    unique_key!(
        "provider_model_capabilities",
        "provider_id",
        "model",
        "task"
    ),
    unique_key!("provider_models", "provider_id", "model"),
    unique_key!("requirement_tags", "tag"),
    unique_key!("skill_tags", "skill_name"),
    unique_key!("system_settings", "singleton_key"),
    unique_key!("tag_settings", "tag"),
    unique_key!("terminal_scrollback", "terminal_id"),
    unique_key!(
        "terminal_turn_admissions",
        "terminal_id",
        "pty_epoch",
        "requirement_id",
        "claim_generation"
    ),
];

#[derive(Clone, Copy, Debug)]
struct UniqueKeyContract {
    table: &'static str,
    columns: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
struct PartialUniqueIndexContract {
    index_name: &'static str,
    table: &'static str,
    columns: &'static [&'static str],
    predicate: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LogicalReferenceKind {
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LogicalReferenceValueContract {
    Opaque,
    CanonicalUuidV7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeletePolicy {
    Restrict,
    Cascade,
    SetNull,
    KeepHistory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RebuildPolicy {
    PreserveBusinessId,
    ExternalOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrphanAuditPolicy {
    /// A live child must resolve to a valid parent.
    RequireParent,
    /// Historical rows intentionally retain the former parent value after the
    /// parent is deleted. Existing parents must still satisfy scope rules.
    AllowMissingHistoricalParent,
    /// The parent belongs to another store and cannot be audited by SQLite.
    ExternalOwner,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LogicalReference {
    pub child_table: &'static str,
    pub child_column: &'static str,
    pub parent_table: Option<&'static str>,
    pub parent_column: Option<&'static str>,
    pub kind: LogicalReferenceKind,
    pub value_contract: LogicalReferenceValueContract,
    pub nullable: bool,
    pub delete_policy: DeletePolicy,
    pub rebuild_policy: RebuildPolicy,
    pub orphan_audit_policy: OrphanAuditPolicy,
    /// Optional child predicate for polymorphic columns.
    pub child_predicate: Option<&'static str>,
    /// Optional sibling-row predicate that makes an immutable projection
    /// authoritative when the relational parent no longer exists.
    pub frozen_projection_authority: Option<&'static str>,
    /// Optional parent predicate for references to live rows in a soft-delete
    /// table. Expressions use the `parent` SQL alias.
    pub parent_predicate: Option<&'static str>,
    /// Optional aggregate-scope predicate. Expressions use the `child` and
    /// `parent` SQL aliases after the reference values have matched.
    pub aggregate_scope_predicate: Option<&'static str>,
}

impl LogicalReference {
    const fn with_orphan_audit_policy(mut self, policy: OrphanAuditPolicy) -> Self {
        self.orphan_audit_policy = policy;
        self
    }

    const fn with_child_predicate(mut self, predicate: &'static str) -> Self {
        self.child_predicate = Some(predicate);
        self
    }

    const fn with_frozen_projection_authority(mut self, predicate: &'static str) -> Self {
        self.frozen_projection_authority = Some(predicate);
        self
    }

    const fn with_parent_predicate(mut self, predicate: &'static str) -> Self {
        self.parent_predicate = Some(predicate);
        self
    }

    const fn with_aggregate_scope(mut self, predicate: &'static str) -> Self {
        self.aggregate_scope_predicate = Some(predicate);
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct JsonLogicalReference {
    pub child_table: &'static str,
    pub child_column: &'static str,
    pub json_path: &'static str,
    pub value_sql: &'static str,
    pub parent_table: Option<&'static str>,
    pub parent_column: Option<&'static str>,
    pub value_contract: LogicalReferenceValueContract,
    pub delete_policy: DeletePolicy,
    pub rebuild_policy: RebuildPolicy,
    pub orphan_audit_policy: OrphanAuditPolicy,
}

macro_rules! json_text_ref {
    ($table:literal, $column:literal, $path:literal, $sql:literal =>
     $parent_table:literal, $parent_column:literal, $delete:ident,
     $audit:ident) => {
        JsonLogicalReference {
            child_table: $table,
            child_column: $column,
            json_path: $path,
            value_sql: $sql,
            parent_table: Some($parent_table),
            parent_column: Some($parent_column),
            value_contract: LogicalReferenceValueContract::CanonicalUuidV7,
            delete_policy: DeletePolicy::$delete,
            rebuild_policy: RebuildPolicy::PreserveBusinessId,
            orphan_audit_policy: OrphanAuditPolicy::$audit,
        }
    };
}

macro_rules! json_external_ref {
    ($table:literal, $column:literal, $path:literal, $sql:literal, $delete:ident) => {
        JsonLogicalReference {
            child_table: $table,
            child_column: $column,
            json_path: $path,
            value_sql: $sql,
            parent_table: None,
            parent_column: None,
            // Cross-store ownership prevents a SQLite parent-existence check,
            // but the identifier itself is still a NomiFun business ID and
            // must remain a canonical bare UUIDv7.
            value_contract: LogicalReferenceValueContract::CanonicalUuidV7,
            delete_policy: DeletePolicy::$delete,
            rebuild_policy: RebuildPolicy::ExternalOwner,
            orphan_audit_policy: OrphanAuditPolicy::ExternalOwner,
        }
    };
}

const fn default_orphan_audit_policy(delete_policy: DeletePolicy) -> OrphanAuditPolicy {
    match delete_policy {
        DeletePolicy::KeepHistory => OrphanAuditPolicy::AllowMissingHistoricalParent,
        DeletePolicy::Restrict | DeletePolicy::Cascade | DeletePolicy::SetNull => {
            OrphanAuditPolicy::RequireParent
        }
    }
}

macro_rules! text_ref {
    ($child_table:literal, $child_column:literal => $parent_table:literal, $parent_column:literal,
     $nullable:expr, $delete:ident) => {
        LogicalReference {
            child_table: $child_table,
            child_column: $child_column,
            parent_table: Some($parent_table),
            parent_column: Some($parent_column),
            kind: LogicalReferenceKind::Text,
            value_contract: LogicalReferenceValueContract::CanonicalUuidV7,
            nullable: $nullable,
            delete_policy: DeletePolicy::$delete,
            rebuild_policy: RebuildPolicy::PreserveBusinessId,
            orphan_audit_policy: default_orphan_audit_policy(DeletePolicy::$delete),
            child_predicate: None,
            frozen_projection_authority: None,
            parent_predicate: None,
            aggregate_scope_predicate: None,
        }
    };
}

macro_rules! opaque_text_ref {
    ($child_table:literal, $child_column:literal => $parent_table:literal, $parent_column:literal,
     $nullable:expr, $delete:ident) => {
        LogicalReference {
            child_table: $child_table,
            child_column: $child_column,
            parent_table: Some($parent_table),
            parent_column: Some($parent_column),
            kind: LogicalReferenceKind::Text,
            value_contract: LogicalReferenceValueContract::Opaque,
            nullable: $nullable,
            delete_policy: DeletePolicy::$delete,
            rebuild_policy: RebuildPolicy::PreserveBusinessId,
            orphan_audit_policy: default_orphan_audit_policy(DeletePolicy::$delete),
            child_predicate: None,
            frozen_projection_authority: None,
            parent_predicate: None,
            aggregate_scope_predicate: None,
        }
    };
}

macro_rules! external_ref {
    ($child_table:literal, $child_column:literal, $kind:ident, $nullable:expr,
     $value_contract:ident, $delete:ident) => {
        LogicalReference {
            child_table: $child_table,
            child_column: $child_column,
            parent_table: None,
            parent_column: None,
            kind: LogicalReferenceKind::$kind,
            value_contract: LogicalReferenceValueContract::$value_contract,
            nullable: $nullable,
            delete_policy: DeletePolicy::$delete,
            rebuild_policy: RebuildPolicy::ExternalOwner,
            orphan_audit_policy: OrphanAuditPolicy::ExternalOwner,
            child_predicate: None,
            frozen_projection_authority: None,
            parent_predicate: None,
            aggregate_scope_predicate: None,
        }
    };
}

/// Database and cross-store links owned by the application. Every entry names
/// its delete and restore/clone policy. Parentless entries are deliberate
/// cross-store references; the database audit reports them as externally owned
/// instead of pretending SQLite can verify them. Indexes are deliberately not
/// part of this registry: relationship integrity and physical access paths are
/// independent concerns.
pub(crate) const LOGICAL_REFERENCES: &[LogicalReference] = &[
    external_ref!("installation_role_bindings", "provider_mount_id", Text, false, Opaque, KeepHistory),
    text_ref!("terminal_sessions", "user_id" => "users", "user_id", false, Cascade),
    text_ref!("ssh_hosts", "user_id" => "users", "user_id", false, Cascade),
    text_ref!("plugins", "owner_user_id" => "users", "user_id", false, Cascade),
    opaque_text_ref!("plugins", "active_artifact_digest" => "plugin_artifacts", "artifact_digest", false, Restrict),
    opaque_text_ref!("plugins", "previous_artifact_digest" => "plugin_artifacts", "artifact_digest", true, Restrict),
    text_ref!("plugin_drafts", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("plugin_drafts", "plugin_id" => "plugins", "plugin_id", true, SetNull)
        .with_aggregate_scope("parent.owner_user_id = child.owner_user_id"),
    text_ref!("plugin_credential_bindings", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("plugin_credential_bindings", "plugin_id" => "plugins", "plugin_id", false, Cascade)
        .with_aggregate_scope("parent.owner_user_id = child.owner_user_id"),
    external_ref!("plugin_credential_bindings", "credential_id", Text, false, Opaque, KeepHistory),
    text_ref!("plugin_grants", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("plugin_grants", "plugin_id" => "plugins", "plugin_id", false, Cascade)
        .with_aggregate_scope("parent.owner_user_id = child.owner_user_id"),
    opaque_text_ref!("plugin_grants", "confirmed_artifact_digest" => "plugin_artifacts", "artifact_digest", false, KeepHistory),
    text_ref!("plugin_library_state", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("plugin_library_state", "plugin_id" => "plugins", "plugin_id", false, Cascade)
        .with_aggregate_scope("parent.owner_user_id = child.owner_user_id"),
    text_ref!("plugin_mutations", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("plugin_mutations", "plugin_id" => "plugins", "plugin_id", false, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    opaque_text_ref!("plugin_mutations", "old_artifact_digest" => "plugin_artifacts", "artifact_digest", true, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    opaque_text_ref!("plugin_mutations", "old_previous_artifact_digest" => "plugin_artifacts", "artifact_digest", true, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    opaque_text_ref!("plugin_mutations", "new_artifact_digest" => "plugin_artifacts", "artifact_digest", true, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),    // Delivery receipts intentionally survive Terminal/Requirement deletion so
    // a replay can never regain PTY write authority.
    text_ref!("terminal_turn_admissions", "terminal_id" => "terminal_sessions", "terminal_id", false, KeepHistory),
    text_ref!("terminal_turn_admissions", "requirement_id" => "requirements", "requirement_id", false, KeepHistory),
    text_ref!("agent_execution_templates", "user_id" => "users", "user_id", false, Cascade),
    text_ref!("agent_execution_templates", "primary_participant_id" => "agent_execution_template_participants", "template_participant_id", false, Restrict)
        .with_aggregate_scope("parent.template_id = child.execution_template_id"),
    text_ref!("agent_executions", "user_id" => "users", "user_id", false, Cascade),
    text_ref!("attachments", "requirement_id" => "requirements", "requirement_id", false, Cascade),
    text_ref!("channel_inbound_receipts", "user_id" => "users", "user_id", true, SetNull),
    text_ref!("channel_inbound_receipts", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", true, SetNull),
    text_ref!("channel_inbound_receipts", "conversation_id" => "agent_sessions", "agent_session_id", true, KeepHistory),
    text_ref!("channel_inbound_receipts", "message_id" => "agent_events", "event_id", true, KeepHistory),
    text_ref!("channel_session_bindings", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", false, Cascade),
    text_ref!("channel_session_bindings", "channel_user_id" => "channel_users", "channel_user_id", false, Cascade),
    text_ref!("channel_session_bindings", "channel_session_id" => "channel_sessions", "channel_session_id", false, Cascade),
    // Busy-time prompt queue rows (spec D1) are short-lived operational
    // records; settled rows keep their historical scope even after the bot,
    // session, or conversation is deleted.
    text_ref!("channel_pending_prompts", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", false, KeepHistory),
    text_ref!("channel_pending_prompts", "channel_session_id" => "channel_sessions", "channel_session_id", false, KeepHistory),
    text_ref!("channel_pending_prompts", "conversation_id" => "agent_sessions", "agent_session_id", false, KeepHistory),
    text_ref!("channel_sessions", "channel_user_id" => "channel_users", "channel_user_id", false, Cascade),
    text_ref!("channel_sessions", "conversation_id" => "agent_sessions", "agent_session_id", true, SetNull),
    text_ref!("channel_sessions", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", true, SetNull),
    text_ref!("agent_execution_participants", "execution_id" => "agent_executions", "execution_id", false, Cascade),
    text_ref!("agent_execution_participants", "source_agent_id" => "agent_metadata", "agent_id", false, KeepHistory),
    text_ref!("agent_execution_participants", "preset_id" => "agent_presets", "preset_id", true, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::RequireParent)
        .with_frozen_projection_authority("child.agent_snapshot IS NOT NULL"),
    text_ref!("agent_execution_participants", "provider_id" => "providers", "provider_id", true, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::RequireParent)
        .with_child_predicate(
            "child.retired_in_revision IS NULL \
             AND EXISTS (\
                 SELECT 1 FROM agent_executions execution \
                 WHERE execution.execution_id = child.execution_id \
                   AND execution.status <> 'cancelled' \
                   AND execution.deleted_at IS NULL\
             )",
        ),
    text_ref!("agent_execution_steps", "execution_id" => "agent_executions", "execution_id", false, Cascade),
    text_ref!("agent_execution_steps", "assigned_participant_id" => "agent_execution_participants", "participant_id", true, Restrict)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("agent_execution_attempts", "execution_id" => "agent_executions", "execution_id", false, Cascade),
    text_ref!("agent_execution_attempts", "step_id" => "agent_execution_steps", "step_id", false, Cascade)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("agent_execution_attempts", "participant_id" => "agent_execution_participants", "participant_id", true, KeepHistory)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("agent_execution_events", "execution_id" => "agent_executions", "execution_id", false, Cascade),
    text_ref!("agent_execution_events", "step_id" => "agent_execution_steps", "step_id", true, KeepHistory)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("agent_execution_events", "attempt_id" => "agent_execution_attempts", "attempt_id", true, KeepHistory)
        .with_aggregate_scope(
            "parent.execution_id = child.execution_id AND parent.step_id = child.step_id",
        ),
    text_ref!("agent_execution_events", "actor_id" => "users", "user_id", true, KeepHistory)
        .with_child_predicate("child.actor_type = 'user'"),
    text_ref!("agent_execution_events", "actor_id" => "agent_sessions", "agent_session_id", true, KeepHistory)
        .with_child_predicate(
            "child.actor_type = 'agent' AND child.actor_conversation_id IS NOT NULL",
        )
        .with_aggregate_scope("parent.agent_session_id = child.actor_conversation_id"),
    external_ref!(
        "agent_execution_events",
        "actor_id",
        Text,
        true,
        CanonicalUuidV7,
        KeepHistory
    )
    .with_child_predicate(
        "child.actor_type = 'agent' \
         AND child.actor_conversation_id IS NULL \
         AND child.actor_id IS NOT NULL",
    ),
    text_ref!("agent_execution_events", "actor_conversation_id" => "agent_sessions", "agent_session_id", true, KeepHistory),
    text_ref!("agent_execution_events", "actor_attempt_id" => "agent_execution_attempts", "attempt_id", true, KeepHistory)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("agent_execution_events", "on_behalf_of_user_id" => "users", "user_id", false, KeepHistory),
    text_ref!("agent_execution_template_participants", "template_id" => "agent_execution_templates", "execution_template_id", false, Cascade),
    text_ref!("agent_execution_template_participants", "source_agent_id" => "agent_metadata", "agent_id", false, Restrict),
    text_ref!("agent_execution_template_participants", "preset_id" => "agent_presets", "preset_id", true, SetNull)
        .with_frozen_projection_authority("child.agent_snapshot IS NOT NULL"),
    text_ref!("agent_execution_template_participants", "provider_id" => "providers", "provider_id", true, Restrict),
    text_ref!("conversation_execution_links", "conversation_id" => "agent_sessions", "agent_session_id", false, KeepHistory),
    text_ref!("conversation_execution_links", "execution_id" => "agent_executions", "execution_id", false, Cascade),
    text_ref!("conversation_execution_links", "step_id" => "agent_execution_steps", "step_id", true, KeepHistory)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("conversation_execution_links", "attempt_id" => "agent_execution_attempts", "attempt_id", true, KeepHistory)
        .with_aggregate_scope(
            "parent.execution_id = child.execution_id AND parent.step_id = child.step_id",
        ),
    text_ref!("cron_jobs", "user_id" => "users", "user_id", false, Cascade),
    text_ref!("cron_jobs", "preset_id" => "agent_presets", "preset_id", true, SetNull)
        .with_frozen_projection_authority("child.agent_snapshot IS NOT NULL"),
    text_ref!("cron_jobs", "conversation_id" => "agent_sessions", "agent_session_id", true, Cascade),
    text_ref!("cron_job_runs", "cron_job_id" => "cron_jobs", "cron_job_id", false, Cascade),
    text_ref!("cron_run_reservations", "cron_job_id" => "cron_jobs", "cron_job_id", false, Cascade),
    text_ref!("cron_run_reservations", "conversation_id" => "agent_sessions", "agent_session_id", true, SetNull),
    external_ref!("channel_plugins", "companion_id", Text, true, CanonicalUuidV7, SetNull),
    text_ref!("channel_users", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", true, Cascade),
    // ── customer-service domain (015) ────────────────────────────────
    // Provider/KB references keep history on parent deletion: the runtime
    // resolves them per turn and degrades gracefully to "model/KB missing".
    text_ref!("cs_agents", "provider_id" => "providers", "provider_id", true, KeepHistory),
    text_ref!("cs_agent_capability_receipts", "owner_user_id" => "users", "user_id", false, KeepHistory),
    text_ref!("cs_agent_capability_receipts", "cs_agent_id" => "cs_agents", "cs_agent_id", false, Cascade),
    text_ref!("cs_channel_bindings", "cs_agent_id" => "cs_agents", "cs_agent_id", false, Cascade),
    text_ref!("cs_channel_bindings", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", false, Cascade),
    text_ref!("cs_dialogues", "cs_agent_id" => "cs_agents", "cs_agent_id", false, Cascade),
    // A dialogue transcript survives bot/visitor deletion as history.
    text_ref!("cs_dialogues", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", false, KeepHistory),
    text_ref!("cs_dialogues", "channel_user_id" => "channel_users", "channel_user_id", false, KeepHistory),
    text_ref!("cs_messages", "cs_dialogue_id" => "cs_dialogues", "cs_dialogue_id", false, Cascade),
    text_ref!("cs_handoffs", "cs_agent_id" => "cs_agents", "cs_agent_id", false, Cascade),
    text_ref!("cs_handoffs", "cs_dialogue_id" => "cs_dialogues", "cs_dialogue_id", false, Cascade),
    text_ref!("cs_handoffs", "requested_by" => "users", "user_id", false, KeepHistory),
    text_ref!("cs_handoffs", "claimed_by" => "users", "user_id", true, KeepHistory),
    text_ref!("cs_handoffs", "updated_by" => "users", "user_id", false, KeepHistory),
    text_ref!("cs_notes", "cs_agent_id" => "cs_agents", "cs_agent_id", true, Cascade),
    // Audit events are retained after the agent is deleted; retention-days
    // cleanup is the only pruning authority.
    text_ref!("cs_audit_events", "cs_agent_id" => "cs_agents", "cs_agent_id", false, KeepHistory),
    // Canonical Creative Studio task history survives project deletion, while
    // creation itself still locks and validates a live project row.
    text_ref!("creation_tasks", "project_id" => "creative_studio_projects", "project_id", true, KeepHistory),
    text_ref!("creation_tasks", "conversation_id" => "agent_sessions", "agent_session_id", true, KeepHistory),
    text_ref!("creation_tasks", "message_id" => "agent_events", "event_id", true, KeepHistory)
        .with_aggregate_scope("parent.session_id = child.conversation_id"),
    text_ref!("creation_tasks", "template_id" => "creative_studio_templates", "template_id", true, KeepHistory),
    text_ref!("creation_tasks", "template_run_id" => "creative_studio_template_runs", "template_run_id", true, KeepHistory)
        .with_aggregate_scope("parent.template_id = child.template_id"),
    text_ref!("creation_tasks", "provider_id" => "providers", "provider_id", false, Restrict),
    text_ref!("creative_studio_template_runs", "template_id" => "creative_studio_templates", "template_id", false, KeepHistory),
    text_ref!("creative_studio_agent_proposal_receipts", "project_id" => "creative_studio_projects", "project_id", false, Cascade),
    text_ref!("creative_studio_agent_proposal_receipts", "assistant_message_id" => "agent_events", "event_id", false, Restrict),
    text_ref!("creative_studio_agent_sessions", "owner_id" => "users", "user_id", false, Restrict),
    text_ref!("creative_studio_agent_sessions", "project_id" => "creative_studio_projects", "project_id", false, Restrict),
    text_ref!("creative_studio_agent_sessions", "conversation_id" => "agent_sessions", "agent_session_id", false, Restrict)
        .with_aggregate_scope("json_extract(parent.owner_ref_json, '$.principal_id') = child.owner_id"),
    // Inactive Requirements follow SET_NULL when their aggregate is deleted.
    // Active/NeedsReview rows deliberately retain the typed owner as immutable
    // execution-history evidence after the parent is gone, so the live-parent
    // orphan audit applies only to rows for which deletion must clear it.
    text_ref!("requirements", "owner_conversation_id" => "agent_sessions", "agent_session_id", true, SetNull)
        .with_child_predicate("child.status NOT IN ('in_progress', 'needs_review')"),
    text_ref!("requirements", "owner_terminal_id" => "terminal_sessions", "terminal_id", true, SetNull)
        .with_child_predicate("child.status NOT IN ('in_progress', 'needs_review')"),
    text_ref!("requirement_pre_effect_abandon_guards", "requirement_id" => "requirements", "requirement_id", false, Restrict),
    text_ref!("requirement_pre_effect_abandon_guards", "owner_conversation_id" => "agent_sessions", "agent_session_id", true, Restrict),
    text_ref!("requirement_pre_effect_abandon_guards", "owner_terminal_id" => "terminal_sessions", "terminal_id", true, Restrict),
    external_ref!("knowledge_bindings", "target_workpath", Text, true, Opaque, Cascade),
    text_ref!("knowledge_bindings", "target_conversation_id" => "agent_sessions", "agent_session_id", true, Cascade)
        .with_child_predicate("child.target_kind = 'conversation'"),
    text_ref!("knowledge_bindings", "target_terminal_id" => "terminal_sessions", "terminal_id", true, Cascade)
        .with_child_predicate("child.target_kind = 'terminal'"),
    external_ref!("knowledge_bindings", "target_companion_id", Text, true, CanonicalUuidV7, Cascade),
    text_ref!("agent_execution_step_dependencies", "execution_id" => "agent_executions", "execution_id", false, Cascade),
    text_ref!("agent_execution_step_dependencies", "blocker_step_id" => "agent_execution_steps", "step_id", false, Cascade)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("agent_execution_step_dependencies", "blocked_step_id" => "agent_execution_steps", "step_id", false, Cascade)
        .with_aggregate_scope("parent.execution_id = child.execution_id"),
    text_ref!("channel_pairing_codes", "channel_plugin_id" => "channel_plugins", "channel_plugin_id", true, Cascade),
    text_ref!("knowledge_binding_bases", "knowledge_binding_id" => "knowledge_bindings", "knowledge_binding_id", false, Cascade),
    text_ref!("knowledge_binding_bases", "knowledge_base_id" => "knowledge_bases", "knowledge_base_id", false, Cascade),
    text_ref!("knowledge_entries", "knowledge_base_id" => "knowledge_bases", "knowledge_base_id", false, Cascade),
    text_ref!("knowledge_entries", "parent_entry_id" => "knowledge_entries", "knowledge_entry_id", true, Cascade)
        .with_child_predicate("child.deleted_at IS NULL")
        .with_parent_predicate("parent.deleted_at IS NULL")
        .with_aggregate_scope("parent.knowledge_base_id = child.knowledge_base_id AND parent.kind = 'directory'"),
    text_ref!("knowledge_sources", "knowledge_base_id" => "knowledge_bases", "knowledge_base_id", false, Cascade),
    text_ref!("knowledge_sources", "default_parent_entry_id" => "knowledge_entries", "knowledge_entry_id", true, SetNull)
        .with_parent_predicate("parent.deleted_at IS NULL")
        .with_aggregate_scope("parent.knowledge_base_id = child.knowledge_base_id AND parent.kind = 'directory'"),
    text_ref!("knowledge_source_items", "knowledge_source_id" => "knowledge_sources", "knowledge_source_id", false, Cascade),
    text_ref!("knowledge_entry_provenance", "knowledge_entry_id" => "knowledge_entries", "knowledge_entry_id", false, KeepHistory),
    text_ref!("knowledge_entry_provenance", "knowledge_source_item_id" => "knowledge_source_items", "knowledge_source_item_id", false, Cascade),
    text_ref!("knowledge_entry_provenance", "derived_from_entry_id" => "knowledge_entries", "knowledge_entry_id", true, KeepHistory),
    text_ref!("knowledge_tree_operations", "knowledge_base_id" => "knowledge_bases", "knowledge_base_id", false, Cascade),
    text_ref!("provider_connections", "provider_id" => "providers", "provider_id", false, Cascade),
    text_ref!("provider_model_capabilities", "provider_id" => "providers", "provider_id", false, Cascade),
    text_ref!("provider_models", "provider_id" => "providers", "provider_id", false, Cascade),
    text_ref!("requirement_tags", "paused_requirement_id" => "requirements", "requirement_id", true, SetNull),
    text_ref!("tag_settings", "webhook_id" => "webhooks", "webhook_id", true, SetNull),
    text_ref!("installation_identity", "owner_user_id" => "users", "user_id", false, Restrict),
    text_ref!("terminal_scrollback", "terminal_id" => "terminal_sessions", "terminal_id", false, Cascade),
    text_ref!("remote_bindings", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("nomi_remote_sessions", "owner_user_id" => "users", "user_id", false, Cascade),
    text_ref!("nomi_remote_sessions", "agent_session_id" => "agent_sessions", "agent_session_id", false, Cascade),
    text_ref!("nomi_remote_sessions", "remote_binding_id" => "remote_bindings", "remote_binding_id", false, KeepHistory)
        .with_aggregate_scope("parent.owner_user_id = child.owner_user_id"),
    text_ref!("nomi_remote_events", "agent_session_id" => "nomi_remote_sessions", "agent_session_id", false, Cascade),
    text_ref!("nomi_wave1_memory_action_receipts", "owner_user_id" => "users", "user_id", false, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    text_ref!("nomi_wave1_memory_action_receipts", "agent_session_id" => "agent_sessions", "agent_session_id", false, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    text_ref!("nomi_wave4_action_receipts", "owner_user_id" => "users", "user_id", false, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    text_ref!("nomi_wave4_action_receipts", "agent_session_id" => "agent_sessions", "agent_session_id", false, KeepHistory)
        .with_orphan_audit_policy(OrphanAuditPolicy::AllowMissingHistoricalParent),
    opaque_text_ref!("agent_preset_revisions", "created_by" => "users", "user_id", false, KeepHistory),
    text_ref!("product_agent_selections", "owner_user_id" => "users", "user_id", false, Cascade),
];

/// Stable JSON paths that carry Provider or business identifiers. The SQL for
/// each entry yields one column named `value`, including one row per array
/// element where necessary.
pub(crate) const JSON_LOGICAL_REFERENCES: &[JsonLogicalReference] = &[
    json_text_ref!(
        "terminal_sessions", "idmm", "$.fault_watch.bypass_model.provider_id",
        "SELECT json_extract(idmm, '$.fault_watch.bypass_model.provider_id') AS value FROM terminal_sessions WHERE idmm IS NOT NULL" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "terminal_sessions", "idmm", "$.decision_watch.bypass_model.provider_id",
        "SELECT json_extract(idmm, '$.decision_watch.bypass_model.provider_id') AS value FROM terminal_sessions WHERE idmm IS NOT NULL" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "cron_jobs", "agent_config", "$.provider_id (agent_type=nomi)",
        "SELECT json_extract(agent_config, '$.provider_id') AS value FROM cron_jobs WHERE agent_type = 'nomi' AND agent_config IS NOT NULL" =>
        "providers", "provider_id", Restrict, RequireParent
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.provider_id",
        "SELECT json_extract(origin, '$.provider_id') AS value FROM workshop_assets WHERE origin IS NOT NULL" =>
        "providers", "provider_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.canvas_id",
        "SELECT json_extract(origin, '$.canvas_id') AS value FROM workshop_assets WHERE json_type(origin, '$.canvas_id') = 'text' AND json_type(origin, '$.node_id') = 'text' AND json_type(origin, '$.project_id') IS NULL" =>
        "creative_studio_projects", "project_id", KeepHistory, AllowMissingHistoricalParent
    ),
    // `project_id` remains a wire/storage compatibility alias only for old
    // Canvas origins.
    json_text_ref!(
        "workshop_assets", "origin", "$.project_id",
        "SELECT json_extract(origin, '$.project_id') AS value FROM workshop_assets WHERE json_type(origin, '$.project_id') = 'text' AND json_type(origin, '$.node_id') = 'text' AND json_type(origin, '$.canvas_id') IS NULL" =>
        "creative_studio_projects", "project_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.conversation_id",
        "SELECT json_extract(origin, '$.conversation_id') AS value FROM workshop_assets WHERE origin IS NOT NULL" =>
        "agent_sessions", "agent_session_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.message_id",
        "SELECT json_extract(origin, '$.message_id') AS value FROM workshop_assets WHERE origin IS NOT NULL" =>
        "agent_events", "event_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.template_id",
        "SELECT json_extract(origin, '$.template_id') AS value FROM workshop_assets WHERE origin IS NOT NULL" =>
        "creative_studio_templates", "template_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.template_run_id",
        "SELECT json_extract(origin, '$.template_run_id') AS value FROM workshop_assets WHERE origin IS NOT NULL" =>
        "creative_studio_template_runs", "template_run_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_external_ref!(
        "workshop_assets", "origin", "$.template_step_id",
        "SELECT json_extract(origin, '$.template_step_id') AS value FROM workshop_assets WHERE origin IS NOT NULL", KeepHistory
    ),
    json_text_ref!(
        "workshop_assets", "origin", "$.creation_task_id",
        "SELECT json_extract(origin, '$.creation_task_id') AS value FROM workshop_assets WHERE origin IS NOT NULL" =>
        "creation_tasks", "creation_task_id", KeepHistory, AllowMissingHistoricalParent
    ),
    json_external_ref!(
        "workshop_assets", "origin", "$.node_id",
        "SELECT json_extract(origin, '$.node_id') AS value FROM workshop_assets WHERE origin IS NOT NULL", KeepHistory
    ),
    json_text_ref!(
        "creation_tasks", "input_bindings", "$[].asset_id",
        "SELECT json_extract(item.value, '$.asset_id') AS value FROM creation_tasks, json_each(creation_tasks.input_bindings) item WHERE creation_tasks.input_bindings IS NOT NULL" =>
        "workshop_assets", "asset_id", Restrict, RequireParent
    ),
    json_text_ref!(
        "creation_tasks", "result_asset_ids", "$[]",
        "SELECT item.value AS value FROM creation_tasks, json_each(creation_tasks.result_asset_ids) item" =>
        "workshop_assets", "asset_id", Restrict, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$.queue[].provider_id",
        "SELECT json_extract(item.value, '$.provider_id') AS value FROM client_preferences preference, json_each(preference.value, '$.queue') item WHERE preference.key = 'agent.model_failover' AND json_valid(preference.value)" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$.config.bypass_model.provider_id (agent_session.idmm.*)",
        "SELECT json_extract(value, '$.config.bypass_model.provider_id') AS value FROM client_preferences WHERE key GLOB 'agent_session.idmm.*' AND json_valid(value) AND json_type(value, '$.config.bypass_model.provider_id') = 'text'" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$.session_id (agent_session.idmm.*)",
        "SELECT json_extract(value, '$.session_id') AS value FROM client_preferences WHERE key GLOB 'agent_session.idmm.*' AND json_valid(value)" =>
        "agent_sessions", "agent_session_id", Cascade, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$[].provider_id",
        "SELECT json_extract(item.value, '$.provider_id') AS value FROM client_preferences preference, json_each(preference.value) item WHERE preference.key = 'nomi.collaborationModels' AND json_valid(preference.value)" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$.provider_id",
        "SELECT json_extract(value, '$.provider_id') AS value FROM client_preferences WHERE (key = 'nomi.defaultModel' OR key = 'knowledge.autogenModel' OR key = 'models.default.imageGeneration' OR key = 'models.default.imageEdit' OR key = 'models.default.vision' OR key = 'models.default.videoGeneration' OR key = 'models.default.musicGeneration' OR key = 'models.default.speechSynthesis' OR key = 'tools.speechToText' OR key = 'tools.textToSpeech' OR key LIKE 'channels.%.defaultModel') AND json_valid(value)" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$.embedding.provider_id",
        "SELECT json_extract(value, '$.embedding.provider_id') AS value FROM client_preferences WHERE key = 'knowledge.retrieval' AND json_valid(value) AND json_extract(value, '$.embedding.mode') = 'remote'" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    json_text_ref!(
        "client_preferences", "value", "$.rerank.provider_id",
        "SELECT json_extract(value, '$.rerank.provider_id') AS value FROM client_preferences WHERE key = 'knowledge.retrieval' AND json_valid(value) AND json_extract(value, '$.rerank.mode') = 'remote'" =>
        "providers", "provider_id", SetNull, RequireParent
    ),
    // Customer-service agents mount knowledge bases by ID; a deleted base
    // simply stops contributing hits, so history is allowed to keep the value.
    json_text_ref!(
        "cs_agents", "knowledge_base_ids", "$[]",
        "SELECT item.value AS value FROM cs_agents, json_each(cs_agents.knowledge_base_ids) item" =>
        "knowledge_bases", "knowledge_base_id", KeepHistory, AllowMissingHistoricalParent
    )
];

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrphanAuditFinding {
    pub child_table: String,
    pub child_column: String,
    pub parent_table: String,
    pub parent_column: String,
    pub count: i64,
    pub delete_policy: &'static str,
    pub rebuild_policy: &'static str,
}

/// Validate the structural v3 invariants and the logical-reference registry.
pub async fn validate_id_schema_contract(pool: &SqlitePool) -> Result<(), DbError> {
    let actual_tables: BTreeSet<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    // The FTS virtual table and its shadow tables belong to the baseline SET
    // (so an unexpected table is still caught) but not to the structural loop
    // below — SQLite owns their shape. See `FTS_SHADOW_TABLES`.
    let expected_tables: BTreeSet<String> = PRODUCT_TABLES
        .iter()
        .chain(std::iter::once(&CS_NOTES_FTS_TABLE))
        .chain(FTS_SHADOW_TABLES.iter())
        .chain(CANONICAL_AGENT_STORE_TABLES.iter())
        .chain(CANONICAL_PLUGIN_TABLES.iter())
        .map(|value| (*value).to_owned())
        .collect();
    if actual_tables != expected_tables {
        let missing = expected_tables.difference(&actual_tables).cloned().collect::<Vec<_>>();
        let extra = actual_tables.difference(&expected_tables).cloned().collect::<Vec<_>>();
        return Err(DbError::Init(format!(
            "v3 schema product-table registry mismatch; missing={missing:?}, extra={extra:?}"
        )));
    }

    for table in PRODUCT_TABLES {
        require_autoincrement_primary_key(pool, table).await?;
    }
    validate_cs_notes_fts_contract(pool).await?;
    validate_no_physical_foreign_keys(pool).await?;
    validate_no_triggers(pool).await?;
    validate_no_row_id_columns(pool).await?;
    for contract in UPSERT_CONFLICT_KEYS {
        require_exact_unique_key(pool, contract.table, contract.columns).await?;
    }
    validate_index_budget(pool).await?;

    validate_business_id_registry(pool).await?;
    require_column(
        pool,
        "creative_studio_agent_proposal_receipts",
        "assistant_message_id",
        "TEXT",
        true,
    )
    .await?;
    require_single_column_unique_index(
        pool,
        "creative_studio_agent_proposal_receipts",
        "assistant_message_id",
    )
    .await?;
    // Channel bot ownership domain (migration 020): every row names its owning
    // domain and defaults to the legacy companion pool.
    require_column(pool, "channel_plugins", "owner_domain", "TEXT", true).await?;
    let owner_domain_default: Option<String> = sqlx::query_scalar(
        "SELECT dflt_value FROM pragma_table_info('channel_plugins') \
         WHERE name = 'owner_domain'",
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    if owner_domain_default.as_deref() != Some("'companion'") {
        return Err(DbError::Init(
            "v3 schema channel_plugins.owner_domain must default to 'companion'".to_owned(),
        ));
    }

    // Group-chat authorization metadata (migration 033) defaults to the
    // backward-compatible, least-privilege interpretation for each row type.
    for (table, column, expected_default) in [
        ("channel_plugins", "group_access_mode", "'allowlist'"),
        ("channel_users", "authorization_kind", "'approved'"),
        ("channel_sessions", "chat_kind", "'unknown'"),
    ] {
        require_column(pool, table, column, "TEXT", true).await?;
        let column_default: Option<String> = sqlx::query_scalar(&format!(
            "SELECT dflt_value FROM pragma_table_info('{table}') WHERE name = ?"
        ))
        .bind(column)
        .fetch_optional(pool)
        .await?
        .flatten();
        if column_default.as_deref() != Some(expected_default) {
            return Err(DbError::Init(format!(
                "v3 schema {table}.{column} must default to {expected_default}"
            )));
        }
    }

    validate_logical_reference_registry(pool).await?;
    validate_logical_reference_coverage(pool).await?;
    validate_json_logical_reference_registry(pool).await?;
    require_workshop_asset_origin_id_contract(pool).await?;
    require_prompt_library_asset_identity_contract(pool).await?;
    require_column(pool, "workshop_assets", "deleted_at", "INTEGER", false).await?;
    require_column(pool, "workshop_assets", "content_deleted_at", "INTEGER", false).await?;
    require_workshop_asset_content_deletion_contract(pool).await?;
    for contract in PARTIAL_UNIQUE_INDEXES {
        require_partial_unique_index(
            pool,
            contract.index_name,
            contract.table,
            contract.columns,
            contract.predicate,
        )
        .await?;
    }
    Ok(())
}

/// Validate every populated stable business ID, managed UUID value and
/// canonical logical-reference column in the v3 registry.
pub(crate) async fn validate_id_value_contract(pool: &SqlitePool) -> Result<(), DbError> {
    for (table, column) in UUIDV7_BUSINESS_COLUMNS {
        validate_uuidv7_column_values(pool, table, column, None).await?;
    }
    for (table, column) in UUIDV7_MANAGED_VALUE_COLUMNS {
        validate_uuidv7_column_values(pool, table, column, None).await?;
    }
    for (table, column) in [
        ("agent_sessions", "agent_session_id"),
        ("agent_sessions", "parent_agent_session_id"),
        ("agent_presets", "preset_id"),
        ("agent_preset_revisions", "created_by"),
    ] {
        validate_uuidv7_column_values(pool, table, column, None).await?;
    }
    for reference in LOGICAL_REFERENCES {
        if reference.value_contract == LogicalReferenceValueContract::CanonicalUuidV7 {
            validate_uuidv7_column_values(
                pool,
                reference.child_table,
                reference.child_column,
                reference.child_predicate,
            )
            .await?;
        }
    }
    Ok(())
}

/// Validate the complete durable v3 ID data contract. Any failure identifies
/// the current managed dataset as incompatible; callers must quarantine/reset
/// the dataset rather than rewrite IDs.
pub async fn validate_id_data_contract(pool: &SqlitePool) -> Result<(), DbError> {
    validate_id_value_contract(pool).await?;
    validate_workshop_asset_origin_values(pool).await?;
    validate_creation_task_result_asset_ids(pool).await?;
    let findings = audit_logical_reference_orphans(pool).await?;
    if findings.is_empty() {
        return Ok(());
    }
    let details = findings
        .iter()
        .map(|finding| {
            format!(
                "{}.{} -> {}.{}: {} invalid/orphan value(s)",
                finding.child_table,
                finding.child_column,
                finding.parent_table,
                finding.parent_column,
                finding.count
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(DbError::Init(format!(
        "v3 ID data contract audit failed: {details}"
    )))
}

/// Read-only database orphan audit. Cross-store registry entries are skipped;
/// their owners must extend this skeleton with side-store inventory checks.
/// Keep-history and catalog-backed references permit an absent parent, but a
/// still-present parent must satisfy live-row and aggregate-scope predicates.
pub(crate) async fn audit_logical_reference_orphans(
    pool: &SqlitePool,
) -> Result<Vec<OrphanAuditFinding>, DbError> {
    let mut findings = Vec::new();
    for reference in LOGICAL_REFERENCES {
        if matches!(
            reference.orphan_audit_policy,
            OrphanAuditPolicy::ExternalOwner
        ) {
            audit_parentless_logical_reference_values(pool, reference, &mut findings).await?;
            continue;
        }
        let (Some(parent_table), Some(parent_column)) =
            (reference.parent_table, reference.parent_column)
        else {
            continue;
        };
        let child_predicate = reference
            .child_predicate
            .map(|value| format!(" AND ({value})"))
            .unwrap_or_default();
        let parent_predicate = reference
            .parent_predicate
            .map(|value| format!(" AND ({value})"))
            .unwrap_or_default();
        let aggregate_scope_predicate = reference
            .aggregate_scope_predicate
            .map(|value| format!(" AND ({value})"))
            .unwrap_or_default();
        let frozen_projection_authority = reference
            .frozen_projection_authority
            .map(|value| format!(" AND NOT ({value})"))
            .unwrap_or_default();
        let parent_exists = format!(
            "EXISTS (SELECT 1 FROM {parent_table} parent \
                     WHERE parent.{parent_column} = child.{child_column})",
            parent_table = quote_sqlite_identifier(parent_table),
            parent_column = quote_sqlite_identifier(parent_column),
            child_column = quote_sqlite_identifier(reference.child_column),
        );
        let valid_parent_exists = format!(
            "EXISTS (SELECT 1 FROM {parent_table} parent \
                     WHERE parent.{parent_column} = child.{child_column}\
                     {parent_predicate}{aggregate_scope_predicate})",
            parent_table = quote_sqlite_identifier(parent_table),
            parent_column = quote_sqlite_identifier(parent_column),
            child_column = quote_sqlite_identifier(reference.child_column),
        );
        let invalid_parent_predicate = match reference.orphan_audit_policy {
            OrphanAuditPolicy::RequireParent => format!("NOT {valid_parent_exists}"),
            OrphanAuditPolicy::AllowMissingHistoricalParent => {
                format!("{parent_exists} AND NOT {valid_parent_exists}")
            }
            OrphanAuditPolicy::ExternalOwner => {
                unreachable!("handled above")
            }
        };
        let sql = format!(
            "SELECT COUNT(*) FROM {child_table} child \
             WHERE child.{child_column} IS NOT NULL{child_predicate} \
               AND ({invalid_parent_predicate}){frozen_projection_authority}",
            child_table = quote_sqlite_identifier(reference.child_table),
            child_column = quote_sqlite_identifier(reference.child_column),
        );
        let count: i64 = sqlx::query_scalar(&sql).fetch_one(pool).await?;
        if count > 0 {
            findings.push(OrphanAuditFinding {
                child_table: reference.child_table.to_owned(),
                child_column: reference.child_column.to_owned(),
                parent_table: parent_table.to_owned(),
                parent_column: parent_column.to_owned(),
                count,
                delete_policy: delete_policy_name(reference.delete_policy),
                rebuild_policy: rebuild_policy_name(reference.rebuild_policy),
            });
        }
    }
    audit_json_logical_reference_orphans(pool, &mut findings).await?;
    Ok(findings)
}

async fn audit_parentless_logical_reference_values(
    pool: &SqlitePool,
    reference: &LogicalReference,
    findings: &mut Vec<OrphanAuditFinding>,
) -> Result<(), DbError> {
    if reference.kind != LogicalReferenceKind::Text
        || reference.value_contract != LogicalReferenceValueContract::CanonicalUuidV7
    {
        return Ok(());
    }
    let child_predicate = reference
        .child_predicate
        .map(|value| format!(" AND ({value})"))
        .unwrap_or_default();
    let sql = format!(
        "SELECT child.{column} AS value \
         FROM {table} child \
         WHERE child.{column} IS NOT NULL{child_predicate}",
        table = quote_sqlite_identifier(reference.child_table),
        column = quote_sqlite_identifier(reference.child_column),
    );
    let values: Vec<String> = sqlx::query_scalar(&sql).fetch_all(pool).await?;
    let invalid = values
        .iter()
        .filter(|value| nomifun_common::validate_uuidv7(value).is_err())
        .count() as i64;
    if invalid > 0 {
        findings.push(OrphanAuditFinding {
            child_table: reference.child_table.to_owned(),
            child_column: reference.child_column.to_owned(),
            parent_table: match reference.orphan_audit_policy {
                OrphanAuditPolicy::ExternalOwner => "<external>",
                OrphanAuditPolicy::RequireParent
                | OrphanAuditPolicy::AllowMissingHistoricalParent => {
                    unreachable!("parentless audit requires a parentless policy")
                }
            }
            .to_owned(),
            parent_column: "<none>".to_owned(),
            count: invalid,
            delete_policy: delete_policy_name(reference.delete_policy),
            rebuild_policy: rebuild_policy_name(reference.rebuild_policy),
        });
    }
    Ok(())
}

async fn require_autoincrement_primary_key(pool: &SqlitePool, table: &str) -> Result<(), DbError> {
    let columns = table_info(pool, table).await?;
    let Some(id) = columns.iter().find(|column| column.name == "id") else {
        return Err(DbError::Init(format!("v3 schema table {table} is missing id")));
    };
    if id.data_type != "INTEGER" || id.primary_key_position != 1 {
        return Err(DbError::Init(format!(
            "v3 schema {table}.id must be the single INTEGER primary key"
        )));
    }
    if columns.iter().filter(|column| column.primary_key_position > 0).count() != 1 {
        return Err(DbError::Init(format!(
            "v3 schema table {table} must not have a composite primary key"
        )));
    }
    let create_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    let normalized = normalize_sql(&create_sql);
    if !normalized.contains("ID INTEGER PRIMARY KEY AUTOINCREMENT") {
        return Err(DbError::Init(format!(
            "v3 schema table {table} must declare id INTEGER PRIMARY KEY AUTOINCREMENT"
        )));
    }
    Ok(())
}

/// Assert the customer-service notes FTS index still has the definition note
/// recall depends on.
///
/// Every fragment here is load-bearing, and a silent edit degrades recall
/// rather than failing loudly, which is why this is a boot assertion:
/// - `content='cs_notes'` / `content_rowid='id'` make it external-content, so
///   `cs_notes` stays the single source of truth for note text. Dropping them
///   turns the index into a second, silently diverging copy.
/// - `tokenize='trigram'` is what allows substring and CJK matching at all.
///   Falling back to the default unicode61 tokenizer would break every
///   Chinese query, since it splits on whitespace that Chinese does not use.
async fn validate_cs_notes_fts_contract(pool: &SqlitePool) -> Result<(), DbError> {
    let create_sql: Option<String> =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
            .bind(CS_NOTES_FTS_TABLE)
            .fetch_optional(pool)
            .await?;
    let create_sql = create_sql.ok_or_else(|| {
        DbError::Init(format!("v3 schema is missing the FTS index table {CS_NOTES_FTS_TABLE}"))
    })?;
    // Collapse whitespace so the check is insensitive to DDL formatting.
    let normalized = create_sql.split_whitespace().collect::<Vec<_>>().join("").to_ascii_lowercase();
    for fragment in ["usingfts5", "content='cs_notes'", "content_rowid='id'", "tokenize='trigram'"] {
        if !normalized.contains(fragment) {
            return Err(DbError::Init(format!(
                "v3 schema {CS_NOTES_FTS_TABLE} is missing the required definition fragment {fragment}"
            )));
        }
    }
    Ok(())
}

async fn validate_no_physical_foreign_keys(pool: &SqlitePool) -> Result<(), DbError> {
    for table in PRODUCT_TABLES {
        let sql = format!("PRAGMA foreign_key_list({})", quote_sqlite_identifier(table));
        if sqlx::query(&sql).fetch_optional(pool).await?.is_some() {
            return Err(DbError::Init(format!(
                "v3 schema forbids physical foreign keys; found one on {table}"
            )));
        }
    }
    Ok(())
}

async fn validate_no_triggers(pool: &SqlitePool) -> Result<(), DbError> {
    const TRIGGER_CONTRACTS: &[(&str, &[&str])] = &[
        (
            "channel_inbound_receipts_identity_immutable",
            &[
                "BEFORE UPDATE OF OPERATION_KEY, USER_SCOPE_ID, CHANNEL_PLUGIN_SCOPE_ID, PLATFORM, CHAT_ID, PROVIDER_EVENT_ID, PAYLOAD_HASH, CREATED_AT ON CHANNEL_INBOUND_RECEIPTS",
                "RAISE(ABORT, 'CHANNEL INBOUND RECEIPT IDENTITY IS IMMUTABLE')",
            ],
        ),
        (
            "channel_inbound_receipts_no_delete",
            &[
                "BEFORE DELETE ON CHANNEL_INBOUND_RECEIPTS",
                "RAISE(ABORT, 'CHANNEL INBOUND RECEIPTS ARE RETAINED INDEFINITELY')",
            ],
        ),
        (
            "channel_inbound_receipts_scope_set_once",
            &[
                "BEFORE UPDATE OF CONVERSATION_SCOPE_ID, MESSAGE_SCOPE_ID ON CHANNEL_INBOUND_RECEIPTS",
                "OLD.PHASE <> 'EFFECTS_STARTED' OR NEW.PHASE <> 'SETTLED'",
                "OLD.CONVERSATION_SCOPE_ID IS NOT NULL",
                "OLD.MESSAGE_SCOPE_ID IS NOT NULL",
                "RAISE(ABORT, 'CHANNEL INBOUND OUTCOME SCOPE CAN ONLY BE SET WHILE SETTLING')",
            ],
        ),
        (
            "channel_session_bindings_identity_immutable",
            &[
                "BEFORE UPDATE OF CHANNEL_PLUGIN_ID, CHANNEL_USER_ID, CHAT_ID, CHANNEL_SESSION_ID, CREATED_AT ON CHANNEL_SESSION_BINDINGS",
                "RAISE(ABORT, 'CHANNEL SESSION BINDING IDENTITY IS IMMUTABLE')",
            ],
        ),
        (
            "prevent_workshop_asset_content_resurrection",
            &[
                "BEFORE UPDATE ON WORKSHOP_ASSETS",
                "OLD.DELETED_AT IS NOT NULL",
                "NEW.DELETED_AT IS NOT OLD.DELETED_AT",
                "NEW.ASSET_ID IS NOT OLD.ASSET_ID",
                "RAISE(ABORT, 'DELETED WORKSHOP ASSET CANNOT BE RESTORED')",
            ],
        ),
        (
            "restrict_creation_task_deleted_assets_insert",
            &[
                "BEFORE INSERT ON CREATION_TASKS",
                "ASSET.DELETED_AT IS NOT NULL",
                "JSON_EACH(NEW.INPUT_BINDINGS)",
                "JSON_EACH(NEW.RESULT_ASSET_IDS)",
                "RAISE(ABORT, 'CREATION TASK REFERENCES A DELETED WORKSHOP ASSET')",
            ],
        ),
        (
            "restrict_creation_task_deleted_assets_update",
            &[
                "BEFORE UPDATE OF INPUT_BINDINGS, RESULT_ASSET_IDS, STATUS ON CREATION_TASKS",
                "ASSET.DELETED_AT IS NOT NULL",
                "NEW.STATUS IN ('QUEUED', 'RUNNING')",
                "JSON_EACH(OLD.INPUT_BINDINGS)",
                "JSON_EACH(OLD.RESULT_ASSET_IDS)",
                "RAISE(ABORT, 'CREATION TASK REFERENCES A DELETED WORKSHOP ASSET')",
            ],
        ),
        (
            "restrict_template_run_deleted_assets_insert",
            &[
                "BEFORE INSERT ON CREATIVE_STUDIO_TEMPLATE_RUNS",
                "ASSET.DELETED_AT IS NOT NULL",
                "JSON_TREE(NEW.AGGREGATE_JSON)",
                "RAISE(ABORT, 'TEMPLATE RUN REFERENCES A DELETED WORKSHOP ASSET')",
            ],
        ),
        (
            "restrict_template_run_deleted_assets_update",
            &[
                "BEFORE UPDATE OF AGGREGATE_JSON, STATUS ON CREATIVE_STUDIO_TEMPLATE_RUNS",
                "ASSET.DELETED_AT IS NOT NULL",
                "NEW.STATUS NOT IN ('SUCCEEDED', 'FAILED', 'CANCELLED')",
                "JSON_TREE(OLD.AGGREGATE_JSON)",
                "RAISE(ABORT, 'TEMPLATE RUN REFERENCES A DELETED WORKSHOP ASSET')",
            ],
        ),
        (
            "restrict_workshop_asset_delete_creation_task_refs",
            &[
                "BEFORE DELETE ON WORKSHOP_ASSETS",
                "RAISE(ABORT, 'WORKSHOP ASSET IS REFERENCED BY CREATION TASK INPUT OR RESULT')",
            ],
        ),
        (
            "trg_channel_plugins_owner_domain_insert_guard",
            &[
                "BEFORE INSERT ON CHANNEL_PLUGINS",
                "NEW.OWNER_DOMAIN = 'CUSTOMER_SERVICE' AND NEW.COMPANION_ID IS NOT NULL",
                "RAISE(ABORT, 'CUSTOMER-SERVICE CHANNEL BOTS CANNOT CARRY A COMPANION BINDING')",
            ],
        ),
        (
            "trg_channel_plugins_owner_domain_update_guard",
            &[
                "BEFORE UPDATE OF OWNER_DOMAIN, COMPANION_ID ON CHANNEL_PLUGINS",
                "NEW.OWNER_DOMAIN = 'CUSTOMER_SERVICE' AND NEW.COMPANION_ID IS NOT NULL",
                "RAISE(ABORT, 'CUSTOMER-SERVICE CHANNEL BOTS CANNOT CARRY A COMPANION BINDING')",
            ],
        ),
        (
            "trg_nomi_remote_events_append_only_delete",
            &[
                "BEFORE DELETE ON NOMI_REMOTE_EVENTS",
                "RAISE(ABORT, 'NOMI REMOTE EVENTS ARE APPEND ONLY')",
            ],
        ),
        (
            "trg_nomi_remote_events_append_only_update",
            &[
                "BEFORE UPDATE ON NOMI_REMOTE_EVENTS",
                "RAISE(ABORT, 'NOMI REMOTE EVENTS ARE APPEND ONLY')",
            ],
        ),
        (
            "trg_nomi_remote_sessions_provenance_immutable",
            &[
                "BEFORE UPDATE ON NOMI_REMOTE_SESSIONS",
                "NEW.AGENT_SESSION_ID IS NOT OLD.AGENT_SESSION_ID",
                "NEW.OWNER_USER_ID IS NOT OLD.OWNER_USER_ID",
                "NEW.REMOTE_BINDING_ID IS NOT OLD.REMOTE_BINDING_ID",
                "NEW.OPEN_IDEMPOTENCY_KEY IS NOT OLD.OPEN_IDEMPOTENCY_KEY",
                "NEW.BINDING_VERSION IS NOT OLD.BINDING_VERSION",
                "NEW.AGENT_BINDING_DIGEST IS NOT OLD.AGENT_BINDING_DIGEST",
                "NEW.INITIAL_INPUT_DIGEST IS NOT OLD.INITIAL_INPUT_DIGEST",
                "NEW.AGENT_BINDING_JSON IS NOT OLD.AGENT_BINDING_JSON",
                "NEW.NOMI_SNAPSHOT_JSON IS NOT OLD.NOMI_SNAPSHOT_JSON",
                "NEW.PROVENANCE_JSON IS NOT OLD.PROVENANCE_JSON",
                "RAISE(ABORT, 'NOMI REMOTE SESSION PROVENANCE IS IMMUTABLE')",
            ],
        ),
        (
            "trg_plugin_artifacts_immutable",
            &[
                "BEFORE UPDATE ON PLUGIN_ARTIFACTS",
                "RAISE(ABORT, 'PLUGIN ARTIFACTS ARE IMMUTABLE')",
            ],
        ),
        (
            "trg_plugins_identity_immutable",
            &[
                "BEFORE UPDATE ON PLUGINS",
                "NEW.PLUGIN_ID IS NOT OLD.PLUGIN_ID",
                "NEW.OWNER_USER_ID IS NOT OLD.OWNER_USER_ID",
                "NEW.PACKAGE_ID IS NOT OLD.PACKAGE_ID",
                "RAISE(ABORT, 'PLUGIN IDENTITY IS IMMUTABLE')",
            ],
        ),
        (
            "trg_plugins_revision_monotonic",
            &[
                "BEFORE UPDATE ON PLUGINS",
                "NEW.REVISION <> OLD.REVISION + 1",
                "RAISE(ABORT, 'PLUGIN REVISION MUST ADVANCE EXACTLY ONCE')",
            ],
        ),
        (
            "trg_plugin_drafts_identity_immutable",
            &[
                "BEFORE UPDATE ON PLUGIN_DRAFTS",
                "NEW.DRAFT_ID IS NOT OLD.DRAFT_ID",
                "RAISE(ABORT, 'PLUGIN DRAFT IDENTITY IS IMMUTABLE AND TIME IS MONOTONIC')",
            ],
        ),
        (
            "trg_plugin_credential_bindings_updated_at_monotonic",
            &[
                "BEFORE UPDATE ON PLUGIN_CREDENTIAL_BINDINGS",
                "NEW.UPDATED_AT_MS < OLD.UPDATED_AT_MS",
                "RAISE(ABORT, 'PLUGIN CREDENTIAL BINDING TIME IS MONOTONIC')",
            ],
        ),
        (
            "trg_plugin_grants_artifact_guard",
            &[
                "BEFORE INSERT ON PLUGIN_GRANTS",
                "ARTIFACT.ARTIFACT_DIGEST = NEW.CONFIRMED_ARTIFACT_DIGEST",
                "RAISE(ABORT, 'PLUGIN GRANT MUST REFERENCE A STORED ARTIFACT')",
            ],
        ),
        (
            "trg_plugin_grants_update_artifact_guard",
            &[
                "BEFORE UPDATE ON PLUGIN_GRANTS",
                "ARTIFACT.ARTIFACT_DIGEST = NEW.CONFIRMED_ARTIFACT_DIGEST",
                "RAISE(ABORT, 'PLUGIN GRANT MUST REFERENCE A STORED ARTIFACT')",
            ],
        ),
        (
            "trg_plugin_mutations_identity_immutable",
            &[
                "BEFORE UPDATE ON PLUGIN_MUTATIONS",
                "NEW.MUTATION_ID IS NOT OLD.MUTATION_ID",
                "NEW.PLUGIN_ID IS NOT OLD.PLUGIN_ID",
                "RAISE(ABORT, 'PLUGIN MUTATION IDENTITY IS IMMUTABLE AND TIME IS MONOTONIC')",
            ],
        ),        (
            "trg_requirements_absorb_done_cancelled",
            &[
                "BEFORE UPDATE OF STATUS ON REQUIREMENTS",
                "OLD.STATUS IN ('DONE', 'CANCELLED') AND NEW.STATUS IS NOT OLD.STATUS",
                "RAISE(ABORT, 'COMPLETED OR CANCELLED REQUIREMENT STATUS IS IMMUTABLE')",
            ],
        ),
        (
            "trg_requirements_active_identity_exit_guard",
            &[
                "BEFORE UPDATE ON REQUIREMENTS",
                "OLD.STATUS = 'IN_PROGRESS' AND NEW.STATUS IS NOT 'PENDING'",
                "NEW.CLAIM_GENERATION IS NOT OLD.CLAIM_GENERATION",
                "NEW.CLAIM_TOKEN IS NOT OLD.CLAIM_TOKEN",
                "NEW.OWNER_CONVERSATION_ID IS NOT OLD.OWNER_CONVERSATION_ID",
                "NEW.OWNER_TERMINAL_ID IS NOT OLD.OWNER_TERMINAL_ID",
                "NEW.ACTIVE_TURN_STARTED_AT IS NOT OLD.ACTIVE_TURN_STARTED_AT",
                "NEW.STARTED_AT IS NOT OLD.STARTED_AT",
                "NEW.ATTEMPT_COUNT IS NOT OLD.ATTEMPT_COUNT",
                "RAISE(ABORT, 'ACTIVE REQUIREMENT IDENTITY IS IMMUTABLE UNTIL EXACT REQUEUE')",
            ],
        ),
        (
            "trg_requirements_active_to_pending_pre_effect_guard",
            &[
                "BEFORE UPDATE ON REQUIREMENTS",
                "OLD.STATUS = 'IN_PROGRESS' AND NEW.STATUS = 'PENDING'",
                "FROM REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS AS GUARD",
                "GUARD.REQUIREMENT_ID = OLD.REQUIREMENT_ID",
                "GUARD.CLAIM_GENERATION = OLD.CLAIM_GENERATION",
                "GUARD.CLAIM_TOKEN = OLD.CLAIM_TOKEN",
                "FROM AGENT_EXECUTIONS AS EXECUTION",
                "'$.SOURCE.REQUIREMENT_ID') = OLD.REQUIREMENT_ID",
                "'$.SOURCE.CLAIM_GENERATION') = OLD.CLAIM_GENERATION",
                "FROM TERMINAL_TURN_ADMISSIONS AS ADMISSION",
                "ADMISSION.REQUIREMENT_ID = OLD.REQUIREMENT_ID",
                "ADMISSION.CLAIM_GENERATION = OLD.CLAIM_GENERATION",
                "NEW.CLAIM_GENERATION IS NOT OLD.CLAIM_GENERATION",
                "NEW.CLAIM_TOKEN IS NOT NULL",
                "NEW.ATTEMPT_COUNT IS NOT MAX(OLD.ATTEMPT_COUNT - 1, 0)",
                "RAISE(ABORT, 'ACTIVE REQUIREMENT MAY BECOME PENDING ONLY THROUGH EXACT PRE-EFFECT ABANDON')",
            ],
        ),
        (
            "trg_requirements_in_progress_insert_guard",
            &[
                "BEFORE INSERT ON REQUIREMENTS",
                "NEW.STATUS = 'IN_PROGRESS'",
                "RAISE(ABORT, 'IN-PROGRESS REQUIREMENT MAY ONLY BE ENTERED BY ATOMICALLY CLAIMING A PENDING ROW')",
            ],
        ),
        (
            "trg_requirements_in_progress_update_guard",
            &[
                "BEFORE UPDATE ON REQUIREMENTS",
                "NEW.STATUS = 'IN_PROGRESS'",
                "NEW.CLAIM_GENERATION IS NULL",
                "NEW.CLAIM_GENERATION <= 0",
                "NEW.CLAIM_TOKEN IS NULL",
                "NEW.ACTIVE_TURN_STARTED_AT IS NULL",
                "NEW.LEASE_EXPIRES_AT IS NULL",
                "NEW.STARTED_AT IS NULL",
                "NEW.LEASE_EXPIRES_AT <= NEW.ACTIVE_TURN_STARTED_AT",
                "OLD.STATUS = 'IN_PROGRESS'",
                "NEW.CLAIM_GENERATION IS NOT OLD.CLAIM_GENERATION",
                "NEW.CLAIM_TOKEN IS NOT OLD.CLAIM_TOKEN",
                "NEW.OWNER_CONVERSATION_ID IS NOT OLD.OWNER_CONVERSATION_ID",
                "NEW.OWNER_TERMINAL_ID IS NOT OLD.OWNER_TERMINAL_ID",
                "NEW.ACTIVE_TURN_STARTED_AT IS NOT OLD.ACTIVE_TURN_STARTED_AT",
                "NEW.STARTED_AT IS NOT OLD.STARTED_AT",
                "NEW.ATTEMPT_COUNT IS NOT OLD.ATTEMPT_COUNT",
                "OLD.STATUS = 'PENDING'",
                "NEW.CLAIM_GENERATION IS NOT OLD.CLAIM_GENERATION + 1",
                "NEW.ATTEMPT_COUNT IS NOT OLD.ATTEMPT_COUNT + 1",
                "OLD.STATUS IS NULL",
                "OLD.STATUS NOT IN ('PENDING', 'IN_PROGRESS')",
                "NEW.OWNER_CONVERSATION_ID IS NOT NULL AND NEW.OWNER_TERMINAL_ID IS NULL",
                "NEW.OWNER_CONVERSATION_ID IS NULL AND NEW.OWNER_TERMINAL_ID IS NOT NULL",
                "RAISE(ABORT, 'IN-PROGRESS REQUIREMENT REQUIRES GENERATION, CAPABILITY, AND EXACTLY ONE TYPED OWNER')",
            ],
        ),
        (
            "trg_requirements_pending_insert_guard",
            &[
                "BEFORE INSERT ON REQUIREMENTS",
                "NEW.STATUS = 'PENDING'",
                "NEW.CLAIM_TOKEN IS NOT NULL",
                "NEW.OWNER_CONVERSATION_ID IS NOT NULL",
                "NEW.OWNER_TERMINAL_ID IS NOT NULL",
                "NEW.ACTIVE_TURN_STARTED_AT IS NOT NULL",
                "NEW.LEASE_EXPIRES_AT IS NOT NULL",
                "RAISE(ABORT, 'PENDING REQUIREMENT CANNOT CARRY EXECUTION AUTHORITY')",
            ],
        ),
        (
            "trg_requirements_pending_update_guard",
            &[
                "BEFORE UPDATE ON REQUIREMENTS",
                "NEW.STATUS = 'PENDING'",
                "NEW.CLAIM_TOKEN IS NOT NULL",
                "NEW.OWNER_CONVERSATION_ID IS NOT NULL",
                "NEW.OWNER_TERMINAL_ID IS NOT NULL",
                "NEW.ACTIVE_TURN_STARTED_AT IS NOT NULL",
                "NEW.LEASE_EXPIRES_AT IS NOT NULL",
                "RAISE(ABORT, 'PENDING REQUIREMENT CANNOT CARRY EXECUTION AUTHORITY')",
            ],
        ),
        (
            "trg_requirements_pre_effect_abandon_guard_apply",
            &[
                "AFTER INSERT ON REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS",
                "UPDATE REQUIREMENTS SET STATUS = 'PENDING'",
                "ATTEMPT_COUNT = MAX(ATTEMPT_COUNT - 1, 0)",
                "UPDATED_AT = MAX(UPDATED_AT, NEW.CREATED_AT)",
                "REQUIREMENT_ID = NEW.REQUIREMENT_ID",
                "CLAIM_GENERATION = NEW.CLAIM_GENERATION",
                "CLAIM_TOKEN = NEW.CLAIM_TOKEN",
                "FROM REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS AS GUARD",
                "GUARD.ID = NEW.ID",
                "RAISE( ABORT, 'REQUIREMENT PRE-EFFECT ABANDON COMMAND DID NOT COMPLETE ITS EXACT TRANSITION' )",
            ],
        ),
        (
            "trg_requirements_pre_effect_abandon_guard_consume",
            &[
                "AFTER UPDATE ON REQUIREMENTS",
                "OLD.STATUS = 'IN_PROGRESS' AND NEW.STATUS = 'PENDING'",
                "DELETE FROM REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS",
                "REQUIREMENT_ID = OLD.REQUIREMENT_ID",
                "CLAIM_GENERATION = OLD.CLAIM_GENERATION",
                "CLAIM_TOKEN = OLD.CLAIM_TOKEN",
            ],
        ),
        (
            "trg_requirements_pre_effect_abandon_guard_delete_guard",
            &[
                "BEFORE DELETE ON REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS",
                "REQUIREMENT.STATUS = 'IN_PROGRESS'",
                "REQUIREMENT.CLAIM_GENERATION = OLD.CLAIM_GENERATION",
                "REQUIREMENT.CLAIM_TOKEN = OLD.CLAIM_TOKEN",
                "RAISE( ABORT, 'ACTIVE REQUIREMENT PRE-EFFECT ABANDON GUARD CAN ONLY BE CONSUMED BY GUARDED TRANSITION' )",
            ],
        ),
        (
            "trg_requirements_pre_effect_abandon_guard_immutable",
            &[
                "BEFORE UPDATE ON REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS",
                "RAISE( ABORT, 'REQUIREMENT PRE-EFFECT ABANDON GUARDS ARE IMMUTABLE' )",
            ],
        ),
        (
            "trg_requirements_pre_effect_abandon_guard_insert",
            &[
                "BEFORE INSERT ON REQUIREMENT_PRE_EFFECT_ABANDON_GUARDS",
                "REQUIREMENT.STATUS = 'IN_PROGRESS'",
                "REQUIREMENT.CLAIM_GENERATION = NEW.CLAIM_GENERATION",
                "REQUIREMENT.CLAIM_TOKEN = NEW.CLAIM_TOKEN",
                "FROM AGENT_EXECUTIONS AS EXECUTION",
                "'$.SOURCE.REQUIREMENT_ID') = REQUIREMENT.REQUIREMENT_ID",
                "'$.SOURCE.CLAIM_GENERATION') = REQUIREMENT.CLAIM_GENERATION",
                "FROM TERMINAL_TURN_ADMISSIONS AS ADMISSION",
                "ADMISSION.REQUIREMENT_ID = REQUIREMENT.REQUIREMENT_ID",
                "ADMISSION.CLAIM_GENERATION = REQUIREMENT.CLAIM_GENERATION",
                "RAISE(ABORT, 'REQUIREMENT PRE-EFFECT ABANDON GUARD REQUIRES EXACT AUTHORITY AND RECEIVER-ADMISSION ABSENCE')",
            ],
        ),
        (
            "trg_terminal_turn_admissions_open_insert_guard",
            &[
                "BEFORE INSERT ON TERMINAL_TURN_ADMISSIONS",
                "NEW.PHASE IS NOT 'SETTLED' AND NEW.CLAIM_TOKEN IS NULL",
                "RAISE(ABORT, 'OPEN TERMINAL TURN ADMISSION REQUIRES A REQUIREMENT CAPABILITY')",
            ],
        ),
        (
            "trg_terminal_turn_admissions_open_update_guard",
            &[
                "BEFORE UPDATE ON TERMINAL_TURN_ADMISSIONS",
                "NEW.PHASE IS NOT 'SETTLED'",
                "NEW.CLAIM_TOKEN IS NULL",
                "NEW.CLAIM_TOKEN IS NOT OLD.CLAIM_TOKEN",
                "RAISE(ABORT, 'TERMINAL TURN ADMISSION CAPABILITY IS REQUIRED AND IMMUTABLE')",
            ],
        ),
        (
            "validate_creation_task_input_bindings_insert",
            &[
                "BEFORE INSERT ON CREATION_TASKS",
                "RAISE(ABORT, 'INVALID CREATION TASK INPUT BINDING')",
            ],
        ),
        (
            "validate_creation_task_input_bindings_update",
            &[
                "BEFORE UPDATE OF INPUT_BINDINGS ON CREATION_TASKS",
                "RAISE(ABORT, 'INVALID CREATION TASK INPUT BINDING')",
            ],
        ),
        (
            "validate_creative_asset_origin_insert",
            &[
                "BEFORE INSERT ON WORKSHOP_ASSETS",
                "RAISE(ABORT, 'UNSUPPORTED CREATIVE ASSET ORIGIN ID KEY')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN CANVAS_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN PROJECT_ID')",
                "JSON_TYPE(NEW.ORIGIN, '$.WORKBENCH_KIND') IS NOT NULL",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN CONVERSATION_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN MESSAGE_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN TEMPLATE_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN TEMPLATE_RUN_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN TEMPLATE_STEP_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET CONVERSATION/CANVAS/TEMPLATE OWNER BRANCH')",
            ],
        ),
        (
            "validate_creative_asset_origin_update",
            &[
                "BEFORE UPDATE OF ORIGIN ON WORKSHOP_ASSETS",
                "RAISE(ABORT, 'UNSUPPORTED CREATIVE ASSET ORIGIN ID KEY')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN CANVAS_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN PROJECT_ID')",
                "JSON_TYPE(NEW.ORIGIN, '$.WORKBENCH_KIND') IS NOT NULL",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN CONVERSATION_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN MESSAGE_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN TEMPLATE_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN TEMPLATE_RUN_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET ORIGIN TEMPLATE_STEP_ID')",
                "RAISE(ABORT, 'INVALID CREATIVE ASSET CONVERSATION/CANVAS/TEMPLATE OWNER BRANCH')",
            ],
        ),
        (
            "validate_prompt_library_asset_origin_insert",
            &[
                "BEFORE INSERT ON WORKSHOP_ASSETS",
                "PROMPT_LIBRARY_SOURCE",
                "PROMPT_LIBRARY_ID",
                "RAISE(ABORT, 'INVALID PROMPT LIBRARY ASSET ORIGIN IDENTITY')",
                "RAISE(ABORT, 'INVALID CATALOG PROMPT LIBRARY ASSET ORIGIN')",
                "RAISE(ABORT, 'INVALID PRESET PROMPT LIBRARY ASSET ORIGIN')",
            ],
        ),
        (
            "validate_prompt_library_asset_origin_update",
            &[
                "BEFORE UPDATE OF ORIGIN, KIND ON WORKSHOP_ASSETS",
                "PROMPT_LIBRARY_SOURCE",
                "PROMPT_LIBRARY_ID",
                "RAISE(ABORT, 'INVALID PROMPT LIBRARY ASSET ORIGIN IDENTITY')",
                "RAISE(ABORT, 'INVALID CATALOG PROMPT LIBRARY ASSET ORIGIN')",
                "RAISE(ABORT, 'INVALID PRESET PROMPT LIBRARY ASSET ORIGIN')",
            ],
        )
];
    let trigger_rows =
        sqlx::query("SELECT name, sql FROM sqlite_schema WHERE type = 'trigger' ORDER BY name")
            .fetch_all(pool)
            .await?;
    let triggers: Vec<String> = trigger_rows
        .iter()
        .map(|row| row.try_get("name").map_err(DbError::Query))
        .collect::<Result<_, _>>()?;
    // Compare contracts in the same name order as SQLite. Declaration order
    // groups related invariants and must not become a startup failure.
    let mut trigger_contracts = TRIGGER_CONTRACTS.to_vec();
    trigger_contracts.sort_by_key(|(name, _)| *name);
    let expected: Vec<String> = trigger_contracts
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    if triggers != expected {
        return Err(DbError::Init(format!(
            "v3 schema permits only registered guard triggers; expected {expected:?}, found {triggers:?}"
        )));
    }
    for (row, (name, required_fragments)) in trigger_rows.iter().zip(trigger_contracts) {
        let create_sql: String = row.try_get("sql").map_err(DbError::Query)?;
        let normalized = normalize_sql(&create_sql);
        for fragment in required_fragments {
            if !normalized.contains(fragment) {
                return Err(DbError::Init(format!(
                    "v3 schema trigger {name} is missing required invariant fragment {fragment}"
                )));
            }
        }
    }
    Ok(())
}

async fn validate_no_row_id_columns(pool: &SqlitePool) -> Result<(), DbError> {
    for table in PRODUCT_TABLES {
        for column in table_info(pool, table).await? {
            if column.name.ends_with("_row_id") {
                return Err(DbError::Init(format!(
                    "v3 schema forbids dual-key column {table}.{}",
                    column.name
                )));
            }
        }
    }
    Ok(())
}

async fn validate_index_budget(pool: &SqlitePool) -> Result<(), DbError> {
    for table in PRODUCT_TABLES
        .iter()
        .chain(CANONICAL_AGENT_STORE_TABLES.iter())
        .chain(CANONICAL_PLUGIN_TABLES.iter())
    {
        let sql = format!("PRAGMA index_list({})", quote_sqlite_identifier(table));
        let rows = sqlx::query(&sql).fetch_all(pool).await?;
        if rows.len() <= MAX_INDEXES_PER_TABLE {
            continue;
        }
        let mut names = rows
            .iter()
            .map(|row| row.try_get::<String, _>("name").map_err(DbError::Query))
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        return Err(DbError::Init(format!(
            "v3 schema table {table} exceeds the physical index budget: {} > {}; indexes={names:?}",
            rows.len(),
            MAX_INDEXES_PER_TABLE,
        )));
    }
    Ok(())
}

async fn validate_business_id_registry(pool: &SqlitePool) -> Result<(), DbError> {
    let mut seen = BTreeSet::new();
    for (table, column) in UUIDV7_BUSINESS_COLUMNS {
        if !seen.insert((*table, *column)) {
            return Err(DbError::Init(format!(
                "v3 business-ID registry duplicates {table}.{column}"
            )));
        }
        require_column(pool, table, column, "TEXT", true).await?;
        require_single_column_unique_index(pool, table, column).await?;
        require_uuidv7_check(pool, table, column).await?;
    }
    for (table, column) in UUIDV7_MANAGED_VALUE_COLUMNS {
        require_column(pool, table, column, "TEXT", false).await?;
        require_uuidv7_check(pool, table, column).await?;
    }
    Ok(())
}

async fn validate_logical_reference_registry(pool: &SqlitePool) -> Result<(), DbError> {
    let mut seen_columns = BTreeSet::new();
    for reference in LOGICAL_REFERENCES {
        let key = (
            reference.child_table,
            reference.child_column,
            reference.parent_table,
            reference.parent_column,
            reference.child_predicate,
            reference.frozen_projection_authority,
        );
        if !seen_columns.insert(key) {
            return Err(DbError::Init(format!(
                "logical-reference registry duplicates {}.{} predicate {:?}",
                reference.child_table, reference.child_column, reference.child_predicate
            )));
        }
        let expected_type = "TEXT";
        require_column(
            pool,
            reference.child_table,
            reference.child_column,
            expected_type,
            !reference.nullable,
        )
        .await?;
        if reference.value_contract == LogicalReferenceValueContract::CanonicalUuidV7 {
            require_uuidv7_check(pool, reference.child_table, reference.child_column).await?;
        }
        if let (Some(parent_table), Some(parent_column)) =
            (reference.parent_table, reference.parent_column)
        {
            require_column(pool, parent_table, parent_column, expected_type, true).await?;
            require_unique_parent_identity(pool, parent_table, parent_column).await?;
        }
    }
    Ok(())
}

async fn validate_logical_reference_coverage(pool: &SqlitePool) -> Result<(), DbError> {
    let registered: BTreeSet<(&str, &str)> = LOGICAL_REFERENCES
        .iter()
        .map(|reference| (reference.child_table, reference.child_column))
        .collect();
    let exempt: BTreeSet<(&str, &str)> = NON_REFERENCE_ID_COLUMNS.iter().copied().collect();
    let mut missing = Vec::new();
    for table in PRODUCT_TABLES.iter().chain(CANONICAL_PLUGIN_TABLES.iter()) {
        for column in table_info(pool, table).await? {
            if column.name != "id"
                && column.name.ends_with("_id")
                && !registered.contains(&(*table, column.name.as_str()))
                && !exempt.contains(&(*table, column.name.as_str()))
            {
                missing.push(format!("{table}.{}", column.name));
            }
        }
    }
    if !missing.is_empty() {
        return Err(DbError::Init(format!(
            "v3 logical-reference registry is missing relationship-like columns {missing:?}"
        )));
    }
    Ok(())
}

async fn validate_json_logical_reference_registry(pool: &SqlitePool) -> Result<(), DbError> {
    let mut seen = BTreeSet::new();
    for reference in JSON_LOGICAL_REFERENCES {
        let key = (
            reference.child_table,
            reference.child_column,
            reference.json_path,
        );
        if !seen.insert(key) {
            return Err(DbError::Init(format!(
                "JSON logical-reference registry duplicates {}.{}:{}",
                reference.child_table, reference.child_column, reference.json_path
            )));
        }
        require_column(
            pool,
            reference.child_table,
            reference.child_column,
            "TEXT",
            false,
        )
        .await?;
        let expected_type = "TEXT";
        if let (Some(parent_table), Some(parent_column)) =
            (reference.parent_table, reference.parent_column)
        {
            require_column(pool, parent_table, parent_column, expected_type, true).await?;
            require_unique_parent_identity(pool, parent_table, parent_column).await?;
        }

        // Compile every registered extractor against the live SQLite build.
        // This catches misspelled paths/columns and unavailable JSON1 support.
        let sql = format!(
            "SELECT value FROM ({}) logical_reference LIMIT 0",
            reference.value_sql
        );
        sqlx::query(&sql).fetch_all(pool).await?;
    }
    Ok(())
}

async fn require_workshop_asset_origin_id_contract(
    pool: &SqlitePool,
) -> Result<(), DbError> {
    let create_sql: String = sqlx::query_scalar(
        "SELECT group_concat(sql, ' ') FROM sqlite_schema \
         WHERE (type = 'table' AND name = 'workshop_assets') \
            OR name IN ('validate_creative_asset_origin_insert', \
                        'validate_creative_asset_origin_update')",
    )
    .fetch_one(pool)
    .await?;
    let normalized = normalize_sql(&create_sql);
    for retired_key in [
        "TASK_ID",
        "PROVIDERID",
        "CANVASID",
        "NODEID",
        "CREATIONTASKID",
    ] {
        let fragment = format!("JSON_TYPE(ORIGIN, '$.{retired_key}') IS NULL");
        if !normalized.contains(&fragment) {
            return Err(DbError::Init(format!(
                "v3 workshop_assets.origin contract permits unsupported field {retired_key}"
            )));
        }
    }
    for retired_key in [
        "PROJECTID",
        "WORKBENCHKIND",
        "TEMPLATEID",
        "TEMPLATERUNID",
        "TEMPLATESTEPID",
    ] {
        let fragment = format!("JSON_TYPE(NEW.ORIGIN, '$.{retired_key}') IS NOT NULL");
        if !normalized.contains(&fragment) {
            return Err(DbError::Init(format!(
                "v3 workshop_assets.origin contract permits unsupported field {retired_key}"
            )));
        }
    }
    for key in [
        "PROVIDER_ID",
        "CANVAS_ID",
        "NODE_ID",
        "CREATION_TASK_ID",
    ] {
        let fragments = [
            format!("JSON_TYPE(ORIGIN, '$.{key}') IS NULL"),
            format!("JSON_TYPE(ORIGIN, '$.{key}') = 'TEXT'"),
            format!("LENGTH(JSON_EXTRACT(ORIGIN, '$.{key}')) = 36"),
            format!(
                "JSON_EXTRACT(ORIGIN, '$.{key}') GLOB '????????-????-7???-[89AB]???-????????????'"
            ),
            format!(
                "REPLACE(JSON_EXTRACT(ORIGIN, '$.{key}'), '-', '') NOT GLOB '*[^0-9A-F]*'"
            ),
        ];
        for fragment in fragments {
            if !normalized.contains(&fragment) {
                return Err(DbError::Init(format!(
                    "v3 workshop_assets.origin {key} contract is missing CHECK fragment {fragment}"
                )));
            }
        }
    }
    for key in [
        "PROJECT_ID",
        "CONVERSATION_ID",
        "MESSAGE_ID",
        "TEMPLATE_ID",
        "TEMPLATE_RUN_ID",
        "TEMPLATE_STEP_ID",
    ] {
        let contract_name = if key == "PROJECT_ID" {
            "legacy Canvas compatibility identifier (PROJECT_ID)"
        } else {
            key
        };
        let fragments = [
            format!("JSON_TYPE(NEW.ORIGIN, '$.{key}') IS NOT NULL"),
            format!("JSON_TYPE(NEW.ORIGIN, '$.{key}') IS 'TEXT'"),
            format!("LENGTH(JSON_EXTRACT(NEW.ORIGIN, '$.{key}')) = 36"),
            format!(
                "JSON_EXTRACT(NEW.ORIGIN, '$.{key}') GLOB '????????-????-7???-[89AB]???-????????????'"
            ),
            format!(
                "REPLACE(JSON_EXTRACT(NEW.ORIGIN, '$.{key}'), '-', '') NOT GLOB '*[^0-9A-F]*'"
            ),
        ];
        for fragment in fragments {
            if !normalized.contains(&fragment) {
                return Err(DbError::Init(format!(
                    "v3 workshop_assets.origin {contract_name} contract is missing trigger fragment {fragment}"
                )));
            }
        }
    }
    Ok(())
}

async fn require_prompt_library_asset_identity_contract(
    pool: &SqlitePool,
) -> Result<(), DbError> {
    const INDEX: &str = "uq_workshop_assets_prompt_library_identity";
    let create_sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?",
    )
    .bind(INDEX)
    .fetch_optional(pool)
    .await?;
    let create_sql = create_sql.ok_or_else(|| {
        DbError::Init(format!(
            "v3 prompt-library asset identity requires unique index {INDEX}"
        ))
    })?;
    let normalized = normalize_sql(&create_sql);
    for fragment in [
        "CREATE UNIQUE INDEX UQ_WORKSHOP_ASSETS_PROMPT_LIBRARY_IDENTITY",
        "ON WORKSHOP_ASSETS(",
        "JSON_EXTRACT(ORIGIN, '$.PROMPT_LIBRARY_SOURCE')",
        "JSON_EXTRACT(ORIGIN, '$.PROMPT_LIBRARY_ID')",
        "WHERE KIND = 'TEXT'",
        "AND DELETED_AT IS NULL",
        "JSON_TYPE(ORIGIN, '$.PROMPT_LIBRARY_SOURCE') = 'TEXT'",
        "JSON_TYPE(ORIGIN, '$.PROMPT_LIBRARY_ID') = 'TEXT'",
    ] {
        if !normalized.contains(fragment) {
            return Err(DbError::Init(format!(
                "v3 prompt-library asset identity index {INDEX} is missing {fragment}"
            )));
        }
    }
    Ok(())
}

async fn require_workshop_asset_content_deletion_contract(pool: &SqlitePool) -> Result<(), DbError> {
    for (name, kind, fragments) in [
        ("workshop_assets", "table", &[
            "DELETED_AT IS NULL OR (TYPEOF(DELETED_AT) = 'INTEGER' AND DELETED_AT >= 0)",
            "DELETED_AT IS NULL OR (IN_LIBRARY = 0 AND TEXT_CONTENT IS NULL)",
            "CONTENT_DELETED_AT >= DELETED_AT",
            "AND REL_PATH IS NULL AND THUMB_REL_PATH IS NULL",
        ][..]),
        ("idx_workshop_assets_pending_content_deletion", "index", &[
            "ON WORKSHOP_ASSETS(DELETED_AT, ASSET_ID)",
            "WHERE DELETED_AT IS NOT NULL AND CONTENT_DELETED_AT IS NULL",
        ][..]),
    ] {
        let sql: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE name = ? AND type = ?",
        ).bind(name).bind(kind).fetch_optional(pool).await?;
        let sql = sql.ok_or_else(|| DbError::Init(format!("asset content deletion requires {name}")))?;
        let normalized = normalize_sql(&sql);
        for fragment in fragments {
            if !normalized.contains(fragment) {
                return Err(DbError::Init(format!("asset content deletion {name} is missing {fragment}")));
            }
        }
    }
    Ok(())
}

async fn validate_workshop_asset_origin_values(pool: &SqlitePool) -> Result<(), DbError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT asset_id, origin FROM workshop_assets WHERE origin IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    for (asset_id, encoded) in rows {
        let value: serde_json::Value = serde_json::from_str(&encoded).map_err(|error| {
            DbError::Init(format!(
                "v3 workshop asset {asset_id} has invalid origin JSON: {error}"
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            DbError::Init(format!(
                "v3 workshop asset {asset_id} origin must be a JSON object"
            ))
        })?;
        for retired_key in [
            "task_id",
            "providerId",
            "canvasId",
            "nodeId",
            "creationTaskId",
            "projectId",
            "workbenchKind",
            "workbench_kind",
            "templateId",
            "templateRunId",
            "templateStepId",
        ] {
            if object.contains_key(retired_key) {
                return Err(DbError::Init(format!(
                    "v3 workshop asset {asset_id} origin contains unsupported ID field {retired_key:?}"
                )));
            }
        }
        for key in [
            "provider_id",
            "canvas_id",
            "node_id",
            "creation_task_id",
            "project_id",
            "template_id",
            "template_run_id",
            "template_step_id",
        ] {
            let Some(value) = object.get(key) else {
                continue;
            };
            let field_name = if key == "project_id" {
                "origin.project_id (legacy Canvas compatibility identifier)"
            } else {
                match key {
                    "provider_id" => "origin.provider_id",
                    "canvas_id" => "origin.canvas_id",
                    "node_id" => "origin.node_id",
                    "creation_task_id" => "origin.creation_task_id",
                    "template_id" => "origin.template_id",
                    "template_run_id" => "origin.template_run_id",
                    "template_step_id" => "origin.template_step_id",
                    _ => unreachable!("workshop origin key list is exhaustive"),
                }
            };
            let value = value.as_str().ok_or_else(|| {
                DbError::Init(format!(
                    "v3 workshop asset {asset_id} {field_name} must be omitted or a canonical UUIDv7 string"
                ))
            })?;
            nomifun_common::validate_uuidv7(value).map_err(|error| {
                DbError::Init(format!(
                    "v3 workshop asset {asset_id} {field_name}={value:?} is invalid: {error}"
                ))
            })?;
        }
        let has_canvas = object.contains_key("canvas_id");
        let has_legacy_canvas = object.contains_key("project_id");
        let has_node = object.contains_key("node_id");
        let any_canvas = has_canvas || has_legacy_canvas || has_node;
        let canvas_owner = (has_canvas != has_legacy_canvas) && has_node;
        let conversation = object.contains_key("conversation_id");
        let message = object.contains_key("message_id");
        for key in ["conversation_id", "message_id"] {
            if let Some(value) = object.get(key) {
                let value=value.as_str().ok_or_else(|| DbError::Init(format!("asset {asset_id} origin.{key} must be a UUIDv7")))?;
                nomifun_common::validate_uuidv7(value).map_err(|e| DbError::Init(e.to_string()))?;
            }
        }
        let template_count = ["template_id","template_run_id","template_step_id"].iter().filter(|key|object.contains_key(**key)).count();
        if (any_canvas && !canvas_owner) || conversation != message || (template_count != 0 && template_count != 3)
            || usize::from(canvas_owner) + usize::from(conversation && message) + usize::from(template_count == 3) > 1 {
            return Err(DbError::Init(format!("v3 workshop asset {asset_id} origin requires a single complete owner branch")));
        }

    }
    Ok(())
}

async fn validate_creation_task_result_asset_ids(pool: &SqlitePool) -> Result<(), DbError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT creation_task_id, status, result_asset_ids FROM creation_tasks",
    )
    .fetch_all(pool)
    .await?;
    for (creation_task_id, status, encoded) in rows {
        let values: serde_json::Value = serde_json::from_str(&encoded).map_err(|error| {
            DbError::Init(format!(
                "v3 creation task {creation_task_id} has invalid result_asset_ids JSON: {error}"
            ))
        })?;
        let values = values.as_array().ok_or_else(|| {
            DbError::Init(format!(
                "v3 creation task {creation_task_id} result_asset_ids must be a JSON array"
            ))
        })?;
        if status == "succeeded" && values.is_empty() {
            return Err(DbError::Init(format!(
                "v3 creation task {creation_task_id} is succeeded but has no result assets"
            )));
        }
        if status != "succeeded" && !values.is_empty() {
            return Err(DbError::Init(format!(
                "v3 creation task {creation_task_id} is {status:?} but claims committed result assets"
            )));
        }
        let mut seen = BTreeSet::new();
        for value in values {
            let value = value.as_str().ok_or_else(|| {
                DbError::Init(format!(
                    "v3 creation task {creation_task_id} result_asset_ids must contain only canonical UUIDv7 strings"
                ))
            })?;
            nomifun_common::WorkshopAssetId::parse(value).map_err(|error| {
                DbError::Init(format!(
                    "v3 creation task {creation_task_id} result asset {value:?} is invalid: {error}"
                ))
            })?;
            if !seen.insert(value) {
                return Err(DbError::Init(format!(
                    "v3 creation task {creation_task_id} contains duplicate result asset {value}"
                )));
            }
            let origin: Option<String> =
                sqlx::query_scalar("SELECT origin FROM workshop_assets WHERE asset_id = ?")
                    .bind(value)
                    .fetch_optional(pool)
                    .await?
                    .flatten();
            let origin = origin.ok_or_else(|| {
                DbError::Init(format!(
                    "v3 creation task {creation_task_id} result asset {value} is missing or has no managed origin"
                ))
            })?;
            let owner = serde_json::from_str::<serde_json::Value>(&origin)
                .ok()
                .and_then(|origin| {
                    origin
                        .get("creation_task_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                });
            if owner.as_deref() != Some(creation_task_id.as_str()) {
                return Err(DbError::Init(format!(
                    "v3 creation task {creation_task_id} result asset {value} belongs to {:?}",
                    owner.as_deref()
                )));
            }
        }
    }
    Ok(())
}

async fn audit_json_logical_reference_orphans(
    pool: &SqlitePool,
    findings: &mut Vec<OrphanAuditFinding>,
) -> Result<(), DbError> {
    for reference in JSON_LOGICAL_REFERENCES {
        let sql = format!(
            "SELECT value, typeof(value) AS value_type \
             FROM ({}) logical_reference WHERE value IS NOT NULL",
            reference.value_sql
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await?;
        let mut invalid = 0_i64;
        for row in rows {
            let value_type: String = row.try_get("value_type").map_err(DbError::Query)?;
            let parent_exists = {
                    if value_type != "text" {
                        invalid += 1;
                        continue;
                    }
                    let value: String = row.try_get("value").map_err(DbError::Query)?;
                    if value.trim().is_empty()
                        || (reference.value_contract
                            == LogicalReferenceValueContract::CanonicalUuidV7
                            && nomifun_common::validate_uuidv7(&value).is_err())
                    {
                        invalid += 1;
                        continue;
                    }
                    match (reference.parent_table, reference.parent_column) {
                        (Some(parent_table), Some(parent_column)) => {
                            let parent_sql = format!(
                                "SELECT EXISTS(SELECT 1 FROM {} WHERE {} = ?)",
                                quote_sqlite_identifier(parent_table),
                                quote_sqlite_identifier(parent_column),
                            );
                            sqlx::query_scalar::<_, bool>(&parent_sql)
                                .bind(value)
                                .fetch_one(pool)
                                .await?
                        }
                        _ => true,
                    }
            };
            if !parent_exists
                && reference.orphan_audit_policy == OrphanAuditPolicy::RequireParent
            {
                invalid += 1;
            }
        }
        if invalid > 0 {
            findings.push(OrphanAuditFinding {
                child_table: reference.child_table.to_owned(),
                child_column: format!("{}:{}", reference.child_column, reference.json_path),
                parent_table: reference.parent_table.unwrap_or("<external>").to_owned(),
                parent_column: reference.parent_column.unwrap_or("<external>").to_owned(),
                count: invalid,
                delete_policy: delete_policy_name(reference.delete_policy),
                rebuild_policy: rebuild_policy_name(reference.rebuild_policy),
            });
        }
    }
    Ok(())
}

async fn validate_uuidv7_column_values(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    predicate: Option<&str>,
) -> Result<(), DbError> {
    let predicate = predicate
        .map(|value| format!(" AND ({value})"))
        .unwrap_or_default();
    let sql = format!(
        "SELECT child.{column} FROM {table} child WHERE child.{column} IS NOT NULL{predicate}",
        table = quote_sqlite_identifier(table),
        column = quote_sqlite_identifier(column),
    );
    let values: Vec<String> = sqlx::query_scalar(&sql).fetch_all(pool).await?;
    for value in values {
        nomifun_common::validate_uuidv7(&value).map_err(|error| {
            DbError::Init(format!(
                "v3 business ID {table}.{column}={value:?} is invalid: {error}"
            ))
        })?;
    }
    Ok(())
}

async fn require_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    expected_type: &str,
    required: bool,
) -> Result<(), DbError> {
    let columns = table_info(pool, table).await?;
    let Some(actual) = columns.iter().find(|value| value.name == column) else {
        return Err(DbError::Init(format!(
            "v3 schema table {table} is missing column {column}"
        )));
    };
    if actual.data_type != expected_type {
        return Err(DbError::Init(format!(
            "v3 schema {table}.{column} must be {expected_type}, found {}",
            actual.data_type
        )));
    }
    if required && !actual.not_null && actual.primary_key_position == 0 {
        return Err(DbError::Init(format!(
            "v3 schema {table}.{column} must be NOT NULL"
        )));
    }
    Ok(())
}

async fn require_single_column_unique_index(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> Result<(), DbError> {
    let indexes = index_columns(pool, table).await?;
    if !indexes
        .values()
        .any(|index| {
            index.unique
                && !index.partial
                && index.columns.len() == 1
                && index.columns[0] == column
        })
    {
        return Err(DbError::Init(format!(
            "v3 business ID {table}.{column} must have a single-column UNIQUE index"
        )));
    }
    Ok(())
}

async fn require_exact_unique_key(
    pool: &SqlitePool,
    table: &str,
    expected_columns: &[&str],
) -> Result<(), DbError> {
    let indexes = index_columns(pool, table).await?;
    if indexes.values().any(|index| {
        index.unique
            && !index.partial
            && index
                .columns
                .iter()
                .map(String::as_str)
                .eq(expected_columns.iter().copied())
    }) {
        return Ok(());
    }
    Err(DbError::Init(format!(
        "v3 schema upsert target {table}{expected_columns:?} requires an exact non-partial UNIQUE key"
    )))
}

async fn require_unique_parent_identity(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> Result<(), DbError> {
    if column == "id" {
        let columns = table_info(pool, table).await?;
        if columns
            .iter()
            .any(|value| value.name == "id" && value.primary_key_position == 1)
        {
            return Ok(());
        }
    }
    require_single_column_unique_index(pool, table, column).await
}

async fn require_uuidv7_check(pool: &SqlitePool, table: &str, column: &str) -> Result<(), DbError> {
    let create_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    let normalized = normalize_sql(&create_sql);
    let column_name = column.to_ascii_uppercase();
    for fragment in [
        format!("LENGTH({column_name}) = 36"),
        format!("LOWER({column_name}) = {column_name}"),
        format!("{column_name} GLOB '????????-????-7???-[89AB]???-????????????'"),
        format!("REPLACE({column_name}, '-', '') NOT GLOB '*[^0-9A-F]*'"),
    ] {
        if !normalized.contains(&fragment) {
            return Err(DbError::Init(format!(
                "v3 business ID {table}.{column} is missing UUIDv7 CHECK fragment {fragment}"
            )));
        }
    }
    Ok(())
}

async fn require_partial_unique_index(
    pool: &SqlitePool,
    index_name: &str,
    table: &str,
    expected_columns: &[&str],
    predicate: &str,
) -> Result<(), DbError> {
    let sql = format!("PRAGMA index_list({})", quote_sqlite_identifier(table));
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let Some(row) = rows
        .iter()
        .find(|row| row.try_get::<String, _>("name").ok().as_deref() == Some(index_name))
    else {
        return Err(DbError::Init(format!(
            "v3 schema {table}{expected_columns:?} requires partial UNIQUE index {index_name}"
        )));
    };
    let unique = row.try_get::<i64, _>("unique").map_err(DbError::Query)? != 0;
    let partial = row.try_get::<i64, _>("partial").map_err(DbError::Query)? != 0;
    if !unique || !partial {
        return Err(DbError::Init(format!(
            "v3 schema index {index_name} must be UNIQUE and partial"
        )));
    }

    let info_sql = format!("PRAGMA index_info({})", quote_sqlite_identifier(index_name));
    let mut columns = sqlx::query(&info_sql).fetch_all(pool).await?;
    columns.sort_by_key(|row| row.try_get::<i64, _>("seqno").unwrap_or(i64::MAX));
    let columns = columns
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    if columns
        != expected_columns
            .iter()
            .map(|column| (*column).to_owned())
            .collect::<Vec<_>>()
    {
        return Err(DbError::Init(format!(
            "v3 schema index {index_name} must uniquely index {table}{expected_columns:?} in order"
        )));
    }

    let create_sql: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?")
            .bind(index_name)
            .fetch_one(pool)
            .await?;
    let normalized = normalize_sql(&create_sql);
    let actual_predicate = normalized
        .split_once(" WHERE ")
        .map(|(_, predicate)| predicate);
    let expected_predicate = normalize_sql(predicate);
    if actual_predicate != Some(expected_predicate.as_str()) {
        return Err(DbError::Init(format!(
            "v3 schema index {index_name} must use predicate {predicate}"
        )));
    }
    Ok(())
}

#[derive(Debug)]
struct ColumnInfo {
    name: String,
    data_type: String,
    not_null: bool,
    primary_key_position: i64,
}

async fn table_info(pool: &SqlitePool, table: &str) -> Result<Vec<ColumnInfo>, DbError> {
    let sql = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            Ok(ColumnInfo {
                name: row.try_get("name").map_err(DbError::Query)?,
                data_type: row
                    .try_get::<String, _>("type")
                    .map_err(DbError::Query)?
                    .to_ascii_uppercase(),
                not_null: row.try_get::<i64, _>("notnull").map_err(DbError::Query)? != 0,
                primary_key_position: row.try_get("pk").map_err(DbError::Query)?,
            })
        })
        .collect::<Result<Vec<_>, DbError>>()?)
}

#[derive(Debug)]
struct IndexInfo {
    unique: bool,
    partial: bool,
    columns: Vec<String>,
}

async fn index_columns(pool: &SqlitePool, table: &str) -> Result<BTreeMap<String, IndexInfo>, DbError> {
    let sql = format!("PRAGMA index_list({})", quote_sqlite_identifier(table));
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut indexes = BTreeMap::new();
    for row in rows {
        let name: String = row.try_get("name").map_err(DbError::Query)?;
        let unique = row.try_get::<i64, _>("unique").map_err(DbError::Query)? != 0;
        let partial = row.try_get::<i64, _>("partial").map_err(DbError::Query)? != 0;
        let info_sql = format!("PRAGMA index_info({})", quote_sqlite_identifier(&name));
        let mut columns = sqlx::query(&info_sql).fetch_all(pool).await?;
        columns.sort_by_key(|column| column.try_get::<i64, _>("seqno").unwrap_or(i64::MAX));
        let columns = columns
            .into_iter()
            .filter_map(|column| column.try_get::<String, _>("name").ok())
            .collect();
        indexes.insert(
            name,
            IndexInfo {
                unique,
                partial,
                columns,
            },
        );
    }
    Ok(indexes)
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

fn delete_policy_name(policy: DeletePolicy) -> &'static str {
    match policy {
        DeletePolicy::Restrict => "RESTRICT",
        DeletePolicy::Cascade => "CASCADE",
        DeletePolicy::SetNull => "SET_NULL",
        DeletePolicy::KeepHistory => "KEEP_HISTORY",
    }
}

fn rebuild_policy_name(policy: RebuildPolicy) -> &'static str {
    match policy {
        RebuildPolicy::PreserveBusinessId => "PRESERVE_BUSINESS_ID",
        RebuildPolicy::ExternalOwner => "EXTERNAL_OWNER",
    }
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
