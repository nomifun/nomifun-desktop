//! Engine-neutral ownership of tool dispatch and durable settlement. This is
//! not a resource factory: application assembly must supply the canonical
//! Kernel or composite owner invoker with exact Session/Snapshot/tool bindings.
use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use nomifun_ai_agent::engine_sdk::EngineTaskGroup;
use nomifun_common::AppError;
use nomifun_engine_core::{
    EngineToolError, EngineToolInvocation, EngineToolInvoker, EngineToolResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::engine_journal::{EngineJournalWrite, EngineTurnJournal};

/// Durable dispatch intent, NOT proof of Kernel authorization, committed
/// effects, or process exit. Arguments/credentials are deliberately absent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineToolDispatchRecord {
    pub operation_id: String,
    pub call_id: String,
    pub capability_id: String,
    pub action_id: String,
    /// Device/MCP names are not canonical actions. Preserve the exact frozen
    /// model mapping to correlate hosted receipts without storing arguments.
    #[serde(default)]
    pub model_name: String,
}

/// Trusted application policy for replay observations. The live result is
/// unchanged; projection cannot grant authority or acknowledge observation.
pub trait EngineToolObservationPolicy: Send + Sync {
    fn project(
        &self,
        invocation: &EngineToolInvocation,
        result: &EngineToolResult,
    ) -> EngineToolResult;
}
pub struct BoundedEngineToolObservation;
impl EngineToolObservationPolicy for BoundedEngineToolObservation {
    fn project(&self, _: &EngineToolInvocation, result: &EngineToolResult) -> EngineToolResult {
        bounded_engine_tool_result(result)
    }
}

struct Turn {
    operation: String,
    journal: EngineTurnJournal,
    operations: BTreeSet<String>,
    calls: BTreeSet<String>,
    closed: bool,
}

pub struct EngineToolHost {
    inner: Arc<dyn EngineToolInvoker>,
    policy: Arc<dyn EngineToolObservationPolicy>,
    tasks: EngineTaskGroup,
    execution: Arc<tokio::sync::RwLock<()>>,
    turn: Mutex<Option<Turn>>,
    settlement_failed: Arc<AtomicBool>,
    unobserved: Arc<Mutex<BTreeSet<String>>>,
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine tools: {message}"))
}
fn tool_error(message: impl std::fmt::Display) -> EngineToolError {
    EngineToolError::ToolInvocation(message.to_string())
}

