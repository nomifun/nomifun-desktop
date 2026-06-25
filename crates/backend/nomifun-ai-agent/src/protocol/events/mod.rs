pub mod permission;
pub mod session_updates;
pub mod tool_call;
pub mod translate;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use nomifun_api_types::AgentStreamErrorData as ErrorEventData;

pub use permission::{
    AcpPermissionEventData, AcpPermissionOptionData, AcpPermissionOptionKind, AcpPermissionRequestData,
    AcpPermissionToolCall,
};
pub use session_updates::{
    AgentStatusEventData, AvailableCommandsEventData, CronTriggerEventData, PlanEventData, SkillSuggestEventData,
    ThinkingEventData,
};
pub use tool_call::{
    AcpToolCallContentItem, AcpToolCallEventData, AcpToolCallKind, AcpToolCallLocationItem,
    AcpToolCallSessionUpdateKind, AcpToolCallStatus, AcpToolCallTextBlock, AcpToolCallTextBlockType,
    AcpToolCallUpdateData, ToolCallEventData, ToolCallStatus, ToolGroupEntry,
};
pub(crate) use translate::{permission_request_to_event_data, session_notification_to_events};

/// Events emitted by an Agent during a message processing turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentStreamEvent {
    Start(StartEventData),
    #[serde(rename = "content")]
    Text(TextEventData),
    Tips(TipsEventData),
    ToolCall(ToolCallEventData),
    AcpToolCall(AcpToolCallEventData),
    ToolGroup(Vec<ToolGroupEntry>),
    AgentStatus(AgentStatusEventData),
    Thinking(ThinkingEventData),
    Plan(PlanEventData),
    Permission(serde_json::Value),
    AcpPermission(AcpPermissionEventData),
    SkillSuggest(SkillSuggestEventData),
    CronTrigger(CronTriggerEventData),
    AcpModelInfo(serde_json::Value),
    AcpModeInfo(serde_json::Value),
    AcpConfigOption(serde_json::Value),
    AcpSessionInfo(serde_json::Value),
    AcpContextUsage(serde_json::Value),
    AcpPromptHookWarning(serde_json::Value),
    SlashCommandsUpdated(serde_json::Value),
    AvailableCommands(AvailableCommandsEventData),
    /// Emitted once at the end of a turn with aggregate metrics so the UI can
    /// show duration / token cost and telemetry can record per-turn stats.
    /// Purely additive: consumers that don't recognise it ignore it.
    TurnCompleted(TurnCompletedEventData),
    Finish(FinishEventData),
    Error(ErrorEventData),
    System(serde_json::Value),
    RequestTrace(serde_json::Value),
    SessionAssigned(SessionAssignedEventData),
}

/// Data for the `Start` event.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct StartEventData {
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Data for the `SessionAssigned` event.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct SessionAssignedEventData {
    pub session_id: String,
}

/// Data for the `Text` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEventData {
    pub content: String,
}

/// Data for the `Tips` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipsEventData {
    pub content: String,
    #[serde(rename = "type")]
    pub tip_type: TipType,
}

/// Severity level for a tip event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipType {
    Error,
    Success,
    Warning,
}

/// Data for the `Finish` event.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct FinishEventData {
    #[serde(default)]
    pub session_id: Option<String>,
    /// Why the turn ended. `None` = the backend did not report (treated as
    /// success for back-compat). `EndTurn` = normal completion; `MaxTokens` /
    /// `MaxTurnRequests` / `Refusal` / `Cancelled` = the turn did NOT accomplish
    /// its goal. AutoWork consults this instead of treating any Finish as done.
    #[serde(default)]
    pub stop_reason: Option<TurnStopReason>,
}

/// Data for the `TurnCompleted` event — aggregate metrics for one turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct TurnCompletedEventData {
    /// Wall-clock duration of the turn in milliseconds.
    #[ts(type = "number")]
    pub elapsed_ms: i64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Tokens written into the provider prompt cache.
    #[serde(default)]
    #[ts(type = "number")]
    pub cache_creation_tokens: u64,
    /// Tokens read back from the provider prompt cache.
    #[serde(default)]
    #[ts(type = "number")]
    pub cache_read_tokens: u64,
    /// Current context occupancy (last request's prompt tokens). Gauge numerator.
    #[serde(default)]
    #[ts(type = "number")]
    pub context_tokens: u64,
    /// Effective context budget (engine compaction window). Gauge denominator.
    #[serde(default)]
    #[ts(type = "number")]
    pub context_window: u64,
    /// Why the turn ended (mirrors Finish), for a single self-contained record.
    #[serde(default)]
    pub stop_reason: Option<TurnStopReason>,
}

