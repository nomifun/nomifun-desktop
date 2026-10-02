//! Orthogonal execution-discipline policy for frozen platform tools.
//!
//! `EngineEffectClass` answers ordering and uncertainty questions. It does not
//! say that every external effect is a long-horizon coding task, nor that every
//! effect mutates the workspace. Keeping those dimensions separate prevents an
//! atomic product action (for example submitting one media-generation job)
//! from activating the planning/completion ledger or invalidating repository
//! observations.

use crate::{AgentToolBinding, AgentToolResult};

/// Tools whose use can grow into a multi-step task that needs source-anchored
/// requirements and completion accounting. Explicit `update_plan`, steering,
/// and task continuation can still activate the ledger independently.
pub(crate) fn requires_task_ledger(binding: &AgentToolBinding) -> bool {
    matches!(
        binding.capability_id.as_ref(),
        "workspace.files"
            | "workspace.vcs"
            | "workspace.process"
            | "workspace.artifacts"
            | "ssh"
            | "requirements"
    )
}

/// Whether an attempted tool can invalidate observations of the local
/// workspace/repository. External effects such as media generation, Browser,
/// Computer, notifications, devices, and schedules are deliberately excluded.
pub(crate) fn affects_workspace(binding: &AgentToolBinding) -> bool {
    match binding.capability_id.as_ref() {
        // A process can keep mutating after a read-shaped poll, so every
        // process action invalidates observations until cleanup proves idle.
        "workspace.process" => true,
        "workspace.files" | "workspace.vcs" | "workspace.artifacts" => {
            !matches!(
                binding.effect_class,
                nomifun_engine_core::EngineEffectClass::ReadOnly
            )
        }
        _ => false,
    }
}

/// A process call can execute correctly while the command itself exits with
/// failure. The tool result is still a valid observation, but later effects
/// must wait for the model to inspect that outcome and replan.
pub(crate) fn failed_process_observation(
    binding: &AgentToolBinding,
    result: &AgentToolResult,
) -> bool {
    if binding.capability_id.as_ref() != "workspace.process" || process_operation_not_applied(binding, result) {
        return false;
    }
    let Some(value) = serde_json::from_str::<serde_json::Value>(&result.output_text()).ok() else {
        return false;
    };
    let reaped_cancellation = matches!(
        binding.action_id.as_ref(),
        "workspace.process/cancel" | "workspace.process/poll"
    ) && !result.is_error
        && value.get("state").and_then(serde_json::Value::as_str) == Some("cancelled")
        && value
            .pointer("/cleanup/reaped")
            .and_then(serde_json::Value::as_bool)
            == Some(true);
    if reaped_cancellation {
        // A terminal poll reports success=false because the process is no
        // longer running. The action-specific contract and Kernel both treat
        // a proven reaped cancellation as a successful lifecycle observation,
        // so it must not reopen an otherwise optional runtime plan.
        return false;
    }
    value
        .get("success")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
}

/// A typed owner fact, never parsed from error prose. Cleanup alone cannot
/// establish this: a reaped process may already have modified the workspace.
pub(crate) fn process_did_not_start(binding: &AgentToolBinding, result: &AgentToolResult) -> bool {
    binding.capability_id.as_ref() == "workspace.process"
        && matches!(binding.action_id.as_ref(), "workspace.process/exec" | "workspace.process/start")
        && serde_json::from_str::<serde_json::Value>(&result.output_text()).is_ok_and(|value|
            value["schema"] == "nomifun.process-start-observation.v1"
                && value["state"] == "not_started" && value["user_code_started"] == false
                && value["success"] == false && value.get("process_id").is_none())
}

/// A command outcome that can be reported without another effect/recovery
/// step. Lost ownership, signals and unproven cleanup never qualify.
pub(crate) fn settled_nonzero_process(binding: &AgentToolBinding, result: &AgentToolResult) -> bool {
    binding.capability_id.as_ref() == "workspace.process"
        && serde_json::from_str::<serde_json::Value>(&result.output_text()).is_ok_and(|value|
            value["state"] == "exited"
                && value["exit_code"].as_i64().is_some_and(|code| code != 0)
                && value.get("signal").is_none_or(serde_json::Value::is_null)
                && value.pointer("/cleanup/reaped") == Some(&serde_json::json!(true))
                && value.pointer("/cleanup/errors").is_none_or(|errors|
                    errors.as_array().is_some_and(Vec::is_empty)))
}

