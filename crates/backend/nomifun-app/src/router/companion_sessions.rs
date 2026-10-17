//! Canonical Companion role affiliation and host-only product entry points.

use super::*;
use nomifun_agent_contracts::{CompanionSessionOrigin, CompanionSessionRole};

/// Only the canonical opening grants product affiliation. Current-generation
/// single-Session openings remain readable through their exact host command.
pub(crate) fn canonical_companion_origin(
    owner_id: &str,
    observation: &SessionObservation,
) -> Result<Option<CompanionSessionOrigin>, AppError> {
    if let Some(scope) = declared_companion_origin(owner_id, &observation.session)? { return Ok(Some(scope)); }
    let Some(companion_id) = companion_id_from_binding(&observation.session.agent_binding)? else { return Ok(None); };
    let key = format!("user:{owner_id}:open:companion-session:{companion_id}");
    if observation.events.iter().any(|event| event.kind.0 == "session/opening"
        && event.seq == 1 && event.kind_version == 1 && event.producer_id.as_ref() == "session_api"
        && event.idempotency_key.as_ref() == key
        && event.event_id.as_ref() == format!("opening:{key}")
        && event.correlation_id.as_ref() == format!("open:{key}")
        && matches!(&event.payload, nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload)
            if payload.0.get("operation_id").and_then(Value::as_str) == Some(format!("open:{key}").as_str()))) {
        return Ok(Some(CompanionSessionOrigin { companion_id, role: CompanionSessionRole::Companion, robot_id: None }));
    }
    Ok(None)
}

