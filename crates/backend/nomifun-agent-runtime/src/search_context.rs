//! Only canonical workspace.files/search envelopes can request instruction discovery.
use crate::{AgentEngineError, AgentToolResult};
use nomifun_chat_model_broker::{ChatToolCall, ChatToolResultPart};
use std::collections::BTreeSet;

const MAX_INSTRUCTION_DISCOVERY_HIT_PATHS: usize = 64;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchResponse {
    query: String,
    matches: Vec<SearchHit>,
    truncated: bool,
    incomplete_reasons: BTreeSet<String>,
    files_scanned: u64,
    files_skipped: u64,
    source_bytes_read: u64,
    notice: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchHit {
    path: String,
    line: u64,
    column_bytes: u64,
    byte_offset: u64,
    sha256: String,
    text: String,
    text_start_column_bytes: u64,
    truncated: bool,
}

pub(super) fn hit_paths(
    call: &ChatToolCall,
    result: &AgentToolResult,
) -> Result<BTreeSet<String>, AgentEngineError> {
    let invalid =
        || AgentEngineError::WorkspaceContext("invalid bounded workspace.files/search response".into());
    // Do not discard media or join arbitrary parts into trusted search JSON.
    let [ChatToolResultPart::Text { text }] = result.output.as_slice() else {
        return Err(invalid());
    };
    if text.len() > 64 * 1024 {
        return Err(invalid());
    }
    let response: SearchResponse = serde_json::from_str(text).map_err(|_| invalid())?;
    let query = call
        .arguments
        .0
        .get("query")
        .and_then(|value| value.as_str())
        .ok_or_else(invalid)?;
    let limit = match call.arguments.0.get("limit") {
        None | Some(serde_json::Value::Null) => 100,
        Some(value) => value.as_u64().ok_or_else(invalid)?,
    };
    let root = match call.arguments.0.get("path") {
        None | Some(serde_json::Value::Null) => "",
        Some(value) => value.as_str().ok_or_else(invalid)?,
    };
    let root = crate::agents_md::normalize_workspace_directory(root)?;
    if query.trim().is_empty()
        || query.len() > 4096
        || query.chars().count() > 1024
        || query.contains(['\r', '\n'])
        || !(1..=200).contains(&limit)
        || response.query != query
        || response.matches.len() as u64 > limit
        || response.truncated != !response.incomplete_reasons.is_empty()
        || response.incomplete_reasons.len() > 32
        || response
            .incomplete_reasons
            .iter()
            .any(|reason| reason.len() > 128 || reason.chars().any(char::is_control))
        || response.files_scanned > 2048
        || response.files_skipped > 20_000
        || response.source_bytes_read > 64 * 1024 * 1024
        || response.notice.len() > 4096
    {
        return Err(invalid());
    }
    let mut paths = BTreeSet::new();
    for hit in response.matches {
        if hit.path.is_empty()
            || crate::agents_md::normalize_workspace_directory(&hit.path)? != hit.path
            || !(root.is_empty()
                || hit.path == root
                || hit
                    .path
                    .strip_prefix(&root)
                    .is_some_and(|suffix| suffix.starts_with('/')))
            || hit.line == 0
            || hit.line > 8 * 1024 * 1024 + 1
            || hit.column_bytes == 0
            || hit.column_bytes > 8 * 1024 * 1024 + 1
            || hit.byte_offset >= 8 * 1024 * 1024
            || hit.text_start_column_bytes == 0
            || hit.text_start_column_bytes > hit.column_bytes
            || hit.text.len() > 4608
            || hit.text.contains(['\r', '\n'])
            || hit.sha256.len() != 64
            || !hit
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || (!hit.truncated && hit.text_start_column_bytes != 1)
        {
            return Err(invalid());
        }
        let at = (hit.column_bytes - hit.text_start_column_bytes) as usize;
        if hit.text.get(at..at.saturating_add(query.len())) != Some(query)
            || hit.byte_offset < hit.column_bytes - 1
        {
            return Err(invalid());
        }
        paths.insert(hit.path);
    }
    if paths.len() as u64 > response.files_scanned {
        return Err(invalid());
    }
    // Each distinct hit needs an owner-backed canonical instruction-scope
    // lookup before its snippet can enter model context. Bound that work
    // independently of the public search result limit: a broad valid search
    // must be withheld before it can fan out into hundreds of internal reads.
    if paths.len() > MAX_INSTRUCTION_DISCOVERY_HIT_PATHS {
        return Err(AgentEngineError::WorkspaceContext(
            "workspace.files/search instruction discovery hit budget exceeded".into(),
        ));
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::StrictJsonValue;
    use nomifun_chat_model_broker::ToolCallId;
    use serde_json::json;

    fn search_call() -> ChatToolCall {
        ChatToolCall {
            call_id: ToolCallId::from("search-many"),
            name: "search_files".into(),
            arguments: StrictJsonValue(json!({
                "path": "burst",
                "query": "needle",
                "limit": 200,
            })),
            provider_metadata: None,
        }
    }

    fn search_result(files: usize) -> AgentToolResult {
        let matches = (0..files)
            .map(|index| {
                json!({
                    "path": format!("burst/file-{index:03}.txt"),
                    "line": 1,
                    "column_bytes": 1,
                    "byte_offset": 0,
                    "sha256": "0".repeat(64),
                    "text": "needle",
                    "text_start_column_bytes": 1,
                    "truncated": false,
                })
            })
            .collect::<Vec<_>>();
        AgentToolResult::text(
            "search-many".into(),
            json!({
                "query": "needle",
                "matches": matches,
                "truncated": false,
                "incomplete_reasons": [],
                "files_scanned": files,
                "files_skipped": 0,
                "source_bytes_read": files * 6,
                "notice": "",
            })
            .to_string(),
            false,
        )
    }

    #[test]
    fn instruction_discovery_rejects_more_than_sixty_four_unique_search_hits() {
        let error = hit_paths(&search_call(), &search_result(65)).unwrap_err();
        assert!(matches!(
            error,
            AgentEngineError::WorkspaceContext(message)
                if message.contains("instruction discovery hit budget")
        ));
    }

    #[test]
    fn instruction_discovery_accepts_sixty_four_unique_search_hits() {
        assert_eq!(hit_paths(&search_call(), &search_result(64)).unwrap().len(), 64);
    }
}
