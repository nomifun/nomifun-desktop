//! Durable-reference validation and rewriting over a workshop canvas doc.
//!
//! Node payload semantics remain frontend-owned. The backend nevertheless owns
//! the durable identity envelope (`wsn_` nodes, `wse_` edges, and declared node
//! references) and must find asset references for export/import/GC.

use std::collections::{BTreeSet, HashMap};

use nomifun_common::{WorkshopEdgeId, WorkshopNodeId};
use serde_json::Value;

/// Asset-id prefix (contract §1). A doc string equal-prefixed with this is an
/// asset reference.
pub(crate) const ASSET_ID_PREFIX: &str = "wsa_";

/// Collect every asset id (`wsa_…`) referenced anywhere in `doc`.
pub(crate) fn collect_asset_refs(doc: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_collect(doc, &mut out);
    out
}

fn walk_collect(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) if s.starts_with(ASSET_ID_PREFIX) => {
            out.insert(s.clone());
        }
        Value::Array(items) => items.iter().for_each(|i| walk_collect(i, out)),
        Value::Object(map) => map.values().for_each(|i| walk_collect(i, out)),
        _ => {}
    }
}

/// Rewrite every asset id in `doc` in place using `remap` (old id → new id).
/// Strings not present in the map are left untouched. Used on import, where
/// every referenced asset is re-registered under a fresh id.
pub(crate) fn remap_asset_ids(doc: &mut Value, remap: &HashMap<String, String>) {
    match doc {
        Value::String(s) => {
            if let Some(new_id) = remap.get(s.as_str()) {
                *s = new_id.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| remap_asset_ids(i, remap)),
        Value::Object(map) => map.values_mut().for_each(|i| remap_asset_ids(i, remap)),
        _ => {}
    }
}

/// Validate the durable identity envelope of a frontend-owned canvas doc.
///
/// This deliberately does not duplicate the complete frontend schema. It only
/// owns identity fields that cross persistence/export/import boundaries, plus
/// referential integrity among those fields.
pub(crate) fn validate_canvas_doc_ids(doc: &Value) -> Result<usize, String> {
    let object = doc
        .as_object()
        .ok_or_else(|| "document must be a JSON object".to_string())?;
    let nodes = object
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "nodes must be an array".to_string())?;
    let edges = object
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| "edges must be an array".to_string())?;

    let mut node_ids = BTreeSet::new();
    let mut node_references: Vec<(String, String)> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let node = node
            .as_object()
            .ok_or_else(|| format!("nodes[{index}] must be an object"))?;
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("nodes[{index}].id must be a string"))?;
        WorkshopNodeId::parse(id)
            .map_err(|error| format!("nodes[{index}].id is not canonical: {error}"))?;
        if !node_ids.insert(id.to_string()) {
            return Err(format!("nodes[{index}].id duplicates '{id}'"));
        }

        if let Some(group_id) = node.get("groupId").filter(|value| !value.is_null()) {
            let group_id = group_id
                .as_str()
                .ok_or_else(|| format!("nodes[{index}].groupId must be a string or null"))?;
            WorkshopNodeId::parse(group_id)
                .map_err(|error| format!("nodes[{index}].groupId is not canonical: {error}"))?;
            node_references.push((format!("nodes[{index}].groupId"), group_id.to_string()));
        }

        if let Some(mentions) = node
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("mentions"))
            .and_then(Value::as_array)
        {
            for (mention_index, mention) in mentions.iter().enumerate() {
                let Some(reference) = mention.as_str().and_then(|value| value.strip_prefix("node:")) else {
                    continue;
                };
                WorkshopNodeId::parse(reference).map_err(|error| {
                    format!("nodes[{index}].data.mentions[{mention_index}] has a non-canonical node reference: {error}")
                })?;
                node_references.push((
                    format!("nodes[{index}].data.mentions[{mention_index}]"),
                    reference.to_string(),
                ));
            }
        }
    }

    for (path, reference) in node_references {
        if !node_ids.contains(&reference) {
            return Err(format!("{path} references missing node '{reference}'"));
        }
    }

    let mut edge_ids = BTreeSet::new();
    for (index, edge) in edges.iter().enumerate() {
        let edge = edge
            .as_object()
            .ok_or_else(|| format!("edges[{index}] must be an object"))?;
        let id = edge
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("edges[{index}].id must be a string"))?;
        WorkshopEdgeId::parse(id)
            .map_err(|error| format!("edges[{index}].id is not canonical: {error}"))?;
        if !edge_ids.insert(id.to_string()) {
            return Err(format!("edges[{index}].id duplicates '{id}'"));
        }
        for field in ["from", "to"] {
            let reference = edge
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edges[{index}].{field} must be a string"))?;
            WorkshopNodeId::parse(reference)
                .map_err(|error| format!("edges[{index}].{field} is not canonical: {error}"))?;
            if !node_ids.contains(reference) {
                return Err(format!("edges[{index}].{field} references missing node '{reference}'"));
            }
        }
    }

    Ok(nodes.len())
}