#[async_trait]
impl EngineToolInvoker for EngineToolHost {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        invocation.call.validate().map_err(tool_error)?;
        nomifun_engine_core::parse_completed_arguments(&invocation.call)?;
        for id in [
            invocation.operation_id.as_ref(),
            invocation.call.call_id.as_ref(),
        ] {
            if id.trim().is_empty() || id.len() > 1024 {
                return Err(tool_error("invalid tool operation identity"));
            }
        }
        let task = {
            // Synchronous registration under the same lock as close/rebind.
            // No effect can slip into a new turn after cleanup fenced this one.
            let mut slot = self
                .turn
                .lock()
                .map_err(|_| tool_error("turn lock poisoned"))?;
            let turn = slot.as_mut().ok_or_else(|| tool_error("no tool turn"))?;
            if turn.closed
                || !turn.journal.matches_tool(&invocation)
                || turn.operation != invocation.turn_operation_id.as_ref()
                || self.settlement_failed.load(Ordering::Acquire)
            {
                return Err(tool_error("tool differs from its healthy admitted turn"));
            }
            if turn.operations.len() >= 512
                || turn.operations.contains(invocation.operation_id.as_ref())
                || turn.calls.contains(invocation.call.call_id.as_ref())
            {
                return Err(tool_error(
                    "duplicate tool dispatch or turn dispatch bound reached",
                ));
            }
            // Never forget a reserved identity on spawn/write failure: a
            // retry must use a new logical operation, not replay this effect.
            turn.operations
                .insert(invocation.operation_id.as_ref().to_owned());
            turn.calls
                .insert(invocation.call.call_id.as_ref().to_owned());
            let (inner, policy, journal, failed, unobserved) = (
                self.inner.clone(),
                self.policy.clone(),
                turn.journal.clone(),
                self.settlement_failed.clone(),
                self.unobserved.clone(),
            );
            let execution = self.execution.clone();
            let before_dispatch = cancellation.clone();
            self.tasks.spawn(async move {
                // Fair shared/exclusive gate, retained through settlement.
                // Kernel verifies classification against the exact mapping
                // before any real effect; an engine cannot self-label a write
                // as parallel-safe to bypass canonical admission.
                let _guard = if invocation.binding.parallel_safe
                    && invocation.binding.effect_class == nomifun_engine_core::EngineEffectClass::ReadOnly {
                    futures_util::future::Either::Left(execution.read_owned().await)
                } else {
                    futures_util::future::Either::Right(execution.write_owned().await)
                };
                if before_dispatch.is_cancelled() { return Err(EngineToolError::Cancelled); }
                let dispatch = EngineToolDispatchRecord {
                    operation_id: invocation.operation_id.as_ref().to_owned(), call_id: invocation.call.call_id.as_ref().to_owned(),
                    capability_id: invocation.binding.capability_id.as_ref().to_owned(), action_id: invocation.binding.action_id.as_ref().to_owned(),
                    model_name: invocation.binding.model_name.clone(),
                };
                let intent = serde_json::json!({"event":"host_tool_dispatch", "dispatch":dispatch}).to_string();
                // This insert enforces the current accepted root/epoch. It is
                // dispatch intent only; the inner Kernel owns actual admission.
                journal.append(intent, None, EngineJournalWrite::Progress).await.map_err(tool_error)?;
                let result = inner.invoke(invocation.clone(), CancellationToken::new()).await
                    .and_then(|result| { result.validate_for(&invocation.call.call_id)?; Ok(result) });
                let recorded = result.as_ref().ok().map(|result| policy.project(&invocation, result));
                let message = result.as_ref().err().map(|error| bounded_error(&error.to_string()));
                let payload = serde_json::json!({"event":"host_tool_settled", "operation_id":dispatch.operation_id,
                    "call_id":dispatch.call_id, "result":recorded, "error":message}).to_string();
                // A policy cannot grow the platform log or substitute another
                // call's observation. Failure quarantines cleanup, not the effect.
                if payload.len() > 64 * 1024 || recorded.as_ref().is_some_and(|result| result.call_id != invocation.call.call_id)
                    || journal.append(payload, None, EngineJournalWrite::Settlement).await.is_err() {
                    failed.store(true, Ordering::Release);
                    return Err(tool_error("tool settlement could not be persisted"));
                }
                unobserved.lock().map_err(|_| { failed.store(true, Ordering::Release); tool_error("observation lock poisoned") })?
                    .insert(dispatch.call_id);
                result
            }).map_err(tool_error)?
        };
        tokio::select! {
            _ = cancellation.cancelled() => Err(EngineToolError::Cancelled),
            result = task.result() => result.map_err(tool_error)?,
        }
    }
}

