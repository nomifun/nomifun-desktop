use std::{collections::BTreeMap, path::PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct LocalConnection {
    pub host_url: String,
    pub local_trust: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind { Source, Artifact }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppState { Running, Stopped, Failed }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppOperation { Preview, Use, Start, Stop, Rebuild }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettings {
    pub project_directory: String,
    pub install_command: Option<String>,
    pub preview_command: Option<String>,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
    pub start_command: Option<String>,
    pub web_url: Option<String>,
    pub node_path: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub mcp_url: Option<String>,
    pub agent_id: Option<String>,
    pub agent_binding_id: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { project_directory: ".".into(), install_command: None, preview_command: None,
            build_command: None, output_directory: None, start_command: None, web_url: None,
            node_path: None, environment: BTreeMap::new(), mcp_url: None, agent_id: None,
            agent_binding_id: None }
    }
}

/// A command setting omitted from an update keeps its current value; an
/// explicit null clears it. Connection and Session references are not editable.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettingsPatch {
    pub project_directory: Option<String>,
    #[serde(deserialize_with = "nullable_setting")]
    pub install_command: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub preview_command: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub build_command: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub output_directory: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub start_command: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub web_url: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub node_path: Option<Option<String>>,
    pub environment: Option<BTreeMap<String, String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub mcp_url: Option<Option<String>>,
    #[serde(deserialize_with = "nullable_setting")]
    pub agent_id: Option<Option<String>>,
}

fn nullable_setting<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

impl AppSettingsPatch {
    pub fn apply(self, settings: &mut AppSettings) {
        if let Some(value) = self.project_directory { settings.project_directory = value; }
        if let Some(value) = self.install_command { settings.install_command = value; }
        if let Some(value) = self.preview_command { settings.preview_command = value; }
        if let Some(value) = self.build_command { settings.build_command = value; }
        if let Some(value) = self.output_directory { settings.output_directory = value; }
        if let Some(value) = self.start_command { settings.start_command = value; }
        if let Some(value) = self.web_url { settings.web_url = value; }
        if let Some(value) = self.node_path { settings.node_path = value; }
        if let Some(value) = self.environment { settings.environment = value; }
        if let Some(value) = self.mcp_url { settings.mcp_url = value; }
        if let Some(value) = self.agent_id { settings.agent_id = value; }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// A navigation reference to an ordinary canonical Session. This is not
    /// exclusive ownership, authorization, or a second Agent binding ledger.
    #[serde(default)]
    pub development_session_id: Option<String>,
    pub directory: PathBuf,
    pub import_kind: ImportKind,
    pub settings: AppSettings,
    pub state: AppState,
    pub operation: Option<String>,
    pub last_error: Option<String>,
    pub output: String,
    pub pid: Option<u32>,
    pub preview: bool,
    pub prepared: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppMetadata {
    pub description: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppDetails {
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub import_kind: Option<ImportKind>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("application not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Command(String),
    #[error("application operation outcome is unknown: {0}")]
    OutcomeUnknown(String),
    #[error("operation stopped")]
    Cancelled,
    #[error("Host is shutting down")]
    ShuttingDown,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
