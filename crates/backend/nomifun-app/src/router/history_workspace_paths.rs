//! Transient paths for canonical history attachments. Historical facts remain unchanged.
use super::{NomiCoreSessionOwner, agent_session_store_error};
use nomifun_agent_contracts::AgentSessionId;
use nomifun_api_types::MessageResponse;
use nomifun_common::AppError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn collect_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            // Attachment markers contain one exact path per line. Structured
            // image/tool paths are already one whole string. Neither case
            // rewrites prose, historical arguments or canonical payloads.
            for candidate in text.lines().map(str::trim) {
                if candidate.len() <= 4096 && std::path::Path::new(candidate).is_absolute() {
                    paths.insert(candidate.to_owned());
                }
            }
        }
        Value::Array(values) => for value in values { collect_paths(value, paths); },
        Value::Object(values) => for (key, value) in values {
            if key != "managed_file_paths" { collect_paths(value, paths); }
        },
        _ => {}
    }
}

pub(super) async fn hydrate(owner: &NomiCoreSessionOwner, session_id: &AgentSessionId,
    created_at: i64, messages: &mut [MessageResponse]) -> Result<(), AppError> {
    let mut candidates: BTreeMap<u64, BTreeSet<String>> = BTreeMap::new();
    for message in messages.iter() {
        // created_at is the documented stable Session sequence cursor; the
        // separate display_at_ms clock is not authority for resource paths.
        let seq = u64::try_from(message.created_at.checked_sub(created_at)
            .ok_or_else(|| AppError::Conflict("historical path source cursor overflow".into()))?)
            .map_err(|_| AppError::Conflict("historical path source precedes its Session".into()))?;
        collect_paths(&message.content, candidates.entry(seq).or_default());
    }
    let resolved = owner.canonical().store().managed_history_file_paths(
        session_id, &owner.managed_workspace_root, &candidates,
    ).await.map_err(agent_session_store_error)?;
    for message in messages {
        let seq = u64::try_from(message.created_at - created_at)
            .map_err(|_| AppError::Conflict("historical path source precedes its Session".into()))?;
        if let Some(content) = message.content.as_object_mut() {
            content.insert("managed_file_paths".into(), serde_json::to_value(resolved.get(&seq)
                .cloned().unwrap_or_default()).map_err(|error| AppError::Internal(error.to_string()))?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn history_workspace_paths_collect_exact_attachment_image_and_tool_paths_without_rewriting_facts() {
        let root = std::env::temp_dir();
        let attachment = root.join("uploads").join("report_nomifun_1234567890123.png").to_string_lossy().into_owned();
        let image = root.join("image.png").to_string_lossy().into_owned();
        let tool = root.join("result.txt").to_string_lossy().into_owned();
        let content = json!({"content":format!("历史原文\n[[NOMI_FILES]]\n{attachment}"),
            "observations":[{"image":{"path":image}}],"args":{"file_path":tool},
            "artifacts":[{"path":tool}],"managed_file_paths":{"ignored":root.join("old").to_string_lossy()}});
        let original = content.clone();
        let mut paths = BTreeSet::new();
        collect_paths(&content, &mut paths);
        assert_eq!(paths, BTreeSet::from([attachment, image, tool]));
        assert_eq!(content, original);
    }
}
