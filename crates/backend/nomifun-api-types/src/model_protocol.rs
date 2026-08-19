//! Wire DTOs for the enumerable model-protocol manifest.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ModelTask;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum ProtocolExecutorKind {
    Agent,
    ModelInvoke,
    AsyncJob,
    RealtimeSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum ProtocolTransportKind {
    Http,
    Websocket,
    Sdk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum ProtocolScope {
    Native,
    OfficialCompat,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum ProtocolEndpointPurpose {
    Submit,
    Poll,
    Content,
    Session,
}

/// Which half of a request URL carries the provider's API version segment.
///
/// The two variants are mutually exclusive, and every HTTP protocol picks one.
/// Nothing in the wire format can infer this: `/chat/completions` and
/// `/v1/messages` are both valid endpoint templates, and the only difference is
/// whether the connection root is expected to end in `/v1` already. Stating it
/// here is what lets a custom provider be told the convention instead of having
/// to guess it — the manifest deliberately withholds `default_connections` from
/// custom-scope providers, but endpoint descriptors always reach them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum EndpointRootShape {
    /// The connection base URL must carry the version segment (`https://host/v1`);
    /// the endpoint template is version-free (`/chat/completions`).
    VersionedRoot,
    /// The connection base URL must be version-free (`https://host`); the
    /// endpoint template carries the version (`/v1/messages`, `/v1beta/...`).
    OriginRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProtocolEndpointDescriptor {
    pub task: ModelTask,
    pub field: String,
    pub purpose: ProtocolEndpointPurpose,
    pub method: Option<String>,
    pub default_value: String,
    /// Which half of the URL owns the version segment for this endpoint.
    pub root_shape: EndpointRootShape,
    /// The complete placeholder vocabulary accepted by this protocol field.
    pub allowed_placeholders: Vec<String>,
    /// Alternatives of which at least one must occur in every configured
    /// override. Poll/content job identifiers are required; submit/session
    /// model placeholders remain optional so an exact model URL is valid.
    pub required_placeholders: Vec<String>,
    pub editable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProtocolDefaultConnection {
    pub preset: String,
    pub platform: String,
    pub connection_role: Option<String>,
    pub connection_label: Option<String>,
    pub base_url: String,
    pub auth_scheme: String,
    pub requires_credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProtocolDescriptor {
    pub protocol_id: String,
    pub supported_tasks: Vec<ModelTask>,
    pub executor: ProtocolExecutorKind,
    pub transport: ProtocolTransportKind,
    /// Persisted auth schemes accepted by this protocol executor. Parameterized
    /// generic transports use `header_key:<name>` / `query_key:<param>` as
    /// wildcard vocabulary; Agent protocols list their exact required scheme.
    pub allowed_auth_schemes: Vec<String>,
    pub scopes: Vec<ProtocolScope>,
    pub platforms: Vec<String>,
    pub default_connections: Vec<ProtocolDefaultConnection>,
    pub endpoints: Vec<ProtocolEndpointDescriptor>,
    /// The shape shared by every endpoint of this protocol. `None` for `sdk`
    /// transports, which build no URL at all.
    pub root_shape: Option<EndpointRootShape>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProtocolTaskDescriptor {
    pub protocol_id: String,
    pub task: ModelTask,
    pub executor: ProtocolExecutorKind,
    pub transport: ProtocolTransportKind,
    pub endpoints: Vec<ProtocolEndpointDescriptor>,
    /// The shape shared by every endpoint of this protocol. `None` for `sdk`
    /// transports, which build no URL at all.
    pub root_shape: Option<EndpointRootShape>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct PlatformPresetDescriptor {
    pub preset: String,
    pub platform: String,
    pub platform_default_base_url: Option<String>,
    pub requires_user_input: bool,
    pub default_auth_scheme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProtocolRecommendation {
    pub protocol_id: String,
    pub connection_role: Option<String>,
    pub default_base_url: Option<String>,
    pub default_auth_scheme: Option<String>,
    pub base_url_override_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct AuthSchemeDescriptor {
    pub scheme: String,
    pub parameterized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelProtocolManifestResponse {
    pub tasks: Vec<ModelTask>,
    pub preset: String,
    pub platform: String,
    pub requested_task: ModelTask,
    pub platform_default_base_url: Option<String>,
    pub requires_user_input: bool,
    pub default_auth_scheme: Option<String>,
    pub auth_schemes: Vec<AuthSchemeDescriptor>,
    pub recommendation: Option<ProtocolRecommendation>,
    pub protocols: Vec<ProtocolDescriptor>,
}