/// A known reaped timeout is reportable as that terminal observation. This
/// does not certify the original command's goal or authorize another effect.
pub(crate) fn settled_reportable_process(binding: &AgentToolBinding, result: &AgentToolResult) -> bool {
    settled_nonzero_process(binding, result)
        || (binding.capability_id.as_ref() == "workspace.process"
            && serde_json::from_str::<serde_json::Value>(&result.output_text()).is_ok_and(|value|
                value["state"] == "timed_out"
                    && value["success"] == false
                    && value.get("exit_code").is_none_or(serde_json::Value::is_null)
                    && value.pointer("/cleanup/reaped") == Some(&serde_json::json!(true))
                    && value.pointer("/cleanup/errors").is_none_or(|errors|
                        errors.as_array().is_some_and(Vec::is_empty))))
}

/// A trusted owner fact that the named control was rejected before native I/O.
/// This says nothing about whether another, already started process is alive.
pub(crate) fn process_operation_not_applied(binding: &AgentToolBinding, result: &AgentToolResult) -> bool {
    if process_did_not_start(binding, result) { return true; }
    if binding.capability_id.as_ref() != "workspace.process" { return false; }
    let operation = match binding.action_id.as_ref() {
        "workspace.process/poll" => "poll",
        "workspace.process/input" => "stdin",
        "workspace.process/close_stdin" => "close_stdin",
        "workspace.process/resize" => "resize",
        "workspace.process/cancel" => "cancel",
        _ => return false,
    };
    serde_json::from_str::<serde_json::Value>(&result.output_text()).is_ok_and(|value|
        value["schema"] == "nomifun.process-control-observation.v1"
            && value["state"] == "not_executed" && value["code"] == "PROCESS_REFERENCE_INVALID"
            && value["operation"] == operation && value["control_applied"] == false
            && value["success"] == false && value.get("process_id").is_none())
}

/// Input was held before I/O, but its real owner's terminal still must be
/// observed. Invalid references have no such terminal and use the path above.
pub(crate) fn process_control_rejected_terminal(binding: &AgentToolBinding, result: &AgentToolResult) -> bool {
    if binding.capability_id.as_ref() != "workspace.process" { return false; }
    let operation=match binding.action_id.as_ref() {"workspace.process/input"=>"stdin","workspace.process/close_stdin"=>"close_stdin","workspace.process/resize"=>"resize",_=>return false};
    serde_json::from_str::<serde_json::Value>(&result.output_text()).is_ok_and(|value|
        value["schema"]=="nomifun.process-control-observation.v1" && value["code"]=="PROCESS_ALREADY_TERMINATED"
            && value["operation"]==operation && value["control_applied"]==false && value["success"]==false
            && matches!(value["state"].as_str(),Some("exited"|"cancelled"|"timed_out"|"lost"))
            && value["process_id"].as_str().is_some_and(|id|!id.is_empty()&&id.len()<=128)
            && value["cleanup"]["reaped"]==true)
}

/// A read failure or a proposal held before dispatch is not a change of task
/// scope. Let the model correct the call within its existing plan. An
/// attempted effect may have partially happened and still requires recovery.
pub(crate) fn requires_replanning_after_result(
    binding: &AgentToolBinding,
    result: &AgentToolResult,
    attempted: bool,
    plan_revision: u32,
) -> bool {
    if !attempted || process_operation_not_applied(binding, result) {
        return false;
    }
    if failed_process_observation(binding, result) {
        // Some owners mark a nonzero command exit as is_error, others expose
        // it as an ordinary observation. Both use the same recovery policy.
        return plan_revision == 0;
    }
    result.is_error && !matches!(binding.effect_class, nomifun_engine_core::EngineEffectClass::ReadOnly)
}

