//! Repository-authenticated group guests have a real immutable model-only
//! Agent contract. Conversation extra is presentation and never grants this role.
use super::*;

#[cfg(test)]
#[path = "channel_guest_session_tests.rs"]
mod tests;

impl NomiCoreSessionOwner {
    pub(crate) async fn is_registered_channel_guest(&self, session: &AgentSessionId) -> Result<bool, AppError> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channel_sessions cs JOIN channel_users cu \
            ON cu.channel_user_id=cs.channel_user_id WHERE cs.conversation_id=? \
            AND cs.chat_kind='group' AND cu.authorization_kind='auto_group')")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))
    }

    pub(crate) async fn ensure_channel_guest_idmm_allowed(&self, session: &AgentSessionId) -> Result<(), AppError> {
        if self.is_registered_channel_guest(session).await? {
            return Err(AppError::Conflict("CHANNEL_GUEST_IDMM_DISABLED: Automatically admitted group guests cannot observe or resume owner tasks through IDMM".into()));
        }
        Ok(())
    }

    /// A post-write authorization race may retire only an override belonging
    /// to a proven dedicated guest, never one merely referenced by a stale row.
    pub(crate) async fn is_dedicated_channel_guest(&self, owner_id: &str, session: &AgentSessionId) -> Result<bool, AppError> {
        let rows: Vec<nomifun_db::models::ChannelSessionRow> = sqlx::query_as(
            "SELECT cs.* FROM channel_sessions cs JOIN channel_users cu ON cu.channel_user_id=cs.channel_user_id \
             WHERE cs.conversation_id=? AND cs.chat_kind='group' AND cu.authorization_kind='auto_group' LIMIT 2",
        ).bind(session.as_ref()).fetch_all(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if rows.len() != 1 { return Ok(false); }
        let key = nomifun_channel::message_service::group_guest_creation_key(owner_id,&rows[0]);
        match self.checked_guest_observation(owner_id,session,&key).await {
            Ok(_) => Ok(true),
            Err(AppError::Conflict(_) | AppError::Forbidden(_) | AppError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) async fn channel_session_opening(&self, owner_id: &str, creation_key: &str)
        -> Result<Option<AgentSessionId>, AppError> {
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() };
        let key = format!("user:{owner_id}:open:{creation_key}");
        self.canonical.store().session_by_opening_key(&principal, &key).await
            .map(|found| found.map(|(session, _)| session.agent_session_id))
            .map_err(agent_session_store_error)
    }

    async fn guest_clear_boundary(&self, session: &AgentSessionId) -> Result<Option<i64>, AppError> {
        // Only canonical control facts decide whether an earlier broad context
        // needs a boundary. A model refresh does not replace the Agent contract.
        let (changed, cleared): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE(MAX(CASE WHEN kind='session/agent-binding-changed' THEN seq END),0), \
             COALESCE(MAX(CASE WHEN kind='context/cleared' THEN seq END),0) FROM agent_events WHERE session_id=?",
        ).bind(session.as_ref()).fetch_one(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok((changed > cleared).then_some(changed))
    }

    /// Opening-key proof prevents a guest ingress binding from repurposing an
    /// owner's existing chat. Explicit product/Attempt origins are also fenced.
    async fn checked_guest_observation(&self, owner_id: &str, session: &AgentSessionId, key: &str)
        -> Result<SessionObservation, AppError> {
        if self.channel_session_opening(owner_id, key).await?.as_ref() != Some(session) {
            return Err(AppError::Conflict("Channel guest Session has no exact dedicated host opening; reset its channel Session binding".into()));
        }
        let channels: Vec<nomifun_db::models::ChannelSessionRow> = sqlx::query_as(
            "SELECT * FROM channel_sessions WHERE conversation_id=? LIMIT 2",
        ).bind(session.as_ref()).fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        if channels.len() > 1 || channels.iter().any(|channel|
            channel.chat_kind != "group" || nomifun_channel::message_service::group_guest_creation_key(owner_id,channel) != key) {
            return Err(AppError::Conflict("A channel guest cannot repurpose a shared channel Session; reset its dedicated binding".into()));
        }
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() };
        let observation = self.canonical.get(&principal, session).await?;
        let linked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=?)")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        if observation.session.remote_binding_provenance.is_some()
            || observation.session.metadata.app_project_id.is_some()
            || canonical_companion_origin(owner_id, &observation)?.is_some()
            || self.is_companion_associated_task(owner_id, session).await? || linked
            || observation.session.metadata.purpose != nomifun_agent_contracts::SessionPurpose::Conversation {
            return Err(AppError::Conflict("A product or execution Session cannot become a dedicated channel guest chat".into()));
        }
        Ok(observation)
    }

    /// Caller holds the normal Session operation write lock. Existing current-
    /// generation guest Sessions narrow only at an idle, settled CAS boundary.
    async fn prepare_channel_guest_locked(&self, owner_id: &str, session: &AgentSessionId, key: &str)
        -> Result<(), AppError> {
        let observation = self.checked_guest_observation(owner_id, session, key).await?;
        let control = self.runtime_control_plane.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let owner = UserId::from(owner_id.to_owned());
        let current = agent_binding_dto(&observation.session.agent_binding).map_err(|error| AppError::Conflict(error.message))?;
        if control.validate_channel_guest_model_only_binding(&owner, &current).await.is_err() {
            if observation.head.status == "running" || observation.head.status == "paused"
                || observation.head.active_turn_id.is_some() {
                return Err(AppError::Conflict("Channel guest requires an idle Session before adopting its model-only contract".into()));
            }
            let (_, revision, _) = control.saved_binding_artifacts(&owner, &current).await.map_err(control_plane_error_to_app)?;
            let route = revision.payload.chat_route_records.get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
                .ok_or_else(|| AppError::Conflict("Channel guest has no exact saved Chat model".into()))?;
            let model = AgentChatModelSelectionDto { provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone() };
            let mut next = control.resolve_channel_guest_model_only_binding(&owner, &model).await.map_err(control_plane_error_to_app)?;
            next.binding_version = current.binding_version.checked_add(1)
                .ok_or_else(|| AppError::Conflict("Session binding version cannot advance".into()))?;
            let replacement: AgentBindingValue = serde_json::to_value(next).and_then(serde_json::from_value)
                .map_err(|error| AppError::Conflict(error.to_string()))?;
            let (previous_label, _) = control.saved_binding_presentation(&owner, &current).await.map_err(control_plane_error_to_app)?;
            self.settle_session_binding_runtime(owner_id, session).await?;
            let transition = Uuid::now_v7().to_string();
            let request_digest = digest_payload(&json!({"kind":"channel_group_guest_ceiling","opening_key":key,
                "previous_binding":observation.session.agent_binding,"next_binding":replacement}))
                .map_err(|error| AppError::Internal(error.to_string()))?;
            self.canonical.store().replace_session_agent_binding(
                &PrincipalRef { principal_kind:"user".into(), principal_id:owner_id.into() }, session,
                nomifun_agent_session::ReplaceSessionAgentBinding {
                    expected:observation.session.agent_binding,replacement,previous_agent_label:previous_label,
                    next_agent_label:"Channel guest".into(),transition_id:transition.clone().into(),request_digest,
                    idempotency_key:format!("channel-group-guest-ceiling:{transition}").into(),
                    handoff_mode:AgentHandoffMode::ContextOnly,handoff:None,initial_active_capability_ids:vec![],
                },
            ).await.map_err(agent_session_store_error)?;
        }
        if let Some(sequence) = self.guest_clear_boundary(session).await? {
            self.clear_session_context_locked(owner_id, session,
                &format!("channel-group-guest-context:{}:{sequence}",session.as_ref()),
                "channel_group_guest_ceiling", None).await?;
        }
        Ok(())
    }

    pub(super) async fn create_channel_group_guest(&self, owner_id: &str, mut request: CreateConversationRequest, key: &str)
        -> Result<ConversationResponse, AppError> {
        let session = if let Some(existing) = self.channel_session_opening(owner_id, key).await? { existing } else {
            let model = request.model.as_ref().ok_or_else(|| AppError::Conflict("Channel guest requires an exact model".into()))?;
            let control = self.runtime_control_plane.get().and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
            let owner = UserId::from(owner_id.to_owned());
            let selected = AgentChatModelSelectionDto { provider_id:model.provider_id.clone(),model:model.model.clone() };
            let binding = control.resolve_channel_guest_model_only_binding(&owner, &selected).await.map_err(control_plane_error_to_app)?;
            let (binding, revision, snapshot) = control.saved_binding_artifacts(&owner, &binding).await.map_err(control_plane_error_to_app)?;
            let common_owner = nomifun_common::UserId::parse(owner_id.to_owned()).map_err(|error| AppError::Forbidden(error.to_string()))?;
            let projected = super::super::agent_binding_projection::project_saved_artifacts(&common_owner,binding,revision,snapshot,request.name.as_deref())?;
            // Caller extras (including product/persona/edited preset choices)
            // cannot replace the host's empty frozen binding.
            request.extra = json!({}); request.preset_id = None;
            self.create_session_idempotent(owner_id,request,Some(projected.projection.snapshot),key).await?.conversation_id.into()
        };
        self.checked_guest_observation(owner_id,&session,key).await?;
        // IDMM evaluation owns its own mutex before entering the Session. Join
        // that producer outside the Session write fence to preserve lock order.
        self.remove_idmm_state(session.as_ref()).await?;
        let _operation = self.session_operation_lock(session.as_ref()).write_owned().await;
        self.prepare_channel_guest_locked(owner_id,&session,key).await?;
        self.canonical_conversation_projection_readonly(owner_id,&session).await?
            .ok_or_else(|| AppError::NotFound("Channel guest Session disappeared".into()))
    }

    pub(super) async fn send_channel_group_guest(&self, owner_id: &str, session_id: &str, key: &str,
        mut request: SendMessageRequest, opening_key: &str) -> Result<nomifun_channel::ChannelTurnDelivery, AppError> {
        Self::validate_ordinary_message_origin(&request)?;
        let session = AgentSessionId::from(session_id.to_owned());
        self.checked_guest_observation(owner_id,&session,opening_key).await?;
        self.remove_idmm_state(session_id).await?;
        let _operation = self.session_operation_lock(session_id).write_owned().await;
        self.prepare_channel_guest_locked(owner_id,&session,opening_key).await?;
        let operation = Self::turn_operation_id(owner_id,session_id,key);
        let receipt = self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
        // The host marks newly accepted external guests so ambient learning
        // cannot mistake them for the installation owner. Exact old accepted
        // roots keep their original source and never broadcast a fresh input.
        request.origin = if receipt.started_event.is_some() {
            let (root, accepted) = self.canonical.store().turn_accepted_input(&session,&operation).await.map_err(agent_session_store_error)?;
            let principal = PrincipalRef { principal_kind:"user".into(),principal_id:owner_id.into() };
            let current = self.canonical.get(&principal,&session).await?;
            let admitted: nomifun_agent_contracts::ResolvedSnapshotRef = serde_json::from_value(
                accepted.0.pointer("/admission/resolved_snapshot_ref").cloned().unwrap_or(Value::Null),
            ).map_err(|_| AppError::Conflict("Channel guest replay has no exact model-only admission proof".into()))?;
            let floor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
                .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
            // Current authority alone cannot authorize old broad results. A
            // replay must belong to this proven empty Snapshot and follow the
            // actual canonical clear that removed its predecessor's context.
            if admitted != current.session.agent_binding.resolved_snapshot_ref
                || root.seq <= u64::try_from(floor).map_err(|_| AppError::Conflict("Invalid guest context boundary".into()))? {
                return Err(AppError::Conflict("Channel guest cannot deliver a result from before its proven model-only context boundary".into()));
            }
            match accepted.0.get("origin") {
                None | Some(Value::Null) => None,
                Some(Value::String(origin)) if origin == "channel_guest" => Some(origin.clone()),
                _ => return Err(AppError::Conflict("Channel guest replay has a different original input source".into())),
            }
        } else { Some("channel_guest".into()) };
        let delivery = self.dispatch_canonical_turn_locked(owner_id,&session,key,request,false,None,None,None,false,None,None).await?;
        let events = if delivery.completed { None } else { wait_for_runtime_subscription(&self.runtime_sessions,session_id).await };
        Ok(nomifun_channel::ChannelTurnDelivery { delivery:channel_delivery_from_conversation(delivery),events })
    }

    /// All native recovery, warmup and new-input admissions cross this guard.
    /// Repository relationships identify guests; actual immutable artifacts and
    /// a canonical context floor prove the execution ceiling. It never repairs
    /// an active/unknown Effect or changes a binding while reading/preparing.
    pub(super) async fn validate_registered_channel_guest_runtime(&self, owner_id: &str, session: &AgentSessionId)
        -> Result<(), AppError> {
        let guests: Vec<nomifun_db::models::ChannelSessionRow> = sqlx::query_as(
            "SELECT cs.* FROM channel_sessions cs JOIN channel_users cu ON cu.channel_user_id=cs.channel_user_id \
             WHERE cs.conversation_id=? AND cs.chat_kind='group' AND cu.authorization_kind='auto_group' LIMIT 2",
        ).bind(session.as_ref()).fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        if guests.is_empty() { return Ok(()); }
        let control = self.runtime_control_plane.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        for guest in guests {
            let key = nomifun_channel::message_service::group_guest_creation_key(owner_id,&guest);
            let observation = self.checked_guest_observation(owner_id,session,&key).await?;
            let binding = agent_binding_dto(&observation.session.agent_binding).map_err(|error| AppError::Conflict(error.message))?;
            control.validate_channel_guest_model_only_binding(&UserId::from(owner_id.to_owned()),&binding).await.map_err(control_plane_error_to_app)?;
        }
        if self.guest_clear_boundary(session).await?.is_some() {
            return Err(AppError::Conflict("Channel guest context adoption must finish at its trusted input boundary before Runtime can start".into()));
        }
        Ok(())
    }
}
