//! Engine-neutral journal backed exclusively by the canonical Agent Store.
//!
//! Runtime-private records are durable `runtime/progress-recorded` facts.
//! User-visible assistant text, thinking, and Turn terminals are projected from the same
//! ordered event stream; no Conversation delivery/runtime table participates.

use std::{
    collections::BTreeMap,
    sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, ArtifactId, ChatRouteIdentity, CorrelationId, EventId,
    EventProducerId, IdempotencyKey, OperationId, ResolvedSnapshotRef,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef, SessionPayloadBody, SessionPayloadRecord,
    StrictJsonValue, canonical_json_bytes, digest_bytes,
};
use nomifun_agent_session::{
    AgentSessionStore, ChatOperationClaimRequest, NativeExecutionLease, TurnReceiptStatus,
};
use nomifun_chat_model_broker::{
    ChatCausality, ChatCausalityGate, ChatModelError, ChatModelErrorCode,
    ChatRetryDirective,
};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_common::{AppError, now_ms};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::engine_session_host::EngineTurnReceipt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineJournalWrite {
    Progress,
    Settlement,
    Cleanup,
    Terminal,
}

#[derive(Default)]
struct Cursor {
    sequence: u64,
    window_start_sequence: u64,
    segment: u16,
    total_bytes: usize,
    budget: nomifun_agent_contracts::NativeExecutionBudget,
    empty_response_step: Option<u16>,
    checkpoint_revision: u64,
    bytes: usize,
    draining: bool,
    terminal: bool,
    uncertain: bool,
    assistant_step: Option<AssistantStepCursor>,
    last_assistant_event_id: Option<EventId>,
    assistant_message_id: Option<String>,
    thinking_messages: BTreeMap<u16, (String, u64)>,
    tool_message_ids: BTreeMap<String, String>,
}

struct AssistantStepCursor {
    step: u16,
    message_id: String,
    parts: u64,
    text: Vec<u8>,
    last_event_id: Option<EventId>,
}

pub(super) struct Journal {
    store: AgentSessionStore,
    lease: NativeExecutionLease,
    heartbeat: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    cancellation_link: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 0 preclaimed, 1 attached, 2 abandoned. Attachment and pre-driver
    /// failure must not both win ownership of cleanup.
    attachment: AtomicU8,
    attached_notify: tokio::sync::Notify,
    recovery: Option<Arc<nomifun_agent_runtime::AgentTurnRecovery>>,
    user: String,
    session: AgentSessionId,
    operation: OperationId,
    /// Accepted user message used by model causality and runtime records.
    root: EventId,
    /// Exact canonical turn/started predecessor for Action/Effect chains.
    turn_started: EventId,
    generation: i64,
    snapshot: ResolvedSnapshotRef,
    route: Option<ChatRouteIdentity>,
    cancellation: CancellationToken,
    cursor: Mutex<Cursor>,
    sequence: AtomicU64,
    pending: Arc<Semaphore>,
    pending_bytes: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct EngineTurnJournal(Arc<Journal>);

impl Drop for Journal {
    fn drop(&mut self) {
        if let Some(task) = self.heartbeat.get_mut().unwrap_or_else(|error| error.into_inner()).take() { task.abort(); }
        if let Some(task) = self.cancellation_link.get_mut().unwrap_or_else(|error| error.into_inner()).take() { task.abort(); }
    }
}

impl Journal {
    async fn append_projection(&self, append: &SessionEventAppend) -> Result<nomifun_agent_session::SessionEventAppendResult, nomifun_agent_session::SessionStoreError> {
        if append.semantic_event.kind.0 == "tool/call-started" {
            self.store.append_native_event(&self.lease, append, None).await
        } else {
            self.store.append_native_observation(&self.lease, append, None).await
        }
    }
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine journal: {message}"))
}

/// Derive the one assistant message identity owned by a canonical Turn.  The
/// HTTP/WS relay and durable journal must agree before either side publishes a
/// frame; otherwise terminal history refresh leaves a duplicate live row.
pub(super) fn canonical_assistant_message_id(
    root_message_id: &str,
) -> Result<String, AppError> {
    canonical_assistant_step_message_id(root_message_id, 1)
}

pub(super) fn canonical_assistant_step_message_id(
    root_message_id: &str,
    step: u16,
) -> Result<String, AppError> {
    let root = Uuid::parse_str(root_message_id)
        .map_err(|error| failure(format!("turn root is not a UUID: {error}")))?;
    if root.get_version_num() != 7 || step == 0 {
        return Err(failure("turn root is not UUIDv7"));
    }
    let mut bytes = *root.as_bytes();
    // Preserve version/variant/time ordering while selecting a deterministic,
    // distinct point in the UUIDv7 random suffix for each model step. Step 1
    // retains the existing canonical assistant ID used by the live relay.
    let suffix = u16::from_be_bytes([bytes[14], bytes[15]]) ^ step;
    let suffix_bytes = suffix.to_be_bytes();
    bytes[14] = suffix_bytes[0];
    bytes[15] = suffix_bytes[1];
    let assistant = Uuid::from_bytes(bytes);
    if assistant == root || assistant.get_version_num() != 7 {
        return Err(failure("assistant message identity derivation failed"));
    }
    Ok(assistant.to_string())
}

fn canonical_event_payload(
    session_id: &AgentSessionId,
    value: Value,
) -> Result<(SessionEventPayloadRef, Option<SessionPayloadRecord>), AppError> {
    let bytes = canonical_json_bytes(&value).map_err(failure)?;
    if bytes.len() <= nomifun_agent_session::MAX_INLINE_JSON_BYTES {
        return Ok((
            SessionEventPayloadRef::InlineJson(StrictJsonValue(value)),
            None,
        ));
    }
    if bytes.len() > nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES {
        return Err(failure(format!(
            "record exceeds canonical payload limit of {} bytes",
            nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES,
        )));
    }
    let digest = digest_bytes(&bytes);
    // `payload_id` is global while payload ownership is Session-scoped.
    let payload_id = ArtifactId::from(format!(
        "runtime-progress:{}:{}",
        session_id.as_ref(),
        digest.as_ref()
    ));
    let payload = SessionPayloadRecord {
        payload_id: payload_id.clone(),
        agent_session_id: session_id.clone(),
        media_type: "application/json".to_owned(),
        byte_len: bytes.len() as u64,
        digest,
        body: SessionPayloadBody::Json(StrictJsonValue(value)),
    };
    Ok((SessionEventPayloadRef::Stored(payload_id), Some(payload)))
}

impl EngineTurnJournal {
    pub(super) async fn require_claimed_model(
        &self,
        causality: &ChatCausality,
    ) -> Result<(), AppError> {
        let journal = &self.0;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain
            || cursor.terminal
            || cursor.draining
            || journal.cancellation.is_cancelled()
            || causality.agent_session_id != journal.session
            || causality.turn_operation_id != journal.operation
            || causality.causation_event_id != journal.root
            || causality.resolved_snapshot_ref != journal.snapshot
            || Some(&causality.route_identity) != journal.route.as_ref()
        {
            return Err(failure("resource request differs from the live model turn"));
        }
        drop(cursor);
        journal.store.verify_native_chat_operation(&journal.lease, &ChatOperationClaimRequest {
            agent_session_id: causality.agent_session_id.clone(), operation_id: causality.operation_id.clone(),
            turn_operation_id: causality.turn_operation_id.clone(), causation_event_id: causality.causation_event_id.clone(),
            route_identity: causality.route_identity.clone(), resolved_snapshot_ref: causality.resolved_snapshot_ref.clone(),
        }).await.map_err(failure)
    }

