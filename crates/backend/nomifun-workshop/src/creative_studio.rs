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
    /// node payload union remains closed to unknown fields; coordinated,
    /// backward-readable optional fields are normalized by both wire parsers.
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
                if let Some(model_input) = pending.model_input.as_deref() {
                    require_trimmed_string(
                        &format!("chatSessions[{index}].pendingTurn.modelInput"),
                        model_input,
                        262_144,
                    )?;
                }
                validate_agent_skill_ids(
                    &format!("chatSessions[{index}].pendingTurn.skillIds"),
                    &pending.skill_ids,
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
    #[serde(default)]
    pub composer: Option<CreativeImageComposerDraft>,
}

impl CreativeImageNodeData {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_optional_id(&format!("{path}.assetId"), self.asset_id.as_deref())?;
        require_string(&format!("{path}.caption"), &self.caption, true, 20_000)?;
        require_string(&format!("{path}.alt"), &self.alt, true, 2_000)?;
        if let Some(size) = self.natural_size {
            size.validate(&format!("{path}.naturalSize"))?;
        }
        if let Some(composer) = &self.composer {
            composer.validate(&format!("{path}.composer"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeImageComposerDraft {
    pub prompt: String,
    pub model: Option<CreativeComposerModel>,
    pub interface_mode: CreativeImageComposerInterfaceMode,
    pub quality: CreativeImageComposerQuality,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub aspect_ratio: String,
    pub count: u8,
}

impl CreativeImageComposerDraft {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_string(&format!("{path}.prompt"), &self.prompt, true, 1_000_000)?;
        if let Some(model) = &self.model {
            model.validate(&format!("{path}.model"))?;
        }
        for (field, value) in [("width", self.width), ("height", self.height)] {
            if value.is_some_and(|value| !(1..=8192).contains(&value)) {
                return Err(format!("{path}.{field} must be between 1 and 8192"));
            }
        }
        if self.width.is_none() != self.height.is_none() {
            return Err(format!(
                "{path}.width and {path}.height must both be null or both be set"
            ));
        }
        require_trimmed_string(
            &format!("{path}.aspectRatio"),
            &self.aspect_ratio,
            128,
        )?;
        if !(1..=10).contains(&self.count) {
            return Err(format!("{path}.count must be between 1 and 10"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeComposerModel {
    pub provider_id: String,
    pub model: String,
}

impl CreativeComposerModel {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_uuidv7(&format!("{path}.providerId"), &self.provider_id)?;
        require_trimmed_string(&format!("{path}.model"), &self.model, 512)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeImageComposerInterfaceMode {
    Images,
    Responses,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CreativeImageComposerQuality {
    Auto,
    High,
    Medium,
    Low,
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

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeConfigNodeData {
    pub task: CreativeModelTask,
    pub capability: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub prompt: String,
    pub negative_prompt: String,
    #[serde(default)]
    pub operation: Option<CreativeConfigOperation>,
    pub parameters: Map<String, Value>,
    pub input_asset_ids: Vec<String>,
    pub task_id: Option<String>,
    pub result_asset_ids: Vec<String>,
    pub status: CreativeGenerationStatus,
    pub error_message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeConfigNodeDataWire {
    task: CreativeModelTask,
    capability: String,
    provider_id: Option<String>,
    model: Option<String>,
    prompt: String,
    negative_prompt: String,
    #[serde(default)]
    operation: Option<CreativeConfigOperation>,
    parameters: Map<String, Value>,
    input_asset_ids: Vec<String>,
    task_id: Option<String>,
    result_asset_ids: Vec<String>,
    status: CreativeGenerationStatus,
    error_message: Option<String>,
}

impl<'de> Deserialize<'de> for CreativeConfigNodeData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut wire = CreativeConfigNodeDataWire::deserialize(deserializer)?;
        if wire.operation.is_some() && wire.parameters.contains_key("canvasOperation") {
            return Err(D::Error::custom(
                "config parameters.canvasOperation must be absent when operation is present",
            ));
        }
        if wire.operation.is_none() {
            wire.operation = normalize_legacy_config_operation(&mut wire.parameters)
                .map_err(D::Error::custom)?;
        }
        Ok(Self {
            task: wire.task,
            capability: wire.capability,
            provider_id: wire.provider_id,
            model: wire.model,
            prompt: wire.prompt,
            negative_prompt: wire.negative_prompt,
            operation: wire.operation,
            parameters: wire.parameters,
            input_asset_ids: wire.input_asset_ids,
            task_id: wire.task_id,
            result_asset_ids: wire.result_asset_ids,
            status: wire.status,
            error_message: wire.error_message,
        })
    }
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
        if let Some(operation) = &self.operation {
            operation.validate(&format!("{path}.operation"))?;
        }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreativeConfigOperation {
    ImageNodeCompose {
        source_node_id: String,
        source_asset_id: Option<String>,
    },
    ImageMaskEdit {
        source_node_id: String,
        source_asset_id: String,
        marked_reference_asset_id: String,
    },
    VideoNodeCompose {
        source_node_id: String,
        source_asset_id: Option<String>,
    },
    AudioNodeCompose {
        source_node_id: String,
        source_asset_id: Option<String>,
    },
}

impl CreativeConfigOperation {
    fn validate(&self, path: &str) -> Result<(), String> {
        match self {
            Self::ImageNodeCompose {
                source_node_id,
                source_asset_id,
            }
            | Self::VideoNodeCompose {
                source_node_id,
                source_asset_id,
            }
            | Self::AudioNodeCompose {
                source_node_id,
                source_asset_id,
            } => {
                require_id(&format!("{path}.sourceNodeId"), source_node_id)?;
                require_optional_id(
                    &format!("{path}.sourceAssetId"),
                    source_asset_id.as_deref(),
                )
            }
            Self::ImageMaskEdit {
                source_node_id,
                source_asset_id,
                marked_reference_asset_id,
            } => {
                require_id(&format!("{path}.sourceNodeId"), source_node_id)?;
                require_id(&format!("{path}.sourceAssetId"), source_asset_id)?;
                require_id(
                    &format!("{path}.markedReferenceAssetId"),
                    marked_reference_asset_id,
                )
            }
        }
    }
}

fn take_legacy_config_string(
    parameters: &mut Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    match parameters.remove(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(format!(
            "legacy config parameters.{key} must be a non-empty string"
        )),
    }
}

fn take_legacy_config_optional_string(
    parameters: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match parameters.remove(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        Some(Value::Null) => Ok(None),
        _ => Err(format!(
            "legacy config parameters.{key} must be null or a non-empty string"
        )),
    }
}

fn normalize_legacy_config_operation(
    parameters: &mut Map<String, Value>,
) -> Result<Option<CreativeConfigOperation>, String> {
    let Some(kind) = parameters.remove("canvasOperation") else {
        return Ok(None);
    };
    let Value::String(kind) = kind else {
        return Err("legacy config parameters.canvasOperation must be a string".into());
    };
    let source_node_id = take_legacy_config_string(parameters, "sourceNodeId")?;
    let source_asset_id = take_legacy_config_optional_string(parameters, "sourceAssetId")?;
    let operation = match kind.as_str() {
        "image-node-compose" => CreativeConfigOperation::ImageNodeCompose {
            source_node_id,
            source_asset_id,
        },
        "video-node-compose" => CreativeConfigOperation::VideoNodeCompose {
            source_node_id,
            source_asset_id,
        },
        "audio-node-compose" => CreativeConfigOperation::AudioNodeCompose {
            source_node_id,
            source_asset_id,
        },
        "image-mask-edit" => CreativeConfigOperation::ImageMaskEdit {
            source_node_id,
            source_asset_id: source_asset_id.ok_or_else(|| {
                "legacy image-mask-edit sourceAssetId must not be null".to_owned()
            })?,
            marked_reference_asset_id: take_legacy_config_string(
                parameters,
                "markedReferenceAssetId",
            )?,
        },
        _ => return Err(format!("unsupported legacy config canvasOperation {kind:?}")),
    };
    for key in ["userPrompt", "referenceWidth", "referenceHeight"] {
        parameters.remove(key);
    }
    Ok(Some(operation))
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
    #[serde(default)]
    pub composer: Option<CreativeVideoComposerDraft>,
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
        if let Some(composer) = &self.composer {
            composer.validate(&format!("{path}.composer"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeVideoComposerDraft {
    pub prompt: String,
    pub model: Option<CreativeComposerModel>,
    pub resolution: String,
    pub aspect_ratio: String,
    pub seconds: u16,
}

impl CreativeVideoComposerDraft {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_string(&format!("{path}.prompt"), &self.prompt, true, 1_000_000)?;
        if let Some(model) = &self.model {
            model.validate(&format!("{path}.model"))?;
        }
        require_trimmed_string(&format!("{path}.resolution"), &self.resolution, 128)?;
        require_trimmed_string(&format!("{path}.aspectRatio"), &self.aspect_ratio, 128)?;
        if !(1..=3_600).contains(&self.seconds) {
            return Err(format!("{path}.seconds must be between 1 and 3600"));
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
    #[serde(default)]
    pub composer: Option<CreativeAudioComposerDraft>,
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
        if let Some(composer) = &self.composer {
            composer.validate(&format!("{path}.composer"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeAudioComposerDraft {
    pub prompt: String,
    pub model: Option<CreativeComposerModel>,
    pub voice: String,
    pub format: String,
}

impl CreativeAudioComposerDraft {
    fn validate(&self, path: &str) -> Result<(), String> {
        require_string(&format!("{path}.prompt"), &self.prompt, true, 1_000_000)?;
        if let Some(model) = &self.model {
            model.validate(&format!("{path}.model"))?;
        }
        require_string(&format!("{path}.voice"), &self.voice, true, 256)?;
        if self.voice.trim() != self.voice {
            return Err(format!("{path}.voice must be trimmed"));
        }
        if !matches!(self.format.as_str(), "mp3" | "wav") {
            return Err(format!("{path}.format must be mp3 or wav"));
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
    #[serde(default)]
    pub model_input: Option<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
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
    Templates,
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

/// Product-facing Canvas document. The persisted project document remains the
/// compatibility boundary for the SQLite repository, while canonical Canvas
/// HTTP/archive wires expose `canvasId` instead of the legacy `projectId`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeCanvasDocument {
    pub schema: String,
    pub canvas_id: String,
    pub viewport: CreativeViewport,
    pub background: CreativeBackground,
    pub nodes: Vec<CreativeNode>,
    pub connections: Vec<CreativeConnection>,
    pub chat_sessions: Vec<CreativeChatSession>,
    pub active_chat_id: Option<String>,
    pub panels: CreativePanels,
    pub pending_task_ids: Vec<String>,
}

impl CreativeCanvasDocument {
    pub fn empty(canvas_id: String) -> Self {
        Self::from(CreativeProjectDocument::empty(canvas_id))
    }

    pub fn validate_for_canvas(&self, expected_canvas_id: &str) -> Result<(), String> {
        self.clone()
            .into_project_document()
            .validate_for_project(expected_canvas_id)
    }

    pub fn into_project_document(self) -> CreativeProjectDocument {
        CreativeProjectDocument {
            schema: self.schema,
            project_id: self.canvas_id,
            viewport: self.viewport,
            background: self.background,
            nodes: self.nodes,
            connections: self.connections,
            chat_sessions: self.chat_sessions,
            active_chat_id: self.active_chat_id,
            panels: self.panels,
            pending_task_ids: self.pending_task_ids,
        }
    }

    pub fn as_project_document(&self) -> CreativeProjectDocument {
        self.clone().into_project_document()
    }
}

impl From<CreativeProjectDocument> for CreativeCanvasDocument {
    fn from(document: CreativeProjectDocument) -> Self {
        Self {
            schema: document.schema,
            canvas_id: document.project_id,
            viewport: document.viewport,
            background: document.background,
            nodes: document.nodes,
            connections: document.connections,
            chat_sessions: document.chat_sessions,
            active_chat_id: document.active_chat_id,
            panels: document.panels,
            pending_task_ids: document.pending_task_ids,
        }
    }
}

impl From<&CreativeProjectDocument> for CreativeCanvasDocument {
    fn from(document: &CreativeProjectDocument) -> Self {
        Self::from(document.clone())
    }
}

impl From<CreativeCanvasDocument> for CreativeProjectDocument {
    fn from(document: CreativeCanvasDocument) -> Self {
        document.into_project_document()
    }
}

impl From<&CreativeCanvasDocument> for CreativeProjectDocument {
    fn from(document: &CreativeCanvasDocument) -> Self {
        document.as_project_document()
    }
}

/// Product-facing summary. `project_id` remains available only on the
/// compatibility type above; canonical Canvas responses have no Project wire.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreativeCanvasSummary {
    pub canvas_id: String,
    pub title: String,
    /// Decimal string on the wire: callers round-trip it as an opaque CAS token.
    pub revision: String,
    pub node_count: i64,
    pub connection_count: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<CreativeProjectSummary> for CreativeCanvasSummary {
    fn from(summary: CreativeProjectSummary) -> Self {
        Self {
            canvas_id: summary.project_id,
            title: summary.title,
            revision: summary.revision,
            node_count: summary.node_count,
            connection_count: summary.connection_count,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

impl From<CreativeStudioProjectRow> for CreativeCanvasSummary {
    fn from(row: CreativeStudioProjectRow) -> Self {
        Self::from(CreativeProjectSummary::from(row))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreativeCanvasDetail {
    pub canvas: CreativeCanvasSummary,
    pub document: CreativeCanvasDocument,
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

fn validate_agent_skill_ids(path: &str, values: &[String]) -> Result<(), String> {
    if values.len() > 8 {
        return Err(format!("{path} must contain at most 8 skill ids"));
    }
    let mut unique = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        require_trimmed_string(&item_path, value, 128)?;
        if !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
        }) {
            return Err(format!(
                "{item_path} must contain only ASCII letters, digits, dots, underscores, or hyphens"
            ));
        }
        if !unique.insert(value.as_str()) {
            return Err(format!("{path} must contain unique skill ids"));
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
                "naturalSize": { "width": 1920, "height": 1080 },
                "composer": {
                    "prompt": "draw a fox",
                    "model": {
                        "providerId": "0190f5fe-7c00-7a00-8abc-000000000188",
                        "model": "image-model-v1"
                    },
                    "interfaceMode": "images",
                    "quality": "high",
                    "width": 1536,
                    "height": 1024,
                    "aspectRatio": "3:2",
                    "count": 2
                }
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
                "trimEndMs": 2100,
                "composer": {
                    "prompt": "slow camera move",
                    "model": {
                        "providerId": "0190f5fe-7c00-7a00-8abc-000000000189",
                        "model": "video-model-v1"
                    },
                    "resolution": "1080p",
                    "aspectRatio": "16:9",
                    "seconds": 5
                }
            }),
            "audio" => serde_json::json!({
                "assetId": "asset-audio",
                "title": "soundtrack",
                "loop": false,
                "volume": 0.75,
                "trimStartMs": 0,
                "trimEndMs": null,
                "composer": {
                    "prompt": "Welcome to NomiFun",
                    "model": {
                        "providerId": "0190f5fe-7c00-7a00-8abc-000000000190",
                        "model": "speech-model-v1"
                    },
                    "voice": "alloy",
                    "format": "mp3"
                }
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

    fn pending_agent_document() -> CreativeProjectDocument {
        let chat_id = "0190f5fe-7c00-7a00-8abc-000000000184";
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.chat_sessions.push(CreativeChatSession {
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
                model_input: None,
                skill_ids: Vec::new(),
                created_at: 20,
            }),
            created_at: 10,
            updated_at: 20,
        });
        document.active_chat_id = Some(chat_id.into());
        document
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
    fn canvas_document_facade_uses_canvas_id_and_maps_storage_document() {
        let project = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        let canvas = CreativeCanvasDocument::from(project.clone());
        canvas.validate_for_canvas(PROJECT_ID).unwrap();

        let value = serde_json::to_value(&canvas).unwrap();
        assert_eq!(value["canvasId"], PROJECT_ID);
        assert!(value.get("projectId").is_none());
        assert_eq!(
            CreativeCanvasDocument::from(project),
            serde_json::from_value(value.clone()).unwrap()
        );

        let mut legacy = value;
        legacy["projectId"] = legacy["canvasId"].take();
        assert!(serde_json::from_value::<CreativeCanvasDocument>(legacy).is_err());

        let summary = CreativeProjectSummary {
            project_id: PROJECT_ID.into(),
            title: "Canvas".into(),
            revision: "1".into(),
            node_count: 0,
            connection_count: 0,
            created_at: 100,
            updated_at: 200,
        };
        let canvas_summary: CreativeCanvasSummary = summary.into();
        let summary_value = serde_json::to_value(canvas_summary).unwrap();
        assert_eq!(summary_value["canvasId"], PROJECT_ID);
        assert!(summary_value.get("projectId").is_none());
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
    fn image_composer_draft_is_strict_but_old_v1_images_default_to_none() {
        let mut old_image = node_value("old-image", "image");
        old_image["data"].as_object_mut().unwrap().remove("composer");
        let old_image: CreativeNode = serde_json::from_value(old_image).unwrap();
        let CreativeNodeData::Image(old_data) = &old_image.data else {
            unreachable!()
        };
        assert_eq!(old_data.composer, None);
        assert_eq!(serde_json::to_value(old_image).unwrap()["data"]["composer"], Value::Null);

        let mut invalid_count = node("invalid-composer", "image");
        let CreativeNodeData::Image(data) = &mut invalid_count.data else {
            unreachable!()
        };
        data.composer.as_mut().unwrap().count = 11;
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(invalid_count);
        assert!(document
            .validate_for_project(PROJECT_ID)
            .unwrap_err()
            .contains("composer.count"));

        let mut partial_model = node_value("partial-model", "image");
        partial_model["data"]["composer"]["model"]
            .as_object_mut()
            .unwrap()
            .remove("providerId");
        assert!(serde_json::from_value::<CreativeNode>(partial_model).is_err());

        let mut partial_dimensions = node("partial-dimensions", "image");
        let CreativeNodeData::Image(data) = &mut partial_dimensions.data else {
            unreachable!()
        };
        let composer = data.composer.as_mut().unwrap();
        composer.width = None;
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(partial_dimensions);
        assert!(document
            .validate_for_project(PROJECT_ID)
            .unwrap_err()
            .contains("must both be null or both be set"));

        let mut padded_model = node("padded-model", "image");
        let CreativeNodeData::Image(data) = &mut padded_model.data else {
            unreachable!()
        };
        data.composer
            .as_mut()
            .unwrap()
            .model
            .as_mut()
            .unwrap()
            .model = " image-model-v1 ".into();
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(padded_model);
        assert!(document
            .validate_for_project(PROJECT_ID)
            .unwrap_err()
            .contains("must be trimmed"));

        let mut unknown_nested = node_value("unknown-nested", "image");
        unknown_nested["data"]["composer"]["legacySetting"] = Value::Bool(true);
        assert!(serde_json::from_value::<CreativeNode>(unknown_nested).is_err());
    }

    #[test]
    fn video_composer_draft_is_strict_but_old_v1_videos_default_to_none() {
        let mut old_video = node_value("old-video", "video");
        old_video["data"].as_object_mut().unwrap().remove("composer");
        let old_video: CreativeNode = serde_json::from_value(old_video).unwrap();
        let CreativeNodeData::Video(old_data) = &old_video.data else {
            unreachable!()
        };
        assert_eq!(old_data.composer, None);
        assert_eq!(
            serde_json::to_value(old_video).unwrap()["data"]["composer"],
            Value::Null
        );

        let mut invalid_seconds = node("invalid-video-composer", "video");
        let CreativeNodeData::Video(data) = &mut invalid_seconds.data else {
            unreachable!()
        };
        data.composer.as_mut().unwrap().seconds = 0;
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(invalid_seconds);
        assert!(document
            .validate_for_project(PROJECT_ID)
            .unwrap_err()
            .contains("composer.seconds"));

        let mut unknown_nested = node_value("unknown-video-composer", "video");
        unknown_nested["data"]["composer"]["legacySetting"] = Value::Bool(true);
        assert!(serde_json::from_value::<CreativeNode>(unknown_nested).is_err());
    }

    #[test]
    fn audio_composer_draft_is_strict_but_old_v1_audio_defaults_to_none() {
        let mut old_audio = node_value("old-audio", "audio");
        old_audio["data"].as_object_mut().unwrap().remove("composer");
        let old_audio: CreativeNode = serde_json::from_value(old_audio).unwrap();
        let CreativeNodeData::Audio(old_data) = &old_audio.data else {
            unreachable!()
        };
        assert_eq!(old_data.composer, None);
        assert_eq!(
            serde_json::to_value(old_audio).unwrap()["data"]["composer"],
            Value::Null
        );

        let mut invalid_format = node("invalid-audio-format", "audio");
        let CreativeNodeData::Audio(data) = &mut invalid_format.data else {
            unreachable!()
        };
        data.composer.as_mut().unwrap().format = "aac".into();
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(invalid_format);
        assert!(
            document
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("composer.format must be mp3 or wav")
        );

        let mut oversized_voice = node("oversized-audio-voice", "audio");
        let CreativeNodeData::Audio(data) = &mut oversized_voice.data else {
            unreachable!()
        };
        data.composer.as_mut().unwrap().voice = "v".repeat(257);
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(oversized_voice);
        assert!(
            document
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("composer.voice")
        );

        let mut untrimmed_voice = node("untrimmed-audio-voice", "audio");
        let CreativeNodeData::Audio(data) = &mut untrimmed_voice.data else {
            unreachable!()
        };
        data.composer.as_mut().unwrap().voice = " alloy ".into();
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.to_owned());
        document.nodes.push(untrimmed_voice);
        assert!(
            document
                .validate_for_project(PROJECT_ID)
                .unwrap_err()
                .contains("composer.voice must be trimmed")
        );

        let mut partial_model = node_value("partial-audio-model", "audio");
        partial_model["data"]["composer"]["model"]
            .as_object_mut()
            .unwrap()
            .remove("providerId");
        assert!(serde_json::from_value::<CreativeNode>(partial_model).is_err());

        let mut unknown_nested = node_value("unknown-audio-composer", "audio");
        unknown_nested["data"]["composer"]["legacySetting"] = Value::Bool(true);
        assert!(serde_json::from_value::<CreativeNode>(unknown_nested).is_err());
    }

    #[test]
    fn legacy_canvas_operation_is_normalized_out_of_provider_parameters() {
        let mut legacy = node_value("legacy-mask", "config");
        legacy["data"]["parameters"] = serde_json::json!({
            "prompt": "provider prompt",
            "width": 1024,
            "canvasOperation": "image-mask-edit",
            "sourceNodeId": "image-source",
            "sourceAssetId": "asset-source",
            "markedReferenceAssetId": "asset-mask",
            "userPrompt": "local prompt",
            "referenceWidth": 1024,
            "referenceHeight": 1024
        });
        let parsed: CreativeNode = serde_json::from_value(legacy.clone()).unwrap();
        let CreativeNodeData::Config(data) = &parsed.data else {
            unreachable!()
        };
        assert_eq!(
            data.operation,
            Some(CreativeConfigOperation::ImageMaskEdit {
                source_node_id: "image-source".into(),
                source_asset_id: "asset-source".into(),
                marked_reference_asset_id: "asset-mask".into(),
            })
        );
        assert_eq!(
            data.parameters,
            serde_json::json!({ "prompt": "provider prompt", "width": 1024 })
                .as_object()
                .unwrap()
                .clone()
        );

        legacy["data"]["operation"] = serde_json::json!({
            "kind": "image-mask-edit",
            "sourceNodeId": "image-source",
            "sourceAssetId": "asset-source",
            "markedReferenceAssetId": "asset-mask"
        });
        assert!(serde_json::from_value::<CreativeNode>(legacy).is_err());

        let mut legacy_audio = node_value("legacy-audio", "config");
        legacy_audio["data"]["parameters"] = serde_json::json!({
            "prompt": "literal narration",
            "voice": "alloy",
            "format": "mp3",
            "canvasOperation": "audio-node-compose",
            "sourceNodeId": "audio-source",
            "sourceAssetId": null
        });
        let parsed: CreativeNode = serde_json::from_value(legacy_audio).unwrap();
        let CreativeNodeData::Config(data) = &parsed.data else {
            unreachable!()
        };
        assert_eq!(
            data.operation,
            Some(CreativeConfigOperation::AudioNodeCompose {
                source_node_id: "audio-source".into(),
                source_asset_id: None,
            })
        );
        assert_eq!(
            data.parameters,
            serde_json::json!({
                "prompt": "literal narration",
                "voice": "alloy",
                "format": "mp3"
            })
            .as_object()
            .unwrap()
            .clone()
        );
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
        let pending = pending_agent_document();
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

    #[test]
    fn pending_agent_planning_input_defaults_old_wire_and_round_trips_new_wire() {
        let mut document = pending_agent_document();
        let pending = document.chat_sessions[0].pending_turn.as_mut().unwrap();
        pending.model_input = Some("Use the selected canvas context".into());
        pending.skill_ids = vec!["canvas.inspect".into(), "asset-search_v2".into()];
        document.validate_for_project(PROJECT_ID).unwrap();

        let value = serde_json::to_value(&document).unwrap();
        assert_eq!(
            value["chatSessions"][0]["pendingTurn"]["modelInput"],
            "Use the selected canvas context"
        );
        assert_eq!(
            value["chatSessions"][0]["pendingTurn"]["skillIds"],
            serde_json::json!(["canvas.inspect", "asset-search_v2"])
        );
        let round_trip: CreativeProjectDocument = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(round_trip, document);

        let mut old_wire = value;
        let old_pending = old_wire["chatSessions"][0]["pendingTurn"]
            .as_object_mut()
            .unwrap();
        old_pending.remove("modelInput");
        old_pending.remove("skillIds");
        let old_document: CreativeProjectDocument = serde_json::from_value(old_wire).unwrap();
        old_document.validate_for_project(PROJECT_ID).unwrap();
        let old_pending = old_document.chat_sessions[0]
            .pending_turn
            .as_ref()
            .unwrap();
        assert_eq!(old_pending.model_input, None);
        assert!(old_pending.skill_ids.is_empty());

        let normalized_wire = serde_json::to_value(old_document).unwrap();
        assert!(normalized_wire["chatSessions"][0]["pendingTurn"]
            .as_object()
            .unwrap()
            .contains_key("modelInput"));
        assert_eq!(
            normalized_wire["chatSessions"][0]["pendingTurn"]["modelInput"],
            Value::Null
        );
        assert_eq!(
            normalized_wire["chatSessions"][0]["pendingTurn"]["skillIds"],
            serde_json::json!([])
        );
    }

    #[test]
    fn pending_agent_planning_input_enforces_bounds_ascii_and_uniqueness() {
        let mut boundary = pending_agent_document();
        let pending = boundary.chat_sessions[0].pending_turn.as_mut().unwrap();
        pending.model_input = Some("x".repeat(262_144));
        pending.skill_ids = vec!["a".repeat(128); 1];
        boundary.validate_for_project(PROJECT_ID).unwrap();

        let cases = [
            ("empty model input", Some(String::new()), Vec::new(), "modelInput"),
            (
                "padded model input",
                Some(" context ".into()),
                Vec::new(),
                "modelInput",
            ),
            (
                "long model input",
                Some("x".repeat(262_145)),
                Vec::new(),
                "modelInput",
            ),
            (
                "too many skills",
                None,
                (0..9).map(|index| format!("skill-{index}")).collect(),
                "at most 8",
            ),
            (
                "long skill id",
                None,
                vec!["a".repeat(129)],
                "skillIds[0]",
            ),
            (
                "padded skill id",
                None,
                vec![" skill".into()],
                "skillIds[0]",
            ),
            (
                "non ascii skill id",
                None,
                vec!["canvas.检查".into()],
                "skillIds[0]",
            ),
            (
                "invalid skill punctuation",
                None,
                vec!["canvas/read".into()],
                "skillIds[0]",
            ),
            (
                "duplicate skill id",
                None,
                vec!["canvas.read".into(), "canvas.read".into()],
                "unique skill ids",
            ),
        ];

        for (label, model_input, skill_ids, expected) in cases {
            let mut document = pending_agent_document();
            let pending = document.chat_sessions[0].pending_turn.as_mut().unwrap();
            pending.model_input = model_input;
            pending.skill_ids = skill_ids;
            let error = document.validate_for_project(PROJECT_ID).unwrap_err();
            assert!(
                error.contains(expected),
                "{label} produced unexpected error: {error}"
            );
        }

        let mut unknown = serde_json::to_value(pending_agent_document()).unwrap();
        unknown["chatSessions"][0]["pendingTurn"]["opaqueContext"] =
            Value::String("forbidden".into());
        assert!(serde_json::from_value::<CreativeProjectDocument>(unknown).is_err());
    }
}