fn declared_companion_origin(owner_id: &str, session: &nomifun_agent_contracts::AgentSessionLiveRecord)
    -> Result<Option<CompanionSessionOrigin>, AppError> {
    if session.owner_ref.principal_kind != "user"
        || session.owner_ref.principal_id != owner_id {
        return Err(AppError::Forbidden("Companion Session belongs to another owner".into()));
    }
    let companion_id = companion_id_from_binding(&session.agent_binding)?;
    if session.agent_binding.typed_resource_bindings.iter().any(|resource|
        matches!(resource.resource_kind.as_ref(), "companion" | "companion_memory") && resource.owner_id != owner_id) {
        return Err(AppError::Forbidden("Companion persona resources belong to another owner".into()));
    }
    if let Some(origin) = session.metadata.companion.as_ref() {
        if session.metadata.purpose != nomifun_agent_contracts::SessionPurpose::Conversation
            || session.metadata.app_project_id.is_some() || session.remote_binding_provenance.is_some() {
            return Err(AppError::Conflict("Companion affiliation requires its local current Conversation purpose".into()));
        }
        if companion_id.as_deref() != Some(origin.companion_id.as_str()) {
            return Err(AppError::Conflict("Companion opening and frozen persona binding differ".into()));
        }
        let robots = session.agent_binding.typed_resource_bindings.iter()
            .filter(|resource| resource.resource_kind.as_ref() == "robot").collect::<Vec<_>>();
        if robots.iter().any(|resource| resource.owner_id.as_str() != owner_id) {
            return Err(AppError::Forbidden("Companion Robot resources belong to another owner".into()));
        }
        if origin.role == CompanionSessionRole::Robot && (origin.robot_id.as_deref().is_none_or(|id| id.trim().is_empty())
            || !robots.is_empty() && !matches!(robots.as_slice(), [resource]
                if Some(resource.resource_id.as_ref()) == origin.robot_id.as_deref()
                    && resource.owner_id.as_str() == owner_id)) {
            return Err(AppError::Conflict("Robot Session opening requires its exact frozen device".into()));
        }
        return Ok(Some(origin.clone()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::*;

    fn observation(role: Option<CompanionSessionRole>) -> SessionObservation {
        let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8abc-012345678901");
        let companion_id = "0190f5fe-7c00-7a00-8abc-012345678902";
        let mut resources = vec![TypedResourceBinding {
            binding_id: "companion-binding".into(), resource_kind: "companion".into(),
            resource_id: companion_id.into(), owner_id: "owner".into(), operations: BTreeSet::from(["read".into()]),
            connection_config_ref: None, typed_parameters: BTreeMap::new(),
        }];
        if role == Some(CompanionSessionRole::Robot) {
            resources.push(TypedResourceBinding { binding_id: "robot-binding".into(), resource_kind: "robot".into(),
                resource_id: "robot-a".into(), owner_id: "owner".into(), operations: BTreeSet::from(["motion".into()]),
                connection_config_ref: None, typed_parameters: BTreeMap::new() });
        }
        let session = AgentSessionLiveRecord { agent_session_id: session_id.clone(),
            owner_ref: PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
            metadata: AgentSessionMetadata { purpose: SessionPurpose::Conversation, app_project_id: None,
                companion: role.map(|role| CompanionSessionOrigin { companion_id: companion_id.into(), role,
                    robot_id: (role == CompanionSessionRole::Robot).then(|| "robot-a".into()) }),
                title: None, archived: false, pinned: false, reasoning_effort: None },
            agent_binding: AgentBindingValue { preset_revision_ref: PresetRevisionRef { preset_id: companion_id.into(),
                revision: 1, revision_digest: "a".repeat(64).into() },
                resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() },
                typed_resource_bindings: resources, binding_version: 1 },
            remote_binding_provenance: None, parent_session_id: None, fork_base_payload_id: None, next_seq: 1 };
        SessionObservation { session, head: nomifun_agent_session::SessionHeadProjection { session_id: session_id.clone(),
            status: "ready".into(), active_turn_id: None, active_set_generation: 0, last_seq: 0, unread_count: 0 },
            events: vec![], messages: vec![], next_cursor: SessionEventCursor { agent_session_id: session_id, seq: 0 }, active_turn_outcome:None }
    }

    #[test]
    fn typed_robot_affiliation_requires_the_exact_owner_persona_and_device() {
        let mut facts = observation(Some(CompanionSessionRole::Robot));
        assert_eq!(canonical_companion_origin("owner", &facts).unwrap().unwrap().robot_id.as_deref(), Some("robot-a"));
        assert!(canonical_companion_origin("other-owner", &facts).is_err());
        facts.session.metadata.companion.as_mut().unwrap().robot_id = Some("robot-b".into());
        assert!(canonical_companion_origin("owner", &facts).is_err());
        facts.session.metadata.companion.as_mut().unwrap().role = CompanionSessionRole::Companion;
        facts.session.metadata.companion.as_mut().unwrap().robot_id = None;
        assert_eq!(canonical_companion_origin("owner", &facts).unwrap().unwrap().role, CompanionSessionRole::Companion,
            "explicit Robot resource access does not change a desktop Session's canonical receive role");
        facts.session.agent_binding.typed_resource_bindings.iter_mut().find(|resource| resource.resource_kind.as_ref() == "robot")
            .unwrap().owner_id = "other-owner".into();
        assert!(matches!(canonical_companion_origin("owner", &facts), Err(AppError::Forbidden(_))),
            "desktop access to an explicitly selected Robot still requires its owner");
    }

    #[test]
    fn legacy_affiliation_requires_the_exact_current_host_opening_command() {
        let mut facts = observation(None);
        assert!(canonical_companion_origin("owner", &facts).unwrap().is_none());
        let companion_id = facts.session.agent_binding.typed_resource_bindings[0].resource_id.as_ref();
        let key = format!("user:owner:open:companion-session:{companion_id}");
        facts.events.push(SessionEventRecord { agent_session_id: facts.session.agent_session_id.clone(), seq: 1,
            event_id: format!("opening:{key}").into(), producer_id: "session_api".into(), idempotency_key: key.clone().into(),
            kind: SessionEventKind("session/opening".into()), kind_version: 1, correlation_id: format!("open:{key}").into(),
            causation_event_id: None, payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({ "operation_id": format!("open:{key}") }))) });
        assert_eq!(canonical_companion_origin("owner", &facts).unwrap().unwrap().role, CompanionSessionRole::Companion);
        facts.events[0].event_id = "unrelated-opening".into();
        assert!(canonical_companion_origin("owner", &facts).unwrap().is_none());
    }

    #[test]
    fn role_resource_updates_cannot_retarget_the_physical_device() {
        let facts = observation(Some(CompanionSessionRole::Robot));
        let scope = facts.session.metadata.companion.unwrap();
        let mut binding: AgentBindingValueDto = serde_json::from_value(serde_json::to_value(facts.session.agent_binding).unwrap()).unwrap();
        validate_role_resources(&scope, &binding).unwrap();
        binding.typed_resource_bindings.iter_mut().find(|resource| resource.resource_kind == "robot").unwrap().resource_id = "robot-b".into();
        assert!(validate_role_resources(&scope, &binding).is_err());
    }
}

