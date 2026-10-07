//! Composer selections use the canonical binding transition and its idle fence.
use super::*;
use nomifun_api_types::{AgentSessionCapabilitySelectionDto, SessionCapabilitySelectionDto, UpdateAgentSessionCapabilitySelectionDto};

pub(super) fn selection(snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope) -> SessionCapabilitySelectionDto {
    let mut skill_names = BTreeSet::new();
    for lock in &snapshot.content.skill_locks {
        match lock {
            nomifun_agent_contracts::ResolvedSkillLock::Package(lock) => { skill_names.insert(lock.skill.id.as_ref().to_owned()); }
            nomifun_agent_contracts::ResolvedSkillLock::Library { skill, selected, .. } if *selected => { skill_names.insert(skill.name.clone()); }
            _ => {}
        }
    }
    SessionCapabilitySelectionDto {
        skill_names: skill_names.into_iter().collect(),
        mcp_server_ids: snapshot.content.mcp_tool_locks.iter().map(|lock| lock.server_id.as_ref().to_owned()).collect::<BTreeSet<_>>().into_iter().collect(),
    }
}

async fn editable(state: &NomiCoreAgentApiState, observation: &nomifun_agent_session::SessionObservation, session_id: &AgentSessionId) -> Result<bool, NomiCoreApiError> {
    if observation.session.remote_binding_provenance.is_some() || observation.head.active_turn_id.is_some()
        || matches!(observation.head.status.as_str(), "running" | "paused" | "reconciliation") {
        return Ok(false);
    }
    let attempt: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id = ? AND relation IN ('attempt', 'automation'))")
        .bind(session_id.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(attempt == 0)
}

pub(super) async fn get_selection(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>, Path(id): Path<String>,
) -> Result<Json<ApiResponse<AgentSessionCapabilitySelectionDto>>, NomiCoreApiError> {
    let id = parse_agent_session_id(&id)?;
    let observation = state.session_owner.canonical().get(&authenticated_principal(&owner), &id).await?;
    let binding = agent_binding_dto(&observation.session.agent_binding)?;
    let (_, _, snapshot) = state.control_plane.saved_binding_artifacts(&owner.0, &binding).await?;
    Ok(Json(ApiResponse::ok(AgentSessionCapabilitySelectionDto {
        selection: selection(&snapshot), binding_version: binding.binding_version, editable: editable(&state, &observation, &id).await?,
    })))
}

pub(super) async fn update_selection(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>, Path(id): Path<String>,
    Json(request): Json<UpdateAgentSessionCapabilitySelectionDto>,
) -> Result<Json<ApiResponse<AgentSessionCapabilitySelectionDto>>, NomiCoreApiError> {
    let id = parse_agent_session_id(&id)?;
    let _operation_fence = state.session_owner.session_operation_lock(id.as_ref()).write_owned().await;
    let principal = authenticated_principal(&owner);
    let observation = state.session_owner.canonical().get(&principal, &id).await?;
    if !editable(&state, &observation, &id).await? {
        return Err(NomiCoreApiError::new(StatusCode::CONFLICT, "AGENT_SESSION_CAPABILITIES_READ_ONLY", "wait for the current task to finish; remote Sessions and execution Attempts have fixed capabilities"));
    }
    if request.expected_binding_version != observation.session.agent_binding.binding_version {
        return Err(NomiCoreApiError::new(StatusCode::CONFLICT, "AGENT_SESSION_BINDING_CHANGED", "the Session configuration changed; refresh its selection before applying"));
    }
    let current = agent_binding_dto(&observation.session.agent_binding)?;
    let mut replacement = state.control_plane.resolve_agent_session_capabilities_binding(&owner.0, &current, Some(&request.selection)).await?;
    if replacement.resolved_snapshot_ref == current.resolved_snapshot_ref && replacement.preset_revision_ref == current.preset_revision_ref {
        let (_, _, snapshot) = state.control_plane.saved_binding_artifacts(&owner.0, &current).await?;
        return Ok(Json(ApiResponse::ok(AgentSessionCapabilitySelectionDto { selection: selection(&snapshot), binding_version: current.binding_version, editable: true })));
    }
    let mcp = state.resource_bindings.resolve_mcp_for_saved_binding(&state.control_plane, &owner.0, &replacement).await?;
    replacement.typed_resource_bindings = current.typed_resource_bindings.iter().filter(|resource| resource.resource_kind != "mcp_server").cloned().chain(mcp).collect();
    replacement.typed_resource_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    replacement.binding_version = current.binding_version.checked_add(1).ok_or_else(|| AppError::Conflict("Session binding version overflow".into()))?;
    let (_, _, snapshot) = state.control_plane.saved_binding_artifacts(&owner.0, &replacement).await?;
    state.product_agent_resolver.official_runtime.validate_agent(&snapshot)?;
    let previous_active = state.session_owner.canonical().active_capability_ids(&principal, &id).await?.into_iter().collect::<BTreeSet<_>>();
    let active = snapshot.content.contributions().filter(|capability| previous_active.contains(capability.capability.id.as_ref())
        || nomifun_agent_control_plane::is_global_extension_module(capability.capability.id.as_ref()))
        .map(|capability| capability.capability.id.as_ref().to_owned()).collect();
    let label = state.control_plane.editor(&owner.0, &current.preset_revision_ref.preset_id, Some(current.preset_revision_ref.revision)).await?.preset.display_name;
    state.session_owner.runtime_sessions.terminate_and_wait_result(id.as_ref(), Some(AgentKillReason::ConfigurationChanged)).await?;
    super::super::hosted_effect_receipts::HostedEffectReceipts::new(state.session_owner.pool.clone()).ensure_settled(owner.as_ref(), id.as_ref()).await?;
    let transition = OperationId::from(Uuid::now_v7().to_string());
    let changed = state.session_owner.canonical().store().replace_session_agent_binding(&principal, &id,
        nomifun_agent_session::ReplaceSessionAgentBinding {
            expected: observation.session.agent_binding, replacement: serde_json::from_value(serde_json::to_value(&replacement)?)?,
            previous_agent_label: label.clone(), next_agent_label: label, transition_id: transition.clone(),
            request_digest: digest_payload(&request).map_err(|error| AppError::Internal(error.to_string()))?,
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(transition.as_ref().to_owned()),
            handoff_mode: AgentHandoffMode::ContextOnly, handoff: None, initial_active_capability_ids: active,
        }).await.map_err(agent_switch_store_error)?;
    let response = AgentSessionCapabilitySelectionDto { selection: selection(&snapshot), binding_version: changed.session.agent_binding.binding_version, editable: true };
    state.session_owner.user_events.send_to_user(owner.as_ref(), WebSocketMessage::new("agentSession.capabilitiesChanged", json!({
        "agent_session_id": id, "selection": response.selection, "binding_version": response.binding_version, "editable": true,
    })));
    Ok(Json(ApiResponse::ok(response)))
}
