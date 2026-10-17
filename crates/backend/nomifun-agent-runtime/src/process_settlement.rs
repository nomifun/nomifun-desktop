//! A soft window can wait for its already-owned processes to settle. This
//! turn-local view grants no work, survives no restart and is never checkpointed.
use std::collections::{BTreeMap, BTreeSet};

use nomifun_chat_model_broker::{ChatToolCall, ChatToolChoice, ChatToolDefinition};

use crate::{AgentEffectClass, AgentExecutionSegmentState, AgentExecutionStopReason, AgentToolBinding, AgentToolPlan, AgentToolResult};

#[derive(Default)]
pub(super) struct ProcessSettlement {
    observations: BTreeMap<String, Observation>,
    boundary: Option<Boundary>,
}

#[derive(Default)]
struct Observation {
    cursor: u64,
    input_digest: Option<nomifun_agent_contracts::DigestHex>,
    stdin_closed: bool,
    reaped: bool,
}

struct Boundary {
    processes: BTreeSet<String>,
    window_start_step: u16,
    no_progress_windows: u16,
    made_progress: bool,
}

fn process_control(binding: &AgentToolBinding) -> bool {
    binding.capability_id.as_ref() == "workspace.process"
        && matches!(binding.action_id.as_ref(), "workspace.process/input" | "workspace.process/close_stdin"
            | "workspace.process/poll" | "workspace.process/cancel")
}

fn input_read(binding: &AgentToolBinding) -> bool {
    binding.capability_id.as_ref() == "workspace.files"
        && binding.action_id.as_ref() == "workspace.files/read"
        && binding.effect_class == AgentEffectClass::ReadOnly
}

impl ProcessSettlement {
    pub(super) fn wait_request(&self, running: &BTreeSet<String>) -> crate::AgentWaitRequest {
        crate::AgentWaitRequest { processes: running.iter().map(|id| crate::AgentWaitProcess {
            process_id: id.clone(), output_cursor: self.observations.get(id).map_or(0, |observation| observation.cursor),
        }).collect() }
    }

    pub(super) fn observe_owner(&mut self, observation: &crate::AgentOwnedProcessObservation) {
        if let Some(owned) = self.observations.get_mut(&observation.process_id) {
            if let Some(cursor) = observation.output_cursor() { owned.cursor = owned.cursor.max(cursor); }
            owned.reaped |= observation.reaped();
        }
    }

    pub(super) fn is_active(&self) -> bool { self.boundary.is_some() }

    pub(super) fn clear_boundary(&mut self) {
        self.boundary = None;
        self.observations.retain(|_, observation| !observation.reaped);
    }

    pub(super) fn at_boundary(&mut self, step: u16, segments: &AgentExecutionSegmentState,
        running: &BTreeSet<String>, plan: &AgentToolPlan) -> Option<AgentExecutionStopReason> {
        if !plan.model_definitions().iter().any(|tool| plan.binding(&tool.name).is_some_and(process_control)) {
            // A launch grant is not a control grant. Without an original
            // settlement action, keep the existing safe pause instead of
            // inventing authority or spending requests on an empty surface.
            return Some(AgentExecutionStopReason::NonQuiescentBoundary);
        }
        let boundary = self.boundary.get_or_insert_with(|| Boundary {
            processes: running.clone(), window_start_step: step,
            no_progress_windows: if segments.progress_fingerprints.len() > segments.progress_at_segment_start {
                0
            } else { segments.no_progress_segments },
            made_progress: false,
        });
        // A returned foreign handle cannot enlarge the boundary's authority.
        if !running.is_subset(&boundary.processes) {
            return Some(AgentExecutionStopReason::NonQuiescentBoundary);
        }
        if step.saturating_sub(boundary.window_start_step) >= segments.model_steps_per_segment {
            boundary.no_progress_windows = if boundary.made_progress { 0 }
                else { boundary.no_progress_windows.saturating_add(1) };
            boundary.window_start_step = step;
            boundary.made_progress = false;
            if boundary.no_progress_windows >= segments.policy.max_no_progress_segments {
                return Some(AgentExecutionStopReason::NoProgress);
            }
        }
        None
    }

    pub(super) fn context(&self) -> Option<String> {
        let boundary = self.boundary.as_ref()?;
        Some(format!("The execution window is awaiting a quiescent checkpoint for these original owned processes: {}. Continue only their authorized stdin, EOF, poll or cancellation, or the authorized read_file needed for their input. request_user_input may park this same turn for a required user action while the original owner remains live. update_plan may account for accepted steering. Do not launch a process, start unrelated work, declare completion or repeat settled effects. No budget or process deadline is renewed; the original cumulative limits and cancellation still apply.",
            serde_json::to_string(&boundary.processes).expect("process IDs serialize")))
    }

