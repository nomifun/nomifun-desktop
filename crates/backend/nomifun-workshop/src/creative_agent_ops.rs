//! CAS-safe Agent operations over the canonical Creative Studio v1 document.
//!
//! Canonical operations mutate one validated project snapshot and save it
//! through the same revision compare-and-swap path as the product editor. A
//! stale editor or Agent receives a conflict instead of silently overwriting
//! another writer.

use nomifun_common::{CreativeStudioConnectionId, CreativeStudioNodeId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::creative_studio::{
    CreativeConfigNodeData, CreativeGenerationStatus, CreativeNode, CreativeNodeData,
    CreativeNodeType, CreativeProjectDocument,
};

pub const MAX_CREATIVE_AGENT_OPS_PER_CALL: usize = 64;

/// Closed mutations for the canonical project graph. Add-node data must match
/// the exact v1 data object for its selected node type. The server mints node
/// and connection UUIDv7 values.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreativeAgentOp {
    AddNode {
        node_type: CreativeNodeType,
        x: f64,
        y: f64,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        #[serde(default)]
        group_id: Option<String>,
        data: Value,
    },
    /// Shallow-merge canonical data fields. Config task outcome fields are
    /// runtime-owned and cannot be patched by an Agent.
    UpdateNodeData { node_id: String, patch: Value },
    MoveNode { node_id: String, x: f64, y: f64 },
    ResizeNode {
        node_id: String,
        width: f64,
        height: f64,
    },
    DeleteNode { node_id: String },
    Connect {
        source_node_id: String,
        target_node_id: String,
        #[serde(default)]
        source_handle: Option<String>,
        #[serde(default)]
        target_handle: Option<String>,
    },
    Disconnect { connection_id: String },
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CreativeAgentOpResult {
    NodeAdded { node_id: String },
    NodeUpdated { node_id: String },
    NodeMoved { node_id: String },
    NodeResized { node_id: String },
    NodeDeleted {
        node_id: String,
        removed_connections: usize,
    },
    NodesConnected { connection_id: String },
    NodesDisconnected { connection_id: String },
}

fn default_size(node_type: CreativeNodeType) -> (f64, f64) {
    match node_type {
        CreativeNodeType::Image => (320.0, 240.0),
        CreativeNodeType::Panorama => (360.0, 220.0),
        CreativeNodeType::Text => (320.0, 180.0),
        CreativeNodeType::Config => (360.0, 300.0),
        CreativeNodeType::Video => (360.0, 240.0),
        CreativeNodeType::Audio => (360.0, 140.0),
        CreativeNodeType::Director => (400.0, 280.0),
        CreativeNodeType::Group => (480.0, 360.0),
    }
}

fn finite(label: &str, value: f64) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{label} must be finite"))
    }
}

fn node_index(document: &CreativeProjectDocument, node_id: &str) -> Result<usize, String> {
    CreativeStudioNodeId::parse(node_id)
        .map_err(|error| format!("node_id must be a canonical Creative Studio UUIDv7: {error}"))?;
    document
        .nodes
        .iter()
        .position(|node| node.id == node_id)
        .ok_or_else(|| format!("node {node_id} does not exist"))
}

fn mutable_node<'a>(
    document: &'a mut CreativeProjectDocument,
    node_id: &str,
) -> Result<&'a mut CreativeNode, String> {
    let index = node_index(document, node_id)?;
    let node = &mut document.nodes[index];
    if node.locked {
        return Err(format!("node {node_id} is locked"));
    }
    Ok(node)
}

fn object(value: Value, label: &str) -> Result<Map<String, Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} must be a JSON object"))
}

fn parse_node(value: Value, label: &str) -> Result<CreativeNode, String> {
    serde_json::from_value(value).map_err(|error| format!("{label} is invalid: {error}"))
}

fn config_add_is_idle(node: &CreativeNode) -> Result<(), String> {
    let CreativeNodeData::Config(CreativeConfigNodeData {
        task_id,
        result_asset_ids,
        status,
        error_message,
        ..
    }) = &node.data
    else {
        return Ok(());
    };
    if task_id.is_some()
        || !result_asset_ids.is_empty()
        || *status != CreativeGenerationStatus::Idle
        || error_message.is_some()
    {
        return Err(
            "new config nodes must start idle without taskId, results, or errorMessage".to_owned(),
        );
    }
    Ok(())
}

fn reject_server_owned_config_patch(
    node: &CreativeNode,
    patch: &Map<String, Value>,
) -> Result<(), String> {
    if node.node_type != CreativeNodeType::Config {
        return Ok(());
    }
    const SERVER_OWNED: [&str; 4] = ["taskId", "resultAssetIds", "status", "errorMessage"];
    if let Some(key) = SERVER_OWNED.iter().find(|key| patch.contains_key(**key)) {
        return Err(format!("config node field {key} is owned by the task runtime"));
    }
    Ok(())
}