pub(super) fn role_target_kind(role: CompanionSessionRole) -> &'static str {
    match role { CompanionSessionRole::Companion => "companion", CompanionSessionRole::Robot => "robot" }
}

pub(super) fn validate_role_resources(scope: &CompanionSessionOrigin, binding: &AgentBindingValueDto) -> Result<(), AppError> {
    let robots = binding.typed_resource_bindings.iter().filter(|resource| resource.resource_kind == "robot").collect::<Vec<_>>();
    match scope.role {
        CompanionSessionRole::Robot if scope.robot_id.as_deref().is_none_or(|id| id.trim().is_empty())
            || !robots.is_empty() && !matches!(robots.as_slice(), [robot] if Some(robot.resource_id.as_str()) == scope.robot_id.as_deref()) =>
            Err(AppError::Forbidden("A Robot Session keeps its exact device identity".into())),
        _ => Ok(()),
    }
}

impl NomiCoreSessionOwner {
    pub(super) async fn companion_execution_delivery_pending(&self, owner_id: &str, session_id: &str) -> Result<bool, AppError> {
        sqlx::query_scalar("WITH linked AS (SELECT DISTINCT execution.execution_id,execution.initial_plan_input FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id WHERE link.conversation_id=? AND execution.user_id=? AND execution.deleted_at IS NULL), reports AS (SELECT linked.execution_id,(SELECT json_extract(event.payload,'$.lead_report_operation_id') FROM agent_execution_events event WHERE event.execution_id=linked.execution_id AND json_type(event.payload,'$.lead_report_operation_id')='text' ORDER BY event.sequence DESC LIMIT 1) operation_id FROM linked) SELECT EXISTS(SELECT 1 FROM reports WHERE reports.operation_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM agent_execution_events delivered WHERE delivered.execution_id=reports.execution_id AND json_extract(delivered.payload,'$.change')='lead_report_delivered' AND json_extract(delivered.payload,'$.operation_id')=reports.operation_id)) OR EXISTS(SELECT 1 FROM linked JOIN agent_execution_attempts attempt ON attempt.execution_id=linked.execution_id JOIN agent_execution_events request ON request.execution_id=attempt.execution_id AND request.step_id=attempt.step_id AND request.attempt_id=attempt.attempt_id AND request.event_type='decision_requested' WHERE json_extract(linked.initial_plan_input,'$.mode')='directed' AND attempt.status='waiting_input' AND json_extract(request.payload,'$.question')=attempt.question AND request.sequence=(SELECT MAX(latest.sequence) FROM agent_execution_events latest WHERE latest.execution_id=attempt.execution_id AND latest.step_id=attempt.step_id AND latest.attempt_id=attempt.attempt_id AND latest.event_type='decision_requested' AND json_extract(latest.payload,'$.question')=attempt.question) AND NOT EXISTS(SELECT 1 FROM agent_execution_events reconciled WHERE reconciled.execution_id=request.execution_id AND json_extract(reconciled.payload,'$.change')='lead_input_notice_reconciled' AND json_extract(reconciled.payload,'$.request_event_sequence')=request.sequence))")
            .bind(session_id).bind(owner_id).fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))
    }

    pub(super) async fn adopt_companion_workspace(&self, companion_id: &str) -> Result<(), AppError> {
        let service = self.companion_service.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion workspace owner is unavailable".into()))?;
        if service.companion_workspace_component(companion_id).await?.is_some() { return Ok(()); }
        let owner_id = self.product_agent_resolver.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion Agent resolver is unavailable".into()))?.owner_id.to_string();
        let base = service.companion_workspaces_root();
        let ids = self.canonical_companion_session_ids(&owner_id, companion_id, false).await?;
        let mut chosen = None;
        // Adopt the oldest still-proven product workspace without moving files
        // or rewriting any existing Session binding.
        for id in ids.iter().rev() {
            let roots = self.companion_owned_workspaces(&owner_id, companion_id, std::slice::from_ref(id), None).await?;
            if let Some(root) = roots.into_iter().next() { chosen = Some(root); break; }
        }
        let component = match chosen {
            Some(root) => {
                let base = canonical_existing_workspace_directory(&base, WorkspaceDirectoryCheck::Runtime)?;
                let relative = root.strip_prefix(base).map_err(|_| AppError::Conflict("Companion workspace has a different owner root".into()))?;
                if relative.components().count() != 1 { return Err(AppError::Conflict("Companion workspace registration requires one exact managed directory".into())); }
                relative.to_str().ok_or_else(|| AppError::Conflict("Companion workspace name is not UTF-8".into()))?.to_owned()
            }
            None => service.companion_workspace_path(companion_id).await?.file_name()
                .and_then(|name| name.to_str()).ok_or_else(|| AppError::Conflict("Companion stable workspace name is invalid".into()))?.to_owned(),
        };
        service.register_companion_workspace_component(companion_id, &component).await?;
        Ok(())
    }

    pub(super) async fn companion_owned_workspaces(&self, owner_id: &str, companion_id: &str,
        session_ids: &[String], proposed: Option<&str>) -> Result<Vec<std::path::PathBuf>, AppError> {
        let service = self.companion_service.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion workspace owner is unavailable".into()))?;
        let base = service.companion_workspaces_root();
        let mut candidates = BTreeSet::new();
        if let Some(proposed) = proposed {
            let proposed = canonical_existing_workspace_directory(std::path::Path::new(proposed), WorkspaceDirectoryCheck::Runtime)?;
            let expected = service.companion_workspace_path(companion_id).await?;
            let expected = canonical_existing_workspace_directory(&expected, WorkspaceDirectoryCheck::Runtime)?;
            if proposed != expected { return Err(AppError::Forbidden("Companion cleanup target differs from its registered workspace".into())); }
            candidates.insert(proposed);
        }
        for session_id in session_ids {
            let (producer, key, payload): (String, String, String) = sqlx::query_as(
                "SELECT producer_id,idempotency_key,inline_json FROM agent_events WHERE session_id=? AND seq=1 AND kind='session/opening' AND kind_version=1")
                .bind(session_id).fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
            let payload: Value = serde_json::from_str(&payload).map_err(|error| AppError::Internal(error.to_string()))?;
            let ordinary = producer == "session_api" && payload["operation_id"].as_str() == Some(format!("open:{key}").as_str());
            let fork = producer == "session_api" && payload.get("parent_session_id").and_then(Value::as_str).is_some_and(|parent|
                key.strip_suffix(":fork:child-opening").is_some_and(|command|
                    command.starts_with(&format!("user:{owner_id}:{parent}:"))
                        && payload["operation_id"].as_str() == Some(format!("fork:{command}").as_str())));
            if !ordinary && !fork { return Err(AppError::Conflict("Companion workspace has no formal host opening ownership".into())); }
            let binding: AgentBindingValue = serde_json::from_value(payload["agent_binding"].clone())
                .map_err(|error| AppError::Conflict(format!("Companion opening binding is invalid: {error}")))?;
            if companion_id_from_binding(&binding)?.as_deref() != Some(companion_id) {
                return Err(AppError::Conflict("Companion opening workspace belongs to another persona".into()));
            }
            for resource in binding.typed_resource_bindings.iter().filter(|resource|
                resource.resource_kind.as_ref() == "workspace" && resource.owner_id == owner_id) {
                let Some(root) = resource.typed_parameters.get("workspace_root") else { continue; };
                match std::fs::symlink_metadata(root) {
                    Ok(_) => { candidates.insert(canonical_existing_workspace_directory(std::path::Path::new(root), WorkspaceDirectoryCheck::Runtime)?); }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(AppError::Internal(format!("inspect Companion opening workspace: {error}"))),
                }
            }
        }
        if candidates.is_empty() { return Ok(Vec::new()); }
        // Workspace candidates use the shared canonical spelling. On Windows
        // std::fs::canonicalize alone retains a verbatim prefix and would make
        // a real managed child fail this exact component containment check.
        let base = canonical_existing_workspace_directory(&base, WorkspaceDirectoryCheck::Runtime)?;
        if candidates.iter().any(|root| root == &base || !root.starts_with(&base)) {
            return Err(AppError::Conflict("A recorded Companion workspace is outside its managed owner directory".into()));
        }
        Ok(candidates.into_iter().collect())
    }

    pub(super) async fn is_companion_associated_task(&self, owner_id: &str, session_id: &AgentSessionId) -> Result<bool, AppError> {
        sqlx::query_scalar("WITH RECURSIVE ancestry(id,depth) AS (SELECT ?,0 UNION ALL SELECT parent.parent_agent_session_id,ancestry.depth+1 FROM agent_sessions parent JOIN ancestry ON parent.agent_session_id=ancestry.id WHERE parent.parent_agent_session_id IS NOT NULL AND ancestry.depth<64) SELECT EXISTS(SELECT 1 FROM ancestry JOIN agent_events opening ON opening.session_id=ancestry.id AND opening.seq=1 WHERE opening.kind='session/opening' AND opening.producer_id='session_api' AND opening.idempotency_key LIKE ?) OR EXISTS(SELECT 1 FROM ancestry WHERE depth=64)")
            .bind(session_id.as_ref()).bind(format!("user:{owner_id}:open:companion-directed:%"))
            .fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))
    }

    pub(crate) async fn cancel_robot_session_request(&self, owner_id: &str,
        request: &nomifun_robot::services::RobotTurnRequest, key: &str) -> Result<(), AppError> {
        let session = AgentSessionId::from(request.conversation_id.clone());
        let operation = Self::turn_operation_id(owner_id, session.as_ref(), key);
        let receipt = self.canonical.turn_receipt(&PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() }, &session, &operation).await?;
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::NotFound { return Ok(()); }
        self.cancel_exact_turn(owner_id, &session, &format!("robot-cancel:{key}"), &operation,
            nomifun_common::AgentKillReason::UserCancelled).await?;
        Ok(())
    }

    pub(super) fn companion_lifecycle_lock(&self, companion_id: &str) -> Arc<tokio::sync::RwLock<()>> {
        self.companion_lifecycle_locks.entry(companion_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(()))).clone()
    }

    pub(super) async fn canonical_companion_session_ids(&self, owner_id: &str, companion_id: &str,
        include_deleting: bool) -> Result<Vec<String>, AppError> {
        let legacy = format!("user:{owner_id}:open:companion-session:{companion_id}");
        // Destructive enumeration also probes unsupported stored openings and
        // remaining persona references. These are validation leads only: a
        // missing canonical scope fails closed instead of granting deletion.
        let candidates: Vec<(String, String)> = sqlx::query_as("SELECT session.agent_session_id,session.state FROM agent_sessions session JOIN agent_events opening ON opening.session_id=session.agent_session_id AND opening.seq=1 WHERE opening.kind='session/opening' AND opening.kind_version=1 AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=? AND (json_extract(opening.inline_json,'$.metadata.companion.companion_id')=? OR (opening.producer_id='session_api' AND opening.idempotency_key=?) OR (? AND (opening.payload_id IS NOT NULL OR EXISTS(SELECT 1 FROM json_each(session.agent_binding_json,'$.typed_resource_bindings') resource WHERE json_extract(resource.value,'$.resource_kind')='companion' AND json_extract(resource.value,'$.resource_id')=?)))) AND (session.state='live' OR (? AND session.state='deleting')) ORDER BY session.created_at DESC,session.agent_session_id DESC")
            .bind(owner_id).bind(companion_id).bind(&legacy).bind(include_deleting).bind(companion_id).bind(include_deleting)
            .fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        let mut ids = Vec::with_capacity(candidates.len());
        for (id, state) in candidates {
            let session_id = AgentSessionId::from(id.clone());
            let session = if state == "deleting" { self.canonical.store().get_deleting_session(&session_id).await }
                else { self.canonical.store().get_live_session(&session_id).await }.map_err(agent_session_store_error)?;
            if state == "deleting" {
                // Live-only event/head readers cannot read a fenced deletion.
                // Typed opening metadata remains available on its delete record.
                let scope = declared_companion_origin(owner_id, &session)?;
                let legacy_matches: bool = if scope.is_none() {
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND seq=1 AND kind='session/opening' AND kind_version=1 AND producer_id='session_api' AND idempotency_key=? AND event_id=? AND correlation_id=? AND json_extract(inline_json,'$.operation_id')=?)")
                        .bind(&id).bind(&legacy).bind(format!("opening:{legacy}")).bind(format!("open:{legacy}"))
                        .bind(format!("open:{legacy}")).fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?
                } else { false };
                if scope.as_ref().is_some_and(|scope| scope.companion_id == companion_id)
                    || (legacy_matches && companion_id_from_binding(&session.agent_binding)?.as_deref() == Some(companion_id)) {
                    ids.push(id);
                    continue;
                }
                return Err(AppError::Forbidden("Deleting Session has no exact Companion opening authority".into()));
            }
            let opening = self.canonical.store().read_events(&session_id, None, 1).await.map_err(agent_session_store_error)?;
            let head = self.canonical.store().head(&session_id).await.map_err(agent_session_store_error)?;
            let observation = SessionObservation { session, head, events: opening.events,
                messages: Vec::new(), next_cursor: opening.next_cursor, active_turn_outcome: None };
            if !canonical_companion_origin(owner_id, &observation)?.is_some_and(|scope| scope.companion_id == companion_id) {
                return Err(AppError::Forbidden("Companion membership has no exact canonical opening authority".into()));
            }
            ids.push(id);
        }
        Ok(ids)
    }

    /// Called only after the Robot gateway has authenticated the device and
    /// rechecked its live connection. Public input DTOs cannot author source.
    pub(crate) async fn send_robot_session_message_idempotent(&self, owner_id: &str,
        request: &nomifun_robot::services::RobotTurnRequest, key: &str,
        message: SendMessageRequest) -> Result<IdempotentMessageDelivery, AppError> {
        let session = AgentSessionId::from(request.conversation_id.clone());
        let _fence = self.session_operation_lock(session.as_ref()).write_owned().await;
        let observation = self.canonical.get(&PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() }, &session).await?;
        let scope = canonical_companion_origin(owner_id, &observation)?
            .ok_or_else(|| AppError::Forbidden("Authenticated Robot input requires a product Session".into()))?;
        if scope.role != CompanionSessionRole::Robot || scope.companion_id != request.companion_id
            || scope.robot_id.as_deref() != Some(request.robot_id.as_str()) {
            return Err(AppError::Forbidden("Robot input differs from its canonical device Session".into()));
        }
        let presented_question_id = self.capture_directed_question(owner_id, &session, &request.connection_id,
            key, request.presented_question_id.as_deref()).await?;
        let source = nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot {
            companion_id: request.companion_id.clone(), robot_id: request.robot_id.clone(),
            connection_id: request.connection_id.clone(),
            presented_question_id,
        };
        self.dispatch_canonical_turn_locked(owner_id, &session, key, message, false,
            None, None, None, false, None, Some(source)).await
    }

    pub(crate) async fn is_companion_robot_session(&self, owner_id: &str, session_id: &AgentSessionId) -> Result<bool, AppError> {
        let observation = self.canonical.get(&PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() }, session_id).await?;
        Ok(canonical_companion_origin(owner_id, &observation)?.is_some_and(|origin| origin.role == CompanionSessionRole::Robot))
    }

    pub(crate) async fn resolve_companion_desktop_delegate_snapshot(&self, owner_id: &str, source_session_id: &AgentSessionId) -> Result<AgentResolvedSnapshot, AppError> {
        let observation = self.canonical.get(&PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() }, source_session_id).await?;
        let origin = canonical_companion_origin(owner_id, &observation)?
            .filter(|origin| origin.role == CompanionSessionRole::Robot)
            .ok_or_else(|| AppError::Forbidden("Desktop delegation requires a canonical Robot Session".into()))?;
        let is_attempt: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id = ? AND relation = 'attempt')")
            .bind(source_session_id.as_ref()).fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        if is_attempt != 0 { return Err(AppError::Forbidden("An execution Attempt cannot recursively delegate".into())); }
        let resolver = self.product_agent_resolver.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion Agent resolver is unavailable".into()))?;
        let companions = self.companion_service.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion role configuration is unavailable".into()))?;
        let launch = companions.desktop_role_launch_configuration(&origin.companion_id).await?;
        let target = ProductAgentTarget { target_kind: "companion".into(), target_id: origin.companion_id,
            default_template_key: "companion.default".into() };
        let resolution = resolver.resolve(owner_id, &target, Some(&launch.model)).await?;
        let owner = UserId::from(owner_id.to_owned());
        let current = resolution.snapshot.canonical_binding.as_ref().ok_or_else(|| AppError::Conflict("Desktop role has no frozen binding".into()))?;
        let (_, _, frozen) = resolver.control_plane.saved_binding_artifacts(&owner, current).await.map_err(control_plane_error_to_app)?;
        let mut selection = session_capability_selection::selection(&frozen);
        selection.skill_names = launch.skill_names;
        let mut binding = resolver.control_plane.resolve_agent_session_capabilities_binding(&owner, current, Some(&selection)).await.map_err(control_plane_error_to_app)?;
        binding.typed_resource_bindings = current.typed_resource_bindings.iter()
            .filter(|resource| resource.resource_kind != "mcp_server").cloned().collect();
        freeze_selected_workspace(&mut binding, owner_id, &launch.workspace, WorkspaceDirectoryCheck::Create)?;
        let mcp = resolver.resource_bindings.resolve_mcp_for_saved_binding(&resolver.control_plane, &owner, &binding).await
            .map_err(|error| AppError::Conflict(format!("{}: {}", error.code(), error.message())))?;
        binding.typed_resource_bindings.retain(|resource| resource.resource_kind != "mcp_server");
        binding.typed_resource_bindings.extend(mcp);
        let (binding, revision, frozen) = resolver.control_plane.saved_binding_artifacts(&owner, &binding).await.map_err(control_plane_error_to_app)?;
        let owner = nomifun_common::UserId::parse(owner_id.to_owned()).map_err(|error| AppError::Forbidden(error.to_string()))?;
        Ok(super::super::agent_binding_projection::project_saved_artifacts(&owner, binding, revision, frozen, None)?.projection.snapshot)
    }
}
