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
    validate_artifact_receipt_integrity, validate_completed_artifact_contract,
};
pub(crate) use translate::{
    AcpArtifactDeliveryState, permission_request_to_event_data, session_notification_to_events,
    session_notification_to_events_with_delivery_state,
};
#[cfg(test)]
pub(crate) use translate::session_notification_to_events_with_store;

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
        ContentBlock as SdkContentBlock, ImageContent, PermissionOption,
        PermissionOptionKind as SdkPermissionOptionKind, RequestPermissionRequest, ResourceLink,
        SessionNotification, SessionUpdate, ToolCall as SdkToolCall, ToolCallContent,
        ToolCallLocation,
        ToolCallStatus as SdkToolCallStatus, ToolCallUpdate as SdkToolCallUpdate,
        ToolCallUpdateFields, ToolKind as SdkToolKind, TextContent,
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
            artifacts: Vec::new(),
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
            artifacts: Vec::new(),
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
            artifacts: Vec::new(),
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
    fn acp_image_content_is_persisted_and_never_serialized_as_base64() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new(PNG, "image/png")
                            .uri(format!("data:image/png;base64,{PNG}")),
                    ))]),
            ),
        );

        let events = session_notification_to_events_with_store(&notif, Some(&store));
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "completed");
        assert_eq!(json["data"]["update"]["content"][0]["type"], "artifact");
        let path = json["data"]["update"]["content"][0]["artifact"]["path"]
            .as_str()
            .unwrap();
        assert!(std::path::Path::new(path).is_file());
        assert!(!json.to_string().contains(PNG), "base64 must not enter event/history JSON");
    }

    #[test]
    fn acp_invalid_image_forces_failed_status_and_explicit_error_content() {
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new("bm90IGFuIGltYWdl", "image/png"),
                    ))]),
            ),
        );

        let events = session_notification_to_events_with_store(&notif, Some(&store));
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "failed");
        assert_eq!(json["data"]["update"]["content"][0]["type"], "artifact_error");
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn acp_artifact_tool_cannot_complete_with_empty_or_text_only_output() {
        for content in [
            Vec::new(),
            vec![ToolCallContent::from(SdkContentBlock::Text(TextContent::new(
                "image generated successfully",
            )))],
        ] {
            let mut state = AcpArtifactDeliveryState::default();
            state.begin_turn("sess-1");
            let notif = SessionNotification::new(
                "sess-1",
                SessionUpdate::ToolCall(
                    SdkToolCall::new("tool-image", "Generate image")
                        .status(SdkToolCallStatus::Completed)
                        .content(content),
                ),
            );

            let events = session_notification_to_events_with_delivery_state(&notif, None, &mut state);
            let json = serde_json::to_value(&events[0]).unwrap();
            assert_eq!(json["data"]["update"]["status"], "failed");
            assert!(json["data"]["update"]["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "artifact_error"));
            assert!(state.turn_failure("sess-1").is_some());
        }
    }

    #[test]
    fn acp_artifact_delivery_failure_is_absorbing_for_late_completed_update() {
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("sess-1");

        let invalid = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::InProgress)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new("bm90IGFuIGltYWdl", "image/png"),
                    ))]),
            ),
        );
        let first = session_notification_to_events_with_delivery_state(&invalid, Some(&store), &mut state);
        assert_eq!(
            serde_json::to_value(&first[0]).unwrap()["data"]["update"]["status"],
            "failed"
        );

        let late_completed = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-image",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Completed),
            )),
        );
        let second =
            session_notification_to_events_with_delivery_state(&late_completed, Some(&store), &mut state);
        let json = serde_json::to_value(&second[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "failed");
        assert_eq!(json["data"]["update"]["content"][0]["type"], "artifact_error");
        assert!(state.turn_failure("sess-1").is_some());
    }

    #[test]
    fn acp_non_artifact_tool_may_complete_without_content() {
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("sess-1");
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-terminal", "Terminal")
                    .status(SdkToolCallStatus::Completed),
            ),
        );

        let events = session_notification_to_events_with_delivery_state(&notif, None, &mut state);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "completed");
        assert!(state.turn_failure("sess-1").is_none());
    }

    #[test]
    fn acp_end_turn_seal_rejects_artifact_call_left_in_progress() {
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("sess-1");
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::InProgress),
            ),
        );

        let events = session_notification_to_events_with_delivery_state(&notif, None, &mut state);
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap()["data"]["update"]["status"],
            "in_progress"
        );
        assert!(state.turn_failure("sess-1").is_none());
        assert!(state.finish_turn("sess-1").is_some());
        assert!(state.turn_failure("sess-1").is_some());
    }

    #[test]
    fn acp_separate_failed_artifact_call_is_not_hidden_by_later_success() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());

        let failed = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image-1", "Generate image")
                    .status(SdkToolCallStatus::Failed),
            ),
        );

        let mut failed_only_state = AcpArtifactDeliveryState::default();
        failed_only_state.begin_turn("sess-1");
        let _ = session_notification_to_events_with_delivery_state(
            &failed,
            Some(&store),
            &mut failed_only_state,
        );
        assert!(failed_only_state.turn_failure("sess-1").is_none());
        assert!(failed_only_state.finish_turn("sess-1").is_some());

        let mut retried_state = AcpArtifactDeliveryState::default();
        retried_state.begin_turn("sess-1");
        let _ = session_notification_to_events_with_delivery_state(
            &failed,
            Some(&store),
            &mut retried_state,
        );
        let retry = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image-2", "Generate image")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new(PNG, "image/png"),
                    ))]),
            ),
        );
        let retry_events = session_notification_to_events_with_delivery_state(
            &retry,
            Some(&store),
            &mut retried_state,
        );
        assert_eq!(
            serde_json::to_value(&retry_events[0]).unwrap()["data"]["update"]["status"],
            "completed"
        );
        assert!(
            retried_state.finish_turn("sess-1").is_some(),
            "without explicit retry lineage, one successful call must not hide a separate failed artifact call"
        );
    }

    #[test]
    fn acp_path_artifacts_require_a_pre_terminal_baseline_and_real_change() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("report.md"), "# Report\n").unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("sess-1");
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-generic", "Worker")
                    .status(SdkToolCallStatus::Completed)
                    .raw_output(json!({ "result": { "path": "report.md" } })),
            ),
        );

        let events = session_notification_to_events_with_delivery_state(&notif, Some(&store), &mut state);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "failed");
        assert_eq!(json["data"]["update"]["rawOutput"]["result"]["path"], "report.md");
        assert!(state.finish_turn("sess-1").is_some());

        let mut input_state = AcpArtifactDeliveryState::default();
        input_state.begin_turn("sess-2");
        let input_path_started = SessionNotification::new(
            "sess-2",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-report", "Create report")
                    .status(SdkToolCallStatus::InProgress)
                    .raw_input(json!({ "artifact_path": "fresh-report.md" })),
            ),
        );
        let _ = session_notification_to_events_with_delivery_state(
            &input_path_started,
            Some(&store),
            &mut input_state,
        );
        std::fs::write(workspace.path().join("fresh-report.md"), "# Fresh report\n").unwrap();
        let input_path_completed = SessionNotification::new(
            "sess-2",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-report",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Completed),
            )),
        );
        let input_events = session_notification_to_events_with_delivery_state(
            &input_path_completed,
            Some(&store),
            &mut input_state,
        );
        let input_json = serde_json::to_value(&input_events[0]).unwrap();
        assert_eq!(input_json["data"]["update"]["status"], "completed");
        assert_eq!(input_json["data"]["update"]["content"][0]["type"], "artifact");
    }

    #[test]
    fn acp_inline_artifact_batch_is_all_or_none() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![
                        ToolCallContent::from(SdkContentBlock::Image(ImageContent::new(
                            PNG,
                            "image/png",
                        ))),
                        ToolCallContent::from(SdkContentBlock::Image(ImageContent::new(
                            "bm90IGFuIGltYWdl",
                            "image/png",
                        ))),
                    ]),
            ),
        );

        let events = session_notification_to_events_with_store(&notif, Some(&store));
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "failed");
        assert!(json["data"]["update"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] == "artifact_error"));
        assert!(
            !workspace.path().join("nomifun-artifacts").exists(),
            "a rejected inline batch must not leave the valid first artifact behind"
        );
    }

    #[test]
    fn acp_inline_and_non_inline_artifacts_are_preflighted_as_one_unit() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_path = outside_dir.path().join("outside.png");
        std::fs::write(&outside_path, b"outside").unwrap();
        let outside_uri = url::Url::from_file_path(&outside_path).unwrap().to_string();

        for rejected_uri in [
            "blob:https://example.invalid/temporary-image".to_owned(),
            outside_uri,
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let store = crate::artifact_store::ArtifactStore::new(workspace.path());
            let notif = SessionNotification::new(
                "sess-1",
                SessionUpdate::ToolCall(
                    SdkToolCall::new("tool-image", "Generate image")
                        .status(SdkToolCallStatus::Completed)
                        .content(vec![
                            ToolCallContent::from(SdkContentBlock::Image(ImageContent::new(
                                PNG,
                                "image/png",
                            ))),
                            ToolCallContent::from(SdkContentBlock::ResourceLink(
                                ResourceLink::new("rejected image", rejected_uri),
                            )),
                        ]),
                ),
            );

            let events = session_notification_to_events_with_store(&notif, Some(&store));
            let json = serde_json::to_value(&events[0]).unwrap();
            assert_eq!(json["data"]["update"]["status"], "failed");
            assert!(json["data"]["update"]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["type"] == "artifact_error"));
            assert!(
                !workspace.path().join("nomifun-artifacts").exists(),
                "preflight failure must prevent every inline artifact write"
            );
        }
    }

    #[test]
    fn acp_completed_update_reattaches_prior_verified_receipts_but_failed_does_not() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());

        let mut completed_state = AcpArtifactDeliveryState::default();
        completed_state.begin_turn("sess-completed");
        let started = SessionNotification::new(
            "sess-completed",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::InProgress)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new(PNG, "image/png"),
                    ))]),
            ),
        );
        let started_events = session_notification_to_events_with_delivery_state(
            &started,
            Some(&store),
            &mut completed_state,
        );
        let started_json = serde_json::to_value(&started_events[0]).unwrap();
        assert_eq!(started_json["data"]["update"]["status"], "in_progress");
        let expected_receipt = started_json["data"]["update"]["content"][0]["artifact"].clone();

        let completed = SessionNotification::new(
            "sess-completed",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-image",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Completed),
            )),
        );
        let completed_events = session_notification_to_events_with_delivery_state(
            &completed,
            Some(&store),
            &mut completed_state,
        );
        let completed_json = serde_json::to_value(&completed_events[0]).unwrap();
        assert_eq!(completed_json["data"]["update"]["status"], "completed");
        assert_eq!(
            completed_json["data"]["update"]["content"][0]["artifact"],
            expected_receipt
        );
        assert!(completed_state.finish_turn("sess-completed").is_none());

        let mut failed_state = AcpArtifactDeliveryState::default();
        failed_state.begin_turn("sess-failed");
        let failed_started = SessionNotification::new(
            "sess-failed",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::InProgress)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new(PNG, "image/png"),
                    ))]),
            ),
        );
        let _ = session_notification_to_events_with_delivery_state(
            &failed_started,
            Some(&store),
            &mut failed_state,
        );
        let failed = SessionNotification::new(
            "sess-failed",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-image",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Failed),
            )),
        );
        let failed_events = session_notification_to_events_with_delivery_state(
            &failed,
            Some(&store),
            &mut failed_state,
        );
        let failed_json = serde_json::to_value(&failed_events[0]).unwrap();
        assert_eq!(failed_json["data"]["update"]["status"], "failed");
        assert!(failed_json["data"]["update"]["content"]
            .as_array()
            .map(|items| items.iter().all(|item| item["type"] != "artifact"))
            .unwrap_or(true));
        assert!(failed_state.finish_turn("sess-failed").is_some());
    }

    #[test]
    fn acp_finish_reverifies_and_rejects_a_receipt_deleted_by_a_later_tool() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("sess-delete-receipt");
        let completed = SessionNotification::new(
            "sess-delete-receipt",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-image", "Generate image")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::Image(
                        ImageContent::new(PNG, "image/png"),
                    ))]),
            ),
        );

        let events = session_notification_to_events_with_delivery_state(
            &completed,
            Some(&store),
            &mut state,
        );
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "completed");
        let receipt_path = json["data"]["update"]["content"][0]["artifact"]["path"]
            .as_str()
            .unwrap();
        std::fs::remove_file(receipt_path).unwrap();

        let error = state
            .finish_turn_with_store("sess-delete-receipt", Some(&store))
            .expect("a deleted published locator must fail the accepted turn");
        assert!(error.contains("failed final verification"));
    }

    #[test]
    fn acp_artifact_tool_accepts_only_verified_workspace_location_receipt() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("report.md"), "# Old report\n").unwrap();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());
        let mut state = AcpArtifactDeliveryState::default();
        state.begin_turn("sess-1");

        let started = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-report", "Create report")
                    .status(SdkToolCallStatus::InProgress)
                    .locations(vec![ToolCallLocation::new("report.md")]),
            ),
        );
        let _ = session_notification_to_events_with_delivery_state(&started, Some(&store), &mut state);

        let completed = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-report",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Completed),
            )),
        );
        let events =
            session_notification_to_events_with_delivery_state(&completed, Some(&store), &mut state);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "failed");
        assert!(state.turn_failure("sess-1").is_some());

        let mut fresh_state = AcpArtifactDeliveryState::default();
        fresh_state.begin_turn("sess-fresh");
        let fresh_started = SessionNotification::new(
            "sess-fresh",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-fresh", "Create report")
                    .status(SdkToolCallStatus::InProgress)
                    .locations(vec![ToolCallLocation::new("fresh.md")]),
            ),
        );
        let _ = session_notification_to_events_with_delivery_state(
            &fresh_started,
            Some(&store),
            &mut fresh_state,
        );
        std::fs::write(workspace.path().join("fresh.md"), "# Fresh\n").unwrap();
        let fresh_completed = SessionNotification::new(
            "sess-fresh",
            SessionUpdate::ToolCallUpdate(SdkToolCallUpdate::new(
                "tool-fresh",
                ToolCallUpdateFields::new().status(SdkToolCallStatus::Completed),
            )),
        );
        let fresh_events = session_notification_to_events_with_delivery_state(
            &fresh_completed,
            Some(&store),
            &mut fresh_state,
        );
        let fresh_json = serde_json::to_value(&fresh_events[0]).unwrap();
        assert_eq!(fresh_json["data"]["update"]["status"], "completed");
        assert_eq!(fresh_json["data"]["update"]["content"][0]["type"], "artifact");
        let fresh_artifact = &fresh_json["data"]["update"]["content"][0]["artifact"];
        assert!(
            fresh_artifact["relative_path"]
                .as_str()
                .is_some_and(|path| path.starts_with("nomifun-artifacts/"))
        );
        let snapshot_path = std::path::PathBuf::from(fresh_artifact["path"].as_str().unwrap());
        assert!(snapshot_path.starts_with(std::fs::canonicalize(store.artifact_root()).unwrap()));
        std::fs::write(workspace.path().join("fresh.md"), "# Overwritten\n").unwrap();
        std::fs::remove_file(workspace.path().join("fresh.md")).unwrap();
        assert_eq!(std::fs::read(&snapshot_path).unwrap(), b"# Fresh\n");
        let snapshot = store.verify_existing_path(&snapshot_path).unwrap();
        assert_eq!(
            snapshot.sha256,
            fresh_artifact["sha256"].as_str().unwrap(),
            "the published receipt must keep the immutable snapshot hash after the source is overwritten and deleted"
        );

        let mut missing_state = AcpArtifactDeliveryState::default();
        missing_state.begin_turn("sess-2");
        let missing = SessionNotification::new(
            "sess-2",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-report", "Create report")
                    .status(SdkToolCallStatus::Completed)
                    .locations(vec![ToolCallLocation::new("missing.md")]),
            ),
        );
        let missing_events =
            session_notification_to_events_with_delivery_state(&missing, Some(&store), &mut missing_state);
        assert_eq!(
            serde_json::to_value(&missing_events[0]).unwrap()["data"]["update"]["status"],
            "failed"
        );
    }

    #[test]
    fn acp_remote_resource_link_is_visible_but_not_verified_delivery() {
        let notif = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-file", "Create report")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::ResourceLink(
                        ResourceLink::new("report.pdf", "https://example.invalid/report.pdf"),
                    ))]),
            ),
        );

        let events = session_notification_to_events(&notif);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "failed");
        assert_eq!(json["data"]["update"]["content"][0]["type"], "resource_link");
        assert_eq!(
            json["data"]["update"]["content"][0]["uri"],
            "https://example.invalid/report.pdf"
        );
        assert!(json["data"]["update"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "artifact_error"));
    }

    #[test]
    fn acp_file_resource_link_requires_verified_workspace_receipt() {
        let workspace = tempfile::tempdir().unwrap();
        let report_path = workspace.path().join("report.md");
        std::fs::write(&report_path, "# Report\n").unwrap();
        let report_uri = url::Url::from_file_path(&report_path).unwrap().to_string();
        let store = crate::artifact_store::ArtifactStore::new(workspace.path());

        let valid = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-file", "Create report")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::ResourceLink(
                        ResourceLink::new("report.md", report_uri),
                    ))]),
            ),
        );
        let events = session_notification_to_events_with_store(&valid, Some(&store));
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["data"]["update"]["status"], "completed");
        assert_eq!(json["data"]["update"]["content"][0]["type"], "artifact");
        assert!(
            json["data"]["update"]["content"][0]["artifact"]["relative_path"]
                .as_str()
                .is_some_and(|path| path.starts_with("nomifun-artifacts/"))
        );

        let outside_dir = tempfile::tempdir().unwrap();
        let outside = outside_dir.path().join("outside.txt");
        std::fs::write(&outside, "outside").unwrap();
        let outside_uri = url::Url::from_file_path(&outside).unwrap().to_string();
        let invalid = SessionNotification::new(
            "sess-1",
            SessionUpdate::ToolCall(
                SdkToolCall::new("tool-file", "Create report")
                    .status(SdkToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(SdkContentBlock::ResourceLink(
                        ResourceLink::new("outside.txt", outside_uri),
                    ))]),
            ),
        );
        let invalid_events = session_notification_to_events_with_store(&invalid, Some(&store));
        let invalid_json = serde_json::to_value(&invalid_events[0]).unwrap();
        assert_eq!(invalid_json["data"]["update"]["status"], "failed");
        assert_eq!(invalid_json["data"]["update"]["content"][0]["type"], "artifact_error");
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
