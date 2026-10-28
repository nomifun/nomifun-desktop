//! Product contracts for manual cloud-connector work. These are not Agent facts.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const FEISHU_CLI_VERSION: &str = "1.0.98";
pub const OFFICIAL_INSTALL_COMMAND: &str = "npx @larksuite/cli@latest install";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePhase { Missing, Ready, Broken, Preparing, Error }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSource { SharedUser, External, Selected }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub phase: RuntimePhase,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub source: Option<RuntimeSource>,
    pub message: Option<String>,
    pub official_install_command: String,
}
impl Default for RuntimeInfo {
    fn default() -> Self {
        Self { phase: RuntimePhase::Missing, executable: None, version: None, source: None,
            message: None, official_install_command: OFFICIAL_INSTALL_COMMAND.into() }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuConnection {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub profile: Option<String>,
    pub executable: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionUpdate {
    pub name: String,
    pub enabled: bool,
    pub profile: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSelection { pub executable: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeUpdateInfo {
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub compatible_version: String,
    pub update_available: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeishuActor {
    pub app_id: String,
    pub user_id: String,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuProfile {
    pub name: String,
    pub app_id: String,
    pub brand: String,
    pub user_name: Option<String>,
    pub token_status: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuPermission {
    /// Product capability, e.g. documents.browse; not a revocation switch.
    pub capability: String,
    pub required_scopes: Vec<String>,
    pub granted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPhase { Disabled, NotConfigured, NeedsLogin, Ready, Expired, Error }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuConnectorStatus {
    pub provider: String,
    pub connection: FeishuConnection,
    pub runtime: RuntimeInfo,
    pub phase: ConnectionPhase,
    pub actor: Option<FeishuActor>,
    pub permissions: Vec<FeishuPermission>,
    pub host_name: String,
    pub host_os: String,
    pub message: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind { ConfigureNew, ConfigureExisting, Login }
/// Never derive Debug: app_secret is supplied only to CLI stdin and discarded.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthStartRequest {
    pub kind: AuthKind,
    #[serde(default)] pub scopes: Vec<String>,
    pub profile: Option<String>,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogoutRequest {
    pub profile: String,
    pub expected_actor: Option<FeishuActor>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthPhase { Starting, WaitingUser, WaitingAdmin, Cancelling, Complete, Partial, Cancelled, Denied, Expired, Error }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationFlow {
    pub id: String,
    pub kind: AuthKind,
    pub phase: AuthPhase,
    pub verification_url: Option<String>,
    pub expires_at: Option<String>,
    pub missing_scopes: Vec<String>,
    pub console_url: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFilter { All, Document, Base }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeishuQuery {
    Documents { query: String, filter: DocumentFilter, cursor: Option<String> },
    BaseRecords { base_token: String, table_id: String, offset: Option<u32>, filter: Option<Value> },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub modified_at: Option<String>,
    pub base_token: Option<String>,
    pub table_id: Option<String>,
    pub pending_operation_id: Option<String>,
    pub pending_status: Option<ItemStatus>,
    #[serde(default)] pub fields: BTreeMap<String, Value>,
}
impl FeishuItem {
    pub fn new(id: impl Into<String>, kind: impl Into<String>, title: impl Into<String>) -> Self {
        Self { id: id.into(), kind: kind.into(), title: title.into(), url: None,
            modified_at: None, base_token: None, table_id: None, pending_operation_id: None,
            pending_status: None, fields: BTreeMap::new() }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuField {
    pub id: String,
    pub name: String,
    pub field_type: String,
    pub writable: bool,
    pub options: Option<Value>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeishuQueryResult {
    pub items: Vec<FeishuItem>,
    pub fields: Vec<FeishuField>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeishuMutation {
    DocumentCreate { client_item_id: String, title: String },
    DocumentDelete { client_item_id: String, document_id: String, document_type: String, title: String },
    BaseUpdate { client_item_id: String, base_token: String, table_id: String,
        record_id: String, fields: BTreeMap<String, Value>, before: BTreeMap<String, Value> },
}
impl FeishuMutation {
    pub fn client_item_id(&self) -> &str {
        match self { Self::DocumentCreate { client_item_id, .. } | Self::DocumentDelete { client_item_id, .. } |
            Self::BaseUpdate { client_item_id, .. } => client_item_id }
    }
    pub fn kind(&self) -> &'static str {
        match self { Self::DocumentCreate { .. } => "document_create", Self::DocumentDelete { .. } => "document_delete",
            Self::BaseUpdate { .. } => "base_update" }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub request_id: String,
    pub expected_actor: FeishuActor,
    pub profile: String,
    pub items: Vec<FeishuMutation>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus { Queued, Running, Succeeded, NotExecuted, Unknown, AcceptedPendingConfirmation }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus { Queued, Running, Succeeded, Partial, NotExecuted, Unknown, AcceptedPendingConfirmation }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationAttempt {
    pub id: String,
    pub number: u32,
    pub client_token: String,
    pub mutation: Value,
    pub status: ItemStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub message: Option<String>,
    pub result: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind { OriginalReceipt, TargetReadback, Inconclusive, Error }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationObservation {
    pub id: String,
    pub kind: ObservationKind,
    pub created_at: String,
    pub message: String,
    pub data: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationItem {
    pub id: String,
    pub mutation: Value,
    pub target_key: String,
    pub status: ItemStatus,
    pub remote_id: Option<String>,
    pub remote_url: Option<String>,
    pub message: Option<String>,
    pub attempts: Vec<OperationAttempt>,
    pub observations: Vec<OperationObservation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuOperation {
    pub id: String,
    pub request_id: String,
    pub connection_id: String,
    pub actor: FeishuActor,
    pub profile: String,
    pub status: OperationStatus,
    pub items: Vec<OperationItem>,
}
