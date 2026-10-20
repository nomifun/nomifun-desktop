//! Bounded, content-free file presentation shared by live and historical reads.
use nomifun_agent_contracts::{ToolFileEntry, ToolFileOperation as Operation, ToolFileSummary, ToolFileSummaryKind};
use nomifun_chat_model_broker::ChatToolResultPart;
use serde::Deserialize;
use serde_json::Value;
use crate::EngineToolResult;

/// Only the original invocation's paths and patch line counts survive here.
/// It is a temporary presenter, not a content mirror or execution ledger.
#[derive(Clone, Debug)]
pub struct ToolFileProjection {
    summary: ToolFileSummary,
    patch_lines: Option<Vec<(u32, u32, usize)>>,
}

fn path(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    (!text.is_empty() && !text.contains('\0') && text.chars().take(4097).count() <= 4096)
        .then(|| text.to_owned())
}

fn entry(path: String) -> ToolFileEntry {
    ToolFileEntry { path, destination: None, added_lines: None, removed_lines: None }
}

fn patch_lines(file: &Value) -> Option<(u32, u32, usize)> {
    let hunks = file.get("hunks")?.as_array().filter(|hunks| !hunks.is_empty() && hunks.len() <= 256)?;
    let (mut added, mut removed) = (0u32, 0u32);
    for hunk in hunks {
        let lines = hunk.get("lines")?.as_array().filter(|lines| !lines.is_empty() && lines.len() <= 16384)?;
        for line in lines {
            match line.get("kind")?.as_str()? {
                "add" => added = added.checked_add(1)?,
                "remove" => removed = removed.checked_add(1)?,
                "context" => {},
                _ => return None,
            }
        }
    }
    Some((added, removed, hunks.len()))
}

#[derive(Deserialize)]
struct PatchReceipt { files: Vec<PatchFileReceipt>, file_count: usize }
#[derive(Deserialize)]
struct PatchFileReceipt {
    path: String,
    hunks_applied: usize,
    #[serde(rename = "bytes_before")]
    _bytes_before: u64,
    #[serde(rename = "bytes_after")]
    _bytes_after: u64,
    #[serde(rename = "created")]
    _created: bool,
}

impl ToolFileProjection {
    /// Exact admitted first-party identity is required. Model aliases and
    /// remote/MCP tools cannot manufacture local file targets or line counts.
    pub fn from_call(capability: Option<&str>, action: Option<&str>, arguments: &Value) -> Option<Self> {
        if capability != Some("workspace.files") { return None; }
        let operation = match action? {
            "workspace.files/read" => Operation::Read,
            "workspace.files/write" => Operation::Write,
            "workspace.files/patch" => Operation::Patch,
            "workspace.files/delete" => Operation::Delete,
            "workspace.files/rename" => Operation::Rename,
            _ => return None,
        };
        let (files, patch_lines) = if operation == Operation::Patch {
            let files = arguments.get("files")?.as_array().filter(|files| !files.is_empty() && files.len() <= 64)?;
            let entries = files.iter().map(|file| Some(entry(path(file.get("path")?)?))).collect::<Option<Vec<_>>>()?;
            (entries, files.iter().map(patch_lines).collect::<Option<Vec<_>>>())
        } else {
            let mut file = entry(path(arguments.get("path")?)?);
            if operation == Operation::Rename { file.destination = Some(path(arguments.get("destination")?)?); }
            (vec![file], None)
        };
        let kind = (operation == Operation::Read && arguments.get("format").and_then(Value::as_str) == Some("instruction_scope"))
            .then_some(ToolFileSummaryKind::InstructionScope);
        Some(Self { summary: ToolFileSummary { operation, kind, files }, patch_lines })
    }

    pub fn summary(&self) -> ToolFileSummary { self.summary.clone() }

