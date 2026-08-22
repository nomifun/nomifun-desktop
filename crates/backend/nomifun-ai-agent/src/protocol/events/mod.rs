pub mod permission;
pub mod session_updates;
pub mod tool_call;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use nomifun_api_types::AgentStreamErrorData as ErrorEventData;

pub use permission::PermissionEventData;
pub use session_updates::{
    AgentStatusEventData, AvailableCommandsEventData, CronTriggerEventData, PlanEventData, SkillSuggestEventData,
    ThinkingEventData,
};
pub use tool_call::{
    ToolCallEventData, ToolCallRetryData, ToolCallStatus, ToolGroupEntry,
    validate_artifact_receipt_integrity, validate_completed_artifact_contract,
};

/// Events emitted by an Agent during a message processing turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentStreamEvent {
    Start(StartEventData),
    /// The current attempt's provisional assistant output was rejected and the
    /// engine is restarting from the most recent `Start` checkpoint.
    OutputDiscarded(OutputDiscardedEventData),
    #[serde(rename = "content")]
    Text(TextEventData),
    Tips(TipsEventData),
    ToolCall(ToolCallEventData),
    ToolGroup(Vec<ToolGroupEntry>),
    AgentStatus(AgentStatusEventData),
    Thinking(ThinkingEventData),
    Plan(PlanEventData),
    Permission(PermissionEventData),
    SkillSuggest(SkillSuggestEventData),
    CronTrigger(CronTriggerEventData),
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
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct StartEventData {
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Data for the `OutputDiscarded` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct OutputDiscardedEventData {
    /// One-based round attempt that will produce the replacement output.
    #[ts(type = "number")]
    pub restart_attempt: u32,
}

/// Data for the `SessionAssigned` event.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
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
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
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
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct TurnCompletedEventData {
    /// Wall-clock duration of the turn in milliseconds.
    #[ts(type = "number")]
    pub elapsed_ms: i64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Provider-reported reasoning tokens; a subset of output_tokens.
    #[serde(default)]
    #[ts(type = "number")]
    pub reasoning_tokens: u64,
    /// Current context occupancy (last request's prompt tokens). Gauge numerator.
    #[serde(default)]
    #[ts(type = "number")]
    pub context_tokens: u64,
    /// Effective context budget (engine compaction window). Gauge denominator.
    #[serde(default)]
    #[ts(type = "number")]
    pub context_window: u64,
}

/// Cross-backend normalized "why did the turn end" reason. Deliberately NOT the
/// external protocol's own stop-reason enum, so the shared event type stays
/// engine-neutral; each backend maps its own outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
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
    use std::path::Path;
    use ts_rs::Config;

    // ts-rs preserves formatting whitespace from wrapped declarations. Keep
    // committed bindings platform-independent and stable across test runs.
    fn normalize_typescript_binding(generated: &str) -> String {
        let mut normalized = generated
            .lines()
            .map(|line| line.trim_end_matches([' ', '\t']))
            .collect::<Vec<_>>()
            .join("\n");
        while normalized.ends_with('\n') {
            normalized.pop();
        }
        normalized.push('\n');
        normalized
    }

    fn export_binding_if_changed<T: TS + 'static>(file_name: &str) {
        let generated = normalize_typescript_binding(
            &T::export_to_string(&Config::default())
                .unwrap_or_else(|error| panic!("{file_name} must export to TypeScript: {error}")),
        );
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../ui/src/common/protocolBindings")
            .join(file_name);
        let unchanged = std::fs::read_to_string(&path)
            .map(|current| current == generated)
            .unwrap_or(false);
        if !unchanged {
            std::fs::write(&path, generated)
                .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        }
    }

    #[test]
    fn export_protocol_bindings() {
        export_binding_if_changed::<StartEventData>("StartEventData.ts");
        export_binding_if_changed::<OutputDiscardedEventData>("OutputDiscardedEventData.ts");
        export_binding_if_changed::<SessionAssignedEventData>("SessionAssignedEventData.ts");
        export_binding_if_changed::<FinishEventData>("FinishEventData.ts");
        export_binding_if_changed::<TurnCompletedEventData>("TurnCompletedEventData.ts");
        export_binding_if_changed::<TurnStopReason>("TurnStopReason.ts");
    }

    #[test]
    fn generated_bindings_have_deterministic_whitespace() {
        assert_eq!(
            normalize_typescript_binding("export type Value = { \r\nfield: string,\t\r\n}\r\n\r\n"),
            "export type Value = {\nfield: string,\n}\n"
        );
    }
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
            retry: None,
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
            retry: None,
            artifacts: Vec::new(),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tool_call");
        assert_eq!(json["data"]["input"]["pattern"], "**/*.rs");
        assert_eq!(json["data"]["output"], "src/main.rs\nsrc/lib.rs");
        assert_eq!(json["data"]["description"], "Search for Rust files");
    }

    #[test]
    fn tool_call_retry_identity_roundtrips_and_legacy_events_default_to_none() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "nomi-call-2".into(),
            name: "nomi_delegate".into(),
            args: json!({ "strategy": "parallel" }),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("ok".into()),
            description: None,
            retry: Some(ToolCallRetryData {
                retry_group_id: "nomi-call-1".into(),
                attempt_no: 2,
                retry_of_call_id: Some("nomi-call-1".into()),
            }),
            artifacts: Vec::new(),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["data"]["retry"]["retry_group_id"], "nomi-call-1");
        assert_eq!(json["data"]["retry"]["attempt_no"], 2);
        assert_eq!(json["data"]["retry"]["retry_of_call_id"], "nomi-call-1");
        let parsed: AgentStreamEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(
            parsed,
            AgentStreamEvent::ToolCall(ToolCallEventData {
                retry: Some(ToolCallRetryData { attempt_no: 2, .. }),
                ..
            })
        ));

        let legacy: AgentStreamEvent = serde_json::from_value(json!({
            "type": "tool_call",
            "data": {
                "call_id": "legacy-call",
                "name": "Read",
                "args": {},
                "status": "completed",
                "artifacts": []
            }
        }))
        .unwrap();
        assert!(matches!(
            legacy,
            AgentStreamEvent::ToolCall(ToolCallEventData { retry: None, .. })
        ));
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
            retry: None,
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
    fn turn_completed_event_roundtrip_and_backcompat() {
        // Serializes under the snake_case wire tag with all metric fields.
        let event = AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
            elapsed_ms: 1234,
            input_tokens: 500,
            output_tokens: 250,
            reasoning_tokens: 125,
            context_tokens: 8000,
            context_window: 100_000,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "turn_completed");
        assert_eq!(json["data"]["elapsed_ms"], 1234);
        assert_eq!(json["data"]["input_tokens"], 500);
        assert_eq!(json["data"]["output_tokens"], 250);
        assert_eq!(json["data"]["reasoning_tokens"], 125);
        assert_eq!(json["data"]["context_tokens"], 8000);
        assert_eq!(json["data"]["context_window"], 100_000);

        // Back-compat: an old payload with extra retired fields and no context
        // fields deserializes cleanly (unknown keys ignored, defaults applied).
        let old = serde_json::json!({
            "type": "turn_completed",
            "data": {
                "elapsed_ms": 1, "input_tokens": 2, "output_tokens": 3,
                "cache_read_tokens": 4, "stop_reason": "end_turn"
            }
        });
        let back: AgentStreamEvent = serde_json::from_value(old).unwrap();
        assert!(matches!(
            back,
            AgentStreamEvent::TurnCompleted(d)
                if d.reasoning_tokens == 0 && d.context_tokens == 0 && d.context_window == 0
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
            (
                AgentStreamEvent::OutputDiscarded(OutputDiscardedEventData {
                    restart_attempt: 2,
                }),
                "output_discarded",
            ),
            (AgentStreamEvent::Text(TextEventData { content: "x".into() }), "content"),
            (
                AgentStreamEvent::Tips(TipsEventData { content: "x".into(), tip_type: TipType::Warning }),
                "tips",
            ),
            (AgentStreamEvent::TurnCompleted(TurnCompletedEventData::default()), "turn_completed"),
            (AgentStreamEvent::Finish(FinishEventData::default()), "finish"),
            (AgentStreamEvent::Error(ErrorEventData::legacy("e", None)), "error"),
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
