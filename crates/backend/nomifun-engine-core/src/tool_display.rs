//! Bounded product summaries of tool facts. Complete invocation content remains
//! canonical and is read separately, only when a user opens its detail.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::EngineToolResult;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDisplayFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_not_started: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_timed_out: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_reference_invalid: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounded_result: Option<ToolBoundedResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_fatal_failure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_summary: Option<nomifun_agent_contracts::ToolFileSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_knowledge_file: Option<WrittenKnowledgeFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied_stop_conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_asset_ids: Vec<String>,
    /// Used to select the existing product status, never persisted separately.
    #[serde(skip)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBoundedResult { SearchContextWithheld }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrittenKnowledgeFile { pub kb_id: String, pub rel_path: String }

pub fn tool_display_operation(capability: Option<&str>, arguments: &Value) -> Option<String> {
    if capability != Some("computer") { return None; }
    let operation = arguments.get("action")?.as_str()?;
    matches!(operation, "screenshot" | "observe" | "click_element" | "set_element_value" | "right_click_element"
        | "double_click_element" | "left_click" | "right_click" | "middle_click" | "double_click" | "triple_click"
        | "mouse_move" | "left_click_drag" | "type" | "key" | "scroll" | "focus_window")
        .then(|| operation.to_owned())
}

/// The compatibility Gateway returns an explicit structured denial. English
/// diagnostics and unrelated tool output cannot create a stop confirmation.
pub fn tool_denied_stop_target(name: &str, arguments: &Value, result: &EngineToolResult) -> Option<String> {
    let alias = name.split("__").collect::<Vec<_>>();
    let gateway_tool = name == "nomi_stop_conversation"
        || (alias.first() == Some(&"mcp") && alias.get(2) == Some(&"nomi_stop_conversation")
            && (alias.len() == 3 || (alias.len() == 4 && alias[3].len() == 20 && alias[3].bytes().all(|byte| byte.is_ascii_hexdigit()))));
    if !gateway_tool { return None; }
    let mut value: Value = serde_json::from_str(&result.output_text()).ok()?;
    if value.get("error").is_none() {
        let content = value.get("content")?.as_array()?;
        if content.len() != 1 || content[0].get("type")?.as_str()? != "text" { return None; }
        value = serde_json::from_str(content[0].get("text")?.as_str()?).ok()?;
    }
    if value.get("error").and_then(Value::as_str) != Some("session_capability_denied")
        || value.get("tool").and_then(Value::as_str) != Some("nomi_stop_conversation") { return None; }
    let target = arguments.get("conversation_id")?.as_str()?;
    uuid::Uuid::parse_str(target).ok().filter(|id| id.get_version_num() == 7).map(|id| id.to_string())
}

fn display_path(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)).then(|| value.to_owned())
}

fn compact(value: &str) -> Option<String> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() { return None; }
    let mut chars = value.chars();
    let mut text: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() { text.push('…'); }
    Some(text)
}

/// Display the bounded first source line without interpreting shell effects.
/// Complete scripts and literal argv remain available through canonical detail.
fn shell_display_target(source: &str) -> Option<String> {
    let mut lines = source.lines().filter(|line| !line.trim().is_empty());
    let mut label = compact(lines.next()?)?;
    if lines.next().is_some() && !label.ends_with('…') { label.push('…'); }
    Some(label)
}

fn process_display_target(arguments: &Value, field: &str, value: &str) -> Option<String> {
    if field == "cmd" { return shell_display_target(value); }
    let mut label = display_path(value)?;
    if let Some(args) = arguments.get("args").and_then(Value::as_array) {
        for argument in args.iter().filter_map(Value::as_str) {
            label.push(' ');
            if argument.chars().any(char::is_whitespace) || argument.contains(['\'', '"']) {
                label.push_str(&serde_json::to_string(argument).ok()?);
            } else { label.push_str(argument); }
            if label.chars().count() > 120 { break; }
        }
    }
    compact(&label)
}

