use std::collections::HashMap;

use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// A. Permissions & Risk
// ---------------------------------------------------------------------------

/// Network access permission — either unrestricted (`true`) or domain-scoped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum NetworkPermission {
    /// Unrestricted network access (dangerous).
    Unrestricted(bool),
    /// Domain-scoped network access (moderate).
    Scoped {
        #[serde(rename = "allowedDomains")]
        allowed_domains: Vec<String>,
        reasoning: String,
    },
}

/// Filesystem access scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FilesystemScope {
    ExtensionOnly,
    Workspace,
    Full,
}

/// Extension permission declarations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExtPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_user: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<bool>,
}

/// Overall risk level derived from permission declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Safe,
    Moderate,
    Dangerous,
}

/// Granularity of a single permission entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevel {
    None,
    Limited,
    Full,
}

/// A single permission detail for display purposes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionDetail {
    pub permission: String,
    pub level: PermissionLevel,
    pub description: String,
}

/// Complete permission analysis summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionSummary {
    pub permissions: ExtPermissions,
    pub risk_level: RiskLevel,
    pub details: Vec<PermissionDetail>,
}

// ---------------------------------------------------------------------------
// B. Contribution types (what an extension provides)
// ---------------------------------------------------------------------------

/// ACP adapter contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtAcpAdapter {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acp_args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yolo_mode: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_key_fields: Vec<serde_json::Value>,
}

/// MCP server contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtMcpServer {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub config: serde_json::Value,
}

/// Assistant contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtAssistant {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "presetAgentType")]
    pub preset_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "enabledSkills")]
    pub enabled_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// Autonomous agent contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtAgent {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "enabledSkills")]
    pub enabled_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// Skill contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtSkill {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Theme contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtTheme {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Relative path to the CSS file.
    pub css_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_image: Option<String>,
}

/// Channel plugin contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtChannelPlugin {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "credentialFields")]
    pub credential_fields: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "configFields")]
    pub config_fields: Vec<serde_json::Value>,
}

/// WebUI route definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtWebuiRoute {
    pub path: String,
    pub method: String,
    pub handler: String,
}

/// WebUI contribution from an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtWebui {
    pub id: String,
    pub directory: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ExtWebuiRoute>,
}

/// Settings tab position relative to a built-in tab.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettingsTabPosition {
    #[serde(rename = "relativeTo", alias = "anchor", alias = "relative_to")]
    pub relative_to: String,
    pub placement: String,
}

fn default_settings_tab_order() -> u32 {
    100
}

/// Settings tab contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtSettingsTab {
    pub id: String,
    #[serde(alias = "name")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(alias = "entryPoint")]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<SettingsTabPosition>,
    #[serde(default = "default_settings_tab_order")]
    pub order: u32,
}

/// Model provider contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtModelProvider {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// All contributions declared by an extension.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExtContributes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acp_adapters: Vec<ExtAcpAdapter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<ExtMcpServer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assistants: Vec<ExtAssistant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<ExtAgent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<ExtSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<ExtTheme>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_plugins: Vec<ExtChannelPlugin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webui: Vec<ExtWebui>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings_tabs: Vec<ExtSettingsTab>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_providers: Vec<ExtModelProvider>,
}

// ---------------------------------------------------------------------------
// C. Extension manifest
// ---------------------------------------------------------------------------

/// i18n configuration block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct I18nConfig {
    pub locales: Vec<String>,
    #[serde(default = "default_i18n_directory")]
    pub directory: String,
}

fn default_i18n_directory() -> String {
    "i18n".to_owned()
}

/// Engine compatibility declaration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EngineConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nomifun: Option<String>,
}

/// Lifecycle hook declarations (paths relative to extension root).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LifecycleHooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_install: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_uninstall: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_deactivate: Option<String>,
}

/// Complete extension manifest parsed from `nomi-extension.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionManifest {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub dependencies: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<ExtPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributes: Option<ExtContributes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i18n: Option<I18nConfig>,
}

// ---------------------------------------------------------------------------
// D. Extension runtime state
// ---------------------------------------------------------------------------

/// Where the extension was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionSource {
    Local,
    Appdata,
    Env,
}

/// Persisted state for an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionState {
    pub name: String,
    pub version: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<TimestampMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activated_at: Option<TimestampMs>,
}

/// A fully loaded extension with its manifest, location, and runtime state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoadedExtension {
    pub manifest: ExtensionManifest,
    pub directory: String,
    pub source: ExtensionSource,
    pub state: ExtensionState,
}

// ---------------------------------------------------------------------------
// E. Extension system events
// ---------------------------------------------------------------------------

/// Events emitted by the extension system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExtensionSystemEvent {
    ExtensionActivated,
    ExtensionDeactivated,
    ExtensionInstalled,
    ExtensionUninstalled,
    RegistryReloaded,
    StatesPersisted,
}

/// Payload for extension lifecycle events (M-46).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionLifecyclePayload {
    pub extension_name: String,
    pub event: ExtensionSystemEvent,
    pub timestamp: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// F. Hub types
// ---------------------------------------------------------------------------

/// Installation status of a Hub extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubExtensionStatus {
    NotInstalled,
    Installed,
    UpdateAvailable,
    Installing,
    InstallFailed,
}

/// A Hub extension entry with runtime status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HubExtensionWithStatus {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub bundled: bool,
    pub status: HubExtensionStatus,
}

// ---------------------------------------------------------------------------
// G. Resolved contribution types (post-processing output)
// ---------------------------------------------------------------------------

/// Resolved ACP adapter (after env template resolution).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedAcpAdapter {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acp_args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yolo_mode: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_key_fields: Vec<serde_json::Value>,
}

/// Resolved MCP server (after env template resolution).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedMcpServer {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub config: serde_json::Value,
}

/// Resolved assistant (after @file: and env template resolution).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedAssistant {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// Resolved agent (after @file: and env template resolution).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedAgent {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// Resolved skill contributed by an extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedSkill {
    pub extension_name: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Resolved theme (CSS content loaded into memory).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedTheme {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub css_content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_image: Option<String>,
}

/// Resolved channel plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedChannelPlugin {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_fields: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<serde_json::Value>,
}

/// Resolved WebUI contribution (after route validation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebuiContribution {
    pub extension_name: String,
    pub id: String,
    pub directory: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ExtWebuiRoute>,
}

/// Resolved settings tab (after position parsing).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedSettingsTab {
    #[serde(rename = "extensionName")]
    pub extension_name: String,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<SettingsTabPosition>,
    pub order: u32,
}

/// Resolved model provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedModelProvider {
    pub extension_name: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

// ---------------------------------------------------------------------------
// H. Resolved contributions container
// ---------------------------------------------------------------------------

/// All resolved contributions from enabled extensions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResolvedContributions {
    pub acp_adapters: Vec<ResolvedAcpAdapter>,
    pub mcp_servers: Vec<ResolvedMcpServer>,
    pub assistants: Vec<ResolvedAssistant>,
    pub agents: Vec<ResolvedAgent>,
    pub skills: Vec<ResolvedSkill>,
    pub themes: Vec<ResolvedTheme>,
    pub channel_plugins: Vec<ResolvedChannelPlugin>,
    pub webui: Vec<WebuiContribution>,
    pub settings_tabs: Vec<ResolvedSettingsTab>,
    pub model_providers: Vec<ResolvedModelProvider>,
    /// i18n data keyed by extension name, then by message key.
    pub i18n: HashMap<String, HashMap<String, String>>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
