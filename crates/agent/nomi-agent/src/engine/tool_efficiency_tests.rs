use super::{AgentResult, CompletionAdjudication, ToolEfficiencyStats};
use crate::tool_execution::SKIPPED_AFTER_PRIOR_ERROR;
use nomi_process_runtime::{CapabilityPolicy, ProcessSupervisor, SupervisorConfig};
use nomi_tools::{
    exec_command::ExecCommandTool, process_store::ProcessStore, read::ReadTool,
    registry::ToolRegistry,
};
use nomi_types::message::{ContentBlock, StopReason, TokenUsage};
use serde_json::json;
use std::sync::Arc;

fn efficiency_registry() -> ToolRegistry {
    let cwd = std::env::current_dir().expect("current directory");
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadTool::new(None, Some(cwd.clone()))));
    registry.register(Box::new(ExecCommandTool::new(
        ProcessSupervisor::new(SupervisorConfig::default()),
        Arc::new(ProcessStore::new()),
        cwd.clone(),
        CapabilityPolicy::local_owner(cwd),
    )));
    registry
}

#[test]
fn accounting_distinguishes_parallel_width_scripts_and_batch_reads() {
    let calls = vec![
        ContentBlock::ToolUse {
            id: "exec".into(),
            name: "exec_command".into(),
            input: json!({ "script": "print('x')", "language": "python", "timeout": 1000 }),
            extra: None,
        },
        ContentBlock::ToolUse {
            id: "read".into(),
            name: "Read".into(),
            input: json!({ "file_paths": ["a", "b", "c"] }),
            extra: None,
        },
        ContentBlock::ToolUse {
            id: "grep".into(),
            name: "Grep".into(),
            input: json!({ "pattern": "needle" }),
            extra: None,
        },
    ];
    let mut stats = ToolEfficiencyStats::default();
    let registry = efficiency_registry();

    stats.observe_model_turn_attempt();
    stats.observe_model_turn_attempt();
    stats.observe_calls(&registry, &calls);
    stats.observe_calls(&registry, &calls[..1]);

    assert_eq!(stats.model_turn_attempts, 2);
    assert_eq!(stats.model_turns_with_tools, 2);
    assert_eq!(stats.total_tool_calls, 4);
    assert_eq!(stats.max_calls_in_model_turn, 3);
    assert_eq!(stats.exec_command_script_calls, 2);
    assert_eq!(stats.batch_read_files_requested, 3);
}

#[test]
fn accounting_does_not_interpret_schema_invalid_string_as_a_batch() {
    let calls = vec![
        ContentBlock::ToolUse {
            id: "exec".into(),
            name: "exec_command".into(),
            input: json!({"script":"print(1)","language":"python","timeout":1000}),
            extra: None,
        },
        ContentBlock::ToolUse {
            id: "read".into(),
            name: "Read".into(),
            input: json!({ "file_paths": "[\"a\",\"b\"]" }),
            extra: None,
        },
    ];
    let mut stats = ToolEfficiencyStats::default();

    stats.observe_calls(&efficiency_registry(), &calls);

    assert_eq!(stats.exec_command_script_calls, 1);
    assert_eq!(stats.batch_read_files_requested, 0);
}

#[test]
fn accounting_counts_errors_and_prior_error_skips() {
    let results = vec![
        ContentBlock::ToolResult {
            tool_use_id: "failed".into(),
            content: "boom".into(),
            is_error: true,
            images: vec![],
        },
        ContentBlock::ToolResult {
            tool_use_id: "skipped".into(),
            content: SKIPPED_AFTER_PRIOR_ERROR.into(),
            is_error: true,
            images: vec![],
        },
    ];
    let mut stats = ToolEfficiencyStats::default();

    stats.observe_results(&results);

    assert_eq!(stats.error_results, 2);
    assert_eq!(stats.skipped_after_prior_error, 1);
}

#[test]
fn terminal_dimensions_report_only_clean_end_turn_as_success() {
    let stats = ToolEfficiencyStats::default();
    for (stop_reason, terminal, error_kind) in [
        (StopReason::EndTurn, "ok", "none"),
        (StopReason::MaxTokens, "error", "output_truncated"),
        (
            StopReason::MaxTurns,
            "error",
            "turn_requests_exhausted",
        ),
        (StopReason::Refusal, "error", "model_refused"),
        (StopReason::ToolUse, "error", "protocol_error"),
    ] {
        let result = Ok(AgentResult {
            text: String::new(),
            stop_reason,
            usage: TokenUsage::default(),
            turns: 1,
            rounds: 1,
            effects_ok: 0,
            durable_effect_targets: Vec::new(),
            cutoff_state_changing: 0,
            state_changing_tools_advertised: false,
            completion_adjudication: None,
        });
        let dimensions = stats.terminal_dimensions(&result);
        assert_eq!(dimensions.0, terminal, "stop_reason={stop_reason:?}");
        assert_eq!(dimensions.2, error_kind, "stop_reason={stop_reason:?}");
    }
}

#[test]
fn terminal_dimensions_preserve_each_typed_adjudication_kind() {
    let stats = ToolEfficiencyStats::default();
    for (issue, expected) in [
        (
            CompletionAdjudication::UnbackedStateChangeClaim {
                target: "miniapp.html".to_owned(),
            },
            "unbacked_state_change_claim",
        ),
        (
            CompletionAdjudication::HistoryRollbackFailed {
                target: "miniapp.html".to_owned(),
            },
            "completion_history_rollback_failed",
        ),
        (
            CompletionAdjudication::SessionCommitFailed {
                detail: "save failed".to_owned(),
            },
            "completion_history_commit_failed",
        ),
    ] {
        let result = Ok(AgentResult {
            text: String::new(),
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            turns: 1,
            rounds: 1,
            effects_ok: 0,
            durable_effect_targets: Vec::new(),
            cutoff_state_changing: 0,
            state_changing_tools_advertised: false,
            completion_adjudication: Some(issue),
        });
        let dimensions = stats.terminal_dimensions(&result);
        assert_eq!(dimensions.0, "error");
        assert_eq!(dimensions.2, expected);
    }
}