/// Summaries never copy complete source objects or determine execution effects.
pub fn tool_display_target(capability: Option<&str>, action: Option<&str>, name: &str, arguments: &Value) -> Option<String> {
    let catalogue = nomifun_agent_contracts::tool_presentation::tool_presentations();
    let entry = match (capability, action) {
        (Some(capability), Some(action)) => nomifun_agent_contracts::tool_presentation::tool_presentation(capability, action),
        (None, None) => catalogue.iter().find(|entry| entry.name == name),
        _ => None,
    }?;
    for field in &entry.target_fields {
        let value = field.split('.').try_fold(arguments, |value, field| match value {
            Value::Array(items) => field.parse::<usize>().ok().and_then(|index| items.get(index)),
            Value::Object(fields) => fields.get(field),
            _ => None,
        }).and_then(Value::as_str);
        let Some(value) = value else { continue; };
        if value.trim().is_empty() { continue; }
        if entry.capability_id.as_deref() == Some("workspace.process")
            || (entry.capability_id.as_deref() == Some("ssh") && field == "command") {
            return if entry.capability_id.as_deref() == Some("workspace.process") {
                process_display_target(arguments, field, value)
            } else { shell_display_target(value) };
        }
        if matches!(entry.capability_id.as_deref(), Some("workspace.files" | "knowledge" | "ssh"))
            && field != "query" {
            return display_path(value);
        }
        if entry.target_kind.as_deref() == Some("url") {
            if let Ok(url) = url::Url::parse(value) {
                if let Some(host) = url.host_str() { return Some(host.to_owned()); }
            }
        }
        return compact(value);
    }
    None
}

/// Knowledge handles are opaque. The actual owner result provides the existing
/// file label; document text and snippets are never used as a fallback.
pub fn tool_result_target(capability: Option<&str>, action: Option<&str>, result: &EngineToolResult) -> Option<String> {
    if result.is_error || capability != Some("knowledge")
        || !matches!(action, Some("knowledge/read" | "knowledge/write")) { return None; }
    let value: Value = serde_json::from_str(&result.output_text()).ok()?;
    display_path(value.get("relative_path")?.as_str()?)
}

fn clean_cleanup(value: &Value) -> bool {
    value.pointer("/cleanup/reaped").and_then(Value::as_bool) == Some(true)
        && value.pointer("/cleanup/errors").and_then(Value::as_array).is_some_and(Vec::is_empty)
}

fn reference_ids(payload: &Value) -> Vec<String> {
    if payload.get("error").is_some_and(|error| !error.is_null()) { return Vec::new(); }
    let payload = payload.get("result").filter(|result| result.is_object()).unwrap_or(payload);
    if payload.get("error").is_some_and(|error| !error.is_null()) { return Vec::new(); }
    payload.get("result_asset_ids").and_then(Value::as_array).filter(|ids| ids.len() <= 100)
        .map(|ids| ids.iter().filter_map(Value::as_str)
            .filter(|id| uuid::Uuid::parse_str(id).is_ok_and(|id| id.get_version_num() == 7))
            .map(str::to_owned).collect()).unwrap_or_default()
}

/// MCP observations may publish explicit local asset references, not execution
/// status or authority. Only documented structured result slots are inspected;
/// arbitrary nested JSON and free-text snippets are never searched for IDs.
fn mcp_reference_ids(capability: Option<&str>, action: Option<&str>, payload: &Value) -> Vec<String> {
    let Some(capability) = capability else { return Vec::new(); };
    let Some(digest) = capability.strip_prefix("nomi.mcp.v1.") else { return Vec::new(); };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || action != Some(format!("{capability}.invoke").as_str())
        || payload.get("isError").and_then(Value::as_bool) == Some(true)
        || payload.get("error").is_some_and(|error| !error.is_null()) { return Vec::new(); }
    if let Some(structured) = payload.get("structuredContent") { return reference_ids(structured); }
    if payload.get("result_asset_ids").is_some() || payload.get("result").is_some() { return reference_ids(payload); }
    let Some(content) = payload.get("content").and_then(Value::as_array).filter(|items| items.len() == 1) else { return Vec::new(); };
    if content[0].get("type").and_then(Value::as_str) != Some("text") { return Vec::new(); }
    let Some(text) = content[0].get("text").and_then(Value::as_str).filter(|text| text.len() <= 64 * 1024) else { return Vec::new(); };
    serde_json::from_str::<Value>(text).ok().map(|payload| reference_ids(&payload)).unwrap_or_default()
}

