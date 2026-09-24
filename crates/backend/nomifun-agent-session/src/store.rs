use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
#[cfg(test)]
use nomifun_agent_contracts::AGENT_STORE_BASELINE_SQL;
use nomifun_agent_contracts::{
    ActionId, AgentBindingChangedPayloadV1, AgentBindingValue, AgentHandoffBindingRefV1,
    AgentHandoffMode, AgentSessionDeletedState, AgentSessionDeletingRecord, AgentSessionId,
    AgentSessionLiveRecord, AgentSessionTombstone, ArtifactId, CapabilityId, ChatRouteIdentity,
    CompactionCompletedPayload, ConnectionConfigRef, CorrelationId, DeleteAgentSessionCommand,
    DigestHex, EventId,
    EventProducerId, AGENT_STORE_DATA_GENERATION,
    AGENT_STORE_MIGRATION_HEAD, AGENT_STORE_PROJECTION_SCHEMA_VERSION, IdempotencyKey, PrincipalRef,
    OperationId, ReasoningEffort, RemoteBindingId, RuntimeBindingId, RuntimeCheckpointValidationInput,
    RuntimeCheckpointValidationResult, RuntimeEventAck, SessionEventAck, SessionEventAppend,
    SessionEventCursor, SessionEventKind, SessionEventPayloadRef, SessionEventPredecessorMode,
    SessionEventRecord, SessionForkContract, SessionForkPayload, SessionPayloadBody,
    ResourceBindingId, ResourceId, ResourceKind, SessionPayloadId, SessionPayloadRecord,
    SnapshotCompatibilityAdmissionInput,
    SnapshotCompatibilityAdmissionResult, StrictJsonValue, VersionString, canonical_json_bytes,
    TypedResourceBinding, digest_bytes, digest_payload, agent_store_schema_manifest_payload,
};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::checkpoint::{evaluate_snapshot_compatibility, validate_checkpoint};
use crate::error::SessionStoreError;
use crate::projector::{initial_head, payload_value, reduce_head, reduce_agent_messages};
use crate::registry::EventRegistry;
use crate::types::{
    AgentDeletionAuditRecord, AgentEffectDeleteBlocker, AgentEffectRecord, AgentEffectState,
    AgentSessionListItem, AgentSessionListPage,
    AgentSessionAutomationConfig, AgentSessionDeleteBlockers, ChatCausalityFacts,
    ChatOperationClaimRequest, CheckpointAdmission, CommitAgentSessionAutomationConfig,
    CreateSessionRequest, DeleteResult, EffectEventRequest, EffectReconcileOutcome,
    EffectStrategy, EffectTerminalState, EnabledAgentSessionAutomationConfig, ForkRequest,
    ForkResult, MessageProjection, ResourceCleanupUncertainty, RuntimeAppendContext,
    RuntimeEventAppendResult, SessionCreateResult, SessionEventAppendResult, SessionEventPage,
    ReplaceSessionAgentBinding, SessionAgentBindingTransitionResult, SessionHeadProjection,
    SessionObservation, SessionRehydrationInput, TurnReceipt, TurnReceiptStatus,
    UpdateAgentSessionMetadata,
};

pub const MAX_INLINE_JSON_BYTES: usize = 64 * 1024;
pub const MAX_SINGLE_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_SESSION_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_EVENT_PAGE_SIZE: u32 = 500;

fn wall_clock_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

const AGENT_STORE_TABLES: [&str; 9] = [
    "agent_sessions",
    "agent_deletion_audits",
    "agent_turns",
    "agent_effects",
    "agent_session_resources",
    "agent_messages",
    "agent_events",
    "agent_session_heads",
    "agent_payloads",
];