fn apply_one(
    document: &mut CreativeProjectDocument,
    op: CreativeAgentOp,
) -> Result<CreativeAgentOpResult, String> {
    match op {
        CreativeAgentOp::AddNode {
            node_type,
            x,
            y,
            width,
            height,
            group_id,
            data,
        } => {
            finite("add_node.x", x)?;
            finite("add_node.y", y)?;
            if let Some(group_id) = group_id.as_deref() {
                CreativeStudioNodeId::parse(group_id).map_err(|error| {
                    format!(
                        "add_node.group_id must be a canonical Creative Studio UUIDv7: {error}"
                    )
                })?;
            }
            let _ = object(data.clone(), "add_node.data")?;
            let (default_width, default_height) = default_size(node_type);
            let id = CreativeStudioNodeId::new().into_string();
            let z_index = document
                .nodes
                .iter()
                .map(|node| node.z_index)
                .max()
                .unwrap_or(-1)
                .saturating_add(1);
            let node = parse_node(
                json!({
                    "id": id,
                    "type": node_type,
                    "position": { "x": x, "y": y },
                    "size": {
                        "width": width.unwrap_or(default_width),
                        "height": height.unwrap_or(default_height)
                    },
                    "groupId": group_id,
                    "zIndex": z_index,
                    "locked": false,
                    "data": data,
                }),
                "add_node",
            )?;
            config_add_is_idle(&node)?;
            let node_id = node.id.clone();
            document.nodes.push(node);
            Ok(CreativeAgentOpResult::NodeAdded { node_id })
        }
        CreativeAgentOp::UpdateNodeData { node_id, patch } => {
            let patch = object(patch, "update_node_data.patch")?;
            let node = mutable_node(document, &node_id)?;
            reject_server_owned_config_patch(node, &patch)?;
            let mut wire = serde_json::to_value(&*node)
                .map_err(|error| format!("node {node_id} could not be serialized: {error}"))?;
            let data = wire
                .get_mut("data")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| format!("node {node_id} has no canonical data object"))?;
            data.extend(patch);
            *node = parse_node(wire, "update_node_data result")?;
            Ok(CreativeAgentOpResult::NodeUpdated { node_id })
        }
        CreativeAgentOp::MoveNode { node_id, x, y } => {
            finite("move_node.x", x)?;
            finite("move_node.y", y)?;
            let node = mutable_node(document, &node_id)?;
            node.position.x = x;
            node.position.y = y;
            Ok(CreativeAgentOpResult::NodeMoved { node_id })
        }
        CreativeAgentOp::ResizeNode {
            node_id,
            width,
            height,
        } => {
            finite("resize_node.width", width)?;
            finite("resize_node.height", height)?;
            if width < 1.0 || height < 1.0 {
                return Err("resize_node dimensions must be at least 1".to_owned());
            }
            let node = mutable_node(document, &node_id)?;
            node.size.width = width;
            node.size.height = height;
            Ok(CreativeAgentOpResult::NodeResized { node_id })
        }
        CreativeAgentOp::DeleteNode { node_id } => {
            let index = node_index(document, &node_id)?;
            if document.nodes[index].locked {
                return Err(format!("node {node_id} is locked"));
            }
            if let CreativeNodeData::Config(data) = &document.nodes[index].data
                && (data.task_id.is_some()
                    || matches!(
                        data.status,
                        CreativeGenerationStatus::Queued | CreativeGenerationStatus::Running
                    ))
            {
                return Err(format!("config node {node_id} owns a live task"));
            }
            document.nodes.remove(index);
            for node in &mut document.nodes {
                if node.group_id.as_deref() == Some(node_id.as_str()) {
                    node.group_id = None;
                }
            }
            let before = document.connections.len();
            document.connections.retain(|connection| {
                connection.source_node_id != node_id && connection.target_node_id != node_id
            });
            Ok(CreativeAgentOpResult::NodeDeleted {
                node_id,
                removed_connections: before - document.connections.len(),
            })
        }
        CreativeAgentOp::Connect {
            source_node_id,
            target_node_id,
            source_handle,
            target_handle,
        } => {
            node_index(document, &source_node_id)?;
            node_index(document, &target_node_id)?;
            if document.connections.iter().any(|connection| {
                connection.source_node_id == source_node_id
                    && connection.target_node_id == target_node_id
            }) {
                return Err(format!(
                    "connection {source_node_id} -> {target_node_id} already exists"
                ));
            }
            let connection_id = CreativeStudioConnectionId::new().into_string();
            document
                .connections
                .push(crate::creative_studio::CreativeConnection {
                    id: connection_id.clone(),
                    source_node_id,
                    target_node_id,
                    source_handle,
                    target_handle,
                });
            Ok(CreativeAgentOpResult::NodesConnected { connection_id })
        }
        CreativeAgentOp::Disconnect { connection_id } => {
            CreativeStudioConnectionId::parse(&connection_id).map_err(|error| {
                format!(
                    "connection_id must be a canonical Creative Studio UUIDv7: {error}"
                )
            })?;
            let index = document
                .connections
                .iter()
                .position(|connection| connection.id == connection_id)
                .ok_or_else(|| format!("connection {connection_id} does not exist"))?;
            document.connections.remove(index);
            Ok(CreativeAgentOpResult::NodesDisconnected { connection_id })
        }
    }
}

