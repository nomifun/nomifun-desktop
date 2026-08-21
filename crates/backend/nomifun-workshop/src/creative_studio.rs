//! Canonical `nomifun.creative-studio/v1` project document and wire metadata.
//!
//! This is a closed, new product contract. It deliberately does not deserialize
//! the retired Workshop canvas document and contains no schema conversion path.

use std::collections::{BTreeSet, HashMap};

use nomifun_common::TimestampMs;
use nomifun_db::CreativeStudioProjectRow;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::{Map, Value};

pub const CREATIVE_STUDIO_SCHEMA: &str = "nomifun.creative-studio/v1";

/// Project documents contain references, never embedded media bytes. Eight MiB
/// leaves ample room for large canvases while bounding SQLite/WAL growth.
pub const MAX_CREATIVE_PROJECT_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeProjectDocument {
    pub schema: String,
    pub project_id: String,
    pub viewport: CreativeViewport,
    pub background: CreativeBackground,
    pub nodes: Vec<CreativeNode>,
    pub connections: Vec<CreativeConnection>,
    pub chat_sessions: Vec<CreativeChatSession>,
    pub active_chat_id: Option<String>,
    pub panels: CreativePanels,
    pub pending_task_ids: Vec<String>,
}

impl CreativeProjectDocument {
    pub fn empty(project_id: String) -> Self {
        Self {
            schema: CREATIVE_STUDIO_SCHEMA.to_owned(),
            project_id,
            viewport: CreativeViewport {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            background: CreativeBackground::Lines,
            nodes: Vec::new(),
            connections: Vec::new(),
            chat_sessions: Vec::new(),
            active_chat_id: None,
            panels: CreativePanels::default(),
            pending_task_ids: Vec::new(),
        }
    }

    /// Validate invariants that must remain stable across every client. The v1
    /// node payload union is closed: kind-specific evolution requires a new
    /// schema version instead of accepting uncoordinated fields.
    pub fn validate_for_project(&self, expected_project_id: &str) -> Result<(), String> {
        nomifun_common::validate_uuidv7(expected_project_id)
            .map_err(|error| format!("project id must be a canonical UUIDv7: {error}"))?;
        if self.schema != CREATIVE_STUDIO_SCHEMA {
            return Err(format!(
                "schema must be {CREATIVE_STUDIO_SCHEMA:?}, got {:?}",
                self.schema
            ));
        }
        if self.project_id != expected_project_id {
            return Err(format!(
                "document projectId {:?} does not match route project id {expected_project_id:?}",
                self.project_id
            ));
        }
        nomifun_common::validate_uuidv7(&self.project_id)
            .map_err(|error| format!("document projectId must be a canonical UUIDv7: {error}"))?;
        require_finite("viewport.x", self.viewport.x)?;
        require_finite("viewport.y", self.viewport.y)?;
        require_finite("viewport.zoom", self.viewport.zoom)?;
        if !(0.05..=5.0).contains(&self.viewport.zoom) {
            return Err("viewport.zoom must be between 0.05 and 5".into());
        }
        self.panels.validate()?;

        let mut node_ids = BTreeSet::new();
        let mut node_kinds = HashMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            require_id(&format!("nodes[{index}].id"), &node.id)?;
            if !node_ids.insert(node.id.as_str()) {
                return Err(format!("duplicate node id {:?}", node.id));
            }
            require_finite(&format!("nodes[{index}].position.x"), node.position.x)?;
            require_finite(&format!("nodes[{index}].position.y"), node.position.y)?;
            node.size.validate(&format!("nodes[{index}].size"))?;
            if let Some(group_id) = node.group_id.as_deref() {
                require_id(&format!("nodes[{index}].groupId"), group_id)?;
                if node.node_type == CreativeNodeType::Group {
                    return Err(format!(
                        "nodes[{index}].groupId must be null because group nesting is not supported"
                    ));
                }
            }
            if node.data.node_type() != node.node_type {
                return Err(format!(
                    "nodes[{index}].data does not match node type {:?}",
                    node.node_type
                ));
            }
            node.data.validate(&format!("nodes[{index}].data"))?;
            node_kinds.insert(node.id.as_str(), node.node_type);
        }
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(group_id) = node.group_id.as_deref() {
                if group_id == node.id {
                    return Err(format!(
                        "nodes[{index}].groupId must reference another group node"
                    ));
                }
                match node_kinds.get(group_id) {
                    Some(CreativeNodeType::Group) => {}
                    Some(_) => {
                        return Err(format!(
                            "nodes[{index}].groupId {group_id:?} must reference a group node"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "nodes[{index}].groupId {group_id:?} references a missing node"
                        ));
                    }
                }
            }
        }