const AGENT_STORE_COLUMNS: &[(&str, &[&str])] = &[
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
            "reasoning_effort",
        ],
    ),
    (
        "agent_deletion_audits",
        &[
            "audit_id",
            "agent_session_id",
            "owner_ref_json",
            "target_kind",
            "target_id",
            "authority",
            "risk_acknowledged",
            "reason_digest",
            "recorded_at",
        ],
    ),
    (
        "agent_turns",
        &[
            "session_id",
            "turn_id",
            "operation_id",
            "idempotency_key",
            "source_message_id",
            "admission_json",
            "state",
            "result_json",
            "error_json",
            "started_event_id",
            "terminal_event_id",
            "accepted_at",
            "started_at",
            "finished_at",
        ],
    ),
    (
        "agent_effects",
        &[
            "effect_id",
            "session_id",
            "turn_id",
            "operation_id",
            "owner_domain",
            "capability_module",
            "action_id",
            "resource_binding_id",
            "resource_key",
            "input_digest",
            "strategy",
            "state",
            "bounded_observation_json",
            "started_event_id",
            "terminal_event_id",
            "created_at",
            "settled_at",
        ],
    ),
    (
        "agent_session_resources",
        &[
            "binding_id",
            "session_id",
            "resource_kind",
            "resource_id",
            "owner_id",
            "operations_json",
            "connection_config_ref",
            "typed_parameters_json",
            "binding_digest",
        ],
    ),
    (
        "agent_events",
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
        "agent_payloads",
        &[
            "payload_id",
            "session_id",
            "media_type",
            "byte_len",
            "digest",
            "storage_kind",
            "body",
            "object_ref",
        ],
    ),
    (
        "agent_session_heads",
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
        "agent_messages",
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

const AGENT_STORE_INDEXES: [&str; 9] = [
    "idx_agent_sessions_owner_state",
    "idx_agent_deletion_audits_session_time",
    "idx_agent_turns_session_state",
    "idx_agent_session_resources_session_kind",
    "idx_agent_messages_sequence",
    "idx_agent_events_correlation",
    "idx_agent_payloads_session",
    "idx_agent_effects_session_turn",
    "idx_agent_effects_resource_unsettled",
];

#[derive(Clone, Debug)]
pub struct AgentSessionStore {
    pool: SqlitePool,
    registry: EventRegistry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppendSessionStatePolicy {
    LiveOnly,
    EffectSettlement,
    DeleteCleanupUncertainty,
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
        validate_agent_store_schema(&pool).await?;
        Ok(Self {
            pool,
            registry: EventRegistry::canonical()?,
        })
    }

    #[cfg(test)]
    pub(crate) async fn open_in_memory() -> Result<Self, SessionStoreError> {
        Self::open_in_memory_with_connections(1).await
    }

    #[cfg(test)]
    pub(crate) async fn open_in_memory_with_connections(
        max_connections: u32,
    ) -> Result<Self, SessionStoreError> {
        let options = SqliteConnectOptions::new()
            .filename(format!(
                "file:nomifun-agent-session-test-{}",
                Uuid::now_v7()
            ))
            .in_memory(true)
            .shared_cache(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Memory)
            .synchronous(SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(AGENT_STORE_BASELINE_SQL).execute(&pool).await?;
        seed_test_schema_metadata(&pool).await?;
        Self::from_pool(pool).await
    }

    pub fn event_registry(&self) -> &nomifun_agent_contracts::SessionEventRegistryPayload {
        self.registry.payload()
    }

    async fn begin_write_transaction(
        &self,
    ) -> Result<Transaction<'static, Sqlite>, SessionStoreError> {
        // SQLite's deferred BEGIN allows two writers to validate the same
        // head snapshot before either one obtains the write lock.  Lifecycle
        // mutations need one serialized validation/append boundary.
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
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
        // Session creation reads the idempotency key before inserting several
        // lifecycle rows. A deferred transaction can lose the writer race
        // after that read and fail immediately with SQLITE_BUSY instead of
        // honoring busy_timeout. Parallel AgentExecution roots create child
        // Sessions concurrently, so acquire the writer lock before replay
        // validation and serialize this short atomic boundary.
        let mut tx = self.begin_write_transaction().await?;
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
        insert_session_resources_tx(
            &mut tx,
            &request.session.agent_session_id,
            &request.session.owner_ref,
            &request.session.agent_binding.typed_resource_bindings,
        )
        .await?;

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
        let mut tx = self.begin_write_transaction().await?;
        let result = self.append_event_tx(&mut tx, append, payload).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Accept one user message and its Turn in the same transaction. The Store
    /// selects the exact predecessor while holding the write fence, so a
    /// concurrent caller cannot attach a second Turn to a stale ready head.
    /// Replays validate the original input and return the original receipt even
    /// after that Turn has crossed a terminal boundary.
    pub async fn start_turn(
        &self,
        session_id: &AgentSessionId,
        producer_id: EventProducerId,
        idempotency_key: IdempotencyKey,
        operation_id: OperationId,
        input: StrictJsonValue,
    ) -> Result<(SessionEventAppendResult, SessionEventAppendResult), SessionStoreError> {
        self.start_turn_with_admission(
            session_id,
            producer_id,
            idempotency_key,
            operation_id,
            input,
            false,
        )
        .await
    }

    /// Accept the creation handoff only while this Session is still at its
    /// untouched generation-zero boundary. Exact-key replays remain valid
    /// after admission, but a different initial delivery can never create a
    /// second first Turn.
    pub async fn start_initial_turn(
        &self,
        session_id: &AgentSessionId,
        producer_id: EventProducerId,
        idempotency_key: IdempotencyKey,
        operation_id: OperationId,
        input: StrictJsonValue,
    ) -> Result<(SessionEventAppendResult, SessionEventAppendResult), SessionStoreError> {
        self.start_turn_with_admission(
            session_id,
            producer_id,
            idempotency_key,
            operation_id,
            input,
            true,
        )
        .await
    }

    async fn start_turn_with_admission(
        &self,
        session_id: &AgentSessionId,
        producer_id: EventProducerId,
        idempotency_key: IdempotencyKey,
        operation_id: OperationId,
        input: StrictJsonValue,
        initial_only: bool,
    ) -> Result<(SessionEventAppendResult, SessionEventAppendResult), SessionStoreError> {
        if input
            .0
            .get("content")
            .and_then(Value::as_str)
            .is_none_or(|content| content.trim().is_empty())
        {
            return Err(SessionStoreError::InvalidPayload(
                "accepted turn input requires non-empty content".to_owned(),
            ));
        }

        let message_key = IdempotencyKey::from(format!("{}:message", idempotency_key.as_ref()));
        let turn_key = IdempotencyKey::from(format!("{}:turn", idempotency_key.as_ref()));
        let admission = input.0.get("admission").cloned();
        let turn_payload = |source_message_id: &EventId| {
            let mut payload = json!({
                "operation_id": operation_id,
                "source_message_id": source_message_id,
            });
            if let Some(admission) = admission.as_ref() {
                payload["admission"] = admission.clone();
                if let Some(route_identity) = admission.get("route_identity") {
                    payload["route_identity"] = route_identity.clone();
                }
                if let Some(snapshot) = admission.get("resolved_snapshot_ref") {
                    payload["resolved_snapshot_ref"] = snapshot.clone();
                }
            }
            payload
        };
        let mut tx = self.begin_write_transaction().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;

        let existing_message = event_by_producer_key_tx(
            &mut tx,
            producer_id.as_ref(),
            message_key.as_ref(),
        )
        .await?;
        let existing_turn =
            event_by_producer_key_tx(&mut tx, producer_id.as_ref(), turn_key.as_ref()).await?;
        match (existing_message, existing_turn) {
            (Some(message), Some(turn)) => {
                let message = event_from_row(message)?;
                let turn = event_from_row(turn)?;
                let expected_turn_payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(
                    turn_payload(&message.event_id),
                ));
                if message.agent_session_id != *session_id
                    || message.producer_id != producer_id
                    || message.idempotency_key != message_key
                    || message.kind.0 != "message/user-accepted"
                    || message.kind_version != 1
                    || message.correlation_id.as_ref() != message.event_id.as_ref()
                    || message.causation_event_id.is_none()
                    || message.payload
                        != SessionEventPayloadRef::InlineJson(input.clone())
                    || turn.agent_session_id != *session_id
                    || turn.producer_id != producer_id
                    || turn.idempotency_key != turn_key
                    || turn.kind.0 != "turn/started"
                    || turn.kind_version != 1
                    || turn.correlation_id.as_ref() != operation_id.as_ref()
                    || turn.causation_event_id.as_ref() != Some(&message.event_id)
                    || turn.payload != expected_turn_payload
                {
                    return Err(SessionStoreError::IdempotencyConflict(
                        "turn start idempotency key was already used for different input"
                            .to_owned(),
                    ));
                }
                let message_ack = event_ack(&message);
                let turn_ack = event_ack(&turn);
                tx.commit().await?;
                return Ok((
                    SessionEventAppendResult {
                        record: Some(message),
                        ack: Some(message_ack.clone()),
                        cursor: message_ack.cursor,
                        persisted: true,
                        duplicate: true,
                    },
                    SessionEventAppendResult {
                        record: Some(turn),
                        ack: Some(turn_ack.clone()),
                        cursor: turn_ack.cursor,
                        persisted: true,
                        duplicate: true,
                    },
                ));
            }
            (None, None) => {}
            _ => {
                return Err(SessionStoreError::IdempotencyConflict(
                    "turn start idempotency pair is incomplete".to_owned(),
                ));
            }
        }

        if initial_only {
            let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
            let has_history = sqlx::query_scalar::<_, i64>(
                "SELECT CASE WHEN \
                    EXISTS(SELECT 1 FROM agent_turns WHERE session_id = ?) OR \
                    EXISTS(SELECT 1 FROM agent_events WHERE session_id = ? AND kind LIKE 'message/%') \
                 THEN 1 ELSE 0 END",
            )
            .bind(session_id.as_ref())
            .bind(session_id.as_ref())
            .fetch_one(&mut *tx)
            .await?
                != 0;
            if head.status != "ready"
                || head.active_turn_id.is_some()
                || head.active_set_generation != 0
                || has_history
            {
                return Err(SessionStoreError::Conflict(
                    "initial-only turn requires a ready generation-zero Session with no committed Turn or transcript"
                        .to_owned(),
                ));
            }
        }

        let boundary = latest_turn_boundary_event_tx(&mut tx, session_id.as_ref()).await?;
        let message_event_id = new_event_id();
        let turn_event_id = new_event_id();
        let message = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: message_event_id.clone(),
            producer_id: producer_id.clone(),
            idempotency_key: message_key,
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("message/user-accepted".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_event_id.as_ref().to_owned()),
                causation_event_id: Some(boundary),
                payload: SessionEventPayloadRef::InlineJson(input),
            },
        };
        let turn = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: turn_event_id,
            producer_id,
            idempotency_key: turn_key,
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("turn/started".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(operation_id.as_ref().to_owned()),
                causation_event_id: Some(message_event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(turn_payload(
                    &message_event_id,
                ))),
            },
        };
        let message_result = self.append_event_tx(&mut tx, &message, None).await?;
        let turn_result = self.append_event_tx(&mut tx, &turn, None).await?;
        tx.commit().await?;
        Ok((message_result, turn_result))
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
        let mut tx = self.begin_write_transaction().await?;
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
        let mut tx = self.begin_write_transaction().await?;
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
        let mut tx = self.begin_write_transaction().await?;
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
             FROM agent_events \
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
                    "target_operation_id": target_operation_id,
                    "finished_at_ms": wall_clock_now_ms()
                }))),
            },
        };
        let result = self.append_event_tx(&mut tx, &append, None).await?;
        tx.commit().await?;
        Ok((target_operation_id, result))
    }

    /// Append one steering input to the exact active Turn under the same
    /// atomic head fence used by cancellation.
    pub async fn steer_active_turn(
        &self,
        session_id: &AgentSessionId,
        idempotency_key: IdempotencyKey,
        producer_id: EventProducerId,
        input: StrictJsonValue,
    ) -> Result<(OperationId, SessionEventAppendResult), SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
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
                || record.kind.0 != "turn/steer-accepted"
            {
                return Err(SessionStoreError::IdempotencyConflict(
                    "steering idempotency key was already used for another event".to_owned(),
                ));
            }
            let target = OperationId::from(record.correlation_id.as_ref().to_owned());
            let replay_matches = match &record.payload {
                SessionEventPayloadRef::InlineJson(value) => {
                    value
                        .0
                        .get("target_operation_id")
                        .and_then(Value::as_str)
                        == Some(target.as_ref())
                        && value.0.get("input") == Some(&input.0)
                }
                SessionEventPayloadRef::Empty | SessionEventPayloadRef::Stored(_) => false,
            };
            if !replay_matches {
                return Err(SessionStoreError::IdempotencyConflict(
                    "steering idempotency key was replayed with different input".to_owned(),
                ));
            }
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
                    "steering requires an active canonical Agent Turn".to_owned(),
                )
            })?;
        let turn_event = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM agent_events \
             WHERE session_id = ? AND kind = 'turn/started' AND correlation_id = ? \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(session_id.as_ref())
        .bind(target_operation_id.as_ref())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            SessionStoreError::Conflict(
                "active canonical Agent Turn has no start fact".to_owned(),
            )
        })?;
        let turn_event = event_from_row(turn_event)?;
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: new_event_id(),
            producer_id,
            idempotency_key,
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("turn/steer-accepted".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(target_operation_id.as_ref().to_owned()),
                causation_event_id: Some(turn_event.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "target_operation_id": target_operation_id,
                    "input": input,
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
             FROM agent_events \
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

    pub async fn session_created_at(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<i64, SessionStoreError> {
        let row = session_row_by_id(&self.pool, session_id.as_ref()).await?;
        if row.state != "live" {
            return Err(SessionStoreError::Deleted(row.agent_session_id));
        }
        row.created_at.ok_or_else(|| {
            SessionStoreError::InvalidSession(
                "live AgentSession lost created_at".to_owned(),
            )
        })
    }

    /// List only live Sessions for one exact owner. Deleted and deleting rows
    /// are never a history/archive fallback and therefore never surface here.
    pub async fn list_live_sessions(
        &self,
        owner: &PrincipalRef,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<AgentSessionListPage, SessionStoreError> {
        validate_principal(owner)?;
        if limit == 0 || limit > 10_000 {
            return Err(SessionStoreError::InvalidSession(format!(
                "AgentSession list limit must be between 1 and 10000",
            )));
        }
        if let Some(cursor) = cursor {
            validate_uuidv7(cursor, "cursor")?;
        }
        let owner_json = serde_json::to_string(owner)?;
        let mut tx = self.pool.begin().await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_sessions WHERE owner_ref_json = ? AND state = 'live'",
        )
        .bind(&owner_json)
        .fetch_one(&mut *tx)
        .await?;
        let fetch_limit = i64::from(limit) + 1;
        let rows = match cursor {
            Some(cursor) => {
                sqlx::query_as::<_, StoredSessionRow>(
                    "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                            agent_binding_json, remote_binding_id, remote_binding_version, \
                            parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, created_at, deleted_at \
                     FROM agent_sessions WHERE owner_ref_json = ? AND state = 'live' \
                       AND agent_session_id < ? \
                     ORDER BY agent_session_id DESC LIMIT ?",
                )
                .bind(&owner_json)
                .bind(cursor)
                .bind(fetch_limit)
                .fetch_all(&mut *tx)
                .await?
            }
            None => {
                sqlx::query_as::<_, StoredSessionRow>(
                    "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                            agent_binding_json, remote_binding_id, remote_binding_version, \
                            parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, created_at, deleted_at \
                     FROM agent_sessions WHERE owner_ref_json = ? AND state = 'live' \
                     ORDER BY agent_session_id DESC LIMIT ?",
                )
                .bind(&owner_json)
                .bind(fetch_limit)
                .fetch_all(&mut *tx)
                .await?
            }
        };
        let has_more = rows.len() > limit as usize;
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        for row in rows.into_iter().take(limit as usize) {
            let created_at = row.created_at.ok_or_else(|| {
                SessionStoreError::InvalidSession(
                    "live AgentSession lost created_at".to_owned(),
                )
            })?;
            let session = live_from_row(row)?;
            let head = head_by_id_tx(&mut tx, session.agent_session_id.as_ref()).await?;
            items.push(AgentSessionListItem {
                session,
                head,
                created_at,
            });
        }
        let next_cursor = has_more
            .then(|| items.last().map(|item| item.session.agent_session_id.as_ref().to_owned()))
            .flatten();
        tx.commit().await?;
        Ok(AgentSessionListPage {
            items,
            total: as_u64(total, "AgentSession list total")?,
            has_more,
            next_cursor,
        })
    }

    pub async fn update_session_metadata(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        update: UpdateAgentSessionMetadata,
    ) -> Result<AgentSessionLiveRecord, SessionStoreError> {
        validate_principal(owner)?;
        if update.title.is_none() && update.archived.is_none() && update.pinned.is_none() {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession metadata update is empty".to_owned(),
            ));
        }
        let title = update
            .title
            .as_deref()
            .map(str::trim)
            .map(str::to_owned);
        if title.as_ref().is_some_and(|title| title.is_empty() || title.len() > 200) {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession title must contain 1-200 UTF-8 bytes".to_owned(),
            ));
        }
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "live" {
            return Err(SessionStoreError::Deleted(row.agent_session_id));
        }
        let result = sqlx::query(
            "UPDATE agent_sessions SET \
                title = COALESCE(?, title), \
                archived = COALESCE(?, archived), \
                pinned = COALESCE(?, pinned) \
             WHERE agent_session_id = ? AND state = 'live'",
        )
        .bind(title)
        .bind(update.archived.map(i64::from))
        .bind(update.pinned.map(i64::from))
        .bind(session_id.as_ref())
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(SessionStoreError::Conflict(
                "AgentSession metadata update lost its live row".to_owned(),
            ));
        }
        let updated = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Replace the session-owned reasoning override. `None` restores model
    /// inheritance. Turn admission is fenced by the host before this call.
    pub async fn update_session_reasoning_effort(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        effort: Option<ReasoningEffort>,
    ) -> Result<AgentSessionLiveRecord, SessionStoreError> {
        validate_principal(owner)?;
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "live" {
            return Err(SessionStoreError::Deleted(row.agent_session_id));
        }
        let result = sqlx::query(
            "UPDATE agent_sessions SET reasoning_effort = ? \
             WHERE agent_session_id = ? AND state = 'live'",
        )
        .bind(effort.map(ReasoningEffort::as_str))
        .bind(session_id.as_ref())
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(SessionStoreError::Conflict(
                "AgentSession reasoning update lost its live row".to_owned(),
            ));
        }
        let updated = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Atomically replace only the host-validated model variant of a local
    /// AgentSession binding.
    ///
    /// The Store deliberately does not resolve routes or Presets. Its boundary
    /// is narrower: compare-and-swap the exact binding while proving that typed
    /// resource authority is unchanged, no Turn owns the Session, and Remote
    /// provenance is not being rewritten through a local product command.
    pub async fn replace_session_model_binding(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        expected: &AgentBindingValue,
        replacement: AgentBindingValue,
    ) -> Result<AgentSessionLiveRecord, SessionStoreError> {
        validate_principal(owner)?;
        if replacement.typed_resource_bindings != expected.typed_resource_bindings {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession model replacement must preserve typed resources".to_owned(),
            ));
        }
        if replacement.binding_version != expected.binding_version.checked_add(1).ok_or_else(|| {
            SessionStoreError::Conflict(
                "AgentSession binding version cannot advance".to_owned(),
            )
        })? {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession model replacement must advance binding_version exactly once"
                    .to_owned(),
            ));
        }
        if replacement.preset_revision_ref == expected.preset_revision_ref
            && replacement.resolved_snapshot_ref == expected.resolved_snapshot_ref
        {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession model replacement requires a new exact Revision/Snapshot"
                    .to_owned(),
            ));
        }

        let replacement_json = serde_json::to_string(&replacement)?;
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "live" {
            return Err(SessionStoreError::Deleted(row.agent_session_id));
        }
        if row.remote_binding_id.is_some() || row.remote_binding_version.is_some() {
            return Err(SessionStoreError::Conflict(
                "Remote AgentSession model binding is immutable".to_owned(),
            ));
        }
        let current: AgentBindingValue = serde_json::from_str(
            row.agent_binding_json.as_deref().ok_or_else(|| {
                SessionStoreError::InvalidSession(
                    "live AgentSession lost agent_binding".to_owned(),
                )
            })?,
        )?;
        if &current != expected {
            return Err(SessionStoreError::Conflict(
                "AgentSession binding changed before model replacement".to_owned(),
            ));
        }
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if head.status == "running" || head.active_turn_id.is_some() {
            return Err(SessionStoreError::Conflict(
                "wait for the active Turn before switching models".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE agent_sessions SET agent_binding_json = ? \
             WHERE agent_session_id = ? AND state = 'live' AND agent_binding_json = ?",
        )
        .bind(replacement_json)
        .bind(session_id.as_ref())
        .bind(serde_json::to_string(expected)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(SessionStoreError::Conflict(
                "AgentSession model replacement lost its compare-and-swap boundary".to_owned(),
            ));
        }
        let updated = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Atomically replace one host-validated resource kind inside a local
    /// AgentSession binding.
    ///
    /// This is the mutable-resource counterpart to model replacement: the
    /// immutable Preset revision/Snapshot stay exact, every other resource
    /// binding is byte-for-byte preserved, and the denormalized
    /// `agent_session_resources` projection changes in the same transaction as
    /// `agent_binding_json`. Active turns and Remote bindings remain immutable.
    pub async fn replace_session_resource_bindings(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        expected: &AgentBindingValue,
        replacement: AgentBindingValue,
        resource_kind: &str,
    ) -> Result<AgentSessionLiveRecord, SessionStoreError> {
        validate_principal(owner)?;
        if resource_kind.trim().is_empty() || resource_kind != resource_kind.trim() {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession resource replacement requires a canonical resource kind"
                    .to_owned(),
            ));
        }
        if replacement.binding_version != expected.binding_version.checked_add(1).ok_or_else(|| {
            SessionStoreError::Conflict(
                "AgentSession binding version cannot advance".to_owned(),
            )
        })? {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession resource replacement must advance binding_version exactly once"
                    .to_owned(),
            ));
        }
        if replacement.preset_revision_ref != expected.preset_revision_ref
            || replacement.resolved_snapshot_ref != expected.resolved_snapshot_ref
        {
            return Err(SessionStoreError::InvalidSession(
                "AgentSession resource replacement must preserve its exact Revision/Snapshot"
                    .to_owned(),
            ));
        }
        let retained = |binding: &AgentBindingValue| {
            binding
                .typed_resource_bindings
                .iter()
                .filter(|resource| resource.resource_kind.as_ref() != resource_kind)
                .cloned()
                .collect::<Vec<_>>()
        };
        if retained(&replacement) != retained(expected) {
            return Err(SessionStoreError::InvalidSession(format!(
                "AgentSession {resource_kind} replacement changed another resource kind"
            )));
        }

        let replacement_json = serde_json::to_string(&replacement)?;
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "live" {
            return Err(SessionStoreError::Deleted(row.agent_session_id));
        }
        if row.remote_binding_id.is_some() || row.remote_binding_version.is_some() {
            return Err(SessionStoreError::Conflict(
                "Remote AgentSession resource bindings are immutable".to_owned(),
            ));
        }
        let current: AgentBindingValue = serde_json::from_str(
            row.agent_binding_json.as_deref().ok_or_else(|| {
                SessionStoreError::InvalidSession(
                    "live AgentSession lost agent_binding".to_owned(),
                )
            })?,
        )?;
        if &current != expected {
            return Err(SessionStoreError::Conflict(
                "AgentSession binding changed before resource replacement".to_owned(),
            ));
        }
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if head.status == "running" || head.active_turn_id.is_some() {
            return Err(SessionStoreError::Conflict(
                "wait for the active Turn before changing Session resources".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE agent_sessions SET agent_binding_json = ? \
             WHERE agent_session_id = ? AND state = 'live' AND agent_binding_json = ?",
        )
        .bind(replacement_json)
        .bind(session_id.as_ref())
        .bind(serde_json::to_string(expected)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(SessionStoreError::Conflict(
                "AgentSession resource replacement lost its compare-and-swap boundary"
                    .to_owned(),
            ));
        }
        sqlx::query(
            "DELETE FROM agent_session_resources WHERE session_id = ? AND resource_kind = ?",
        )
        .bind(session_id.as_ref())
        .bind(resource_kind)
        .execute(&mut *tx)
        .await?;
        let replacements = replacement
            .typed_resource_bindings
            .iter()
            .filter(|resource| resource.resource_kind.as_ref() == resource_kind)
            .cloned()
            .collect::<Vec<_>>();
        insert_session_resources_tx(&mut tx, session_id, owner, &replacements).await?;
        let updated = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Atomically transition a local idle AgentSession to one complete,
    /// host-resolved Agent binding. Binding JSON, the denormalized resource
    /// projection, optional bounded handoff payload, transition audit event,
    /// runtime-checkpoint discard, and the next active capability generation
    /// commit or roll back together.
    pub async fn replace_session_agent_binding(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        request: ReplaceSessionAgentBinding,
    ) -> Result<SessionAgentBindingTransitionResult, SessionStoreError> {
        validate_principal(owner)?;
        validate_uuidv7(request.transition_id.as_ref(), "transition_id")?;
        let next_version = request
            .expected
            .binding_version
            .checked_add(1)
            .ok_or_else(|| {
                SessionStoreError::Conflict(
                    "AgentSession binding version cannot advance".to_owned(),
                )
            })?;
        if request.replacement.binding_version != next_version {
            return Err(SessionStoreError::InvalidSession(
                "Agent replacement must advance binding_version exactly once".to_owned(),
            ));
        }
        if request.replacement.preset_revision_ref == request.expected.preset_revision_ref
            && request.replacement.resolved_snapshot_ref
                == request.expected.resolved_snapshot_ref
        {
            return Err(SessionStoreError::InvalidSession(
                "Agent replacement requires a different exact Revision/Snapshot".to_owned(),
            ));
        }
        if [
            request.previous_agent_label.as_str(),
            request.next_agent_label.as_str(),
        ]
        .into_iter()
        .any(|label| label.trim().is_empty() || label != label.trim() || label.len() > 256)
        {
            return Err(SessionStoreError::InvalidSession(
                "Agent transition labels must be canonical and bounded".to_owned(),
            ));
        }
        match (request.handoff_mode, request.handoff.as_ref()) {
            (AgentHandoffMode::ContinueTask, Some(handoff)) => {
                handoff.validate().map_err(SessionStoreError::InvalidPayload)?;
                if handoff.source_agent_session_id != *session_id
                    || handoff.source_binding_ref
                        != AgentHandoffBindingRefV1::from(&request.expected)
                    || handoff.target_binding_ref
                        != AgentHandoffBindingRefV1::from(&request.replacement)
                {
                    return Err(SessionStoreError::InvalidPayload(
                        "Agent handoff identity differs from the exact transition".to_owned(),
                    ));
                }
            }
            (AgentHandoffMode::ContextOnly, None) => {}
            _ => {
                return Err(SessionStoreError::InvalidPayload(
                    "continue_task requires one handoff envelope; context_only forbids it"
                        .to_owned(),
                ));
            }
        }

        let mut active_ids = request.initial_active_capability_ids.clone();
        if active_ids.iter().any(|id| {
            id.trim().is_empty() || id.trim() != id || id.len() > 256
        }) {
            return Err(SessionStoreError::InvalidSession(
                "target active capability IDs must be canonical and bounded".to_owned(),
            ));
        }
        active_ids.sort();
        active_ids.dedup();
        if active_ids.len() > 128 {
            return Err(SessionStoreError::InvalidSession(
                "target active capability set exceeds 128 IDs".to_owned(),
            ));
        }

        let handoff_payload = request
            .handoff
            .as_ref()
            .map(|handoff| -> Result<SessionPayloadRecord, SessionStoreError> {
                let body = serde_json::to_value(handoff)?;
                let logical_bytes = canonical_json_bytes(&body)?;
                Ok(SessionPayloadRecord {
                    payload_id: ArtifactId::from(format!(
                        "agent-handoff:{}",
                        request.transition_id.as_ref()
                    )),
                    agent_session_id: session_id.clone(),
                    media_type: "application/vnd.nomifun.agent-handoff+json;version=1".to_owned(),
                    byte_len: logical_bytes.len() as u64,
                    digest: digest_bytes(&logical_bytes),
                    body: SessionPayloadBody::Json(StrictJsonValue(body)),
                })
            })
            .transpose()?;
        let producer_id = EventProducerId::from("session_api");
        let transition_event_id = EventId::from(format!(
            "agent-binding-changed:{}",
            request.transition_id.as_ref()
        ));
        let mut tx = self.begin_write_transaction().await?;

        if let Some(existing) = event_by_producer_key_tx(
            &mut tx,
            producer_id.as_ref(),
            request.idempotency_key.as_ref(),
        )
        .await?
        {
            let record = event_from_row(existing)?;
            if record.agent_session_id != *session_id
                || record.kind.0 != "session/agent-binding-changed"
            {
                return Err(SessionStoreError::IdempotencyConflict(
                    "Agent switch idempotency key was used by another command".to_owned(),
                ));
            }
            let payload = payload_value_for_event_tx(&mut tx, &record).await?;
            let transition: AgentBindingChangedPayloadV1 = serde_json::from_value(payload)?;
            if transition.transition_id != request.transition_id
                || transition.request_digest != request.request_digest
                || transition.previous_binding_ref
                    != AgentHandoffBindingRefV1::from(&request.expected)
                || transition.next_binding_ref
                    != AgentHandoffBindingRefV1::from(&request.replacement)
                || transition.previous_agent_label != request.previous_agent_label
                || transition.next_agent_label != request.next_agent_label
                || transition.handoff_mode != request.handoff_mode
                || transition.handoff_payload_digest
                    != handoff_payload.as_ref().map(|payload| payload.digest.clone())
            {
                return Err(SessionStoreError::IdempotencyConflict(
                    "Agent switch idempotency key was replayed with different input".to_owned(),
                ));
            }
            let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
            if session.agent_binding != request.replacement {
                return Err(SessionStoreError::IdempotencyConflict(
                    "replayed Agent switch no longer matches the live binding".to_owned(),
                ));
            }
            let active = event_by_kind_correlation_tx(
                &mut tx,
                session_id.as_ref(),
                "capability/active-set-committed",
                request.transition_id.as_ref(),
            )
            .await?
            .ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "replayed Agent switch lost its active-set commit".to_owned(),
                )
            })?;
            let discard = event_by_kind_causation_tx(
                &mut tx,
                session_id.as_ref(),
                "runtime/binding-discarded",
                record.event_id.as_ref(),
            )
            .await?;
            let transition_ack = event_ack(&record);
            let active_set_ack = event_ack(&event_from_row(active)?);
            let runtime_discard_ack = discard
                .map(event_from_row)
                .transpose()?
                .map(|event| event_ack(&event));
            tx.commit().await?;
            return Ok(SessionAgentBindingTransitionResult {
                session,
                transition,
                transition_ack,
                runtime_discard_ack,
                active_set_ack,
                duplicate: true,
            });
        }

        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "live" {
            return Err(SessionStoreError::Deleted(row.agent_session_id));
        }
        if row.remote_binding_id.is_some() || row.remote_binding_version.is_some() {
            return Err(SessionStoreError::Conflict(
                "Remote AgentSession Agent binding is immutable".to_owned(),
            ));
        }
        let current: AgentBindingValue = serde_json::from_str(
            row.agent_binding_json.as_deref().ok_or_else(|| {
                SessionStoreError::InvalidSession(
                    "live AgentSession lost agent_binding".to_owned(),
                )
            })?,
        )?;
        if current != request.expected {
            return Err(SessionStoreError::Conflict(
                "AgentSession binding changed before Agent replacement".to_owned(),
            ));
        }
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if head.status == "running" || head.active_turn_id.is_some() {
            return Err(SessionStoreError::Conflict(
                "wait for the active Turn before switching Agents".to_owned(),
            ));
        }
        if let Some(handoff) = request.handoff.as_ref() {
            let source: Option<(String, String)> = sqlx::query_as(
                "SELECT kind, correlation_id FROM agent_events \
                 WHERE session_id = ? AND seq = ?",
            )
            .bind(session_id.as_ref())
            .bind(as_i64(handoff.source_through_seq, "handoff source sequence")?)
            .fetch_optional(&mut *tx)
            .await?;
            if source.as_ref().is_none_or(|(kind, correlation)| {
                !matches!(kind.as_str(), "turn/completed" | "turn/failed" | "turn/cancelled")
                    || correlation != handoff.source_turn_operation_id.as_ref()
            }) {
                return Err(SessionStoreError::InvalidPayload(
                    "Agent handoff source is not the exact canonical closed Turn boundary"
                        .to_owned(),
                ));
            }
        }
        let unsettled: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_effects \
             WHERE session_id = ? AND state IN ('pending', 'unknown'))",
        )
        .bind(session_id.as_ref())
        .fetch_one(&mut *tx)
        .await?;
        if unsettled != 0 {
            return Err(SessionStoreError::Conflict(
                "AgentSession has unsettled effects".to_owned(),
            ));
        }
        let next_generation = head
            .active_set_generation
            .checked_add(1)
            .ok_or_else(|| {
                SessionStoreError::Conflict(
                    "AgentSession active capability generation cannot advance".to_owned(),
                )
            })?;
        let transition = AgentBindingChangedPayloadV1 {
            transition_id: request.transition_id.clone(),
            request_digest: request.request_digest,
            previous_binding_ref: AgentHandoffBindingRefV1::from(&request.expected),
            next_binding_ref: AgentHandoffBindingRefV1::from(&request.replacement),
            previous_agent_label: request.previous_agent_label,
            next_agent_label: request.next_agent_label,
            handoff_mode: request.handoff_mode,
            handoff_payload_id: handoff_payload
                .as_ref()
                .map(|payload| payload.payload_id.clone()),
            handoff_payload_digest: handoff_payload
                .as_ref()
                .map(|payload| payload.digest.clone()),
            completion_gate_inherited: false,
            effective_after_seq: head.last_seq,
        };

        let replacement_json = serde_json::to_string(&request.replacement)?;
        let result = sqlx::query(
            "UPDATE agent_sessions SET agent_binding_json = ? \
             WHERE agent_session_id = ? AND state = 'live' AND agent_binding_json = ?",
        )
        .bind(replacement_json)
        .bind(session_id.as_ref())
        .bind(serde_json::to_string(&request.expected)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(SessionStoreError::Conflict(
                "Agent replacement lost its compare-and-swap boundary".to_owned(),
            ));
        }
        sqlx::query("DELETE FROM agent_session_resources WHERE session_id = ?")
            .bind(session_id.as_ref())
            .execute(&mut *tx)
            .await?;
        insert_session_resources_tx(
            &mut tx,
            session_id,
            owner,
            &request.replacement.typed_resource_bindings,
        )
        .await?;
        if let Some(payload) = handoff_payload.as_ref() {
            insert_payload_tx(&mut tx, payload).await?;
        }

        let transition_append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: transition_event_id.clone(),
            producer_id,
            idempotency_key: request.idempotency_key.clone(),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("session/agent-binding-changed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(request.transition_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                    serde_json::to_value(&transition)?,
                )),
            },
        };
        let transition_result = self
            .append_event_tx(&mut tx, &transition_append, None)
            .await?;
        let transition_ack = required_ack(transition_result)?;

        let runtime_discard_ack = if let Some(runtime_bound_event_id) =
            head.runtime_bound_event_id.as_deref()
        {
            let bound = event_by_event_id_tx(&mut tx, runtime_bound_event_id)
                .await?
                .ok_or_else(|| {
                    SessionStoreError::InvalidEvent(
                        "runtime head references a missing bound event".to_owned(),
                    )
                })?;
            let bound = event_from_row(bound)?;
            let runtime_binding_id = bound.runtime_binding_id.clone().ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "runtime/bound event has no runtime binding identity".to_owned(),
                )
            })?;
            let maximum: Option<i64> = sqlx::query_scalar(
                "SELECT MAX(runtime_producer_seq) FROM agent_events WHERE runtime_binding_id = ?",
            )
            .bind(runtime_binding_id.as_ref())
            .fetch_one(&mut *tx)
            .await?;
            let runtime_producer_seq = maximum
                .map(|value| as_u64(value, "runtime producer sequence"))
                .transpose()?
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| {
                    SessionStoreError::Conflict(
                        "runtime producer sequence cannot advance".to_owned(),
                    )
                })?;
            let append = SessionEventAppend {
                agent_session_id: session_id.clone(),
                event_id: EventId::from(format!(
                    "agent-switch-runtime-discard:{}",
                    request.transition_id.as_ref()
                )),
                producer_id: EventProducerId::from("runtime_supervisor"),
                idempotency_key: IdempotencyKey::from(format!(
                    "{}:runtime-discard",
                    request.idempotency_key.as_ref()
                )),
                runtime_binding_id: Some(runtime_binding_id.clone()),
                runtime_producer_seq: Some(runtime_producer_seq),
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: SessionEventKind("runtime/binding-discarded".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(runtime_binding_id.as_ref().to_owned()),
                    causation_event_id: Some(transition_event_id.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "reason": "agent_binding_changed",
                        "previous_resolved_snapshot_ref": request.expected.resolved_snapshot_ref,
                        "next_resolved_snapshot_ref": request.replacement.resolved_snapshot_ref,
                    }))),
                },
            };
            Some(required_ack(
                self.append_event_tx(&mut tx, &append, None).await?,
            )?)
        } else {
            None
        };

        let active_set_digest = digest_payload(&active_ids)?.0;
        let active_append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(format!(
                "agent-switch-active-set:{}",
                request.transition_id.as_ref()
            )),
            producer_id: EventProducerId::from("capability_host"),
            idempotency_key: IdempotencyKey::from(format!(
                "{}:active-set",
                request.idempotency_key.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("capability/active-set-committed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(request.transition_id.as_ref().to_owned()),
                causation_event_id: Some(transition_event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "generation": next_generation,
                    "active_capability_ids": active_ids,
                    "active_set_digest": active_set_digest,
                    "delta": [],
                }))),
            },
        };
        let active_set_ack = required_ack(
            self.append_event_tx(&mut tx, &active_append, None).await?,
        )?;
        let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(SessionAgentBindingTransitionResult {
            session,
            transition,
            transition_ack,
            runtime_discard_ack,
            active_set_ack,
            duplicate: false,
        })
    }

    pub async fn active_capability_ids(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Vec<String>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let row = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM agent_events WHERE session_id = ? \
               AND kind = 'capability/active-set-committed' \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(session_id.as_ref())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            SessionStoreError::Conflict(
                "AgentSession has no committed active capability set".to_owned(),
            )
        })?;
        let event = event_from_row(row)?;
        let payload = payload_value_for_event_tx(&mut tx, &event).await?;
        let ids = payload
            .get("active_capability_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "active capability projection lost active_capability_ids".to_owned(),
                )
            })?
            .iter()
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    SessionStoreError::InvalidEvent(
                        "active capability projection contains a non-string ID".to_owned(),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await?;
        Ok(ids)
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
             INNER JOIN agent_session_heads AS heads \
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
        let head = self.head(session_id).await?;
        Ok(SessionEventCursor {
            agent_session_id: session_id.clone(),
            seq: head.last_seq,
        })
    }

    pub async fn session_resources(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Vec<TypedResourceBinding>, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let rows = sqlx::query_as::<_, StoredResourceRow>(
            "SELECT binding_id, resource_kind, resource_id, owner_id, operations_json, \
                    connection_config_ref, typed_parameters_json \
             FROM agent_session_resources WHERE session_id = ? ORDER BY binding_id",
        )
        .bind(session_id.as_ref())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(resource_from_row).collect()
    }

    pub async fn read_events(
        &self,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionEventPage, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let after_seq = validate_cursor(session_id, after)?;
        let page =
            Self::read_event_page_tx(&mut tx, session_id, after_seq, head.last_seq, limit).await?;
        tx.commit().await?;
        Ok(page)
    }

    async fn read_event_page_tx(
        tx: &mut Transaction<'_, Sqlite>,
        session_id: &AgentSessionId,
        after_seq: u64,
        last_seq: u64,
        limit: u32,
    ) -> Result<SessionEventPage, SessionStoreError> {
        if after_seq > last_seq {
            return Err(SessionStoreError::InvalidEvent(
                "cursor is ahead of the committed AgentSession sequence".to_owned(),
            ));
        }
        let limit = limit.clamp(1, MAX_EVENT_PAGE_SIZE);
        let rows = sqlx::query_as::<_, StoredEventRow>(
            "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                    runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                    correlation_id, causation_event_id, inline_json, payload_id \
             FROM agent_events \
             WHERE session_id = ? AND seq > ? \
             ORDER BY seq ASC LIMIT ?",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(after_seq, "cursor")?)
        .bind(i64::from(limit))
        .fetch_all(&mut **tx)
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

    /// Read the canonical turn receipt for an exact
    /// `(AgentSessionId, OperationId)` pair.
    ///
    /// Both reads are bounded by `LIMIT 1` and share one read-only
    /// transaction. A terminal event is considered part of the selected turn
    /// only when it was committed after the original matching `turn/started`.
    /// The first terminal fact wins; the write-side lifecycle fence prevents a
    /// later terminal from replacing it. No message content, projection state,
    /// or timeout is used to infer completion.
    pub async fn read_turn_receipt(
        &self,
        session_id: &AgentSessionId,
        operation_id: &OperationId,
    ) -> Result<TurnReceipt, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;

        let turn = sqlx::query_as::<_, StoredTurnRow>(
            "SELECT turn_id, operation_id, state, started_event_id, terminal_event_id \
             FROM agent_turns WHERE session_id = ? AND operation_id = ?",
        )
        .bind(session_id.as_ref())
        .bind(operation_id.as_ref())
        .fetch_optional(&mut *tx)
        .await?;

        let Some(turn) = turn else {
            tx.commit().await?;
            return Ok(TurnReceipt {
                agent_session_id: session_id.clone(),
                operation_id: operation_id.clone(),
                status: TurnReceiptStatus::NotFound,
                started_event: None,
                terminal_event: None,
            });
        };
        if turn.turn_id != turn.operation_id || turn.operation_id != operation_id.as_ref() {
            return Err(SessionStoreError::InvalidSession(
                "canonical Agent Turn identity is inconsistent".to_owned(),
            ));
        }
        let started_event = event_by_event_id_tx(&mut tx, &turn.started_event_id)
            .await?
            .map(event_from_row)
            .transpose()?
            .ok_or_else(|| SessionStoreError::InvalidSession(
                "canonical Agent Turn is missing its started event".to_owned(),
            ))?;
        let terminal_event = match &turn.terminal_event_id {
            Some(event_id) => Some(
                event_by_event_id_tx(&mut tx, event_id)
                    .await?
                    .map(event_from_row)
                    .transpose()?
                    .ok_or_else(|| SessionStoreError::InvalidSession(
                        "canonical Agent Turn is missing its terminal event".to_owned(),
                    ))?,
            ),
            None => None,
        };
        let status = match turn.state.as_str() {
            "accepted" | "running" => TurnReceiptStatus::Running,
            "completed" => TurnReceiptStatus::Completed,
            "failed" => TurnReceiptStatus::Failed,
            "cancelled" | "interrupted" => TurnReceiptStatus::Cancelled,
            state => return Err(SessionStoreError::InvalidSession(format!(
                "canonical Agent Turn has unknown state {state}"
            ))),
        };

        tx.commit().await?;
        Ok(TurnReceipt {
            agent_session_id: session_id.clone(),
            operation_id: operation_id.clone(),
            status,
            started_event: Some(started_event),
            terminal_event,
        })
    }

    pub async fn read_effect(
        &self,
        session_id: &AgentSessionId,
        effect_id: &str,
    ) -> Result<Option<AgentEffectRecord>, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let row = sqlx::query_as::<_, StoredEffectRow>(
            "SELECT effect_id, session_id, turn_id, operation_id, owner_domain, \
                    capability_module, action_id, resource_binding_id, resource_key, \
                    input_digest, strategy, state, bounded_observation_json, \
                    started_event_id, terminal_event_id, created_at, settled_at \
             FROM agent_effects WHERE session_id = ? AND effect_id = ?",
        )
        .bind(session_id.as_ref())
        .bind(effect_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(effect_from_row).transpose()
    }

    pub async fn has_unsettled_effects(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<bool, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let unsettled: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_effects \
             WHERE session_id = ? AND state IN ('pending', 'unknown'))",
        )
        .bind(session_id.as_ref())
        .fetch_one(&self.pool)
        .await?;
        Ok(unsettled != 0)
    }

    pub async fn list_effects(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Vec<AgentEffectRecord>, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let rows = sqlx::query_as::<_, StoredEffectRow>(
            "SELECT effect_id, session_id, turn_id, operation_id, owner_domain, \
                    capability_module, action_id, resource_binding_id, resource_key, \
                    input_digest, strategy, state, bounded_observation_json, \
                    started_event_id, terminal_event_id, created_at, settled_at \
             FROM agent_effects WHERE session_id = ? \
             ORDER BY created_at DESC, effect_id DESC",
        )
        .bind(session_id.as_ref())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(effect_from_row).collect()
    }

    pub async fn effect_causation_event_id(
        &self,
        session_id: &AgentSessionId,
        turn_id: &OperationId,
        operation_id: &OperationId,
        capability_module: &nomifun_agent_contracts::CapabilityId,
        action_id: &nomifun_agent_contracts::ActionId,
    ) -> Result<EventId, SessionStoreError> {
        require_live_session(&self.pool, session_id.as_ref()).await?;
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT e.event_id FROM agent_events e \
             JOIN agent_turns t ON t.session_id = e.session_id \
              AND t.turn_id = ? AND t.started_event_id = e.causation_event_id \
              AND t.state IN ('accepted', 'running') \
             WHERE e.session_id = ? AND e.kind = 'tool/call-started' \
              AND json_extract(e.inline_json, '$.operation_id') = ? \
              AND json_extract(e.inline_json, '$.capability_id') = ? \
              AND json_extract(e.inline_json, '$.action_id') = ? \
             ORDER BY e.seq LIMIT 2",
        )
        .bind(turn_id.as_ref())
        .bind(session_id.as_ref())
        .bind(operation_id.as_ref())
        .bind(capability_module.as_ref())
        .bind(action_id.as_ref())
        .fetch_all(&self.pool)
        .await?;
        match rows.as_slice() {
            [event_id] => Ok(EventId::from(event_id.clone())),
            [] => Err(SessionStoreError::InvalidEvent(
                "effect requires its exact committed tool/call-started predecessor".into(),
            )),
            _ => Err(SessionStoreError::InvalidEvent(
                "effect tool/call-started predecessor is ambiguous".into(),
            )),
        }
    }

    pub async fn observe(
        &self,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionObservation, SessionStoreError> {
        // All response fields must describe one committed SQLite snapshot.
        // Reacquiring the pool between reads lets concurrent appends mix epochs.
        let mut tx = self.pool.begin().await?;
        let session = require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let after_seq = validate_cursor(session_id, after)?;
        let page = Self::read_event_page_tx(&mut tx, session_id, after_seq, head.last_seq, limit).await?;
        let messages =
            Self::messages_after_tx(&mut tx, session_id, after_seq, head.last_seq).await?;
        tx.commit().await?;
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

        let mut tx = self.begin_write_transaction().await?;
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
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(head)
    }

    pub async fn messages_after(
        &self,
        session_id: &AgentSessionId,
        after_seq: u64,
    ) -> Result<Vec<MessageProjection>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let messages = Self::messages_after_tx(&mut tx, session_id, after_seq, head.last_seq).await?;
        tx.commit().await?;
        Ok(messages)
    }

    pub async fn messages_before(
        &self,
        session_id: &AgentSessionId,
        before_seq: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<MessageProjection>, bool, u64), SessionStoreError> {
        if limit == 0 || limit > MAX_EVENT_PAGE_SIZE {
            return Err(SessionStoreError::InvalidEvent(format!(
                "message projection page limit must be between 1 and {MAX_EVENT_PAGE_SIZE}",
            )));
        }
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let boundary = before_seq.unwrap_or(head.last_seq.saturating_add(1));
        if boundary > head.last_seq.saturating_add(1) {
            return Err(SessionStoreError::InvalidEvent(
                "message projection cursor is ahead of the committed AgentSession sequence"
                    .to_owned(),
            ));
        }
        let rows = sqlx::query_as::<_, StoredProjectionRow>(
            "SELECT session_id, projection_id, first_seq, last_seq, presentation_intent, \
                    projection_json, semantic_digest \
             FROM agent_messages \
             WHERE session_id = ? AND first_seq < ? \
               AND presentation_intent IN ('message', 'tool', 'agent_transition') \
             ORDER BY first_seq DESC, projection_id DESC LIMIT ?",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(boundary, "before_seq")?)
        .bind(i64::from(limit) + 1)
        .fetch_all(&mut *tx)
        .await?;
        let has_more = rows.len() > limit as usize;
        let messages = rows
            .into_iter()
            .take(limit as usize)
            .map(projection_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM agent_messages WHERE session_id = ? \
               AND presentation_intent IN ('message', 'tool', 'agent_transition')",
        )
        .bind(session_id.as_ref())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((messages, has_more, as_u64(total, "message projection total")?))
    }

    /// Read the renderer-facing conversation history. In addition to canonical
    /// message/tool/thinking projections, expose one derived lifecycle summary for every
    /// durable Turn. The summary is reconstructed from `agent_turns`, so older
    /// Sessions created before this view existed receive the same cold-reload
    /// behavior without mutating their event log or projection tables.
    pub async fn message_history_before(
        &self,
        session_id: &AgentSessionId,
        before_seq: Option<u64>,
        limit: u32,
    ) -> Result<(Vec<MessageProjection>, bool, u64), SessionStoreError> {
        if limit == 0 || limit > MAX_EVENT_PAGE_SIZE {
            return Err(SessionStoreError::InvalidEvent(format!(
                "message history page limit must be between 1 and {MAX_EVENT_PAGE_SIZE}",
            )));
        }
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        let boundary = before_seq.unwrap_or(head.last_seq.saturating_add(1));
        if boundary > head.last_seq.saturating_add(1) {
            return Err(SessionStoreError::InvalidEvent(
                "message history cursor is ahead of the committed AgentSession sequence"
                    .to_owned(),
            ));
        }
        let query_limit = i64::from(limit) + 1;
        let projection_rows = sqlx::query_as::<_, StoredProjectionRow>(
            "SELECT session_id, projection_id, first_seq, last_seq, presentation_intent, \
                    projection_json, semantic_digest \
             FROM agent_messages \
             WHERE session_id = ? AND first_seq < ? \
               AND presentation_intent IN ('message', 'tool', 'agent_transition', 'thinking') \
             ORDER BY first_seq DESC, projection_id DESC LIMIT ?",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(boundary, "before_seq")?)
        .bind(query_limit)
        .fetch_all(&mut *tx)
        .await?;
        let turn_rows = sqlx::query_as::<_, StoredTurnHistoryRow>(
            "SELECT session_id, turn_id, source_message_id, state, result_json, error_json, \
                    started_event_id, accepted_at, started_at, finished_at \
             FROM agent_turns \
             WHERE session_id = ? AND source_message_id IS NOT NULL \
               AND COALESCE(started_at, accepted_at) < ? \
             ORDER BY COALESCE(started_at, accepted_at) DESC, turn_id DESC LIMIT ?",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(boundary, "before_seq")?)
        .bind(query_limit)
        .fetch_all(&mut *tx)
        .await?;

        let mut history = projection_rows
            .into_iter()
            .map(projection_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        for row in turn_rows {
            if let Some(summary) = turn_history_projection_from_row(row)? {
                history.push(summary);
            }
        }
        history.sort_by(|left, right| {
            right
                .first_seq
                .cmp(&left.first_seq)
                .then_with(|| right.projection_id.cmp(&left.projection_id))
        });
        let has_more = history.len() > limit as usize;
        history.truncate(limit as usize);

        let message_total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM agent_messages WHERE session_id = ? \
               AND presentation_intent IN ('message', 'tool', 'agent_transition', 'thinking')",
        )
        .bind(session_id.as_ref())
        .fetch_one(&mut *tx)
        .await?;
        let turn_total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM agent_turns WHERE session_id = ? AND source_message_id IS NOT NULL",
        )
        .bind(session_id.as_ref())
        .fetch_one(&mut *tx)
        .await?;
        let total = message_total.checked_add(turn_total).ok_or_else(|| {
            SessionStoreError::InvalidSession("message history total overflowed".to_owned())
        })?;
        tx.commit().await?;
        Ok((history, has_more, as_u64(total, "message history total")?))
    }

    async fn messages_after_tx(
        tx: &mut Transaction<'_, Sqlite>,
        session_id: &AgentSessionId,
        after_seq: u64,
        last_seq: u64,
    ) -> Result<Vec<MessageProjection>, SessionStoreError> {
        if after_seq > last_seq {
            return Err(SessionStoreError::InvalidEvent(
                "projection cursor is ahead of the committed AgentSession sequence".to_owned(),
            ));
        }
        let rows = sqlx::query_as::<_, StoredProjectionRow>(
            "SELECT session_id, projection_id, first_seq, last_seq, presentation_intent, \
                    projection_json, semantic_digest \
             FROM agent_messages \
             WHERE session_id = ? AND last_seq > ? \
             ORDER BY first_seq ASC, projection_id ASC",
        )
        .bind(session_id.as_ref())
        .bind(as_i64(after_seq, "after_seq")?)
        .fetch_all(&mut **tx)
        .await?;
        rows.into_iter().map(projection_from_row).collect()
    }

    pub async fn rebuild_projections(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionHeadProjection, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        sqlx::query("DELETE FROM agent_messages WHERE session_id = ?")
            .bind(session_id.as_ref())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_session_heads WHERE session_id = ?")
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
            if event_uses_agent_messages(self.registry.entry(&event.kind, event.kind_version)?)
            {
                let existing = projection_by_identity_tx(&mut tx, &event).await?;
                let projection = reduce_agent_messages(existing, &event, &payload)?;
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
             FROM agent_events \
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
             FROM agent_events \
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
        self.append_event(&effect_append(request, "effect/started")?)
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
        let append = effect_append(request, kind)?;
        let mut tx = self.begin_write_transaction().await?;
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::EffectSettlement,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reconcile_effect(
        &self,
        mut request: EffectEventRequest,
        outcome: EffectReconcileOutcome,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        request.payload =
            SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(outcome)?));
        let append = effect_append(request, "effect/reconciled")?;
        let mut tx = self.begin_write_transaction().await?;
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::EffectSettlement,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn record_resource_cleanup_started(
        &self,
        session_id: &AgentSessionId,
        owner_domain: &str,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_resource_cleanup_domain(owner_domain)?;
        let identity = format!(
            "resource-cleanup-started:{}:{owner_domain}",
            session_id.as_ref()
        );
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("resource/cleanup-started".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "owner_domain": owner_domain,
                    "outcome": "pending",
                }))),
            },
        };
        let mut tx = self.begin_write_transaction().await?;
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::DeleteCleanupUncertainty,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn record_resource_cleanup_succeeded(
        &self,
        session_id: &AgentSessionId,
        owner_domain: &str,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_resource_cleanup_domain(owner_domain)?;
        let identity = format!(
            "resource-cleanup-succeeded:{}:{owner_domain}",
            session_id.as_ref()
        );
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("resource/cleanup-succeeded".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "owner_domain": owner_domain,
                    "outcome": "succeeded",
                }))),
            },
        };
        let mut tx = self.begin_write_transaction().await?;
        if duplicate_event_tx(&mut tx, &append).await?.is_none() {
            let blockers = delete_blockers_tx(&mut tx, session_id.as_ref()).await?;
            if !blockers
                .resource_cleanup_pending
                .iter()
                .any(|domain| domain == owner_domain)
            {
                let reconciled: i64 = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM agent_events \
                     WHERE session_id = ? AND kind = 'resource/cleanup-reconciled' \
                       AND json_extract(inline_json, '$.owner_domain') = ?)",
                )
                .bind(session_id.as_ref())
                .bind(owner_domain)
                .fetch_one(&mut *tx)
                .await?;
                if reconciled == 0 {
                    return Err(SessionStoreError::Conflict(format!(
                        "resource owner {owner_domain} has no pending cleanup"
                    )));
                }
            }
        }
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::DeleteCleanupUncertainty,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Persist an owner-domain cleanup quarantine after the delete admission
    /// fence has committed. The payload intentionally excludes transport
    /// diagnostics and credentials; retries reuse the same immutable fact.
    pub async fn record_resource_cleanup_uncertain(
        &self,
        session_id: &AgentSessionId,
        owner_domain: &str,
        recorded_at: i64,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_resource_cleanup_domain(owner_domain)?;
        if recorded_at < 0 {
            return Err(SessionStoreError::InvalidEvent(
                "resource cleanup uncertainty requires a canonical owner domain and timestamp"
                    .to_owned(),
            ));
        }
        let identity = format!(
            "resource-cleanup-uncertain:{}:{owner_domain}",
            session_id.as_ref()
        );
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("resource/cleanup-uncertain".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "owner_domain": owner_domain,
                    "outcome": "unknown",
                    "recorded_at": recorded_at,
                    "recovery": "external_reconciliation_required",
                }))),
            },
        };
        let mut tx = self.begin_write_transaction().await?;
        if duplicate_event_tx(&mut tx, &append).await?.is_none() {
            let blockers = delete_blockers_tx(&mut tx, session_id.as_ref()).await?;
            if !blockers
                .resource_cleanup_pending
                .iter()
                .any(|domain| domain == owner_domain)
            {
                return Err(SessionStoreError::Conflict(format!(
                    "resource owner {owner_domain} has no pending cleanup"
                )));
            }
        }
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::DeleteCleanupUncertainty,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reconcile_resource_cleanup_for_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        owner_domain: &str,
        evidence_digest: &DigestHex,
        recorded_at: i64,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_cleanup_reconciliation(owner_domain, evidence_digest, recorded_at)?;
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "deleting" {
            return Err(SessionStoreError::Conflict(
                "resource cleanup reconciliation requires a deleting AgentSession".to_owned(),
            ));
        }
        let identity = format!(
            "resource-cleanup-reconciled:{}:{owner_domain}:{}",
            session_id.as_ref(),
            evidence_digest.as_ref()
        );
        if let Some(existing) = event_by_event_id_tx(&mut tx, &identity).await? {
            let record = event_from_row(existing)?;
            let payload = effect_payload_from_event(&record)?;
            if record.kind.0 != "resource/cleanup-reconciled"
                || payload.get("owner_domain").and_then(Value::as_str) != Some(owner_domain)
                || payload.get("evidence_digest").and_then(Value::as_str)
                    != Some(evidence_digest.as_ref())
            {
                return Err(SessionStoreError::IdempotencyConflict(
                    "resource cleanup evidence was reused for different input".to_owned(),
                ));
            }
            let ack = event_ack(&record);
            tx.commit().await?;
            return Ok(SessionEventAppendResult {
                record: Some(record),
                ack: Some(ack.clone()),
                cursor: ack.cursor,
                persisted: true,
                duplicate: true,
            });
        }
        let blockers = delete_blockers_tx(&mut tx, session_id.as_ref()).await?;
        if !blockers
            .resource_cleanup_uncertainties
            .iter()
            .any(|uncertainty| uncertainty.owner_domain == owner_domain)
        {
            return Err(SessionStoreError::Conflict(format!(
                "resource owner {owner_domain} has no unresolved cleanup uncertainty"
            )));
        }
        let append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("resource/cleanup-reconciled".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "owner_domain": owner_domain,
                    "outcome": "confirmed_safe_to_delete",
                    "evidence_digest": evidence_digest,
                    "recorded_at": recorded_at,
                }))),
            },
        };
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::DeleteCleanupUncertainty,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn reconcile_effect_for_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        effect_id: &str,
        confirmed_succeeded: bool,
        evidence_digest: &DigestHex,
        recorded_at: i64,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_cleanup_reconciliation("effect", evidence_digest, recorded_at)?;
        if effect_id.is_empty() || effect_id.len() > 512 || effect_id.trim() != effect_id {
            return Err(SessionStoreError::InvalidEvent(
                "effect reconciliation requires a canonical effect_id".to_owned(),
            ));
        }
        let mut tx = self.begin_write_transaction().await?;
        let session_row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&session_row, owner)?;
        if session_row.state != "deleting" {
            return Err(SessionStoreError::Conflict(
                "delete effect reconciliation requires a deleting AgentSession".to_owned(),
            ));
        }
        let reconciliation_event_id = format!(
            "effect-delete-reconciled:{}:{}:{}",
            session_id.as_ref(),
            effect_id,
            evidence_digest.as_ref()
        );
        if let Some(existing) = event_by_event_id_tx(&mut tx, &reconciliation_event_id).await? {
            let record = event_from_row(existing)?;
            let payload = effect_payload_from_event(&record)?;
            let expected_outcome = if confirmed_succeeded {
                "confirmed_succeeded"
            } else {
                "confirmed_failed"
            };
            let stored_evidence = if confirmed_succeeded {
                payload
                    .get("receipt")
                    .and_then(|receipt| receipt.get("evidence_digest"))
                    .and_then(Value::as_str)
            } else {
                payload.get("evidence_digest").and_then(Value::as_str)
            };
            if record.kind.0 != "effect/reconciled"
                || record.correlation_id.as_ref() != effect_id
                || payload.get("outcome").and_then(Value::as_str) != Some(expected_outcome)
                || stored_evidence != Some(evidence_digest.as_ref())
            {
                return Err(SessionStoreError::IdempotencyConflict(
                    "effect reconciliation evidence was reused for different input".to_owned(),
                ));
            }
            let ack = event_ack(&record);
            tx.commit().await?;
            return Ok(SessionEventAppendResult {
                record: Some(record),
                ack: Some(ack.clone()),
                cursor: ack.cursor,
                persisted: true,
                duplicate: true,
            });
        }
        let row = sqlx::query_as::<_, StoredEffectRow>(
            "SELECT effect_id, session_id, turn_id, operation_id, owner_domain, \
                    capability_module, action_id, resource_binding_id, resource_key, \
                    input_digest, strategy, state, bounded_observation_json, \
                    started_event_id, terminal_event_id, created_at, settled_at \
             FROM agent_effects WHERE session_id = ? AND effect_id = ?",
        )
        .bind(session_id.as_ref())
        .bind(effect_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| SessionStoreError::NotFound(effect_id.to_owned()))?;
        let effect = effect_from_row(row)?;
        if effect.state != AgentEffectState::Unknown {
            return Err(SessionStoreError::Conflict(
                "only an unknown effect may be externally reconciled for delete".to_owned(),
            ));
        }
        let uncertain_event_id = effect.terminal_event_id.clone().ok_or_else(|| {
            SessionStoreError::InvalidEvent(
                "unknown effect has no terminal uncertainty event".to_owned(),
            )
        })?;
        let started_event = event_by_event_id_tx(&mut tx, effect.started_event_id.as_ref())
            .await?
            .ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "unknown effect has no started event".to_owned(),
                )
            })?;
        let outcome = if confirmed_succeeded {
            json!({
                "outcome": "confirmed_succeeded",
                "receipt": {
                    "evidence_digest": evidence_digest,
                    "reconciled_for": "delete"
                }
            })
        } else {
            json!({
                "outcome": "confirmed_failed",
                "error": "EXTERNAL_EFFECT_CONFIRMED_FAILED",
                "evidence_digest": evidence_digest,
            })
        };
        let request = EffectEventRequest {
            agent_session_id: session_id.clone(),
            effect_id: effect.effect_id,
            turn_id: effect.turn_id,
            operation_id: effect.operation_id,
            owner_domain: effect.owner_domain,
            capability_module: effect.capability_module,
            action_id: effect.action_id,
            resource_binding_id: effect.resource_binding_id,
            resource_key: effect.resource_key,
            input_digest: effect.input_digest,
            recorded_at,
            event_id: EventId::from(reconciliation_event_id),
            producer_id: EventProducerId::from("session_api:delete_reconciliation"),
            idempotency_key: IdempotencyKey::from(started_event.idempotency_key),
            correlation_id: CorrelationId::from(effect_id.to_owned()),
            strategy: effect.strategy,
            causation_event_id: Some(uncertain_event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(outcome)),
        };
        let append = effect_append(request, "effect/reconciled")?;
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::EffectSettlement,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn override_unknown_effect_for_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        effect_id: &str,
        reason_digest: &DigestHex,
        recorded_at: i64,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_cleanup_reconciliation("effect", reason_digest, recorded_at)?;
        if effect_id.is_empty() || effect_id.len() > 512 || effect_id.trim() != effect_id {
            return Err(SessionStoreError::InvalidEvent(
                "delete override requires a canonical effect_id".to_owned(),
            ));
        }
        let identity = format!(
            "deletion-effect-override:{}:{}:{}",
            session_id.as_ref(),
            effect_id,
            reason_digest.as_ref()
        );
        let mut append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api:manual_delete_override"),
            idempotency_key: IdempotencyKey::from(identity.clone()),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("deletion/effect-override".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "effect_id": effect_id,
                    "authority": "installation_owner_manual_override",
                    "risk_acknowledged": true,
                    "reason_digest": reason_digest,
                    "recorded_at": recorded_at,
                }))),
            },
        };
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "deleting" {
            return Err(SessionStoreError::Conflict(
                "effect delete override requires a deleting AgentSession".to_owned(),
            ));
        }
        let existing_audit = deletion_audit_by_id_tx(&mut tx, &identity).await?;
        if existing_audit.is_none() {
            let effect_state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM agent_effects WHERE session_id = ? AND effect_id = ?",
            )
            .bind(session_id.as_ref())
            .bind(effect_id)
            .fetch_optional(&mut *tx)
            .await?;
            if effect_state.as_deref() != Some("unknown") {
                return Err(SessionStoreError::Conflict(
                    "manual delete override may target only an unknown effect".to_owned(),
                ));
            }
        }
        let audit = record_deletion_audit_tx(
            &mut tx,
            &identity,
            session_id,
            owner,
            "effect",
            effect_id,
            reason_digest,
            recorded_at,
        )
        .await?;
        append.semantic_event.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "effect_id": effect_id,
            "authority": "installation_owner_manual_override",
            "risk_acknowledged": true,
            "reason_digest": reason_digest,
            "recorded_at": audit.recorded_at,
        })));
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::DeleteCleanupUncertainty,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn override_resource_cleanup_for_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        owner_domain: &str,
        reason_digest: &DigestHex,
        recorded_at: i64,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_cleanup_reconciliation(owner_domain, reason_digest, recorded_at)?;
        let identity = format!(
            "resource-cleanup-override:{}:{owner_domain}:{}",
            session_id.as_ref(),
            reason_digest.as_ref()
        );
        let mut append = SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api:manual_delete_override"),
            idempotency_key: IdempotencyKey::from(identity.clone()),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("resource/cleanup-override".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "owner_domain": owner_domain,
                    "authority": "installation_owner_manual_override",
                    "risk_acknowledged": true,
                    "reason_digest": reason_digest,
                    "recorded_at": recorded_at,
                }))),
            },
        };
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        if row.state != "deleting" {
            return Err(SessionStoreError::Conflict(
                "resource cleanup override requires a deleting AgentSession".to_owned(),
            ));
        }
        let existing_audit = deletion_audit_by_id_tx(&mut tx, &identity).await?;
        if existing_audit.is_none() {
            let blockers = delete_blockers_tx(&mut tx, session_id.as_ref()).await?;
            if !blockers
                .resource_cleanup_uncertainties
                .iter()
                .any(|uncertainty| uncertainty.owner_domain == owner_domain)
            {
                return Err(SessionStoreError::Conflict(format!(
                    "resource owner {owner_domain} has no unknown cleanup to override"
                )));
            }
        }
        let audit = record_deletion_audit_tx(
            &mut tx,
            &identity,
            session_id,
            owner,
            "resource_cleanup",
            owner_domain,
            reason_digest,
            recorded_at,
        )
        .await?;
        append.semantic_event.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "owner_domain": owner_domain,
            "authority": "installation_owner_manual_override",
            "risk_acknowledged": true,
            "reason_digest": reason_digest,
            "recorded_at": audit.recorded_at,
        })));
        let result = self
            .append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::DeleteCleanupUncertainty,
            )
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn deletion_audits(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<Vec<AgentDeletionAuditRecord>, SessionStoreError> {
        validate_principal(owner)?;
        let mut tx = self.pool.begin().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&row, owner)?;
        let rows = sqlx::query_as::<_, StoredDeletionAuditRow>(
            "SELECT audit_id, agent_session_id, owner_ref_json, target_kind, target_id, \
                    authority, risk_acknowledged, reason_digest, recorded_at \
             FROM agent_deletion_audits WHERE agent_session_id = ? \
             ORDER BY recorded_at, audit_id",
        )
        .bind(session_id.as_ref())
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter().map(deletion_audit_from_row).collect()
    }

    /// Return the exact durable facts that currently prevent physical purge.
    /// This read is valid only after the Session has entered `deleting`.
    pub async fn delete_blockers(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<AgentSessionDeleteBlockers, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if row.state != "deleting" {
            return Err(SessionStoreError::Conflict(
                "delete blockers may only be inspected after the admission fence".to_owned(),
            ));
        }
        let blockers = delete_blockers_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(blockers)
    }

    /// A restarted process cannot recover the execution owner behind a
    /// persisted `effect/started`. Convert those pending effects to durable
    /// unknown outcomes before startup deletion recovery proceeds. No effect
    /// is reported failed or replay-safe merely because its process vanished.
    pub async fn quarantine_pending_effects_for_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        recorded_at: i64,
    ) -> Result<u64, SessionStoreError> {
        if recorded_at < 0 {
            return Err(SessionStoreError::InvalidEvent(
                "effect recovery timestamp must not be negative".to_owned(),
            ));
        }
        let mut tx = self.begin_write_transaction().await?;
        let session_row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
        require_owner(&session_row, owner)?;
        if session_row.state != "deleting" {
            return Err(SessionStoreError::Conflict(
                "pending effect quarantine requires a deleting AgentSession".to_owned(),
            ));
        }
        let rows = sqlx::query_as::<_, StoredEffectRow>(
            "SELECT effect_id, session_id, turn_id, operation_id, owner_domain, \
                    capability_module, action_id, resource_binding_id, resource_key, \
                    input_digest, strategy, state, bounded_observation_json, \
                    started_event_id, terminal_event_id, created_at, settled_at \
             FROM agent_effects WHERE session_id = ? AND state = 'pending' \
             ORDER BY effect_id",
        )
        .bind(session_id.as_ref())
        .fetch_all(&mut *tx)
        .await?;
        let mut quarantined = 0_u64;
        for row in rows {
            let effect = effect_from_row(row)?;
            let started_event = event_by_event_id_tx(&mut tx, effect.started_event_id.as_ref())
                .await?
                .ok_or_else(|| {
                    SessionStoreError::InvalidEvent(
                        "pending effect has no started event".to_owned(),
                    )
                })?;
            let request = EffectEventRequest {
                agent_session_id: session_id.clone(),
                effect_id: effect.effect_id.clone(),
                turn_id: effect.turn_id,
                operation_id: effect.operation_id,
                owner_domain: effect.owner_domain,
                capability_module: effect.capability_module,
                action_id: effect.action_id,
                resource_binding_id: effect.resource_binding_id,
                resource_key: effect.resource_key,
                input_digest: effect.input_digest,
                recorded_at,
                event_id: EventId::from(format!(
                    "effect-delete-recovery:{}:{}",
                    session_id.as_ref(),
                    effect.effect_id
                )),
                producer_id: EventProducerId::from("runtime_supervisor"),
                idempotency_key: IdempotencyKey::from(started_event.idempotency_key),
                correlation_id: CorrelationId::from(effect.effect_id),
                strategy: effect.strategy,
                causation_event_id: Some(effect.started_event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "outcome": "unknown",
                    "recovery": "process_restart_external_reconciliation_required",
                }))),
            };
            let append = effect_append(request, "effect/uncertain")?;
            self.append_event_tx_with_policy(
                &mut tx,
                &append,
                None,
                AppendSessionStatePolicy::EffectSettlement,
            )
            .await?;
            quarantined = quarantined.saturating_add(1);
        }
        tx.commit().await?;
        Ok(quarantined)
    }

    pub async fn quarantine_pending_resource_cleanups_for_delete(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        recorded_at: i64,
    ) -> Result<u64, SessionStoreError> {
        if recorded_at < 0 {
            return Err(SessionStoreError::InvalidEvent(
                "resource cleanup recovery timestamp must not be negative".to_owned(),
            ));
        }
        let pending = {
            let mut tx = self.pool.begin().await?;
            let row = session_row_by_id_tx(&mut tx, session_id.as_ref()).await?;
            require_owner(&row, owner)?;
            if row.state != "deleting" {
                return Err(SessionStoreError::Conflict(
                    "resource cleanup recovery requires a deleting AgentSession".to_owned(),
                ));
            }
            let pending = delete_blockers_tx(&mut tx, session_id.as_ref())
                .await?
                .resource_cleanup_pending;
            tx.commit().await?;
            pending
        };
        let mut quarantined = 0_u64;
        for owner_domain in pending {
            self.record_resource_cleanup_uncertain(
                session_id,
                &owner_domain,
                recorded_at,
            )
            .await?;
            quarantined = quarantined.saturating_add(1);
        }
        Ok(quarantined)
    }

    pub async fn automation_config(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<AgentSessionAutomationConfig, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let config = automation_config_tx(&mut tx, session_id.as_ref()).await?;
        tx.commit().await?;
        Ok(config)
    }

    pub async fn commit_automation_config(
        &self,
        request: CommitAgentSessionAutomationConfig,
    ) -> Result<AgentSessionAutomationConfig, SessionStoreError> {
        validate_automation_config_request(&request)?;
        let mut tx = self.begin_write_transaction().await?;
        let row = session_row_by_id_tx(&mut tx, request.agent_session_id.as_ref()).await?;
        require_owner(&row, &request.owner_ref)?;
        require_live_row(row)?;
        if let Some(operation_id) = request.operation_id.as_deref() {
            let identity = format!(
                "automation-config:{}:{operation_id}",
                request.agent_session_id.as_ref()
            );
            if let Some(existing) = event_by_event_id_tx(&mut tx, &identity).await? {
                let event = event_from_row(existing)?;
                let SessionEventPayloadRef::InlineJson(payload) = event.payload else {
                    return Err(SessionStoreError::InvalidEvent(
                        "AutoWork config event lost its inline receipt".to_owned(),
                    ));
                };
                let (expected_revision, committed) =
                    automation_config_event_from_value(payload.0)?;
                if event.kind.0 != "automation/config-committed"
                    || expected_revision != request.expected_revision
                    || committed.enabled != request.enabled
                    || committed.tag != request.tag
                    || committed.max_requirements != request.max_requirements
                    || committed.operation_id.as_deref() != Some(operation_id)
                {
                    return Err(SessionStoreError::IdempotencyConflict(
                        "AutoWork config operation was replayed with different input".to_owned(),
                    ));
                }
                tx.commit().await?;
                return Ok(committed);
            }
        }
        let current = automation_config_tx(&mut tx, request.agent_session_id.as_ref()).await?;
        if current.revision != request.expected_revision {
            return Err(SessionStoreError::Conflict(
                "AutoWork config revision changed concurrently".to_owned(),
            ));
        }
        let unchanged = current.enabled == request.enabled
            && current.tag == request.tag
            && current.max_requirements == request.max_requirements;
        if unchanged && request.operation_id.is_none() {
            tx.commit().await?;
            return Ok(current);
        }
        let revision = if unchanged {
            current.revision
        } else {
            current.revision.checked_add(1).ok_or_else(|| {
                SessionStoreError::Conflict("AutoWork config revision overflow".to_owned())
            })?
        };
        let committed = AgentSessionAutomationConfig {
            enabled: request.enabled,
            tag: request.tag,
            max_requirements: request.max_requirements,
            revision,
            operation_id: request.operation_id.clone(),
        };
        let nonce = request
            .operation_id
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let identity = format!(
            "automation-config:{}:{nonce}",
            request.agent_session_id.as_ref()
        );
        let session_correlation = request.agent_session_id.as_ref().to_owned();
        let append = SessionEventAppend {
            agent_session_id: request.agent_session_id,
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("automation/config-committed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(session_correlation),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "expected_revision": request.expected_revision,
                    "committed": committed,
                }))),
            },
        };
        self.append_event_tx(&mut tx, &append, None).await?;
        tx.commit().await?;
        Ok(committed)
    }

    pub async fn list_enabled_automation_configs(
        &self,
        owner: &PrincipalRef,
    ) -> Result<Vec<EnabledAgentSessionAutomationConfig>, SessionStoreError> {
        validate_principal(owner)?;
        let owner_json = serde_json::to_string(owner)?;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query_as::<_, StoredSessionRow>(
            "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, remote_binding_id, remote_binding_version, \
                    parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, created_at, deleted_at \
             FROM agent_sessions WHERE owner_ref_json = ? AND state = 'live' \
             ORDER BY agent_session_id",
        )
        .bind(owner_json)
        .fetch_all(&mut *tx)
        .await?;
        let mut enabled = Vec::new();
        for row in rows {
            let session = live_from_row(row)?;
            let config = automation_config_tx(&mut tx, session.agent_session_id.as_ref()).await?;
            if config.enabled {
                enabled.push(EnabledAgentSessionAutomationConfig { session, config });
            }
        }
        tx.commit().await?;
        Ok(enabled)
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
        if request.parent_through_seq > parent_head.last_seq {
            return Err(SessionStoreError::InvalidSession(
                "fork parent_through_seq exceeds the committed Session cursor".to_owned(),
            ));
        }
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
        insert_session_resources_tx(
            &mut tx,
            &request.child_session_id,
            &request.child_owner_ref,
            &request.child_agent_binding.typed_resource_bindings,
        )
        .await?;

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
                    "parent_through_seq": request.parent_through_seq,
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
                causation_event_id: Some(child_opening.event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "generation": 0,
                    "active_capability_ids": child_active_ids,
                    "active_set_digest": child_active_set_digest,
                    "delta": []
                }))),
            },
        };
        let _child_activation_ack = required_ack(
            self.append_event_tx(&mut tx, &child_activation, None)
                .await?,
        )?;
        let child_ready = SessionEventAppend {
            agent_session_id: request.child_session_id.clone(),
            event_id: EventId::from(format!(
                "child-ready:{}",
                request.idempotency_key.as_ref()
            )),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(format!(
                "{}:child-ready",
                request.idempotency_key.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("session/ready".to_owned()),
                kind_version: 1,
                correlation_id: request.correlation_id.clone(),
                causation_event_id: Some(child_opening.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "resolved_snapshot_ref": &request.child_agent_binding.resolved_snapshot_ref,
                }))),
            },
        };
        let child_ready_ack =
            required_ack(self.append_event_tx(&mut tx, &child_ready, None).await?)?;

        let fork_payload = SessionForkPayload {
            parent_session_id: parent_session_id.clone(),
            parent_through_seq: request.parent_through_seq,
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
            child_cursor: child_ready_ack.cursor,
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
            "deleted" => {
                return Err(SessionStoreError::Deleted(
                    command.agent_session_id.0.clone(),
                ));
            }
            "deleting" => {
                let live = live_from_row(row)?;
                tx.commit().await?;
                return Ok(AgentSessionDeletingRecord {
                    live,
                    delete_operation_id: command.operation_id.clone(),
                    admission_fenced_at: command.requested_at,
                });
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
        deleted_at: i64,
    ) -> Result<DeleteResult, SessionStoreError> {
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

        let blockers = delete_blockers_tx(&mut tx, command.agent_session_id.as_ref()).await?;
        if !blockers.is_empty() {
            return Err(SessionStoreError::Conflict(format!(
                "AgentSession delete remains fenced: {} unsettled effect(s), {} pending resource cleanup(s), {} resource cleanup uncertainty fact(s)",
                blockers.effects.len(),
                blockers.resource_cleanup_pending.len(),
                blockers.resource_cleanup_uncertainties.len(),
            )));
        }

        purge_private_content_tx(&mut tx, command.agent_session_id.as_ref()).await?;
        sqlx::query(
            "UPDATE agent_sessions SET \
                state = 'deleted', title = NULL, archived = NULL, pinned = NULL, \
                agent_binding_json = NULL, remote_binding_id = NULL, \
                remote_binding_version = NULL, parent_agent_session_id = NULL, \
                fork_base_payload_id = NULL, reasoning_effort = NULL, next_seq = NULL, created_at = NULL, \
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

    /// Read one exact deleting Session so a resource owner can resume its
    /// cleanup saga after restart without scanning or completing other rows.
    pub async fn get_deleting_session(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<AgentSessionLiveRecord, SessionStoreError> {
        let row = sqlx::query_as::<_, StoredSessionRow>(
            "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, remote_binding_id, remote_binding_version, \
                    parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, \
                    created_at, deleted_at \
             FROM agent_sessions WHERE agent_session_id = ? AND state = 'deleting'",
        )
        .bind(session_id.as_ref())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| SessionStoreError::NotFound(session_id.as_ref().to_owned()))?;
        live_from_row(row)
    }

    /// Enumerate fenced Sessions for the application-owned startup cleanup
    /// saga. This method never purges or completes a row on its own.
    pub async fn list_deleting_sessions(
        &self,
    ) -> Result<Vec<AgentSessionLiveRecord>, SessionStoreError> {
        let rows = sqlx::query_as::<_, StoredSessionRow>(
            "SELECT agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, remote_binding_id, remote_binding_version, \
                    parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, \
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
        self.append_event_tx_with_policy(
            tx,
            append,
            payload,
            AppendSessionStatePolicy::LiveOnly,
        )
        .await
    }

    async fn append_event_tx_with_policy(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        append: &SessionEventAppend,
        payload: Option<&SessionPayloadRecord>,
        state_policy: AppendSessionStatePolicy,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        validate_event_append(append)?;
        let registry_entry = self
            .registry
            .entry(
                &append.semantic_event.kind,
                append.semantic_event.kind_version,
            )?
            .clone();
        let session_row = session_row_by_id_tx(tx, append.agent_session_id.as_ref()).await?;
        let permitted_states: &[&str] = match state_policy {
            AppendSessionStatePolicy::LiveOnly => {
                require_live_row(session_row)?;
                &["live"]
            }
            AppendSessionStatePolicy::EffectSettlement => {
                if !matches!(
                    append.semantic_event.kind.0.as_str(),
                    "effect/succeeded"
                        | "effect/failed"
                        | "effect/uncertain"
                        | "effect/reconciled"
                ) {
                    return Err(SessionStoreError::InvalidEvent(
                        "deleting Session append authority is limited to effect settlement"
                            .to_owned(),
                    ));
                }
                match session_row.state.as_str() {
                    "live" | "deleting" => {}
                    "deleted" => {
                        return Err(SessionStoreError::Deleted(
                            append.agent_session_id.0.clone(),
                        ));
                    }
                    other => {
                        return Err(SessionStoreError::InvalidSession(format!(
                            "unknown AgentSession state {other}"
                        )));
                    }
                }
                &["live", "deleting"]
            }
            AppendSessionStatePolicy::DeleteCleanupUncertainty => {
                if !matches!(
                    append.semantic_event.kind.0.as_str(),
                    "resource/cleanup-started"
                        | "resource/cleanup-succeeded"
                        | "resource/cleanup-uncertain"
                        | "resource/cleanup-reconciled"
                        | "resource/cleanup-override"
                        | "deletion/effect-override"
                ) {
                    return Err(SessionStoreError::InvalidEvent(
                        "delete cleanup append authority is limited to cleanup reconciliation"
                            .to_owned(),
                    ));
                }
                match session_row.state.as_str() {
                    "deleting" => {}
                    "live" => {
                        return Err(SessionStoreError::Conflict(
                            "resource cleanup uncertainty requires a committed delete fence"
                                .to_owned(),
                        ));
                    }
                    "deleted" => {
                        return Err(SessionStoreError::Deleted(
                            append.agent_session_id.0.clone(),
                        ));
                    }
                    other => {
                        return Err(SessionStoreError::InvalidSession(format!(
                            "unknown AgentSession state {other}"
                        )));
                    }
                }
                &["deleting"]
            }
        };

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
        validate_turn_lifecycle_tx(tx, &head, append).await?;
        validate_predecessor_tx(tx, append, &registry_entry).await?;
        validate_runtime_sequence_tx(tx, append).await?;
        validate_effect_transition_tx(tx, append).await?;

        let stored_payload_value = validate_payload_reference_tx(tx, append).await?;

        let state_placeholders = std::iter::repeat_n("?", permitted_states.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE agent_sessions SET next_seq = next_seq + 1 \
             WHERE agent_session_id = ? AND state IN ({state_placeholders}) \
             RETURNING next_seq - 1"
        );
        let mut query = sqlx::query_scalar::<_, i64>(&sql).bind(append.agent_session_id.as_ref());
        for state in permitted_states {
            query = query.bind(state);
        }
        let seq = query
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| SessionStoreError::Deleted(append.agent_session_id.0.clone()))?;
        let record = record_from_append(append, as_u64(seq, "seq")?);
        insert_event_tx(tx, &record).await?;

        let payload_value = payload_value(&record, stored_payload_value);
        validate_semantic_event_tx(tx, &record, &payload_value).await?;
        project_turn_fact_tx(tx, &record, &payload_value).await?;
        project_effect_fact_tx(tx, &record, &payload_value).await?;
        let mut head = head_by_id_tx(tx, append.agent_session_id.as_ref()).await?;
        reduce_head(&mut head, &record, &payload_value)?;
        persist_head_tx(tx, &head).await?;

        if event_uses_agent_messages(&registry_entry) {
            let existing = projection_by_identity_tx(tx, &record).await?;
            let projection = reduce_agent_messages(existing, &record, &payload_value)?;
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

async fn validate_turn_lifecycle_tx(
    tx: &mut Transaction<'_, Sqlite>,
    head: &SessionHeadProjection,
    append: &SessionEventAppend,
) -> Result<(), SessionStoreError> {
    let kind = append.semantic_event.kind.0.as_str();
    let operation_id = append.semantic_event.correlation_id.as_ref();

    if kind == "turn/started" {
        if head.status != "ready" || head.active_turn_id.is_some() {
            return Err(SessionStoreError::Conflict(
                "turn start requires a ready Session with no active turn".to_owned(),
            ));
        }
        if let Some(existing_kind) =
            first_turn_lifecycle_kind_tx(tx, append.agent_session_id.as_ref(), operation_id)
                .await?
        {
            return Err(SessionStoreError::Conflict(format!(
                "turn operation {operation_id} already has a committed {existing_kind} fact"
            )));
        }
        return Ok(());
    }

    if !is_turn_terminal_kind(kind) {
        return Ok(());
    }

    if let Some(existing_kind) =
        first_turn_terminal_kind_tx(tx, append.agent_session_id.as_ref(), operation_id).await?
    {
        return Err(SessionStoreError::Conflict(format!(
            "turn operation {operation_id} already crossed the terminal fence with {existing_kind}"
        )));
    }
    if head.status != "running"
        || head.active_turn_id.as_deref() != Some(operation_id)
    {
        return Err(SessionStoreError::Conflict(
            "turn terminal requires the exact active turn boundary".to_owned(),
        ));
    }
    if !turn_started_exists_tx(tx, append.agent_session_id.as_ref(), operation_id).await? {
        return Err(SessionStoreError::Conflict(
            "turn terminal requires a committed turn/started event".to_owned(),
        ));
    }
    Ok(())
}

async fn project_turn_fact_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    let operation_id = event.correlation_id.as_ref();
    match event.kind.0.as_str() {
        "turn/started" => {
            let source_message_id = payload
                .get("source_message_id")
                .or_else(|| payload.get("message_id"))
                .and_then(Value::as_str);
            let admission_json = payload
                .get("admission")
                .map(serde_json::to_string)
                .transpose()?;
            sqlx::query(
                "INSERT INTO agent_turns (\
                    session_id, turn_id, operation_id, idempotency_key, source_message_id, \
                    admission_json, state, result_json, error_json, started_event_id, \
                    terminal_event_id, accepted_at, started_at, finished_at\
                 ) VALUES (?, ?, ?, ?, ?, ?, 'running', NULL, NULL, ?, NULL, ?, ?, NULL)",
            )
            .bind(event.agent_session_id.as_ref())
            .bind(operation_id)
            .bind(operation_id)
            .bind(event.idempotency_key.as_ref())
            .bind(source_message_id)
            .bind(admission_json)
            .bind(event.event_id.as_ref())
            .bind(as_i64(event.seq, "turn accepted sequence")?)
            .bind(as_i64(event.seq, "turn started sequence")?)
            .execute(&mut **tx)
            .await?;
        }
        "turn/completed" | "turn/failed" | "turn/cancelled" => {
            let state = match event.kind.0.as_str() {
                "turn/completed" => "completed",
                "turn/failed" => "failed",
                "turn/cancelled" => "cancelled",
                _ => unreachable!(),
            };
            let payload_json = serde_json::to_string(payload)?;
            let (result_json, error_json) = if state == "completed" {
                (Some(payload_json), None)
            } else {
                (None, Some(payload_json))
            };
            let result = sqlx::query(
                "UPDATE agent_turns SET state = ?, result_json = ?, error_json = ?, \
                    terminal_event_id = ?, finished_at = ? \
                 WHERE session_id = ? AND operation_id = ? AND state = 'running'",
            )
            .bind(state)
            .bind(result_json)
            .bind(error_json)
            .bind(event.event_id.as_ref())
            .bind(as_i64(event.seq, "turn finished sequence")?)
            .bind(event.agent_session_id.as_ref())
            .bind(operation_id)
            .execute(&mut **tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(SessionStoreError::Conflict(format!(
                    "canonical Agent Turn {operation_id} was not running"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_turn_terminal_kind(kind: &str) -> bool {
    matches!(kind, "turn/completed" | "turn/failed" | "turn/cancelled")
}

async fn first_turn_lifecycle_kind_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    operation_id: &str,
) -> Result<Option<String>, SessionStoreError> {
    Ok(sqlx::query_scalar(
        "SELECT kind FROM agent_events \
         WHERE session_id = ? AND correlation_id = ? \
           AND kind IN ('turn/started', 'turn/completed', 'turn/failed', 'turn/cancelled') \
         ORDER BY seq ASC LIMIT 1",
    )
    .bind(session_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn first_turn_terminal_kind_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    operation_id: &str,
) -> Result<Option<String>, SessionStoreError> {
    Ok(sqlx::query_scalar(
        "SELECT kind FROM agent_events \
         WHERE session_id = ? AND correlation_id = ? \
           AND kind IN ('turn/completed', 'turn/failed', 'turn/cancelled') \
         ORDER BY seq ASC LIMIT 1",
    )
    .bind(session_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn turn_started_exists_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    operation_id: &str,
) -> Result<bool, SessionStoreError> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_events \
         WHERE session_id = ? AND correlation_id = ? AND kind = 'turn/started')",
    )
    .bind(session_id)
    .bind(operation_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(exists != 0)
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
    reasoning_effort: Option<String>,
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
    storage_kind: String,
    body: Option<Vec<u8>>,
    object_ref: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredTurnRow {
    turn_id: String,
    operation_id: String,
    state: String,
    started_event_id: String,
    terminal_event_id: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredTurnHistoryRow {
    session_id: String,
    turn_id: String,
    source_message_id: String,
    state: String,
    result_json: Option<String>,
    error_json: Option<String>,
    started_event_id: String,
    accepted_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredEffectRow {
    effect_id: String,
    session_id: String,
    turn_id: String,
    operation_id: String,
    owner_domain: String,
    capability_module: String,
    action_id: String,
    resource_binding_id: Option<String>,
    resource_key: Option<String>,
    input_digest: String,
    strategy: String,
    state: String,
    bounded_observation_json: Option<String>,
    started_event_id: String,
    terminal_event_id: Option<String>,
    created_at: i64,
    settled_at: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredDeletionAuditRow {
    audit_id: String,
    agent_session_id: String,
    owner_ref_json: String,
    target_kind: String,
    target_id: String,
    authority: String,
    risk_acknowledged: i64,
    reason_digest: String,
    recorded_at: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredResourceRow {
    binding_id: String,
    resource_kind: String,
    resource_id: String,
    owner_id: String,
    operations_json: String,
    connection_config_ref: Option<String>,
    typed_parameters_json: String,
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

async fn validate_agent_store_schema(pool: &SqlitePool) -> Result<(), SessionStoreError> {
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    if foreign_keys != 1 {
        return Err(SessionStoreError::InvalidSession(
            "shared canonical Agent Store pool must enforce SQLite foreign keys".to_owned(),
        ));
    }
    let actual = schema_table_names(pool).await?;
    let required = agent_store_schema_manifest_payload()
        .tables
        .into_iter()
        .filter(|table| {
            table.owner == "platform.agent-session" || table.table_name == "schema_metadata"
        })
        .map(|table| table.table_name)
        .collect::<BTreeSet<_>>();
    let missing = required
        .difference(&actual)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(SessionStoreError::InvalidSession(format!(
            "AgentSessionStore requires the shared canonical Agent Store schema; missing tables {missing:?}"
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
            "canonical Agent Store schema_metadata row is missing".to_owned(),
        ));
    };
    if data_generation != i64::from(AGENT_STORE_DATA_GENERATION)
        || migration_head < i64::from(AGENT_STORE_MIGRATION_HEAD)
        || projection_schema_version < i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)
    {
        return Err(SessionStoreError::InvalidSession(format!(
            "unsupported canonical Agent Store metadata: generation={data_generation}, migration_head={migration_head}, projection_schema_version={projection_schema_version}"
        )));
    }

    let owned = expected_owned_table_names();
    let missing_owned = owned.difference(&actual).cloned().collect::<BTreeSet<_>>();
    if !missing_owned.is_empty() {
        return Err(SessionStoreError::InvalidSession(format!(
            "AgentSession owned tables are missing: {missing_owned:?}"
        )));
    }
    for (table, expected_columns) in AGENT_STORE_COLUMNS {
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
               'agent_sessions', 'agent_deletion_audits', 'agent_turns', \
               'agent_events', 'agent_payloads', \
               'agent_effects', 'agent_session_resources', \
               'agent_session_heads', 'agent_messages'\
           )",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    let expected_indexes = AGENT_STORE_INDEXES
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
    AGENT_STORE_TABLES
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
    .bind(i64::from(AGENT_STORE_DATA_GENERATION))
    .bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
    .bind("0".repeat(64))
    .bind("1".repeat(64))
    .bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION))
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
         FROM agent_events \
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
            parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, created_at, deleted_at\
         ) VALUES (?, ?, 'live', ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, NULL)",
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
    .bind(session.metadata.reasoning_effort.map(ReasoningEffort::as_str))
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

async fn insert_session_resources_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &AgentSessionId,
    owner: &PrincipalRef,
    bindings: &[TypedResourceBinding],
) -> Result<(), SessionStoreError> {
    let mut ids = BTreeSet::new();
    for binding in bindings {
        if !ids.insert(binding.binding_id.clone())
            || binding.binding_id.as_ref().trim().is_empty()
            || binding.resource_kind.as_ref().trim().is_empty()
            || binding.resource_id.as_ref().trim().is_empty()
            || binding.owner_id != owner.principal_id
            || binding
                .operations
                .iter()
                .any(|operation| operation.trim().is_empty())
        {
            return Err(SessionStoreError::InvalidSession(format!(
                "Session {} has an invalid or foreign resource binding {}",
                session_id.as_ref(),
                binding.binding_id.as_ref()
            )));
        }
        if binding.resource_kind.as_ref() == "knowledge_base" {
            let exists: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM knowledge_bases WHERE knowledge_base_id = ?)",
            )
            .bind(binding.resource_id.as_ref())
            .fetch_one(&mut **tx)
            .await?;
            if exists == 0 {
                return Err(SessionStoreError::InvalidSession(format!(
                    "Session {} selects unavailable Knowledge base {}",
                    session_id.as_ref(),
                    binding.resource_id.as_ref()
                )));
            }
        }
        let operations_json = serde_json::to_string(&binding.operations)?;
        let typed_parameters_json = serde_json::to_string(&binding.typed_parameters)?;
        let binding_digest = digest_payload(binding)?;
        sqlx::query(
            "INSERT INTO agent_session_resources (\
                binding_id, session_id, resource_kind, resource_id, owner_id, \
                operations_json, connection_config_ref, typed_parameters_json, binding_digest\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(binding.binding_id.as_ref())
        .bind(session_id.as_ref())
        .bind(binding.resource_kind.as_ref())
        .bind(binding.resource_id.as_ref())
        .bind(&binding.owner_id)
        .bind(operations_json)
        .bind(
            binding
                .connection_config_ref
                .as_ref()
                .map(|value| value.as_ref()),
        )
        .bind(typed_parameters_json)
        .bind(binding_digest.as_ref())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_head_tx(
    tx: &mut Transaction<'_, Sqlite>,
    head: &SessionHeadProjection,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "INSERT INTO agent_session_heads (\
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
        "UPDATE agent_session_heads SET \
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
        "INSERT INTO agent_events (\
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
        "SELECT COALESCE(SUM(byte_len), 0) FROM agent_payloads WHERE session_id = ?",
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
        "INSERT INTO agent_payloads \
            (payload_id, session_id, media_type, byte_len, digest, storage_kind, body, object_ref) \
         VALUES (?, ?, ?, ?, ?, 'inline', ?, NULL)",
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
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id = ?")
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
                "SELECT EXISTS(SELECT 1 FROM agent_events \
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
            sqlx::query_scalar("SELECT session_id FROM agent_events WHERE event_id = ?")
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
            "SELECT EXISTS(SELECT 1 FROM agent_events \
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
        "SELECT MAX(runtime_producer_seq) FROM agent_events WHERE runtime_binding_id = ?",
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
         FROM agent_events \
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
    let started_strategy = started
        .map(effect_strategy_from_event)
        .transpose()?;

    match append.semantic_event.kind.0.as_str() {
        "effect/started" => {
            if started.is_some() || terminal.is_some() || reconciled.is_some() {
                return Err(SessionStoreError::InvalidEvent(
                    "effect cannot be started more than once".to_owned(),
                ));
            }
            let strategy = effect_strategy_from_append(append)?;
            if strategy == crate::types::EffectStrategy::ReadOnly {
                return Err(SessionStoreError::InvalidEvent(
                    "read-only operations must not emit effect lifecycle events".to_owned(),
                ));
            }
            validate_effect_started_causation_tx(tx, append).await?;
            Ok(())
        }
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
            validate_effect_identity_transition(started, append, &started.event_id)?;
            let strategy = started_strategy.ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "effect/started must declare a lifecycle strategy".to_owned(),
                )
            })?;
            let terminal_kind = append.semantic_event.kind.0.as_str();
            if terminal_kind == "effect/uncertain"
                && strategy != crate::types::EffectStrategy::ExternalUncertainEffect
            {
                let recovery = effect_payload_from_append(append)?
                    .get("recovery")
                    .and_then(Value::as_str);
                if append.producer_id.as_ref() != "runtime_supervisor"
                    || recovery != Some("process_restart_external_reconciliation_required")
                {
                    return Err(SessionStoreError::InvalidEvent(
                        "managed effects require process-restart proof before becoming uncertain"
                            .to_owned(),
                    ));
                }
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
            if started_strategy != Some(crate::types::EffectStrategy::ExternalUncertainEffect) {
                let terminal_payload = effect_payload_from_event(
                    terminal.expect("reconciled effect has uncertain terminal"),
                )?;
                if terminal_payload.get("recovery").and_then(Value::as_str)
                    != Some("process_restart_external_reconciliation_required")
                {
                    return Err(SessionStoreError::InvalidEvent(
                        "managed effect reconciliation requires a process-restart uncertainty"
                            .to_owned(),
                    ));
                }
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
            let terminal = terminal.expect("reconciled effect has uncertain terminal");
            validate_effect_identity_transition(started, append, &terminal.event_id)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn validate_effect_started_causation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    append: &SessionEventAppend,
) -> Result<(), SessionStoreError> {
    let causation_event_id = append
        .semantic_event
        .causation_event_id
        .as_ref()
        .ok_or_else(|| {
            SessionStoreError::InvalidEvent(
                "effect/started requires its exact tool/call-started causation event".to_owned(),
            )
        })?;
    let tool = event_by_event_id_tx(tx, causation_event_id.as_ref())
        .await?
        .map(event_from_row)
        .transpose()?
        .ok_or_else(|| {
            SessionStoreError::InvalidEvent(
                "effect/started causation event is not committed".to_owned(),
            )
        })?;
    if tool.agent_session_id != append.agent_session_id
        || tool.kind.0 != "tool/call-started"
    {
        return Err(SessionStoreError::InvalidEvent(
            "effect/started causation must be a tool/call-started event in the same AgentSession"
                .to_owned(),
        ));
    }

    let payload = effect_payload_from_append(append)?;
    let turn_id = effect_required_string(payload, "turn_id")?;
    let operation_id = effect_required_string(payload, "operation_id")?;
    let capability_module = effect_required_string(payload, "capability_module")?;
    let action_id = effect_required_string(payload, "action_id")?;
    let turn_started_event_id = sqlx::query_scalar::<_, String>(
        "SELECT started_event_id FROM agent_turns \
         WHERE session_id = ? AND turn_id = ? AND state IN ('accepted', 'running')",
    )
    .bind(append.agent_session_id.as_ref())
    .bind(turn_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        SessionStoreError::InvalidEvent(
            "effect/started references a Turn that is not active in the canonical Store"
                .to_owned(),
        )
    })?;
    if tool.causation_event_id.as_ref().map(EventId::as_ref)
        != Some(turn_started_event_id.as_str())
    {
        return Err(SessionStoreError::InvalidEvent(
            "effect/started tool causation does not belong to its declared Turn".to_owned(),
        ));
    }
    let SessionEventPayloadRef::InlineJson(tool_payload) = &tool.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "effect/started tool causation requires inline canonical identity".to_owned(),
        ));
    };
    for (field, expected) in [
        ("operation_id", operation_id),
        ("capability_id", capability_module),
        ("action_id", action_id),
    ] {
        if tool_payload.0.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(SessionStoreError::InvalidEvent(format!(
                "effect/started causation tool identity differs at {field}"
            )));
        }
    }
    Ok(())
}

fn validate_effect_identity_transition(
    started: &SessionEventRecord,
    append: &SessionEventAppend,
    expected_causation_event_id: &EventId,
) -> Result<(), SessionStoreError> {
    if append.semantic_event.causation_event_id.as_ref() != Some(expected_causation_event_id) {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle causation does not reference its immediate predecessor".to_owned(),
        ));
    }
    if append.semantic_event.correlation_id != started.correlation_id {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle correlation changed after effect/started".to_owned(),
        ));
    }
    let started_payload = effect_payload_from_event(started)?;
    let next_payload = effect_payload_from_append(append)?;
    for field in [
        "effect_id",
        "turn_id",
        "operation_id",
        "owner_domain",
        "capability_module",
        "action_id",
        "input_digest",
        "strategy",
        "resource_binding_id",
        "resource_key",
    ] {
        if started_payload.get(field) != next_payload.get(field) {
            return Err(SessionStoreError::InvalidEvent(format!(
                "effect lifecycle identity changed at {field}"
            )));
        }
    }
    Ok(())
}

fn effect_payload_from_append(
    append: &SessionEventAppend,
) -> Result<&Value, SessionStoreError> {
    let SessionEventPayloadRef::InlineJson(payload) = &append.semantic_event.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle payload must be inline canonical JSON".to_owned(),
        ));
    };
    Ok(&payload.0)
}

fn effect_payload_from_event(event: &SessionEventRecord) -> Result<&Value, SessionStoreError> {
    let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle payload must be inline canonical JSON".to_owned(),
        ));
    };
    Ok(&payload.0)
}