    pub(super) fn validate_receipt(&self, receipt: &EngineTurnReceipt) -> Result<(), AppError> {
        if self.0.session.as_ref() != receipt.session().session().conversation_id
            || self.0.operation.as_ref() != receipt.operation_id()
        {
            return Err(failure("journal belongs to another resource turn"));
        }
        Self::from_existing(self.0.clone(), receipt).map(|_| ())
    }

    pub(super) fn matches_tool(
        &self,
        invocation: &nomifun_engine_core::EngineToolInvocation,
    ) -> bool {
        invocation.agent_session_id == self.0.session
            && invocation.principal.principal_kind == "user"
            && invocation.principal.principal_id == self.0.user
            && invocation.resolved_snapshot_ref == self.0.snapshot
            && invocation.turn_operation_id == self.0.operation
    }

    pub(super) fn downgrade(&self) -> std::sync::Weak<Journal> {
        Arc::downgrade(&self.0)
    }

    pub(super) fn cached_generation(journal: &Arc<Journal>) -> u64 { journal.lease.generation() }

    pub(super) fn from_existing(
        journal: Arc<Journal>,
        receipt: &EngineTurnReceipt,
    ) -> Result<Self, AppError> {
        if journal.user != receipt.session().principal().principal_id
            || journal.generation != receipt.admission_epoch()
            || journal.root.as_ref() != receipt.root_message_id()
            || journal.turn_started.as_ref() != receipt.turn_started_event_id()
            || journal.snapshot != receipt.session().snapshot().snapshot_ref
        {
            return Err(failure("receipt changed for existing journal"));
        }
        Ok(Self(journal))
    }

    pub(super) fn new(
        store: AgentSessionStore,
        receipt: &EngineTurnReceipt,
        cancellation: CancellationToken,
        lease: NativeExecutionLease,
    ) -> Result<Self, AppError> {
        Self::new_with_recovery(store, receipt, cancellation, lease, None, 0, 0)
    }

    pub(super) fn new_with_recovery(store: AgentSessionStore, receipt: &EngineTurnReceipt, cancellation: CancellationToken,
        lease: NativeExecutionLease, recovery: Option<nomifun_agent_runtime::AgentTurnRecovery>, sequence: u64, total_bytes: usize) -> Result<Self, AppError> {
        let response_step = recovery.as_ref().map(|state| state.last_model_step().checked_add(1)
            .ok_or_else(|| failure("recovered model step counter exhausted"))).transpose()?.unwrap_or(1);
        let checkpoint_revision = recovery.as_ref().map_or(0, |state| state.checkpoint_revision());
        let segment = recovery.as_ref().and_then(|state| state.checkpoint().segments.as_ref()).map_or(0, |state| state.segment);
        let generation = i64::try_from(lease.generation()).map_err(failure)?;
        let assistant_message_id = canonical_assistant_step_message_id(receipt.root_message_id(), response_step)?;
        let journal = Self(Arc::new(Journal {
            store, lease, heartbeat: Default::default(), cancellation_link: Default::default(),
            attachment: AtomicU8::new(0), attached_notify: Default::default(), recovery: recovery.map(Arc::new),
            user: receipt.session().principal().principal_id.clone(),
            session: AgentSessionId::from(
                receipt.session().session().conversation_id.clone(),
            ),
            operation: OperationId::from(receipt.operation_id().to_owned()),
            root: EventId::from(receipt.root_message_id().to_owned()),
            turn_started: EventId::from(receipt.turn_started_event_id().to_owned()),
            generation,
            snapshot: receipt.session().snapshot().snapshot_ref.clone(),
            route: receipt
                .session()
                .snapshot()
                .content
                .chat_route_identity
                .clone(),
            cancellation,
            cursor: Mutex::new(Cursor {
                assistant_message_id: Some(assistant_message_id),
                sequence, window_start_sequence: sequence, segment, total_bytes, checkpoint_revision, empty_response_step: Some(response_step),
                ..Cursor::default()
            }),
            sequence: AtomicU64::new(sequence),
            pending: Arc::new(Semaphore::new(64)),
            pending_bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
        }));
        journal.start_heartbeat();
        Ok(journal)
    }

    pub(super) fn recovery(&self) -> Option<Arc<nomifun_agent_runtime::AgentTurnRecovery>> { self.0.recovery.clone() }
    pub(super) fn generation(&self) -> u64 { self.0.lease.generation() }
    pub(super) async fn pause_with_unproven_cleanup(&self) -> Result<(),AppError> {
        self.0.store.pause_native_execution(&self.0.lease,"EXECUTION_CLEANUP_UNPROVEN",false).await.map_err(failure)?;
        Ok(())
    }

    pub(super) async fn response_message_id(&self) -> Result<String,AppError> {
        self.0.cursor.lock().await.assistant_message_id.clone().ok_or_else(||failure("response message identity missing"))
    }
    pub(super) fn recovered(&self) -> bool { self.0.lease.fence() > 0 }