        let mut connection_ids = BTreeSet::new();
        let mut directed_connections = BTreeSet::new();
        for (index, connection) in self.connections.iter().enumerate() {
            require_id(&format!("connections[{index}].id"), &connection.id)?;
            if !connection_ids.insert(connection.id.as_str()) {
                return Err(format!("duplicate connection id {:?}", connection.id));
            }
            let Some(source_kind) = node_kinds.get(connection.source_node_id.as_str()) else {
                return Err(format!(
                    "connections[{index}].sourceNodeId {:?} references a missing node",
                    connection.source_node_id
                ));
            };
            let Some(target_kind) = node_kinds.get(connection.target_node_id.as_str()) else {
                return Err(format!(
                    "connections[{index}].targetNodeId {:?} references a missing node",
                    connection.target_node_id
                ));
            };
            if connection.source_node_id == connection.target_node_id {
                return Err(format!("connections[{index}] must not connect a node to itself"));
            }
            if !directed_connections.insert((
                connection.source_node_id.as_str(),
                connection.target_node_id.as_str(),
            )) {
                return Err(format!(
                    "connections[{index}] duplicates an existing directed connection"
                ));
            }
            if *source_kind == CreativeNodeType::Group || *target_kind == CreativeNodeType::Group {
                return Err(format!(
                    "connections[{index}] must not connect group nodes"
                ));
            }
            if *source_kind == CreativeNodeType::Config
                && *target_kind == CreativeNodeType::Config
            {
                return Err(format!(
                    "connections[{index}] must not connect config to config"
                ));
            }
            if *source_kind == CreativeNodeType::Director {
                return Err(format!(
                    "connections[{index}] must not use director as a source"
                ));
            }
            if *target_kind == CreativeNodeType::Director
                && !matches!(
                    source_kind,
                    CreativeNodeType::Image | CreativeNodeType::Panorama
                )
            {
                return Err(format!(
                    "connections[{index}] director targets require an image or panorama source"
                ));
            }
            if let Some(handle) = connection.source_handle.as_deref() {
                require_id(&format!("connections[{index}].sourceHandle"), handle)?;
            }
            if let Some(handle) = connection.target_handle.as_deref() {
                require_id(&format!("connections[{index}].targetHandle"), handle)?;
            }
        }

        let mut chat_ids = BTreeSet::new();
        let mut pending_chat_id: Option<&str> = None;
        for (index, chat) in self.chat_sessions.iter().enumerate() {
            require_uuidv7(&format!("chatSessions[{index}].id"), &chat.id)?;
            require_string(
                &format!("chatSessions[{index}].title"),
                &chat.title,
                false,
                1_000,
            )?;
            if !chat_ids.insert(chat.id.as_str()) {
                return Err(format!("duplicate chat session id {:?}", chat.id));
            }
            if chat.created_at < 0 || chat.updated_at < chat.created_at {
                return Err(format!(
                    "chatSessions[{index}] timestamps must be non-negative and updatedAt must not precede createdAt"
                ));
            }
            validate_id_array(
                &format!("chatSessions[{index}].messageIds"),
                &chat.message_ids,
            )?;
            if chat.message_ids.len() % 2 != 0 {
                return Err(format!(
                    "chatSessions[{index}].messageIds must contain completed user/assistant pairs"
                ));
            }
            for (message_index, message_id) in chat.message_ids.iter().enumerate() {
                require_uuidv7(
                    &format!("chatSessions[{index}].messageIds[{message_index}]"),
                    message_id,
                )?;
            }
            if let Some(model) = chat.model.as_ref() {
                nomifun_common::ProviderId::parse(&model.provider_id).map_err(|error| {
                    format!("chatSessions[{index}].model.providerId is invalid: {error}")
                })?;
                require_trimmed_string(
                    &format!("chatSessions[{index}].model.model"),
                    &model.model,
                    512,
                )?;
            }
            if let Some(pending) = chat.pending_turn.as_ref() {
                if pending_chat_id.replace(chat.id.as_str()).is_some() {
                    return Err("chatSessions must contain at most one pending Agent turn".to_owned());
                }
                if chat.model.is_none() {
                    return Err(format!(
                        "chatSessions[{index}].model is required for a pending Agent turn"
                    ));
                }
                require_uuidv7(
                    &format!("chatSessions[{index}].pendingTurn.idempotencyKey"),
                    &pending.idempotency_key,
                )?;
                require_trimmed_string(
                    &format!("chatSessions[{index}].pendingTurn.prompt"),
                    &pending.prompt,
                    65_536,
                )?;
                if pending.created_at < chat.created_at || pending.created_at > chat.updated_at {
                    return Err(format!(
                        "chatSessions[{index}].pendingTurn.createdAt must be within the chat session lifetime"
                    ));
                }
            }
            if !chat.message_ids.is_empty() && chat.model.is_none() {
                return Err(format!(
                    "chatSessions[{index}].model is required for persisted Agent messages"
                ));
            }
        }
        if let Some(active_chat_id) = self.active_chat_id.as_deref() {
            require_uuidv7("activeChatId", active_chat_id)?;
            if !chat_ids.contains(active_chat_id) {
                return Err(format!(
                    "activeChatId {active_chat_id:?} references a missing chat session"
                ));
            }
        }
        if pending_chat_id.is_some() && pending_chat_id != self.active_chat_id.as_deref() {
            return Err("activeChatId must identify the session owning the pending Agent turn".to_owned());
        }