/// Apply a batch to a cloned project snapshot. Any invalid operation or final
/// graph invariant rejects the whole batch; callers save only the returned
/// document.
pub fn apply_creative_agent_ops(
    document: &CreativeProjectDocument,
    ops: Vec<CreativeAgentOp>,
) -> Result<(CreativeProjectDocument, Vec<CreativeAgentOpResult>), String> {
    if ops.is_empty() {
        return Err("no Creative Studio operations provided".to_owned());
    }
    if ops.len() > MAX_CREATIVE_AGENT_OPS_PER_CALL {
        return Err(format!(
            "too many Creative Studio operations: {} (max {MAX_CREATIVE_AGENT_OPS_PER_CALL})",
            ops.len()
        ));
    }
    let mut next = document.clone();
    let mut results = Vec::with_capacity(ops.len());
    for (index, op) in ops.into_iter().enumerate() {
        results.push(apply_one(&mut next, op).map_err(|error| format!("ops[{index}]: {error}"))?);
    }
    next.validate_for_project(&next.project_id)
        .map_err(|error| format!("operation batch produced an invalid project: {error}"))?;
    Ok((next, results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::CreativeStudioProjectId;

    fn document() -> CreativeProjectDocument {
        CreativeProjectDocument::empty(CreativeStudioProjectId::new().into_string())
    }

    fn text_data(text: &str) -> Value {
        json!({
            "text": text,
            "format": "plain",
            "fontSize": 16,
            "textAlign": "left"
        })
    }

    #[test]
    fn applies_canonical_batch_atomically() {
        let (with_nodes, added) = apply_creative_agent_ops(
            &document(),
            vec![
                CreativeAgentOp::AddNode {
                    node_type: CreativeNodeType::Text,
                    x: 10.0,
                    y: 20.0,
                    width: None,
                    height: None,
                    group_id: None,
                    data: text_data("first"),
                },
                CreativeAgentOp::AddNode {
                    node_type: CreativeNodeType::Text,
                    x: 500.0,
                    y: 20.0,
                    width: None,
                    height: None,
                    group_id: None,
                    data: text_data("second"),
                },
            ],
        )
        .unwrap();
        let first = match &added[0] {
            CreativeAgentOpResult::NodeAdded { node_id } => node_id.clone(),
            other => panic!("unexpected result {other:?}"),
        };
        let second = match &added[1] {
            CreativeAgentOpResult::NodeAdded { node_id } => node_id.clone(),
            other => panic!("unexpected result {other:?}"),
        };
        let (connected, results) = apply_creative_agent_ops(
            &with_nodes,
            vec![CreativeAgentOp::Connect {
                source_node_id: first,
                target_node_id: second,
                source_handle: None,
                target_handle: None,
            }],
        )
        .unwrap();
        assert_eq!(connected.nodes.len(), 2);
        assert_eq!(connected.connections.len(), 1);
        assert!(matches!(
            results[0],
            CreativeAgentOpResult::NodesConnected { .. }
        ));
    }

    #[test]
    fn rejects_partial_batch_and_non_idle_config_add() {
        let original = document();
        let error = apply_creative_agent_ops(
            &original,
            vec![
                CreativeAgentOp::AddNode {
                    node_type: CreativeNodeType::Text,
                    x: 10.0,
                    y: 20.0,
                    width: None,
                    height: None,
                    group_id: None,
                    data: text_data("valid first op"),
                },
                CreativeAgentOp::DeleteNode {
                    node_id: CreativeStudioNodeId::new().into_string(),
                },
            ],
        )
        .unwrap_err();
        assert!(error.contains("ops[1]"));
        assert!(original.nodes.is_empty());

        let error = apply_creative_agent_ops(
            &original,
            vec![CreativeAgentOp::AddNode {
                node_type: CreativeNodeType::Config,
                x: 0.0,
                y: 0.0,
                width: None,
                height: None,
                group_id: None,
                data: json!({
                    "task": "image_generation",
                    "capability": "t2i",
                    "providerId": null,
                    "model": null,
                    "prompt": "",
                    "negativePrompt": "",
                    "parameters": {},
                    "inputAssetIds": [],
                    "taskId": CreativeStudioNodeId::new().into_string(),
                    "resultAssetIds": [],
                    "status": "running",
                    "errorMessage": null
                }),
            }],
        )
        .unwrap_err();
        assert!(error.contains("must start idle"));
    }
}