    /// Finish an interrupted assistant display segment without changing its
    /// text or pretending the task finished. New output has a fresh step ID.
    /// The model replay independently discards the unexecuted model tail.
    pub(super) async fn restore_public_projection(&self) -> Result<(), AppError> {
        let journal = &self.0;
        let Some(recovery) = &journal.recovery else { return Ok(()); };
        let facts = journal.store.native_recovery_facts(&journal.session, &journal.operation).await.map_err(failure)?;
        let identities = (1..=recovery.last_model_step()).map(|step| {
            canonical_assistant_step_message_id(journal.root.as_ref(), step).map(|id| (id, step))
        }).collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut open = BTreeMap::<String, AssistantStepCursor>::new();
        let mut last_event = None;
        let mut bytes = 0usize;
        for event in &facts.events {
            let Some(step) = identities.get(event.correlation_id.as_ref()) else { continue; };
            match event.kind.0.as_str() {
                "message/content-part" => {
                    let payload = facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(|| failure("recovered display payload missing"))?;
                    if payload.get("turn_id").and_then(Value::as_str) != Some(journal.root.as_ref()) { continue; }
                    let text = payload.get("content").and_then(Value::as_str).ok_or_else(|| failure("recovered display text missing"))?;
                    bytes = bytes.saturating_add(text.len());
                    if bytes > nomifun_agent_contracts::MAX_NATIVE_APPROVED_JOURNAL_BYTES as usize { return Err(failure("recovered display exceeds its bounded reader")); }
                    let current = open.entry(event.correlation_id.as_ref().to_owned()).or_insert_with(|| AssistantStepCursor {
                        step: *step, message_id: event.correlation_id.as_ref().to_owned(), parts: 0, text: Vec::new(), last_event_id: None,
                    });
                    current.parts += 1;
                    current.text.extend_from_slice(text.as_bytes());
                    current.last_event_id = Some(event.event_id.clone());
                    last_event = Some(event.event_id.clone());
                }
                "message/completed" => { open.remove(event.correlation_id.as_ref()); last_event = Some(event.event_id.clone()); }
                _ => {}
            }
        }
        if open.len() > 1 { return Err(failure("recovered display has multiple unfinished assistant segments")); }
        if let Some((_, step)) = open.into_iter().next() {
            let completed = Self::assistant_completion_event(journal, &step);
            journal.append_projection(&completed).await.map_err(failure)?;
            last_event = Some(completed.event_id);
        }
        journal.cursor.lock().await.last_assistant_event_id = last_event;
        Ok(())
    }

    pub(super) fn is_attached(&self) -> bool { self.0.attachment.load(Ordering::Acquire) == 1 }

    pub(super) fn attach_runtime(&self, cancellation: CancellationToken) -> Result<(), AppError> {
        match self.0.attachment.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {}
            Err(1) => return Ok(()),
            Err(_) => return Err(failure("recovery attachment was abandoned")),
        }
        let journal_token = self.0.cancellation.clone();
        let task = tokio::spawn(async move {
            tokio::select! { _ = cancellation.cancelled() => journal_token.cancel(), _ = journal_token.cancelled() => cancellation.cancel() }
        });
        *self.0.cancellation_link.lock().unwrap_or_else(|error| error.into_inner()) = Some(task);
        self.0.attached_notify.notify_waiters();
        Ok(())
    }

    pub(super) async fn wait_attached(&self) {
        loop {
            let notified = self.0.attached_notify.notified();
            if self.is_attached() { return; }
            notified.await;
        }
    }

