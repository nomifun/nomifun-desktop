//! Forward execution contracts are admitted at the canonical idle boundary.
//! Old snapshots remain immutable; the binding event is the history/replay fence.

use super::*;

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
        if attempt {
            return Ok(());
        }
        let control_plane = self.runtime_control_plane.get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let owner = UserId::from(owner_id.to_owned());
        let current = agent_binding_dto(&observation.session.agent_binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let candidate = control_plane.resolve_agent_session_contract_evolution(&owner, &current)
            .await.map_err(control_plane_error_to_app)?;
        if candidate == current {
            return Ok(());
        }
        let replacement: AgentBindingValue = serde_json::to_value(&candidate)
            .and_then(serde_json::from_value)
            .map_err(|error| AppError::Conflict(format!("evolved Session binding is invalid: {error}")))?;
        let (label, _) = control_plane.saved_binding_presentation(&owner, &current)
            .await.map_err(control_plane_error_to_app)?;
        let active = self.canonical.store().active_capability_ids(session_id)
            .await.map_err(agent_session_store_error)?;
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