/// Status details are interpreted only for the exact admitted first-party
/// action. JSON-looking MCP output cannot impersonate a process or search fact.
pub fn tool_result_display(capability: Option<&str>, action: Option<&str>, result: &EngineToolResult) -> ToolDisplayFacts {
    let mut facts = ToolDisplayFacts::default();
    if result.is_error {
        facts.error = Some("The tool did not complete successfully. Do not retry automatically; review the task result first.".into());
    }
    let Ok(value) = serde_json::from_str::<Value>(&result.output_text()) else { return facts; };
    if capability == Some("workspace.process") && action.is_some_and(|action| action.starts_with("workspace.process/")) {
        facts.cancelled = value.get("state").and_then(Value::as_str) == Some("cancelled")
            && value.pointer("/cleanup/reaped").and_then(Value::as_bool) == Some(true);
        let process_id = value.get("process_id").and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && id.len() <= 128);
        let output = value.pointer("/output/text").is_some_and(Value::is_string);
        if matches!(action, Some("workspace.process/start" | "workspace.process/poll")) {
            let exited = value.get("state").and_then(Value::as_str) == Some("exited");
            let exit = value.get("exit_code").and_then(Value::as_i64).filter(|exit| *exit >= 0);
            if exited && process_id && output && value.get("signal") == Some(&Value::Null)
                && clean_cleanup(&value) && exit.is_some_and(|exit| value.get("success").and_then(Value::as_bool) == Some(exit == 0))
                && ["interrupt_attempted", "terminate_attempted", "force_kill_attempted"].iter()
                    .all(|field| value.get("cleanup").and_then(|cleanup| cleanup.get(field)).and_then(Value::as_bool) == Some(false)) {
                facts.command_exit_code = exit;
                facts.non_fatal_failure = exit.filter(|exit| *exit != 0).filter(|_| result.is_error).map(|_| true);
            }
            if result.is_error && value.get("state").and_then(Value::as_str) == Some("timed_out")
                && value.get("success").and_then(Value::as_bool) == Some(false)
                && value.get("exit_code").is_none_or(Value::is_null) && value.get("signal").is_none_or(Value::is_null)
                && process_id && output && clean_cleanup(&value)
                && ["interrupt_attempted", "terminate_attempted", "force_kill_attempted"].iter()
                    .all(|field| value.get("cleanup").and_then(|cleanup| cleanup.get(field)).is_some_and(Value::is_boolean)) {
                facts.command_timed_out = Some(true);
            }
        }
        if result.is_error && action == Some("workspace.process/start")
            && value.get("schema").and_then(Value::as_str) == Some("nomifun.process-start-observation.v1")
            && value.get("state").and_then(Value::as_str) == Some("not_started")
            && value.get("code").and_then(Value::as_str) == Some("PROCESS_NOT_STARTED")
            && value.get("user_code_started").and_then(Value::as_bool) == Some(false)
            && value.get("success").and_then(Value::as_bool) == Some(false)
            && ["process_id", "exit_code", "signal"].iter().all(|field| value.get(field).is_none()) {
            facts.command_not_started = Some(true);
        }
        let operation = match action {
            Some("workspace.process/poll") => Some("poll"), Some("workspace.process/input") => Some("stdin"),
            Some("workspace.process/close_stdin") => Some("close_stdin"), Some("workspace.process/resize") => Some("resize"),
            Some("workspace.process/cancel") => Some("cancel"), _ => None,
        };
        if result.is_error && operation.is_some()
            && value.get("schema").and_then(Value::as_str) == Some("nomifun.process-control-observation.v1")
            && value.get("state").and_then(Value::as_str) == Some("not_executed")
            && value.get("code").and_then(Value::as_str) == Some("PROCESS_REFERENCE_INVALID")
            && value.get("operation").and_then(Value::as_str) == operation
            && value.get("control_applied").and_then(Value::as_bool) == Some(false)
            && value.get("success").and_then(Value::as_bool) == Some(false) && value.get("process_id").is_none() {
            facts.process_reference_invalid = Some(true);
        }
    }
    if result.is_error && capability == Some("workspace.files") && action == Some("workspace.files/search")
        && value.as_object().is_some_and(|fields| fields.len() == 4)
        && value.get("kind").and_then(Value::as_str) == Some("search_context_withheld")
        && value.get("search_executed").and_then(Value::as_bool) == Some(true)
        && value.get("snippets_withheld").and_then(Value::as_bool) == Some(true)
        && value.get("notice").and_then(Value::as_str).is_some_and(|notice| !notice.is_empty() && notice.len() <= 2048) {
        facts.bounded_result = Some(ToolBoundedResult::SearchContextWithheld);
        facts.non_fatal_failure = Some(true);
    }
    let media_ids = if !result.is_error && capability == Some("creation.media")
        && action.is_some_and(|action| action.starts_with("creation.media/")) {
        value.get("result_asset_ids")
    } else if !result.is_error && capability == Some("creative.workshop") && action == Some("creative.workshop/template.run")
        && value.get("kind").and_then(Value::as_str) == Some("nomifun.creative-studio.template-run")
        && value.get("version").and_then(Value::as_u64) == Some(1)
        && value.pointer("/record/status").and_then(Value::as_str) == Some("succeeded") {
        value.pointer("/record/resultAssetIds")
    } else { None };
    if let Some(ids) = media_ids.and_then(Value::as_array).filter(|ids| ids.len() <= 100) {
        facts.result_asset_ids = ids.iter().filter_map(Value::as_str)
            .filter(|id| uuid::Uuid::parse_str(id).is_ok_and(|id| id.get_version_num() == 7))
            .map(str::to_owned).collect();
    }
    if !result.is_error && facts.result_asset_ids.is_empty() {
        facts.result_asset_ids = mcp_reference_ids(capability, action, &value);
    }
    if !result.is_error && capability == Some("knowledge") && action == Some("knowledge/write")
        && value.get("created").is_some_and(Value::is_boolean) {
        if let (Some(kb_id), Some(path)) = (value.get("resource_id").and_then(Value::as_str), value.get("relative_path").and_then(Value::as_str)) {
            if uuid::Uuid::parse_str(kb_id).is_ok_and(|id| id.get_version_num() == 7)
                && !path.is_empty() && path.len() <= 4096 && !path.chars().any(char::is_control) {
                facts.written_knowledge_file = Some(WrittenKnowledgeFile { kb_id: kb_id.to_owned(), rel_path: path.to_owned() });
            }
        }
    }
    if facts.non_fatal_failure == Some(true) || facts.process_reference_invalid == Some(true) || facts.cancelled { facts.error = None; }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn targets_keep_file_paths_and_bounded_command_summaries() {
        let args = json!({"path":"src/app.rs", "content":"PRIVATE_FILE_BODY"});
        assert_eq!(tool_display_target(Some("workspace.files"), Some("workspace.files/write"), "write_file", &args).as_deref(), Some("src/app.rs"));
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process", &json!({"cmd":"head -n 5 file.txt"})).as_deref(), Some("head -n 5 file.txt"));
        let quoted = "head -n 300 \"/Users/muri/Desktop/Nomifun 上架指南.html\"";
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process", &json!({"cmd":quoted})).as_deref(), Some(quoted));
        let windows = r#"head -n 2 "C:\Users\Muri\文件 a.md""#;
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process", &json!({"cmd":windows})).as_deref(), Some(windows));
        let path = format!("docs/{}  multiple spaces/文档.md", "long".repeat(50));
        assert_eq!(tool_display_target(Some("workspace.files"), Some("workspace.files/read"), "read_file", &json!({"path":path})), Some(path));
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process",
            &json!({"cmd":"git status --short && rg -n 'requestAnimationFrame' src/game.ts"})).as_deref(),
            Some("git status --short && rg -n 'requestAnimationFrame' src/game.ts"));
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process",
            &json!({"cmd":"cat <<EOF\nPRIVATE_FILE_BODY\nEOF"})).as_deref(),Some("cat <<EOF…"));
        let long = format!("python3 -c '{}'", "source".repeat(100));
        let label = tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process", &json!({"cmd":long})).unwrap();
        assert!(label.starts_with("python3 -c "));assert!(label.ends_with('…'));assert_eq!(label.chars().count(),121);
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process",
            &json!({"command":"git","args":["status","--short"]})).as_deref(),Some("git status --short"));
        assert_eq!(tool_display_target(Some("workspace.process"), Some("workspace.process/start"), "start_process",
            &json!({"component":"bun","args":["run","check"]})).as_deref(),Some("bun run check"));
        for action in ["ssh/exec", "ssh/sudo"] {
            assert_eq!(tool_display_target(Some("ssh"), Some(action), "ssh_exec", &json!({"command":quoted})).as_deref(), Some(quoted));
            assert_eq!(tool_display_target(Some("ssh"), Some(action), "ssh_exec",
                &json!({"command":"git diff --stat"})).as_deref(),Some("git diff --stat"));
        }
        assert!(tool_display_target(Some("mcp.fake"), Some("workspace.files/write"), "write_file", &args).is_none());
    }

    #[test]
    fn process_status_is_typed_and_foreign_output_cannot_impersonate_it() {
        let output = json!({"state":"exited", "exit_code":2, "signal":null, "success":false,
            "process_id":"owned", "output":{"text":"PRIVATE_FILE_BODY"},
            "cleanup":{"reaped":true,"errors":[],"interrupt_attempted":false,"terminate_attempted":false,"force_kill_attempted":false}});
        let result = EngineToolResult::text("call".into(), output.to_string(), true);
        let facts = tool_result_display(Some("workspace.process"), Some("workspace.process/start"), &result);
        assert_eq!(facts.command_exit_code, Some(2));
        assert_eq!(facts.non_fatal_failure, Some(true));
        assert!(!serde_json::to_string(&facts).unwrap().contains("PRIVATE_FILE_BODY"));
        assert!(tool_result_display(Some("workspace.process"), Some("workspace.process/exec"), &result).command_exit_code.is_none());
        assert!(tool_result_display(Some("mcp.remote"), Some("workspace.process/start"), &result).command_exit_code.is_none());
    }

    #[test]
    fn media_references_and_knowledge_paths_survive_without_their_body() {
        let id = "0190f5fe-7c00-7a00-8000-000000000081";
        let result = EngineToolResult::text("call".into(), json!({"result_asset_ids":[id,"invalid"],"content":"PRIVATE"}).to_string(), false);
        assert_eq!(tool_result_display(Some("creation.media"), Some("creation.media/image"), &result).result_asset_ids, vec![id]);
        let result = EngineToolResult::text("call".into(), json!({"relative_path":"notes/fact.md","content":"PRIVATE"}).to_string(), false);
        assert_eq!(tool_result_target(Some("knowledge"), Some("knowledge/read"), &result).as_deref(), Some("notes/fact.md"));
    }
    #[test]
    fn existing_product_notices_keep_only_their_typed_fields() {
        let kb = "0190f5fe-7c00-7a00-8000-000000000081";
        let result = EngineToolResult::text("write".into(), json!({"resource_id":kb,"relative_path":"notes/fact.md","created":true,"content":"PRIVATE"}).to_string(), false);
        let facts = tool_result_display(Some("knowledge"), Some("knowledge/write"), &result);
        assert_eq!(facts.written_knowledge_file, Some(WrittenKnowledgeFile { kb_id:kb.into(),rel_path:"notes/fact.md".into() }));
        assert_eq!(tool_display_operation(Some("computer"), &json!({"action":"screenshot","text":"PRIVATE"})).as_deref(), Some("screenshot"));
        assert!(tool_display_operation(Some("mcp.fake"), &json!({"action":"screenshot"})).is_none());
        assert_eq!(tool_display_target(Some("web.research"), Some("web.research/fetch"), "web_fetch", &json!({"url":"https://pvp.qq.com/path?q=x"})).as_deref(), Some("pvp.qq.com"));
        let denial = json!({"error":"session_capability_denied","tool":"nomi_stop_conversation"});
        let result = EngineToolResult::text("stop".into(), json!({"content":[{"type":"text","text":denial.to_string()}]}).to_string(), false);
        assert_eq!(tool_denied_stop_target("mcp__nomi__nomi_stop_conversation__0123456789abcdef0123", &json!({"conversation_id":kb}), &result).as_deref(), Some(kb));
        assert!(tool_denied_stop_target("read_file", &json!({"conversation_id":kb}), &result).is_none());
        let prose = EngineToolResult::text("stop".into(), "nomi_stop_conversation is not permitted on the Channel surface", true);
        assert!(tool_denied_stop_target("nomi_stop_conversation", &json!({"conversation_id":kb}), &prose).is_none());
    }

    #[test]
    fn gateway_and_mcp_asset_references_keep_the_actual_product_results_without_status_inference() {
        let capability = format!("nomi.mcp.v1.{}", "a".repeat(64));
        let action = format!("{capability}.invoke");
        let id = "0190f5fe-7c00-7a00-8000-000000000081";
        // Exact Gateway send_file_success and get_task result slots.
        let send_file = json!({"result":{"result_asset_ids":[id],"file_name":"file.png","delivered":true,"note":"PRIVATE_TOOL_BODY"}});
        let get_task = json!({"result":{"creation_task_id":id,"status":"succeeded","result_asset_ids":[id],"prompt":"PRIVATE_TOOL_BODY"}});
        for payload in [send_file.clone(), get_task.clone(),
            json!({"isError":false,"structuredContent":get_task}),
            json!({"isError":false,"content":[{"type":"text","text":send_file.to_string()}]})] {
            let result = EngineToolResult::text("media".into(), payload.to_string(), false);
            let facts = tool_result_display(Some(&capability), Some(&action), &result);
            assert_eq!(facts.result_asset_ids, vec![id]);
            assert!(facts.command_exit_code.is_none() && facts.command_not_started.is_none() && facts.non_fatal_failure.is_none());
            assert!(!serde_json::to_string(&facts).unwrap().contains("PRIVATE_TOOL_BODY"));
        }
        for payload in [json!({"isError":true,"structuredContent":send_file}),
            json!({"error":"failed","result_asset_ids":[id]}),
            json!({"details":{"result_asset_ids":[id]},"state":"exited","exit_code":0}),
            json!({"content":[{"type":"text","text":"arbitrary text"},{"type":"text","text":get_task.to_string()}]})] {
            let result = EngineToolResult::text("media".into(), payload.to_string(), false);
            let facts = tool_result_display(Some(&capability), Some(&action), &result);
            assert!(facts.result_asset_ids.is_empty());
            assert!(facts.command_exit_code.is_none());
        }
        let failed = EngineToolResult::text("media".into(), send_file.to_string(), true);
        assert!(tool_result_display(Some(&capability), Some(&action), &failed).result_asset_ids.is_empty());
        let result = EngineToolResult::text("media".into(), send_file.to_string(), false);
        assert!(tool_result_display(Some("unrelated"), Some(&action), &result).result_asset_ids.is_empty());
    }

}