fn effect_required_string<'a>(
    payload: &'a Value,
    field: &str,
) -> Result<&'a str, SessionStoreError> {
    payload.get(field).and_then(Value::as_str).ok_or_else(|| {
        SessionStoreError::InvalidEvent(format!(
            "effect ledger payload is missing {field}"
        ))
    })
}

async fn project_effect_fact_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    if !event.kind.0.starts_with("effect/") {
        return Ok(());
    }
    let field = |name: &str| -> Result<&str, SessionStoreError> {
        payload.get(name).and_then(Value::as_str).ok_or_else(|| {
            SessionStoreError::InvalidEvent(format!(
                "effect ledger payload is missing {name}"
            ))
        })
    };
    let effect_id = field("effect_id")?;
    if effect_id != event.correlation_id.as_ref() {
        return Err(SessionStoreError::InvalidEvent(
            "effect ledger identity differs from event correlation".to_owned(),
        ));
    }
    let recorded_at = payload
        .get("recorded_at")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| {
            SessionStoreError::InvalidEvent(
                "effect ledger payload has invalid recorded_at".to_owned(),
            )
        })?;
    match event.kind.0.as_str() {
        "effect/started" => {
            let strategy = field("strategy")?;
            if !matches!(strategy, "managed_effect" | "external_uncertain_effect") {
                return Err(SessionStoreError::InvalidEvent(
                    "effect ledger cannot persist a read-only strategy".to_owned(),
                ));
            }
            sqlx::query(
                "INSERT INTO agent_effects (\
                    effect_id, session_id, turn_id, operation_id, owner_domain, \
                    capability_module, action_id, resource_binding_id, resource_key, \
                    input_digest, strategy, state, bounded_observation_json, \
                    started_event_id, terminal_event_id, created_at, settled_at\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', NULL, ?, NULL, ?, NULL)",
            )
            .bind(effect_id)
            .bind(event.agent_session_id.as_ref())
            .bind(field("turn_id")?)
            .bind(field("operation_id")?)
            .bind(field("owner_domain")?)
            .bind(field("capability_module")?)
            .bind(field("action_id")?)
            .bind(payload.get("resource_binding_id").and_then(Value::as_str))
            .bind(payload.get("resource_key").and_then(Value::as_str))
            .bind(field("input_digest")?)
            .bind(strategy)
            .bind(event.event_id.as_ref())
            .bind(recorded_at)
            .execute(&mut **tx)
            .await?;
        }
        "effect/succeeded" | "effect/failed" | "effect/uncertain" => {
            let state = match event.kind.0.as_str() {
                "effect/succeeded" => "returned",
                "effect/failed" => "rejected",
                "effect/uncertain" => "unknown",
                _ => unreachable!(),
            };
            let observation = serde_json::to_string(payload)?;
            let result = sqlx::query(
                "UPDATE agent_effects SET state = ?, bounded_observation_json = ?, \
                    terminal_event_id = ?, settled_at = ? \
                 WHERE effect_id = ? AND session_id = ? AND state = 'pending'",
            )
            .bind(state)
            .bind(observation)
            .bind(event.event_id.as_ref())
            .bind(recorded_at)
            .bind(effect_id)
            .bind(event.agent_session_id.as_ref())
            .execute(&mut **tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(SessionStoreError::Conflict(format!(
                    "canonical effect {effect_id} was not pending"
                )));
            }
        }
        "effect/reconciled" => {
            let state = match payload.get("outcome").and_then(Value::as_str) {
                Some("confirmed_succeeded") => "returned",
                Some("confirmed_failed") => "rejected",
                Some("still_uncertain") => "unknown",
                other => {
                    return Err(SessionStoreError::InvalidEvent(format!(
                        "effect reconciliation has invalid outcome {other:?}"
                    )));
                }
            };
            let observation = serde_json::to_string(payload)?;
            let result = sqlx::query(
                "UPDATE agent_effects SET state = ?, bounded_observation_json = ?, \
                    terminal_event_id = ?, settled_at = ? \
                 WHERE effect_id = ? AND session_id = ? AND state = 'unknown'",
            )
            .bind(state)
            .bind(observation)
            .bind(event.event_id.as_ref())
            .bind(recorded_at)
            .bind(effect_id)
            .bind(event.agent_session_id.as_ref())
            .execute(&mut **tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(SessionStoreError::Conflict(format!(
                    "canonical effect {effect_id} was not unknown"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn event_uses_agent_messages(
    entry: &nomifun_agent_contracts::SessionEventRegistryEntry,
) -> bool {
    entry
        .projector
        .reducers
        .iter()
        .any(|reducer| reducer.as_ref() == "message-projection")
}

fn effect_append(
    mut request: EffectEventRequest,
    kind: &str,
) -> Result<SessionEventAppend, SessionStoreError> {
    if request.effect_id.trim().is_empty()
        || request.effect_id != request.correlation_id.as_ref()
        || request.turn_id.as_ref().trim().is_empty()
        || request.operation_id.as_ref().trim().is_empty()
        || request.owner_domain.trim().is_empty()
        || request.capability_module.as_ref().trim().is_empty()
        || request.action_id.as_ref().trim().is_empty()
        || request.input_digest.as_ref().len() != 64
        || request.recorded_at < 0
    {
        return Err(SessionStoreError::InvalidEvent(
            "effect ledger identity is incomplete or inconsistent".to_owned(),
        ));
    }
    let SessionEventPayloadRef::InlineJson(mut payload) = request.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle payload must be inline canonical JSON".to_owned(),
        ));
    };
    let object = payload.0.as_object_mut().ok_or_else(|| {
        SessionStoreError::InvalidEvent(
            "effect lifecycle payload must be a JSON object".to_owned(),
        )
    })?;
    if let Some(existing) = object.get("strategy").and_then(Value::as_str) {
        if existing != request.strategy.as_str() {
            return Err(SessionStoreError::InvalidEvent(
                "effect lifecycle strategy changed between events".to_owned(),
            ));
        }
    } else {
        object.insert(
            "strategy".to_owned(),
            Value::String(request.strategy.as_str().to_owned()),
        );
    }
    for (key, value) in [
        ("effect_id", Value::String(request.effect_id.clone())),
        ("turn_id", Value::String(request.turn_id.as_ref().to_owned())),
        (
            "operation_id",
            Value::String(request.operation_id.as_ref().to_owned()),
        ),
        ("owner_domain", Value::String(request.owner_domain.clone())),
        (
            "capability_module",
            Value::String(request.capability_module.as_ref().to_owned()),
        ),
        ("action_id", Value::String(request.action_id.as_ref().to_owned())),
        (
            "input_digest",
            Value::String(request.input_digest.as_ref().to_owned()),
        ),
        ("recorded_at", Value::from(request.recorded_at)),
    ] {
        if let Some(existing) = object.get(key) {
            if existing != &value {
                return Err(SessionStoreError::InvalidEvent(format!(
                    "effect ledger field {key} changed between events"
                )));
            }
        } else {
            object.insert(key.to_owned(), value);
        }
    }
    for (key, value) in [
        (
            "resource_binding_id",
            request
                .resource_binding_id
                .as_ref()
                .map(|value| Value::String(value.as_ref().to_owned())),
        ),
        (
            "resource_key",
            request.resource_key.as_ref().map(|value| Value::String(value.clone())),
        ),
    ] {
        if let Some(value) = value {
            if let Some(existing) = object.get(key) {
                if existing != &value {
                    return Err(SessionStoreError::InvalidEvent(format!(
                        "effect ledger field {key} changed between events"
                    )));
                }
            } else {
                object.insert(key.to_owned(), value);
            }
        }
    }
    request.payload = SessionEventPayloadRef::InlineJson(payload);
    Ok(SessionEventAppend {
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
    })
}

fn effect_strategy_from_append(
    append: &SessionEventAppend,
) -> Result<crate::types::EffectStrategy, SessionStoreError> {
    let SessionEventPayloadRef::InlineJson(payload) = &append.semantic_event.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle payload must be inline canonical JSON".to_owned(),
        ));
    };
    effect_strategy_from_payload(&payload.0)
}

