use nomifun_agent_contracts::{
    AgentBindingValue, AgentHandoffBindingRefV1, AgentSessionId, AgentSessionLiveRecord, AgentSessionMetadata, ArtifactId,
    CorrelationId, DeleteAgentSessionCommand, EventId, EventProducerId, IdempotencyKey,
    OperationId, PrincipalRef, ReasoningEffort, SemanticSessionEventDraft, SessionEventAppend, SessionEventCursor,
    SessionEventKind, SessionEventPayloadRef, SessionPayloadBody, StrictJsonValue,
};
use nomifun_agent_session::{
    AgentSessionStore, CreateSessionRequest, DeleteResult, ForkContextSnapshot, ForkRequest, ForkResult,
    MessageProjection, SessionEventPage, SessionObservation, SessionStoreError, TurnReceipt,
    canonical_context_messages,
};
use nomifun_common::AppError;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone)]
pub struct CanonicalAgentSessionOwner {
    store: AgentSessionStore,
}

#[derive(Clone, Debug)]
pub struct OpenAgentSession {
    pub session: AgentSessionLiveRecord,
    pub cursor: SessionEventCursor,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub struct AgentTurnReceipt {
    pub operation_id: OperationId,
    pub cursor: SessionEventCursor,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub struct AgentMutationReceipt {
    pub target_operation_id: OperationId,
    pub event_id: EventId,
    pub cursor: SessionEventCursor,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub enum PreparedAgentSessionDelete {
    AlreadyDeleted(DeleteResult),
    Fenced(DeleteAgentSessionCommand),
}

impl CanonicalAgentSessionOwner {
    pub async fn from_pool(pool: nomifun_db::SqlitePool) -> Result<Self, AppError> {
        Ok(Self {
            store: AgentSessionStore::from_pool(pool)
                .await
                .map_err(store_error)?,
        })
    }

    pub fn store(&self) -> &AgentSessionStore {
        &self.store
    }

    pub async fn open(
        &self,
        owner: PrincipalRef,
        binding: AgentBindingValue,
        title: Option<String>,
        active_capability_ids: Vec<String>,
        idempotency_key: &str,
        created_at: i64,
    ) -> Result<OpenAgentSession, AppError> {
        self.open_with_provenance(
            owner,
            binding,
            title,
            active_capability_ids,
            None,
            idempotency_key,
            created_at,
        )
        .await
    }

    pub async fn open_with_provenance(
        &self,
        owner: PrincipalRef,
        binding: AgentBindingValue,
        title: Option<String>,
        active_capability_ids: Vec<String>,
        remote_binding_provenance: Option<nomifun_agent_contracts::RemoteBindingProvenance>,
        idempotency_key: &str,
        created_at: i64,
    ) -> Result<OpenAgentSession, AppError> {
        self.open_with_options(
            owner,
            binding,
            title,
            active_capability_ids,
            remote_binding_provenance,
            None,
            idempotency_key,
            created_at,
        )
        .await
    }

    pub async fn open_with_reasoning_effort(
        &self,
        owner: PrincipalRef,
        binding: AgentBindingValue,
        title: Option<String>,
        active_capability_ids: Vec<String>,
        reasoning_effort: Option<ReasoningEffort>,
        idempotency_key: &str,
        created_at: i64,
    ) -> Result<OpenAgentSession, AppError> {
        self.open_with_options(
            owner,
            binding,
            title,
            active_capability_ids,
            None,
            reasoning_effort,
            idempotency_key,
            created_at,
        )
        .await
    }

    async fn open_with_options(
        &self,
        owner: PrincipalRef,
        binding: AgentBindingValue,
        title: Option<String>,
        active_capability_ids: Vec<String>,
        remote_binding_provenance: Option<nomifun_agent_contracts::RemoteBindingProvenance>,
        reasoning_effort: Option<ReasoningEffort>,
        idempotency_key: &str,
        created_at: i64,
    ) -> Result<OpenAgentSession, AppError> {
        let key = scoped_key(&owner, idempotency_key, "open")?;
        let producer = EventProducerId::from("session_api");
        let session_id = AgentSessionId::from(Uuid::now_v7().to_string());
        let session = AgentSessionLiveRecord {
            agent_session_id: session_id.clone(),
            owner_ref: owner.clone(),
            metadata: AgentSessionMetadata {
                title,
                archived: false,
                pinned: false,
                reasoning_effort,
            },
            agent_binding: binding,
            remote_binding_provenance,
            parent_session_id: None,
            fork_base_payload_id: None,
            next_seq: 1,
        };
        let request = CreateSessionRequest {
            session: session.clone(),
            created_at,
            operation_id: OperationId::from(format!("open:{key}")),
            producer_id: producer.clone(),
            idempotency_key: IdempotencyKey::from(key.clone()),
            correlation_id: CorrelationId::from(format!("open:{key}")),
            initial_input: None,
            opening_event_id: Some(EventId::from(format!("opening:{key}"))),
            activation_event_id: Some(EventId::from(format!("active-set:{key}"))),
            initial_active_capability_ids: active_capability_ids,
        };
        let created = self
            .store
            .create_session(request)
            .await
            .map_err(store_error)?;
        self.finish_open(created.session, &key, created.duplicate)
            .await
    }

    async fn finish_open(
        &self,
        session: AgentSessionLiveRecord,
        key: &str,
        duplicate: bool,
    ) -> Result<OpenAgentSession, AppError> {
        let ready = SessionEventAppend {
            agent_session_id: session.agent_session_id.clone(),
            event_id: EventId::from(format!("ready:{key}")),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(format!("{key}:ready")),
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("session/ready".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(format!(
                    "session:{}",
                    session.agent_session_id.as_ref()
                )),
                causation_event_id: Some(EventId::from(format!("opening:{key}"))),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "resolved_snapshot_ref": session.agent_binding.resolved_snapshot_ref,
                }))),
            },
        };
        let ready = self.store.append_event(&ready).await.map_err(store_error)?;
        Ok(OpenAgentSession {
            cursor: ready.cursor,
            session,
            duplicate,
        })
    }

    pub async fn get(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionObservation, AppError> {
        self.require_owner(owner, session_id).await?;
        self.store
            .observe(session_id, None, 500)
            .await
            .map_err(store_error)
    }

    pub async fn messages(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        after_seq: u64,
    ) -> Result<Vec<MessageProjection>, AppError> {
        self.require_owner(owner, session_id).await?;
        self.store
            .messages_after(session_id, after_seq)
            .await
            .map_err(store_error)
    }

    /// Latest runtime state is independent of transcript/event paging. Do not
    /// hydrate the full SessionObservation for frequently refreshed progress.
    pub async fn latest_runtime_state(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        event_name: &str,
    ) -> Result<nomifun_agent_session::RuntimeStateObservation, AppError> {
        self.require_owner(owner, session_id).await?;
        self.store.latest_runtime_state(session_id, event_name).await.map_err(store_error)
    }

    pub async fn events(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionEventPage, AppError> {
        self.require_owner(owner, session_id).await?;
        self.store
            .read_events(session_id, after, limit)
            .await
            .map_err(store_error)
    }

    pub async fn active_capability_ids(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<Vec<String>, AppError> {
        self.require_owner(owner, session_id).await?;
        self.store
            .active_capability_ids(session_id)
            .await
            .map_err(store_error)
    }

    pub async fn start_turn(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        input: Value,
    ) -> Result<AgentTurnReceipt, AppError> {
        self.start_turn_with_admission(owner, session_id, idempotency_key, input, false)
            .await
    }

    pub async fn start_initial_turn(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        input: Value,
    ) -> Result<AgentTurnReceipt, AppError> {
        self.start_turn_with_admission(owner, session_id, idempotency_key, input, true)
            .await
    }

    pub async fn start_voice_turn(&self,owner:&PrincipalRef,session:&AgentSessionId,key:&str,input:Value,fence:&nomifun_agent_session::NativeInputContextFence)->Result<AgentTurnReceipt,AppError> {
        self.require_owner(owner,session).await?;let key=scoped_key(owner,key,session.as_ref())?;let operation_id=OperationId::from(format!("turn:{key}"));
        let (_,result)=self.store.start_voice_turn(session,"session_api".into(),key.into(),operation_id.clone(),StrictJsonValue(input),fence).await.map_err(store_error)?;
        Ok(AgentTurnReceipt {operation_id,cursor:result.cursor,duplicate:result.duplicate})
    }

    async fn start_turn_with_admission(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        input: Value,
        initial_only: bool,
    ) -> Result<AgentTurnReceipt, AppError> {
        self.require_owner(owner, session_id).await?;
        let key = scoped_key(owner, idempotency_key, session_id.as_ref())?;
        let operation_id = OperationId::from(format!("turn:{key}"));
        let (_, turn_result) = if initial_only {
            self.store
                .start_initial_turn(
                    session_id,
                    EventProducerId::from("session_api"),
                    IdempotencyKey::from(key),
                    operation_id.clone(),
                    StrictJsonValue(input),
                )
                .await
        } else {
            self.store
                .start_turn(
                    session_id,
                    EventProducerId::from("session_api"),
                    IdempotencyKey::from(key),
                    operation_id.clone(),
                    StrictJsonValue(input),
                )
                .await
        }
        .map_err(store_error)?;
        Ok(AgentTurnReceipt {
            operation_id,
            cursor: turn_result.cursor,
            duplicate: turn_result.duplicate,
        })
    }

    pub async fn steer(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        input: Value,
    ) -> Result<AgentMutationReceipt, AppError> {
        self.require_owner(owner, session_id).await?;
        let key = scoped_key(owner, idempotency_key, session_id.as_ref())?;
        let (target_operation_id, result) = self
            .store
            .steer_active_turn(
                session_id,
                IdempotencyKey::from(format!("{key}:steer")),
                EventProducerId::from("session_api"),
                StrictJsonValue(input),
            )
            .await
            .map_err(store_error)?;
        let event_id = result
            .record
            .as_ref()
            .map(|record| record.event_id.clone())
            .ok_or_else(|| AppError::Conflict("canonical steer produced no durable event".to_owned()))?;
        Ok(AgentMutationReceipt {
            target_operation_id,
            event_id,
            cursor: result.cursor,
            duplicate: result.duplicate,
        })
    }

    pub async fn steer_exact_turn(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        target_operation_id: &OperationId,
        input: Value,
    ) -> Result<AgentMutationReceipt, AppError> {
        self.require_owner(owner, session_id).await?;
        let key = scoped_key(owner, idempotency_key, session_id.as_ref())?;
        let (target_operation_id, result) = self.store.steer_exact_turn(
            session_id, target_operation_id, IdempotencyKey::from(format!("{key}:steer")),
            EventProducerId::from("session_api"), StrictJsonValue(input),
        ).await.map_err(store_error)?;
        let event_id = result.record.as_ref().map(|record| record.event_id.clone())
            .ok_or_else(|| AppError::Conflict("canonical steering has no durable Turn receipt".into()))?;
        Ok(AgentMutationReceipt { target_operation_id, event_id, cursor: result.cursor, duplicate: result.duplicate })
    }

    pub async fn cancel(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
    ) -> Result<AgentMutationReceipt, AppError> {
        self.require_owner(owner, session_id).await?;
        let key = scoped_key(owner, idempotency_key, session_id.as_ref())?;
        let (target_operation_id, result) = self
            .store
            .cancel_active_turn(
                session_id,
                IdempotencyKey::from(format!("{key}:cancel")),
                EventProducerId::from("session_api"),
            )
            .await
            .map_err(store_error)?;
        let event_id = result
            .record
            .as_ref()
            .map(|record| record.event_id.clone())
            .ok_or_else(|| AppError::Conflict("canonical cancel produced no durable event".to_owned()))?;
        Ok(AgentMutationReceipt {
            target_operation_id,
            event_id,
            cursor: result.cursor,
            duplicate: result.duplicate,
        })
    }

    pub async fn cancel_exact_turn(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        target_operation_id: &OperationId,
    ) -> Result<AgentMutationReceipt, AppError> {
        self.require_owner(owner, session_id).await?;
        let key = scoped_key(owner, idempotency_key, session_id.as_ref())?;
        let (target_operation_id, result) = self.store.cancel_exact_turn(
            session_id, target_operation_id, IdempotencyKey::from(format!("{key}:cancel")),
            EventProducerId::from("session_api"),
        ).await.map_err(store_error)?;
        let event_id = result.record.as_ref().map(|record| record.event_id.clone())
            .ok_or_else(|| AppError::Conflict("canonical cancellation has no durable Turn receipt".into()))?;
        Ok(AgentMutationReceipt { target_operation_id, event_id, cursor: result.cursor, duplicate: result.duplicate })
    }

    /// Opt-in native proof over the same exact Turn cancellation writer.
    pub async fn cancel_exact_native_turn(&self,owner:&PrincipalRef,session_id:&AgentSessionId,key:&str,target:&OperationId,
        fence:&nomifun_agent_session::NativeTurnMutationFence)->Result<AgentMutationReceipt,AppError> {
        self.require_owner(owner,session_id).await?;let key=scoped_key(owner,key,session_id.as_ref())?;
        let (target_operation_id,result)=self.store.cancel_exact_native_turn(session_id,target,format!("{key}:cancel").into(),"session_api".into(),fence).await.map_err(store_error)?;
        let event_id=result.record.as_ref().map(|record|record.event_id.clone()).ok_or_else(||AppError::Conflict("canonical cancellation has no durable Turn receipt".into()))?;
        Ok(AgentMutationReceipt {target_operation_id,event_id,cursor:result.cursor,duplicate:result.duplicate})
    }

    pub async fn steer_exact_native_turn(&self,owner:&PrincipalRef,session_id:&AgentSessionId,key:&str,target:&OperationId,input:Value,
        fence:&nomifun_agent_session::NativeTurnMutationFence)->Result<AgentMutationReceipt,AppError> {
        self.require_owner(owner,session_id).await?;let key=scoped_key(owner,key,session_id.as_ref())?;
        let (target_operation_id,result)=self.store.steer_exact_native_turn(session_id,target,format!("{key}:steer").into(),"session_api".into(),StrictJsonValue(input),fence).await.map_err(store_error)?;
        let event_id=result.record.as_ref().map(|record|record.event_id.clone()).ok_or_else(||AppError::Conflict("canonical steering has no durable Turn receipt".into()))?;
        Ok(AgentMutationReceipt {target_operation_id,event_id,cursor:result.cursor,duplicate:result.duplicate})
    }

    pub async fn steer_exact_native_immediate_turn(&self,owner:&PrincipalRef,session_id:&AgentSessionId,key:&str,target:&OperationId,input:Value,
        fence:&nomifun_agent_session::NativeTurnMutationFence)->Result<AgentMutationReceipt,AppError> {
        self.require_owner(owner,session_id).await?;let key=scoped_key(owner,key,session_id.as_ref())?;
        let (target_operation_id,result)=self.store.steer_exact_native_immediate_turn(session_id,target,format!("{key}:steer").into(),"session_api".into(),StrictJsonValue(input),fence).await.map_err(store_error)?;
        let event_id=result.record.as_ref().map(|record|record.event_id.clone()).ok_or_else(||AppError::Conflict("canonical voice steering has no durable Turn receipt".into()))?;
        Ok(AgentMutationReceipt {target_operation_id,event_id,cursor:result.cursor,duplicate:result.duplicate})
    }

    pub async fn fork(
        &self,
        owner: &PrincipalRef,
        parent_session_id: &AgentSessionId,
        parent_through_seq: u64,
        title: Option<String>,
        idempotency_key: &str,
        created_at: i64,
    ) -> Result<ForkResult, AppError> {
        let key = scoped_key(owner, idempotency_key, parent_session_id.as_ref())?;
        let producer = EventProducerId::from("session_api");
        let event_key = IdempotencyKey::from(format!("{key}:fork"));
        if let Some(receipt) = self.store.existing_fork_receipt(owner, parent_session_id,
            parent_through_seq, title.as_deref(), &producer, &event_key).await.map_err(store_error)? {
            return Ok(receipt);
        }
        let parent = self.require_owner(owner, parent_session_id).await?;
        let facts = self.store.chat_causality_facts(parent_session_id, &OperationId::from("fork-context"))
            .await.map_err(store_error)?;
        if parent_through_seq > facts.head.last_seq {
            return Err(AppError::Conflict("fork cursor is ahead of its committed parent events".into()));
        }
        let before_seq = parent_through_seq.checked_add(1)
            .ok_or_else(|| AppError::Conflict("fork cursor is exhausted".into()))?;
        let context_floor = facts.events.iter().filter(|event| event.kind.0 == "context/cleared" && event.seq <= parent_through_seq)
            .map(|event| event.seq).max().unwrap_or(0);
        let mut messages = facts.fork_context.as_ref().map(|snapshot| snapshot.base_context(context_floor))
            .transpose().map_err(store_error)?.unwrap_or_default();
        messages.extend(canonical_context_messages(&facts.events, &facts.event_payloads,
            context_floor, parent_through_seq, before_seq).map_err(store_error)?);
        let depth = facts.fork_context.as_ref().map_or(0, |snapshot| snapshot.fork_depth)
            .checked_add(1).ok_or_else(|| AppError::Conflict("fork ancestry depth is exhausted".into()))?;
        let base = ForkContextSnapshot::new(parent_session_id.clone(), parent_through_seq,
            AgentHandoffBindingRefV1::from(&parent.agent_binding), depth, messages).map_err(store_error)?;
        let active_capability_ids = self
            .store
            .active_capability_ids(parent_session_id)
            .await
            .map_err(store_error)?;
        let child_id = AgentSessionId::from(Uuid::now_v7().to_string());
        let request = ForkRequest {
            child_session_id: child_id,
            child_owner_ref: owner.clone(),
            child_metadata: AgentSessionMetadata {
                title,
                archived: false,
                pinned: false,
                reasoning_effort: parent.metadata.reasoning_effort,
            },
            child_agent_binding: parent.agent_binding.clone(),
            parent_through_seq,
            created_at,
            producer_id: producer.clone(),
            operation_id: OperationId::from(format!("fork:{key}")),
            idempotency_key: event_key.clone(),
            correlation_id: CorrelationId::from(format!("fork:{key}")),
            event_id: Some(EventId::from(format!("forked:{key}"))),
            base_payload_id: ArtifactId::from(format!("fork-base:{key}")),
            base_body: SessionPayloadBody::Json(StrictJsonValue(serde_json::to_value(base)
                .map_err(|error| AppError::Internal(error.to_string()))?)),
            base_media_type: "application/json".to_owned(),
            child_initial_active_capability_ids: active_capability_ids,
        };
        self.store
            .fork_session(parent_session_id, request)
            .await
            .map_err(store_error)
    }

    /// Commit the canonical admission fence before any resource owner begins
    /// cleanup. Re-entering an interrupted `deleting` row is intentional: the
    /// caller must rerun idempotent owner cleanup before completing the purge.
    pub async fn fence_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        deleted_at: i64,
    ) -> Result<PreparedAgentSessionDelete, AppError> {
        let command = DeleteAgentSessionCommand {
            operation_id: OperationId::from(format!(
                "delete:{}",
                scoped_key(owner, idempotency_key, session_id.as_ref())?
            )),
            agent_session_id: session_id.clone(),
            owner_ref: owner.clone(),
            requested_at: deleted_at,
        };
        match self.store.get_live_session(session_id).await {
            Ok(session) if &session.owner_ref == owner => {}
            Ok(_) => {
                return Err(AppError::Forbidden(
                    "AgentSession belongs to another owner".to_owned(),
                ));
            }
            Err(SessionStoreError::Deleted(_)) => {
                if let Some(tombstone) = self
                    .store
                    .inspect_tombstone(session_id)
                    .await
                    .map_err(store_error)?
                {
                    if &tombstone.owner_ref != owner {
                        return Err(AppError::Forbidden(
                            "AgentSession belongs to another owner".to_owned(),
                        ));
                    }
                    return Ok(PreparedAgentSessionDelete::AlreadyDeleted(DeleteResult {
                        tombstone,
                        operation_id: command.operation_id,
                    }));
                }
                let deleting = self
                    .store
                    .get_deleting_session(session_id)
                    .await
                    .map_err(store_error)?;
                if &deleting.owner_ref != owner {
                    return Err(AppError::Forbidden(
                        "AgentSession belongs to another owner".to_owned(),
                    ));
                }
            }
            Err(error) => return Err(store_error(error)),
        }
        match self.store.fence_delete(&command).await {
            Ok(_) => Ok(PreparedAgentSessionDelete::Fenced(command)),
            Err(SessionStoreError::Deleted(_)) => {
                let tombstone = self
                    .store
                    .inspect_tombstone(session_id)
                    .await
                    .map_err(store_error)?
                    .ok_or_else(|| AppError::Conflict(
                        "AgentSession delete state changed without a durable tombstone".to_owned(),
                    ))?;
                if &tombstone.owner_ref != owner {
                    return Err(AppError::Forbidden(
                        "AgentSession belongs to another owner".to_owned(),
                    ));
                }
                Ok(PreparedAgentSessionDelete::AlreadyDeleted(DeleteResult {
                    tombstone,
                    operation_id: command.operation_id,
                }))
            }
            Err(error) => Err(store_error(error)),
        }
    }

    pub async fn complete_fenced_delete(
        &self,
        command: &DeleteAgentSessionCommand,
        deleted_at: i64,
    ) -> Result<DeleteResult, AppError> {
        self.store
            .complete_delete(command, deleted_at)
            .await
            .map_err(store_error)
    }

    pub async fn turn_receipt(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        operation_id: &OperationId,
    ) -> Result<TurnReceipt, AppError> {
        self.require_owner(owner, session_id).await?;
        self.store
            .read_turn_receipt(session_id, operation_id)
            .await
            .map_err(store_error)
    }

    async fn require_owner(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<AgentSessionLiveRecord, AppError> {
        let session = self
            .store
            .get_live_session(session_id)
            .await
            .map_err(store_error)?;
        if &session.owner_ref != owner {
            return Err(AppError::Forbidden(
                "AgentSession belongs to another owner".to_owned(),
            ));
        }
        Ok(session)
    }
}

fn scoped_key(owner: &PrincipalRef, key: &str, scope: &str) -> Result<String, AppError> {
    let key = key.trim();
    if key.is_empty() || key.len() > 256 || scope.trim().is_empty() {
        return Err(AppError::BadRequest(
            "idempotency key and scope must be canonical and bounded".to_owned(),
        ));
    }
    Ok(format!(
        "{}:{}:{scope}:{key}",
        owner.principal_kind, owner.principal_id
    ))
}

fn store_error(error: SessionStoreError) -> AppError {
    match error {
        SessionStoreError::NotFound(message) | SessionStoreError::Deleted(message) => {
            AppError::NotFound(message)
        }
        SessionStoreError::IdempotencyConflict(message)
        | SessionStoreError::Conflict(message) => AppError::Conflict(message),
        SessionStoreError::InvalidEvent(message)
        | SessionStoreError::InvalidPayload(message)
        | SessionStoreError::InvalidSession(message)
        | SessionStoreError::Registry(message) => AppError::BadRequest(message),
        other => AppError::Internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::{
        AgentPresetId, DigestHex, PresetRevisionRef, ResolvedSnapshotId, ResolvedSnapshotRef,
    };

    use super::*;

    fn owner() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".into(),
            principal_id: "owner-1".into(),
        }
    }

    fn binding() -> AgentBindingValue {
        AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from("preset-1"),
                revision: 1,
                revision_digest: DigestHex::from("a".repeat(64)),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot-1"),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        }
    }

    #[tokio::test]
    async fn exact_cancel_replay_and_late_delivery_cannot_cancel_a_successor_turn() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let service = CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let opened = service.open(owner(), binding(), None, Vec::new(), "cancel-isolation", 1).await.unwrap();
        let session = &opened.session.agent_session_id;
        let original = service.start_turn(&owner(), session, "original", json!({"content":"first"})).await.unwrap();
        let cancelled = service.cancel_exact_turn(&owner(), session, "stop-original", &original.operation_id).await.unwrap();
        let successor = service.start_turn(&owner(), session, "successor", json!({"content":"second"})).await.unwrap();
        let before = service.store.head(session).await.unwrap();

        let replay = service.cancel(&owner(), session, "stop-original").await.unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.target_operation_id, original.operation_id);
        assert_eq!(replay.event_id, cancelled.event_id);
        let delayed = service.cancel_exact_turn(&owner(), session, "late-stop-original", &original.operation_id).await.unwrap();
        assert!(delayed.duplicate);
        assert_eq!(delayed.event_id, cancelled.event_id);
        assert_eq!(service.store.head(session).await.unwrap(), before);
        assert_eq!(before.active_turn_id.as_deref(), Some(successor.operation_id.as_ref()));
        assert!(matches!(service.cancel_exact_turn(&owner(), session, "stop-original", &successor.operation_id).await,
            Err(AppError::Conflict(_))), "an idempotency replay cannot change its target");
        assert!(service.cancel_exact_turn(&owner(), session, "unknown-stop", &"missing-turn".into()).await.is_err());
        assert!(service.cancel_exact_turn(&owner(), session, "empty-stop", &"".into()).await.is_err());
        let foreign = PrincipalRef { principal_kind:"user".into(), principal_id:"foreign".into() };
        assert!(service.cancel_exact_turn(&foreign, session, "foreign-stop", &successor.operation_id).await.is_err());
        assert_eq!(service.store.head(session).await.unwrap(), before);
        assert_eq!(service.turn_receipt(&owner(), session, &successor.operation_id).await.unwrap().status,
            nomifun_agent_session::TurnReceiptStatus::Running);
    }

    #[tokio::test]
    async fn late_cancel_of_a_completed_turn_returns_its_fact_without_rewriting_completion() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let service = CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let opened = service.open(owner(), binding(), None, Vec::new(), "cancel-completed", 1).await.unwrap();
        let session = &opened.session.agent_session_id;
        let first = service.start_turn(&owner(), session, "completed-original", json!({"content":"first"})).await.unwrap();
        let started = service.turn_receipt(&owner(), session, &first.operation_id).await.unwrap().started_event.unwrap();
        let append = SessionEventAppend {
            agent_session_id:session.clone(), event_id:"completed-before-stop".into(), producer_id:"runtime_supervisor".into(),
            idempotency_key:"completed-before-stop".into(), semantic_event:SemanticSessionEventDraft {
                kind:SessionEventKind("turn/completed".into()),kind_version:1,correlation_id:first.operation_id.as_ref().into(),
                causation_event_id:Some(started.event_id),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"output":"done"}))),
            },
        };
        service.store.append_turn_terminal(&append, &first.operation_id).await.unwrap();
        let successor = service.start_turn(&owner(), session, "after-completion", json!({"content":"second"})).await.unwrap();
        let before = service.store.head(session).await.unwrap();
        let late = service.cancel_exact_turn(&owner(), session, "delayed-stop", &first.operation_id).await.unwrap();
        assert_eq!(late.target_operation_id, first.operation_id);
        assert_eq!(late.event_id, append.event_id);
        assert!(late.duplicate);
        assert_eq!(service.store.head(session).await.unwrap(), before);
        assert_eq!(before.active_turn_id.as_deref(), Some(successor.operation_id.as_ref()));
        assert_eq!(service.turn_receipt(&owner(), session, &first.operation_id).await.unwrap().status,
            nomifun_agent_session::TurnReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn exact_steer_replay_and_first_late_delivery_cannot_inject_a_successor_turn() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let service = CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let opened = service.open(owner(), binding(), None, Vec::new(), "steer-isolation", 1).await.unwrap();
        let session = &opened.session.agent_session_id;
        let original = service.start_turn(&owner(), session, "original", json!({"content":"first"})).await.unwrap();
        let input = json!({"content":"focus on this original task","files":[],"inject_skills":[]});
        let accepted = service.steer_exact_turn(&owner(), session, "original-steer", &original.operation_id, input.clone()).await.unwrap();
        let repeated = service.steer_exact_turn(&owner(), session, "original-steer", &original.operation_id, input.clone()).await.unwrap();
        assert!(repeated.duplicate);
        assert_eq!(repeated.event_id, accepted.event_id);
        service.cancel_exact_turn(&owner(), session, "original-stop", &original.operation_id).await.unwrap();
        let original_terminal = service.turn_receipt(&owner(), session, &original.operation_id).await.unwrap().terminal_event.unwrap();
        let successor = service.start_turn(&owner(), session, "successor", json!({"content":"second"})).await.unwrap();
        let before = service.store.head(session).await.unwrap();
        let replay = service.steer_exact_turn(&owner(), session, "original-steer", &original.operation_id, input.clone()).await.unwrap();
        assert_eq!(replay.event_id, accepted.event_id);
        assert!(replay.duplicate);
        let late = service.steer_exact_turn(&owner(), session, "first-late-steer", &original.operation_id, input.clone()).await.unwrap();
        assert_eq!(late.event_id, original_terminal.event_id);
        assert!(late.duplicate);
        assert!(service.steer_exact_turn(&owner(), session, "original-steer", &successor.operation_id, input.clone()).await.is_err());
        assert!(service.steer_exact_turn(&owner(), session, "original-steer", &original.operation_id, json!({"content":"changed"})).await.is_err());
        assert!(service.steer_exact_turn(&owner(), session, "missing-target", &"missing".into(), input).await.is_err());
        assert_eq!(service.store.head(session).await.unwrap(), before);
        assert_eq!(before.active_turn_id.as_deref(), Some(successor.operation_id.as_ref()));
        let count: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted'")
            .bind(session.as_ref()).fetch_one(database.pool()).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn fork_context_survives_parent_deletion_and_clear_context_drops_inherited_base() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner_service = CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let parent = owner_service.open(owner(), binding(), None, Vec::new(), "fork-parent", 1).await.unwrap();
        let turn = owner_service.start_turn(&owner(), &parent.session.agent_session_id,
            "parent-input", json!({"content":"keep accepted parent input"})).await.unwrap();
        owner_service.cancel(&owner(), &parent.session.agent_session_id, "cancel-parent").await.unwrap();
        let cursor = owner_service.store.current_cursor(&parent.session.agent_session_id).await.unwrap();
        let fork = owner_service.fork(&owner(), &parent.session.agent_session_id,
            cursor.seq, None, "fork-child", 2).await.unwrap();
        let command = match owner_service.fence_delete(&owner(), &parent.session.agent_session_id,
            "delete-parent", 3).await.unwrap() {
            PreparedAgentSessionDelete::Fenced(command) => command,
            PreparedAgentSessionDelete::AlreadyDeleted(_) => panic!("parent unexpectedly deleted"),
        };
        owner_service.complete_fenced_delete(&command, 3).await.unwrap();
        let child_id = &fork.child_session.agent_session_id;
        let facts = owner_service.store.chat_causality_facts(child_id, &turn.operation_id).await.unwrap();
        let base = facts.fork_context.unwrap();
        assert_eq!(base.parent_agent_session_id, parent.session.agent_session_id);
        assert_eq!(base.messages[0].content, "keep accepted parent input");
        assert_eq!(base.source_binding, AgentHandoffBindingRefV1::from(&binding()));
        let child_cursor = owner_service.store.current_cursor(child_id).await.unwrap();
        let grandchild = owner_service.fork(&owner(), child_id, child_cursor.seq, None, "fork-grandchild", 4).await.unwrap();
        let grandchild_facts = owner_service.store.chat_causality_facts(&grandchild.child_session.agent_session_id,
            &turn.operation_id).await.unwrap();
        assert_eq!(grandchild_facts.fork_context.unwrap().messages[0].content, "keep accepted parent input");
        let ready = facts.events.iter().find(|event| event.kind.0 == "session/ready").unwrap();
        owner_service.store.append_event(&SessionEventAppend {
            agent_session_id: child_id.clone(), event_id: "clear-child".into(), producer_id: "session_api".into(),
            idempotency_key: "clear-child".into(), semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("context/cleared".into()), kind_version: 1,
                correlation_id: child_id.as_ref().into(), causation_event_id: Some(ready.event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"clear-child"}))),
            },
        }).await.unwrap();
        let cleared_cursor = owner_service.store.current_cursor(child_id).await.unwrap();
        let cleared_fork = owner_service.fork(&owner(), child_id, cleared_cursor.seq, None, "fork-cleared-child", 5).await.unwrap();
        let cleared_facts = owner_service.store.chat_causality_facts(&cleared_fork.child_session.agent_session_id,
            &turn.operation_id).await.unwrap();
        assert!(cleared_facts.fork_context.unwrap().messages.is_empty());
    }

    #[tokio::test]
    async fn fork_replay_uses_the_first_receipt_after_binding_changes_rename_and_parent_deletion() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner_service = CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let parent = owner_service.open(owner(), binding(), None, Vec::new(), "receipt-parent", 1).await.unwrap();
        let parent_id = &parent.session.agent_session_id;
        let cursor = owner_service.store.current_cursor(parent_id).await.unwrap();
        let first = owner_service.fork(&owner(), parent_id, cursor.seq,
            Some("First title".into()), "stable-fork", 2).await.unwrap();
        owner_service.store.update_session_metadata(&owner(), &first.child_session.agent_session_id,
            nomifun_agent_session::UpdateAgentSessionMetadata {
                title: Some("Renamed child".into()), archived: None, pinned: None,
            }).await.unwrap();
        let mut next_binding = binding();
        next_binding.binding_version += 1;
        next_binding.preset_revision_ref.preset_id = "next-preset".into();
        next_binding.resolved_snapshot_ref.snapshot_id = "next-snapshot".into();
        owner_service.store.replace_session_agent_binding(&owner(), parent_id,
            nomifun_agent_session::ReplaceSessionAgentBinding {
                expected: binding(), replacement: next_binding,
                previous_agent_label: "First Agent".into(), next_agent_label: "Next Agent".into(),
                transition_id: Uuid::now_v7().to_string().into(), request_digest: "c".repeat(64).into(),
                idempotency_key: "switch-parent-agent".into(), handoff_mode: nomifun_agent_contracts::AgentHandoffMode::ContextOnly,
                handoff: None, initial_active_capability_ids: Vec::new(),
            }).await.unwrap();
        let replay = owner_service.fork(&owner(), parent_id, cursor.seq,
            Some("First title".into()), "stable-fork", 999).await.unwrap();
        assert_eq!(replay.child_session.agent_session_id, first.child_session.agent_session_id);
        assert_eq!(replay.child_session.metadata.title.as_deref(), Some("Renamed child"));
        assert_eq!(replay.contract, first.contract);
        assert_eq!(replay.fork_ack, first.fork_ack);
        assert_eq!(replay.child_cursor, first.child_cursor);
        assert!(matches!(owner_service.fork(&owner(), parent_id, cursor.seq + 1,
            Some("First title".into()), "stable-fork", 4).await, Err(AppError::Conflict(_))));
        assert!(matches!(owner_service.fork(&owner(), parent_id, cursor.seq,
            Some("Changed title".into()), "stable-fork", 4).await, Err(AppError::Conflict(_))));
        let foreign = PrincipalRef { principal_kind: "user".into(), principal_id: "another-owner".into() };
        let key = IdempotencyKey::from(format!("{}:fork", scoped_key(&owner(), "stable-fork", parent_id.as_ref()).unwrap()));
        assert!(owner_service.store.existing_fork_receipt(&foreign, parent_id, cursor.seq,
            Some("First title"), &EventProducerId::from("session_api"), &key).await.is_err());
        assert!(matches!(owner_service.fork(&foreign, parent_id, cursor.seq,
            Some("First title".into()), "stable-fork", 4).await, Err(AppError::Forbidden(_))));
        let command = match owner_service.fence_delete(&owner(), parent_id, "delete-receipt-parent", 5).await.unwrap() {
            PreparedAgentSessionDelete::Fenced(command) => command,
            PreparedAgentSessionDelete::AlreadyDeleted(_) => panic!("parent unexpectedly deleted"),
        };
        owner_service.complete_fenced_delete(&command, 5).await.unwrap();
        let replay = owner_service.fork(&owner(), parent_id, cursor.seq,
            Some("First title".into()), "stable-fork", 6).await.unwrap();
        assert_eq!(replay.child_session.agent_session_id, first.child_session.agent_session_id);
        assert_eq!(replay.contract, first.contract);
        assert_eq!(replay.fork_ack, first.fork_ack);
        assert_eq!(replay.child_cursor, first.child_cursor);
    }

    #[tokio::test]
    async fn one_owner_covers_open_turn_steer_cancel_fork_delete_and_replay() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner_service = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
            .await
            .unwrap();
        let opened = owner_service
            .open(
                owner(),
                binding(),
                Some("Canonical".into()),
                vec!["workspace.files".into()],
                "open-1",
                1,
            )
            .await
            .unwrap();
        let replay = owner_service
            .open(
                owner(),
                binding(),
                Some("Canonical".into()),
                vec!["workspace.files".into()],
                "open-1",
                1,
            )
            .await
            .unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.session.agent_session_id, opened.session.agent_session_id);
        assert!(matches!(
            owner_service
                .open(
                    owner(),
                    binding(),
                    Some("Canonical".into()),
                    vec!["different.capability".into()],
                    "open-1",
                    1,
                )
                .await,
            Err(AppError::Conflict(_))
        ));

        let turn = owner_service
            .start_turn(
                &owner(),
                &opened.session.agent_session_id,
                "turn-1",
                json!({"content":"hello"}),
            )
            .await
            .unwrap();
        let replayed_turn = owner_service
            .start_turn(
                &owner(),
                &opened.session.agent_session_id,
                "turn-1",
                json!({"content":"hello"}),
            )
            .await
            .unwrap();
        assert!(replayed_turn.duplicate);
        assert_eq!(replayed_turn.operation_id, turn.operation_id);
        assert!(matches!(
            owner_service
                .start_turn(
                    &owner(),
                    &opened.session.agent_session_id,
                    "turn-2",
                    json!({"content":"must wait"}),
                )
                .await,
            Err(AppError::Conflict(_))
        ));
        let open_while_running = owner_service
            .open(
                owner(),
                binding(),
                Some("Canonical".into()),
                vec!["workspace.files".into()],
                "open-1",
                1,
            )
            .await
            .unwrap();
        assert!(open_while_running.duplicate);
        assert_eq!(open_while_running.cursor, opened.cursor);

        let steer = owner_service
            .steer(
                &owner(),
                &opened.session.agent_session_id,
                "steer-1",
                json!({"content":"add tests"}),
            )
            .await
            .unwrap();
        assert_eq!(steer.target_operation_id, turn.operation_id);
        assert!(owner_service
            .steer(
                &owner(),
                &opened.session.agent_session_id,
                "steer-1",
                json!({"content":"add tests"}),
            )
            .await
            .unwrap()
            .duplicate);
        assert!(matches!(
            owner_service
                .steer(
                    &owner(),
                    &opened.session.agent_session_id,
                    "steer-1",
                    json!({"content":"changed"}),
                )
                .await,
            Err(AppError::Conflict(_))
        ));
        let cancel = owner_service
            .cancel(
                &owner(),
                &opened.session.agent_session_id,
                "cancel-1",
            )
            .await
            .unwrap();
        assert_eq!(cancel.target_operation_id, turn.operation_id);
        assert!(owner_service
            .cancel(
                &owner(),
                &opened.session.agent_session_id,
                "cancel-1",
            )
            .await
            .unwrap()
            .duplicate);
        let terminal_turn_replay = owner_service
            .start_turn(
                &owner(),
                &opened.session.agent_session_id,
                "turn-1",
                json!({"content":"hello"}),
            )
            .await
            .unwrap();
        assert!(terminal_turn_replay.duplicate);
        assert_eq!(terminal_turn_replay.operation_id, turn.operation_id);
        assert!(matches!(
            owner_service
                .start_turn(
                    &owner(),
                    &opened.session.agent_session_id,
                    "turn-1",
                    json!({"content":"changed"}),
                )
                .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            owner_service
                .turn_receipt(&owner(), &opened.session.agent_session_id, &turn.operation_id)
                .await
                .unwrap()
                .status,
            nomifun_agent_session::TurnReceiptStatus::Cancelled
        ));

        let cursor = owner_service
            .store()
            .current_cursor(&opened.session.agent_session_id)
            .await
            .unwrap();
        let forked = owner_service
            .fork(
                &owner(),
                &opened.session.agent_session_id,
                cursor.seq,
                Some("Fork".into()),
                "fork-1",
                2,
            )
            .await
            .unwrap();
        let replayed_fork = owner_service
            .fork(
                &owner(),
                &opened.session.agent_session_id,
                cursor.seq,
                Some("Fork".into()),
                "fork-1",
                2,
            )
            .await
            .unwrap();
        assert_eq!(
            replayed_fork.child_session.agent_session_id,
            forked.child_session.agent_session_id
        );
        assert_eq!(
            owner_service
                .active_capability_ids(&owner(), &forked.child_session.agent_session_id)
                .await
                .unwrap(),
            vec!["workspace.files"]
        );
        assert!(matches!(
            owner_service
                .fork(
                    &owner(),
                    &opened.session.agent_session_id,
                    cursor.seq,
                    Some("Changed fork".into()),
                    "fork-1",
                    2,
                )
                .await,
            Err(AppError::Conflict(_))
        ));
        let child_turn = owner_service
            .start_turn(
                &owner(),
                &forked.child_session.agent_session_id,
                "child-turn-1",
                json!({"content":"continue from fork"}),
            )
            .await
            .unwrap();
        owner_service
            .cancel(
                &owner(),
                &forked.child_session.agent_session_id,
                "child-cancel-1",
            )
            .await
            .unwrap();
        assert!(matches!(
            owner_service
                .turn_receipt(
                    &owner(),
                    &forked.child_session.agent_session_id,
                    &child_turn.operation_id,
                )
                .await
                .unwrap()
                .status,
            nomifun_agent_session::TurnReceiptStatus::Cancelled
        ));

        owner_service
            .store()
            .rebuild_projections(&opened.session.agent_session_id)
            .await
            .unwrap();
        let messages = owner_service
            .messages(&owner(), &opened.session.agent_session_id, 0)
            .await
            .unwrap();
        assert!(
            messages
                .iter()
                .any(|message| message.projection["content"] == "hello")
        );
        let command = match owner_service
            .fence_delete(
                &owner(),
                &forked.child_session.agent_session_id,
                "delete-1",
                3,
            )
            .await
            .unwrap()
        {
            PreparedAgentSessionDelete::Fenced(command) => command,
            PreparedAgentSessionDelete::AlreadyDeleted(_) => {
                panic!("first delete unexpectedly replayed a tombstone")
            }
        };
        let deleted = owner_service
            .complete_fenced_delete(&command, 3)
            .await
            .unwrap();
        assert_eq!(deleted.tombstone.state, nomifun_agent_contracts::AgentSessionDeletedState::Deleted);
        let replayed_delete = match owner_service
            .fence_delete(
                &owner(),
                &forked.child_session.agent_session_id,
                "delete-1",
                3,
            )
            .await
            .unwrap()
        {
            PreparedAgentSessionDelete::AlreadyDeleted(deleted) => deleted,
            PreparedAgentSessionDelete::Fenced(_) => {
                panic!("deleted Session unexpectedly re-entered cleanup")
            }
        };
        assert_eq!(replayed_delete.tombstone, deleted.tombstone);
        assert_eq!(replayed_delete.tombstone.deleted_at, 3);
        assert!(owner_service
            .get(&owner(), &opened.session.agent_session_id)
            .await
            .is_ok());
    }
}
