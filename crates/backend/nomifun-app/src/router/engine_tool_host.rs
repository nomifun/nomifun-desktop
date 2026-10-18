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
            // The Runtime may drop its waiter before this select is polled
            // again. Parent cancellation must still reach retained work;
            // dropping a waiter alone must not cancel that work.
            let effect_cancellation = cancellation.child_token();
            self.tasks.spawn(async move {
                // Fair shared/exclusive gate, retained through settlement.
                // Kernel verifies classification against the exact mapping
                // before any real effect; an engine cannot self-label a write
                // as parallel-safe to bypass canonical admission.
                let admission = async {
                    if invocation.binding.parallel_safe
                        && invocation.binding.effect_class == nomifun_engine_core::EngineEffectClass::ReadOnly {
                        futures_util::future::Either::Left(execution.read_owned().await)
                    } else {
                        futures_util::future::Either::Right(execution.write_owned().await)
                    }
                };
                // A queued call has not entered the owner. Cancellation must
                // release its task without waiting behind a long-running write.
                let _guard = tokio::select! {
                    biased;
                    _ = effect_cancellation.cancelled() => return Err(EngineToolError::Cancelled),
                    guard = admission => guard,
                };
                if effect_cancellation.is_cancelled() { return Err(EngineToolError::Cancelled); }
                let dispatch = EngineToolDispatchRecord {
                    operation_id: invocation.operation_id.as_ref().to_owned(), call_id: invocation.call.call_id.as_ref().to_owned(),
                    capability_id: invocation.binding.capability_id.as_ref().to_owned(), action_id: invocation.binding.action_id.as_ref().to_owned(),
                    model_name: invocation.binding.model_name.clone(),
                };
                let intent = serde_json::json!({"event":"host_tool_dispatch", "dispatch":dispatch}).to_string();
                // This insert enforces the current accepted root/epoch. It is
                // dispatch intent only; the inner Kernel owns actual admission.
                journal.append(intent, None, EngineJournalWrite::Progress).await.map_err(tool_error)?;
                // Cancellation is cooperative, never rollback proof. Retain
                // this owner task through the actual result while still
                // forwarding the turn's cancellation to Service/process
                // owners so they can stop bounded work and be reaped.
                let result = inner.invoke(invocation.clone(), effect_cancellation).await
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
    // Keep every operational JSON metadata value unchanged. Large process
    // stdout is a string field, not a reason to turn exit/cleanup observations
    // into an unparseable serialized-JSON excerpt. This grants no tool status;
    // domain readers still validate the exact admitted Action and envelope.
    if let [ChatToolResultPart::Text {text}] = projected.output.as_slice() {
        if let Ok(mut value)=serde_json::from_str::<serde_json::Value>(text) {
            if let Some(source)=value.pointer("/output/text").and_then(serde_json::Value::as_str).map(str::to_owned) {
                let mut keep=8192usize.min(source.len()/2);
                loop {
                    *value.pointer_mut("/output/text").expect("selected output string retained")=
                        serde_json::Value::String(durable_excerpt(&source,keep));
                    let candidate=EngineToolResult {call_id:result.call_id.clone(),is_error:result.is_error,
                        output:vec![ChatToolResultPart::Text {text:value.to_string()}]};
                    if serde_json::to_vec(&candidate).is_ok_and(|bytes|bytes.len()<=32*1024) {return candidate;}
                    // Metadata plus the mandatory omission notice cannot fit.
                    if keep==0 {break;}
                    keep/=2;
                }
            }
        }
    }
    let text = projected.output_text();
    let mut keep = 8192usize.min(text.len() / 2);
    loop {
        let output = durable_excerpt(&text,keep);
        projected = EngineToolResult::text(result.call_id.clone(), output, result.is_error);
        if keep == 0 || serde_json::to_vec(&projected).is_ok_and(|bytes| bytes.len() <= 32 * 1024) {
            return projected;
        }
        keep /= 2;
    }
}

fn durable_excerpt(text:&str,keep:usize)->String {
    let(head,tail)=nomifun_engine_core::tool_text_excerpt_ranges(text,keep*2);
    format!("{}\n[Durable observation truncated: original {} bytes, sha256:{:x}; do not infer omitted content]\n{}",
        &text[..head],text.len(),Sha256::digest(text.as_bytes()),&text[tail..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{PrincipalRef, ResolvedSnapshotRef, StrictJsonValue};
    use nomifun_chat_model_broker::ChatToolCall;
    use nomifun_db::sqlx;
    use serde_json::Value;

    struct BoundedEngineToolObservation;
    impl EngineToolObservationPolicy for BoundedEngineToolObservation {
        fn project(&self, _: &EngineToolInvocation, result: &EngineToolResult) -> EngineToolResult {
            bounded_engine_tool_result(result)
        }
    }

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

    fn cancellation_invocation() -> EngineToolInvocation {
        EngineToolInvocation {
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            principal: PrincipalRef { principal_kind: "user".into(), principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into() },
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() },
            active_set_generation: 1, turn_operation_id: "turn".into(), operation_id: "queued-operation".into(),
            idempotency_key: "queued-key".into(), correlation_id: "queued-correlation".into(),
            call: ChatToolCall { call_id: "queued-call".into(), name: "write_file".into(), arguments: StrictJsonValue(serde_json::json!({})), provider_metadata: Default::default() },
            binding: serde_json::from_value(serde_json::json!({
                "model_name":"write_file", "definition":{"name":"write_file","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64), "canonical_input_schema_ref":"fixture", "capability_contract_digest":"b".repeat(64),
                "capability_id":"workspace.files", "action_id":"workspace.files/write", "resource_binding_ids":[],
                "effect_class":"managed_effect", "parallel_safe":false
            })).unwrap(),
        }
    }

    #[tokio::test]
    async fn cancellation_releases_queued_call_without_waiting_for_business_gate() {
        let owner = Arc::new(tools(Arc::new(NeverInvoke)));
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(), journal).unwrap();
        let gate = owner.execution.clone().write_owned().await;
        let invocation = cancellation_invocation();
        let cancellation = CancellationToken::new();
        let caller_owner = owner.clone();
        let token = cancellation.clone();
        let caller = tokio::spawn(async move { caller_owner.invoke(invocation, token).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while owner.tasks.pending_tasks().unwrap() == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        cancellation.cancel();
        assert!(matches!(caller.await.unwrap(), Err(EngineToolError::Cancelled)));
        tokio::time::timeout(std::time::Duration::from_secs(1), owner.join()).await.unwrap().unwrap();
        let dispatched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind = 'runtime/progress-recorded'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(dispatched, 0, "queued cancellation must not dispatch or fabricate a settlement");
        drop(gate);
    }

    #[tokio::test]
    async fn parent_cancel_reaches_retained_effect_after_its_waiter_was_dropped() {
        struct CooperativeEffect { started: CancellationToken }
        #[async_trait]
        impl EngineToolInvoker for CooperativeEffect {
            async fn invoke(&self, invocation: EngineToolInvocation, cancellation: CancellationToken)
                -> Result<EngineToolResult, EngineToolError> {
                self.started.cancel();
                cancellation.cancelled().await;
                Ok(EngineToolResult::text(invocation.call.call_id, "original owner observed cancellation", false))
            }
        }
        let started = CancellationToken::new();
        let owner = Arc::new(tools(Arc::new(CooperativeEffect { started: started.clone() })));
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(), journal).unwrap();
        let cancellation = CancellationToken::new();
        let caller_owner = owner.clone();
        let token = cancellation.clone();
        let caller = tokio::spawn(async move { caller_owner.invoke(cancellation_invocation(), token).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), started.cancelled()).await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert_eq!(owner.tasks.pending_tasks().unwrap(), 1, "dropping a waiter must retain its original effect");
        cancellation.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), owner.join()).await.unwrap().unwrap();
        let rows: Vec<(String,)> = sqlx::query_as("SELECT inline_json FROM agent_events WHERE kind='runtime/progress-recorded' ORDER BY seq")
            .fetch_all(&pool).await.unwrap();
        let records = rows.iter().map(|(value,)| serde_json::from_str::<Value>(value).unwrap()).collect::<Vec<_>>();
        let settlements = records.iter().filter(|record| record.pointer("/event/event").and_then(Value::as_str) == Some("host_tool_settled")).collect::<Vec<_>>();
        assert_eq!(settlements.len(), 1);
        assert_eq!(settlements[0].pointer("/event/operation_id").and_then(Value::as_str), Some("queued-operation"));
        assert!(settlements[0].pointer("/event/result").is_some_and(|value| !value.is_null()));
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
                    cancellation.is_cancelled(),
                    "retained effects must receive cooperative cancellation"
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
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000002".into(), principal: PrincipalRef { principal_kind: "user".into(), principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into() },
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() },
            active_set_generation: 1, turn_operation_id: "turn".into(), operation_id: "tool-operation".into(), idempotency_key: "key".into(), correlation_id: "correlation".into(),
            call: ChatToolCall { call_id: "call".into(), name: "write_file".into(), arguments: StrictJsonValue(serde_json::json!({})), provider_metadata: Default::default() },
            binding: serde_json::from_value(serde_json::json!({
                "model_name":"write_file", "definition":{"name":"write_file","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64), "canonical_input_schema_ref":"fixture", "capability_contract_digest":"b".repeat(64),
                "capability_id":"workspace.files", "action_id":"workspace.files/write", "resource_binding_ids":[], "effect_class":"managed_effect", "parallel_safe":false
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
            sqlx::query_as("SELECT inline_json FROM agent_events WHERE kind = 'runtime/progress-recorded' ORDER BY seq")
                .fetch_all(&journal_pool)
                .await
                .unwrap();
        assert_eq!(settled.len(), 2);
        let event: Value = serde_json::from_str(&settled[1].0).unwrap();
        let event = &event["event"];
        assert_eq!(event["operation_id"], "tool-operation");
        assert_eq!(event["result"]["is_error"], false);
        assert!(event.to_string().contains("effect committed"));
    }

    #[tokio::test]
    async fn cancelled_knowledge_write_waiter_retains_publication_and_its_real_receipt() {
        use nomifun_knowledge::{
            AgentKnowledgeAuthority, AgentKnowledgeResource, AgentKnowledgeWriteRequest,
            AuthorizedKnowledgeService, KnowledgeEventEmitter, KnowledgeResourceOperation,
            KnowledgeService, WriteTargetSpec,
        };

        struct KnowledgeWrite {
            knowledge: AuthorizedKnowledgeService,
            base: nomifun_common::KnowledgeBaseId,
            started: CancellationToken,
            release: CancellationToken,
            invocations: Arc<std::sync::atomic::AtomicUsize>,
        }
        #[async_trait]
        impl EngineToolInvoker for KnowledgeWrite {
            async fn invoke(
                &self,
                invocation: EngineToolInvocation,
                _: CancellationToken,
            ) -> Result<EngineToolResult, EngineToolError> {
                self.invocations.fetch_add(1, Ordering::AcqRel);
                self.started.cancel();
                // Delay the admitted owner, not the model waiter. The real
                // Knowledge adapter does not yet receive end-to-end cancellation.
                self.release.cancelled().await;
                let written = self
                    .knowledge
                    .write(AgentKnowledgeWriteRequest {
                        target: WriteTargetSpec::Path {
                            kb_id: self.base.clone(),
                            rel_path: "README.md".into(),
                        },
                        content: "## Added fact\n\nA durable fact from this tool.".into(),
                    })
                    .await
                    .map_err(tool_error)?;
                Ok(EngineToolResult::text(
                    invocation.call.call_id,
                    serde_json::to_string(&written).map_err(tool_error)?,
                    false,
                ))
            }
        }

        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        let directory = tempfile::tempdir().unwrap();
        let repository = Arc::new(nomifun_db::SqliteKnowledgeRepository::new(pool.clone()));
        let service = Arc::new(KnowledgeService::new(
            repository.clone(),
            directory.path(),
            KnowledgeEventEmitter::new(
                Arc::new(nomifun_realtime::BroadcastEventBus::new(16)),
                Arc::from("0190f5fe-7c00-7a00-8000-000000000001"),
            ),
        ));
        service.set_entry_repository(repository);
        let base = service
            .create_base("Knowledge fixture", "", None, None)
            .await
            .unwrap();
        service
            .write_file(
                base.knowledge_base_id.as_str(),
                "README.md",
                "# Existing knowledge\n\nKeep this curated text.",
            )
            .await
            .unwrap();
        service
            .read_file(base.knowledge_base_id.as_str(), "README.md")
            .await
            .unwrap();
        // A completed create dirties the stable-entry projection immediately
        // before updating an existing document, matching the reported hang.
        service
            .write_file(
                base.knowledge_base_id.as_str(),
                "new-fact.md",
                "# New fact\n",
            )
            .await
            .unwrap();
        let resource = AgentKnowledgeResource::new(
            "knowledge-binding",
            "0190f5fe-7c00-7a00-8000-000000000001",
            base.knowledge_base_id.clone(),
            "Knowledge fixture",
            std::path::PathBuf::from(&base.root_path),
            [KnowledgeResourceOperation::Write],
        )
        .unwrap();
        let started = CancellationToken::new();
        let release = CancellationToken::new();
        let invocations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = Arc::new(tools(Arc::new(KnowledgeWrite {
            knowledge: AuthorizedKnowledgeService::new(
                service.clone(),
                AgentKnowledgeAuthority::new("0190f5fe-7c00-7a00-8000-000000000001", [resource])
                    .unwrap(),
            ),
            base: base.knowledge_base_id.clone(),
            started: started.clone(),
            release: release.clone(),
            invocations: invocations.clone(),
        })));
        owner.bind_turn("turn".into(), journal).unwrap();
        let invocation = EngineToolInvocation {
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            principal: PrincipalRef { principal_kind: "user".into(), principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into() },
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() },
            active_set_generation: 1, turn_operation_id: "turn".into(),
            operation_id: "knowledge-write-operation".into(), idempotency_key: "knowledge-write-key".into(), correlation_id: "knowledge-write-correlation".into(),
            call: ChatToolCall { call_id: "knowledge-write-call".into(), name: "knowledge_write".into(), arguments: StrictJsonValue(serde_json::json!({})), provider_metadata: Default::default() },
            binding: serde_json::from_value(serde_json::json!({
                "model_name":"knowledge_write", "definition":{"name":"knowledge_write","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64), "canonical_input_schema_ref":"fixture", "capability_contract_digest":"b".repeat(64),
                "capability_id":"knowledge", "action_id":"knowledge/write", "resource_binding_ids":[], "effect_class":"managed_effect", "parallel_safe":false
            })).unwrap(),
        };
        let cancellation = CancellationToken::new();
        let caller_owner = owner.clone();
        let token = cancellation.clone();
        let caller = tokio::spawn(async move { caller_owner.invoke(invocation, token).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), started.cancelled())
            .await
            .unwrap();
        cancellation.cancel();
        assert!(matches!(
            caller.await.unwrap(),
            Err(EngineToolError::Cancelled)
        ));
        owner.close_turn().unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        assert_eq!(owner.tasks.pending_tasks().unwrap(), 1);
        let readme_path = std::path::PathBuf::from(&base.root_path).join("README.md");
        assert_eq!(
            std::fs::read_to_string(&readme_path).unwrap(),
            "# Existing knowledge\n\nKeep this curated text."
        );

        release.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), owner.join())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner.tasks.pending_tasks().unwrap(), 0);
        let content = std::fs::read_to_string(&readme_path).unwrap();
        assert!(content.contains("Keep this curated text."));
        assert_eq!(content.matches("A durable fact from this tool.").count(), 1);
        let progress: Vec<(String,)> = sqlx::query_as(
            "SELECT inline_json FROM agent_events WHERE kind = 'runtime/progress-recorded' ORDER BY seq",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(progress.len(), 2);
        let settled: Value = serde_json::from_str(&progress[1].0).unwrap();
        let settled = &settled["event"];
        assert_eq!(settled["event"], "host_tool_settled");
        assert_eq!(settled["operation_id"], "knowledge-write-operation");
        assert_eq!(settled["call_id"], "knowledge-write-call");
        assert!(settled["error"].is_null());
        let result: EngineToolResult = serde_json::from_value(settled["result"].clone()).unwrap();
        assert!(!result.is_error);
        let written: Value = serde_json::from_str(&result.output_text()).unwrap();
        assert_eq!(written["resource_id"], base.knowledge_base_id.as_str());
        assert_eq!(written["relative_path"], "README.md");
        assert_eq!(written["created"], false);
        assert_eq!(invocations.load(Ordering::Acquire), 1);

        // A later owner write must survive repeated cleanup. The retained task
        // has actually exited, so no cancelled waiter can cause a late publish.
        service
            .write_file(
                base.knowledge_base_id.as_str(),
                "README.md",
                "# Later manual edit\n",
            )
            .await
            .unwrap();
        owner.join().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&readme_path).unwrap(),
            "# Later manual edit\n"
        );
        assert_eq!(invocations.load(Ordering::Acquire), 1);
        let results: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind = 'tool/result-recorded'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(results, 1);
    }

    #[tokio::test]
    async fn settlement_loss_persists_one_error_projection_for_cold_reads() {
        struct SettlementLoss;
        #[async_trait]
        impl EngineToolInvoker for SettlementLoss {
            async fn invoke(
                &self,
                _:EngineToolInvocation,
                _:CancellationToken,
            ) -> Result<EngineToolResult,EngineToolError> {
                Err(EngineToolError::CapabilityKernel {
                    code:"CAPABILITY_UNAVAILABLE".to_owned(),
                    message:"The workspace owner reported success, but the canonical receipt was not stored. Do not retry automatically. Re-read the affected owner state and reconcile the pending effect.".to_owned(),
                })
            }
        }
        let owner = tools(Arc::new(SettlementLoss));
        let invocation = EngineToolInvocation {
            agent_session_id:"0190f5fe-7c00-7a00-8000-000000000002".into(),
            principal:PrincipalRef { principal_kind:"user".into(),principal_id:"0190f5fe-7c00-7a00-8000-000000000001".into() },
            resolved_snapshot_ref:ResolvedSnapshotRef { snapshot_id:"snapshot".into(),snapshot_digest:"b".repeat(64).into() },
            active_set_generation:1,
            turn_operation_id:"turn".into(),
            operation_id:"settlement-operation".into(),
            idempotency_key:"settlement-key".into(),
            correlation_id:"settlement-correlation".into(),
            call:ChatToolCall { call_id:"settlement-call".into(),name:"write_file".into(),arguments:StrictJsonValue(serde_json::json!({})),provider_metadata:Default::default() },
            binding:serde_json::from_value(serde_json::json!({
                "model_name":"write_file","definition":{"name":"write_file","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64),"canonical_input_schema_ref":"fixture","capability_contract_digest":"b".repeat(64),
                "capability_id":"workspace.files","action_id":"workspace.files/write","resource_binding_ids":[],"effect_class":"managed_effect","parallel_safe":false
            })).unwrap(),
        };
        let (journal,pool) = super::super::engine_journal::test_fixture().await;
        let session_id = nomifun_agent_contracts::AgentSessionId::from(
            "0190f5fe-7c00-7a00-8000-000000000002",
        );
        owner.bind_turn("turn".into(),journal).unwrap();
        let error = owner.invoke(invocation,CancellationToken::new()).await.unwrap_err();
        let EngineToolError::CapabilityKernel { code,message } = error else {
            panic!("settlement loss changed error class")
        };
        assert_eq!(code,"CAPABILITY_UNAVAILABLE");
        assert!(message.contains("owner reported success"));
        assert!(message.contains("Do not retry"));
        owner.mark_observed("settlement-call").unwrap();
        owner.close_turn().unwrap();
        owner.join().await.unwrap();

        let projections:Vec<(String,String,String,Option<String>)> = sqlx::query_as(
            "SELECT kind,inline_json,correlation_id,causation_event_id FROM agent_events WHERE kind IN ('tool/call-started','tool/result-recorded') ORDER BY seq",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(projections.len(),2);
        assert_eq!(projections[0].0,"tool/call-started");
        assert_eq!(projections[1].0,"tool/result-recorded");
        assert_eq!(projections[0].2,projections[1].2);
        assert_eq!(projections[1].3.as_deref(),Some("tool-call:0190f5fe-7c00-7a00-8000-000000000002:settlement-operation"));
        let result_payload:Value = serde_json::from_str(&projections[1].1).unwrap();
        assert_eq!(result_payload["call_id"],"settlement-call");
        assert!(result_payload["output"].is_null());
        assert!(result_payload["error"].as_str().unwrap().contains("Do not retry"));

        for _ in 0..2 {
            let store = nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await.unwrap();
            let (history,_,_) = store.message_history_before(&session_id,None,50).await.unwrap();
            let tools = history.iter().filter(|row| row.presentation_intent=="tool").collect::<Vec<_>>();
            assert_eq!(tools.len(),1);
            assert_eq!(tools[0].projection["state"],"recorded");
            assert!(tools[0].projection["tool_summary"]["error"].as_str().unwrap().contains("Do not retry"));
        }
    }

    #[tokio::test]
    async fn settlement_loss_survives_database_reopen_and_cursor_reconnect() {
        struct SettlementLoss;
        #[async_trait]
        impl EngineToolInvoker for SettlementLoss {
            async fn invoke(
                &self,
                _:EngineToolInvocation,
                _:CancellationToken,
            ) -> Result<EngineToolResult,EngineToolError> {
                Err(EngineToolError::CapabilityKernel {
                    code:"CAPABILITY_UNAVAILABLE".to_owned(),
                    message:"The owner outcome is unknown. Do not retry automatically. Reconcile the external owner.".to_owned(),
                })
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("agent.db");
        let (journal,pool) = super::super::engine_journal::test_fixture_at(&database_path).await;
        let owner = tools(Arc::new(SettlementLoss));
        owner.bind_turn("turn".into(),journal).unwrap();
        let invocation = EngineToolInvocation {
            agent_session_id:"0190f5fe-7c00-7a00-8000-000000000002".into(),
            principal:PrincipalRef { principal_kind:"user".into(),principal_id:"0190f5fe-7c00-7a00-8000-000000000001".into() },
            resolved_snapshot_ref:ResolvedSnapshotRef { snapshot_id:"snapshot".into(),snapshot_digest:"b".repeat(64).into() },
            active_set_generation:1,
            turn_operation_id:"turn".into(),
            operation_id:"reopen-settlement-operation".into(),
            idempotency_key:"reopen-settlement-key".into(),
            correlation_id:"reopen-settlement-correlation".into(),
            call:ChatToolCall { call_id:"reopen-settlement-call".into(),name:"write_file".into(),arguments:StrictJsonValue(serde_json::json!({})),provider_metadata:Default::default() },
            binding:serde_json::from_value(serde_json::json!({
                "model_name":"write_file","definition":{"name":"write_file","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64),"canonical_input_schema_ref":"fixture","capability_contract_digest":"b".repeat(64),
                "capability_id":"workspace.files","action_id":"workspace.files/write","resource_binding_ids":[],"effect_class":"managed_effect","parallel_safe":false
            })).unwrap(),
        };
        let error = owner.invoke(invocation,CancellationToken::new()).await.unwrap_err();
        assert!(matches!(error,EngineToolError::CapabilityKernel { ref code,ref message }
            if code=="CAPABILITY_UNAVAILABLE" && message.contains("Reconcile")));
        owner.mark_observed("reopen-settlement-call").unwrap();
        owner.close_turn().unwrap();
        owner.join().await.unwrap();
        drop(owner);
        pool.close().await;

        let database = nomifun_db::init_database(&database_path).await.unwrap();
        let session_id = nomifun_agent_contracts::AgentSessionId::from(
            "0190f5fe-7c00-7a00-8000-000000000002",
        );
        let mut before = None;
        let mut history = Vec::new();
        let mut expected_total = None;
        loop {
            let store = nomifun_agent_session::AgentSessionStore::from_pool(
                database.pool().clone(),
            ).await.unwrap();
            let (page,has_more,total) = store.message_history_before(
                &session_id,before,1,
            ).await.unwrap();
            assert_eq!(*expected_total.get_or_insert(total),total);
            assert!(!page.is_empty());
            before = page.last().map(|row| row.first_seq);
            history.extend(page);
            if !has_more { break; }
        }
        let tool_rows = history.iter().filter(|row| row.presentation_intent=="tool").collect::<Vec<_>>();
        assert_eq!(tool_rows.len(),1);
        assert_eq!(tool_rows[0].projection["state"],"recorded");
        assert!(tool_rows[0].projection["tool_summary"]["error"].as_str().unwrap().contains("Reconcile"));
        let unique = history.iter().map(|row| row.projection_id.as_str()).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(),history.len());
        let projections:i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_events WHERE kind IN ('tool/call-started','tool/result-recorded')",
        ).fetch_one(database.pool()).await.unwrap();
        assert_eq!(projections,2);
        database.close().await;
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
                .is_some_and(|turn| !turn.closed || (turn.operation == operation && turn.journal.generation() == journal.generation()))
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
    pub(super) fn execution_window_near_limit(&self) -> Result<bool, AppError> {
        let turn = self.turn.lock().map_err(|_| failure("turn lock poisoned"))?;
        Ok(turn.as_ref().is_some_and(|turn| turn.operations.len() >= 448))
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
