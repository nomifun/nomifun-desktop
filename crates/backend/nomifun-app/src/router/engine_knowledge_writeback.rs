//! Post-answer Knowledge work owned by the existing resource lifecycle.
//! These handles are cleanup witnesses, never a queue or execution ledger.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use nomifun_agent_contracts::{
    EventId, KnowledgeWritebackNoticeFile, KnowledgeWritebackNoticeStatus as Status,
    KnowledgeWritebackSettledV1, OperationId, KnowledgeWritebackDiagnostic, KnowledgeWritebackStage,
};
use nomifun_agent_session::{AgentSessionStore, KnowledgeWritebackStartRequest};
use nomifun_ai_agent::engine_sdk::EngineTaskGroup;
use nomifun_common::AppError;
use tokio_util::sync::CancellationToken;

use super::{EngineKernelSession, explicitly_requests_knowledge_writeback, failure};

struct Settlement {
    store: AgentSessionStore,
    owner: nomifun_agent_contracts::PrincipalRef,
    session: nomifun_agent_contracts::AgentSessionId,
    start: EventId,
    outcome: KnowledgeWritebackSettledV1,
    host: Arc<super::super::engine_session_host::EngineSessionHost>,
}

#[derive(Default)]
struct State {
    tokens: BTreeMap<String, CancellationToken>,
    // Retain the actual owner result if its canonical commit fails. Cleanup
    // retries that same settlement; it never runs extraction or writes again.
    settlements: BTreeMap<String, Arc<Settlement>>,
}

pub(super) struct KnowledgeWritebacks {
    binding_version: u64,
    tasks: EngineTaskGroup,
    state: Arc<Mutex<State>>,
}

impl KnowledgeWritebacks {
    pub(super) fn has_unsettled_work(&self)->Result<bool,AppError> {
        let state=self.state.lock().map_err(|_|failure("Knowledge task ownership poisoned"))?;
        Ok(!state.tokens.is_empty()||!state.settlements.is_empty())
    }
    pub(super) fn running_operations(&self) -> Result<Vec<OperationId>,AppError> {
        let state=self.state.lock().map_err(|_|failure("Knowledge task ownership poisoned"))?;
        Ok(state.tokens.keys().cloned().map(OperationId::from).collect())
    }
    pub(super) fn stop_requested(&self,operation:&OperationId)->Result<bool,AppError> {
        Ok(self.state.lock().map_err(|_|failure("Knowledge task ownership poisoned"))?.tokens
            .get(operation.as_ref()).is_some_and(CancellationToken::is_cancelled))
    }
    pub(super) fn new(binding_version: u64) -> Result<Self, AppError> {
        Ok(Self { binding_version, tasks: EngineTaskGroup::new(32)?,
            state: Default::default() })
    }

    pub(super) fn close(&self, cancel: bool) -> Result<(), AppError> {
        self.tasks.close()?;
        if cancel { self.cancel_all()?; }
        Ok(())
    }

    fn cancel(&self, operation: &str) -> Result<bool, AppError> {
        let state = self.state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?;
        if let Some(token) = state.tokens.get(operation) { token.cancel(); return Ok(true); }
        Ok(false)
    }

    fn cancel_all(&self) -> Result<(), AppError> {
        for token in self.state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?.tokens.values() {
            token.cancel();
        }
        Ok(())
    }

    pub(super) async fn join(&self) -> Result<(), AppError> {
        self.tasks.join().await?;
        let pending = self.state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?
            .settlements.values().cloned().collect::<Vec<_>>();
        for settlement in pending { commit_settlement(&self.state, settlement).await?; }
        Ok(())
    }
}

async fn commit_settlement(state: &Mutex<State>, settlement: Arc<Settlement>) -> Result<(), AppError> {
    settlement.store.settle_knowledge_writeback(&settlement.owner, &settlement.session,
        &settlement.start, settlement.outcome.clone()).await.map_err(failure)?;
    state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?
        .settlements.remove(settlement.outcome.operation_id.as_ref());
    // Display delivery is retrievable from the canonical history. A lost
    // notification cannot undo a committed result or keep resources alive.
    if let Err(error) = settlement.host.publish_knowledge_writeback_summary(
        &settlement.session, &settlement.outcome.operation_id,
    ).await {
        tracing::warn!(%error, "Knowledge result committed but display delivery failed");
    }
    Ok(())
}