fn effect_strategy_from_event(
    event: &SessionEventRecord,
) -> Result<crate::types::EffectStrategy, SessionStoreError> {
    let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
        return Err(SessionStoreError::InvalidEvent(
            "effect lifecycle payload must be inline canonical JSON".to_owned(),
        ));
    };
    effect_strategy_from_payload(&payload.0)
}

fn effect_strategy_from_payload(
    payload: &Value,
) -> Result<crate::types::EffectStrategy, SessionStoreError> {
    match payload.get("strategy").and_then(Value::as_str) {
        Some("read_only") => Ok(crate::types::EffectStrategy::ReadOnly),
        Some("managed_effect") => Ok(crate::types::EffectStrategy::ManagedEffect),
        Some("external_uncertain_effect") => {
            Ok(crate::types::EffectStrategy::ExternalUncertainEffect)
        }
        Some(other) => Err(SessionStoreError::InvalidEvent(format!(
            "unknown effect lifecycle strategy {other:?}"
        ))),
        None => Err(SessionStoreError::InvalidEvent(
            "effect lifecycle payload must declare strategy".to_owned(),
        )),
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
                parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, created_at, deleted_at \
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
                parent_agent_session_id, fork_base_payload_id, reasoning_effort, next_seq, created_at, deleted_at \
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
            reasoning_effort: parse_reasoning_effort(row.reasoning_effort.as_deref())?,
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
         FROM agent_events WHERE event_id = ?",
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
         FROM agent_events WHERE producer_id = ? AND idempotency_key = ?",
    )
    .bind(producer_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn event_by_kind_correlation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    kind: &str,
    correlation_id: &str,
) -> Result<Option<StoredEventRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM agent_events WHERE session_id = ? AND kind = ? AND correlation_id = ? \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(session_id)
    .bind(kind)
    .bind(correlation_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn event_by_kind_causation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    kind: &str,
    causation_event_id: &str,
) -> Result<Option<StoredEventRow>, SessionStoreError> {
    Ok(sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id, seq, event_id, producer_id, idempotency_key, \
                runtime_binding_id, runtime_producer_seq, kind, kind_version, \
                correlation_id, causation_event_id, inline_json, payload_id \
         FROM agent_events WHERE session_id = ? AND kind = ? AND causation_event_id = ? \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(session_id)
    .bind(kind)
    .bind(causation_event_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn latest_turn_boundary_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<EventId, SessionStoreError> {
    sqlx::query_scalar::<_, String>(
        "SELECT event_id FROM agent_events \
         WHERE session_id = ? AND kind IN (\
            'session/ready', 'turn/completed', 'turn/failed', 'turn/cancelled'\
         ) ORDER BY seq DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(EventId::from)
    .ok_or_else(|| {
        SessionStoreError::Conflict(
            "AgentSession has no committed turn admission boundary".to_owned(),
        )
    })
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
         FROM agent_events WHERE runtime_binding_id = ? AND runtime_producer_seq = ?",
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
         FROM agent_events WHERE session_id = ? ORDER BY seq ASC",
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
        "SELECT payload_id, session_id, media_type, byte_len, digest, storage_kind, body, object_ref \
         FROM agent_payloads WHERE payload_id = ?",
    )
    .bind(payload_id)
    .fetch_optional(&mut **tx)
    .await?)
}

fn payload_from_row(row: StoredPayloadRow) -> Result<SessionPayloadRecord, SessionStoreError> {
    if row.storage_kind != "inline" || row.object_ref.is_some() {
        return Err(SessionStoreError::InvalidPayload(
            "object payload must be resolved by the content-addressed object port".to_owned(),
        ));
    }
    let body = row.body.ok_or_else(|| {
        SessionStoreError::InvalidPayload("inline payload body is missing".to_owned())
    })?;
    Ok(SessionPayloadRecord {
        payload_id: ArtifactId(row.payload_id),
        agent_session_id: AgentSessionId(row.session_id),
        media_type: row.media_type,
        byte_len: as_u64(row.byte_len, "payload byte_len")?,
        digest: DigestHex(row.digest),
        body: serde_json::from_slice(&body)?,
    })
}

fn effect_from_row(row: StoredEffectRow) -> Result<AgentEffectRecord, SessionStoreError> {
    let strategy = match row.strategy.as_str() {
        "managed_effect" => EffectStrategy::ManagedEffect,
        "external_uncertain_effect" => EffectStrategy::ExternalUncertainEffect,
        value => {
            return Err(SessionStoreError::InvalidEvent(format!(
                "canonical effect has unknown strategy {value}"
            )));
        }
    };
    let state = match row.state.as_str() {
        "pending" => AgentEffectState::Pending,
        "returned" => AgentEffectState::Returned,
        "rejected" => AgentEffectState::Rejected,
        "cancelled" => AgentEffectState::Cancelled,
        "unknown" => AgentEffectState::Unknown,
        value => {
            return Err(SessionStoreError::InvalidEvent(format!(
                "canonical effect has unknown state {value}"
            )));
        }
    };
    Ok(AgentEffectRecord {
        effect_id: row.effect_id,
        agent_session_id: AgentSessionId::from(row.session_id),
        turn_id: OperationId::from(row.turn_id),
        operation_id: OperationId::from(row.operation_id),
        owner_domain: row.owner_domain,
        capability_module: CapabilityId::from(row.capability_module),
        action_id: ActionId::from(row.action_id),
        resource_binding_id: row.resource_binding_id.map(ResourceBindingId::from),
        resource_key: row.resource_key,
        input_digest: DigestHex::from(row.input_digest),
        strategy,
        state,
        bounded_observation: row
            .bounded_observation_json
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        started_event_id: EventId::from(row.started_event_id),
        terminal_event_id: row.terminal_event_id.map(EventId::from),
        created_at: row.created_at,
        settled_at: row.settled_at,
    })
}

fn deletion_audit_from_row(
    row: StoredDeletionAuditRow,
) -> Result<AgentDeletionAuditRecord, SessionStoreError> {
    if row.risk_acknowledged != 1
        || row.authority != "installation_owner_manual_override"
        || !matches!(row.target_kind.as_str(), "effect" | "resource_cleanup")
        || row.recorded_at < 0
    {
        return Err(SessionStoreError::InvalidSession(
            "Agent deletion audit row violates its immutable contract".to_owned(),
        ));
    }
    Ok(AgentDeletionAuditRecord {
        audit_id: row.audit_id,
        agent_session_id: AgentSessionId::from(row.agent_session_id),
        owner_ref: serde_json::from_str(&row.owner_ref_json)?,
        target_kind: row.target_kind,
        target_id: row.target_id,
        authority: row.authority,
        risk_acknowledged: true,
        reason_digest: DigestHex::from(row.reason_digest),
        recorded_at: row.recorded_at,
    })
}

async fn deletion_audit_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    audit_id: &str,
) -> Result<Option<AgentDeletionAuditRecord>, SessionStoreError> {
    sqlx::query_as::<_, StoredDeletionAuditRow>(
        "SELECT audit_id, agent_session_id, owner_ref_json, target_kind, target_id, \
                authority, risk_acknowledged, reason_digest, recorded_at \
         FROM agent_deletion_audits WHERE audit_id = ?",
    )
    .bind(audit_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(deletion_audit_from_row)
    .transpose()
}

async fn record_deletion_audit_tx(
    tx: &mut Transaction<'_, Sqlite>,
    audit_id: &str,
    session_id: &AgentSessionId,
    owner: &PrincipalRef,
    target_kind: &str,
    target_id: &str,
    reason_digest: &DigestHex,
    recorded_at: i64,
) -> Result<AgentDeletionAuditRecord, SessionStoreError> {
    if !matches!(target_kind, "effect" | "resource_cleanup")
        || target_id.is_empty()
        || target_id.len() > 512
        || target_id.trim() != target_id
    {
        return Err(SessionStoreError::InvalidEvent(
            "delete override audit target is invalid".to_owned(),
        ));
    }
    let owner_json = serde_json::to_string(owner)?;
    sqlx::query(
        "INSERT INTO agent_deletion_audits (audit_id, agent_session_id, owner_ref_json, \
                target_kind, target_id, authority, risk_acknowledged, reason_digest, recorded_at) \
         VALUES (?, ?, ?, ?, ?, 'installation_owner_manual_override', 1, ?, ?) \
         ON CONFLICT DO NOTHING",
    )
    .bind(audit_id)
    .bind(session_id.as_ref())
    .bind(&owner_json)
    .bind(target_kind)
    .bind(target_id)
    .bind(reason_digest.as_ref())
    .bind(recorded_at)
    .execute(&mut **tx)
    .await?;
    let audit = deletion_audit_by_id_tx(tx, audit_id)
        .await?
        .ok_or_else(|| {
            SessionStoreError::IdempotencyConflict(
                "delete override audit identity conflicts with another record".to_owned(),
            )
        })?;
    if audit.agent_session_id != *session_id
        || audit.owner_ref != *owner
        || audit.target_kind != target_kind
        || audit.target_id != target_id
        || audit.reason_digest != *reason_digest
        || !audit.risk_acknowledged
        || audit.authority != "installation_owner_manual_override"
    {
        return Err(SessionStoreError::IdempotencyConflict(
            "delete override audit identity was reused for different input".to_owned(),
        ));
    }
    Ok(audit)
}

fn resource_from_row(row: StoredResourceRow) -> Result<TypedResourceBinding, SessionStoreError> {
    Ok(TypedResourceBinding {
        binding_id: ResourceBindingId::from(row.binding_id),
        resource_kind: ResourceKind::from(row.resource_kind),
        resource_id: ResourceId::from(row.resource_id),
        owner_id: row.owner_id,
        operations: serde_json::from_str(&row.operations_json)?,
        connection_config_ref: row.connection_config_ref.map(ConnectionConfigRef::from),
        typed_parameters: serde_json::from_str(&row.typed_parameters_json)?,
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
         FROM agent_session_heads WHERE session_id = ?",
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
         FROM agent_messages WHERE session_id = ? AND projection_id = ?",
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
        "INSERT INTO agent_messages (\
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
        message_type: None,
        message_status: None,
        projection: serde_json::from_str(&row.projection_json)?,
        semantic_digest: row.semantic_digest,
    })
}

fn turn_history_projection_from_row(
    row: StoredTurnHistoryRow,
) -> Result<Option<MessageProjection>, SessionStoreError> {
    let Ok(source_message_uuid) = Uuid::parse_str(&row.source_message_id) else {
        return Ok(None);
    };
    if source_message_uuid.get_version_num() != 7 {
        return Ok(None);
    }
    let Ok(started_event_uuid) = Uuid::parse_str(&row.started_event_id) else {
        return Ok(None);
    };
    if started_event_uuid.get_version_num() != 7 {
        return Ok(None);
    }
    if !matches!(
        row.state.as_str(),
        "running" | "completed" | "failed" | "cancelled" | "interrupted"
    ) {
        return Err(SessionStoreError::InvalidSession(format!(
            "canonical Agent Turn has unknown history state {}",
            row.state
        )));
    }
    let first_seq = as_u64(
        row.started_at.unwrap_or(row.accepted_at),
        "turn history first_seq",
    )?;
    let last_seq = row
        .finished_at
        .map(|value| as_u64(value, "turn history last_seq"))
        .transpose()?
        .unwrap_or(first_seq);
    if last_seq < first_seq {
        return Err(SessionStoreError::InvalidSession(
            "canonical Agent Turn history ends before it starts".to_owned(),
        ));
    }
    let started_at_ms = {
        let bytes = started_event_uuid.as_bytes();
        let value = ((bytes[0] as u64) << 40)
            | ((bytes[1] as u64) << 32)
            | ((bytes[2] as u64) << 24)
            | ((bytes[3] as u64) << 16)
            | ((bytes[4] as u64) << 8)
            | bytes[5] as u64;
        i64::try_from(value).map_err(|_| {
            SessionStoreError::InvalidSession(
                "canonical Agent Turn UUIDv7 timestamp overflowed".to_owned(),
            )
        })?
    };
    let terminal_payload = row
        .error_json
        .as_deref()
        .or(row.result_json.as_deref())
        .map(serde_json::from_str::<Value>)
        .transpose()?;
    let finished_at_ms = terminal_payload
        .as_ref()
        .and_then(|payload| payload.get("finished_at_ms"))
        .and_then(Value::as_i64)
        .filter(|finished| *finished >= started_at_ms);
    let error = if row.state == "failed" {
        terminal_payload
            .as_ref()
            .and_then(|payload| payload.get("error"))
            .cloned()
            .or_else(|| {
                terminal_payload
                    .as_ref()
                    .and_then(|payload| payload.get("message"))
                    .and_then(Value::as_str)
                    .map(|message| {
                        json!({
                            "message": message,
                            "code": "UNKNOWN_UPSTREAM_ERROR",
                            "ownership": "unknown_upstream",
                            "retryable": true,
                            "feedback_recommended": true,
                            "resolution": {
                                "kind": "send_feedback",
                                "target": "feedback"
                            }
                        })
                    })
            })
            .unwrap_or_else(|| {
                json!({
                    "message": "The upstream Agent failed while handling the request",
                    "code": "UNKNOWN_UPSTREAM_ERROR",
                    "ownership": "unknown_upstream",
                    "retryable": true,
                    "feedback_recommended": true,
                    "resolution": {
                        "kind": "send_feedback",
                        "target": "feedback"
                    }
                })
            })
    } else {
        Value::Null
    };
    let is_failed = row.state == "failed";
    let projection_id = format!("turn_summary:{}", row.started_event_id);
    let projection = json!({
        "projection_id": projection_id,
        "correlation_id": started_event_uuid.to_string(),
        "presentation_intent": "turn_summary",
        "state": row.state,
        "source_message_id": source_message_uuid.to_string(),
        "turn_operation_id": row.turn_id,
        "started_seq": first_seq,
        "finished_seq": row.finished_at,
        "started_at_ms": started_at_ms,
        "finished_at_ms": finished_at_ms,
        "error": error,
    });
    let semantic_digest = digest_payload(&projection)?.0;
    Ok(Some(MessageProjection {
        session_id: AgentSessionId::from(row.session_id),
        projection_id,
        first_seq,
        last_seq,
        presentation_intent: "turn_summary".to_owned(),
        message_type: Some(if is_failed { "tips" } else { "agent_status" }.to_owned()),
        message_status: Some(
            if row.state == "running" {
                "work"
            } else if is_failed {
                "error"
            } else {
                "finish"
            }
            .to_owned(),
        ),
        projection,
        semantic_digest,
    }))
}

fn validate_automation_config_request(
    request: &CommitAgentSessionAutomationConfig,
) -> Result<(), SessionStoreError> {
    validate_uuidv7(request.agent_session_id.as_ref(), "agent_session_id")?;
    validate_principal(&request.owner_ref)?;
    if request.recorded_at < 0 {
        return Err(SessionStoreError::InvalidSession(
            "AutoWork config timestamp must not be negative".to_owned(),
        ));
    }
    validate_automation_config_fields(
        request.enabled,
        request.tag.as_deref(),
        request.operation_id.as_deref(),
    )
}

fn validate_cleanup_reconciliation(
    owner_domain: &str,
    evidence_digest: &DigestHex,
    recorded_at: i64,
) -> Result<(), SessionStoreError> {
    validate_resource_cleanup_domain(owner_domain)?;
    if evidence_digest.as_ref().len() != 64
        || !evidence_digest
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || recorded_at < 0
    {
        return Err(SessionStoreError::InvalidEvent(
            "delete reconciliation requires a canonical owner, evidence digest and timestamp"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_resource_cleanup_domain(owner_domain: &str) -> Result<(), SessionStoreError> {
    if owner_domain.is_empty()
        || owner_domain.len() > 64
        || owner_domain.trim() != owner_domain
        || !owner_domain
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SessionStoreError::InvalidEvent(
            "resource cleanup requires a canonical owner domain".to_owned(),
        ));
    }
    Ok(())
}

fn validate_automation_config_fields(
    enabled: bool,
    tag: Option<&str>,
    operation_id: Option<&str>,
) -> Result<(), SessionStoreError> {
    if enabled && tag.is_none() {
        return Err(SessionStoreError::InvalidSession(
            "enabled AutoWork config requires a tag".to_owned(),
        ));
    }
    if tag.is_some_and(|tag| {
        tag.is_empty()
            || tag.len() > 256
            || tag.trim() != tag
            || tag.chars().any(char::is_control)
    }) {
        return Err(SessionStoreError::InvalidSession(
            "AutoWork tag must be canonical and bounded".to_owned(),
        ));
    }
    if operation_id.is_some_and(|operation_id| {
        operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(SessionStoreError::InvalidSession(
            "AutoWork config operation must contain 1-128 visible ASCII bytes".to_owned(),
        ));
    }
    Ok(())
}

async fn automation_config_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<AgentSessionAutomationConfig, SessionStoreError> {
    let payload = sqlx::query_scalar::<_, String>(
        "SELECT inline_json FROM agent_events \
         WHERE session_id = ? AND kind = 'automation/config-committed' \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(payload) = payload else {
        return Ok(AgentSessionAutomationConfig::default());
    };
    let (_, config) = automation_config_event_from_value(serde_json::from_str(&payload)?)?;
    Ok(config)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAutomationConfigEventPayload {
    expected_revision: u64,
    committed: AgentSessionAutomationConfig,
}

fn automation_config_event_from_value(
    value: Value,
) -> Result<(u64, AgentSessionAutomationConfig), SessionStoreError> {
    let payload: StoredAutomationConfigEventPayload = serde_json::from_value(value)?;
    let config = payload.committed;
    validate_automation_config_fields(
        config.enabled,
        config.tag.as_deref(),
        config.operation_id.as_deref(),
    )?;
    let next_revision = payload.expected_revision.checked_add(1);
    if config.revision != payload.expected_revision
        && next_revision != Some(config.revision)
    {
        return Err(SessionStoreError::InvalidEvent(
            "AutoWork config receipt has an invalid revision transition".to_owned(),
        ));
    }
    Ok((payload.expected_revision, config))
}

async fn delete_blockers_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<AgentSessionDeleteBlockers, SessionStoreError> {
    let effect_rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT effect.effect_id, effect.owner_domain, effect.state FROM agent_effects effect \
         WHERE effect.session_id = ? AND effect.state IN ('pending', 'unknown', 'cancelled') \
           AND NOT EXISTS (SELECT 1 FROM agent_events override \
               WHERE override.session_id = effect.session_id \
                 AND override.kind = 'deletion/effect-override' \
                 AND json_extract(override.inline_json, '$.effect_id') = effect.effect_id) \
         ORDER BY owner_domain, effect_id",
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    let effects = effect_rows
        .into_iter()
        .map(|(effect_id, owner_domain, state)| {
            let state = match state.as_str() {
                "pending" => AgentEffectState::Pending,
                "unknown" => AgentEffectState::Unknown,
                "cancelled" => AgentEffectState::Cancelled,
                other => {
                    return Err(SessionStoreError::InvalidEvent(format!(
                        "delete blocker has unsupported effect state {other}"
                    )));
                }
            };
            Ok(AgentEffectDeleteBlocker {
                effect_id,
                owner_domain,
                state,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let cleanup_events = sqlx::query_as::<_, (String, String)>(
        "SELECT kind, inline_json FROM agent_events \
         WHERE session_id = ? \
           AND kind IN ('resource/cleanup-started', 'resource/cleanup-succeeded', \
                        'resource/cleanup-uncertain', 'resource/cleanup-reconciled', \
                        'resource/cleanup-override') \
         ORDER BY seq",
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut cleanup_pending = BTreeSet::new();
    let mut cleanup_uncertainties = BTreeMap::new();
    let mut cleanup_reconciled = BTreeSet::new();
    for (kind, payload) in cleanup_events {
        let payload: Value = serde_json::from_str(&payload)?;
        let owner_domain = payload
            .get("owner_domain")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "resource cleanup event lost owner_domain".to_owned(),
                )
            })?;
        match kind.as_str() {
            "resource/cleanup-started" => {
                if payload.get("outcome").and_then(Value::as_str) != Some("pending") {
                    return Err(SessionStoreError::InvalidEvent(
                        "resource cleanup start has invalid semantics".to_owned(),
                    ));
                }
                cleanup_pending.insert(owner_domain.to_owned());
            }
            "resource/cleanup-succeeded" => {
                if payload.get("outcome").and_then(Value::as_str) != Some("succeeded")
                    || (!cleanup_pending.remove(owner_domain)
                        && !cleanup_reconciled.contains(owner_domain))
                {
                    return Err(SessionStoreError::InvalidEvent(
                        "resource cleanup success has no exact pending predecessor".to_owned(),
                    ));
                }
            }
            "resource/cleanup-uncertain" => {
                let recorded_at = payload
                    .get("recorded_at")
                    .and_then(Value::as_i64)
                    .filter(|value| *value >= 0)
                    .ok_or_else(|| {
                        SessionStoreError::InvalidEvent(
                            "resource cleanup uncertainty lost recorded_at".to_owned(),
                        )
                    })?;
                if payload.get("outcome").and_then(Value::as_str) != Some("unknown")
                    || payload.get("recovery").and_then(Value::as_str)
                        != Some("external_reconciliation_required")
                {
                    return Err(SessionStoreError::InvalidEvent(
                        "resource cleanup uncertainty has invalid recovery semantics".to_owned(),
                    ));
                }
                if !cleanup_pending.remove(owner_domain) {
                    return Err(SessionStoreError::InvalidEvent(
                        "resource cleanup uncertainty has no exact pending predecessor"
                            .to_owned(),
                    ));
                }
                cleanup_uncertainties.insert(
                    owner_domain.to_owned(),
                    ResourceCleanupUncertainty {
                        owner_domain: owner_domain.to_owned(),
                        recorded_at,
                    },
                );
            }
            "resource/cleanup-reconciled" => {
                if payload.get("outcome").and_then(Value::as_str)
                    != Some("confirmed_safe_to_delete")
                    || payload
                        .get("evidence_digest")
                        .and_then(Value::as_str)
                        .is_none_or(|digest| {
                            digest.len() != 64
                                || !digest.bytes().all(|byte| {
                                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                                })
                        })
                    || cleanup_uncertainties.remove(owner_domain).is_none()
                {
                    return Err(SessionStoreError::InvalidEvent(
                        "resource cleanup reconciliation has no exact uncertainty predecessor"
                            .to_owned(),
                    ));
                }
                cleanup_reconciled.insert(owner_domain.to_owned());
            }
            "resource/cleanup-override" => {
                let reason_digest = payload.get("reason_digest").and_then(Value::as_str);
                if payload.get("authority").and_then(Value::as_str)
                    != Some("installation_owner_manual_override")
                    || payload.get("risk_acknowledged").and_then(Value::as_bool) != Some(true)
                    || reason_digest.is_none_or(|digest| {
                        digest.len() != 64
                            || !digest.bytes().all(|byte| {
                                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                            })
                    })
                    || cleanup_uncertainties.remove(owner_domain).is_none()
                {
                    return Err(SessionStoreError::InvalidEvent(
                        "resource cleanup override has no exact uncertainty predecessor"
                            .to_owned(),
                    ));
                }
                cleanup_reconciled.insert(owner_domain.to_owned());
            }
            _ => unreachable!(),
        }
    }
    let resource_cleanup_pending = cleanup_pending.into_iter().collect();
    let resource_cleanup_uncertainties = cleanup_uncertainties.into_values().collect();

    Ok(AgentSessionDeleteBlockers {
        effects,
        resource_cleanup_pending,
        resource_cleanup_uncertainties,
    })
}

async fn purge_private_content_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
) -> Result<(), SessionStoreError> {
    sqlx::query("DELETE FROM agent_effects WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM agent_turns WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM agent_session_resources WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM agent_messages WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM agent_session_heads WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE agent_events SET causation_event_id = NULL WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM agent_events WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM agent_payloads WHERE session_id = ?")
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
        || row.reasoning_effort.is_some()
        || row.next_seq.is_some()
        || row.created_at.is_some()
        || row.deleted_at.is_none()
    {
        return Err(SessionStoreError::Conflict(
            "final AgentSession row is not the exact four-field tombstone".to_owned(),
        ));
    }
    for table in [
        "agent_turns",
        "agent_effects",
        "agent_session_resources",
        "agent_events",
        "agent_payloads",
        "agent_session_heads",
        "agent_messages",
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
        || fork.parent_through_seq != request.parent_through_seq
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
         FROM agent_events \
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
    if child_head.status != "ready" || child_head.active_turn_id.is_some() {
        return Err(SessionStoreError::Conflict(
            "fork child did not reach the canonical ready boundary".to_owned(),
        ));
    }
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

fn parse_reasoning_effort(value: Option<&str>) -> Result<Option<ReasoningEffort>, SessionStoreError> {
    match value {
        None => Ok(None),
        Some("low") => Ok(Some(ReasoningEffort::Low)),
        Some("medium") => Ok(Some(ReasoningEffort::Medium)),
        Some("high") => Ok(Some(ReasoningEffort::High)),
        Some(_) => Err(SessionStoreError::InvalidSession(
            "reasoning_effort is not low, medium, or high".to_owned(),
        )),
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
