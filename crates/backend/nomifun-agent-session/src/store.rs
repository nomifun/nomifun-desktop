use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
#[cfg(test)]
use nomifun_agent_contracts::FRESH_V4_BASELINE_SQL;
use nomifun_agent_contracts::{
    AgentSessionDeletedState, AgentSessionDeletingRecord, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionTombstone, ArtifactId, ChatRouteIdentity, CompactionCompletedPayload, CorrelationId,
    DeleteAgentSessionCommand, DigestHex, EventId, EventProducerId, FRESH_V4_DATA_GENERATION,
    FRESH_V4_MIGRATION_HEAD, FRESH_V4_PROJECTION_SCHEMA_VERSION, IdempotencyKey, PrincipalRef,
    OperationId, RemoteBindingId, RuntimeBindingId, RuntimeCheckpointValidationInput,
    RuntimeCheckpointValidationResult, RuntimeEventAck, SessionEventAck, SessionEventAppend,
    SessionEventCursor, SessionEventKind, SessionEventPayloadRef, SessionEventPredecessorMode,
    SessionEventRecord, SessionForkContract, SessionForkPayload, SessionPayloadBody,
    SessionPayloadId, SessionPayloadRecord, SnapshotCompatibilityAdmissionInput,
    SnapshotCompatibilityAdmissionResult, StrictJsonValue, VersionString, canonical_json_bytes,
    digest_bytes, digest_payload, fresh_v4_schema_manifest_payload,
};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::checkpoint::{evaluate_snapshot_compatibility, validate_checkpoint};
use crate::error::SessionStoreError;
use crate::projector::{initial_head, payload_value, reduce_head, reduce_message_projection};
use crate::registry::EventRegistry;
use crate::types::{
    CheckpointAdmission, CreateSessionRequest, DeleteResult, EffectEventRequest,
    EffectReconcileOutcome, EffectTerminalState, ForkRequest, ForkResult, MessageProjection,
    ChatCausalityFacts, RuntimeAppendContext, RuntimeEventAppendResult, SessionCreateResult,
    ChatOperationClaimRequest, SessionEventAppendResult, SessionEventPage,
    SessionHeadProjection, SessionObservation, SessionRehydrationInput,
    ZeroOutstandingProof,
};

pub const MAX_INLINE_JSON_BYTES: usize = 64 * 1024;
pub const MAX_SINGLE_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_SESSION_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_EVENT_PAGE_SIZE: u32 = 500;

const SESSION_TABLES: [&str; 5] = [
    "agent_sessions",
    "message_projection",
    "session_events",
    "session_heads",
    "session_payloads",
];

const SESSION_COLUMNS: &[(&str, &[&str])] = &[
    (
        "agent_sessions",
        &[
            "agent_session_id",
            "owner_ref_json",
            "state",
            "title",
            "archived",
            "pinned",
            "agent_binding_json",
            "remote_binding_id",
            "remote_binding_version",
            "parent_agent_session_id",
            "fork_base_payload_id",
            "next_seq",
            "created_at",
            "deleted_at",
        ],
    ),
    (
        "session_events",
        &[
            "session_id",
            "seq",
            "event_id",
            "producer_id",
            "idempotency_key",
            "runtime_binding_id",
            "runtime_producer_seq",
            "kind",
            "kind_version",
            "correlation_id",
            "causation_event_id",
            "inline_json",
            "payload_id",
        ],
    ),
    (
        "session_payloads",
        &[
            "payload_id",
            "session_id",
            "media_type",
            "byte_len",
            "digest",
            "body",
        ],
    ),
    (
        "session_heads",
        &[
            "session_id",
            "status",
            "active_turn_id",
            "active_set_generation",
            "runtime_checkpoint_locator",
            "runtime_checkpoint_digest",
            "runtime_bound_event_id",
            "runtime_protocol_version",
            "snapshot_digest",
            "checkpoint_through_seq",
            "last_seq",
            "unread_count",
        ],
    ),
    (
        "message_projection",
        &[
            "session_id",
            "projection_id",
            "first_seq",
            "last_seq",
            "presentation_intent",
            "projection_json",
            "semantic_digest",
        ],
    ),
];

const SESSION_INDEXES: [&str; 4] = [
    "idx_agent_sessions_owner_state",
    "idx_message_projection_sequence",
    "idx_session_events_correlation",
    "idx_session_payloads_session",
];

#[derive(Clone, Debug)]
pub struct AgentSessionStore {
    pool: SqlitePool,
    registry: EventRegistry,
}