    pub(super) async fn fail_unattached_recovery(&self) -> Result<(), AppError> {
        if self.0.attachment.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err(failure("runtime attachment or abandonment already owns cleanup"));
        }
        self.append(json!({"event":"host_cleanup_proven","reason":"turn_resources_not_opened"}).to_string(), None, EngineJournalWrite::Cleanup).await?;
        let event = if let Some(recovery) = &self.0.recovery {
            AgentEngineEvent::TurnPaused { model_steps: recovery.last_model_step(), reason: "EXECUTION_ATTACH_FAILED".into() }
        } else {
            AgentEngineEvent::TurnFailed { model_steps: 0, message: "Recovery could not attach an uninitialized runtime".into() }
        };
        self.append(serde_json::to_string(&event).map_err(failure)?, None, EngineJournalWrite::Terminal).await
    }

    pub(super) async fn verify_execution_lease(&self) -> Result<(), AppError> {
        self.0.store.verify_native_execution(&self.0.lease).await.map_err(failure)
    }

    fn start_heartbeat(&self) {
        let weak = Arc::downgrade(&self.0);
        let cancellation = self.0.cancellation.clone();
        let task = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! { biased; _ = cancellation.cancelled() => break, _ = ticks.tick() => {} }
                let Some(journal) = weak.upgrade() else { break; };
                if journal.cursor.lock().await.terminal { break; }
                match journal.store.heartbeat_native_execution(&journal.lease).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(nomifun_agent_session::SessionStoreError::ExecutionFenced) => { cancellation.cancel(); break; }
                    Err(_) => tracing::warn!("native execution heartbeat could not be committed"),
                }
            }
        });
        *self.0.heartbeat.lock().unwrap_or_else(|error| error.into_inner()) = Some(task);
    }

    pub fn sequence(&self) -> u64 {
        self.0.sequence.load(Ordering::Acquire)
    }

    pub(super) fn matches_generation(&self, generation: u64) -> bool { self.generation() == generation }

    pub(super) async fn refresh_budget(&self) -> Result<(), AppError> {
        let budget = self.0.store.native_execution_budget(&self.0.lease).await.map_err(failure)?;
        self.0.cursor.lock().await.budget = budget;
        Ok(())
    }

    pub async fn execution_pressure(&self) -> Result<nomifun_agent_runtime::AgentExecutionPressure, AppError> {
        use nomifun_agent_runtime::{AgentExecutionPressure, AgentExecutionStopReason};
        let journal = &self.0;
        let payload_bytes = journal.store.native_payload_bytes(&journal.lease).await.map_err(failure)?;
        let pause_requested = journal.store.native_pause_requested(&journal.lease).await.map_err(failure)?;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain || cursor.terminal || cursor.draining { return Err(failure("execution budget requested on closed journal")); }
        Ok(AgentExecutionPressure {
            // Leave room for the next bounded model/tool batch and its
            // checkpoint; settlement/cleanup also have a separate reserve.
            renew_window: cursor.sequence.saturating_sub(cursor.window_start_sequence) >= 2400 || cursor.bytes >= nomifun_agent_contracts::MAX_NATIVE_JOURNAL_WINDOW_BYTES / 2,
            stop: if pause_requested {
                Some(AgentExecutionStopReason::UserRequested)
            } else if cursor.total_bytes as u64 >= cursor.budget.journal_bytes.saturating_sub(4 * 1024 * 1024)
                || cursor.sequence >= cursor.budget.journal_records.saturating_sub(12_000) {
                Some(AgentExecutionStopReason::TurnJournalBudget)
            } else if payload_bytes >= cursor.budget.session_payload_bytes.saturating_sub(4 * 1024 * 1024) {
                Some(AgentExecutionStopReason::SessionPayloadBudget)
            } else { None },
        })
    }

    pub async fn save_execution_checkpoint(
        &self, checkpoint: nomifun_agent_runtime::AgentExecutionCheckpoint,
        owner: nomifun_agent_contracts::PrincipalRef,
    ) -> Result<Option<nomifun_agent_runtime::AgentCheckpointReceipt>, AppError> {
        checkpoint.validate().map_err(failure)?;
        let journal = self.0.clone();
        if checkpoint.binding.agent_session_id() != &journal.session
            || checkpoint.turn_operation_id != journal.operation
            || checkpoint.binding.resolved_snapshot_ref() != &journal.snapshot
            || owner.principal_id != journal.user
        { return Err(failure("checkpoint differs from admitted journal authority")); }
        let state = serde_json::to_value(&checkpoint).map_err(failure)?;
        let state_bytes = canonical_json_bytes(&state).map_err(failure)?;
        let digest = digest_bytes(&state_bytes);
        let permit = journal.pending.clone().try_acquire_owned().map_err(|_| failure("pending checkpoint bound reached"))?;
        let byte_permit = journal.pending_bytes.clone().try_acquire_many_owned(state_bytes.len() as u32)
            .map_err(|_| failure("pending checkpoint byte bound reached"))?;
        let task = tokio::spawn(async move {
            let _permit = permit;
            let _byte_permit = byte_permit;
            let mut cursor = journal.cursor.lock().await;
            if cursor.uncertain || cursor.terminal || cursor.draining || journal.cancellation.is_cancelled() {
                return Err(failure("journal no longer admits a checkpoint"));
            }
            let next = cursor.sequence.checked_add(1).ok_or_else(|| failure("journal sequence exhausted"))?;
            let revision = cursor.checkpoint_revision.checked_add(1).ok_or_else(|| failure("checkpoint revision exhausted"))?;
            let metadata = serde_json::to_value(AgentEngineEvent::ExecutionCheckpointSaved {
                step: checkpoint.model_steps, revision, digest,
            }).map_err(failure)?;
            let payload = json!({"runtime_binding_id":format!("nomi:{}",journal.session.as_ref()),
                "producer_seq":next,"event":metadata});
            let metadata_bytes = canonical_json_bytes(&payload).map_err(failure)?.len();
            let segment = checkpoint.segments.as_ref().map_or(0, |state| state.segment);
            if segment != cursor.segment && segment != cursor.segment.saturating_add(1) {
                return Err(failure("checkpoint cannot skip or rewind execution windows"));
            }
            let renew_window = cursor.segment > 0 && segment == cursor.segment + 1;
            if renew_window && checkpoint.segments.as_ref().is_none_or(|state| state.segment_start_step != checkpoint.model_steps) {
                return Err(failure("execution window renewal needs this exact checkpoint boundary"));
            }
            if next.saturating_sub(cursor.window_start_sequence) > 3200 || cursor.bytes.saturating_add(metadata_bytes) > nomifun_agent_contracts::MAX_NATIVE_JOURNAL_WINDOW_BYTES
                || cursor.total_bytes.saturating_add(metadata_bytes) as u64 > cursor.budget.journal_bytes || next > cursor.budget.journal_records {
                return Err(failure("bounded evidence journal exhausted before checkpoint"));
            }
            let identity = format!("runtime-progress:{}:{}:{next}",journal.session.as_ref(),journal.operation.as_ref());
            let append = SessionEventAppend {
                agent_session_id: journal.session.clone(), event_id: EventId::from(identity.clone()),
                producer_id: EventProducerId::from("runtime_supervisor"), idempotency_key: IdempotencyKey::from(identity),
                runtime_binding_id: None, runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("runtime/progress-recorded".into()), kind_version: 1,
                    correlation_id: CorrelationId::from(journal.operation.as_ref()), causation_event_id: Some(journal.root.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
                },
            };
            cursor.uncertain = true;
            let saved = journal.store.save_native_checkpoint(&append, nomifun_agent_session::NativeCheckpointWrite {
                owner, operation_id: journal.operation.clone(), snapshot: journal.snapshot.clone(),
                active_set_generation: checkpoint.active_set_generation,
                expected_revision: cursor.checkpoint_revision, execution_fence: journal.lease.fence(),
                lease: Some(journal.lease.clone()), state: StrictJsonValue(state),
            }).await;
            let saved = match saved {
                Ok(saved) => saved,
                Err(nomifun_agent_session::SessionStoreError::CheckpointNotQuiescent) => {
                    cursor.uncertain = false;
                    return Ok(None);
                }
                Err(error) => return Err(failure(error)),
            };
            cursor.sequence = next;
            cursor.checkpoint_revision = saved.revision;
            cursor.segment = segment;
            cursor.total_bytes += metadata_bytes;
            if renew_window {
                cursor.window_start_sequence = next;
                cursor.bytes = 0;
            } else { cursor.bytes += metadata_bytes; }
            cursor.uncertain = false;
            journal.sequence.store(next, Ordering::Release);
            Ok(Some(nomifun_agent_runtime::AgentCheckpointReceipt {
                revision: saved.revision, through_seq: saved.through_seq, digest: saved.digest,
            }))
        });
        task.await.map_err(failure)?
    }

    async fn append_progress(
        journal: &Journal,
        cursor: &Cursor,
        event: &Value,
        kind: EngineJournalWrite,
    ) -> Result<(), AppError> {
        let next = cursor.sequence.saturating_add(1);
        let value = json!({
            "runtime_binding_id": format!("nomi:{}", journal.session.as_ref()),
            "producer_seq": next,
            "event": event,
        });
        let (payload_ref, payload) = canonical_event_payload(&journal.session, value)?;
        let identity = format!(
            "runtime-progress:{}:{}:{next}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
        );
        let append = SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("runtime/progress-recorded".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(journal.operation.as_ref().to_owned()),
                causation_event_id: Some(journal.root.clone()),
                payload: payload_ref,
            },
        };
        if kind == EngineJournalWrite::Progress {
            journal.store.append_native_event(&journal.lease, &append, payload.as_ref()).await.map_err(failure)?;
        } else {
            journal.store.append_native_observation(&journal.lease, &append, payload.as_ref()).await.map_err(failure)?;
        }
        Ok(())
    }

    async fn append_assistant_part(
        journal: &Journal,
        cursor: &mut Cursor,
        step: u16,
        text: &str,
    ) -> Result<(), AppError> {
        if text.is_empty() {
            return Ok(());
        }
        if cursor.assistant_step.as_ref().is_some_and(|current| current.step != step) {
            let previous = cursor.assistant_step.take().expect("checked above");
            if step <= previous.step {
                return Err(failure("assistant model step moved backwards"));
            }
            let completed = Self::assistant_completion_event(journal, &previous);
            let completed_id = completed.event_id.clone();
            journal.append_projection(&completed).await.map_err(failure)?;
            cursor.last_assistant_event_id = Some(completed_id);
        }
        if cursor.assistant_step.is_none() {
            cursor.assistant_step = Some(AssistantStepCursor {
                step,
                message_id: canonical_assistant_step_message_id(journal.root.as_ref(), step)?,
                parts: 0,
                text: Vec::new(),
                last_event_id: None,
            });
        }
        let current = cursor.assistant_step.as_mut().expect("initialized above");
        let part = current.parts.saturating_add(1);
        let message_id = current.message_id.clone();
        let identity = format!(
            "assistant-part:{}:{}:{step}:{part}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
        );
        let event_id = EventId::from(identity.clone());
        let append = SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: event_id.clone(),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/content-part".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_id),
                causation_event_id: Some(
                    cursor
                        .last_assistant_event_id
                        .clone()
                        .unwrap_or_else(|| journal.root.clone()),
                ),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "content": text,
                    "turn_id": journal.root.as_ref(),
                    "display_at_ms": now_ms(),
                }))),
            },
        };
        journal.append_projection(&append).await.map_err(failure)?;
        current.parts = part;
        current.text.extend_from_slice(text.as_bytes());
        current.last_event_id = Some(event_id.clone());
        cursor.last_assistant_event_id = Some(event_id);
        Ok(())
    }

    fn assistant_completion_event(
        journal: &Journal,
        step: &AssistantStepCursor,
    ) -> SessionEventAppend {
        let identity = format!(
            "assistant-complete:{}:{}:{}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
            step.step,
        );
        SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/completed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(step.message_id.clone()),
                causation_event_id: Some(
                    step.last_event_id
                        .clone()
                        .unwrap_or_else(|| journal.root.clone()),
                ),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "part_count": step.parts,
                    "content_digest": digest_bytes(&step.text),
                }))),
            },
        }
    }

    async fn append_thinking_part(
        journal: &Journal,
        cursor: &mut Cursor,
        step: u16,
        text: &str,
    ) -> Result<(), AppError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let (message_id, previous_parts) = cursor
            .thinking_messages
            .entry(step)
            .or_insert_with(|| (Uuid::now_v7().to_string(), 0));
        let part = previous_parts.saturating_add(1);
        let identity = format!(
            "thinking-part:{}:{}:{step}:{part}",
            journal.session.as_ref(),
            journal.operation.as_ref(),
        );
        let (payload_ref, payload) = canonical_event_payload(
            &journal.session,
            json!({ "content": text, "turn_id": journal.root.as_ref() }),
        )?;
        let append = SessionEventAppend {
            agent_session_id: journal.session.clone(),
            event_id: EventId::from(identity.clone()),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("thinking/content-part".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_id.clone()),
                causation_event_id: Some(journal.turn_started.clone()),
                payload: payload_ref,
            },
        };
        journal
            .store
            .append_native_observation(&journal.lease, &append, payload.as_ref())
            .await
            .map_err(failure)?;
        *previous_parts = part;
        Ok(())
    }

    async fn append_tool_projection(
        journal: &Journal,
        cursor: &mut Cursor,
        value: &Value,
    ) -> Result<(), AppError> {
        match value.get("event").and_then(Value::as_str) {
            Some("host_tool_dispatch") => {
                let dispatch = value.get("dispatch").and_then(Value::as_object).ok_or_else(|| {
                    failure("host tool dispatch has no canonical payload")
                })?;
                let operation = dispatch
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no operation_id"))?;
                let call_id = dispatch
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no call_id"))?;
                if call_id.starts_with("agent-instructions:") {
                    return Ok(());
                }
                let capability_id = dispatch
                    .get("capability_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no capability_id"))?;
                let action_id = dispatch
                    .get("action_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool dispatch has no action_id"))?;
                let identity = format!(
                    "tool-call:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .entry(operation.to_owned())
                    .or_insert_with(|| Uuid::now_v7().to_string())
                    .clone();
                journal
                    .append_projection(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/call-started".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(journal.turn_started.clone()),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "call_id": call_id,
                                "capability_id": capability_id,
                                "action_id": action_id,
                                "name": dispatch.get("model_name").cloned().unwrap_or(Value::Null),
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            Some("host_tool_settled") => {
                let operation = value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool settlement has no operation_id"))?;
                let call_id = value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host tool settlement has no call_id"))?;
                if call_id.starts_with("agent-instructions:") {
                    return Ok(());
                }
                let identity = format!(
                    "tool-result:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .get(operation)
                    .cloned()
                    .ok_or_else(|| failure("host tool settlement has no admitted projection"))?;
                journal
                    .append_projection(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/result-recorded".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(EventId::from(format!(
                                "tool-call:{}:{operation}",
                                journal.session.as_ref(),
                            ))),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "call_id": call_id,
                                "output": value.get("result").cloned().unwrap_or(Value::Null),
                                "error": value.get("error").cloned().unwrap_or(Value::Null),
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            Some("host_resource_dispatch") => {
                let operation = value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host resource dispatch has no operation_id"))?;
                let call_id = value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host resource dispatch has no call_id"))?;
                let identity = format!(
                    "tool-call:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .entry(operation.to_owned())
                    .or_insert_with(|| Uuid::now_v7().to_string())
                    .clone();
                journal
                    .append_projection(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/call-started".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(journal.turn_started.clone()),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "call_id": call_id,
                                "capability_id": "mcp.server",
                                "action_id": "mcp.resource/read",
                                "name": "mcp_resource_read",
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            Some("host_resource_settled") => {
                let operation = value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure("host resource settlement has no operation_id"))?;
                let identity = format!(
                    "tool-result:{}:{operation}",
                    journal.session.as_ref(),
                );
                let projection_id = cursor
                    .tool_message_ids
                    .get(operation)
                    .cloned()
                    .ok_or_else(|| failure("host resource settlement has no admitted projection"))?;
                journal
                    .append_projection(&SessionEventAppend {
                        agent_session_id: journal.session.clone(),
                        event_id: EventId::from(identity.clone()),
                        producer_id: EventProducerId::from("runtime_supervisor"),
                        idempotency_key: IdempotencyKey::from(identity),
                        runtime_binding_id: None,
                        runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("tool/result-recorded".to_owned()),
                            kind_version: 1,
                            correlation_id: CorrelationId::from(projection_id),
                            causation_event_id: Some(EventId::from(format!(
                                "tool-call:{}:{operation}",
                                journal.session.as_ref(),
                            ))),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                                "operation_id": operation,
                                "output": {"owner_returned": value.get("owner_returned").cloned().unwrap_or(Value::Null)},
                            }))),
                        },
                    })
                    .await
                    .map_err(failure)?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn append_terminal(
        journal: &Journal,
        cursor: &Cursor,
        event: &AgentEngineEvent,
    ) -> Result<(), AppError> {
        match event {
            AgentEngineEvent::TurnPaused { reason, .. } => {
                if let Some(step) = cursor.assistant_step.as_ref() {
                    journal.append_projection(&Self::assistant_completion_event(journal, step)).await.map_err(failure)?;
                }
                journal.store.pause_native_execution(&journal.lease, reason, true).await.map_err(failure)?;
                Ok(())
            }
            AgentEngineEvent::TurnCancelled { .. } => {
                let receipt = journal
                    .store
                    .read_turn_receipt(&journal.session, &journal.operation)
                    .await
                    .map_err(failure)?;
                if receipt.status != TurnReceiptStatus::Cancelled {
                    let identity = format!("runtime-cancel:{}:{}", journal.session.as_ref(), journal.operation.as_ref());
                    journal.store.append_native_event(&journal.lease, &SessionEventAppend {
                        agent_session_id: journal.session.clone(), event_id: identity.clone().into(),
                        producer_id: "runtime_supervisor".into(), idempotency_key: identity.into(),
                        runtime_binding_id: None, runtime_producer_seq: None,
                        semantic_event: SemanticSessionEventDraft {
                            kind: SessionEventKind("turn/cancelled".into()), kind_version: 1,
                            correlation_id: journal.operation.as_ref().into(), causation_event_id: Some(journal.turn_started.clone()),
                            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":journal.operation}))),
                        },
                    }, None).await.map_err(failure)?;
                }
                Ok(())
            }
            AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnFailed { .. } => {
                let empty_step = AssistantStepCursor {
                    step: cursor.empty_response_step.unwrap_or(1),
                    message_id: cursor
                        .assistant_message_id
                        .clone()
                        .unwrap_or_else(|| Uuid::now_v7().to_string()),
                    parts: 0,
                    text: Vec::new(),
                    last_event_id: None,
                };
                let message = Self::assistant_completion_event(
                    journal,
                    cursor.assistant_step.as_ref().unwrap_or(&empty_step),
                );
                let message_event_id = message.event_id.clone();
                let (kind, payload) = match event {
                    AgentEngineEvent::TurnCompleted { model_steps, finish_reason } => (
                        "turn/completed",
                        json!({
                            "model_steps": model_steps,
                            "finish_reason": finish_reason,
                            "finished_at_ms": now_ms(),
                        }),
                    ),
                    AgentEngineEvent::TurnFailed { model_steps, message } => {
                        let error = nomifun_ai_agent::AgentSendError::from_engine_turn_failure(
                            message.clone(),
                        )
                        .into_stream_error();
                        (
                            "turn/failed",
                            json!({
                                "model_steps": model_steps,
                                "message": message,
                                "error": error,
                                "finished_at_ms": now_ms(),
                            }),
                        )
                    }
                    _ => unreachable!(),
                };
                let turn_identity = format!(
                    "turn-terminal:{}:{}",
                    journal.session.as_ref(),
                    journal.operation.as_ref(),
                );
                let turn = SessionEventAppend {
                    agent_session_id: journal.session.clone(),
                    event_id: EventId::from(turn_identity.clone()),
                    producer_id: EventProducerId::from("runtime_supervisor"),
                    idempotency_key: IdempotencyKey::from(turn_identity),
                    runtime_binding_id: None,
                    runtime_producer_seq: None,
                    semantic_event: SemanticSessionEventDraft {
                        kind: SessionEventKind(kind.to_owned()),
                        kind_version: 1,
                        correlation_id: CorrelationId::from(journal.operation.as_ref().to_owned()),
                        causation_event_id: Some(message_event_id),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
                    },
                };
                journal
                    .store
                    .append_native_chat_completion(&journal.lease, &message, &turn)
                    .await
                    .map_err(failure)?;
                Ok(())
            }
            _ => Err(failure("terminal write did not contain a terminal Runtime event")),
        }
    }

    pub async fn append(
        &self,
        payload: String,
        model_operation: Option<String>,
        kind: EngineJournalWrite,
    ) -> Result<(), AppError> {
        if payload.len() > 8 * 1024 * 1024 {
            return Err(failure("record exceeds hard byte limit"));
        }
        if model_operation
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 1024)
            || (model_operation.is_some() && kind != EngineJournalWrite::Progress)
        {
            return Err(failure("invalid model-operation admission"));
        }
        let event_value: Value = serde_json::from_str(&payload).map_err(failure)?;
        let runtime_event = serde_json::from_value::<AgentEngineEvent>(event_value.clone()).ok();
        if kind == EngineJournalWrite::Terminal && runtime_event.is_none() {
            return Err(failure("terminal write did not contain a terminal Runtime event"));
        }
        let permit = self
            .0
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| failure("pending write bound reached"))?;
        let byte_permit = self
            .0
            .pending_bytes
            .clone()
            .try_acquire_many_owned(payload.len() as u32)
            .map_err(|_| failure("pending write byte bound reached"))?;
        let journal = self.0.clone();
        let task = tokio::runtime::Handle::try_current().map_err(failure)?.spawn(async move {
            let _permit = permit;
            let _byte_permit = byte_permit;
            let mut cursor = journal.cursor.lock().await;
            if cursor.uncertain || cursor.terminal {
                return Err(failure("journal is closed or uncertain"));
            }
            if kind == EngineJournalWrite::Progress
                && (cursor.draining || journal.cancellation.is_cancelled())
            {
                return Err(failure("turn no longer admits progress"));
            }
            if kind == EngineJournalWrite::Terminal && !cursor.draining {
                return Err(failure("terminal requires host cleanup phase"));
            }
            let reserved = matches!(kind, EngineJournalWrite::Cleanup | EngineJournalWrite::Terminal)
                || (kind == EngineJournalWrite::Settlement
                    && (cursor.draining || journal.cancellation.is_cancelled()));
            let (records, bytes) = if reserved {
                (4095_u64, 8 * 1024 * 1024)
            } else {
                (3200_u64, nomifun_agent_contracts::MAX_NATIVE_JOURNAL_WINDOW_BYTES)
            };
            let accounted_bytes = payload.len().saturating_add(256);
            let next_bytes = cursor.bytes.saturating_add(accounted_bytes);
            let next_total_bytes = cursor.total_bytes.saturating_add(accounted_bytes);
            let total_limit = cursor.budget.journal_bytes.saturating_add(if reserved { 4 * 1024 * 1024 } else { 0 });
            if cursor.sequence.saturating_sub(cursor.window_start_sequence) >= records || next_bytes > bytes
                || next_total_bytes as u64 > total_limit || cursor.sequence >= cursor.budget.journal_records.saturating_add(if reserved { 4000 } else { 0 }) {
                return Err(failure("bounded evidence journal exhausted"));
            }
            cursor.uncertain = true;
            Self::append_progress(&journal, &cursor, &event_value, kind).await?;
            Self::append_tool_projection(&journal, &mut cursor, &event_value).await?;
            if let Some(AgentEngineEvent::OutputTextDelta { step, text } | AgentEngineEvent::CompletionDelivered { step, text }) = &runtime_event {
                Self::append_assistant_part(&journal, &mut cursor, *step, text).await?;
            }
            if let Some(AgentEngineEvent::ReasoningDelta { step, text }) = &runtime_event {
                Self::append_thinking_part(&journal, &mut cursor, *step, text).await?;
            }
            if kind == EngineJournalWrite::Terminal {
                Self::append_terminal(
                    &journal,
                    &cursor,
                    runtime_event.as_ref().expect("terminal Runtime event checked above"),
                )
                .await?;
            }
            cursor.sequence = cursor.sequence.saturating_add(1);
            cursor.bytes = next_bytes;
            cursor.total_bytes = next_total_bytes;
            cursor.draining |= matches!(kind, EngineJournalWrite::Cleanup | EngineJournalWrite::Terminal);
            cursor.terminal = kind == EngineJournalWrite::Terminal;
            cursor.uncertain = false;
            journal.sequence.store(cursor.sequence, Ordering::Release);
            Ok(())
        });
        task.await.map_err(failure)?
    }
}

