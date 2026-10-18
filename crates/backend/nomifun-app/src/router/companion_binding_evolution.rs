//! One current-generation transition from a retired physical-role
//! binding to this Companion's saved primary Agent. Historical Snapshots and
//! Session identity remain immutable; active work and unknown effects fence it.

use super::*;

impl NomiCoreSessionOwner {
    /// Caller holds the existing Session operation write fence. This is only
    /// the precise prior product entry generation, never a general defaults
    /// refresh. Later explicit Session choices retain their own binding.
    pub(super) async fn prepare_retired_companion_device_binding_locked(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<bool, AppError> {
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner_id.into(),
        };
        let observation = self.canonical.get(&principal, session_id).await?;
        let Some(scope) = canonical_companion_origin(owner_id, &observation)?
            .filter(|scope| scope.role == nomifun_agent_contracts::CompanionSessionRole::Robot)
        else {
            return Ok(false);
        };
        let opening = self
            .canonical
            .store()
            .read_events(session_id, None, 1)
            .await
            .map_err(agent_session_store_error)?
            .events
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Conflict("Physical entry has no canonical opening".into()))?;
        let opening_key = opening.idempotency_key.as_ref();
        let prefix = format!("user:{owner_id}:open:");
        let host_opening = opening.producer_id.as_ref() == "session_api"
            && opening.seq == 1
            && opening.kind.0 == "session/opening"
            && opening.kind_version == 1
            && opening_key.starts_with(&prefix)
            && opening.event_id.as_ref() == format!("opening:{opening_key}")
            && opening.correlation_id.as_ref() == format!("open:{opening_key}")
            && matches!(&opening.payload,nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload)
                if payload.0.get("operation_id").and_then(Value::as_str)==Some(format!("open:{opening_key}").as_str()));
        if !host_opening {
            return Err(AppError::Conflict("Physical entry needs an explicit canonical Agent apply before its prior role can execute".into()));
        }
        if opening_key.starts_with(&format!("{prefix}unified-companion-entry:")) {
            return Ok(false);
        }
        let adoption_key = format!("companion-unified-agent:{}", session_id.as_ref());
        let adopted:Option<String>=sqlx::query_scalar("SELECT correlation_id FROM agent_events WHERE session_id=? AND producer_id='session_api' AND kind='session/agent-binding-changed' AND kind_version=1 AND idempotency_key=?")
            .bind(session_id.as_ref()).bind(&adoption_key).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if let Some(transition_id) = adopted {
            let transitions = self
                .canonical
                .store()
                .agent_binding_transitions(session_id)
                .await
                .map_err(agent_session_store_error)?;
            if !transitions
                .iter()
                .any(|transition| transition.transition_id.as_ref() == transition_id)
            {
                return Err(AppError::Conflict(
                    "Unified Companion adoption lost its exact canonical transition".into(),
                ));
            }
            return Ok(false);
        }
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let owner = UserId::from(owner_id.to_owned());
        let current = agent_binding_dto(&observation.session.agent_binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let retired_official = control_plane
            .is_retired_companion_device_binding(&owner, &current)
            .await
            .map_err(control_plane_error_to_app)?;
        if observation.session.remote_binding_provenance.is_some()
            || observation.head.active_turn_id.is_some()
            || observation.head.status != "ready"
        {
            return Err(AppError::Conflict("The retired physical entry must become idle before applying the unified Companion Agent".into()));
        }
        let linked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND relation IN ('attempt','automation'))")
            .bind(session_id.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if linked
            || self
                .companion_execution_delivery_pending(owner_id, session_id.as_ref())
                .await?
        {
            return Err(AppError::Conflict("Settle the physical entry's exact background work before applying its unified Agent".into()));
        }
        // The existing runtime/effect/process owner proves teardown. A pending
        // or unknown effect cannot be turned into a replay by this transition.
        self.settle_session_binding_runtime(owner_id, session_id)
            .await?;
        let companions = self
            .companion_service
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                AppError::Conflict("Companion launch configuration is unavailable".into())
            })?;
        let launch = companions
            .companion_launch_configuration(&scope.companion_id)
            .await?;
        let resolver = self
            .product_agent_resolver
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion Agent resolver is unavailable".into()))?;
        let target = ProductAgentTarget {
            target_kind: "companion".into(),
            target_id: scope.companion_id.clone(),
            default_template_key: "companion.default".into(),
        };
        let resolution = resolver
            .resolve(owner_id, &target, Some(&launch.model))
            .await?;
        let base = resolution
            .snapshot
            .canonical_binding
            .as_ref()
            .ok_or_else(|| {
                AppError::Conflict("Unified Companion Agent has no frozen binding".into())
            })?;
        let (_, _, snapshot) = control_plane
            .saved_binding_artifacts(&owner, base)
            .await
            .map_err(control_plane_error_to_app)?;
        let mut selection = session_capability_selection::selection(&snapshot);
        selection.skill_names.extend(launch.skill_names);
        selection.skill_names.sort();
        selection.skill_names.dedup();
        selection.mcp_server_ids.extend(launch.mcp_server_ids);
        selection.mcp_server_ids.sort();
        selection.mcp_server_ids.dedup();
        let mut candidate = control_plane
            .resolve_agent_session_capabilities_binding(&owner, base, Some(&selection))
            .await
            .map_err(control_plane_error_to_app)?;
        let (_, _, snapshot) = control_plane
            .saved_binding_artifacts(&owner, &candidate)
            .await
            .map_err(control_plane_error_to_app)?;
        let required = snapshot
            .content
            .required_resource_kinds
            .iter()
            .map(|kind| kind.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        let mut selections = product_agent_resource_selections(
            &target,
            &required,
            &candidate.typed_resource_bindings,
            None,
        );
        selections.retain(|resource| resource.resource_kind != "robot");
        if required.contains("robot") {
            selections.push(AgentResourceSelectionDto {
                resource_kind: "robot".into(),
                resource_id: scope.robot_id.clone().ok_or_else(|| {
                    AppError::Conflict("The physical entry has no exact device".into())
                })?,
            });
        }
        candidate = resolver
            .resource_bindings
            .resolve_for_agent_switch(&control_plane, &owner, candidate, &selections)
            .await
            .map_err(|error| {
                AppError::Conflict(format!("{}: {}", error.code(), error.message()))
            })?;
        let workspace = current
            .typed_resource_bindings
            .iter()
            .find(|resource| resource.resource_kind == "workspace")
            .and_then(|resource| resource.typed_parameters.get("workspace_root"))
            .map(String::as_str)
            .unwrap_or(&launch.workspace);
        freeze_carried_agent_switch_workspace(&mut candidate, owner_id, workspace, true)?;
        preserve_companion_refresh_resource_choices(&mut candidate, &current)?;
        // A physical input retains its exact device grant even when the newly
        // selected primary Agent has no device Action. Resource presence alone
        // never authorizes a model tool.
        candidate
            .typed_resource_bindings
            .retain(|resource| resource.resource_kind != "robot");
        candidate.typed_resource_bindings.extend(
            current
                .typed_resource_bindings
                .iter()
                .filter(|resource| resource.resource_kind == "robot")
                .cloned(),
        );
        let mcp = resolver
            .resource_bindings
            .resolve_mcp_for_saved_binding(&control_plane, &owner, &candidate)
            .await
            .map_err(|error| {
                AppError::Conflict(format!("{}: {}", error.code(), error.message()))
            })?;
        candidate
            .typed_resource_bindings
            .retain(|resource| resource.resource_kind != "mcp_server");
        candidate.typed_resource_bindings.extend(mcp);
        companion_sessions::validate_role_resources(&scope, &candidate)?;
        if candidate.preset_revision_ref == current.preset_revision_ref
            && candidate.resolved_snapshot_ref == current.resolved_snapshot_ref
        {
            // Even an identical shared selection needs a real canonical
            // generation boundary, so later explicit choices are not followed
            // by another implicit adoption of mutable profile defaults.
            let distinguished = control_plane
                .distinguish_agent_session_binding_identity(
                    &owner,
                    &candidate,
                    &candidate.preset_revision_ref.preset_id,
                )
                .await
                .map_err(control_plane_error_to_app)?;
            candidate.preset_revision_ref = distinguished.preset_revision_ref;
            candidate.resolved_snapshot_ref = distinguished.resolved_snapshot_ref;
        }
        let (mut replacement, _, next_snapshot) = control_plane
            .saved_binding_artifacts(&owner, &candidate)
            .await
            .map_err(control_plane_error_to_app)?;
        replacement.binding_version = observation
            .session
            .agent_binding
            .binding_version
            .checked_add(1)
            .ok_or_else(|| {
                AppError::Conflict("Companion Session binding version cannot advance".into())
            })?;
        if companion_id_from_binding(&replacement)?.as_deref() != Some(scope.companion_id.as_str())
        {
            return Err(AppError::Forbidden(
                "Unified Agent belongs to another Companion".into(),
            ));
        }
        let (previous_label, _) = control_plane
            .saved_binding_presentation(&owner, &current)
            .await
            .map_err(control_plane_error_to_app)?;
        let (next_label, _) = control_plane
            .saved_binding_presentation(&owner, &candidate)
            .await
            .map_err(control_plane_error_to_app)?;
        let transition_id = Uuid::now_v7().to_string();
        let request_digest=digest_payload(&json!({"kind":"unified_companion_entry_adoption","retired_official":retired_official,"companion_id":scope.companion_id,
            "previous_binding":observation.session.agent_binding,"next_binding":replacement})).map_err(|error|AppError::Internal(error.to_string()))?;
        self.canonical
            .store()
            .replace_session_agent_binding(
                &principal,
                session_id,
                nomifun_agent_session::ReplaceSessionAgentBinding {
                    expected: observation.session.agent_binding,
                    replacement,
                    previous_agent_label: previous_label,
                    next_agent_label: next_label,
                    transition_id: transition_id.clone().into(),
                    request_digest,
                    idempotency_key: adoption_key.into(),
                    handoff_mode: AgentHandoffMode::ContextOnly,
                    handoff: None,
                    initial_active_capability_ids: next_snapshot
                        .content
                        .enabled_capabilities
                        .iter()
                        .filter(|capability| capability.consumption.is_contribution())
                        .map(|capability| capability.capability.id.as_ref().to_owned())
                        .collect(),
                },
            )
            .await
            .map_err(agent_session_store_error)?;
        Ok(true)
    }
}