        validate_id_array("pendingTaskIds", &self.pending_task_ids)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeBackground {
    Dots,
    Lines,
    Blank,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreativeNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: CreativeNodeType,
    pub position: CreativePoint,
    pub size: CreativeSize,
    pub group_id: Option<String>,
    pub z_index: i64,
    pub locked: bool,
    pub data: CreativeNodeData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeNodeWire {
    id: String,
    #[serde(rename = "type")]
    node_type: CreativeNodeType,
    position: CreativePoint,
    size: CreativeSize,
    group_id: Option<String>,
    z_index: i64,
    locked: bool,
    data: Value,
}

impl<'de> Deserialize<'de> for CreativeNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CreativeNodeWire::deserialize(deserializer)?;
        let data = match wire.node_type {
            CreativeNodeType::Image => CreativeNodeData::Image(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Panorama => CreativeNodeData::Panorama(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Text => CreativeNodeData::Text(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Config => CreativeNodeData::Config(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Video => CreativeNodeData::Video(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Audio => CreativeNodeData::Audio(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Director => CreativeNodeData::Director(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
            CreativeNodeType::Group => CreativeNodeData::Group(
                serde_json::from_value(wire.data).map_err(D::Error::custom)?,
            ),
        };

        Ok(Self {
            id: wire.id,
            node_type: wire.node_type,
            position: wire.position,
            size: wire.size,
            group_id: wire.group_id,
            z_index: wire.z_index,
            locked: wire.locked,
            data,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeNodeType {
    Image,
    Panorama,
    Text,
    Config,
    Video,
    Audio,
    Director,
    Group,
}

/// Closed payload union for the eight canonical v1 node kinds. Untagged wire
/// encoding keeps the product JSON shape as `type + data`; [`CreativeNode`]'s
/// custom deserializer selects exactly one strict payload from the sibling
/// `type`, so kind/data drift is rejected before service validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CreativeNodeData {
    Image(CreativeImageNodeData),
    Panorama(CreativePanoramaNodeData),
    Text(CreativeTextNodeData),
    Config(CreativeConfigNodeData),
    Video(CreativeVideoNodeData),
    Audio(CreativeAudioNodeData),
    Director(CreativeDirectorNodeData),
    Group(CreativeGroupNodeData),
}

impl CreativeNodeData {
    fn node_type(&self) -> CreativeNodeType {
        match self {
            Self::Image(_) => CreativeNodeType::Image,
            Self::Panorama(_) => CreativeNodeType::Panorama,
            Self::Text(_) => CreativeNodeType::Text,
            Self::Config(_) => CreativeNodeType::Config,
            Self::Video(_) => CreativeNodeType::Video,
            Self::Audio(_) => CreativeNodeType::Audio,
            Self::Director(_) => CreativeNodeType::Director,
            Self::Group(_) => CreativeNodeType::Group,
        }
    }

    fn validate(&self, path: &str) -> Result<(), String> {
        match self {
            Self::Image(data) => data.validate(path),
            Self::Panorama(data) => data.validate(path),
            Self::Text(data) => data.validate(path),
            Self::Config(data) => data.validate(path),
            Self::Video(data) => data.validate(path),
            Self::Audio(data) => data.validate(path),
            Self::Director(data) => data.validate(path),
            Self::Group(data) => data.validate(path),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeImageNodeData {
    pub asset_id: Option<String>,
    pub caption: String,
    pub alt: String,
    pub fit: CreativeImageFit,
    pub natural_size: Option<CreativeSize>,
}

impl CreativeImageNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_optional_id(&format!("{path}.assetId"), self.asset_id.as_deref())?;
        require_string(&format!("{path}.caption"), &self.caption, true, 20_000)?;
        require_string(&format!("{path}.alt"), &self.alt, true, 2_000)?;
        if let Some(size) = self.natural_size {
            size.validate(&format!("{path}.naturalSize"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeImageFit {
    Contain,
    Cover,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativePanoramaNodeData {
    pub asset_id: Option<String>,
    pub projection: CreativePanoramaProjection,
    pub yaw: f64,
    pub pitch: f64,
    pub field_of_view: f64,
}

impl CreativePanoramaNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_optional_id(&format!("{path}.assetId"), self.asset_id.as_deref())?;
        require_range(&format!("{path}.yaw"), self.yaw, -360.0, 360.0)?;
        require_range(&format!("{path}.pitch"), self.pitch, -90.0, 90.0)?;
        require_range(
            &format!("{path}.fieldOfView"),
            self.field_of_view,
            10.0,
            150.0,
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativePanoramaProjection {
    Equirectangular,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeTextNodeData {
    pub text: String,
    pub format: CreativeTextFormat,
    pub font_size: f64,
    pub text_align: CreativeTextAlign,
}

impl CreativeTextNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_string(&format!("{path}.text"), &self.text, true, 1_000_000)?;
        require_range(&format!("{path}.fontSize"), self.font_size, 8.0, 256.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeTextFormat {
    Plain,
    Markdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeTextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeConfigNodeData {
    pub task: CreativeModelTask,
    pub capability: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub prompt: String,
    pub negative_prompt: String,
    pub parameters: Map<String, Value>,
    pub input_asset_ids: Vec<String>,
    pub task_id: Option<String>,
    pub result_asset_ids: Vec<String>,
    pub status: CreativeGenerationStatus,
    pub error_message: Option<String>,
}

impl CreativeConfigNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_string(
            &format!("{path}.capability"),
            &self.capability,
            false,
            128,
        )?;
        require_optional_id(&format!("{path}.providerId"), self.provider_id.as_deref())?;
        if let Some(model) = self.model.as_deref() {
            require_string(&format!("{path}.model"), model, false, 512)?;
        }
        require_string(&format!("{path}.prompt"), &self.prompt, true, 1_000_000)?;
        require_string(
            &format!("{path}.negativePrompt"),
            &self.negative_prompt,
            true,
            1_000_000,
        )?;
        validate_json_object(&format!("{path}.parameters"), &self.parameters)?;
        validate_id_array(&format!("{path}.inputAssetIds"), &self.input_asset_ids)?;
        require_optional_id(&format!("{path}.taskId"), self.task_id.as_deref())?;
        validate_id_array(&format!("{path}.resultAssetIds"), &self.result_asset_ids)?;
        if let Some(error_message) = self.error_message.as_deref() {
            require_string(
                &format!("{path}.errorMessage"),
                error_message,
                true,
                20_000,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreativeModelTask {
    Chat,
    ImageGeneration,
    ImageEdit,
    VideoGeneration,
    SpeechSynthesis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeGenerationStatus {
    Idle,
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeVideoNodeData {
    pub asset_id: Option<String>,
    pub poster_asset_id: Option<String>,
    pub autoplay: bool,
    pub r#loop: bool,
    pub muted: bool,
    pub trim_start_ms: f64,
    pub trim_end_ms: Option<f64>,
}

impl CreativeVideoNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_optional_id(&format!("{path}.assetId"), self.asset_id.as_deref())?;
        require_optional_id(
            &format!("{path}.posterAssetId"),
            self.poster_asset_id.as_deref(),
        )?;
        require_min(&format!("{path}.trimStartMs"), self.trim_start_ms, 0.0)?;
        if let Some(trim_end_ms) = self.trim_end_ms {
            require_min(
                &format!("{path}.trimEndMs"),
                trim_end_ms,
                self.trim_start_ms,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeAudioNodeData {
    pub asset_id: Option<String>,
    pub title: String,
    pub r#loop: bool,
    pub volume: f64,
    pub trim_start_ms: f64,
    pub trim_end_ms: Option<f64>,
}

impl CreativeAudioNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_optional_id(&format!("{path}.assetId"), self.asset_id.as_deref())?;
        require_string(&format!("{path}.title"), &self.title, true, 1_000)?;
        require_range(&format!("{path}.volume"), self.volume, 0.0, 1.0)?;
        require_min(&format!("{path}.trimStartMs"), self.trim_start_ms, 0.0)?;
        if let Some(trim_end_ms) = self.trim_end_ms {
            require_min(
                &format!("{path}.trimEndMs"),
                trim_end_ms,
                self.trim_start_ms,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeDirectorNodeData {
    /// Asset ID of the hidden canonical DirectorState v1 text sidecar.
    pub scene_id: Option<String>,
    pub camera_id: Option<String>,
    pub timeline_ms: f64,
    pub duration_ms: f64,
}

impl CreativeDirectorNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_optional_id(&format!("{path}.sceneId"), self.scene_id.as_deref())?;
        require_optional_id(&format!("{path}.cameraId"), self.camera_id.as_deref())?;
        require_min(&format!("{path}.durationMs"), self.duration_ms, 0.0)?;
        require_range(
            &format!("{path}.timelineMs"),
            self.timeline_ms,
            0.0,
            self.duration_ms,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeGroupNodeData {
    pub title: String,
    pub color: Option<String>,
    pub collapsed: bool,
}

impl CreativeGroupNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_string(&format!("{path}.title"), &self.title, false, 1_000)?;
        if let Some(color) = self.color.as_deref() {
            require_string(&format!("{path}.color"), color, false, 128)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativePoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeSize {
    pub width: f64,
    pub height: f64,
}

impl CreativeSize {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_min(&format!("{path}.width"), self.width, 1.0)?;
        require_min(&format!("{path}.height"), self.height, 1.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeConnection {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub source_handle: Option<String>,
    pub target_handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeChatSession {
    pub id: String,
    pub title: String,
    pub message_ids: Vec<String>,
    pub model: Option<CreativeChatModel>,
    pub pending_turn: Option<CreativeChatPendingTurn>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeChatModel {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeChatPendingTurn {
    pub idempotency_key: String,
    pub prompt: String,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativePanels {
    pub left: CreativeLeftPanel,
    pub right: CreativeRightPanel,
    pub bottom: CreativeBottomPanel,
}

impl Default for CreativePanels {
    fn default() -> Self {
        Self {
            left: CreativeLeftPanel {
                open: true,
                width: 280.0,
                active_view: CreativeLeftView::Canvas,
            },
            right: CreativeRightPanel {
                open: false,
                width: 390.0,
                active_view: CreativeRightView::Assistant,
            },
            bottom: CreativeBottomPanel {
                open: false,
                height: 240.0,
                active_view: CreativeBottomView::History,
            },
        }
    }
}

impl CreativePanels {
    fn validate(&self) -> Result<(), String> {
        require_range("panels.left.width", self.left.width, 180.0, 800.0)?;
        require_range("panels.right.width", self.right.width, 240.0, 960.0)?;
        require_range("panels.bottom.height", self.bottom.height, 120.0, 800.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeLeftPanel {
    pub open: bool,
    pub width: f64,
    pub active_view: CreativeLeftView,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeLeftView {
    Canvas,
    Assets,
    Prompts,
    Workflows,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeRightPanel {
    pub open: bool,
    pub width: f64,
    pub active_view: CreativeRightView,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeRightView {
    Assistant,
    Properties,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeBottomPanel {
    pub open: bool,
    pub height: f64,
    pub active_view: CreativeBottomView,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeBottomView {
    Timeline,
    History,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreativeProjectSummary {
    pub project_id: String,
    pub title: String,
    /// Decimal string on the wire: callers round-trip it as an opaque CAS token.
    pub revision: String,
    pub node_count: i64,
    pub connection_count: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<CreativeStudioProjectRow> for CreativeProjectSummary {
    fn from(row: CreativeStudioProjectRow) -> Self {
        Self {
            project_id: row.project_id,
            title: row.title,
            revision: row.revision.to_string(),
            node_count: row.node_count,
            connection_count: row.connection_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

fn require_string(
    path: &str,
    value: &str,
    allow_empty: bool,
    max_utf16_units: usize,
) -> Result<(), String> {
    let len = value.encode_utf16().count();
    if (!allow_empty && value.is_empty()) || len > max_utf16_units {
        return Err(format!(
            "{path} must be {} string no longer than {max_utf16_units} UTF-16 code units",
            if allow_empty { "a" } else { "a non-empty" }
        ));
    }
    Ok(())
}

fn require_id(path: &str, value: &str) -> Result<(), String> {
    require_string(path, value, false, 256)
}

fn require_uuidv7(path: &str, value: &str) -> Result<(), String> {
    nomifun_common::validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| format!("{path} must be a canonical lowercase UUIDv7: {error}"))
}

fn require_trimmed_string(path: &str, value: &str, max_utf16_units: usize) -> Result<(), String> {
    require_string(path, value, false, max_utf16_units)?;
    if value.trim() != value {
        return Err(format!("{path} must be trimmed"));
    }
    Ok(())
}

fn require_optional_id(path: &str, value: Option<&str>) -> Result<(), String> {
    if let Some(value) = value {
        require_id(path, value)?;
    }
    Ok(())
}

fn validate_id_array(path: &str, values: &[String]) -> Result<(), String> {
    let mut unique = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        require_id(&format!("{path}[{index}]"), value)?;
        if !unique.insert(value.as_str()) {
            return Err(format!("{path} must contain unique ids"));
        }
    }
    Ok(())
}

fn require_finite(path: &str, value: f64) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{path} must be finite"))
    }
}

fn require_min(path: &str, value: f64, min: f64) -> Result<(), String> {
    require_finite(path, value)?;
    if value < min {
        return Err(format!("{path} must be at least {min}"));
    }
    Ok(())
}

fn require_range(path: &str, value: f64, min: f64, max: f64) -> Result<(), String> {
    require_finite(path, value)?;
    if value < min || value > max {
        return Err(format!("{path} must be between {min} and {max}"));
    }
    Ok(())
}

fn validate_json_object(path: &str, value: &Map<String, Value>) -> Result<(), String> {
    for (key, value) in value {
        validate_json_value(&format!("{path}.{key}"), value, 1)?;
    }
    Ok(())
}

fn validate_json_value(path: &str, value: &Value, depth: usize) -> Result<(), String> {
    if depth > 40 {
        return Err(format!("{path} exceeds the maximum JSON depth of 40"));
    }
    match value {
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_json_value(&format!("{path}[{index}]"), value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                validate_json_value(&format!("{path}.{key}"), value, depth + 1)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000181";

    fn node_data(kind: &str) -> Value {
        match kind {
            "image" => serde_json::json!({
                "assetId": null,
                "caption": "image caption",
                "alt": "image alt",
                "fit": "contain",
                "naturalSize": { "width": 1920, "height": 1080 }
            }),
            "panorama" => serde_json::json!({
                "assetId": "asset-panorama",
                "projection": "equirectangular",
                "yaw": 45,
                "pitch": -10,
                "fieldOfView": 90
            }),
            "text" => serde_json::json!({
                "text": "hello",
                "format": "markdown",
                "fontSize": 18,
                "textAlign": "center"
            }),
            "config" => serde_json::json!({
                "task": "image_generation",
                "capability": "text-to-image",
                "providerId": "provider-a",
                "model": "image-model-v1",
                "prompt": "draw a fox",
                "negativePrompt": "",
                "parameters": { "seed": 42, "guidance": 7.5, "nested": { "ok": true } },
                "inputAssetIds": ["asset-input"],
                "taskId": "task-a",
                "resultAssetIds": ["asset-result"],
                "status": "succeeded",
                "errorMessage": null
            }),
            "video" => serde_json::json!({
                "assetId": "asset-video",
                "posterAssetId": null,
                "autoplay": false,
                "loop": true,
                "muted": true,
                "trimStartMs": 100,
                "trimEndMs": 2100
            }),
            "audio" => serde_json::json!({
                "assetId": "asset-audio",
                "title": "soundtrack",
                "loop": false,
                "volume": 0.75,
                "trimStartMs": 0,
                "trimEndMs": null
            }),
            "director" => serde_json::json!({
                "sceneId": "scene-a",
                "cameraId": null,
                "timelineMs": 1200,
                "durationMs": 5000
            }),
            "group" => serde_json::json!({
                "title": "scene group",
                "color": "#f8a100",
                "collapsed": false
            }),
            other => panic!("missing test payload for {other}"),
        }
    }

    fn node_value(id: &str, kind: &str) -> Value {
        serde_json::json!({
            "id": id,
            "type": kind,
            "position": { "x": 10, "y": 20 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 1,
            "locked": false,
            "data": node_data(kind)
        })
    }

    fn node(id: &str, kind: &str) -> CreativeNode {
        serde_json::from_value(node_value(id, kind)).unwrap()
    }

    fn connection(id: &str, source: &str, target: &str) -> CreativeConnection {
        CreativeConnection {
            id: id.to_owned(),
            source_node_id: source.to_owned(),
            target_node_id: target.to_owned(),
            source_handle: None,
            target_handle: None,
        }
    }

    fn graph_document() -> CreativeProjectDocument {
        let mut doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        doc.nodes = vec![
            node("image", "image"),
            node("panorama", "panorama"),
            node("text", "text"),
            node("config-a", "config"),
            node("config-b", "config"),
            node("director", "director"),
            node("group", "group"),
        ];
        doc
    }

    #[test]
    fn empty_document_round_trips_with_the_closed_v1_shape() {
        let doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        doc.validate_for_project(PROJECT_ID).unwrap();
        let value = serde_json::to_value(&doc).unwrap();
        assert_eq!(value["schema"], CREATIVE_STUDIO_SCHEMA);
        assert_eq!(value["projectId"], PROJECT_ID);
        assert_eq!(value["background"], "lines");
        assert_eq!(value["panels"]["left"]["open"], true);
        assert_eq!(value["panels"]["left"]["width"], 280.0);
        assert_eq!(value["panels"]["left"]["activeView"], "canvas");
        assert_eq!(value["panels"]["right"]["open"], false);
        assert_eq!(value["panels"]["right"]["width"], 390.0);
        assert_eq!(value["panels"]["right"]["activeView"], "assistant");
        assert!(value.get("pendingTaskIds").unwrap().is_array());
        assert_eq!(serde_json::from_value::<CreativeProjectDocument>(value).unwrap(), doc);
    }

    #[test]
    fn project_summary_uses_camel_case_node_and_connection_counts() {
        let summary = CreativeProjectSummary {
            project_id: PROJECT_ID.into(),
            title: "Project".into(),
            revision: "2".into(),
            node_count: 3,
            connection_count: 2,
            created_at: 100,
            updated_at: 200,
        };
        let value = serde_json::to_value(summary).unwrap();
        assert_eq!(value["nodeCount"], 3);
        assert_eq!(value["connectionCount"], 2);
        assert!(value.get("node_count").is_none());
        assert!(value.get("connection_count").is_none());
    }

    #[test]
    fn document_rejects_legacy_schema_mismatched_project_and_unknown_fields() {
        let mut doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        doc.schema = "1".to_owned();
        assert!(doc.validate_for_project(PROJECT_ID).unwrap_err().contains("schema must be"));

        let other = "0190f5fe-7c00-7a00-8abc-000000000182";
        let doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        assert!(doc.validate_for_project(other).unwrap_err().contains("does not match"));

        let mut value = serde_json::to_value(doc).unwrap();
        value.as_object_mut().unwrap().insert("legacyEdges".into(), Value::Array(vec![]));
        assert!(serde_json::from_value::<CreativeProjectDocument>(value).is_err());

        let mut value = serde_json::to_value(CreativeProjectDocument::empty(PROJECT_ID.into()))
            .unwrap();
        value["background"] = Value::String("grid".into());
        assert!(serde_json::from_value::<CreativeProjectDocument>(value).is_err());
    }

    #[test]
    fn all_eight_node_payloads_round_trip_and_validate() {
        for kind in [
            "image",
            "panorama",
            "text",
            "config",
            "video",
            "audio",
            "director",
            "group",
        ] {
            let parsed = node(&format!("node-{kind}"), kind);
            let round_trip: CreativeNode =
                serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
            assert_eq!(round_trip, parsed, "{kind} payload must round-trip");

            let mut doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
            doc.nodes.push(parsed);
            doc.validate_for_project(PROJECT_ID)
                .unwrap_or_else(|error| panic!("{kind} payload must validate: {error}"));
        }
    }

    #[test]
    fn node_deserialization_rejects_kind_payload_and_enum_drift() {
        let mut unknown_field = node_value("image", "image");
        unknown_field["data"]["legacyUrl"] = Value::String("file:///legacy.png".into());
        assert!(serde_json::from_value::<CreativeNode>(unknown_field).is_err());

        let mut wrong_kind = node_value("image", "image");
        wrong_kind["data"] = node_data("text");
        assert!(serde_json::from_value::<CreativeNode>(wrong_kind).is_err());

        let mut invalid_task = node_value("config", "config");
        invalid_task["data"]["task"] = Value::String("imageGeneration".into());
        assert!(serde_json::from_value::<CreativeNode>(invalid_task).is_err());

        let mut invalid_status = node_value("config", "config");
        invalid_status["data"]["status"] = Value::String("complete".into());
        assert!(serde_json::from_value::<CreativeNode>(invalid_status).is_err());

        let mut invalid_parameters = node_value("config", "config");
        invalid_parameters["data"]["parameters"] = serde_json::json!([]);
        assert!(serde_json::from_value::<CreativeNode>(invalid_parameters).is_err());

        let mut invalid_range = node("panorama", "panorama");
        let CreativeNodeData::Panorama(data) = &mut invalid_range.data else {
            unreachable!()
        };
        data.pitch = 91.0;
        let mut doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        doc.nodes.push(invalid_range);
        assert!(
            doc.validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("pitch")
        );
    }

    #[test]
    fn config_parameters_reject_excessive_json_depth() {
        let mut nested = Value::Bool(true);
        for _ in 0..41 {
            nested = serde_json::json!({ "next": nested });
        }
        let mut config = node("config", "config");
        let CreativeNodeData::Config(data) = &mut config.data else {
            unreachable!()
        };
        data.parameters.insert("nested".into(), nested);
        let mut doc = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        doc.nodes.push(config);
        assert!(
            doc.validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("maximum JSON depth")
        );
    }

    #[test]
    fn graph_rejects_self_duplicate_group_config_and_invalid_director_edges() {
        let cases = [
            ("self", vec![connection("edge-a", "text", "text")], "itself"),
            (
                "duplicate directed edge",
                vec![
                    connection("edge-a", "text", "image"),
                    connection("edge-b", "text", "image"),
                ],
                "duplicates",
            ),
            (
                "group source",
                vec![connection("edge-a", "group", "text")],
                "group nodes",
            ),
            (
                "group target",
                vec![connection("edge-a", "text", "group")],
                "group nodes",
            ),
            (
                "config to config",
                vec![connection("edge-a", "config-a", "config-b")],
                "config to config",
            ),
            (
                "director source",
                vec![connection("edge-a", "director", "image")],
                "director as a source",
            ),
            (
                "invalid director input",
                vec![connection("edge-a", "text", "director")],
                "image or panorama",
            ),
        ];

        for (label, connections, expected) in cases {
            let mut doc = graph_document();
            doc.connections = connections;
            let error = doc.validate_for_project(PROJECT_ID).unwrap_err();
            assert!(
                error.contains(expected),
                "{label} produced unexpected error: {error}"
            );
        }
    }

    #[test]
    fn graph_accepts_image_and_panorama_as_director_inputs() {
        let mut doc = graph_document();
        doc.connections = vec![
            connection("edge-image", "image", "director"),
            connection("edge-panorama", "panorama", "director"),
        ];
        doc.validate_for_project(PROJECT_ID).unwrap();
    }

    #[test]
    fn document_rejects_group_nesting_and_reversed_chat_timestamps() {
        let mut group_nesting = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        let mut outer = node("outer", "group");
        outer.group_id = Some("inner".into());
        group_nesting.nodes = vec![outer, node("inner", "group")];
        assert!(
            group_nesting
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("group nesting")
        );

        let mut reversed_chat = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        reversed_chat.chat_sessions.push(CreativeChatSession {
            id: "0190f5fe-7c00-7a00-8abc-000000000183".into(),
            title: "Chat".into(),
            message_ids: Vec::new(),
            model: None,
            pending_turn: None,
            created_at: 200,
            updated_at: 199,
        });
        assert!(
            reversed_chat
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("updatedAt")
        );
    }

    #[test]
    fn agent_chat_sessions_pin_model_recovery_and_completed_pairs() {
        let chat_id = "0190f5fe-7c00-7a00-8abc-000000000184";
        let mut pending = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        pending.chat_sessions.push(CreativeChatSession {
            id: chat_id.into(),
            title: "Poster".into(),
            message_ids: Vec::new(),
            model: Some(CreativeChatModel {
                provider_id: "0190f5fe-7c00-7a00-8abc-000000000188".into(),
                model: "gpt-5".into(),
            }),
            pending_turn: Some(CreativeChatPendingTurn {
                idempotency_key: "0190f5fe-7c00-7a00-8abc-000000000185".into(),
                prompt: "Create a poster".into(),
                created_at: 20,
            }),
            created_at: 10,
            updated_at: 20,
        });
        pending.active_chat_id = Some(chat_id.into());
        pending.validate_for_project(PROJECT_ID).unwrap();

        let mut completed = pending.clone();
        completed.chat_sessions[0].message_ids = vec![
            "0190f5fe-7c00-7a00-8abc-000000000186".into(),
            "0190f5fe-7c00-7a00-8abc-000000000187".into(),
        ];
        completed.chat_sessions[0].pending_turn = None;
        completed.validate_for_project(PROJECT_ID).unwrap();

        let mut half_pair = completed.clone();
        half_pair.chat_sessions[0].message_ids.pop();
        assert!(
            half_pair
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("user/assistant pairs")
        );

        let mut inactive_pending = pending;
        inactive_pending.active_chat_id = None;
        assert!(
            inactive_pending
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("owning the pending Agent turn")
        );
    }
}
