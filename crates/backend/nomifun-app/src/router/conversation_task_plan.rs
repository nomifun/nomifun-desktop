//! Public task progress from the canonical runtime journal.
use super::*;
use nomifun_api_types::{TaskPlan, TaskPlanSnapshot, TaskPlanStep, TaskPlanStepStatus};

fn project(
    session_id: &AgentSessionId,
    state: nomifun_agent_session::RuntimeStateObservation,
) -> Result<TaskPlanSnapshot, NomiCoreApiError> {
    let plan = state.event.map(|event| {
        let AgentEngineEvent::PlanUpdated { plan } = serde_json::from_value(event)
            .map_err(|error| AppError::Conflict(format!("Invalid recorded task plan: {error}")))?
        else {
            return Err(AppError::Conflict("Recorded task state is not a plan update".into()));
        };
        Ok(TaskPlan {
            revision: plan.revision,
            explanation: plan.explanation,
            needs_replan: plan.needs_replan,
            steps: plan.steps.into_iter().map(|item| TaskPlanStep {
                step: item.step,
                status: match item.status {
                    AgentPlanStatus::Pending => TaskPlanStepStatus::Pending,
                    AgentPlanStatus::InProgress => TaskPlanStepStatus::InProgress,
                    AgentPlanStatus::Completed => TaskPlanStepStatus::Completed,
                    AgentPlanStatus::Blocked => TaskPlanStepStatus::Blocked,
                },
            }).collect(),
        })
    }).transpose()?;
    Ok(TaskPlanSnapshot {
        conversation_id: session_id.as_ref().to_owned(),
        sequence: state.sequence,
        turn_id: state.turn_id,
        turn_status: state.turn_status,
        plan,
    })
}

pub(super) async fn get(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<TaskPlanSnapshot>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let snapshot = state.session_owner.canonical()
        .latest_runtime_state(&authenticated_principal(&owner), &session_id, "plan_updated").await?;
    Ok(Json(ApiResponse::ok(project(&session_id, snapshot)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_plan_notification_preserves_session_and_turn_identity() {
        let session = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let root = "0190f5fe-7c00-7a00-8000-000000000002";
        let assistant = NomiCoreSessionOwner::canonical_assistant_stream_message_id(root).unwrap();
        let wire = NomiCoreSessionOwner::canonical_stream_wire_event(
            &session, root, &assistant, &AgentStreamEvent::TaskPlanChanged,
        ).unwrap();
        let json = serde_json::to_value(wire).unwrap();
        assert_eq!(json["name"], "message.stream");
        assert_eq!(json["data"]["type"], "task_plan_changed");
        assert_eq!(json["data"]["turn_id"], root);
        assert_eq!(json["data"]["conversation_id"], session.as_ref());
        assert_eq!(json["data"]["data"], json!({}));
    }

    #[test]
    fn task_plan_codec_exposes_steps_and_lifecycle_without_private_ledgers() {
        let mut plan = AgentPlan::default();
        plan.revision = 3;
        plan.needs_replan = true;
        plan.steps = vec![nomifun_agent_runtime::AgentPlanStep {
            step: "Wait for access".into(), status: AgentPlanStatus::Blocked,
        }];
        let output = project(&"0190f5fe-7c00-7a00-8000-000000000001".into(),
            nomifun_agent_session::RuntimeStateObservation {
                sequence: 200, turn_id: Some("turn".into()), turn_status: Some("paused".into()),
                event: Some(serde_json::to_value(AgentEngineEvent::PlanUpdated { plan }).unwrap()),
            }).unwrap();
        let json = serde_json::to_value(output).unwrap();
        assert_eq!(json["plan"]["steps"][0]["status"], "blocked");
        assert_eq!(json["turn_status"], "paused");
        assert!(json["plan"].get("requirements").is_none());
        assert!(json["plan"].get("exact_actions").is_none());
    }
}
