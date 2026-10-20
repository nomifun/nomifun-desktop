//! Typed Session boundary used by Cron execution.

use async_trait::async_trait;
use nomifun_agent_contracts::AgentSessionId;

use nomifun_api_types::{AgentBindingValueDto, ConversationResponse};

use nomifun_common::AppError;

/// One Cron turn handed to the canonical Session owner.
///
/// Cron supplies only the user-visible message and the server-owned per-run
/// overlay (for example the Cron id and selected skills). The Session port
/// resolves the authoritative agent type, model, delegation policy, workspace
/// identity and conversation creation timestamp from its own projection before
/// it constructs any runtime preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnMessage {
    /// Original task body, without server execution framing or metadata.
    /// The host places its trusted run instructions in the instruction lane.
    pub content: String,
    pub files: Vec<String>,
    pub inject_skills: Vec<String>,
    pub hidden: bool,
    pub origin: Option<String>,
    pub channel_platform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnRuntimeOverlay {
    /// Cron-owned annotation used for artifacts, diagnostics, and runtime
    /// attribution. It is not a Session identity or runtime selector.
    pub cron_job_id: String,
    /// Exact durable reservation that authorized this turn. New-conversation
    /// jobs intentionally do not store their per-run Session on `cron_jobs`,
    /// so the reservation is the canonical job/Session relation.
    pub cron_job_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnRuntimePreparation {
    pub overlay: CronTurnRuntimeOverlay,
    pub clear_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnRequest {
    pub message: CronTurnMessage,
    pub runtime: CronTurnRuntimePreparation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronRuntimePreparationRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
    pub turn: CronTurnRequest,
}

/// Canonical handle exposed to Cron execution.  The scheduler does not need
/// the legacy Conversation DTO or its mutable `extra` map; it only needs the
/// stable AgentSession identity and the host-resolved workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSessionHandle {
    pub agent_session_id: AgentSessionId,
    pub workspace: String,
}

/// Canonical Session projection used by Cron scheduling and execution.
///
/// Every field is an explicit capability input. Cron never receives or scans a
/// mutable Session metadata bag in production.
#[derive(Debug, Clone, PartialEq)]
pub struct CronSessionProjection {
    pub agent_session_id: AgentSessionId,
    pub owner_id: String,
    pub name: String,
    pub workspace: String,
    pub cron_job_id: Option<String>,
    pub skills: Vec<String>,
    /// Read-only presentation from this Session's current canonical artifacts.
    /// These fields never select or override scheduled execution authority.
    pub agent_binding: Option<nomifun_api_types::AgentBindingValueDto>,
    pub agent_selection: Option<nomifun_api_types::ProductAgentSelectionDto>,
    pub model: Option<nomifun_api_types::AgentChatModelSelectionDto>,
    pub agent_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronScheduledSessionLookup {
    pub owner_id: String,
    pub cron_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSessionLookup {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
}

/// Exact operation receipt owned by the Cron boundary, including nonterminal
/// suspension. Consumers never need a Conversation row or runtime handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub turn_outcome: Option<nomifun_agent_contracts::AgentTurnOutcome>,
    pub user_input_request: Option<nomifun_agent_contracts::AgentUserInputRequest>,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

impl CronTurnDelivery {
    pub(crate) fn failure_projection(&self) -> nomifun_api_types::AgentStreamErrorData {
        let code = self
            .result_error_code
            .as_ref()
            .and_then(|code| serde_json::from_value(serde_json::Value::String(code.clone())).ok());
        let mut failure = nomifun_api_types::AgentStreamErrorData::legacy(
            self.result_error
                .clone()
                .or_else(|| self.result_text.clone())
                .unwrap_or_else(|| "The scheduled Agent turn failed".into()),
            code,
        );
        if failure.code.is_none() {
            failure.code = code;
        }
        failure.retryable = self.result_error_retryable.or(failure.retryable);
        failure
    }

    /// A closed execution is not necessarily a delivered scheduled result.
    /// Waiting and paused facts remain nonterminal; missing facts fail closed.
    /// A closed question requires human input and never authorizes replay.
    pub(crate) fn disposition(&self) -> Option<CronTurnDisposition> {
        use nomifun_agent_contracts::AgentTurnOutcome;
        let outcome = self.turn_outcome.as_ref()?;
        if matches!(outcome, AgentTurnOutcome::Waiting { .. }) {
            return self.completed.then(|| {
                CronTurnDisposition::Failed(nomifun_api_types::AgentStreamErrorData::legacy(
                    "A completed scheduled Turn cannot retain a waiting disposition",
                    None,
                ))
            });
        }
        if !self.completed {
            return None;
        }
        match outcome {
            AgentTurnOutcome::InputRequired { request_id } => {
                Some(match &self.user_input_request {
                    Some(request)
                        if request.request_id.as_ref() == request_id
                            && request.validate().is_ok() =>
                    {
                        CronTurnDisposition::InputRequired
                    }
                    _ => CronTurnDisposition::Failed(
                        nomifun_api_types::AgentStreamErrorData::legacy(
                            "The scheduled Agent input handoff has no matching canonical question",
                            None,
                        ),
                    ),
                })
            }
            AgentTurnOutcome::Answered { .. } if self.user_input_request.is_none() => {
                self.result_ok.map(|ok| {
                    if ok {
                        CronTurnDisposition::Delivered
                    } else {
                        CronTurnDisposition::Failed(self.failure_projection())
                    }
                })
            }
            AgentTurnOutcome::Unavailable => Some(CronTurnDisposition::Failed(
                nomifun_api_types::AgentStreamErrorData::legacy(
                    "The scheduled Agent turn closed without a public answer or input handoff; its effects will not be replayed",
                    None,
                ),
            )),
            AgentTurnOutcome::Paused { .. } => None,
            AgentTurnOutcome::Cancelled { .. } => Some(CronTurnDisposition::Cancelled),
            _ => Some(CronTurnDisposition::Failed(self.failure_projection())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CronTurnDisposition {
    Delivered,
    InputRequired,
    Cancelled,
    Failed(nomifun_api_types::AgentStreamErrorData),
}

/// Result of the host-owned preparation gate.
///
/// `workspace` is the canonical workspace resolved from the latest Session
/// snapshot inside the same preparation lease that admitted the turn. Cron
/// may use it for post-turn Cron-owned artifacts, but cannot choose or
/// override it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronPreparedTurnDelivery {
    pub delivery: CronTurnDelivery,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronTurnReceiptState {
    Missing,
    Accepted { message_id: String },
    Suspended(CronTurnDelivery),
    Completed(CronTurnDelivery),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnReceiptQuery {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnCancellationRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub cron_job_id: String,
    pub cron_job_run_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CronTurnCancellationReceipt {
    /// Positive exact-owner/no-admission evidence, never just a cancel signal.
    pub execution_stopped: bool,
}

/// The immutable Session created for one new-conversation Cron occurrence.
/// A successful handoff means the host retained the cleanup responsibility;
/// it does not claim that physical resources have already closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronRunResourceRetirement {
    pub owner_id: String,
    pub cron_job_id: String,
    pub cron_job_run_id: String,
    pub agent_session_id: AgentSessionId,
    finalized: Option<CronRunResourceFinalization>,
}

/// A live receipt from the original formal finalizer, retained across deletion
/// of the job/run rows. Its scope cannot be borrowed by editing a public field
/// on a cloned request. It is neither serialized nor a second durable ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CronRunResourceFinalization {
    owner_id: String,
    cron_job_id: String,
    cron_job_run_id: String,
    agent_session_id: AgentSessionId,
    status: &'static str,
}

impl CronRunResourceRetirement {
    pub(crate) fn capture(
        owner_id: String,
        cron_job_id: String,
        cron_job_run_id: String,
        agent_session_id: AgentSessionId,
    ) -> Self {
        Self {
            owner_id,
            cron_job_id,
            cron_job_run_id,
            agent_session_id,
            finalized: None,
        }
    }

    pub(crate) fn seal_finalized(mut self, status: &'static str) -> Option<Self> {
        if !matches!(status, "ok" | "error" | "cancelled") {
            return None;
        }
        self.finalized = Some(CronRunResourceFinalization {
            owner_id: self.owner_id.clone(),
            cron_job_id: self.cron_job_id.clone(),
            cron_job_run_id: self.cron_job_run_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            status,
        });
        Some(self)
    }

    /// Only the exact source tuple sealed by an Applied/AlreadyApplied formal
    /// finalizer can authorize the host resource handoff. A prepared request,
    /// or a clone whose public coordinates were changed, has no such authority.
    pub fn finalized_status(&self) -> Option<&str> {
        let finalized = self.finalized.as_ref()?;
        (self.owner_id == finalized.owner_id
            && self.cron_job_id == finalized.cron_job_id
            && self.cron_job_run_id == finalized.cron_job_run_id
            && self.agent_session_id == finalized.agent_session_id)
            .then_some(finalized.status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnReconciliationRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnDeliveryQuery {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
    pub message: CronTurnMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronTurnReconciliation {
    LiveExactOwnerWait,
    ReconciledOrTerminalReRead,
    ExternalProofRequiredFailClosed,
    StaleConflict,
}

/// Exact Session operations needed by the Cron domain.
#[async_trait]
pub trait CronSessionPort: Send + Sync {
    async fn retire_new_conversation_run_resources(
        &self,
        request: &CronRunResourceRetirement,
    ) -> Result<(), AppError>;

    /// Only an already persisted exact reservation cancellation can authorize
    /// this operation. Never cancel the current/latest Turn of a Session.
    async fn cancel_reserved_turn(
        &self,
        _request: &CronTurnCancellationRequest,
    ) -> Result<CronTurnCancellationReceipt, AppError> {
        Err(AppError::Conflict(
            "exact scheduled Turn cancellation is unavailable".into(),
        ))
    }
    async fn get_session(
        &self,
        query: &CronSessionLookup,
    ) -> Result<CronSessionProjection, AppError>;

    /// Legacy HTTP projection retained for the existing Cron conversations
    /// endpoint. Core scheduling and execution use `get_session` with the
    /// relation selected from the authoritative Cron row.
    async fn list_conversation_responses_for_cron(
        &self,
        query: &CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError>;

    async fn read_turn_receipt(
        &self,
        query: &CronTurnReceiptQuery,
    ) -> Result<CronTurnReceiptState, AppError>;

    /// Read-only failure projection of this exact canonical Turn. The host
    /// overrides this to retain the original typed terminal context.
    async fn read_turn_failure(
        &self,
        query: &CronTurnReceiptQuery,
    ) -> Result<Option<nomifun_api_types::AgentStreamErrorData>, AppError> {
        Ok(match self.read_turn_receipt(query).await? {
            CronTurnReceiptState::Completed(delivery) if delivery.result_ok == Some(false) => {
                Some(delivery.failure_projection())
            }
            _ => None,
        })
    }

    async fn reconcile_turn_receipt(
        &self,
        request: &CronTurnReconciliationRequest,
    ) -> Result<CronTurnReconciliation, AppError>;

    async fn create_idempotent(
        &self,
        user_id: &str,
        name: &str,
        agent_binding: AgentBindingValueDto,
        creation_key: &str,
        cron_job_id: &str,
    ) -> Result<CronSessionHandle, AppError>;

    async fn prepare_runtime_and_send(
        &self,
        request: CronRuntimePreparationRequest,
    ) -> Result<CronPreparedTurnDelivery, AppError>;

    async fn delivery_result(
        &self,
        query: &CronTurnDeliveryQuery,
    ) -> Result<Option<CronTurnDelivery>, AppError>;

    /// Append a durable owner-visible scheduler notice to the canonical
    /// AgentSession projection. Implementations must not write legacy message
    /// tables. Test adapters may ignore notices.
    async fn append_notice(
        &self,
        _owner_id: &str,
        _agent_session_id: &AgentSessionId,
        _content: &str,
        _notice_kind: &str,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    pub(crate) fn test_binding() -> nomifun_api_types::AgentBindingValueDto {
        serde_json::from_value(serde_json::json!({"preset_revision_ref":{"preset_id":"0190f5fe-7c00-7a00-8000-000000000901","revision":1,"revision_digest":"a".repeat(64)},"resolved_snapshot_ref":{"snapshot_id":"0190f5fe-7c00-7a00-8000-000000000902","snapshot_digest":"b".repeat(64)},"typed_resource_bindings":[],"binding_version":1})).unwrap()
    }

    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    /// Receipt consumers must never turn observation into execution.
    pub(crate) struct ReceiptOnlySessions {
        pub receipt: Mutex<CronTurnReceiptState>,
        pub reads: AtomicUsize,
        pub transient_failures: AtomicUsize,
        pub cancellations: Mutex<Vec<CronTurnCancellationRequest>>,
        pub cancellation_gate: Mutex<Option<std::sync::Arc<CancellationGate>>>,
    }
    #[derive(Default)]
    pub(crate) struct CancellationGate {
        pub entered: tokio::sync::Notify,
        pub release: tokio::sync::Notify,
    }
    impl ReceiptOnlySessions {
        pub fn new(receipt: CronTurnReceiptState) -> Self {
            Self {
                receipt: Mutex::new(receipt),
                reads: AtomicUsize::new(0),
                transient_failures: AtomicUsize::new(0),
                cancellations: Mutex::new(Vec::new()),
                cancellation_gate: Mutex::new(None),
            }
        }
    }
    #[async_trait]
    impl CronSessionPort for ReceiptOnlySessions {
        async fn retire_new_conversation_run_resources(
            &self,
            _: &CronRunResourceRetirement,
        ) -> Result<(), AppError> {
            // Receipt-only adapters own no Runtime or physical resources.
            Ok(())
        }
        async fn cancel_reserved_turn(
            &self,
            request: &CronTurnCancellationRequest,
        ) -> Result<CronTurnCancellationReceipt, AppError> {
            self.cancellations.lock().unwrap().push(request.clone());
            let gate = self.cancellation_gate.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            let mut receipt = self.receipt.lock().unwrap();
            let current = match receipt.clone() {
                CronTurnReceiptState::Missing => {
                    return Ok(CronTurnCancellationReceipt {
                        execution_stopped: true,
                    });
                }
                CronTurnReceiptState::Completed(_) => {
                    return Ok(CronTurnCancellationReceipt {
                        execution_stopped: false,
                    });
                }
                CronTurnReceiptState::Suspended(delivery) => delivery,
                CronTurnReceiptState::Accepted { message_id } => CronTurnDelivery {
                    message_id,
                    replayed: true,
                    completed: false,
                    turn_outcome: None,
                    user_input_request: None,
                    result_ok: None,
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                },
            };
            *receipt = CronTurnReceiptState::Completed(CronTurnDelivery {
                completed: true,
                turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Cancelled {
                    message_ids: Vec::new(),
                }),
                result_ok: Some(false),
                result_error_code: Some("cancelled".into()),
                ..current
            });
            // This adapter publishes a durable terminal, not a native cleanup
            // receipt. It deliberately cannot claim physical completion.
            Ok(CronTurnCancellationReceipt {
                execution_stopped: false,
            })
        }
        async fn read_turn_receipt(
            &self,
            _: &CronTurnReceiptQuery,
        ) -> Result<CronTurnReceiptState, AppError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self
                .transient_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    (remaining > 0).then(|| remaining - 1)
                })
                .is_ok()
            {
                return Err(AppError::Internal(
                    "transient receipt query interruption".into(),
                ));
            }
            Ok(self.receipt.lock().unwrap().clone())
        }
        async fn reconcile_turn_receipt(
            &self,
            _: &CronTurnReconciliationRequest,
        ) -> Result<CronTurnReconciliation, AppError> {
            Ok(CronTurnReconciliation::LiveExactOwnerWait)
        }
        async fn get_session(
            &self,
            query: &CronSessionLookup,
        ) -> Result<CronSessionProjection, AppError> {
            // Event/API presentation may read this exact Session. Creation,
            // runtime preparation and business submission remain forbidden.
            Ok(CronSessionProjection {
                agent_session_id: query.agent_session_id.clone(),
                owner_id: query.owner_id.clone(),
                name: "Original receipt Session".into(),
                workspace: String::new(),
                cron_job_id: None,
                skills: Vec::new(),
                agent_binding: None,
                agent_selection: None,
                model: None,
                agent_label: None,
            })
        }
        async fn list_conversation_responses_for_cron(
            &self,
            _: &CronScheduledSessionLookup,
        ) -> Result<Vec<ConversationResponse>, AppError> {
            panic!("receipt observation cannot discover another Session")
        }
        async fn create_idempotent(
            &self,
            _: &str,
            _: &str,
            _: AgentBindingValueDto,
            _: &str,
            _: &str,
        ) -> Result<CronSessionHandle, AppError> {
            panic!("receipt observation cannot create a Session")
        }
        async fn prepare_runtime_and_send(
            &self,
            _: CronRuntimePreparationRequest,
        ) -> Result<CronPreparedTurnDelivery, AppError> {
            panic!("receipt observation cannot prepare or send another Turn")
        }
        async fn delivery_result(
            &self,
            _: &CronTurnDeliveryQuery,
        ) -> Result<Option<CronTurnDelivery>, AppError> {
            panic!("receipt observation only reads its exact durable receipt")
        }
    }
}