#[async_trait]
impl ChatCausalityGate for EngineTurnJournal {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        let reject = |reason: &str| {
            ChatModelError::new(
                ChatModelErrorCode::CausalityRejected,
                reason,
                ChatRetryDirective::Never,
            )
        };
        let journal = &self.0;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain
            || cursor.terminal
            || cursor.draining
            || journal.cancellation.is_cancelled()
            || causality.agent_session_id != journal.session
            || causality.turn_operation_id != journal.operation
            || causality.causation_event_id != journal.root
            || causality.resolved_snapshot_ref != journal.snapshot
            || Some(&causality.route_identity) != journal.route.as_ref()
        {
            return Err(reject("model request differs from admitted live turn"));
        }
        drop(cursor);
        if journal
            .store
            .has_unsettled_effects(&journal.session)
            .await
            .map_err(|_| reject("cannot establish durable effect fence"))?
        {
            return Err(reject("an earlier effect has no settled outcome"));
        }
        journal
            .store
            .claim_native_chat_operation(&journal.lease, ChatOperationClaimRequest {
                agent_session_id: journal.session.clone(),
                operation_id: causality.operation_id.clone(),
                turn_operation_id: journal.operation.clone(),
                causation_event_id: journal.root.clone(),
                route_identity: causality.route_identity.clone(),
                resolved_snapshot_ref: causality.resolved_snapshot_ref.clone(),
            })
            .await
            .map_err(|_| reject("model operation already claimed or turn was fenced"))?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) async fn test_fixture() -> (EngineTurnJournal, nomifun_db::SqlitePool) {
    use nomifun_agent_contracts::{
        AgentBindingValue, AgentPresetId, AgentSessionLiveRecord, AgentSessionMetadata,
        DigestHex, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId,
    };
    use nomifun_agent_session::CreateSessionRequest;

    let database = nomifun_db::init_database_memory().await.unwrap();
    let pool = database.pool().clone();
    let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
    let session_id = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
    };
    let binding = AgentBindingValue {
        preset_revision_ref: PresetRevisionRef {
            preset_id: AgentPresetId::from("preset"),
            revision: 1,
            revision_digest: DigestHex::from("a".repeat(64)),
        },
        resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("b".repeat(64)),
        },
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let created = store
        .create_session(CreateSessionRequest::new(
            AgentSessionLiveRecord {
                agent_session_id: session_id.clone(),
                owner_ref: owner.clone(),
                metadata: AgentSessionMetadata {
                    title: Some("fixture".into()),
                    archived: false,
                    pinned: false,
                    reasoning_effort: None,
                },
                agent_binding: binding.clone(),
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            },
            1,
            "open",
            EventProducerId::from("session_api"),
            IdempotencyKey::from("open"),
            CorrelationId::from("open"),
        ))
        .await
        .unwrap();
    let ready = SessionEventAppend {
        agent_session_id: session_id.clone(),
        event_id: EventId::from("ready"),
        producer_id: EventProducerId::from("runtime_supervisor"),
        idempotency_key: IdempotencyKey::from("ready"),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("session/ready".into()),
            kind_version: 1,
            correlation_id: CorrelationId::from("ready"),
            causation_event_id: Some(created.opening_ack.event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
        },
    };
    store.append_event(&ready).await.unwrap();
    let input = StrictJsonValue(json!({
        "content": "fixture",
        "admission": {
            "route_identity": {
                "model_task": "chat",
                "provider_id": "provider",
                "model": "model",
                "protocol": "openai_chat_completions",
                "connection_role": "default",
                "capability_revision": 1
            },
            "resolved_snapshot_ref": binding.resolved_snapshot_ref,
        }
    }));
    let (message, turn) = store
        .start_turn(
            &session_id,
            EventProducerId::from("session_api"),
            IdempotencyKey::from("turn-key"),
            OperationId::from("turn"),
            input,
        )
        .await
        .unwrap();
    let root = message.record.unwrap().event_id;
    let turn_started = turn.record.as_ref().unwrap().event_id.clone();
    let lease = store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
        owner: owner.clone(), agent_session_id: session_id.clone(), operation_id: "turn".into(),
        snapshot: binding.resolved_snapshot_ref.clone(), active_set_generation: 0,
        holder: Uuid::now_v7().to_string(), expected_fence: 0, checkpoint: None,
    }).await.unwrap();
    let journal = EngineTurnJournal(Arc::new(Journal {
        store, lease, heartbeat: Default::default(), cancellation_link: Default::default(),
        attachment: AtomicU8::new(0), attached_notify: Default::default(), recovery: None,
        user: owner.principal_id,
        session: session_id,
        operation: OperationId::from("turn"),
        root,
        turn_started,
        generation: turn.cursor.seq as i64,
        snapshot: binding.resolved_snapshot_ref,
        route: None,
        cancellation: CancellationToken::new(),
        cursor: Mutex::new(Cursor::default()),
        sequence: AtomicU64::new(0),
        pending: Arc::new(Semaphore::new(64)),
        pending_bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
    }));
    (journal, pool)
}

