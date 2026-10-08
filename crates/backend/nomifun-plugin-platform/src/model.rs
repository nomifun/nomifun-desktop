use std::collections::BTreeMap;

use nomifun_agent_contracts::{DigestHex, PluginArtifact, PluginDraftId, PluginId, PluginMutationId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRecord {
    pub owner_user_id: String,
    pub plugin_id: PluginId,
    pub package_id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trashed_at_ms: Option<i64>,
    pub active_artifact_digest: DigestHex,
    pub data_generation: String,
    pub revision: u64,
    pub config: Value,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl PluginRecord {
    pub fn is_available(&self) -> bool {
        self.enabled && self.trashed_at_ms.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredArtifactRecord {
    pub artifact: PluginArtifact,
    pub artifact_root: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDraftStatus {
    Ready,
    Failed,
}

impl PluginDraftStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDraftRecord {
    pub owner_user_id: String,
    pub draft_id: PluginDraftId,
    pub revision: u64,
    pub plugin_id: Option<PluginId>,
    pub base_revision: Option<u64>,
    pub name: String,
    pub workspace_path: String,
    pub source_conversation_id: Option<String>,
    pub source_message_id: Option<String>,
    /// Host-owned idempotency token for draft creation, never a product identity.
    pub source_operation_key: Option<String>,
    pub source_request_digest: Option<String>,
    /// Host-produced exact-artifact verification; never writable through file tools.
    pub verification: Value,
    pub status: PluginDraftStatus,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCredentialBinding {
    pub plugin_id: PluginId,
    pub slot: String,
    pub credential_id: String,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginGrant {
    pub plugin_id: PluginId,
    pub permission: String,
    pub granted: bool,
    pub confirmed_artifact_digest: DigestHex,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLibraryState {
    pub plugin_id: PluginId,
    pub pinned: bool,
    pub collection: Option<String>,
    pub custom_name: Option<String>,
    pub last_opened_at_ms: Option<i64>,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginMutationKind {
    Install,
    Update,
    PermanentDelete,
}

impl PluginMutationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::PermanentDelete => "permanent_delete",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginMutationPhase {
    Staging,
    Prepared,
    Committed,
    RollingBack,
    Failed,
}

impl PluginMutationPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Prepared => "prepared",
            Self::Committed => "committed",
            Self::RollingBack => "rolling_back",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInstallAssociation {
    pub draft_id: PluginDraftId,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMutationRecord {
    pub mutation_id: PluginMutationId,
    pub owner_user_id: String,
    pub plugin_id: PluginId,
    pub kind: PluginMutationKind,
    pub phase: PluginMutationPhase,
    pub old_artifact_digest: Option<DigestHex>,
    pub new_artifact_digest: Option<DigestHex>,
    pub old_data_generation: Option<String>,
    pub new_data_generation: Option<String>,
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub draft_association: Option<DraftInstallAssociation>,
    pub error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct InstallCommit {
    pub mutation_id: PluginMutationId,
    pub owner_user_id: String,
    pub plugin_id: PluginId,
    pub package_id: String,
    pub expected_revision: Option<u64>,
    pub artifact: StoredArtifactRecord,
    pub data_generation: String,
    pub config: Value,
    pub credential_bindings: BTreeMap<String, String>,
    pub grants: BTreeMap<String, bool>,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct PluginInventory {
    pub plugin: PluginRecord,
    pub artifact: StoredArtifactRecord,
    pub credential_bindings: BTreeMap<String, String>,
    pub grants: BTreeMap<String, PluginGrant>,
    pub library: PluginLibraryState,
}
