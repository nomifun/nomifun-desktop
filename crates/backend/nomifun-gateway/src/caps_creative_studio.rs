//! Canonical Creative Studio Gateway capabilities.
//!
//! These tools operate on the same v1 project documents, revision CAS, asset
//! service, and idempotent task queue as the product UI. No retired Workshop
//! canvas schema, queue, or unowned task surface is reachable here.

use std::sync::Arc;

use nomifun_common::{
    CreationTaskId, CreativeStudioNodeId, CreativeStudioProjectId,
};
use nomifun_creation::{
    CreationInput, CreativeCreationTask, CreativeTaskOwner, NewCreationTask,
};
use nomifun_workshop::creative_studio::{
    CreativeConfigNodeData, CreativeGenerationStatus, CreativeNode, CreativeNodeData,
    CreativeProjectDocument,
};
use nomifun_workshop::service::AssetQuery;
use nomifun_workshop::CreativeAgentOp;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::ok;

const SUMMARY_TEXT_MAX: usize = 160;
const MAX_SUMMARY_NODES: usize = 200;
const MAX_SUMMARY_CONNECTIONS: usize = 400;
const LIST_PROJECTS_DEFAULT: i64 = 100;
const LIST_PROJECTS_MAX: i64 = 200;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListProjectsParams {
    /// Optional case-insensitive substring filter over project titles.
    #[serde(default)]
    query: Option<String>,
    /// Maximum projects to return (default 100, capped at 200).
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetProjectParams {
    /// Project UUIDv7 from nomi_creative_studio_list_projects.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    project_id: CreativeStudioProjectId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListAssetsParams {
    /// Optional case-insensitive title search.
    #[serde(default)]
    q: Option<String>,
    /// Optional kind filter: image, video, audio, or text.
    #[serde(default)]
    kind: Option<String>,
    /// Maximum rows to return (default 20, capped at 50).
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ApplyOperationsParams {
    /// Target canonical project UUIDv7.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    project_id: CreativeStudioProjectId,
    /// Decimal revision returned by get_project. A stale value returns conflict.
    #[serde(deserialize_with = "deserialize_revision")]
    expected_revision: String,
    /// Ordered all-or-nothing canonical graph mutations.
    ops: Vec<CreativeAgentOp>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GenerateParams {
    /// Project containing the persisted config node.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    project_id: CreativeStudioProjectId,
    /// Config-node UUIDv7. Provider, model, capability, prompt, parameters, and
    /// input assets are read from this node rather than duplicated in tool args.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    node_id: CreativeStudioNodeId,
    /// Decimal revision returned by get_project. The task fence is committed by
    /// CAS before submission.
    #[serde(deserialize_with = "deserialize_revision")]
    expected_revision: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetTaskParams {
    /// Canonical task UUIDv7 returned by nomi_creative_studio_generate.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    creation_task_id: CreationTaskId,
}

fn deserialize_revision<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(serde::de::Error::custom(
            "expected_revision must be a canonical decimal string",
        ));
    }
    value
        .parse::<i64>()
        .map_err(serde::de::Error::custom)
        .and_then(|revision| {
            if revision >= 1 {
                Ok(value)
            } else {
                Err(serde::de::Error::custom(
                    "expected_revision must be at least 1",
                ))
            }
        })
}

fn caller_source(ctx: &CallerCtx) -> String {
    if let Some(companion_id) = &ctx.companion_id {
        format!("companion:{companion_id}")
    } else if let Some(conversation_id) = &ctx.conversation_id {
        format!("conversation:{conversation_id}")
    } else {
        "gateway".to_owned()
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

fn node_asset_ids(node: &CreativeNode) -> Vec<&str> {
    match &node.data {
        CreativeNodeData::Image(data) => data.asset_id.iter().map(String::as_str).collect(),
        CreativeNodeData::Panorama(data) => {
            data.asset_id.iter().map(String::as_str).collect()
        }
        CreativeNodeData::Text(_) | CreativeNodeData::Group(_) => Vec::new(),
        CreativeNodeData::Config(data) => data
            .input_asset_ids
            .iter()
            .chain(data.result_asset_ids.iter())
            .map(String::as_str)
            .collect(),
        CreativeNodeData::Video(data) => data
            .asset_id
            .iter()
            .chain(data.poster_asset_id.iter())
            .map(String::as_str)
            .collect(),
        CreativeNodeData::Audio(data) => data.asset_id.iter().map(String::as_str).collect(),
        CreativeNodeData::Director(data) => {
            data.scene_id.iter().map(String::as_str).collect()
        }
    }
}

fn summarize_node(node: &CreativeNode) -> Value {
    let text = match &node.data {
        CreativeNodeData::Image(data) => {
            let value = if data.caption.is_empty() {
                &data.alt
            } else {
                &data.caption
            };
            (!value.is_empty()).then(|| truncate_chars(value, SUMMARY_TEXT_MAX))
        }
        CreativeNodeData::Text(data) => {
            (!data.text.is_empty()).then(|| truncate_chars(&data.text, SUMMARY_TEXT_MAX))
        }
        CreativeNodeData::Config(data) => {
            (!data.prompt.is_empty()).then(|| truncate_chars(&data.prompt, SUMMARY_TEXT_MAX))
        }
        CreativeNodeData::Audio(data) => {
            (!data.title.is_empty()).then(|| truncate_chars(&data.title, SUMMARY_TEXT_MAX))
        }
        CreativeNodeData::Group(data) => Some(truncate_chars(&data.title, SUMMARY_TEXT_MAX)),
        CreativeNodeData::Panorama(_)
        | CreativeNodeData::Video(_)
        | CreativeNodeData::Director(_) => None,
    };
    let status = match &node.data {
        CreativeNodeData::Config(data) => Some(data.status),
        _ => None,
    };
    json!({
        "id": node.id,
        "type": node.node_type,
        "position": node.position,
        "size": node.size,
        "group_id": node.group_id,
        "locked": node.locked,
        "text": text,
        "status": status,
        "asset_ids": node_asset_ids(node),
    })
}

fn summarize_project(
    project: &nomifun_workshop::CreativeProjectSummary,
    document: &CreativeProjectDocument,
) -> Value {
    let total_nodes = document.nodes.len();
    let total_connections = document.connections.len();
    json!({
        "project_id": project.project_id,
        "title": project.title,
        "revision": project.revision,
        "node_count": project.node_count,
        "connection_count": project.connection_count,
        "updated_at": project.updated_at,
        "nodes": document
            .nodes
            .iter()
            .take(MAX_SUMMARY_NODES)
            .map(summarize_node)
            .collect::<Vec<_>>(),
        "connections": document
            .connections
            .iter()
            .take(MAX_SUMMARY_CONNECTIONS)
            .map(|connection| json!({
                "id": connection.id,
                "source_node_id": connection.source_node_id,
                "target_node_id": connection.target_node_id,
                "source_handle": connection.source_handle,
                "target_handle": connection.target_handle,
            }))
            .collect::<Vec<_>>(),
        "total_nodes": total_nodes,
        "total_connections": total_connections,
        "nodes_truncated": total_nodes > MAX_SUMMARY_NODES,
        "connections_truncated": total_connections > MAX_SUMMARY_CONNECTIONS,
    })
}

async fn list_projects(deps: Arc<GatewayDeps>, params: ListProjectsParams) -> Value {
    let query = params
        .query
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty());
    let limit = params
        .limit
        .unwrap_or(LIST_PROJECTS_DEFAULT)
        .clamp(1, LIST_PROJECTS_MAX) as usize;
    match deps.workshop_service.list_creative_projects().await {
        Ok(projects) => {
            let filtered = projects
                .into_iter()
                .filter(|project| {
                    query
                        .as_deref()
                        .is_none_or(|query| project.title.to_lowercase().contains(query))
                })
                .collect::<Vec<_>>();
            let total = filtered.len();
            ok(json!({
                "total": total,
                "truncated": total > limit,
                "projects": filtered.into_iter().take(limit).collect::<Vec<_>>(),
            }))
        }
        Err(error) => json!({ "error": error.to_string() }),
    }
}

async fn get_project(deps: Arc<GatewayDeps>, params: GetProjectParams) -> Value {
    match deps
        .workshop_service
        .get_creative_project(params.project_id.as_str())
        .await
    {
        Ok(project) => ok(summarize_project(&project.project, &project.document)),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

async fn list_assets(deps: Arc<GatewayDeps>, params: ListAssetsParams) -> Value {
    let query = AssetQuery {
        kind: params.kind.filter(|kind| !kind.trim().is_empty()),
        q: params.q.filter(|query| !query.trim().is_empty()),
        page: 1,
        page_size: params.limit.unwrap_or(20).clamp(1, 50),
        ..Default::default()
    };
    match deps.workshop_service.list_assets(query).await {
        Ok(page) => {
            let items = page
                .items
                .iter()
                .map(|asset| {
                    json!({
                        "asset_id": asset.asset_id,
                        "kind": asset.kind,
                        "title": asset.title,
                        "collection": asset.collection,
                        "tags": asset.tags,
                        "mime": asset.mime,
                        "width": asset.width,
                        "height": asset.height,
                        "in_library": asset.in_library,
                    })
                })
                .collect::<Vec<_>>();
            ok(json!({ "total": page.total, "items": items }))
        }
        Err(error) => json!({ "error": error.to_string() }),
    }
}

async fn apply_operations(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    params: ApplyOperationsParams,
) -> Value {
    let source = caller_source(&ctx);
    match deps
        .workshop_service
        .apply_creative_agent_ops(
            params.project_id.as_str(),
            &params.expected_revision,
            params.ops,
            &source,
        )
        .await
    {
        Ok(applied) => ok(json!({
            "project": applied.project,
            "ops": applied.ops,
        })),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

fn generation_request(
    config: &CreativeConfigNodeData,
) -> Result<(String, String, String, Value, Vec<CreationInput>), String> {
    let provider_id = config
        .provider_id
        .clone()
        .ok_or_else(|| "config node has no selected provider".to_owned())?;
    let model = config
        .model
        .clone()
        .ok_or_else(|| "config node has no selected model".to_owned())?;
    if config.capability.trim().is_empty() {
        return Err("config node has no capability".to_owned());
    }
    let mut params = config.parameters.clone();
    params.insert("prompt".to_owned(), json!(config.prompt));
    if !config.negative_prompt.is_empty() {
        params.insert("negative_prompt".to_owned(), json!(config.negative_prompt));
    }
    let inputs = config
        .input_asset_ids
        .iter()
        .map(|asset_id| CreationInput {
            asset_id: asset_id.clone(),
            role: "reference".to_owned(),
        })
        .collect();
    Ok((
        provider_id,
        model,
        config.capability.clone(),
        Value::Object(params),
        inputs,
    ))
}

async fn generate(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    params: GenerateParams,
) -> Value {
    let Some(operation_id) = ctx.operation_id.as_deref() else {
        return json!({ "error": "Creative Studio generation requires transport operation identity" });
    };
    let operation_id = match CreationTaskId::parse(operation_id) {
        Ok(id) => id.into_string(),
        Err(error) => {
            return json!({
                "error": format!(
                    "Creative Studio generation operation identity is not a canonical UUIDv7: {error}"
                )
            });
        }
    };

    let mut current = match deps
        .workshop_service
        .get_creative_project(params.project_id.as_str())
        .await
    {
        Ok(project) => project,
        Err(error) => return json!({ "error": error.to_string() }),
    };
    let Some(node_index) = current
        .document
        .nodes
        .iter()
        .position(|node| node.id == params.node_id.as_str())
    else {
        return json!({ "error": format!("config node {} does not exist", params.node_id) });
    };
    let config = match &current.document.nodes[node_index].data {
        CreativeNodeData::Config(config) => config.clone(),
        _ => {
            return json!({
                "error": format!("node {} is not a config node", params.node_id)
            });
        }
    };
    let (provider_id, model, capability, task_params, inputs) =
        match generation_request(&config) {
            Ok(request) => request,
            Err(error) => return json!({ "error": error }),
        };

    let already_fenced = config.task_id.as_deref() == Some(operation_id.as_str());
    if !already_fenced {
        if current.project.revision != params.expected_revision {
            return json!({
                "error": format!(
                    "creative studio project {} revision is {}, expected {}",
                    params.project_id, current.project.revision, params.expected_revision
                )
            });
        }
        if config.task_id.is_some()
            && matches!(
                config.status,
                CreativeGenerationStatus::Queued | CreativeGenerationStatus::Running
            )
        {
            return json!({
                "error": format!("config node {} already owns a live task", params.node_id)
            });
        }
        let CreativeNodeData::Config(config) =
            &mut current.document.nodes[node_index].data
        else {
            unreachable!("node kind was checked above");
        };
        config.task_id = Some(operation_id.clone());
        config.result_asset_ids.clear();
        config.status = CreativeGenerationStatus::Queued;
        config.error_message = None;
        if !current.document.pending_task_ids.contains(&operation_id) {
            current.document.pending_task_ids.push(operation_id.clone());
        }
        if let Err(error) = deps
            .workshop_service
            .save_creative_project(
                params.project_id.as_str(),
                &current.project.revision,
                &current.document,
            )
            .await
        {
            return json!({ "error": error.to_string() });
        }
    }

    let task = deps
        .creation_service
        .create_creative_task(
            CreativeTaskOwner::CanvasNode {
                project_id: params.project_id.into_string(),
                node_id: params.node_id.into_string(),
            },
            operation_id,
            NewCreationTask {
                canvas_id: None,
                node_id: None,
                provider_id,
                model,
                capability,
                params: task_params,
                inputs,
            },
        )
        .await;
    match task {
        Ok(task) => match CreativeCreationTask::try_from(task) {
            Ok(task) => ok(json!(task)),
            Err(error) => json!({ "error": error.to_string() }),
        },
        Err(error) => json!({ "error": error.to_string() }),
    }
}

async fn get_task(deps: Arc<GatewayDeps>, params: GetTaskParams) -> Value {
    match deps
        .creation_service
        .get_task(params.creation_task_id.as_str())
        .await
    {
        Ok(task) => match CreativeCreationTask::try_from(task) {
            Ok(task) => ok(json!(task)),
            Err(_) => json!({ "error": "creative task not found" }),
        },
        Err(error) => json!({ "error": error.to_string() }),
    }
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<ListProjectsParams, _, _>(
        CapabilityMeta::new(
            "nomi_creative_studio_list_projects",
            "creative_studio",
            "List canonical Creative Studio projects with revision, node count, and connection count.",
            DangerTier::Read,
        ),
        |deps, _ctx, params| list_projects(deps, params),
    ));
    out.push(Capability::new::<GetProjectParams, _, _>(
        CapabilityMeta::new(
            "nomi_creative_studio_get_project",
            "creative_studio",
            "Read a bounded canonical project graph summary and its revision CAS token.",
            DangerTier::Read,
        ),
        |deps, _ctx, params| get_project(deps, params),
    ));
    out.push(Capability::new::<ListAssetsParams, _, _>(
        CapabilityMeta::new(
            "nomi_creative_studio_list_assets",
            "creative_studio",
            "List Creative Studio assets by title or kind without returning binary bytes.",
            DangerTier::Read,
        ),
        |deps, _ctx, params| list_assets(deps, params),
    ));
    out.push(Capability::new::<ApplyOperationsParams, _, _>(
        CapabilityMeta::new(
            "nomi_creative_studio_apply_ops",
            "creative_studio",
            "Apply an all-or-nothing canonical graph mutation batch using the expected project revision.",
            DangerTier::Write,
        ),
        apply_operations,
    ));
    out.push(Capability::new::<GenerateParams, _, _>(
        CapabilityMeta::new(
            "nomi_creative_studio_generate",
            "creative_studio",
            "Fence and submit an idempotent generation task from a persisted config node using the project revision.",
            DangerTier::Write,
        ),
        generate,
    ));
    out.push(Capability::new::<GetTaskParams, _, _>(
        CapabilityMeta::new(
            "nomi_creative_studio_get_task",
            "creative_studio",
            "Inspect a canonical Creative Studio generation task and its produced asset ids.",
            DangerTier::Read,
        ),
        |deps, _ctx, params| get_task(deps, params),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Registry, Surface};
    use nomifun_workshop::creative_studio::{
        CreativeNodeType, CreativePoint, CreativeSize, CreativeTextAlign,
        CreativeTextFormat, CreativeTextNodeData,
    };

    const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000701";
    const NODE_ID: &str = "0190f5fe-7c00-7a00-8000-000000000702";

    #[test]
    fn project_summary_uses_canonical_nodes_and_connections() {
        let project_id = CreativeStudioProjectId::parse(PROJECT_ID).unwrap();
        let mut document = CreativeProjectDocument::empty(project_id.into_string());
        document.nodes.push(CreativeNode {
            id: NODE_ID.to_owned(),
            node_type: CreativeNodeType::Text,
            position: CreativePoint { x: 1.0, y: 2.0 },
            size: CreativeSize {
                width: 320.0,
                height: 180.0,
            },
            group_id: None,
            z_index: 0,
            locked: false,
            data: CreativeNodeData::Text(CreativeTextNodeData {
                text: "canonical".to_owned(),
                format: CreativeTextFormat::Plain,
                font_size: 16.0,
                text_align: CreativeTextAlign::Left,
            }),
        });
        let project = nomifun_workshop::CreativeProjectSummary {
            project_id: PROJECT_ID.to_owned(),
            title: "Project".to_owned(),
            revision: "4".to_owned(),
            node_count: 1,
            connection_count: 0,
            created_at: 1,
            updated_at: 2,
        };
        let summary = summarize_project(&project, &document);
        assert_eq!(summary["project_id"], PROJECT_ID);
        assert_eq!(summary["revision"], "4");
        assert_eq!(summary["nodes"][0]["type"], "text");
        assert_eq!(summary["nodes"][0]["text"], "canonical");
        assert!(summary.get("canvas_id").is_none());
        assert!(summary.get("edges").is_none());
    }

    #[test]
    fn params_reject_retired_canvas_contracts() {
        assert!(serde_json::from_value::<GetProjectParams>(json!({
            "project_id": PROJECT_ID
        }))
        .is_ok());
        assert!(serde_json::from_value::<GetProjectParams>(json!({
            "canvas_id": PROJECT_ID
        }))
        .is_err());
        assert!(serde_json::from_value::<GenerateParams>(json!({
            "project_id": PROJECT_ID,
            "node_id": NODE_ID,
            "expected_revision": "1"
        }))
        .is_ok());
        assert!(serde_json::from_value::<GenerateParams>(json!({
            "canvas_id": PROJECT_ID,
            "provider_id": PROJECT_ID,
            "model": "legacy"
        }))
        .is_err());
    }

    #[test]
    fn registry_exposes_only_canonical_creative_studio_names() {
        let registry = Registry::global();
        let names = registry
            .tool_specs(Surface::Desktop)
            .iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        for name in [
            "nomi_creative_studio_list_projects",
            "nomi_creative_studio_get_project",
            "nomi_creative_studio_list_assets",
            "nomi_creative_studio_apply_ops",
            "nomi_creative_studio_generate",
            "nomi_creative_studio_get_task",
        ] {
            assert!(names.contains(&name), "missing {name}");
        }
        assert!(!names.iter().any(|name| name.starts_with("nomi_workshop_")));
    }

    #[test]
    fn revision_parser_is_canonical() {
        for valid in ["1", "42", "9223372036854775807"] {
            assert!(serde_json::from_value::<ApplyOperationsParams>(json!({
                "project_id": PROJECT_ID,
                "expected_revision": valid,
                "ops": [{
                    "type": "delete_node",
                    "node_id": NODE_ID
                }]
            }))
            .is_ok());
        }
        for invalid in ["", "0", "01", "-1", " 1", "1 "] {
            assert!(serde_json::from_value::<ApplyOperationsParams>(json!({
                "project_id": PROJECT_ID,
                "expected_revision": invalid,
                "ops": [{
                    "type": "delete_node",
                    "node_id": NODE_ID
                }]
            }))
            .is_err());
        }
    }

    #[test]
    fn list_asset_schema_has_no_binary_or_legacy_canvas_fields() {
        let spec = Registry::global()
            .tool_specs(Surface::Desktop)
            .into_iter()
            .find(|spec| spec.name == "nomi_creative_studio_list_assets")
            .expect("canonical asset capability registered");
        let properties = spec.input_schema["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 3);
        assert!(!properties.contains_key("canvas_id"));
        assert!(!properties.contains_key("bytes"));
    }
}