/// Give every durable document entity a fresh identity when importing a canvas
/// as a clone, and rewrite every declared node reference through one remap.
pub(crate) fn remap_canvas_doc_ids_for_clone(doc: &mut Value) -> Result<(), String> {
    validate_canvas_doc_ids(doc)?;

    let mut node_remap = HashMap::new();
    for node in doc["nodes"].as_array().expect("validated nodes array") {
        let old_id = node["id"].as_str().expect("validated node id");
        node_remap.insert(old_id.to_string(), WorkshopNodeId::new().into_string());
    }

    for node in doc["nodes"].as_array_mut().expect("validated nodes array") {
        let object = node.as_object_mut().expect("validated node object");
        let old_id = object["id"].as_str().expect("validated node id").to_string();
        object.insert(
            "id".to_string(),
            Value::String(node_remap[&old_id].clone()),
        );
        if let Some(group_id) = object
            .get("groupId")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            *object.get_mut("groupId").expect("groupId exists") =
                Value::String(node_remap[&group_id].clone());
        }
        if let Some(mentions) = object
            .get_mut("data")
            .and_then(Value::as_object_mut)
            .and_then(|data| data.get_mut("mentions"))
            .and_then(Value::as_array_mut)
        {
            for mention in mentions {
                let Some(old_reference) = mention.as_str().and_then(|value| value.strip_prefix("node:")) else {
                    continue;
                };
                *mention = Value::String(format!("node:{}", node_remap[old_reference]));
            }
        }
    }

    for edge in doc["edges"].as_array_mut().expect("validated edges array") {
        let object = edge.as_object_mut().expect("validated edge object");
        object.insert(
            "id".to_string(),
            Value::String(WorkshopEdgeId::new().into_string()),
        );
        for field in ["from", "to"] {
            let old_reference = object[field]
                .as_str()
                .expect("validated edge node reference")
                .to_string();
            object.insert(field.to_string(), Value::String(node_remap[&old_reference].clone()));
        }
    }

    validate_canvas_doc_ids(doc).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_doc() -> Value {
        let first = WorkshopNodeId::new().into_string();
        let second = WorkshopNodeId::new().into_string();
        let third = WorkshopNodeId::new().into_string();
        json!({
            "schema": 1,
            "nodes": [
                { "id": first, "kind": "image", "data": { "assetId": "wsa_a", "caption": "hi" } },
                { "id": second, "kind": "generator", "data": {
                    "prompt": "cat",
                    "mentions": ["wsa_b", "wsa_c"],
                    "resultAssetIds": ["wsa_a", "wsa_d"],
                    "status": "idle"
                }},
                { "id": third, "kind": "text", "data": { "content": "no refs here" } }
            ],
            "edges": []
        })
    }

    #[test]
    fn collects_all_distinct_refs() {
        let refs = collect_asset_refs(&sample_doc());
        let got: Vec<&str> = refs.iter().map(String::as_str).collect();
        assert_eq!(got, vec!["wsa_a", "wsa_b", "wsa_c", "wsa_d"]);
    }

    #[test]
    fn ignores_non_asset_strings_and_empty_doc() {
        assert!(collect_asset_refs(&json!({})).is_empty());
        assert!(collect_asset_refs(&json!({ "x": "wscanvas", "y": "awsa_" })).is_empty());
    }

    #[test]
    fn remap_rewrites_only_known_ids() {
        let mut doc = sample_doc();
        let remap: HashMap<String, String> = [
            ("wsa_a".to_string(), "wsa_X".to_string()),
            ("wsa_b".to_string(), "wsa_Y".to_string()),
            ("wsa_c".to_string(), "wsa_Z".to_string()),
            ("wsa_d".to_string(), "wsa_W".to_string()),
        ]
        .into_iter()
        .collect();
        remap_asset_ids(&mut doc, &remap);
        let refs = collect_asset_refs(&doc);
        let got: Vec<&str> = refs.iter().map(String::as_str).collect();
        assert_eq!(got, vec!["wsa_W", "wsa_X", "wsa_Y", "wsa_Z"]);
    }

    #[test]
    fn validates_and_remaps_the_complete_document_identity_envelope() {
        let group_id = WorkshopNodeId::new().into_string();
        let member_id = WorkshopNodeId::new().into_string();
        let edge_id = WorkshopEdgeId::new().into_string();
        let mut doc = json!({
            "nodes": [
                {"id": group_id},
                {"id": member_id, "groupId": group_id, "data": {
                    "mentions": [format!("node:{group_id}")]
                }}
            ],
            "edges": [{"id": edge_id, "from": group_id, "to": member_id}]
        });
        assert_eq!(validate_canvas_doc_ids(&doc), Ok(2));

        remap_canvas_doc_ids_for_clone(&mut doc).unwrap();
        let new_group_id = doc["nodes"][0]["id"].as_str().unwrap();
        let new_member_id = doc["nodes"][1]["id"].as_str().unwrap();
        assert_ne!(new_group_id, group_id);
        assert_ne!(new_member_id, member_id);
        assert_eq!(doc["nodes"][1]["groupId"].as_str(), Some(new_group_id));
        assert_eq!(doc["edges"][0]["from"].as_str(), Some(new_group_id));
        assert_eq!(doc["edges"][0]["to"].as_str(), Some(new_member_id));
        let expected_mention = format!("node:{new_group_id}");
        assert_eq!(
            doc["nodes"][1]["data"]["mentions"][0].as_str(),
            Some(expected_mention.as_str())
        );
        assert_eq!(validate_canvas_doc_ids(&doc), Ok(2));
    }

    #[test]
    fn rejects_noncanonical_or_dangling_document_ids() {
        let node_id = WorkshopNodeId::new().into_string();
        let missing_id = WorkshopNodeId::new().into_string();
        let edge_id = WorkshopEdgeId::new().into_string();
        let legacy = json!({"nodes": [{"id": "legacy"}], "edges": []});
        assert!(validate_canvas_doc_ids(&legacy).unwrap_err().contains("not canonical"));

        let dangling = json!({
            "nodes": [{"id": node_id}],
            "edges": [{"id": edge_id, "from": node_id, "to": missing_id}]
        });
        assert!(validate_canvas_doc_ids(&dangling).unwrap_err().contains("missing node"));
    }
}