fn detail(value: impl std::fmt::Display) -> String {
    let mut bounded = String::new();
    for ch in nomi_redact::redact_secrets_owned(value.to_string()).chars() {
        if ch.is_control() && !matches!(ch, '\n' | '\t') { continue; }
        if bounded.len() + ch.len_utf8() > 2048 { break; }
        bounded.push(ch);
    }
    let bounded = bounded.trim();
    if bounded.is_empty() { "Knowledge write-back did not return a confirmed result".into() }
    else { bounded.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn soft_retirement_joins_the_original_writeback_without_cancelling_it() {
        let owner = KnowledgeWritebacks::new(1).unwrap();
        let token = CancellationToken::new();
        owner.state.lock().unwrap().tokens.insert("original-turn".into(), token.clone());
        let release = Arc::new(tokio::sync::Notify::new());
        let worker_release = release.clone();
        let worker_token = token.clone();
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_completed = completed.clone();
        let _result = owner.tasks.spawn(async move {
            worker_release.notified().await;
            assert!(!worker_token.is_cancelled());
            worker_completed.store(true, std::sync::atomic::Ordering::Release);
        }).unwrap();
        owner.close(false).unwrap();
        assert!(!token.is_cancelled(), "main Turn completion cannot cancel its writeback");
        assert!(owner.join().now_or_never().is_none(), "retirement must retain and join the live owner");
        release.notify_one();
        owner.join().await.unwrap();
        assert!(completed.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test]
    async fn explicit_session_teardown_still_cancels_and_joins_writebacks() {
        let owner = KnowledgeWritebacks::new(1).unwrap();
        let token = CancellationToken::new();
        owner.state.lock().unwrap().tokens.insert("original-turn".into(), token.clone());
        let worker_token = token.clone();
        let _result = owner.tasks.spawn(async move { worker_token.cancelled().await; }).unwrap();
        owner.close(true).unwrap();
        owner.join().await.unwrap();
        assert!(token.is_cancelled());
    }

    #[test]
    fn knowledge_writeback_owner_error_is_bounded_in_bytes_and_valid_for_settlement() {
        for message in ["中文错误🙂".repeat(1000), "\u{0000}\r \t\n".into()] {
            let mut diagnostic = KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Resolve);
            diagnostic.code = Some("NOMIFUN_INTERNAL_ERROR".into());
            diagnostic.detail = Some(detail(message));
            let outcome = KnowledgeWritebackSettledV1 {
                operation_id: "original-turn".into(), status: Status::Failed,
                written: vec![], detail: None, diagnostic: Some(diagnostic),
            };
            outcome.validate().unwrap();
        }
    }
}

impl EngineKernelSession {
    pub(in crate::router) fn running_knowledge_operations(&self)->Result<Vec<OperationId>,AppError> {
        self.knowledge_writebacks.running_operations()
    }
    pub(in crate::router) fn knowledge_stop_requested(&self,operation:&OperationId)->Result<bool,AppError> {
        self.knowledge_writebacks.stop_requested(operation)
    }
    pub(in crate::router) fn has_unsettled_knowledge_work(&self)->Result<bool,AppError> {
        self.knowledge_writebacks.has_unsettled_work()
    }
    pub(in crate::router) fn cancel_knowledge_writeback(&self, operation: &str) -> Result<bool, AppError> {
        self.knowledge_writebacks.cancel(operation)
    }

    pub(in crate::router) async fn cancel_and_join_knowledge_writebacks(&self) -> Result<(), AppError> {
        self.knowledge_writebacks.cancel_all()?;
        self.knowledge_writebacks.join().await
    }

    pub(in crate::router) async fn join_knowledge_writebacks(&self) -> Result<(), AppError> {
        self.knowledge_writebacks.join().await
    }

    /// A bounded owner refuses additional work without delaying the answer on
    /// extraction. Record that refusal through the same fact chain.
    pub(in crate::router) async fn reject_knowledge_writeback(
        &self, operation: OperationId, error: AppError,
        host: Arc<super::super::engine_session_host::EngineSessionHost>,
    ) -> Result<(), AppError> {
        let store = host.canonical_store()?;
        let started = store.start_knowledge_writeback(KnowledgeWritebackStartRequest {
            owner: self.principal.clone(), agent_session_id: self.session_id.clone(), operation_id: operation.clone(),
            resolved_snapshot_ref: self.compiled.snapshot_ref().clone(), binding_version: self.knowledge_writebacks.binding_version,
            execution_instance_id: host.execution_instance_id().to_owned(),
        }).await.map_err(failure)?;
        if !started.first { return Ok(()); }
        let mut diagnostic = KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Resolve);
        diagnostic.code = Some("NOMIFUN_TASK_INCOMPLETE".into());
        diagnostic.detail = Some(detail(error));
        let settlement = Arc::new(Settlement { store, owner: self.principal.clone(), session: self.session_id.clone(),
            start: started.start_event_id, outcome: KnowledgeWritebackSettledV1 {
                operation_id: operation.clone(), status: Status::Failed, written: Vec::new(), detail: None, diagnostic: Some(diagnostic),
            }, host });
        self.knowledge_writebacks.state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?
            .settlements.insert(operation.as_ref().to_owned(), settlement.clone());
        commit_settlement(&self.knowledge_writebacks.state, settlement).await
    }

    /// Register ownership before publishing the main terminal. The future may
    /// start only after its exact terminal and native release have succeeded.
    pub(in crate::router) fn prepare_knowledge_writeback(
        self: &Arc<Self>,
        operation: OperationId,
        user_text: String,
        assistant_text: String,
        model: Option<nomifun_common::ProviderWithModel>,
        configuration: Result<Option<Arc<nomifun_model_invoke::ResolvedTaskConfig>>, KnowledgeWritebackDiagnostic>,
        host: Arc<super::super::engine_session_host::EngineSessionHost>,
    ) -> Result<Option<tokio::sync::oneshot::Sender<()>>, AppError> {
        let Some(binding) = self.knowledge_binding()? else { return Ok(None); };
        if !binding.writeback || (binding.writeback_eagerness == "manual"
            && !explicitly_requests_knowledge_writeback(&user_text)) { return Ok(None); }
        let _resource_state = self.state.lock().map_err(|_| failure("resource state poisoned"))?;
        if _resource_state.release.is_some() { return Err(failure("Knowledge task admission is closed")); }
        let mut state = self.knowledge_writebacks.state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?;
        if state.tokens.contains_key(operation.as_ref()) { return Ok(None); }
        if !state.settlements.is_empty() { return Err(failure("Knowledge result settlement is unproven")); }
        // The Runtime execution token is closed on every successful answer.
        // Post-answer work needs its own exact-operation cancellation scope.
        let token = CancellationToken::new();
        let task_token = token.clone();
        let task_owner = self.clone();
        let task_operation = operation.clone();
        let (release, ready) = tokio::sync::oneshot::channel();
        let registered = self.knowledge_writebacks.tasks.spawn(async move {
            let admitted = tokio::select! {
                biased;
                _ = task_token.cancelled() => false,
                released = ready => released.is_ok(),
            };
            let result = if admitted { task_owner.run_knowledge_writeback(task_operation.clone(), binding,
                user_text, assistant_text, model, configuration, task_token, host).await } else { Ok(()) };
            if let Err(error) = result {
                tracing::warn!(%error, operation=task_operation.as_ref(), "Knowledge write-back owner could not settle");
            }
            if let Ok(mut state) = task_owner.knowledge_writebacks.state.lock() {
                state.tokens.remove(task_operation.as_ref());
            }
        })?;
        state.tokens.insert(operation.as_ref().to_owned(), token);
        // The existing EngineTaskGroup retains the join witness even when this
        // result receiver is dropped. Session cleanup is its final owner.
        drop(registered);
        Ok(Some(release))
    }

    async fn run_knowledge_writeback(
        self: &Arc<Self>, operation: OperationId, binding: nomifun_knowledge::KnowledgeBinding,
        user_text: String, assistant_text: String, model: Option<nomifun_common::ProviderWithModel>,
        configuration: Result<Option<Arc<nomifun_model_invoke::ResolvedTaskConfig>>, KnowledgeWritebackDiagnostic>,
        cancellation: CancellationToken, host: Arc<super::super::engine_session_host::EngineSessionHost>,
    ) -> Result<(), AppError> {
        if cancellation.is_cancelled() { return Ok(()); }
        let store = host.canonical_store()?;
        let started = store.start_knowledge_writeback(KnowledgeWritebackStartRequest {
            owner: self.principal.clone(), agent_session_id: self.session_id.clone(),
            operation_id: operation.clone(), resolved_snapshot_ref: self.compiled.snapshot_ref().clone(),
            binding_version: self.knowledge_writebacks.binding_version,
            execution_instance_id: host.execution_instance_id().to_owned(),
        }).await.map_err(failure)?;
        if !started.first { return Ok(()); }
        if let Err(error) = host.publish_knowledge_writeback_summary(&self.session_id, &operation).await {
            tracing::warn!(%error, "Knowledge start committed but display delivery failed");
        }
        let elapsed = std::time::Instant::now();
        let execution = async {
            let configuration = match configuration {
                Ok(configuration) => configuration,
                Err(diagnostic) => return Ok::<_, AppError>(nomifun_knowledge::TurnWritebackReport::failed(diagnostic)),
            };
            let mounts = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(failure("Knowledge write-back cancelled")),
                mounts = self.knowledge.mount_info_for_bases(&binding.kb_ids) => mounts?,
            };
            Ok::<_, AppError>(self.knowledge.finalize_turn_writeback(nomifun_knowledge::TurnWritebackRequest {
                mounts, binding, surface: nomifun_knowledge::WriteSurface::RegularChat,
                user_text, assistant_text, model, captured_configuration: configuration, cancellation: Some(cancellation.clone()),
            }).await)
        };
        // Cancellation is observed by the Knowledge owner at safe boundaries.
        // Never select/drop its mutating future: already admitted publication
        // must finish before an outcome or cleanup proof can be recorded.
        let result = std::panic::AssertUnwindSafe(execution).catch_unwind().await;
        let mut outcome = KnowledgeWritebackSettledV1 {
            operation_id: operation.clone(), status: Status::Failed, written: Vec::new(), detail: None, diagnostic: None,
        };
        match result {
            Ok(Ok(report)) => {
                outcome.status = if cancellation.is_cancelled() { Status::Cancelled } else { match report.status {
                    nomifun_knowledge::TurnWritebackStatus::Written => Status::Written,
                    nomifun_knowledge::TurnWritebackStatus::Partial => Status::Partial,
                    nomifun_knowledge::TurnWritebackStatus::NoCandidate => Status::NoCandidate,
                    nomifun_knowledge::TurnWritebackStatus::NoCompleter => Status::NoCompleter,
                    nomifun_knowledge::TurnWritebackStatus::Disabled | nomifun_knowledge::TurnWritebackStatus::Failed => Status::Failed,
                }};
                outcome.written = report.written.into_iter().map(|file| KnowledgeWritebackNoticeFile {
                    kb_id: file.kb_id.as_str().into(), rel_path: file.final_rel_path,
                }).collect();
                outcome.diagnostic = report.diagnostic;
            }
            Ok(Err(error)) => {
                outcome.status = if cancellation.is_cancelled() { Status::Cancelled } else { Status::Failed };
                let stage = if cancellation.is_cancelled() { KnowledgeWritebackStage::Cancelled } else { KnowledgeWritebackStage::Resolve };
                let mut diagnostic = KnowledgeWritebackDiagnostic::new(stage);
                diagnostic.code = Some(error.error_code().to_owned());
                diagnostic.detail = Some(detail(error));
                diagnostic.elapsed_ms = Some(elapsed.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
                outcome.diagnostic = Some(diagnostic);
            }
            Err(_) => {
                outcome.status = Status::Interrupted;
                let mut diagnostic = KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Interrupted);
                diagnostic.code = Some("NOMIFUN_INTERNAL_ERROR".into());
                diagnostic.detail = Some("Knowledge owner exited unexpectedly; write result is unconfirmed".into());
                diagnostic.elapsed_ms = Some(elapsed.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
                outcome.diagnostic = Some(diagnostic);
            }
        }
        let settlement = Arc::new(Settlement { store, owner: self.principal.clone(), session: self.session_id.clone(),
            start: started.start_event_id, outcome, host });
        self.knowledge_writebacks.state.lock().map_err(|_| failure("Knowledge task ownership poisoned"))?
            .settlements.insert(operation.as_ref().to_owned(), settlement.clone());
        commit_settlement(&self.knowledge_writebacks.state, settlement).await
    }
}