#[cfg(test)]
#[path = "engine_journal_reliability_tests.rs"]
mod reliability_tests;

#[cfg(test)]
mod history_display_tests {
    use super::*;
    use super::super::runtime_event_buffer::AgentEventBuffer;

    #[tokio::test]
    async fn checkpoint_uses_the_production_journal_and_disappears_on_completion() {
        use nomifun_agent_runtime::{AgentExecutionCheckpoint, EngineBinding};
        let (journal, pool) = test_fixture().await;
        let owner = nomifun_agent_contracts::PrincipalRef { principal_kind: "user".into(), principal_id: journal.0.user.clone() };
        let checkpoint = AgentExecutionCheckpoint {
            version: 1,
            binding: EngineBinding::new(journal.0.session.clone(), "native-binding".into(), "test".into(), "a".repeat(64).into(), journal.0.snapshot.clone()).unwrap(),
            turn_operation_id: journal.0.operation.clone(), active_set_generation: 0,
            model_steps: 0, tool_call_count: 0, accepted_input_count: 1, applied_steering_receipts: vec![],
            plan: Default::default(), work: Default::default(), patch_recovery: Default::default(),
            segments: None, control_rejections: Default::default(),
        };
        let receipt = journal.save_execution_checkpoint(checkpoint.clone(), owner.clone()).await.unwrap().unwrap();
        assert_eq!(receipt.revision, 1);
        assert_eq!(journal.sequence(), 1);
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let saved = store.load_native_checkpoint(&owner, &journal.0.session, &journal.0.operation).await.unwrap().unwrap();
        let restored: AgentExecutionCheckpoint = serde_json::from_value(saved.state.0).unwrap();
        assert_eq!(restored, checkpoint);
        assert_eq!(receipt.through_seq, saved.through_seq);
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(), None, EngineJournalWrite::Cleanup).await.unwrap();
        assert!(journal.save_execution_checkpoint(checkpoint, owner.clone()).await.is_err());
        journal.append(serde_json::to_string(&AgentEngineEvent::TurnCompleted {
            model_steps: 0, finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed,
        }).unwrap(), None, EngineJournalWrite::Terminal).await.unwrap();
        assert!(store.load_native_checkpoint(&owner, &journal.0.session, &journal.0.operation).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn public_progress_keeps_model_steps_separate_across_tool_history() {
        let (journal, pool) = test_fixture().await;
        for event in [
            serde_json::to_value(AgentEngineEvent::OutputTextDelta {
                step: 1,
                text: "I found the cause.".into(),
            }).unwrap(),
            json!({
                "event": "host_tool_dispatch",
                "dispatch": {
                    "operation_id": "read-op",
                    "call_id": "read-call",
                    "capability_id": "workspace.files",
                    "action_id": "workspace.files/read",
                    "model_name": "read_file"
                }
            }),
            json!({
                "event": "host_tool_settled",
                "operation_id": "read-op",
                "call_id": "read-call",
                "result": "read complete"
            }),
            serde_json::to_value(AgentEngineEvent::OutputTextDelta {
                step: 2,
                text: "The check passed.".into(),
            }).unwrap(),
        ] {
            journal.append(event.to_string(), None, EngineJournalWrite::Progress)
                .await.unwrap();
        }
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(), None, EngineJournalWrite::Cleanup)
            .await.unwrap();
        journal.append(
            serde_json::to_string(&AgentEngineEvent::TurnCompleted {
                model_steps: 2,
                finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed,
            }).unwrap(),
            None,
            EngineJournalWrite::Terminal,
        ).await.unwrap();

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store.message_history_before(&journal.0.session, None, 50)
            .await.unwrap();
        let mut notes = history.iter()
            .filter(|row| row.presentation_intent == "message"
                && row.projection["content"].as_str().is_some_and(|text| {
                    text == "I found the cause." || text == "The check passed."
                }))
            .collect::<Vec<_>>();
        notes.sort_by_key(|row| row.first_seq);
        let tool = history.iter().find(|row| row.presentation_intent == "tool")
            .expect("tool history exists");
        assert_eq!(notes.len(), 2);
        assert!(notes[0].first_seq < tool.first_seq);
        assert!(tool.first_seq < notes[1].first_seq);
        assert_ne!(notes[0].projection_id, notes[1].projection_id);
        assert!(notes.iter().all(|row| row.projection["state"] == "completed"));
        assert!(notes.iter().all(|row| row.projection["turn_id"] == journal.0.root.as_ref()));
    }