    pub(super) fn activate_controls(&self, plan: &AgentToolPlan, discovered: &mut BTreeSet<String>) -> Result<(), crate::AgentEngineError> {
        if self.is_active() {
            let names = plan.model_definitions().into_iter().filter(|tool| tool.deferred
                && plan.binding(&tool.name).is_some_and(|binding| process_control(binding) || input_read(binding)))
                .map(|tool| tool.name).collect::<BTreeSet<_>>();
            if !names.is_empty() {
                // Use the existing bounded presentation activation, not a
                // second grant set or an unbounded catalog append.
                crate::tool_discovery::preactivate_deferred_tools(plan, discovered, &names)?;
            }
        }
        Ok(())
    }

    pub(super) fn constrain_tools(&self, tools: &mut Vec<ChatToolDefinition>, choice: &mut ChatToolChoice, plan: &AgentToolPlan) {
        if self.is_active() {
            tools.retain(|tool| tool.name == crate::planning::TOOL_NAME || tool.name == crate::user_input_request::TOOL_NAME
                || plan.binding(&tool.name).is_some_and(|binding| process_control(binding) || input_read(binding)));
            if matches!(choice, ChatToolChoice::Specific { name } if !tools.iter().any(|tool| tool.name == *name)) {
                // This is called only for Execution, never a locked report
                // phase. Keep the narrowed request valid rather than forcing
                // a forbidden new action absent from its advertised schemas.
                *choice = if tools.is_empty() { ChatToolChoice::None } else { ChatToolChoice::Auto };
            }
        }
    }

    pub(super) fn allows_batch(&self, calls: &[ChatToolCall], plan: &AgentToolPlan) -> bool {
        let Some(boundary) = &self.boundary else { return true; };
        calls.iter().all(|call| call.name == crate::planning::TOOL_NAME || call.name == crate::user_input_request::TOOL_NAME || plan.binding(&call.name).is_some_and(|binding|
            input_read(binding) || (process_control(binding) && call.arguments.0.get("process_id")
                .and_then(serde_json::Value::as_str).is_some_and(|id| boundary.processes.contains(id)))))
    }

    /// Observe only an actually attempted original owner result. Wait duration,
    /// call IDs, arguments' cursor and instruction reads are never progress.
    pub(super) fn observe(&mut self, binding: &AgentToolBinding, call: &ChatToolCall, result: &AgentToolResult) {
        if binding.capability_id.as_ref() != "workspace.process" { return; }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&result.output_text()) else { return; };
        let Some(id) = value.get("process_id").and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128) else { return; };
        let launch = matches!(binding.action_id.as_ref(), "workspace.process/start" | "workspace.process/exec");
        if !launch && call.arguments.0.get("process_id").and_then(serde_json::Value::as_str) != Some(id) { return; }
        if !self.observations.contains_key(id) {
            // Same bound as the existing live-process observation/owner scope.
            if !launch || self.observations.len() >= 64 { return; }
            self.observations.insert(id.to_owned(), Observation::default());
        }
        let observation = self.observations.get_mut(id).expect("registered process");
        let terminal = matches!(value.get("state").and_then(serde_json::Value::as_str),
            Some("exited" | "cancelled" | "timed_out" | "lost"));
        let reaped = terminal && value.pointer("/cleanup/reaped").and_then(serde_json::Value::as_bool) == Some(true);
        let mut progress = reaped && !observation.reaped;
        observation.reaped |= reaped;
        if !result.is_error {
            if let Some(cursor) = value.pointer("/output/next_cursor").and_then(serde_json::Value::as_u64) {
                progress |= cursor > observation.cursor && value.pointer("/output/text")
                    .and_then(serde_json::Value::as_str).is_some_and(|text| !text.is_empty());
                observation.cursor = observation.cursor.max(cursor);
            }
            match binding.action_id.as_ref() {
                "workspace.process/input" => {
                    if let Some(input) = call.arguments.0.get("input").and_then(serde_json::Value::as_str) {
                        let mut bytes = input.as_bytes().to_vec();
                        if call.arguments.0.get("append_newline").and_then(serde_json::Value::as_bool) == Some(true) { bytes.push(b'\n'); }
                        if !bytes.is_empty() {
                            let digest = nomifun_agent_contracts::digest_bytes(&bytes);
                            progress |= observation.input_digest.as_ref() != Some(&digest);
                            observation.input_digest = Some(digest);
                        }
                    }
                }
                "workspace.process/close_stdin" => {
                    progress |= !observation.stdin_closed;
                    observation.stdin_closed = true;
                }
                _ => {}
            }
        }
        if let Some(boundary) = self.boundary.as_mut() {
            if boundary.processes.contains(id) { boundary.made_progress |= progress; }
        } else if reaped {
            self.observations.remove(id);
        }
    }
}