/// Cross-backend normalized "why did the turn end" reason. Deliberately NOT the
/// ACP SDK's `StopReason` so the shared event type does not couple to ACP
/// (nomi / openclaw / remote are not ACP); each backend maps its own outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum TurnStopReason {
    /// Turn completed normally.
    EndTurn,
    /// Output token limit reached (turn truncated).
    MaxTokens,
    /// Per-turn request cap reached (turn truncated).
    MaxTurnRequests,
    /// Model refused to continue.
    Refusal,
    /// Turn was cancelled / aborted (server or transport, not a clean finish).
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionKind as SdkPermissionOptionKind, RequestPermissionRequest,
        SessionNotification, SessionUpdate, ToolCall as SdkToolCall, ToolCallStatus as SdkToolCallStatus,
        ToolCallUpdate as SdkToolCallUpdate, ToolCallUpdateFields, ToolKind as SdkToolKind,
    };
    use serde_json::json;

    #[test]
    fn text_event_roundtrip() {
        let event = AgentStreamEvent::Text(TextEventData {
            content: "Hello world".into(),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "content");
        assert_eq!(json["data"]["content"], "Hello world");

        let parsed: AgentStreamEvent = serde_json::from_value(json).unwrap();
        if let AgentStreamEvent::Text(data) = parsed {
            assert_eq!(data.content, "Hello world");
        } else {
            panic!("Expected Text event");
        }
    }

    #[test]
    fn tips_event_roundtrip() {
        let event = AgentStreamEvent::Tips(TipsEventData {
            content: "Something went wrong".into(),
            tip_type: TipType::Error,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tips");
        assert_eq!(json["data"]["type"], "error");
    }

    #[test]
    fn tool_call_event_roundtrip() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "read_file".into(),
            args: json!({ "path": "/tmp/a.txt" }),
            status: ToolCallStatus::Running,
            input: None,
            output: None,
            description: None,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tool_call");
        assert_eq!(json["data"]["call_id"], "call-1");
        assert_eq!(json["data"]["status"], "running");
    }

    #[test]
    fn tool_call_event_includes_enriched_fields() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "Glob".into(),
            args: json!({}),
            status: ToolCallStatus::Completed,
            input: Some(json!({ "pattern": "**/*.rs" })),
            output: Some("src/main.rs\nsrc/lib.rs".into()),
            description: Some("Search for Rust files".into()),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tool_call");
        assert_eq!(json["data"]["input"]["pattern"], "**/*.rs");
        assert_eq!(json["data"]["output"], "src/main.rs\nsrc/lib.rs");
        assert_eq!(json["data"]["description"], "Search for Rust files");
    }

    #[test]
    fn tool_call_event_omits_none_fields() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "Glob".into(),
            args: json!({}),
            status: ToolCallStatus::Running,
            input: None,
            output: None,
            description: None,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert!(json["data"].get("input").is_none());
        assert!(json["data"].get("output").is_none());
        assert!(json["data"].get("description").is_none());
    }

    #[test]
    fn finish_event_roundtrip() {
        let event = AgentStreamEvent::Finish(FinishEventData {
            session_id: Some("sess-abc".into()),
            stop_reason: None,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "finish");
        assert_eq!(json["data"]["session_id"], "sess-abc");
    }

    #[test]
    fn finish_event_stop_reason_serde_and_backcompat() {
        // stop_reason serializes snake_case for the WS wire.
        let event = AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: Some(TurnStopReason::MaxTurnRequests),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["data"]["stop_reason"], "max_turn_requests");

        // Back-compat: an old Finish payload with no stop_reason deserializes to
        // None (so older producers / persisted events keep parsing).
        let old = serde_json::json!({ "type": "finish", "data": { "session_id": "s" } });
        let back: AgentStreamEvent = serde_json::from_value(old).unwrap();
        assert!(matches!(back, AgentStreamEvent::Finish(d) if d.stop_reason.is_none()));
    }

    #[test]
    fn error_event_roundtrip() {
        let event = AgentStreamEvent::Error(ErrorEventData::legacy("timeout", None));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["data"]["message"], "timeout");
    }

    #[test]
    fn start_event_default_session_id() {
        let event = AgentStreamEvent::Start(StartEventData::default());
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "start");
        assert_eq!(json["data"]["session_id"], serde_json::Value::Null);
    }

    #[test]
    fn tool_group_event_roundtrip() {
        let entries = vec![
            ToolGroupEntry {
                call_id: "c1".into(),
                name: "read".into(),
                status: ToolCallStatus::Completed,
                description: Some("Read file".into()),
            },
            ToolGroupEntry {
                call_id: "c2".into(),
                name: "write".into(),
                status: ToolCallStatus::Running,
                description: None,
            },
        ];
        let event = AgentStreamEvent::ToolGroup(entries);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tool_group");
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0]["call_id"], "c1");
    }

    #[test]
    fn agent_status_event_roundtrip() {
        let event = AgentStreamEvent::AgentStatus(AgentStatusEventData {
            backend: "claude".into(),
            status: "running".into(),
            agent_name: Some("default".into()),
            session_id: None,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "agent_status");
        assert_eq!(json["data"]["backend"], "claude");
    }

    #[test]
    fn session_tool_call_maps_to_acp_tool_call_event() {
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-1", "Terminal")
                    .kind(SdkToolKind::Execute)
                    .status(SdkToolCallStatus::Pending)
                    .raw_input(json!({ "command": "echo hi" })),
            ),
        );

        let events = session_notification_to_events(&notif);
        assert_eq!(events.len(), 1);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["type"], "acp_tool_call");
        assert_eq!(json["data"]["session_id"], "sess-1");
        assert_eq!(json["data"]["update"]["sessionUpdate"], "tool_call");
        assert_eq!(json["data"]["update"]["tool_call_id"], "tool-1");
        assert_eq!(json["data"]["update"]["title"], "Terminal");
        assert_eq!(json["data"]["update"]["kind"], "execute");
        assert_eq!(json["data"]["update"]["rawInput"]["command"], "echo hi");
    }

    #[test]
    fn session_tool_call_update_omits_missing_fields_for_frontend_merge() {
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-1",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Completed),
            )),
        );

        let events = session_notification_to_events(&notif);
        assert_eq!(events.len(), 1);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["type"], "acp_tool_call");
        assert_eq!(json["data"]["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(json["data"]["update"]["tool_call_id"], "tool-1");
        assert_eq!(json["data"]["update"]["status"], "completed");
        assert!(json["data"]["update"].get("title").is_none());
        assert!(json["data"]["update"].get("rawInput").is_none());
    }

    #[test]
    fn permission_request_maps_to_snake_case_event_data() {
        let request = RequestPermissionRequest::new(
            "sess-1",
            SdkToolCallUpdate::new(
                "tool-1",
                ToolCallUpdateFields::new()
                    .title("Write file")
                    .kind(SdkToolKind::Edit)
                    .raw_input(json!({ "file_path": "/tmp/a.txt" })),
            ),
            vec![
                PermissionOption::new("allow", "Allow", SdkPermissionOptionKind::AllowOnce),
                PermissionOption::new("reject", "Reject", SdkPermissionOptionKind::RejectOnce),
            ],
        );

        let event = AgentStreamEvent::AcpPermission(permission_request_to_event_data(&request));
        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["type"], "acp_permission");
        assert_eq!(json["data"]["session_id"], "sess-1");
        assert_eq!(json["data"]["tool_call"]["tool_call_id"], "tool-1");
        assert_eq!(json["data"]["tool_call"]["raw_input"]["file_path"], "/tmp/a.txt");
        assert_eq!(json["data"]["options"][0]["option_id"], "allow");
        assert_eq!(json["data"]["options"][0]["kind"], "allow_once");
        assert!(json["data"].get("toolCall").is_none());
        assert!(json["data"]["options"][0].get("optionId").is_none());
    }

    #[test]
    fn turn_completed_event_roundtrip_and_backcompat() {
        // Serializes under the snake_case wire tag with all metric fields.
        let event = AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
            elapsed_ms: 1234,
            input_tokens: 500,
            output_tokens: 250,
            cache_creation_tokens: 120,
            cache_read_tokens: 380,
            context_tokens: 8000,
            context_window: 100_000,
            stop_reason: Some(TurnStopReason::EndTurn),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "turn_completed");
        assert_eq!(json["data"]["elapsed_ms"], 1234);
        assert_eq!(json["data"]["input_tokens"], 500);
        assert_eq!(json["data"]["output_tokens"], 250);
        assert_eq!(json["data"]["cache_creation_tokens"], 120);
        assert_eq!(json["data"]["cache_read_tokens"], 380);
        assert_eq!(json["data"]["context_tokens"], 8000);
        assert_eq!(json["data"]["context_window"], 100_000);
        assert_eq!(json["data"]["stop_reason"], "end_turn");

        // Back-compat: an old payload with no stop_reason / context fields
        // deserializes to defaults (None / 0) via `#[serde(default)]`.
        let old = serde_json::json!({
            "type": "turn_completed",
            "data": { "elapsed_ms": 1, "input_tokens": 2, "output_tokens": 3 }
        });
        let back: AgentStreamEvent = serde_json::from_value(old).unwrap();
        assert!(matches!(
            back,
            AgentStreamEvent::TurnCompleted(d)
                if d.stop_reason.is_none() && d.context_tokens == 0 && d.context_window == 0
        ));
    }

    #[test]
    fn wire_type_tags_are_stable_protocol_contract() {
        // The `type` tag is the wire contract the frontend switches on. This
        // locks it to the Rust structs (dep-free drift guard — the §3.6
        // single-source-of-truth goal without a TS-codegen dependency). If a
        // variant's tag changes here, the frontend must change in lockstep.
        let cases: Vec<(AgentStreamEvent, &str)> = vec![
            (AgentStreamEvent::Start(StartEventData::default()), "start"),
            (AgentStreamEvent::Text(TextEventData { content: "x".into() }), "content"),
            (
                AgentStreamEvent::Tips(TipsEventData { content: "x".into(), tip_type: TipType::Warning }),
                "tips",
            ),
            (AgentStreamEvent::TurnCompleted(TurnCompletedEventData::default()), "turn_completed"),
            (AgentStreamEvent::Finish(FinishEventData::default()), "finish"),
            (AgentStreamEvent::Error(ErrorEventData::legacy("e", None)), "error"),
            (AgentStreamEvent::Permission(serde_json::json!({})), "permission"),
            (AgentStreamEvent::AcpModelInfo(serde_json::json!({})), "acp_model_info"),
            (AgentStreamEvent::AcpModeInfo(serde_json::json!({})), "acp_mode_info"),
            (AgentStreamEvent::AcpConfigOption(serde_json::json!({})), "acp_config_option"),
            (AgentStreamEvent::AcpSessionInfo(serde_json::json!({})), "acp_session_info"),
            (AgentStreamEvent::AcpContextUsage(serde_json::json!({})), "acp_context_usage"),
            (AgentStreamEvent::AcpPromptHookWarning(serde_json::json!({})), "acp_prompt_hook_warning"),
            (AgentStreamEvent::SlashCommandsUpdated(serde_json::json!({})), "slash_commands_updated"),
            (AgentStreamEvent::System(serde_json::json!({})), "system"),
            (AgentStreamEvent::RequestTrace(serde_json::json!({})), "request_trace"),
            (
                AgentStreamEvent::SessionAssigned(SessionAssignedEventData { session_id: "s".into() }),
                "session_assigned",
            ),
        ];
        for (event, expected_tag) in cases {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(
                json["type"], expected_tag,
                "wire `type` tag drifted for {expected_tag:?}: got {:?}",
                json["type"]
            );
        }
    }

    #[test]
    fn thinking_event_roundtrip() {
        let event = AgentStreamEvent::Thinking(ThinkingEventData {
            content: "Analyzing...".into(),
            subject: Some("code review".into()),
            duration: Some(1500),
            status: Some("in_progress".into()),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "thinking");
        assert_eq!(json["data"]["duration"], 1500);
    }
}