    #[tokio::test]
    async fn engine_step_limit_projects_a_local_incomplete_turn_error() {
        let (journal, pool) = test_fixture().await;
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(), None, EngineJournalWrite::Cleanup)
            .await.unwrap();
        let terminal = AgentEngineEvent::TurnFailed {
            model_steps: 32,
            message: "model step limit of 32 exceeded".into(),
        };
        journal.append(serde_json::to_string(&terminal).unwrap(), None, EngineJournalWrite::Terminal)
            .await.unwrap();

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store.message_history_before(&journal.0.session, None, 50)
            .await.unwrap();
        let summary = history.iter().find(|projection| projection.presentation_intent == "turn_summary")
            .expect("terminal summary persisted");
        assert_eq!(summary.projection["error"]["code"], "NOMIFUN_TASK_INCOMPLETE");
        assert_eq!(summary.projection["error"]["ownership"], "nomifun");
    }

    #[tokio::test]
    async fn internal_instruction_preflight_keeps_audit_events_out_of_message_history() {
        let (journal, pool) = test_fixture().await;
        for event in [
            json!({
                "event": "host_tool_dispatch",
                "dispatch": {
                    "operation_id": "instruction-read-1",
                    "call_id": "agent-instructions:100",
                    "capability_id": "workspace.files",
                    "action_id": "workspace.files/read",
                    "model_name": "read_file"
                }
            }),
            json!({
                "event": "host_tool_settled",
                "operation_id": "instruction-read-1",
                "call_id": "agent-instructions:100",
                "error": "instruction discovery failed"
            }),
        ] {
            journal.append(event.to_string(), None, EngineJournalWrite::Progress).await.unwrap();
        }

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store
            .message_history_before(&journal.0.session, None, 50)
            .await
            .unwrap();
        assert!(history.iter().all(|message| message.presentation_intent != "tool"));
        assert_eq!(journal.sequence(), 2);
    }

    #[tokio::test]
    async fn buffered_thinking_survives_a_cold_history_read() {
        let (journal, pool) = test_fixture().await;
        let mut buffer = AgentEventBuffer::default();
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta {
            step: 1,
            text: "Inspect the workspace. ".to_owned(),
        }).is_empty());
        let mut records = Vec::new();
        buffer.flush(&mut records);
        assert_eq!(records.len(), 1);
        journal.append(
            serde_json::to_string(&records[0]).unwrap(),
            None,
            EngineJournalWrite::Progress,
        ).await.unwrap();

        let store = AgentSessionStore::from_pool(pool).await.unwrap();
        let (history, _, _) = store
            .message_history_before(&journal.0.session, None, 50)
            .await
            .unwrap();
        let thinking = history.iter().find(|projection| projection.presentation_intent == "thinking")
            .expect("thinking projection is present in a new history reader");
        assert_eq!(thinking.projection["content"], "Inspect the workspace. ");
        assert_eq!(thinking.projection["turn_id"], journal.0.root.as_ref());
        store.rebuild_projections(&journal.0.session).await.unwrap();
        let (rebuilt, _, _) = store
            .message_history_before(&journal.0.session, None, 50)
            .await
            .unwrap();
        assert_eq!(
            rebuilt.iter().find(|projection| projection.presentation_intent == "thinking")
                .unwrap().projection["content"],
            "Inspect the workspace. "
        );
    }
}