    /// Success and matching native publication receipts prove only this
    /// historical patch. Held, failed, partial and unknown outcomes keep targets
    /// without implying that their requested changes were applied.
    pub fn settled(&self, result: &EngineToolResult, runtime_held: bool) -> ToolFileSummary {
        let mut summary = self.summary();
        if result.is_error || runtime_held { return summary; }
        let Some(lines) = &self.patch_lines else { return summary; };
        let [ChatToolResultPart::Text { text }] = result.output.as_slice() else { return summary; };
        let Ok(receipt) = serde_json::from_str::<PatchReceipt>(text) else { return summary; };
        // Native patch publication returns request order, including canonical
        // path spellings. Do not compare aliases by consulting today's disk.
        if receipt.file_count != lines.len() || receipt.files.len() != lines.len()
            || receipt.files.iter().zip(lines).any(|(file, (_, _, hunks))|
                file.hunks_applied != *hunks || path(&Value::String(file.path.clone())).is_none()) {
            return summary;
        }
        for (file, (added, removed, _)) in summary.files.iter_mut().zip(lines) {
            file.added_lines = Some(*added);
            file.removed_lines = Some(*removed);
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Value { json!({"files":[
        {"path":" dir  one/a.rs ","hunks":[{"lines":[{"kind":"context","text":"PRIVATE_CONTEXT"},{"kind":"add","text":"PRIVATE_ADD"},{"kind":"remove","text":"PRIVATE_REMOVE"}]}]},
        {"path":"b.rs","hunks":[{"lines":[{"kind":"add","text":"PRIVATE_NEW"}]}]}
    ]}) }
    fn receipt(error: bool) -> EngineToolResult { EngineToolResult::text("call".into(),
        json!({"file_count":2,"files":[
            {"path":"/canonical/a.rs","hunks_applied":1,"bytes_before":10,"bytes_after":12,"created":false},
            {"path":"b.rs","hunks_applied":1,"bytes_before":0,"bytes_after":12,"created":true}
        ]}).to_string(),error) }

    #[test]
    fn file_projection_keeps_paths_and_batch_order_without_source_text() {
        let projection = ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/patch"),&request()).unwrap();
        let summary = projection.settled(&receipt(false),false);
        assert_eq!(summary.files.iter().map(|file|file.path.as_str()).collect::<Vec<_>>(),[" dir  one/a.rs ","b.rs"]);
        assert_eq!((summary.files[0].added_lines,summary.files[0].removed_lines),(Some(1),Some(1)));
        assert_eq!((summary.files[1].added_lines,summary.files[1].removed_lines),(Some(1),Some(0)));
        assert!(!format!("{projection:?}").contains("PRIVATE"));
        assert!(!serde_json::to_string(&summary).unwrap().contains("PRIVATE"));
    }

    #[test]
    fn file_projection_never_promotes_failed_partial_or_held_patches_to_applied_changes() {
        let projection = ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/patch"),&request()).unwrap();
        let partial = EngineToolResult::text("call".into(),json!({"file_count":1,"files":[{"path":"a.rs","hunks_applied":1}],"published":[0]}).to_string(),false);
        for summary in [projection.summary(),projection.settled(&receipt(true),false),projection.settled(&receipt(false),true),projection.settled(&partial,false)] {
            assert_eq!(summary.files.len(),2);
            assert!(summary.files.iter().all(|file|file.added_lines.is_none() && file.removed_lines.is_none()));
        }
    }

    #[test]
    fn file_projection_requires_native_identity_and_preserves_rename_and_instruction_targets() {
        for capability in [None,Some("ssh"),Some("nomi.mcp.v1.fake")] {
            assert!(ToolFileProjection::from_call(capability,Some("workspace.files/read"),&json!({"path":"local"})).is_none());
        }
        let rename = ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/rename"),&json!({"path":" a  b ","destination":" c  d "})).unwrap().summary();
        assert_eq!(rename.files[0].path," a  b ");assert_eq!(rename.files[0].destination.as_deref(),Some(" c  d "));
        let read = ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/read"),&json!({"path":".","format":"instruction_scope"})).unwrap().summary();
        assert_eq!(read.kind,Some(ToolFileSummaryKind::InstructionScope));
        let write = ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/write"),&json!({"path":"same.rs","content":"PRIVATE"})).unwrap();
        let result = EngineToolResult::text("call".into(),json!({"written":true,"bytes":100,"line_count":90}).to_string(),false);
        assert!(write.settled(&result,false).files[0].added_lines.is_none());
        let file = request()["files"][0].clone();
        assert_eq!(ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/patch"),&json!({"files":vec![file.clone();64]})).unwrap().summary().files.len(),64);
        assert!(ToolFileProjection::from_call(Some("workspace.files"),Some("workspace.files/patch"),&json!({"files":vec![file;65]})).is_none());
    }
}