pub fn bounded_engine_tool_result(result: &EngineToolResult) -> EngineToolResult {
    use nomifun_chat_model_broker::ChatToolResultPart;
    let mut projected = result.clone();
    for part in &mut projected.output {
        match part {
            ChatToolResultPart::Image {
                media_type,
                data_base64,
            }
            | ChatToolResultPart::Audio {
                media_type,
                data_base64,
            } => {
                *part = ChatToolResultPart::Text {
                    text: format!(
                        "[Historical media {media_type}, {} encoded bytes; binary body omitted from replay]",
                        data_base64.len()
                    ),
                };
            }
            _ => {}
        }
    }
    if serde_json::to_vec(&projected).is_ok_and(|bytes| bytes.len() <= 32 * 1024) {
        return projected;
    }
    let text = projected.output_text();
    let mut keep = 8192usize.min(text.len() / 2);
    loop {
        let mut head = keep;
        let mut tail = text.len().saturating_sub(keep);
        while !text.is_char_boundary(head) {
            head -= 1;
        }
        while !text.is_char_boundary(tail) {
            tail += 1;
        }
        let output = format!(
            "{}\n[Durable observation truncated: original {} bytes, sha256:{:x}; do not infer omitted content]\n{}",
            &text[..head],
            text.len(),
            Sha256::digest(text.as_bytes()),
            &text[tail..]
        );
        projected = EngineToolResult::text(result.call_id.clone(), output, result.is_error);
        if keep == 0 || serde_json::to_vec(&projected).is_ok_and(|bytes| bytes.len() <= 32 * 1024) {
            return projected;
        }
        keep /= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{PrincipalRef, ResolvedSnapshotRef, StrictJsonValue};
    use nomifun_chat_model_broker::ChatToolCall;
    use nomifun_db::sqlx;
    use serde_json::Value;
    fn tools(inner: Arc<dyn EngineToolInvoker>) -> EngineToolHost {
        EngineToolHost::new(inner, Arc::new(BoundedEngineToolObservation))
    }
    struct NeverInvoke;
    #[async_trait]
    impl EngineToolInvoker for NeverInvoke {
        async fn invoke(
            &self,
            _: EngineToolInvocation,
            _: CancellationToken,
        ) -> Result<EngineToolResult, EngineToolError> {
            panic!("not used")
        }
    }

    #[tokio::test]
    async fn teardown_timeout_retains_the_same_completion_witness() {
        let owner = tools(Arc::new(NeverInvoke));
        let release = CancellationToken::new();
        let token = release.clone();
        let _task = owner
            .tasks
            .spawn(async move {
                token.cancelled().await;
            })
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        assert_eq!(owner.tasks.pending_tasks().unwrap(), 1);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        release.cancel();
        owner.join().await.unwrap();
        owner.join().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_caller_retains_effect_until_exit_and_records_its_outcome() {
        struct DelayedEffect {
            started: CancellationToken,
            release: CancellationToken,
        }
        #[async_trait]
        impl EngineToolInvoker for DelayedEffect {
            async fn invoke(
                &self,
                invocation: EngineToolInvocation,
                cancellation: CancellationToken,
            ) -> Result<EngineToolResult, EngineToolError> {
                self.started.cancel();
                self.release.cancelled().await;
                assert!(
                    !cancellation.is_cancelled(),
                    "admitted effects must not lose their completion witness"
                );
                Ok(EngineToolResult::text(
                    invocation.call.call_id,
                    "effect committed",
                    false,
                ))
            }
        }
        let started = CancellationToken::new();
        let release = CancellationToken::new();
        let owner = Arc::new(tools(Arc::new(DelayedEffect {
            started: started.clone(),
            release: release.clone(),
        })));
        let invocation = EngineToolInvocation {
            agent_session_id: "session".into(), principal: PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "a".repeat(64).into() },
            active_set_generation: 1, turn_operation_id: "turn".into(), operation_id: "tool-operation".into(), idempotency_key: "key".into(), correlation_id: "correlation".into(),
            call: ChatToolCall { call_id: "call".into(), name: "write_file".into(), arguments: StrictJsonValue(serde_json::json!({})), provider_metadata: Default::default() },
            binding: serde_json::from_value(serde_json::json!({
                "model_name":"write_file", "definition":{"name":"write_file","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64), "canonical_input_schema_ref":"fixture", "capability_contract_digest":"b".repeat(64),
                "capability_id":"fs.write", "action_id":"write", "resource_binding_ids":[], "effect_class":"managed_effect", "parallel_safe":false
            })).unwrap(),
        };
        let cancellation = CancellationToken::new();
        let (journal, journal_pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(), journal).unwrap();
        let caller_owner = owner.clone();
        let token = cancellation.clone();
        let caller = tokio::spawn(async move { caller_owner.invoke(invocation, token).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), started.cancelled())
            .await
            .unwrap();
        cancellation.cancel();
        assert!(matches!(
            caller.await.unwrap(),
            Err(EngineToolError::Cancelled)
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        release.cancel();
        owner.join().await.unwrap();
        let settled: Vec<(String,)> =
            sqlx::query_as("SELECT event_json FROM conversation_runtime_events ORDER BY sequence")
                .fetch_all(&journal_pool)
                .await
                .unwrap();
        assert_eq!(settled.len(), 2);
        let event: Value = serde_json::from_str(&settled[1].0).unwrap();
        assert_eq!(event["operation_id"], "tool-operation");
        assert_eq!(event["result"]["is_error"], false);
        assert!(event.to_string().contains("effect committed"));
    }

    #[tokio::test]
    async fn tool_panic_permanently_refuses_cleanup_proof() {
        let owner = tools(Arc::new(NeverInvoke));
        let _task = owner
            .tasks
            .spawn(async { panic!("fixture tool panic") })
            .unwrap();
        assert!(owner.join().await.is_err());
        assert!(owner.join().await.is_err());
    }
}

fn bounded_error(message: &str) -> String {
    if message.len() <= 2048 {
        return message.to_owned();
    }
    let mut end = 1024;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{} [error truncated; {} bytes, sha256:{:x}]",
        &message[..end],
        message.len(),
        Sha256::digest(message.as_bytes())
    )
}

impl EngineToolHost {
    pub fn new(
        inner: Arc<dyn EngineToolInvoker>,
        policy: Arc<dyn EngineToolObservationPolicy>,
    ) -> Self {
        Self {
            inner,
            policy,
            tasks: EngineTaskGroup::new(128).expect("fixed engine task bound is valid"),
            execution: Arc::new(tokio::sync::RwLock::new(())),
            turn: Mutex::new(None),
            settlement_failed: Arc::new(false.into()),
            unobserved: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Rebinding cannot clear uncertainty or forget an unobserved outcome.
    /// The caller must close/join the previous turn and record its cleanup.
    pub fn bind_turn(&self, operation: String, journal: EngineTurnJournal) -> Result<(), AppError> {
        let mut turn = self
            .turn
            .lock()
            .map_err(|_| failure("turn lock poisoned"))?;
        if self.settlement_failed.load(Ordering::Acquire)
            || !self.tasks.is_quiescent()?
            || self.has_unobserved()?
            || turn
                .as_ref()
                .is_some_and(|turn| !turn.closed || turn.operation == operation)
        {
            return Err(failure("previous tool turn is not safely closed"));
        }
        *turn = Some(Turn {
            operation,
            journal,
            operations: BTreeSet::new(),
            calls: BTreeSet::new(),
            closed: false,
        });
        Ok(())
    }

    /// Fence producers BEFORE asking process owners to cancel or join tasks.
    pub fn close_turn(&self) -> Result<(), AppError> {
        if let Some(turn) = self
            .turn
            .lock()
            .map_err(|_| failure("turn lock poisoned"))?
            .as_mut()
        {
            turn.closed = true;
        }
        Ok(())
    }
    pub fn close_session(&self) -> Result<(), AppError> {
        self.close_turn()?;
        self.tasks.close()
    }
    pub async fn join(&self) -> Result<(), AppError> {
        self.tasks.join().await?;
        if self.settlement_failed.load(Ordering::Acquire) {
            return Err(failure("settlement persistence is uncertain"));
        }
        Ok(())
    }
    pub fn has_unobserved(&self) -> Result<bool, AppError> {
        Ok(!self
            .unobserved
            .lock()
            .map_err(|_| failure("observation lock poisoned"))?
            .is_empty())
    }
    /// Call only after engine observation is itself durably recorded.
    pub fn mark_observed(&self, call: &str) -> Result<(), AppError> {
        self.unobserved
            .lock()
            .map_err(|_| failure("observation lock poisoned"))?
            .remove(call);
        Ok(())
    }
    /// Cleanup may discard unread results, never an in-flight effect. The
    /// process/resource owners must still supply their separate exit proof.
    pub fn discard_closed_observations(&self) -> Result<(), AppError> {
        let turn = self
            .turn
            .lock()
            .map_err(|_| failure("turn lock poisoned"))?;
        if turn.as_ref().is_some_and(|turn| !turn.closed)
            || !self.tasks.is_quiescent()?
            || self.settlement_failed.load(Ordering::Acquire)
        {
            return Err(failure("cannot discard open or uncertain outcomes"));
        }
        self.unobserved
            .lock()
            .map_err(|_| failure("observation lock poisoned"))?
            .clear();
        Ok(())
    }
}