/// A successful collaboration handoff transfers completion ownership to the
/// durable AgentExecution. The parent turn must stop after the accepted
/// single-call batch so it cannot poll, create sibling Executions, or publish
/// a speculative answer ahead of the authoritative terminal projection.
pub(crate) fn completes_turn_on_success(binding: &AgentToolBinding) -> bool {
    binding.capability_id.as_ref() == "agent.collaboration"
        && matches!(binding.action_id.as_ref(), "agent/delegate" | "agent/fork")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_terminal_control_keeps_epoch_but_observes_actual_timeout() {
        let input=binding("workspace.process","workspace.process/input");
        let mut work=crate::AgentWorkStatus {workspace_observation_epoch:7,..Default::default()};
        work.running_processes.insert("original".into());
        let call=nomifun_chat_model_broker::ChatToolCall {call_id:"late".into(),name:"write_process_stdin".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"process_id":"original","input":"not-sent"}))};
        let mut receipt=serde_json::json!({"schema":"nomifun.process-control-observation.v1","code":"PROCESS_ALREADY_TERMINATED",
            "operation":"stdin","process_id":"original","state":"timed_out","success":false,"control_applied":false,"cleanup":{"reaped":true,"errors":[]}});
        let result=crate::AgentToolResult::text(call.call_id.clone(),receipt.to_string(),true);
        assert!(process_control_rejected_terminal(&input,&result));
        let mut commands=crate::workflow::CommandTracker::default();
        work.observe(&input,&call,&result,&mut commands);
        assert_eq!(work.workspace_observation_epoch,7);
        assert!(work.running_processes.is_empty());
        assert_eq!(work.failed_tools,1);
        assert_eq!(work.failed_commands,1);
        receipt["cleanup"]["reaped"]=serde_json::json!(false);
        let uncertain=crate::AgentToolResult::text(call.call_id.clone(),receipt.to_string(),true);
        assert!(!process_control_rejected_terminal(&input,&uncertain));
        receipt["cleanup"]["reaped"]=serde_json::json!(true);
        receipt["control_applied"]=serde_json::json!(true);
        let applied=crate::AgentToolResult::text(call.call_id,receipt.to_string(),true);
        assert!(!process_control_rejected_terminal(&input,&applied));
    }
    use nomifun_agent_contracts::{
        ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, StrictJsonValue,
    };
    use nomifun_chat_model_broker::ChatToolDefinition;
    use nomifun_engine_core::{EngineEffectClass, EngineToolBinding};
    use std::collections::BTreeSet;

    #[test]
    fn reportable_timeout_requires_known_timeout_and_proven_cleanup() {
        let process=binding("workspace.process","workspace.process/poll");
        let known=serde_json::json!({"state":"timed_out","success":false,"exit_code":null,
            "cleanup":{"reaped":true,"errors":[]}});
        let result=|value:serde_json::Value|crate::AgentToolResult::text("timeout".into(),value.to_string(),true);
        assert!(settled_reportable_process(&process,&result(known.clone())));
        for (pointer,value) in [("/state",serde_json::json!("lost")),("/state",serde_json::json!("not_started")),
            ("/cleanup/reaped",serde_json::json!(false)),("/cleanup/errors",serde_json::json!(["still-running"])),
            ("/exit_code",serde_json::json!(0)),("/success",serde_json::json!(true))] {
            let mut invalid=known.clone();*invalid.pointer_mut(pointer).unwrap()=value;
            assert!(!settled_reportable_process(&process,&result(invalid)),"{pointer} must not open report-only failure closure");
        }
        assert!(!settled_reportable_process(&binding("workspace.files","workspace.files/read"),&result(known)));
    }

    #[test]
    fn reportable_nonzero_excludes_lost_signalled_or_unreaped_processes() {
        let process = binding("workspace.process", "workspace.process/exec");
        let known = serde_json::json!({"state":"exited","exit_code":1,"signal":null,
            "cleanup":{"reaped":true,"errors":[]},"success":false});
        let result = crate::AgentToolResult::text("diagnostic".into(), known.to_string(), true);
        assert!(settled_nonzero_process(&process, &result));
        for invalid in [
            serde_json::json!({"state":"lost","exit_code":1,"cleanup":{"reaped":true}}),
            serde_json::json!({"state":"exited","exit_code":1,"signal":9,"cleanup":{"reaped":true}}),
            serde_json::json!({"state":"exited","exit_code":1,"cleanup":{"reaped":false}}),
            serde_json::json!({"state":"exited","exit_code":1,"cleanup":{"reaped":true,"errors":["unproven"]}}),
        ] {
            let result = crate::AgentToolResult::text("diagnostic".into(), invalid.to_string(), true);
            assert!(!settled_nonzero_process(&process, &result));
        }
    }

    fn binding_with_effect(
        capability: &str,
        action: &str,
        effect_class: EngineEffectClass,
    ) -> EngineToolBinding {
        let definition = ChatToolDefinition {
            name: "tool".into(),
            description: "fixture".into(),
            input_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
            deferred: false,
        };
        EngineToolBinding {
            model_name: definition.name.clone(),
            schema_digest: crate::input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: CanonicalSchemaRef::from("schema://fixture/input"),
            capability_contract_digest: DigestHex::from("a".repeat(64)),
            definition,
            capability_id: CapabilityId::from(capability),
            action_id: ActionId::from(action),
            resource_binding_ids: BTreeSet::new(),
            effect_class,
            parallel_safe: false,
        }
    }

    fn binding(capability: &str, action: &str) -> EngineToolBinding {
        binding_with_effect(
            capability,
            action,
            EngineEffectClass::ExternalUncertainEffect,
        )
    }

    #[test]
    fn media_submission_is_atomic_and_not_a_workspace_mutation() {
        let media = binding("creation.media", "creation.media/image");
        assert!(!requires_task_ledger(&media));
        assert!(!affects_workspace(&media));
    }

    #[test]
    fn system_execution_policy_requires_instruction_scope_before_source_reads() {
        assert!(crate::workflow::MINIMAL_EXECUTION_INSTRUCTIONS.contains(
            "AGENTS.md and AGENTS.override.md bodies are already supplied in system context"
        ));
        assert!(crate::workflow::MINIMAL_EXECUTION_INSTRUCTIONS.contains(
            "never read those files with the default text format"
        ));
        assert!(crate::workflow::MINIMAL_EXECUTION_INSTRUCTIONS.contains(
            "call read_file alone on the directory with format=instruction_scope"
        ));
        assert!(crate::workflow::MINIMAL_EXECUTION_INSTRUCTIONS.contains("use fresh call IDs"));
    }

    #[test]
    fn workspace_keeps_long_horizon_accounting_but_delegation_owns_its_plan() {
        let workspace = binding("workspace.files", "workspace.files/write");
        assert!(requires_task_ledger(&workspace));
        assert!(affects_workspace(&workspace));

        let read = binding_with_effect(
            "workspace.files",
            "workspace.files/read",
            EngineEffectClass::ReadOnly,
        );
        assert!(requires_task_ledger(&read));
        assert!(!affects_workspace(&read));

        let process_poll = binding_with_effect(
            "workspace.process",
            "workspace.process/poll",
            EngineEffectClass::ReadOnly,
        );
        assert!(affects_workspace(&process_poll));

        let delegation = binding("agent.collaboration", "agent/delegate");
        // AgentExecution is already the durable plan, scheduler, completion
        // ledger, and terminal-report owner. Requiring a second parent-turn
        // plan blocks the first delegation call and later competes with the
        // authoritative synthesis report.
        assert!(!requires_task_ledger(&delegation));
        assert!(!affects_workspace(&delegation));
        assert!(completes_turn_on_success(&delegation));

        let fork = binding("agent.collaboration", "agent/fork");
        assert!(completes_turn_on_success(&fork));
        assert!(!completes_turn_on_success(&workspace));
    }

    #[test]
    fn recoverable_reads_and_unexecuted_proposals_do_not_reopen_the_task_plan() {
        let read = binding_with_effect("workspace.files", "workspace.files/read", EngineEffectClass::ReadOnly);
        let write = binding("workspace.files", "workspace.files/write");
        let failed = crate::AgentToolResult::text("call".into(), "failed", true);
        assert!(!requires_replanning_after_result(&read, &failed, true, 1));
        assert!(!requires_replanning_after_result(&write, &failed, false, 1));
        assert!(requires_replanning_after_result(&write, &failed, true, 1));
    }

    #[test]
    fn nonzero_process_exit_is_a_failed_observation_even_when_dispatch_succeeded() {
        let process = binding("workspace.process", "workspace.process/exec");
        let failed = crate::AgentToolResult::text(
            "call-1".into(),
            serde_json::json!({"state":"exited","exit_code":1,"success":false}).to_string(),
            false,
        );
        let successful = crate::AgentToolResult::text(
            "call-2".into(),
            serde_json::json!({"state":"exited","exit_code":0,"success":true}).to_string(),
            false,
        );
        let invalid_arguments = crate::AgentToolResult::text(
            "call-3".into(), "Invalid arguments; no process launched", true,
        );
        assert!(failed_process_observation(&process, &failed));
        assert!(!failed_process_observation(&process, &successful));
        assert!(!failed_process_observation(&process, &invalid_arguments));
        assert!(!failed_process_observation(&binding("workspace.files", "workspace.files/read"), &failed));
    }

    #[test]
    fn reaped_cancel_and_terminal_poll_do_not_force_replanning() {
        let cancelled = crate::AgentToolResult::text(
            "call".into(),
            serde_json::json!({
                "state":"cancelled",
                "success":false,
                "cleanup":{"reaped":true,"errors":[]}
            })
            .to_string(),
            false,
        );
        for action in ["workspace.process/cancel", "workspace.process/poll"] {
            let process = binding_with_effect(
                "workspace.process",
                action,
                EngineEffectClass::ReadOnly,
            );
            assert!(!failed_process_observation(&process, &cancelled));
            assert!(!requires_replanning_after_result(
                &process,
                &cancelled,
                true,
                0,
            ));
        }

        let unreaped = crate::AgentToolResult::text(
            "call".into(),
            serde_json::json!({
                "state":"cancelled",
                "success":false,
                "cleanup":{"reaped":false,"errors":["still running"]}
            })
            .to_string(),
            false,
        );
        let poll = binding_with_effect(
            "workspace.process",
            "workspace.process/poll",
            EngineEffectClass::ReadOnly,
        );
        assert!(failed_process_observation(&poll, &unreaped));
    }

    #[test]
    fn proven_nonstart_preserves_workspace_evidence_but_cleanup_does_not_prove_nonstart() {
        let process = binding("workspace.process", "workspace.process/exec");
        let mut work = crate::AgentWorkStatus { workspace_observation_epoch:7,
            successful_commands:1, command_observed_after_latest_mutation:true, ..Default::default() };
        let mut commands = crate::workflow::CommandTracker::default();
        let call = nomifun_chat_model_broker::ChatToolCall { call_id:"failed-spawn".into(), name:"exec_command".into(),
            arguments:StrictJsonValue(serde_json::json!({"command":"node --check game.js"})),provider_metadata:None };
        let result = crate::AgentToolResult::text(call.call_id.clone(),serde_json::json!({
            "schema":"nomifun.process-start-observation.v1","state":"not_started","user_code_started":false,"success":false
        }).to_string(),true);
        assert!(process_did_not_start(&process,&result));
        assert!(!requires_replanning_after_result(&process,&result,true,0));
        work.observe(&process,&call,&result,&mut commands);
        assert_eq!(work.workspace_observation_epoch,7);
        assert!(work.command_observed_after_latest_mutation);
        let uncertain = crate::AgentToolResult::text(call.call_id.clone(),
            serde_json::json!({"state":"lost","success":false,"cleanup":{"reaped":true}}).to_string(),true);
        assert!(!process_did_not_start(&process,&uncertain));
        work.observe(&process,&call,&uncertain,&mut commands);
        assert_eq!(work.workspace_observation_epoch,8);
        assert!(!work.command_observed_after_latest_mutation);
    }

    #[test]
    fn explicit_reaped_cancel_is_not_a_failed_command_observation() {
        let start = binding("workspace.process", "workspace.process/start");
        let cancel = binding("workspace.process", "workspace.process/cancel");
        let mut work = crate::AgentWorkStatus::default();
        let mut commands = crate::workflow::CommandTracker::default();
        let start_call = nomifun_chat_model_broker::ChatToolCall {
            call_id: "start-1".into(),
            name: "start_process".into(),
            arguments: StrictJsonValue(serde_json::json!({"command":"worker"})),
            provider_metadata: None,
        };
        work.observe(
            &start,
            &start_call,
            &crate::AgentToolResult::text(
                start_call.call_id.clone(),
                serde_json::json!({
                    "state":"running", "process_id":"process-1", "success":null
                })
                .to_string(),
                false,
            ),
            &mut commands,
        );
        assert!(work.running_processes.contains("process-1"));

        let poll = binding_with_effect(
            "workspace.process",
            "workspace.process/poll",
            EngineEffectClass::ReadOnly,
        );
        let poll_call = nomifun_chat_model_broker::ChatToolCall {
            call_id: "poll-1".into(),
            name: "poll_process".into(),
            arguments: StrictJsonValue(serde_json::json!({"process_id":"process-1"})),
            provider_metadata: None,
        };
        work.observe(
            &poll,
            &poll_call,
            &crate::AgentToolResult::text(
                poll_call.call_id.clone(),
                serde_json::json!({
                    "state":"running", "process_id":"process-1", "success":null
                })
                .to_string(),
                false,
            ),
            &mut commands,
        );
        assert_eq!(work.workspace_observation_epoch, 1);

        let cancel_call = nomifun_chat_model_broker::ChatToolCall {
            call_id: "cancel-1".into(),
            name: "cancel_process".into(),
            arguments: StrictJsonValue(serde_json::json!({"process_id":"process-1"})),
            provider_metadata: None,
        };
        work.observe(
            &cancel,
            &cancel_call,
            &crate::AgentToolResult::text(
                cancel_call.call_id.clone(),
                serde_json::json!({
                    "state":"cancelled", "process_id":"process-1", "success":true,
                    "cleanup":{"reaped":true,"errors":["interrupt unavailable"]}
                })
                .to_string(),
                false,
            ),
            &mut commands,
        );

        assert_eq!(work.successful_commands, 0);
        assert_eq!(work.failed_commands, 0);
        assert!(work.running_processes.is_empty());
        assert!(!work.command_observed_after_latest_mutation);
        assert_eq!(work.recent_commands.len(), 1);
        assert_eq!(work.recent_commands[0].state, "cancelled");
        assert!(work.recent_commands[0].cleanup_proven);
        assert_eq!(work.recent_commands[0].interaction_call_ids, ["poll-1"]);
        assert!(!work.recent_commands[0].was_current_at_observation);
    }

    #[test]
    fn rejected_process_control_keeps_the_original_handle_and_workspace_evidence() {
        let input = binding("workspace.process", "workspace.process/input");
        let mut work = crate::AgentWorkStatus { workspace_observation_epoch:7,
            successful_commands:1, command_observed_after_latest_mutation:true, ..Default::default() };
        work.running_processes.insert("original-process".into());
        let mut commands = crate::workflow::CommandTracker::default();
        let call = nomifun_chat_model_broker::ChatToolCall { call_id:"rejected-control".into(),
            name:"write_process_stdin".into(), arguments:StrictJsonValue(serde_json::json!({
                "process_id":"wrong-reference", "input":"do not send"
            })), provider_metadata:None };
        let receipt = serde_json::json!({"schema":"nomifun.process-control-observation.v1",
            "state":"not_executed", "code":"PROCESS_REFERENCE_INVALID", "operation":"stdin",
            "control_applied":false, "success":false});
        let result = crate::AgentToolResult::text(call.call_id.clone(), receipt.to_string(), true);
        assert!(process_operation_not_applied(&input, &result));
        assert!(!failed_process_observation(&input, &result));
        assert!(!requires_replanning_after_result(&input, &result, true, 0));
        work.observe(&input, &call, &result, &mut commands);
        assert_eq!(work.failed_tools, 1, "the rejected call stays in failure history");
        assert_eq!(work.failed_commands, 0);
        assert_eq!(work.workspace_observation_epoch, 7);
        assert!(work.command_observed_after_latest_mutation);
        assert!(work.running_processes.contains("original-process"));
        let mut uncertain = receipt;
        uncertain["control_applied"] = serde_json::json!(true);
        let result = crate::AgentToolResult::text(call.call_id.clone(), uncertain.to_string(), true);
        assert!(!process_operation_not_applied(&input, &result));
        assert!(!process_operation_not_applied(&binding("workspace.process", "workspace.process/exec"), &result));
        work.observe(&input, &call, &result, &mut commands);
        assert_eq!(work.workspace_observation_epoch, 8);
    }
}