impl AgentSessionStore {
    pub async fn connect_existing(path: impl AsRef<Path>) -> Result<Self, SessionStoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        Self::from_pool(pool).await
    }

    pub async fn from_pool(pool: SqlitePool) -> Result<Self, SessionStoreError> {
        validate_fresh_v4_schema(&pool).await?;
        Ok(Self {
            pool,
            registry: EventRegistry::canonical()?,
        })
    }

    #[cfg(test)]
    pub(crate) async fn open_in_memory() -> Result<Self, SessionStoreError> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Memory)
            .synchronous(SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(FRESH_V4_BASELINE_SQL).execute(&pool).await?;
        seed_test_schema_metadata(&pool).await?;
        Self::from_pool(pool).await
    }

    pub fn event_registry(&self) -> &nomifun_agent_contracts::SessionEventRegistryPayload {
        self.registry.payload()
    }

    #[cfg(test)]
    pub(crate) fn test_pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<SessionCreateResult, SessionStoreError> {
        validate_live_session(&request.session)?;
        if request.created_at < 0 {
            return Err(SessionStoreError::InvalidSession(
                "created_at must not be negative".to_owned(),
            ));
        }
        if request.session.next_seq != 1 {
            return Err(SessionStoreError::InvalidSession(
                "new AgentSession must start with next_seq=1".to_owned(),
            ));
        }

        let opening_payload = opening_payload(&request)?;
        let mut tx = self.pool.begin().await?;
        if let Some(existing) = event_by_producer_key_tx(
            &mut tx,
            request.producer_id.as_ref(),
            request.idempotency_key.as_ref(),
        )
        .await?
        {
            return replay_create(&mut tx, &request, opening_payload, existing).await;
        }

        insert_live_session_tx(&mut tx, &request.session, request.created_at).await?;
        insert_head_tx(&mut tx, &initial_head(&request.session.agent_session_id)).await?;

        let opening = SessionEventAppend {
            agent_session_id: request.session.agent_session_id.clone(),
            event_id: request
                .opening_event_id
                .clone()
                .unwrap_or_else(new_event_id),
            producer_id: request.producer_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("session/opening".to_owned()),
                kind_version: 1,
                correlation_id: request.correlation_id.clone(),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(opening_payload)),
            },
        };
        let opening_result = self.append_event_tx(&mut tx, &opening, None).await?;
        let opening_ack = required_ack(opening_result)?;

        let mut active_ids = request.initial_active_capability_ids.clone();
        active_ids.sort();
        active_ids.dedup();
        let active_set_digest = digest_payload(&active_ids)?.0;
        let activation = SessionEventAppend {
            agent_session_id: request.session.agent_session_id.clone(),
            event_id: request
                .activation_event_id
                .clone()
                .unwrap_or_else(new_event_id),
            producer_id: EventProducerId(format!(
                "{}:capability-host",
                request.producer_id.as_ref()
            )),
            idempotency_key: IdempotencyKey(format!(
                "{}:active-set-0",
                request.idempotency_key.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("capability/active-set-committed".to_owned()),
                kind_version: 1,
                correlation_id: request.correlation_id,
                causation_event_id: Some(opening.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "generation": 0,
                    "active_capability_ids": active_ids,
                    "active_set_digest": active_set_digest,
                    "delta": []
                }))),
            },
        };
        let activation_result = self.append_event_tx(&mut tx, &activation, None).await?;
        let activation_ack = required_ack(activation_result)?;
        let session =
            live_session_by_id_tx(&mut tx, request.session.agent_session_id.as_ref()).await?;
        tx.commit().await?;

        Ok(SessionCreateResult {
            session,
            opening_ack,
            activation_ack,
            duplicate: false,
        })
    }

    pub async fn append_event(
        &self,
        append: &SessionEventAppend,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        self.append_event_with_payload(append, None).await
    }

    pub async fn append_event_with_payload(
        &self,
        append: &SessionEventAppend,
        payload: Option<&SessionPayloadRecord>,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let result = self.append_event_tx(&mut tx, append, payload).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Append a Chat message terminal and its turn terminal under one
    /// active-turn fence. A cancel/terminal event cannot be committed between
    /// the two semantic records, and a failed second append rolls the first
    /// append back with the same SQLite transaction.
    pub async fn append_chat_completion(
        &self,
        message: &SessionEventAppend,
        turn: &SessionEventAppend,
        turn_operation_id: &OperationId,
    ) -> Result<(SessionEventAppendResult, SessionEventAppendResult), SessionStoreError> {
        if message.semantic_event.kind.0 != "message/completed"
            || !matches!(
                turn.semantic_event.kind.0.as_str(),
                "turn/completed" | "turn/failed"
            )
            || turn.semantic_event.correlation_id.as_ref() != turn_operation_id.as_ref()
        {
            return Err(SessionStoreError::InvalidEvent(
                "chat completion append has an invalid terminal event shape".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, message.agent_session_id.as_ref()).await?;
        if message.agent_session_id != turn.agent_session_id {
            return Err(SessionStoreError::InvalidEvent(
                "chat completion events belong to different AgentSessions".to_owned(),
            ));
        }
        let message_duplicate = duplicate_event_tx(&mut tx, message).await?.is_some();
        let turn_duplicate = duplicate_event_tx(&mut tx, turn).await?.is_some();
        if !(message_duplicate && turn_duplicate) {
            ensure_active_turn_tx(
                &mut tx,
                message.agent_session_id.as_ref(),
                turn_operation_id,
            )
            .await?;
        }
        let message_result = self.append_event_tx(&mut tx, message, None).await?;
        let turn_result = self.append_event_tx(&mut tx, turn, None).await?;
        tx.commit().await?;
        Ok((message_result, turn_result))
    }

    /// Append one turn terminal while the exact operation is still active.
    /// Duplicate replays are accepted by the normal event idempotency path,
    /// but a new terminal cannot cross a committed cancel/terminal fence.
    pub async fn append_turn_terminal(
        &self,
        append: &SessionEventAppend,
        turn_operation_id: &OperationId,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        if !matches!(
            append.semantic_event.kind.0.as_str(),
            "turn/completed" | "turn/failed"
        ) || append.semantic_event.correlation_id.as_ref() != turn_operation_id.as_ref()
        {
            return Err(SessionStoreError::InvalidEvent(
                "turn terminal append has an invalid event shape".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, append.agent_session_id.as_ref()).await?;
        let duplicate = duplicate_event_tx(&mut tx, append).await?;
        if duplicate.is_none() {
            ensure_active_turn_tx(
                &mut tx,
                append.agent_session_id.as_ref(),
                turn_operation_id,
            )
            .await?;
        }
        let result = self.append_event_tx(&mut tx, append, None).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Atomically select the current active turn and append its cancellation
    /// event. The caller receives the exact operation id that was fenced.
    ///
    /// Reading `active_turn_id` outside this transaction would allow a
    /// concurrent terminal event to change the target between the read and the
    /// append. Replays use the producer/idempotency key and return the original
    /// target without creating a second event.
    pub async fn cancel_active_turn(
        &self,
        session_id: &AgentSessionId,
        idempotency_key: IdempotencyKey,
        producer_id: EventProducerId,
    ) -> Result<(OperationId, SessionEventAppendResult), SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;

        if let Some(existing) = event_by_producer_key_tx(
            &mut tx,
            producer_id.as_ref(),
            idempotency_key.as_ref(),
        )
        .await?
        {
            let record = event_from_row(existing)?;
            if record.agent_session_id != *session_id
                || record.kind.0 != "turn/cancelled"
            {
                return Err(SessionStoreError::IdempotencyConflict(
                    "cancellation idempotency key was already used for another event".to_owned(),
                ));
            }
            let target = match &record.payload {
                SessionEventPayloadRef::InlineJson(value) => value
                    .0
                    .get("target_operation_id")
                    .and_then(Value::as_str)
                    .map(|value| OperationId::from(value.to_owned()))
                    .ok_or_else(|| {
                        SessionStoreError::InvalidEvent(
                            "turn/cancelled replay has no target_operation_id".to_owned(),
                        )
                    })?,
                _ => {
                    return Err(SessionStoreError::InvalidEvent(
                        "turn/cancelled replay must retain inline provenance".to_owned(),
                    ));
                }
            };
            let ack = event_ack(&record);
            tx.commit().await?;
            return Ok((
                target,
                SessionEventAppendResult {
                    record: Some(record),
                    ack: Some(ack.clone()),
                    cursor: ack.cursor,
                    persisted: true,
                    duplicate: true,
                },
            ));
        }

        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let target_operation_id = head
            .active_turn_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .map(OperationId::from)
            .ok_or_else(|| {
                SessionStoreError::Conflict(
                    "Remote cancellation requires an active turn".to_owned(),
                )
            })?;
        let turn_event = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM session_events \
             WHERE session_id = ? AND kind = 'turn/started' AND correlation_id = ? \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(session_id.as_ref())
        .bind(target_operation_id.as_ref())
        .fetch_optional(&mut *tx)
        .await?;
        let turn_event = turn_event.ok_or_else(|| {
            SessionStoreError::Conflict(
                "active turn has no committed turn/started event".to_owned(),
            )
        })?;
        let turn_event = event_from_row(turn_event)?;
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(format!(
                "turn-cancelled:{}:{}",
                session_id.as_ref(),
                idempotency_key.as_ref()
            )),
            producer_id,
            idempotency_key,
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("turn/cancelled".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(target_operation_id.as_ref().to_owned()),
                causation_event_id: Some(turn_event.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "target_operation_id": target_operation_id
                }))),
            },
        };
        let result = self.append_event_tx(&mut tx, &append, None).await?;
        tx.commit().await?;
        Ok((target_operation_id, result))
    }

    /// Converge an opening Session to `open_failed` without racing a runtime
    /// ready event. The head-state check and event append share one SQLite
    /// transaction; if another opener already committed `ready`, this returns
    /// `None` and leaves the successful opening untouched.
    pub async fn append_open_failed(
        &self,
        session_id: &AgentSessionId,
        code: &str,
        message: &str,
        recoverable: bool,
    ) -> Result<Option<SessionEventAppendResult>, SessionStoreError> {
        if code.trim().is_empty() || code.trim() != code {
            return Err(SessionStoreError::InvalidEvent(
                "open failure code must be canonical and non-empty".to_owned(),
            ));
        }
        if message.trim().is_empty() || message.trim() != message {
            return Err(SessionStoreError::InvalidEvent(
                "open failure message must be canonical and non-empty".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let idempotency_key =
            IdempotencyKey::from(format!("session-open-failed:{}", session_id.as_ref()));
        let producer_id = EventProducerId::from("runtime_supervisor");
        if let Some(existing) =
            event_by_producer_key_tx(&mut tx, producer_id.as_ref(), idempotency_key.as_ref())
                .await?
        {
            let record = event_from_row(existing)?;
            let ack = event_ack(&record);
            tx.commit().await?;
            return Ok(Some(SessionEventAppendResult {
                record: Some(record),
                ack: Some(ack.clone()),
                cursor: ack.cursor,
                persisted: true,
                duplicate: true,
            }));
        }

        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if head.status != "opening" {
            tx.commit().await?;
            return Ok(None);
        }
        let opening = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM session_events \
             WHERE session_id = ? AND kind = 'session/opening' \
             ORDER BY seq ASC LIMIT 1",
        )
        .bind(session_id.as_ref())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            SessionStoreError::Conflict(
                "opening Session has no committed session/opening event".to_owned(),
            )
        })?;
        let opening = event_from_row(opening)?;
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(format!("session-open-failed:{}", session_id.as_ref())),
            producer_id,
            idempotency_key,
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("session/open-failed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: Some(opening.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "code": code,
                    "message": message,
                    "recoverable": recoverable
                }))),
            },
        };
        let result = self.append_event_tx(&mut tx, &append, None).await?;
        tx.commit().await?;
        Ok(Some(result))
    }

    pub async fn append_runtime_event(
        &self,
        context: RuntimeAppendContext,
    ) -> Result<RuntimeEventAppendResult, SessionStoreError> {
        if self.registry.is_transient(
            &context.envelope.semantic_event.kind,
            context.envelope.semantic_event.kind_version,
        )? {
            return Err(SessionStoreError::InvalidEvent(
                "RuntimeEventEnvelope cannot carry a transient diagnostic".to_owned(),
            ));
        }

        let runtime_binding_id = context.envelope.runtime_binding_id.clone();
        let producer_seq = context.envelope.producer_seq;
        let append = runtime_event_append(context)?;
        let result = self.append_event(&append).await?;
        let ack = result.ack.clone().map(|session_event_ack| RuntimeEventAck {
            runtime_binding_id,
            committed_producer_seq: producer_seq,
            session_event_ack,
        });
        Ok(RuntimeEventAppendResult {
            append: result,
            ack,
        })
    }

    /// Commit the Runtime admission boundary as one SessionStore transaction.
    ///
    /// `runtime/bound` and `session/ready` are a single durable transition:
    /// an open failure committed first must prevent the ready event, and a
    /// ready transition must not expose a half-written Runtime binding.
    pub async fn append_runtime_bound_and_ready(
        &self,
        context: RuntimeAppendContext,
        ready: &SessionEventAppend,
    ) -> Result<(), SessionStoreError> {
        if self.registry.is_transient(
            &context.envelope.semantic_event.kind,
            context.envelope.semantic_event.kind_version,
        )? {
            return Err(SessionStoreError::InvalidEvent(
                "RuntimeEventEnvelope cannot carry a transient diagnostic".to_owned(),
            ));
        }
        if context.envelope.semantic_event.kind.0 != "runtime/bound"
            || ready.agent_session_id != context.agent_session_id
            || ready.semantic_event.kind.0 != "session/ready"
            || ready.semantic_event.causation_event_id.as_ref()
                != Some(&context.envelope.event_id)
        {
            return Err(SessionStoreError::InvalidEvent(
                "Runtime admission boundary has an invalid bound/ready event shape".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, context.agent_session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, context.agent_session_id.as_ref()).await?;
        if head.status != "opening" {
            return Err(SessionStoreError::Conflict(format!(
                "Runtime admission requires an opening Session, found {}",
                head.status
            )));
        }
        let bound = runtime_event_append(context)?;
        self.append_event_tx(&mut tx, &bound, None).await?;
        self.append_event_tx(&mut tx, ready, None).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_live_session(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<AgentSessionLiveRecord, SessionStoreError> {
        let row = session_row_by_id(&self.pool, session_id.as_ref()).await?;
        require_live_row(row)
    }

    /// Return live Remote Sessions whose post-commit Runtime admission has
    /// not reached a terminal state. The result is used only by the host
    /// startup recovery coordinator; it never reconstructs a Session or
    /// changes its frozen binding.
    pub async fn list_opening_remote_sessions(
        &self,
    ) -> Result<Vec<AgentSessionId>, SessionStoreError> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT sessions.agent_session_id \
             FROM agent_sessions AS sessions \
             INNER JOIN session_heads AS heads \
               ON heads.session_id = sessions.agent_session_id \
             WHERE sessions.state = 'live' \
               AND sessions.remote_binding_id IS NOT NULL \
               AND heads.status = 'opening' \
             ORDER BY sessions.created_at ASC, sessions.agent_session_id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|session_id| {
                validate_uuidv7(&session_id, "agent_session_id")?;
                Ok(AgentSessionId::from(session_id))
            })
            .collect()
    }

    pub async fn inspect_tombstone(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Option<AgentSessionTombstone>, SessionStoreError> {
        let Some(row) = optional_session_row_by_id(&self.pool, session_id.as_ref()).await? else {
            return Ok(None);
        };
        if row.state != "deleted" {
            return Ok(None);
        }
        Ok(Some(tombstone_from_row(row)?))
    }

    pub async fn current_cursor(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionEventCursor, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let last_seq: i64 =
            sqlx::query_scalar("SELECT last_seq FROM session_heads WHERE session_id = ?")
                .bind(session_id.as_ref())
                .fetch_one(&self.pool)
                .await?;
        Ok(SessionEventCursor {
            agent_session_id: session_id.clone(),
            seq: as_u64(last_seq, "last_seq")?,
        })
    }

    pub async fn read_events(
        &self,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionEventPage, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let after_seq = validate_cursor(session_id, after)?;
        let current_last_seq: i64 =
            sqlx::query_scalar("SELECT last_seq FROM session_heads WHERE session_id = ?")
                .bind(session_id.as_ref())
                .fetch_one(&self.pool)
                .await?;
        if after_seq > as_u64(current_last_seq, "last_seq")? {
            return Err(SessionStoreError::InvalidEvent(
                "cursor is ahead of the committed AgentSession sequence".to_owned(),
            ));
        }
        let limit = limit.clamp(1, MAX_EVENT_PAGE_SIZE);
        let rows = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM session_events \
             WHERE session_id = ? AND seq > ? \
             ORDER BY seq ASC LIMIT ?",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(after_seq, "cursor")?)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let events = rows
            .into_iter()
            .map(event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_seq = events.last().map_or(after_seq, |event| event.seq);
        Ok(SessionEventPage {
            agent_session_id: session_id.clone(),
            events,
            next_cursor: SessionEventCursor {
                agent_session_id: session_id.clone(),
                seq: next_seq,
            },
        })
    }

    pub async fn observe(
        &self,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionObservation, SessionStoreError> {
        let session = self.get_live_session(session_id).await?;
        let head = self.head(session_id).await?;
        let page = self.read_events(session_id, after, limit).await?;
        let after_seq = validate_cursor(session_id, after)?;
        let messages = self
            .message_projections_after(session_id, after_seq)
            .await?;
        Ok(SessionObservation {
            session,
            head,
            events: page.events,
            messages,
            next_cursor: page.next_cursor,
        })
    }

    /// Read all committed facts used by the production Chat causality gate in
    /// one SQLite transaction.  This is intentionally read-only: operation
    /// claiming remains an explicit admission port because the canonical
    /// session schema has no operation-claim table.
    pub async fn chat_causality_facts(
        &self,
        session_id: &AgentSessionId,
        turn_operation_id: &nomifun_agent_contracts::OperationId,
    ) -> Result<ChatCausalityFacts, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let rows = event_rows_for_session_tx(&mut tx, session_id.as_ref()).await?;

        let mut events = Vec::with_capacity(rows.len());
        let mut event_payloads = BTreeMap::new();
        let mut operation_ids = BTreeSet::new();
        let mut turn_route_identities = BTreeSet::new();
        for row in rows {
            let event = event_from_row(row)?;
            let payload = payload_value_for_event_tx(&mut tx, &event).await?;
            if event_belongs_to_turn(&event, &payload, turn_operation_id) {
                collect_chat_fact_metadata(
                    &payload,
                    &mut operation_ids,
                    &mut turn_route_identities,
                )?;
            } else {
                collect_operation_ids(&payload, &mut operation_ids);
            }
            event_payloads.insert(event.event_id.as_ref().to_owned(), payload);
            events.push(event);
        }
        tx.commit().await?;

        Ok(ChatCausalityFacts {
            session,
            head,
            events,
            event_payloads,
            operation_ids,
            turn_route_identities,
        })
    }

    /// Atomically admit one model operation against the current Session turn.
    ///
    /// This is deliberately implemented beside the canonical event append
    /// transaction. A read-then-append adapter cannot close the race with
    /// cancel/terminal events or guarantee operation uniqueness.
    pub async fn claim_chat_operation(
        &self,
        request: ChatOperationClaimRequest,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        request
            .route_identity
            .validate()
            .map_err(|error| SessionStoreError::Conflict(error.to_string()))?;
        let append = SessionEventAppend {
            agent_session_id: request.agent_session_id.clone(),
            event_id: EventId::from(format!(
                "model-input-admitted:{}:{}",
                request.agent_session_id.as_ref(),
                request.operation_id.as_ref()
            )),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(format!(
                "model-input-admitted:{}:{}",
                request.agent_session_id.as_ref(),
                request.operation_id.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("context/model-visible-applied".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(
                    request.turn_operation_id.as_ref().to_owned(),
                ),
                causation_event_id: Some(request.causation_event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "operation_id": request.operation_id,
                    "turn_operation_id": request.turn_operation_id,
                    "route_identity": request.route_identity,
                    "resolved_snapshot_ref": request.resolved_snapshot_ref,
                }))),
            },
        };

        let mut tx = self.pool.begin().await?;
        let _session = require_live_session_tx(&mut tx, request.agent_session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, request.agent_session_id.as_ref()).await?;
        if head.status != "running"
            || head.active_turn_id.as_deref() != Some(request.turn_operation_id.as_ref())
        {
            return Err(SessionStoreError::Conflict(
                "model operation requires the exact active turn boundary".to_owned(),
            ));
        }
        if let Some(existing) = duplicate_event_tx(&mut tx, &append).await? {
            let record = event_from_row(existing)?;
            let existing_payload = payload_value_for_event_tx(&mut tx, &record).await?;
            validate_existing_claim_payload(&existing_payload, &request)?;
            let ack = event_ack(&record);
            tx.commit().await?;
            return Ok(SessionEventAppendResult {
                cursor: ack.cursor.clone(),
                record: Some(record),
                ack: Some(ack),
                persisted: true,
                duplicate: true,
            });
        }

        let rows = event_rows_for_session_tx(&mut tx, request.agent_session_id.as_ref()).await?;
        let mut turn = None;
        let mut cause = None;
        let mut route_identities = BTreeSet::new();
        for row in rows {
            let event = event_from_row(row)?;
            let payload = payload_value_for_event_tx(&mut tx, &event).await?;
            if event.kind.0 == "turn/started"
                && event.correlation_id.as_ref() == request.turn_operation_id.as_ref()
            {
                turn = Some((event.clone(), payload.clone()));
            }
            if event.event_id == request.causation_event_id {
                cause = Some(event.clone());
            }
            if event.correlation_id.as_ref() == request.turn_operation_id.as_ref() {
                let mut ignored_operation_ids = BTreeSet::new();
                collect_chat_fact_metadata(
                    &payload,
                    &mut ignored_operation_ids,
                    &mut route_identities,
                )?;
            }
        }
        let (turn, turn_payload) = turn.ok_or_else(|| {
            SessionStoreError::Conflict("active turn fact is missing".to_owned())
        })?;
        let cause = cause.ok_or_else(|| {
            SessionStoreError::Conflict("model causation event is missing".to_owned())
        })?;
        if turn.causation_event_id.as_ref() != Some(&cause.event_id)
            || cause.seq >= turn.seq
            || !matches!(
                cause.kind.0.as_str(),
                "message/user-accepted" | "context/model-visible-applied"
            )
        {
            return Err(SessionStoreError::Conflict(
                "model causation is not linked to the active turn".to_owned(),
            ));
        }
        if route_identity_from_payload(&turn_payload)?
            .as_ref()
            != Some(&request.route_identity)
            || turn_payload
                .get("resolved_snapshot_ref")
                .and_then(|value| value.get("snapshot_digest"))
                .and_then(Value::as_str)
                != Some(request.resolved_snapshot_ref.snapshot_digest.as_ref())
            || route_identities != BTreeSet::from([request.route_identity.clone()])
        {
            return Err(SessionStoreError::Conflict(
                "model route facts differ from the active turn".to_owned(),
            ));
        }

        let result = self.append_event_tx(&mut tx, &append, None).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn head(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionHeadProjection, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let row = sqlx::query_as::<_, StoredHeadRow>(
            "SELECT session_id, status, active_turn_id, active_set_generation, \
                    runtime_checkpoint_locator, runtime_checkpoint_digest, \
                    runtime_bound_event_id, runtime_protocol_version, snapshot_digest, \
                    checkpoint_through_seq, last_seq, unread_count \
             FROM session_heads WHERE session_id = ?",
        )
        .bind(session_id.as_ref())
        .fetch_one(&self.pool)
        .await?;
        head_from_row(row)
    }

    pub async fn message_projections_after(
        &self,
        session_id: &AgentSessionId,
        after_seq: u64,
    ) -> Result<Vec<MessageProjection>, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let current_last_seq: i64 =
            sqlx::query_scalar("SELECT last_seq FROM session_heads WHERE session_id = ?")
                .bind(session_id.as_ref())
                .fetch_one(&self.pool)
                .await?;
        if after_seq > as_u64(current_last_seq, "last_seq")? {
            return Err(SessionStoreError::InvalidEvent(
                "projection cursor is ahead of the committed AgentSession sequence".to_owned(),
            ));
        }
        let rows = sqlx::query_as::<_, StoredProjectionRow>(
            "SELECT session_id, projection_id, first_seq, last_seq, presentation_intent, \
                    projection_json, semantic_digest \
             FROM message_projection \
             WHERE session_id = ? AND last_seq > ? \
             ORDER BY first_seq ASC, projection_id ASC",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(after_seq, "after_seq")?)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(projection_from_row).collect()
    }

    pub async fn rebuild_projections(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionHeadProjection, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        sqlx::query("DELETE FROM message_projection WHERE session_id = ?")
            .bind(session_id.as_ref())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM session_heads WHERE session_id = ?")
            .bind(session_id.as_ref())
            .execute(&mut *tx)
            .await?;

        let mut head = initial_head(session_id);
        insert_head_tx(&mut tx, &head).await?;
        let rows = event_rows_for_session_tx(&mut tx, session_id.as_ref()).await?;
        for row in rows {
            let event = event_from_row(row)?;
            let payload = payload_value_for_event_tx(&mut tx, &event).await?;
            reduce_head(&mut head, &event, &payload)?;
            persist_head_tx(&mut tx, &head).await?;
            if event_uses_message_projection(self.registry.entry(&event.kind, event.kind_version)?)
            {
                let existing = projection_by_identity_tx(&mut tx, &event).await?;
                let projection = reduce_message_projection(existing, &event, &payload)?;
                upsert_projection_tx(&mut tx, &projection).await?;
            }
        }

        let next_seq: i64 =
            sqlx::query_scalar("SELECT next_seq FROM agent_sessions WHERE agent_session_id = ?")
                .bind(session_id.as_ref())
                .fetch_one(&mut *tx)
                .await?;
        let expected_last = as_u64(next_seq, "next_seq")?.saturating_sub(1);
        if head.last_seq != expected_last {
            return Err(SessionStoreError::Conflict(format!(
                "projection rebuild ended at seq {}, expected {expected_last}",
                head.last_seq
            )));
        }
        tx.commit().await?;
        Ok(head)
    }

    pub async fn rehydration_input(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionRehydrationInput, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let compaction_row = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM session_events \
             WHERE session_id = ? AND kind = 'compaction/completed' \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(session_id.as_ref())
        .fetch_optional(&mut *tx)
        .await?;
        let completed_compaction = match compaction_row {
            Some(row) => {
                let event = event_from_row(row)?;
                let payload = payload_value_for_event_tx(&mut tx, &event).await?;
                Some(validate_compaction_payload_tx(&mut tx, &event, &payload).await?)
            }
            None => None,
        };
        let after_seq = completed_compaction
            .as_ref()
            .map_or(0, |compaction| compaction.through_seq);
        let rows = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM session_events \
             WHERE session_id = ? AND seq > ? \
             ORDER BY seq ASC",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(after_seq, "compaction through_seq")?)
        .fetch_all(&mut *tx)
        .await?;
        let subsequent_events = rows
            .into_iter()
            .map(event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(SessionRehydrationInput {
            agent_session_id: session_id.clone(),
            resolved_snapshot_ref: session.agent_binding.resolved_snapshot_ref,
            completed_compaction,
            subsequent_events,
            through_cursor: SessionEventCursor {
                agent_session_id: session_id.clone(),
                seq: head.last_seq,
            },
        })
    }

    pub async fn record_effect_started(
        &self,
        request: EffectEventRequest,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        self.append_event(&effect_append(request, "effect/started"))
            .await
    }

    pub async fn record_effect_terminal(
        &self,
        request: EffectEventRequest,
        state: EffectTerminalState,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        let kind = match state {
            EffectTerminalState::Succeeded => "effect/succeeded",
            EffectTerminalState::Failed => "effect/failed",
            EffectTerminalState::Uncertain => "effect/uncertain",
        };
        self.append_event(&effect_append(request, kind)).await
    }

    pub async fn reconcile_effect(
        &self,
        mut request: EffectEventRequest,
        outcome: EffectReconcileOutcome,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        request.payload =
            SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(outcome)?));
        self.append_event(&effect_append(request, "effect/reconciled"))
            .await
    }

    pub async fn admit_checkpoint(
        &self,
        session_id: &AgentSessionId,
        input: &RuntimeCheckpointValidationInput,
        compatibility_input: &SnapshotCompatibilityAdmissionInput,
    ) -> Result<CheckpointAdmission, SessionStoreError> {
        let session = require_live_session(&self.pool, session_id.as_ref()).await?;
        if compatibility_input.resolved_snapshot_ref != session.agent_binding.resolved_snapshot_ref
            || input.expected_snapshot_ref != session.agent_binding.resolved_snapshot_ref
        {
            return Err(SessionStoreError::InvalidSession(
                "checkpoint admission must use the AgentSession frozen Snapshot".to_owned(),
            ));
        }
        let compatibility = evaluate_snapshot_compatibility(compatibility_input);
        if let SnapshotCompatibilityAdmissionResult::ExecutorUnavailable {
            error_code,
            mismatches,
        } = &compatibility
        {
            return Err(SessionStoreError::Contract {
                code: error_code.clone(),
                message: format!(
                    "frozen Snapshot is unavailable on the active executor: {mismatches:?}"
                ),
            });
        }

        let validation = validate_checkpoint(input);
        let checkpoint_reusable =
            matches!(validation, RuntimeCheckpointValidationResult::ExactMatch);
        Ok(CheckpointAdmission {
            validation,
            compatibility: Some(compatibility),
            checkpoint_reusable,
        })
    }

    pub async fn discard_runtime_binding(
        &self,
        append: &SessionEventAppend,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        if append.semantic_event.kind.0 != "runtime/binding-discarded" {
            return Err(SessionStoreError::InvalidEvent(
                "checkpoint discard requires runtime/binding-discarded".to_owned(),
            ));
        }
        self.append_event(append).await
    }

    pub async fn fork_session(
        &self,
        parent_session_id: &AgentSessionId,
        request: ForkRequest,
    ) -> Result<ForkResult, SessionStoreError> {
        validate_uuidv7(request.child_session_id.as_ref(), "child_agent_session_id")?;
        if request.created_at < 0 {
            return Err(SessionStoreError::InvalidSession(
                "fork child created_at must not be negative".to_owned(),
            ));
        }
        if &request.child_session_id == parent_session_id {
            return Err(SessionStoreError::InvalidSession(
                "fork child must have a new AgentSessionId".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        let parent = require_live_session_tx(&mut tx, parent_session_id.as_ref()).await?;
        if parent.owner_ref != request.child_owner_ref {
            return Err(SessionStoreError::Conflict(
                "fork child owner must match parent owner".to_owned(),
            ));
        }

        if let Some(existing) = event_by_producer_key_tx(
            &mut tx,
            request.producer_id.as_ref(),
            request.idempotency_key.as_ref(),
        )
        .await?
        {
            return replay_fork(&mut tx, parent_session_id, &request, existing).await;
        }

        let parent_head = head_by_id_tx(&mut tx, parent_session_id.as_ref()).await?;
        let payload = build_payload_record(
            request.base_payload_id.clone(),
            request.child_session_id.clone(),
            request.base_media_type.clone(),
            request.base_body.clone(),
        )?;
        let child_session = AgentSessionLiveRecord {
            agent_session_id: request.child_session_id.clone(),
            owner_ref: request.child_owner_ref.clone(),
            metadata: request.child_metadata.clone(),
            agent_binding: request.child_agent_binding.clone(),
            remote_binding_provenance: None,
            parent_session_id: Some(parent_session_id.clone()),
            fork_base_payload_id: Some(request.base_payload_id.clone()),
            next_seq: 1,
        };
        validate_live_session(&child_session)?;
        insert_live_session_tx(&mut tx, &child_session, request.created_at).await?;
        insert_payload_tx(&mut tx, &payload).await?;
        insert_head_tx(&mut tx, &initial_head(&request.child_session_id)).await?;

        let child_opening = SessionEventAppend {
            agent_session_id: request.child_session_id.clone(),
            event_id: new_event_id(),
            producer_id: request.producer_id.clone(),
            idempotency_key: IdempotencyKey(format!(
                "{}:child-opening",
                request.idempotency_key.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("session/opening".to_owned()),
                kind_version: 1,
                correlation_id: request.correlation_id.clone(),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "operation_id": request.operation_id.as_ref(),
                    "agent_binding": &request.child_agent_binding,
                    "parent_session_id": parent_session_id,
                    "fork_base_payload_id": &request.base_payload_id
                }))),
            },
        };
        let _child_opening_ack =
            required_ack(self.append_event_tx(&mut tx, &child_opening, None).await?)?;
        let mut child_active_ids = request.child_initial_active_capability_ids.clone();
        child_active_ids.sort();
        child_active_ids.dedup();
        let child_active_set_digest = digest_payload(&child_active_ids)?;
        let child_activation = SessionEventAppend {
            agent_session_id: request.child_session_id.clone(),
            event_id: new_event_id(),
            producer_id: EventProducerId(format!(
                "{}:capability-host",
                request.producer_id.as_ref()
            )),
            idempotency_key: IdempotencyKey(format!(
                "{}:child-active-set-0",
                request.idempotency_key.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("capability/active-set-committed".to_owned()),
                kind_version: 1,
                correlation_id: request.correlation_id.clone(),
                causation_event_id: Some(child_opening.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "generation": 0,
                    "active_capability_ids": child_active_ids,
                    "active_set_digest": child_active_set_digest,
                    "delta": []
                }))),
            },
        };
        let child_activation_ack = required_ack(
            self.append_event_tx(&mut tx, &child_activation, None)
                .await?,
        )?;

        let fork_payload = SessionForkPayload {
            parent_session_id: parent_session_id.clone(),
            parent_through_seq: parent_head.last_seq,
            child_session_id: request.child_session_id.clone(),
            child_base_payload_id: request.base_payload_id.clone(),
            child_base_digest: payload.digest.clone(),
            child_agent_binding: request.child_agent_binding.clone(),
        };
        let fork_event = SessionEventAppend {
            agent_session_id: parent_session_id.clone(),
            event_id: request.event_id.clone().unwrap_or_else(new_event_id),
            producer_id: request.producer_id,
            idempotency_key: request.idempotency_key,
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("session/forked".to_owned()),
                kind_version: 1,
                correlation_id: request.correlation_id,
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(
                    &fork_payload,
                )?)),
            },
        };
        let fork_ack = required_ack(self.append_event_tx(&mut tx, &fork_event, None).await?)?;
        let child_session =
            live_session_by_id_tx(&mut tx, request.child_session_id.as_ref()).await?;
        tx.commit().await?;

        Ok(ForkResult {
            child_session,
            contract: SessionForkContract {
                contract_version: VersionString("session-fork-v1".to_owned()),
                fork: fork_payload,
                child_base_is_self_contained: true,
                copies_full_transcript: false,
                migrates_runtime_private_handles: false,
                replays_tool_or_effect: false,
            },
            fork_ack,
            child_cursor: child_activation_ack.cursor,
        })
    }

    pub async fn fence_delete(
        &self,
        command: &DeleteAgentSessionCommand,
    ) -> Result<AgentSessionDeletingRecord, SessionStoreError> {
        validate_uuidv7(command.agent_session_id.as_ref(), "agent_session_id")?;
        validate_principal(&command.owner_ref)?;
        if command.requested_at < 0 {
            return Err(SessionStoreError::InvalidSession(
                "delete requested_at must not be negative".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let row = session_row_by_id_tx(&mut tx, command.agent_session_id.as_ref()).await?;
        require_owner(&row, &command.owner_ref)?;
        match row.state.as_str() {
            "deleted" | "deleting" => {
                return Err(SessionStoreError::Deleted(
                    command.agent_session_id.0.clone(),
                ));
            }
            "live" => {}
            other => {
                return Err(SessionStoreError::InvalidSession(format!(
                    "unknown AgentSession state {other}"
                )));
            }
        }
        let live = live_from_row(row)?;
        let changed = sqlx::query(
            "UPDATE agent_sessions SET state = 'deleting' \
             WHERE agent_session_id = ? AND state = 'live'",
        )
        .bind(command.agent_session_id.as_ref())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(SessionStoreError::Deleted(
                command.agent_session_id.0.clone(),
            ));
        }
        tx.commit().await?;
        Ok(AgentSessionDeletingRecord {
            live,
            delete_operation_id: command.operation_id.clone(),
            admission_fenced_at: command.requested_at,
        })
    }

    pub async fn complete_delete(
        &self,
        command: &DeleteAgentSessionCommand,
        proof: &ZeroOutstandingProof,
        deleted_at: i64,
    ) -> Result<DeleteResult, SessionStoreError> {
        proof.validate().map_err(SessionStoreError::Conflict)?;
        if deleted_at < command.requested_at {
            return Err(SessionStoreError::Conflict(
                "deleted_at cannot precede delete requested_at".to_owned(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        let row = session_row_by_id_tx(&mut tx, command.agent_session_id.as_ref()).await?;
        require_owner(&row, &command.owner_ref)?;
        match row.state.as_str() {
            "deleted" => {
                return Err(SessionStoreError::Deleted(
                    command.agent_session_id.0.clone(),
                ));
            }
            "deleting" => {}
            "live" => {
                return Err(SessionStoreError::Conflict(
                    "delete admission fence has not committed".to_owned(),
                ));
            }
            other => {
                return Err(SessionStoreError::InvalidSession(format!(
                    "unknown AgentSession state {other}"
                )));
            }
        }

        purge_private_content_tx(&mut tx, command.agent_session_id.as_ref()).await?;
        sqlx::query(
            "UPDATE agent_sessions SET \
                state = 'deleted', title = NULL, archived = NULL, pinned = NULL, \
                agent_binding_json = NULL, remote_binding_id = NULL, \
                remote_binding_version = NULL, parent_agent_session_id = NULL, \
                fork_base_payload_id = NULL, next_seq = NULL, created_at = NULL, \
                deleted_at = ? \
             WHERE agent_session_id = ? AND state = 'deleting'",
        )
        .bind(deleted_at)
        .bind(command.agent_session_id.as_ref())
        .execute(&mut *tx)
        .await?;
        assert_tombstone_exact_tx(&mut tx, command.agent_session_id.as_ref()).await?;
        tx.commit().await?;

        Ok(DeleteResult {
            tombstone: AgentSessionTombstone {
                agent_session_id: command.agent_session_id.clone(),
                owner_ref: command.owner_ref.clone(),
                state: AgentSessionDeletedState::Deleted,
                deleted_at,
            },
            operation_id: command.operation_id.clone(),
        })
    }

    pub async fn delete_session(
        &self,
        command: &DeleteAgentSessionCommand,
        proof: &ZeroOutstandingProof,
        deleted_at: i64,
    ) -> Result<DeleteResult, SessionStoreError> {
        self.fence_delete(command).await?;
        self.complete_delete(command, proof, deleted_at).await
    }

    pub async fn deleting_sessions(
        &self,
    ) -> Result<Vec<AgentSessionLiveRecord>, SessionStoreError> {
        let rows = sqlx::query_as::<_, StoredSessionRow>(
            "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, remote_binding_id, remote_binding_version, \
                    parent_agent_session_id, fork_base_payload_id, next_seq, \
                    created_at, deleted_at \
             FROM agent_sessions WHERE state = 'deleting' ORDER BY agent_session_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(live_from_row).collect()
    }

    async fn append_event_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        append: &SessionEventAppend,
        payload: Option<&SessionPayloadRecord>,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_event_append(append)?;
        let registry_entry = self
            .registry
            .entry(
                &append.semantic_event.kind,
                append.semantic_event.kind_version,
            )?
            .clone();
        require_live_session_tx(tx, append.agent_session_id.as_ref()).await?;

        if self.registry.is_transient(
            &append.semantic_event.kind,
            append.semantic_event.kind_version,
        )? {
            if payload.is_some()
                || matches!(
                    append.semantic_event.payload,
                    SessionEventPayloadRef::Stored(_)
                )
            {
                return Err(SessionStoreError::InvalidPayload(
                    "transient diagnostics cannot create stored payload facts".to_owned(),
                ));
            }
            let last_seq = head_by_id_tx(tx, append.agent_session_id.as_ref())
                .await?
                .last_seq;
            return Ok(SessionEventAppendResult {
                record: None,
                ack: None,
                cursor: SessionEventCursor {
                    agent_session_id: append.agent_session_id.clone(),
                    seq: last_seq,
                },
                persisted: false,
                duplicate: false,
            });
        }

        if let Some(payload) = payload {
            validate_supplied_payload(append, payload)?;
            insert_payload_tx(tx, payload).await?;
        }
        if let Some(existing) = duplicate_event_tx(tx, append).await? {
            let record = event_from_row(existing)?;
            let ack = event_ack(&record);
            return Ok(SessionEventAppendResult {
                cursor: ack.cursor.clone(),
                record: Some(record),
                ack: Some(ack),
                persisted: true,
                duplicate: true,
            });
        }

        let head = head_by_id_tx(tx, append.agent_session_id.as_ref()).await?;
        validate_session_event_transition(&head, append)?;
        validate_predecessor_tx(tx, append, &registry_entry).await?;
        validate_runtime_sequence_tx(tx, append).await?;
        validate_effect_transition_tx(tx, append).await?;

        let stored_payload_value = validate_payload_reference_tx(tx, append).await?;

        let seq: i64 = sqlx::query_scalar(
            "UPDATE agent_sessions SET next_seq = next_seq + 1 \
             WHERE agent_session_id = ? AND state = 'live' \
             RETURNING next_seq - 1",
        )
        .bind(append.agent_session_id.as_ref())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| SessionStoreError::Deleted(append.agent_session_id.0.clone()))?;
        let record = record_from_append(append, as_u64(seq, "seq")?);
        insert_event_tx(tx, &record).await?;

        let payload_value = payload_value(&record, stored_payload_value);
        validate_semantic_event_tx(tx, &record, &payload_value).await?;
        let mut head = head_by_id_tx(tx, append.agent_session_id.as_ref()).await?;
        reduce_head(&mut head, &record, &payload_value)?;
        persist_head_tx(tx, &head).await?;

        if event_uses_message_projection(&registry_entry) {
            let existing = projection_by_identity_tx(tx, &record).await?;
            let projection = reduce_message_projection(existing, &record, &payload_value)?;
            upsert_projection_tx(tx, &projection).await?;
        }

        let ack = event_ack(&record);
        Ok(SessionEventAppendResult {
            cursor: ack.cursor.clone(),
            record: Some(record),
            ack: Some(ack),
            persisted: true,
            duplicate: false,
        })
    }
}

fn validate_session_event_transition(
    head: &SessionHeadProjection,
    append: &SessionEventAppend,
) -> Result<(), SessionStoreError> {
    let kind = append.semantic_event.kind.0.as_str();
    let requires_opening = matches!(kind, "session/ready" | "session/open-failed");
    if requires_opening && head.status != "opening" {
        return Err(SessionStoreError::Conflict(format!(
            "{kind} requires an opening Session, found {}",
            head.status
        )));
    }
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct StoredSessionRow {
    agent_session_id: String,
    owner_ref_json: String,
    state: String,
    title: Option<String>,
    archived: Option<i64>,
    pinned: Option<i64>,
    agent_binding_json: Option<String>,
    remote_binding_id: Option<String>,
    remote_binding_version: Option<i64>,
    parent_agent_session_id: Option<String>,
    fork_base_payload_id: Option<String>,
    next_seq: Option<i64>,
    created_at: Option<i64>,
    deleted_at: Option<i64>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct StoredEventRow {
    session_id: String,
    seq: i64,
    event_id: String,
    producer_id: String,
    idempotency_key: String,
    runtime_binding_id: Option<String>,
    runtime_producer_seq: Option<i64>,
    kind: String,
    kind_version: i64,
    correlation_id: String,
    causation_event_id: Option<String>,
    inline_json: Option<String>,
    payload_id: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredPayloadRow {
    payload_id: String,
    session_id: String,
    media_type: String,
    byte_len: i64,
    digest: String,
    body: Vec<u8>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredHeadRow {
    session_id: String,
    status: String,
    active_turn_id: Option<String>,
    active_set_generation: i64,
    runtime_checkpoint_locator: Option<String>,
    runtime_checkpoint_digest: Option<String>,
    runtime_bound_event_id: Option<String>,
    runtime_protocol_version: Option<String>,
    snapshot_digest: Option<String>,
    checkpoint_through_seq: Option<i64>,
    last_seq: i64,
    unread_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredProjectionRow {
    session_id: String,
    projection_id: String,
    first_seq: i64,
    last_seq: i64,
    presentation_intent: String,
    projection_json: String,
    semantic_digest: String,
}

async fn validate_fresh_v4_schema(pool: &SqlitePool) -> Result<(), SessionStoreError> {
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    if foreign_keys != 1 {
        return Err(SessionStoreError::InvalidSession(
            "shared Fresh-v4 pool must enforce SQLite foreign keys".to_owned(),
        ));
    }
    let actual = schema_table_names(pool).await?;
    let required = fresh_v4_schema_manifest_payload()
        .tables
        .into_iter()
        .map(|table| table.table_name)
        .collect::<BTreeSet<_>>();
    let missing = required
        .difference(&actual)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(SessionStoreError::InvalidSession(format!(
            "AgentSessionStore requires the shared canonical Fresh-v4 schema; missing tables {missing:?}"
        )));
    }
    let metadata: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT data_generation, migration_head, projection_schema_version \
         FROM schema_metadata WHERE singleton_key = 'canonical'",
    )
    .fetch_optional(pool)
    .await?;
    let Some((data_generation, migration_head, projection_schema_version)) = metadata else {
        return Err(SessionStoreError::InvalidSession(
            "canonical Fresh-v4 schema_metadata row is missing".to_owned(),
        ));
    };
    if data_generation != i64::from(FRESH_V4_DATA_GENERATION)
        || migration_head < i64::from(FRESH_V4_MIGRATION_HEAD)
        || projection_schema_version < i64::from(FRESH_V4_PROJECTION_SCHEMA_VERSION)
    {
        return Err(SessionStoreError::InvalidSession(format!(
            "unsupported Fresh-v4 metadata: generation={data_generation}, migration_head={migration_head}, projection_schema_version={projection_schema_version}"
        )));
    }

    let owned = expected_owned_table_names();
    let missing_owned = owned.difference(&actual).cloned().collect::<BTreeSet<_>>();
    if !missing_owned.is_empty() {
        return Err(SessionStoreError::InvalidSession(format!(
            "AgentSession owned tables are missing: {missing_owned:?}"
        )));
    }
    for (table, expected_columns) in SESSION_COLUMNS {
        let sql = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid");
        let actual_columns: Vec<String> = sqlx::query_scalar(&sql).fetch_all(pool).await?;
        let expected_columns = expected_columns
            .iter()
            .map(|column| (*column).to_owned())
            .collect::<Vec<_>>();
        if actual_columns != expected_columns {
            return Err(SessionStoreError::InvalidSession(format!(
                "{table} columns differ from the canonical AgentSession schema; expected {expected_columns:?}, found {actual_columns:?}"
            )));
        }
    }
    let actual_indexes: BTreeSet<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'index' AND name NOT LIKE 'sqlite_autoindex_%' \
           AND tbl_name IN (\
               'agent_sessions', 'session_events', 'session_payloads', \
               'session_heads', 'message_projection'\
           )",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    let expected_indexes = SESSION_INDEXES
        .iter()
        .map(|index| (*index).to_owned())
        .collect::<BTreeSet<_>>();
    if !expected_indexes.is_subset(&actual_indexes) {
        let missing_indexes = expected_indexes
            .difference(&actual_indexes)
            .cloned()
            .collect::<BTreeSet<_>>();
        return Err(SessionStoreError::InvalidSession(format!(
            "AgentSession canonical indexes are missing: {missing_indexes:?}"
        )));
    }
    // `remote_binding_id` is immutable provenance, not a live configuration
    // dependency. A RemoteBinding may be deleted to prevent future opens while
    // existing Sessions retain the ID/version they froze at creation time.
    Ok(())
}

async fn schema_table_names(pool: &SqlitePool) -> Result<BTreeSet<String>, SessionStoreError> {
    Ok(sqlx::query_scalar(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect())
}

fn expected_owned_table_names() -> BTreeSet<String> {
    SESSION_TABLES
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

#[cfg(test)]
async fn seed_test_schema_metadata(pool: &SqlitePool) -> Result<(), SessionStoreError> {
    sqlx::query(
        "INSERT INTO schema_metadata (\
            singleton_key, data_generation, root_instance_id, migration_head, \
            seed_manifest_digest, canonical_schema_manifest_digest, projection_schema_version\
         ) VALUES ('canonical', ?, 'agent-session-test-root', ?, ?, ?, ?)",
    )
    .bind(i64::from(FRESH_V4_DATA_GENERATION))
    .bind(i64::from(FRESH_V4_MIGRATION_HEAD))
    .bind("0".repeat(64))
    .bind("1".repeat(64))
    .bind(i64::from(FRESH_V4_PROJECTION_SCHEMA_VERSION))
    .execute(pool)
    .await?;
    Ok(())
}

fn validate_live_session(session: &AgentSessionLiveRecord) -> Result<(), SessionStoreError> {
    validate_uuidv7(session.agent_session_id.as_ref(), "agent_session_id")?;
    validate_principal(&session.owner_ref)?;
    if session
        .metadata
        .title
        .as_ref()
        .is_some_and(|title| title.trim() != title)
    {
        return Err(SessionStoreError::InvalidSession(
            "session title must not have edge whitespace".to_owned(),
        ));
    }
    if let Some(parent) = session.parent_session_id.as_ref() {
        validate_uuidv7(parent.as_ref(), "parent_session_id")?;
        if parent == &session.agent_session_id {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession cannot parent itself".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_principal(owner: &PrincipalRef) -> Result<(), SessionStoreError> {
    if owner.principal_kind.trim().is_empty()
        || owner.principal_kind.trim() != owner.principal_kind
        || owner.principal_id.trim().is_empty()
        || owner.principal_id.trim() != owner.principal_id
    {
        return Err(SessionStoreError::InvalidSession(
            "owner_ref requires canonical non-empty kind and id".to_owned(),
        ));
    }
    Ok(())
}

fn validate_uuidv7(value: &str, field: &str) -> Result<(), SessionStoreError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| SessionStoreError::InvalidSession(format!("{field} must be UUIDv7")))?;
    if parsed.get_version_num() != 7 || value != parsed.hyphenated().to_string() {
        return Err(SessionStoreError::InvalidSession(format!(
            "{field} must be lowercase canonical UUIDv7"
        )));
    }
    Ok(())
}

fn validate_event_append(append: &SessionEventAppend) -> Result<(), SessionStoreError> {
    validate_uuidv7(append.agent_session_id.as_ref(), "agent_session_id")?;
    for (field, value) in [
        ("event_id", append.event_id.as_ref()),
        ("producer_id", append.producer_id.as_ref()),
        ("idempotency_key", append.idempotency_key.as_ref()),
        (
            "correlation_id",
            append.semantic_event.correlation_id.as_ref(),
        ),
        ("kind", append.semantic_event.kind.0.as_str()),
    ] {
        if value.trim().is_empty() || value.trim() != value {
            return Err(SessionStoreError::InvalidEvent(format!(
                "{field} must be canonical and non-empty"
            )));
        }
    }
    if append.semantic_event.kind_version == 0 {
        return Err(SessionStoreError::InvalidEvent(
            "event kind_version must be at least one".to_owned(),
        ));
    }
    match (
        append.runtime_binding_id.as_ref(),
        append.runtime_producer_seq,
    ) {
        (None, None) => {}
        (Some(_), Some(sequence)) if sequence > 0 => {}
        _ => {
            return Err(SessionStoreError::InvalidEvent(
                "runtime_binding_id and positive runtime_producer_seq must be present together"
                    .to_owned(),
            ));
        }
    }
    if let SessionEventPayloadRef::InlineJson(value) = &append.semantic_event.payload {
        let bytes = canonical_json_bytes(&value.0)?;
        if bytes.len() > MAX_INLINE_JSON_BYTES {
            return Err(SessionStoreError::InvalidPayload(format!(
                "inline JSON exceeds {MAX_INLINE_JSON_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

fn opening_payload(request: &CreateSessionRequest) -> Result<Value, SessionStoreError> {
    let mut payload = json!({
        "operation_id": request.operation_id.as_ref(),
        "metadata": &request.session.metadata,
        "agent_binding": &request.session.agent_binding,
        "remote_binding_provenance": &request.session.remote_binding_provenance,
        "parent_session_id": &request.session.parent_session_id,
        "fork_base_payload_id": &request.session.fork_base_payload_id
    });
    if let Some(initial_input) = &request.initial_input {
        payload["initial_input"] = initial_input.0.clone();
    }
    Ok(payload)
}

fn runtime_event_append(
    context: RuntimeAppendContext,
) -> Result<SessionEventAppend, SessionStoreError> {
    if context.envelope.runtime_binding_id.as_ref().trim().is_empty()
        || context.envelope.producer_seq == 0
    {
        return Err(SessionStoreError::InvalidEvent(
            "Runtime event requires a canonical binding id and positive producer sequence"
                .to_owned(),
        ));
    }
    Ok(SessionEventAppend {
        agent_session_id: context.agent_session_id,
        event_id: context.envelope.event_id,
        producer_id: EventProducerId(format!(
            "runtime:{}",
            context.envelope.runtime_binding_id.as_ref()
        )),
        idempotency_key: context.envelope.idempotency_key,
        runtime_binding_id: Some(context.envelope.runtime_binding_id),
        runtime_producer_seq: Some(context.envelope.producer_seq),
        semantic_event: context.envelope.semantic_event,
    })
}

async fn replay_create(
    tx: &mut Transaction<'_, Sqlite>,
    request: &CreateSessionRequest,
    expected_payload: Value,
    existing: StoredEventRow,
) -> Result<SessionCreateResult, SessionStoreError> {
    let opening = event_from_row(existing)?;
    if opening.kind.0 != "session/opening"
        || opening.producer_id != request.producer_id
        || opening.idempotency_key != request.idempotency_key
        || opening.correlation_id != request.correlation_id
        || opening.payload != SessionEventPayloadRef::InlineJson(StrictJsonValue(expected_payload))
        || request
            .opening_event_id
            .as_ref()
            .is_some_and(|event_id| event_id != &opening.event_id)
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "session creation idempotency key was already used for different input".to_owned(),
        ));
    }

    let row = session_row_by_id_tx(tx, opening.agent_session_id.as_ref()).await?;
    let session = require_live_row(row)?;
    let activation_row = sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events \
         WHERE session_id = ? AND kind = 'capability/active-set-committed' \
         ORDER BY seq ASC LIMIT 1",
    )
    .bind(opening.agent_session_id.as_ref())
    .fetch_one(&mut **tx)
    .await?;
    let activation = event_from_row(activation_row)?;
    let mut active_ids = request.initial_active_capability_ids.clone();
    active_ids.sort();
    active_ids.dedup();
    let active_set_digest = digest_payload(&active_ids)?;
    let expected_activation_payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
        "generation": 0,
        "active_capability_ids": active_ids,
        "active_set_digest": active_set_digest,
        "delta": []
    })));
    if activation.payload != expected_activation_payload
        || request
            .activation_event_id
            .as_ref()
            .is_some_and(|event_id| event_id != &activation.event_id)
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "session creation replay changed the initial active capability set".to_owned(),
        ));
    }
    Ok(SessionCreateResult {
        session,
        opening_ack: event_ack(&opening),
        activation_ack: event_ack(&activation),
        duplicate: true,
    })
}

async fn insert_live_session_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session: &AgentSessionLiveRecord,
    created_at: i64,
) -> Result<(), SessionStoreError> {
    let owner_ref = serde_json::to_string(&session.owner_ref)?;
    let binding = serde_json::to_string(&session.agent_binding)?;
    let (remote_binding_id, remote_binding_version) = session
        .remote_binding_provenance
        .as_ref()
        .map(|provenance| {
            (
                Some(provenance.remote_binding_id.as_ref()),
                Some(provenance.binding_version),
            )
        })
        .unwrap_or((None, None));
    sqlx::query(
        "INSERT INTO agent_sessions (\
            agent_session_id, owner_ref_json, state, title, archived, pinned, \
            agent_binding_json, remote_binding_id, remote_binding_version, \
            parent_agent_session_id, fork_base_payload_id, next_seq, created_at, deleted_at\
         ) VALUES (?, ?, 'live', ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, NULL)",
    )
    .bind(session.agent_session_id.as_ref())
    .bind(owner_ref)
    .bind(&session.metadata.title)
    .bind(if session.metadata.archived {
        1_i64
    } else {
        0_i64
    })
    .bind(if session.metadata.pinned {
        1_i64
    } else {
        0_i64
    })
    .bind(binding)
    .bind(remote_binding_id)
    .bind(
        remote_binding_version
            .map(|version| as_i64(version, "remote binding version"))
            .transpose()?,
    )
    .bind(session.parent_session_id.as_ref().map(|id| id.as_ref()))
    .bind(session.fork_base_payload_id.as_ref().map(|id| id.as_ref()))
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(|error| {
        if is_unique_violation(&error) {
            SessionStoreError::Conflict(format!(
                "AgentSession {} already exists",
                session.agent_session_id.as_ref()
            ))
        } else {
            error.into()
        }
    })?;
    Ok(())
}

async fn insert_head_tx(
    tx: &mut Transaction<'_, Sqlite>,
    head: &SessionHeadProjection,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "INSERT INTO session_heads (\
            session_id, status, active_turn_id, active_set_generation, \
            runtime_checkpoint_locator, runtime_checkpoint_digest, runtime_bound_event_id, \
            runtime_protocol_version, snapshot_digest, checkpoint_through_seq, \
            last_seq, unread_count\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(head.session_id.as_ref())
    .bind(&head.status)
    .bind(&head.active_turn_id)
    .bind(as_i64(head.active_set_generation, "active_set_generation")?)
    .bind(&head.runtime_checkpoint_locator)
    .bind(&head.runtime_checkpoint_digest)
    .bind(&head.runtime_bound_event_id)
    .bind(&head.runtime_protocol_version)
    .bind(&head.snapshot_digest)
    .bind(
        head.checkpoint_through_seq
            .map(|value| as_i64(value, "checkpoint_through_seq"))
            .transpose()?,
    )
    .bind(as_i64(head.last_seq, "last_seq")?)
    .bind(as_i64(head.unread_count, "unread_count")?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn persist_head_tx(
    tx: &mut Transaction<'_, Sqlite>,
    head: &SessionHeadProjection,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "UPDATE session_heads SET \
            status = ?, active_turn_id = ?, active_set_generation = ?, \
            runtime_checkpoint_locator = ?, runtime_checkpoint_digest = ?, \
            runtime_bound_event_id = ?, runtime_protocol_version = ?, snapshot_digest = ?, \
            checkpoint_through_seq = ?, last_seq = ?, unread_count = ? \
         WHERE session_id = ?",
    )
    .bind(&head.status)
    .bind(&head.active_turn_id)
    .bind(as_i64(head.active_set_generation, "active_set_generation")?)
    .bind(&head.runtime_checkpoint_locator)
    .bind(&head.runtime_checkpoint_digest)
    .bind(&head.runtime_bound_event_id)
    .bind(&head.runtime_protocol_version)
    .bind(&head.snapshot_digest)
    .bind(
        head.checkpoint_through_seq
            .map(|value| as_i64(value, "checkpoint_through_seq"))
            .transpose()?,
    )
    .bind(as_i64(head.last_seq, "last_seq")?)
    .bind(as_i64(head.unread_count, "unread_count")?)
    .bind(head.session_id.as_ref())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
) -> Result<(), SessionStoreError> {
    let (inline_json, payload_id) = event_payload_columns(&event.payload)?;
    sqlx::query(
        "INSERT INTO session_events (\
            session_id, seq, event_id, producer_id, idempotency_key, \
            runtime_binding_id, runtime_producer_seq, kind, kind_version, \
            correlation_id, causation_event_id, inline_json, payload_id\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(event.agent_session_id.as_ref())
    .bind(as_i64(event.seq, "seq")?)
    .bind(event.event_id.as_ref())
    .bind(event.producer_id.as_ref())
    .bind(event.idempotency_key.as_ref())
    .bind(event.runtime_binding_id.as_ref().map(|id| id.as_ref()))
    .bind(
        event
            .runtime_producer_seq
            .map(|value| as_i64(value, "runtime_producer_seq"))
            .transpose()?,
    )
    .bind(&event.kind.0)
    .bind(i64::from(event.kind_version))
    .bind(event.correlation_id.as_ref())
    .bind(event.causation_event_id.as_ref().map(|id| id.as_ref()))
    .bind(inline_json)
    .bind(payload_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_payload_tx(
    tx: &mut Transaction<'_, Sqlite>,
    payload: &SessionPayloadRecord,
) -> Result<(), SessionStoreError> {
    validate_uuidv7(
        payload.agent_session_id.as_ref(),
        "payload.agent_session_id",
    )?;
    if payload.media_type.trim().is_empty() || payload.media_type.trim() != payload.media_type {
        return Err(SessionStoreError::InvalidPayload(
            "payload media_type must be canonical and non-empty".to_owned(),
        ));
    }
    let logical_bytes = logical_payload_bytes(&payload.body)?;
    if logical_bytes.len() > MAX_SINGLE_PAYLOAD_BYTES {
        return Err(SessionStoreError::InvalidPayload(format!(
            "payload exceeds {MAX_SINGLE_PAYLOAD_BYTES} bytes"
        )));
    }
    if payload.byte_len != logical_bytes.len() as u64 {
        return Err(SessionStoreError::InvalidPayload(format!(
            "payload byte_len {} does not match {} bytes",
            payload.byte_len,
            logical_bytes.len()
        )));
    }
    let digest = digest_bytes(&logical_bytes);
    if payload.digest != digest {
        return Err(SessionStoreError::InvalidPayload(
            "payload digest does not match canonical body bytes".to_owned(),
        ));
    }

    if let Some(existing) = payload_by_id_tx(tx, payload.payload_id.as_ref()).await? {
        let existing = payload_from_row(existing)?;
        if &existing == payload {
            return Ok(());
        }
        return Err(SessionStoreError::IdempotencyConflict(format!(
            "payload {} already exists with different content",
            payload.payload_id.as_ref()
        )));
    }

    let total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(byte_len), 0) FROM session_payloads WHERE session_id = ?",
    )
    .bind(payload.agent_session_id.as_ref())
    .fetch_one(&mut **tx)
    .await?;
    let projected_total = as_u64(total, "payload budget")?.saturating_add(payload.byte_len);
    if projected_total > MAX_SESSION_PAYLOAD_BYTES {
        return Err(SessionStoreError::InvalidPayload(format!(
            "session payload budget exceeds {MAX_SESSION_PAYLOAD_BYTES} bytes"
        )));
    }

    let stored_body = serde_json::to_vec(&payload.body)?;
    sqlx::query(
        "INSERT INTO session_payloads \
            (payload_id, session_id, media_type, byte_len, digest, body) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(payload.payload_id.as_ref())
    .bind(payload.agent_session_id.as_ref())
    .bind(&payload.media_type)
    .bind(as_i64(payload.byte_len, "payload byte_len")?)
    .bind(payload.digest.as_ref())
    .bind(stored_body)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn validate_supplied_payload(
    append: &SessionEventAppend,
    payload: &SessionPayloadRecord,
) -> Result<(), SessionStoreError> {
    let expected_payload_id = match &append.semantic_event.payload {
        SessionEventPayloadRef::Stored(payload_id) => Some(payload_id.as_ref()),
        SessionEventPayloadRef::InlineJson(value)
            if append.semantic_event.kind.0 == "compaction/completed" =>
        {
            value.0.get("context_payload_id").and_then(Value::as_str)
        }
        _ => None,
    }
    .ok_or_else(|| {
        SessionStoreError::InvalidPayload(
            "a supplied payload must be referenced by the same SessionEvent".to_owned(),
        )
    })?;
    if expected_payload_id != payload.payload_id.as_ref() {
        return Err(SessionStoreError::InvalidPayload(
            "supplied payload id does not match the SessionEvent payload reference".to_owned(),
        ));
    }
    if append.agent_session_id != payload.agent_session_id {
        return Err(SessionStoreError::InvalidPayload(
            "supplied payload belongs to another AgentSession".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_semantic_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    if event.kind.0 == "compaction/completed" {
        validate_compaction_payload_tx(tx, event, payload).await?;
    }
    Ok(())
}

async fn validate_compaction_payload_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<CompactionCompletedPayload, SessionStoreError> {
    let compaction: CompactionCompletedPayload =
        serde_json::from_value(payload.clone()).map_err(|error| {
            SessionStoreError::InvalidEvent(format!(
                "invalid compaction/completed payload: {error}"
            ))
        })?;
    if compaction.agent_session_id != event.agent_session_id {
        return Err(SessionStoreError::InvalidEvent(
            "compaction payload AgentSession does not match its event".to_owned(),
        ));
    }
    if compaction.through_seq >= event.seq {
        return Err(SessionStoreError::InvalidEvent(
            "compaction through_seq must precede the completed event".to_owned(),
        ));
    }
    let row = payload_by_id_tx(tx, compaction.context_payload_id.as_ref())
        .await?
        .ok_or_else(|| {
            SessionStoreError::InvalidPayload(
                "completed compaction context payload is missing".to_owned(),
            )
        })?;
    if row.session_id != event.agent_session_id.as_ref()
        || row.digest != compaction.context_digest.0
    {
        return Err(SessionStoreError::InvalidPayload(
            "completed compaction context payload identity/digest mismatch".to_owned(),
        ));
    }
    Ok(compaction)
}

async fn validate_payload_reference_tx(
    tx: &mut Transaction<'_, Sqlite>,
    append: &SessionEventAppend,
) -> Result<Option<Value>, SessionStoreError> {
    let SessionEventPayloadRef::Stored(payload_id) = &append.semantic_event.payload else {
        return Ok(None);
    };
    let row = payload_by_id_tx(tx, payload_id.as_ref())
        .await?
        .ok_or_else(|| {
            SessionStoreError::InvalidPayload(format!(
                "stored payload {} does not exist",
                payload_id.as_ref()
            ))
        })?;
    if row.session_id != append.agent_session_id.as_ref() {
        return Err(SessionStoreError::InvalidPayload(
            "stored payload belongs to another AgentSession".to_owned(),
        ));
    }
    Ok(Some(payload_body_to_value(&payload_from_row(row)?.body)?))
}

async fn duplicate_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    append: &SessionEventAppend,
) -> Result<Option<StoredEventRow>, SessionStoreError> {
    if let Some(existing) = event_by_event_id_tx(tx, append.event_id.as_ref()).await? {
        return compare_duplicate(append, existing).map(Some);
    }
    if let Some(existing) = event_by_producer_key_tx(
        tx,
        append.producer_id.as_ref(),
        append.idempotency_key.as_ref(),
    )
    .await?
    {
        return compare_duplicate(append, existing).map(Some);
    }
    if let (Some(binding), Some(sequence)) = (
        append.runtime_binding_id.as_ref(),
        append.runtime_producer_seq,
    ) {
        if let Some(existing) = event_by_runtime_sequence_tx(tx, binding.as_ref(), sequence).await?
        {
            return compare_duplicate(append, existing).map(Some);
        }
    }
    Ok(None)
}

fn compare_duplicate(
    append: &SessionEventAppend,
    existing: StoredEventRow,
) -> Result<StoredEventRow, SessionStoreError> {
    let record = event_from_row(existing.clone())?;
    if record_from_append(append, record.seq) == record {
        Ok(existing)
    } else {
        Err(SessionStoreError::IdempotencyConflict(format!(
            "event identity/idempotency already committed at {}/{}",
            record.agent_session_id.as_ref(),
            record.seq
        )))
    }
}

async fn validate_predecessor_tx(
    tx: &mut Transaction<'_, Sqlite>,
    append: &SessionEventAppend,
    entry: &nomifun_agent_contracts::SessionEventRegistryEntry,
) -> Result<(), SessionStoreError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_events WHERE session_id = ?")
        .bind(append.agent_session_id.as_ref())
        .fetch_one(&mut **tx)
        .await?;
    match entry.predecessor.mode.clone() {
        SessionEventPredecessorMode::None if count != 0 => {
            return Err(SessionStoreError::InvalidEvent(format!(
                "{} must be the first committed event",
                append.semantic_event.kind.0
            )));
        }
        SessionEventPredecessorMode::AnyCommitted if count == 0 => {
            return Err(SessionStoreError::InvalidEvent(format!(
                "{} requires a committed predecessor",
                append.semantic_event.kind.0
            )));
        }
        SessionEventPredecessorMode::AnyOf => {
            let kinds = &entry.predecessor.kinds;
            if kinds.is_empty() {
                return Err(SessionStoreError::InvalidEvent(format!(
                    "{} registry predecessor set is empty",
                    append.semantic_event.kind.0
                )));
            }
            let placeholders = std::iter::repeat_n("?", kinds.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT EXISTS(SELECT 1 FROM session_events \
                 WHERE session_id = ? AND kind IN ({placeholders}))"
            );
            let mut query =
                sqlx::query_scalar::<_, i64>(&sql).bind(append.agent_session_id.as_ref());
            for kind in kinds {
                query = query.bind(&kind.0);
            }
            if query.fetch_one(&mut **tx).await? == 0 {
                return Err(SessionStoreError::InvalidEvent(format!(
                    "{} has no allowed predecessor",
                    append.semantic_event.kind.0
                )));
            }
        }
        _ => {}
    }

    if let Some(causation_event_id) = append.semantic_event.causation_event_id.as_ref() {
        let cause: Option<String> =
            sqlx::query_scalar("SELECT session_id FROM session_events WHERE event_id = ?")
                .bind(causation_event_id.as_ref())
                .fetch_optional(&mut **tx)
                .await?;
        match cause.as_deref() {
            Some(session_id) if session_id == append.agent_session_id.as_ref() => {}
            Some(_) => {
                return Err(SessionStoreError::InvalidEvent(
                    "causation event belongs to another AgentSession".to_owned(),
                ));
            }
            None => {
                return Err(SessionStoreError::InvalidEvent(
                    "causation event does not exist".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

async fn validate_runtime_sequence_tx(
    tx: &mut Transaction<'_, Sqlite>,
    append: &SessionEventAppend,
) -> Result<(), SessionStoreError> {
    let (Some(binding), Some(actual)) = (
        append.runtime_binding_id.as_ref(),
        append.runtime_producer_seq,
    ) else {
        return Ok(());
    };
    if append.semantic_event.kind.0 != "runtime/bound" {
        let bound_exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM session_events \
             WHERE session_id = ? AND runtime_binding_id = ? AND kind = 'runtime/bound')",
        )
        .bind(append.agent_session_id.as_ref())
        .bind(binding.as_ref())
        .fetch_one(&mut **tx)
        .await?;
        if bound_exists == 0 {
            return Err(SessionStoreError::InvalidEvent(format!(
                "runtime binding {} has no committed runtime/bound event",
                binding.as_ref()
            )));
        }
    }
    let maximum: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(runtime_producer_seq) FROM session_events WHERE runtime_binding_id = ?",
    )
    .bind(binding.as_ref())
    .fetch_one(&mut **tx)
    .await?;
    let expected = maximum
        .map(|value| as_u64(value, "runtime producer sequence"))
        .transpose()?
        .unwrap_or(0)
        .saturating_add(1);
    if actual != expected {
        return Err(SessionStoreError::RuntimeSequenceGap {
            runtime_binding_id: binding.0.clone(),
            committed_producer_seq: expected.saturating_sub(1),
            expected,
            actual,
        });
    }
    Ok(())
}

async fn validate_effect_transition_tx(
    tx: &mut Transaction<'_, Sqlite>,
    append: &SessionEventAppend,
) -> Result<(), SessionStoreError> {
    if !append.semantic_event.kind.0.starts_with("effect/") {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events \
         WHERE session_id = ? AND correlation_id = ? AND kind LIKE 'effect/%' \
         ORDER BY seq ASC",
    )
    .bind(append.agent_session_id.as_ref())
    .bind(append.semantic_event.correlation_id.as_ref())
    .fetch_all(&mut **tx)
    .await?;
    let events = rows
        .into_iter()
        .map(event_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let started = events.iter().find(|event| event.kind.0 == "effect/started");
    let terminal = events.iter().find(|event| {
        matches!(
            event.kind.0.as_str(),
            "effect/succeeded" | "effect/failed" | "effect/uncertain"
        )
    });
    let reconciled = events
        .iter()
        .find(|event| event.kind.0 == "effect/reconciled");

    match append.semantic_event.kind.0.as_str() {
        "effect/started" if started.is_some() || terminal.is_some() || reconciled.is_some() => Err(
            SessionStoreError::InvalidEvent("effect cannot be started more than once".to_owned()),
        ),
        "effect/succeeded" | "effect/failed" | "effect/uncertain" => {
            let Some(started) = started else {
                return Err(SessionStoreError::InvalidEvent(
                    "effect terminal event requires effect/started".to_owned(),
                ));
            };
            if terminal.is_some() || reconciled.is_some() {
                return Err(SessionStoreError::InvalidEvent(
                    "effect already has a terminal outcome".to_owned(),
                ));
            }
            if started.idempotency_key != append.idempotency_key {
                return Err(SessionStoreError::InvalidEvent(
                    "effect lifecycle must retain the original idempotency key".to_owned(),
                ));
            }
            Ok(())
        }
        "effect/reconciled" => {
            let Some(started) = started else {
                return Err(SessionStoreError::InvalidEvent(
                    "effect reconciliation requires effect/started".to_owned(),
                ));
            };
            if terminal.is_none_or(|event| event.kind.0 != "effect/uncertain") {
                return Err(SessionStoreError::InvalidEvent(
                    "only an uncertain effect may be reconciled".to_owned(),
                ));
            }
            if reconciled.is_some() {
                return Err(SessionStoreError::InvalidEvent(
                    "effect reconciliation is already committed".to_owned(),
                ));
            }
            if started.idempotency_key != append.idempotency_key {
                return Err(SessionStoreError::InvalidEvent(
                    "effect reconciliation must use the original idempotency key".to_owned(),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn event_uses_message_projection(
    entry: &nomifun_agent_contracts::SessionEventRegistryEntry,
) -> bool {
    entry
        .projector
        .reducers
        .iter()
        .any(|reducer| reducer.as_ref() == "message-projection")
}

fn effect_append(request: EffectEventRequest, kind: &str) -> SessionEventAppend {
    SessionEventAppend {
        agent_session_id: request.agent_session_id,
        event_id: request.event_id,
        producer_id: request.producer_id,
        idempotency_key: request.idempotency_key,
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: SessionEventKind(kind.to_owned()),
            kind_version: 1,
            correlation_id: request.correlation_id,
            causation_event_id: request.causation_event_id,
            payload: request.payload,
        },
    }
}

fn record_from_append(append: &SessionEventAppend, seq: u64) -> SessionEventRecord {
    SessionEventRecord {
        agent_session_id: append.agent_session_id.clone(),
        seq,
        event_id: append.event_id.clone(),
        producer_id: append.producer_id.clone(),
        idempotency_key: append.idempotency_key.clone(),
        runtime_binding_id: append.runtime_binding_id.clone(),
        runtime_producer_seq: append.runtime_producer_seq,
        kind: append.semantic_event.kind.clone(),
        kind_version: append.semantic_event.kind_version,
        correlation_id: append.semantic_event.correlation_id.clone(),
        causation_event_id: append.semantic_event.causation_event_id.clone(),
        payload: append.semantic_event.payload.clone(),
    }
}

fn event_ack(event: &SessionEventRecord) -> SessionEventAck {
    SessionEventAck {
        agent_session_id: event.agent_session_id.clone(),
        event_id: event.event_id.clone(),
        seq: event.seq,
        cursor: SessionEventCursor {
            agent_session_id: event.agent_session_id.clone(),
            seq: event.seq,
        },
    }
}

fn required_ack(result: SessionEventAppendResult) -> Result<SessionEventAck, SessionStoreError> {
    result.ack.ok_or_else(|| {
        SessionStoreError::InvalidEvent("persistent event did not produce an ACK".to_owned())
    })
}

async fn session_row_by_id(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<StoredSessionRow, SessionStoreError> {
    optional_session_row_by_id(pool, session_id)
        .await?
        .ok_or_else(|| SessionStoreError::NotFound(session_id.to_owned()))
}

async fn optional_session_row_by_id(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Option<StoredSessionRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredSessionRow>(
        "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                agent_binding_json, remote_binding_id, remote_binding_version, \
                parent_agent_session_id, fork_base_payload_id, next_seq, created_at, deleted_at \
         FROM agent_sessions WHERE agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?)
}

async fn session_row_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<StoredSessionRow, SessionStoreError> {
    sqlx::query_as::<_, StoredSessionRow>(
        "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                agent_binding_json, remote_binding_id, remote_binding_version, \
                parent_agent_session_id, fork_base_payload_id, next_seq, created_at, deleted_at \
         FROM agent_sessions WHERE agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| SessionStoreError::NotFound(session_id.to_owned()))
}

async fn require_live_session(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<AgentSessionLiveRecord, SessionStoreError> {
    require_live_row(session_row_by_id(pool, session_id).await?)
}

async fn require_live_session_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<AgentSessionLiveRecord, SessionStoreError> {
    require_live_row(session_row_by_id_tx(tx, session_id).await?)
}

async fn live_session_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<AgentSessionLiveRecord, SessionStoreError> {
    require_live_session_tx(tx, session_id).await
}

fn require_live_row(row: StoredSessionRow) -> Result<AgentSessionLiveRecord, SessionStoreError> {
    match row.state.as_str() {
        "live" => live_from_row(row),
        "deleting" | "deleted" => Err(SessionStoreError::Deleted(row.agent_session_id)),
        other => Err(SessionStoreError::InvalidSession(format!(
            "unknown AgentSession state {other}"
        ))),
    }
}

fn live_from_row(row: StoredSessionRow) -> Result<AgentSessionLiveRecord, SessionStoreError> {
    if !matches!(row.state.as_str(), "live" | "deleting") {
        return Err(SessionStoreError::InvalidSession(format!(
            "{} is not a live/deleting AgentSession row",
            row.agent_session_id
        )));
    }
    let owner_ref: PrincipalRef = serde_json::from_str(&row.owner_ref_json)?;
    let binding_json = row.agent_binding_json.ok_or_else(|| {
        SessionStoreError::InvalidSession("live AgentSession lost agent_binding".to_owned())
    })?;
    let agent_binding = serde_json::from_str(&binding_json)?;
    if row.created_at.is_none() {
        return Err(SessionStoreError::InvalidSession(
            "live AgentSession lost created_at".to_owned(),
        ));
    }
    let remote_binding_provenance = match (row.remote_binding_id, row.remote_binding_version) {
        (Some(id), Some(version)) => Some(nomifun_agent_contracts::RemoteBindingProvenance {
            remote_binding_id: RemoteBindingId(id),
            binding_version: as_u64(version, "remote binding version")?,
        }),
        (None, None) => None,
        _ => {
            return Err(SessionStoreError::InvalidSession(
                "remote binding provenance is partial".to_owned(),
            ));
        }
    };
    Ok(AgentSessionLiveRecord {
        agent_session_id: AgentSessionId(row.agent_session_id),
        owner_ref,
        metadata: nomifun_agent_contracts::AgentSessionMetadata {
            title: row.title,
            archived: bool_from_i64(row.archived, "archived")?,
            pinned: bool_from_i64(row.pinned, "pinned")?,
        },
        agent_binding,
        remote_binding_provenance,
        parent_session_id: row.parent_agent_session_id.map(AgentSessionId),
        fork_base_payload_id: row.fork_base_payload_id.map(ArtifactId),
        next_seq: as_u64(
            row.next_seq.ok_or_else(|| {
                SessionStoreError::InvalidSession("live AgentSession lost next_seq".to_owned())
            })?,
            "next_seq",
        )?,
    })
}

fn tombstone_from_row(row: StoredSessionRow) -> Result<AgentSessionTombstone, SessionStoreError> {
    if row.state != "deleted" {
        return Err(SessionStoreError::InvalidSession(
            "row is not a deletion tombstone".to_owned(),
        ));
    }
    Ok(AgentSessionTombstone {
        agent_session_id: AgentSessionId(row.agent_session_id),
        owner_ref: serde_json::from_str(&row.owner_ref_json)?,
        state: AgentSessionDeletedState::Deleted,
        deleted_at: row.deleted_at.ok_or_else(|| {
            SessionStoreError::InvalidSession("deleted AgentSession lost deleted_at".to_owned())
        })?,
    })
}

fn require_owner(row: &StoredSessionRow, expected: &PrincipalRef) -> Result<(), SessionStoreError> {
    let actual: PrincipalRef = serde_json::from_str(&row.owner_ref_json)?;
    if &actual != expected {
        return Err(SessionStoreError::Conflict(
            "AgentSession owner mismatch".to_owned(),
        ));
    }
    Ok(())
}

async fn event_by_event_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event_id: &str,
) -> Result<Option<StoredEventRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events WHERE event_id = ?",
    )
    .bind(event_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn event_by_producer_key_tx(
    tx: &mut Transaction<'_, Sqlite>,
    producer_id: &str,
    idempotency_key: &str,
) -> Result<Option<StoredEventRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events WHERE producer_id = ? AND idempotency_key = ?",
    )
    .bind(producer_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn event_by_runtime_sequence_tx(
    tx: &mut Transaction<'_, Sqlite>,
    runtime_binding_id: &str,
    producer_seq: u64,
) -> Result<Option<StoredEventRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events WHERE runtime_binding_id = ? AND runtime_producer_seq = ?",
    )
    .bind(runtime_binding_id)
    .bind(as_i64(producer_seq, "runtime_producer_seq")?)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn event_rows_for_session_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<Vec<StoredEventRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events WHERE session_id = ? ORDER BY seq ASC",
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?)
}

fn event_from_row(row: StoredEventRow) -> Result<SessionEventRecord, SessionStoreError> {
    let payload = match (row.inline_json, row.payload_id) {
        (Some(_), Some(_)) => {
            return Err(SessionStoreError::InvalidEvent(
                "event row has both inline and stored payload".to_owned(),
            ));
        }
        (Some(value), None) => {
            SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::from_str(&value)?))
        }
        (None, Some(payload_id)) => SessionEventPayloadRef::Stored(ArtifactId(payload_id)),
        (None, None) => SessionEventPayloadRef::Empty,
    };
    Ok(SessionEventRecord {
        agent_session_id: AgentSessionId(row.session_id),
        seq: as_u64(row.seq, "event seq")?,
        event_id: EventId(row.event_id),
        producer_id: EventProducerId(row.producer_id),
        idempotency_key: IdempotencyKey(row.idempotency_key),
        runtime_binding_id: row.runtime_binding_id.map(RuntimeBindingId),
        runtime_producer_seq: row
            .runtime_producer_seq
            .map(|value| as_u64(value, "runtime_producer_seq"))
            .transpose()?,
        kind: SessionEventKind(row.kind),
        kind_version: u32::try_from(row.kind_version).map_err(|_| {
            SessionStoreError::InvalidEvent("kind_version is out of range".to_owned())
        })?,
        correlation_id: CorrelationId(row.correlation_id),
        causation_event_id: row.causation_event_id.map(EventId),
        payload,
    })
}

fn event_payload_columns(
    payload: &SessionEventPayloadRef,
) -> Result<(Option<String>, Option<String>), SessionStoreError> {
    match payload {
        SessionEventPayloadRef::Empty => Ok((None, None)),
        SessionEventPayloadRef::InlineJson(value) => {
            Ok((Some(serde_json::to_string(&value.0)?), None))
        }
        SessionEventPayloadRef::Stored(payload_id) => Ok((None, Some(payload_id.0.clone()))),
    }
}

async fn payload_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    payload_id: &str,
) -> Result<Option<StoredPayloadRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredPayloadRow>(
        "SELECT payload_id, session_id, media_type, byte_len, digest, body \
         FROM session_payloads WHERE payload_id = ?",
    )
    .bind(payload_id)
    .fetch_optional(&mut **tx)
    .await?)
}

fn payload_from_row(row: StoredPayloadRow) -> Result<SessionPayloadRecord, SessionStoreError> {
    Ok(SessionPayloadRecord {
        payload_id: ArtifactId(row.payload_id),
        agent_session_id: AgentSessionId(row.session_id),
        media_type: row.media_type,
        byte_len: as_u64(row.byte_len, "payload byte_len")?,
        digest: DigestHex(row.digest),
        body: serde_json::from_slice(&row.body)?,
    })
}

fn build_payload_record(
    payload_id: SessionPayloadId,
    session_id: AgentSessionId,
    media_type: String,
    body: SessionPayloadBody,
) -> Result<SessionPayloadRecord, SessionStoreError> {
    let bytes = logical_payload_bytes(&body)?;
    Ok(SessionPayloadRecord {
        payload_id,
        agent_session_id: session_id,
        media_type,
        byte_len: bytes.len() as u64,
        digest: digest_bytes(&bytes),
        body,
    })
}

fn logical_payload_bytes(body: &SessionPayloadBody) -> Result<Vec<u8>, SessionStoreError> {
    match body {
        SessionPayloadBody::Utf8(value) => Ok(value.as_bytes().to_vec()),
        SessionPayloadBody::Base64(value) => BASE64
            .decode(value)
            .map_err(|error| SessionStoreError::InvalidPayload(error.to_string())),
        SessionPayloadBody::Json(value) => Ok(canonical_json_bytes(&value.0)?),
        SessionPayloadBody::ArtifactRef(reference) => Ok(canonical_json_bytes(reference)?),
    }
}

fn payload_body_to_value(body: &SessionPayloadBody) -> Result<Value, SessionStoreError> {
    Ok(match body {
        SessionPayloadBody::Utf8(value) => json!({"text": value}),
        SessionPayloadBody::Base64(value) => json!({"base64": value}),
        SessionPayloadBody::Json(value) => value.0.clone(),
        SessionPayloadBody::ArtifactRef(reference) => serde_json::to_value(reference)?,
    })
}

async fn payload_value_for_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
) -> Result<Value, SessionStoreError> {
    match &event.payload {
        SessionEventPayloadRef::Empty => Ok(Value::Null),
        SessionEventPayloadRef::InlineJson(value) => Ok(value.0.clone()),
        SessionEventPayloadRef::Stored(payload_id) => {
            let row = payload_by_id_tx(tx, payload_id.as_ref())
                .await?
                .ok_or_else(|| {
                    SessionStoreError::InvalidPayload(format!(
                        "projection references missing payload {}",
                        payload_id.as_ref()
                    ))
                })?;
            payload_body_to_value(&payload_from_row(row)?.body)
        }
    }
}

fn event_belongs_to_turn(
    event: &SessionEventRecord,
    payload: &Value,
    turn_operation_id: &nomifun_agent_contracts::OperationId,
) -> bool {
    event.correlation_id.as_ref() == turn_operation_id.as_ref()
        || ["operation_id", "turn_operation_id"]
            .into_iter()
            .any(|key| payload.get(key).and_then(Value::as_str) == Some(turn_operation_id.as_ref()))
}

fn collect_operation_ids(payload: &Value, operation_ids: &mut BTreeSet<String>) {
    for key in ["operation_id", "turn_operation_id", "target_operation_id"] {
        if let Some(value) = payload.get(key).and_then(Value::as_str) {
            operation_ids.insert(value.to_owned());
        }
    }
    if let Some(value) = payload.get("causality") {
        collect_operation_ids(value, operation_ids);
    }
}

fn collect_chat_fact_metadata(
    payload: &Value,
    operation_ids: &mut BTreeSet<String>,
    route_identities: &mut BTreeSet<ChatRouteIdentity>,
) -> Result<(), SessionStoreError> {
    collect_operation_ids(payload, operation_ids);
    if let Some(identity) = route_identity_from_payload(payload)? {
        route_identities.insert(identity);
    }
    if let Some(value) = payload.get("causality") {
        collect_chat_fact_metadata(value, operation_ids, route_identities)?;
    }
    Ok(())
}

fn route_identity_from_payload(
    payload: &Value,
) -> Result<Option<ChatRouteIdentity>, SessionStoreError> {
    let Some(value) = payload.get("route_identity") else {
        return Ok(None);
    };
    let identity = serde_json::from_value::<ChatRouteIdentity>(value.clone()).map_err(|error| {
        SessionStoreError::Conflict(format!(
            "route_identity payload is not a canonical ChatRouteIdentity: {error}"
        ))
    })?;
    identity
        .validate()
        .map_err(|error| SessionStoreError::Conflict(error.to_string()))?;
    Ok(Some(identity))
}

fn validate_existing_claim_payload(
    payload: &Value,
    request: &ChatOperationClaimRequest,
) -> Result<(), SessionStoreError> {
    if route_identity_from_payload(payload)?.as_ref() != Some(&request.route_identity)
        || payload
            .get("resolved_snapshot_ref")
            .and_then(|value| value.get("snapshot_digest"))
            .and_then(Value::as_str)
            != Some(request.resolved_snapshot_ref.snapshot_digest.as_ref())
        || payload
            .get("operation_id")
            .and_then(Value::as_str)
            != Some(request.operation_id.as_ref())
        || payload
            .get("turn_operation_id")
            .and_then(Value::as_str)
            != Some(request.turn_operation_id.as_ref())
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "model operation claim already exists with a different immutable identity".to_owned(),
        ));
    }
    Ok(())
}

async fn head_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<SessionHeadProjection, SessionStoreError> {
    let row = sqlx::query_as::<_, StoredHeadRow>(
        "SELECT session_id, status, active_turn_id, active_set_generation, \
                runtime_checkpoint_locator, runtime_checkpoint_digest, \
                runtime_bound_event_id, runtime_protocol_version, snapshot_digest, \
                checkpoint_through_seq, last_seq, unread_count \
         FROM session_heads WHERE session_id = ?",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await?;
    head_from_row(row)
}

async fn ensure_active_turn_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    turn_operation_id: &OperationId,
) -> Result<(), SessionStoreError> {
    let head = head_by_id_tx(tx, session_id).await?;
    if head.status != "running"
        || head.active_turn_id.as_deref() != Some(turn_operation_id.as_ref())
    {
        return Err(SessionStoreError::Conflict(
            "chat terminal requires the exact active turn boundary".to_owned(),
        ));
    }
    Ok(())
}

fn head_from_row(row: StoredHeadRow) -> Result<SessionHeadProjection, SessionStoreError> {
    Ok(SessionHeadProjection {
        session_id: AgentSessionId(row.session_id),
        status: row.status,
        active_turn_id: row.active_turn_id,
        active_set_generation: as_u64(row.active_set_generation, "active_set_generation")?,
        runtime_checkpoint_locator: row.runtime_checkpoint_locator,
        runtime_checkpoint_digest: row.runtime_checkpoint_digest,
        runtime_bound_event_id: row.runtime_bound_event_id,
        runtime_protocol_version: row.runtime_protocol_version,
        snapshot_digest: row.snapshot_digest,
        checkpoint_through_seq: row
            .checkpoint_through_seq
            .map(|value| as_u64(value, "checkpoint_through_seq"))
            .transpose()?,
        last_seq: as_u64(row.last_seq, "last_seq")?,
        unread_count: as_u64(row.unread_count, "unread_count")?,
    })
}

async fn projection_by_identity_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
) -> Result<Option<MessageProjection>, SessionStoreError> {
    let prefix = event.kind.0.split('/').next().unwrap_or("event");
    let projection_id = format!("{prefix}:{}", event.correlation_id.as_ref());
    let row = sqlx::query_as::<_, StoredProjectionRow>(
        "SELECT session_id, projection_id, first_seq, last_seq, presentation_intent, \
                projection_json, semantic_digest \
         FROM message_projection WHERE session_id = ? AND projection_id = ?",
    )
    .bind(event.agent_session_id.as_ref())
    .bind(projection_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(projection_from_row).transpose()
}

async fn upsert_projection_tx(
    tx: &mut Transaction<'_, Sqlite>,
    projection: &MessageProjection,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "INSERT INTO message_projection (\
            session_id, projection_id, first_seq, last_seq, presentation_intent, \
            projection_json, semantic_digest\
         ) VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(session_id, projection_id) DO UPDATE SET \
            first_seq = excluded.first_seq, last_seq = excluded.last_seq, \
            presentation_intent = excluded.presentation_intent, \
            projection_json = excluded.projection_json, \
            semantic_digest = excluded.semantic_digest",
    )
    .bind(projection.session_id.as_ref())
    .bind(&projection.projection_id)
    .bind(as_i64(projection.first_seq, "projection first_seq")?)
    .bind(as_i64(projection.last_seq, "projection last_seq")?)
    .bind(&projection.presentation_intent)
    .bind(serde_json::to_string(&projection.projection)?)
    .bind(&projection.semantic_digest)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn projection_from_row(row: StoredProjectionRow) -> Result<MessageProjection, SessionStoreError> {
    Ok(MessageProjection {
        session_id: AgentSessionId(row.session_id),
        projection_id: row.projection_id,
        first_seq: as_u64(row.first_seq, "projection first_seq")?,
        last_seq: as_u64(row.last_seq, "projection last_seq")?,
        presentation_intent: row.presentation_intent,
        projection: serde_json::from_str(&row.projection_json)?,
        semantic_digest: row.semantic_digest,
    })
}

async fn purge_private_content_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<(), SessionStoreError> {
    sqlx::query("DELETE FROM message_projection WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM session_heads WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE session_events SET causation_event_id = NULL WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM session_events WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM session_payloads WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn assert_tombstone_exact_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<(), SessionStoreError> {
    let row = session_row_by_id_tx(tx, session_id).await?;
    if row.state != "deleted"
        || row.title.is_some()
        || row.archived.is_some()
        || row.pinned.is_some()
        || row.agent_binding_json.is_some()
        || row.remote_binding_id.is_some()
        || row.remote_binding_version.is_some()
        || row.parent_agent_session_id.is_some()
        || row.fork_base_payload_id.is_some()
        || row.next_seq.is_some()
        || row.created_at.is_some()
        || row.deleted_at.is_none()
    {
        return Err(SessionStoreError::Conflict(
            "final AgentSession row is not the exact four-field tombstone".to_owned(),
        ));
    }
    for table in [
        "session_events",
        "session_payloads",
        "session_heads",
        "message_projection",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?");
        let count: i64 = sqlx::query_scalar(&sql)
            .bind(session_id)
            .fetch_one(&mut **tx)
            .await?;
        if count != 0 {
            return Err(SessionStoreError::Conflict(format!(
                "tombstone retains {count} rows in {table}"
            )));
        }
    }
    Ok(())
}

async fn replay_fork(
    tx: &mut Transaction<'_, Sqlite>,
    parent_session_id: &AgentSessionId,
    request: &ForkRequest,
    existing: StoredEventRow,
) -> Result<ForkResult, SessionStoreError> {
    let event = event_from_row(existing)?;
    if event.agent_session_id != *parent_session_id
        || event.kind.0 != "session/forked"
        || event.producer_id != request.producer_id
        || event.idempotency_key != request.idempotency_key
        || request
            .event_id
            .as_ref()
            .is_some_and(|event_id| event_id != &event.event_id)
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "fork idempotency key was already used for another operation".to_owned(),
        ));
    }
    let SessionEventPayloadRef::InlineJson(value) = &event.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "session/forked event lost inline provenance".to_owned(),
        ));
    };
    let fork: SessionForkPayload = serde_json::from_value(value.0.clone())?;
    if fork.parent_session_id != *parent_session_id
        || fork.child_agent_binding != request.child_agent_binding
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "fork replay input does not match committed provenance".to_owned(),
        ));
    }
    let child_session = live_session_by_id_tx(tx, fork.child_session_id.as_ref()).await?;
    if child_session.owner_ref != request.child_owner_ref
        || child_session.metadata != request.child_metadata
        || child_session.agent_binding != request.child_agent_binding
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "fork replay changed child owner, metadata, or AgentBinding".to_owned(),
        ));
    }
    let actual_base = payload_by_id_tx(tx, fork.child_base_payload_id.as_ref())
        .await?
        .ok_or_else(|| {
            SessionStoreError::InvalidPayload(
                "fork child lost its self-contained base payload".to_owned(),
            )
        })
        .and_then(payload_from_row)?;
    let expected_base = build_payload_record(
        fork.child_base_payload_id.clone(),
        fork.child_session_id.clone(),
        request.base_media_type.clone(),
        request.base_body.clone(),
    )?;
    if actual_base != expected_base {
        return Err(SessionStoreError::IdempotencyConflict(
            "fork replay changed the self-contained base payload".to_owned(),
        ));
    }
    let activation_row = sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM session_events \
         WHERE session_id = ? AND kind = 'capability/active-set-committed' \
         ORDER BY seq ASC LIMIT 1",
    )
    .bind(fork.child_session_id.as_ref())
    .fetch_one(&mut **tx)
    .await?;
    let activation = event_from_row(activation_row)?;
    let mut active_ids = request.child_initial_active_capability_ids.clone();
    active_ids.sort();
    active_ids.dedup();
    let active_set_digest = digest_payload(&active_ids)?;
    let expected_activation = SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
        "generation": 0,
        "active_capability_ids": active_ids,
        "active_set_digest": active_set_digest,
        "delta": []
    })));
    if activation.payload != expected_activation {
        return Err(SessionStoreError::IdempotencyConflict(
            "fork replay changed the child initial active capability set".to_owned(),
        ));
    }
    let child_head = head_by_id_tx(tx, fork.child_session_id.as_ref()).await?;
    Ok(ForkResult {
        child_session,
        contract: SessionForkContract {
            contract_version: VersionString("session-fork-v1".to_owned()),
            fork,
            child_base_is_self_contained: true,
            copies_full_transcript: false,
            migrates_runtime_private_handles: false,
            replays_tool_or_effect: false,
        },
        fork_ack: event_ack(&event),
        child_cursor: SessionEventCursor {
            agent_session_id: child_head.session_id.clone(),
            seq: child_head.last_seq,
        },
    })
}

fn validate_cursor(
    session_id: &AgentSessionId,
    cursor: Option<&SessionEventCursor>,
) -> Result<u64, SessionStoreError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    if &cursor.agent_session_id != session_id {
        return Err(SessionStoreError::InvalidEvent(
            "cursor belongs to another AgentSession".to_owned(),
        ));
    }
    Ok(cursor.seq)
}

fn bool_from_i64(value: Option<i64>, field: &str) -> Result<bool, SessionStoreError> {
    match value {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(SessionStoreError::InvalidSession(format!(
            "{field} is not a canonical boolean"
        ))),
    }
}

fn as_i64(value: u64, field: &str) -> Result<i64, SessionStoreError> {
    i64::try_from(value).map_err(|_| {
        SessionStoreError::InvalidSession(format!("{field} exceeds SQLite INTEGER range"))
    })
}

fn as_u64(value: i64, field: &str) -> Result<u64, SessionStoreError> {
    u64::try_from(value)
        .map_err(|_| SessionStoreError::InvalidSession(format!("{field} must not be negative")))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}

fn new_event_id() -> EventId {
    EventId(Uuid::now_v7().to_string())
}
