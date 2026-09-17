use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, AgentSessionLiveRecord, AgentSessionMetadata, ArtifactId,
    CorrelationId, DeleteAgentSessionCommand, EventId, EventProducerId, IdempotencyKey,
    OperationId, PrincipalRef, SemanticSessionEventDraft, SessionEventAppend, SessionEventCursor,
    SessionEventKind, SessionEventPayloadRef, SessionPayloadBody, StrictJsonValue,
};
use nomifun_agent_session::{
    AgentSessionStore, CreateSessionRequest, DeleteResult, ForkRequest, ForkResult,
    MessageProjection, SessionEventPage, SessionObservation, SessionStoreError, TurnReceipt,
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
            },
            agent_binding: binding,
            remote_binding_provenance: None,
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
            runtime_binding_id: None,
            runtime_producer_seq: None,
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
        self.require_owner(owner, session_id).await?;
        let key = scoped_key(owner, idempotency_key, session_id.as_ref())?;
        let operation_id = OperationId::from(format!("turn:{key}"));
        let (_, turn_result) = self
            .store
            .start_turn(
                session_id,
                EventProducerId::from("session_api"),
                IdempotencyKey::from(key),
                operation_id.clone(),
                StrictJsonValue(input),
            )
            .await
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
        Ok(AgentMutationReceipt {
            target_operation_id,
            cursor: result.cursor,
            duplicate: result.duplicate,
        })
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
        Ok(AgentMutationReceipt {
            target_operation_id,
            cursor: result.cursor,
            duplicate: result.duplicate,
        })
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
        let parent = self.require_owner(owner, parent_session_id).await?;
        let key = scoped_key(owner, idempotency_key, parent_session_id.as_ref())?;
        let producer = EventProducerId::from("session_api");
        let event_key = IdempotencyKey::from(format!("{key}:fork"));
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
            base_body: SessionPayloadBody::Json(StrictJsonValue(json!({
                "parent_agent_session_id": parent_session_id,
                "parent_through_seq": parent_through_seq,
            }))),
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
        SessionStoreError::NotFound(message) => AppError::NotFound(message),
        SessionStoreError::Deleted(message)
        | SessionStoreError::IdempotencyConflict(message)
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
                json!({"text":"add tests"}),
            )
            .await
            .unwrap();
        assert_eq!(steer.target_operation_id, turn.operation_id);
        assert!(owner_service
            .steer(
                &owner(),
                &opened.session.agent_session_id,
                "steer-1",
                json!({"text":"add tests"}),
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
