//! Forward execution contracts are admitted at the canonical idle boundary.
//! Old snapshots remain immutable; the binding event is the history/replay fence.

use super::*;
use nomifun_agent_contracts::CapabilityId;

impl NomiCoreSessionOwner {
    /// Caller holds the Session operation write lock. Read-only projections,
    /// exact Turn redelivery and frozen voice/Remote/Attempt bindings do not
    /// acquire authority to advance a contract.
    pub(super) async fn prepare_session_contract_evolution(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<(), AppError> {
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner_id.into(),
        };
        let observation = self.canonical.get(&principal, session_id).await?;
        if observation.session.remote_binding_provenance.is_some()
            || observation.head.status == "running"
            || observation.head.active_turn_id.is_some()
        {
            return Ok(());
        }
        let attempt: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
             WHERE conversation_id = ? AND relation IN ('attempt', 'automation'))",
        )
        .bind(session_id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        // Group guests own a dedicated model-only contract. Their trusted
        // ingress prepares that ceiling separately; its empty official
        // chat.minimal variant is not the retired personal default.
        if attempt || self.is_companion_associated_task(owner_id, session_id).await?
            || self.is_registered_channel_guest(session_id).await?
        {
            return Ok(());
        }
        let control_plane = self.runtime_control_plane.get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let owner = UserId::from(owner_id.to_owned());
        let current = agent_binding_dto(&observation.session.agent_binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let mut candidate = control_plane.resolve_agent_session_contract_evolution(&owner, &current)
            .await.map_err(control_plane_error_to_app)?;
        if candidate == current {
            return Ok(());
        }
        let (_, _, source_snapshot) = control_plane.saved_binding_artifacts(&owner, &current)
            .await.map_err(control_plane_error_to_app)?;
        let (_, _, candidate_snapshot) = control_plane.saved_binding_artifacts(&owner, &candidate)
            .await.map_err(control_plane_error_to_app)?;
        let default_local_os_added = source_snapshot.content.enabled_capabilities.is_empty()
            && candidate_snapshot.content.capability_allowlist.contains(&CapabilityId::from("workspace.files"))
            && candidate_snapshot.content.capability_allowlist.contains(&CapabilityId::from("workspace.process"));
        if default_local_os_added {
            // Resolve infrastructure through the existing authority, then keep
            // the Session's actual selected directory. Historical zero-operation
            // workspace definitions remain immutable for old Effect references.
            let resolver = self.product_agent_resolver.get().and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| AppError::Conflict("Session product Agent resolver is unavailable".into()))?;
            let required = candidate_snapshot.content.required_resource_kinds.iter()
                .map(|kind| kind.as_ref().to_owned()).collect::<BTreeSet<_>>();
            let mut selections = current.typed_resource_bindings.iter().filter(|resource|
                required.contains(&resource.resource_kind)).map(|resource| AgentResourceSelectionDto {
                    resource_kind: resource.resource_kind.clone(),
                    resource_id: automatic_agent_resource_id(&resource.resource_kind)
                        .unwrap_or(&resource.resource_id).to_owned(),
                }).collect::<Vec<_>>();
            complete_automatic_agent_resource_selections(&mut selections, &required);
            candidate = resolver.resource_bindings.resolve_for_agent_switch(
                &control_plane, &owner, candidate, &selections,
            ).await.map_err(|error| AppError::Conflict(format!("{}: {}", error.code(), error.message())))?;
            // Keep the exact source's logical/selected/legacy location facts;
            // a contract transition must not reclassify a managed directory as
            // a user-selected project merely because it has an absolute root.
            let _ = frozen_workspace_root(&self.managed_workspace_root, owner_id, session_id,
                &observation.session.agent_binding)?;
            let location = current.typed_resource_bindings.iter()
                .find(|resource| resource.resource_kind == "workspace")
                .or_else(|| current.typed_resource_bindings.iter()
                    .find(|resource| resource.resource_kind == "process_session"))
                .map(|resource| resource.typed_parameters.clone());
            for resource in candidate.typed_resource_bindings.iter_mut().filter(|resource|
                matches!(resource.resource_kind.as_str(), "workspace" | "process_session"))
            {
                if let Some(previous) = current.typed_resource_bindings.iter()
                    .find(|previous| previous.resource_kind == resource.resource_kind)
                {
                    resource.resource_id = previous.resource_id.clone();
                    resource.connection_config_ref = previous.connection_config_ref.clone();
                }
                if let Some(location) = &location { resource.typed_parameters = location.clone(); }
                super::super::nomi_core_resource_bindings::freeze_resource_definition_id(resource)?;
            }
            // The product default changes local file/process infrastructure
            // only. An existing unrelated identity/resource definition remains
            // exact even when this empty predecessor had no consuming Action.
            candidate.typed_resource_bindings.extend(current.typed_resource_bindings.iter()
                .filter(|resource| !matches!(resource.resource_kind.as_str(), "workspace" | "process_session"))
                .cloned());
            candidate.typed_resource_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
        }
        candidate.binding_version = current.binding_version.checked_add(1)
            .ok_or_else(|| AppError::Conflict("AgentSession binding version cannot advance".into()))?;
        let (_, _, next_snapshot) = control_plane.saved_binding_artifacts(&owner, &candidate)
            .await.map_err(control_plane_error_to_app)?;
        let next_capabilities = next_snapshot.content.enabled_capabilities.iter()
            .map(|capability| capability.capability.id.as_ref().to_owned()).collect::<BTreeSet<_>>();
        let replacement: AgentBindingValue = serde_json::to_value(&candidate)
            .and_then(serde_json::from_value)
            .map_err(|error| AppError::Conflict(format!("evolved Session binding is invalid: {error}")))?;
        let (label, _) = control_plane.saved_binding_presentation(&owner, &current)
            .await.map_err(control_plane_error_to_app)?;
        let mut active = self.canonical.store().active_capability_ids(session_id)
            .await.map_err(agent_session_store_error)?;
        if default_local_os_added {
            active.extend(["workspace.files".to_owned(), "workspace.process".to_owned()]);
            active.sort();
            active.dedup();
        }
        active.retain(|id| next_capabilities.contains(id));
        self.settle_session_binding_runtime(owner_id, session_id).await?;
        let transition_id = Uuid::now_v7().to_string();
        let request_digest = digest_payload(&json!({
            "kind": "bundled_contract_evolution",
            "previous_binding": observation.session.agent_binding,
            "next_binding": replacement,
        })).map_err(|error| AppError::Internal(format!("digest contract transition: {error}")))?;
        self.canonical.store().replace_session_agent_binding(
            &principal,
            session_id,
            nomifun_agent_session::ReplaceSessionAgentBinding {
                expected: observation.session.agent_binding,
                replacement,
                previous_agent_label: label.clone(),
                next_agent_label: label,
                transition_id: transition_id.clone().into(),
                request_digest,
                idempotency_key: format!("contract-evolution:{transition_id}").into(),
                handoff_mode: AgentHandoffMode::ContextOnly,
                handoff: None,
                initial_active_capability_ids: active,
            },
        ).await.map_err(agent_session_store_error)?;
        Ok(())
    }

    /// Close the exact idle host before changing a binding. Settlement is
    /// checked again by the canonical transaction to cover late effect writes.
    pub(super) async fn settle_session_binding_runtime(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<(), AppError> {
        if let Some(blocker) = agent_switch_recovery_blocker(&self.pool, session_id).await
            .map_err(|error| AppError::Conflict(format!("{}: {}", error.code, error.message)))?
        {
            return Err(AppError::Conflict(format!("{}: {}", blocker.code, blocker.message)));
        }
        self.runtime_sessions.terminate_and_wait_result(
            session_id.as_ref(), Some(AgentKillReason::ConfigurationChanged),
        ).await?;
        super::super::hosted_effect_receipts::HostedEffectReceipts::new(self.pool.clone())
            .ensure_settled(owner_id, session_id.as_ref()).await?;
        Ok(())
    }
}
