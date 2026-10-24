//! Engine-neutral ownership of tool dispatch and durable settlement. This is
//! not a resource factory: application assembly must supply the canonical
//! Kernel or composite owner invoker with exact Session/Snapshot/tool bindings.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::sync::atomic::Ordering;

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

#[path = "engine_owned_settlement.rs"]
pub(super) mod owned_settlement;
use owned_settlement::OwnedJournalSettlement;
#[path = "engine_tool_return.rs"]
mod tool_return;

struct ToolSettlement {
    call_id: String,
    rejection: Option<tool_return::ToolReturnRejection>,
    write: OwnedJournalSettlement<Result<EngineToolResult, EngineToolError>>,
}
type ToolSettlements = Mutex<BTreeMap<String, Arc<ToolSettlement>>>;

fn has_unconfirmed_settlements(settlements: &ToolSettlements) -> Result<bool, AppError> {
    let pending = settlements.lock().map_err(|_| failure("settlement owner lock poisoned"))?;
    for settlement in pending.values() {
        if settlement.write.is_unconfirmed()? { return Ok(true); }
    }
    Ok(false)
}

async fn commit_tool_settlement(settlements: &ToolSettlements, unobserved: &Mutex<BTreeSet<String>>,
    operation: &str, settlement: &Arc<ToolSettlement>) -> Result<(), AppError> {
    settlement.write.commit().await?;
    match unobserved.lock() {
        Ok(mut observations) => { observations.insert(settlement.call_id.clone()); }
        Err(_) => {
            settlement.write.retain_owner_failure("observation lock poisoned after the original record committed")?;
            return Err(failure("observation lock poisoned"));
        }
    }
    let mut pending = settlements.lock().map_err(|_| failure("settlement owner lock poisoned"))?;
    if pending.get(operation).is_some_and(|current| Arc::ptr_eq(current, settlement)) {
        pending.remove(operation);
    }
    Ok(())
}

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

struct Turn {
    operation: String,
    journal: EngineTurnJournal,
    operations: BTreeSet<String>,
    calls: BTreeSet<String>,
    closed: bool,
}

pub struct EngineToolHost {
    inner: Arc<dyn EngineToolInvoker>,
    tasks: EngineTaskGroup,
    execution: Arc<tokio::sync::RwLock<()>>,
    turn: Mutex<Option<Turn>>,
    settlements: Arc<ToolSettlements>,
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
                || has_unconfirmed_settlements(&self.settlements).map_err(tool_error)?
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
            let (inner, journal, settlements, unobserved) = (
                self.inner.clone(),
                turn.journal.clone(),
                self.settlements.clone(),
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
                let dispatch_receipt = journal.append(intent, None, EngineJournalWrite::Progress).await;
                // Cancellation is cooperative, never rollback proof. Retain
                // this owner task through the actual result while still
                // forwarding the turn's cancellation to Service/process
                // owners so they can stop bounded work and be reaped.
                let (result, owner_invoked) = match dispatch_receipt {
                    Ok(()) => (inner.invoke(invocation.clone(), effect_cancellation).await, true),
                    Err(error) => (Err(tool_error(format!("native invocation not started; original dispatch receipt unconfirmed: {error}"))), false),
                };
                let (payload,rejection) = tool_return::settlement_payload(&dispatch,&invocation,&result,owner_invoked);
                let settlement = Arc::new(ToolSettlement { call_id: dispatch.call_id.clone(), rejection,
                    write: OwnedJournalSettlement::new(journal, payload, result) });
                settlements.lock().map_err(|_| tool_error("settlement owner lock poisoned"))?
                    .insert(dispatch.operation_id.clone(), settlement.clone());
                commit_tool_settlement(&settlements, &unobserved, &dispatch.operation_id, &settlement)
                    .await.map_err(tool_error)?;
                if let Some(rejection) = &settlement.rejection { return Err(rejection.error()); }
                settlement.write.take_outcome().map_err(tool_error)?
            }).map_err(tool_error)?
        };
        tokio::select! {
            _ = cancellation.cancelled() => Err(EngineToolError::Cancelled),
            result = task.result() => result.map_err(tool_error)?,
        }
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
        EngineToolHost::new(inner)
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
                    details:None,
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
        let EngineToolError::CapabilityKernel { code,message, .. } = error else {
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
                    details:None,
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
        assert!(matches!(error,EngineToolError::CapabilityKernel { ref code,ref message, .. }
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

    struct CountedReturn { calls: Arc<std::sync::atomic::AtomicUsize> }

    #[async_trait]
    impl EngineToolInvoker for CountedReturn {
        async fn invoke(&self, invocation: EngineToolInvocation, _: CancellationToken)
            -> Result<EngineToolResult, EngineToolError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(EngineToolResult::text(invocation.call.call_id, "original immutable owner result", false))
        }
    }

    #[tokio::test]
    async fn failed_tool_settlement_retries_original_return_without_invoking_again() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = tools(Arc::new(CountedReturn { calls: calls.clone() }));
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(), journal).unwrap();
        sqlx::query("CREATE TRIGGER reject_original_tool_return BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_tool_settled' BEGIN SELECT RAISE(FAIL,'fixture original return write unavailable'); END")
            .execute(&pool).await.unwrap();
        assert!(owner.invoke(cancellation_invocation(), CancellationToken::new()).await.is_err());
        owner.close_turn().unwrap();
        assert!(owner.join().await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 1);
        sqlx::query("DROP TRIGGER reject_original_tool_return").execute(&pool).await.unwrap();
        owner.join().await.unwrap();
        owner.join().await.unwrap();
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let records: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_tool_settled'")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(records.len(), 1);
        let value: Value = serde_json::from_str(&records[0]).unwrap();
        let result: EngineToolResult = serde_json::from_value(value["event"]["result"].clone()).unwrap();
        assert_eq!(result, EngineToolResult::text("queued-call".into(), "original immutable owner result", false));
        owner.discard_closed_observations().unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn committed_dispatch_with_failed_projection_closes_as_not_invoked() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = tools(Arc::new(CountedReturn { calls: calls.clone() }));
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(), journal).unwrap();
        sqlx::query("CREATE TRIGGER reject_original_dispatch_projection BEFORE INSERT ON agent_events WHEN NEW.kind='tool/call-started' BEGIN SELECT RAISE(FAIL,'fixture dispatch projection unavailable'); END")
            .execute(&pool).await.unwrap();
        assert!(owner.invoke(cancellation_invocation(), CancellationToken::new()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 0);
        owner.close_turn().unwrap();
        sqlx::query("DROP TRIGGER reject_original_dispatch_projection").execute(&pool).await.unwrap();
        owner.join().await.unwrap();
        owner.join().await.unwrap();
        assert_eq!(calls.load(Ordering::Acquire), 0);
        let records: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_tool_settled'")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(records.len(), 1);
        let value: Value = serde_json::from_str(&records[0]).unwrap();
        assert_eq!(value["event"]["owner_invoked"], false);
        assert!(value["event"]["result"].is_null());
        assert!(value["event"]["error"].as_str().unwrap().contains("native invocation not started"));
        owner.discard_closed_observations().unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn rejected_dispatch_without_native_call_does_not_fabricate_a_settlement() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = tools(Arc::new(CountedReturn { calls: calls.clone() }));
        let (journal, pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(), journal).unwrap();
        sqlx::query("CREATE TRIGGER reject_original_dispatch BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_tool_dispatch' BEGIN SELECT RAISE(FAIL,'fixture dispatch never committed'); END")
            .execute(&pool).await.unwrap();
        assert!(owner.invoke(cancellation_invocation(), CancellationToken::new()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 0);
        owner.close_turn().unwrap();
        sqlx::query("DROP TRIGGER reject_original_dispatch").execute(&pool).await.unwrap();
        owner.join().await.unwrap();
        assert_eq!(calls.load(Ordering::Acquire), 0);
        let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind IN ('tool/call-started','tool/result-recorded') OR (kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event') IN ('host_tool_dispatch','host_tool_settled'))")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(records, 0, "uncommitted intent and an uninvoked native call cannot become history");
        pool.close().await;
    }

    #[tokio::test]
    async fn stopped_call_keeps_rejected_native_return_until_its_error_observation_commits() {
        struct ForeignOwner { calls: Arc<std::sync::atomic::AtomicUsize> }
        #[async_trait]
        impl EngineToolInvoker for ForeignOwner {
            async fn invoke(&self, _: EngineToolInvocation, _: CancellationToken)
                -> Result<EngineToolResult,EngineToolError> {
                self.calls.fetch_add(1,Ordering::AcqRel);
                Ok(EngineToolResult::text("foreign-native-call".into(),"raw return is not this call's result",false))
            }
        }
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = tools(Arc::new(ForeignOwner { calls: calls.clone() }));
        let (journal,pool) = super::super::engine_journal::test_fixture().await;
        let store = journal.canonical_store();
        let invocation = cancellation_invocation();
        let session = invocation.agent_session_id.clone();
        let operation = invocation.turn_operation_id.clone();
        owner.bind_turn("turn".into(),journal).unwrap();
        sqlx::query("CREATE TRIGGER reject_contract_observation BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_tool_settled' BEGIN SELECT RAISE(FAIL,'fixture rejection observation unavailable'); END")
            .execute(&pool).await.unwrap();
        assert!(owner.invoke(invocation,CancellationToken::new()).await.is_err());
        assert_eq!(owner.settlements.lock().unwrap().len(),1);
        store.cancel_exact_turn(&session,&operation,"stop-rejected-return".into(),"session_api".into()).await.unwrap();
        owner.close_turn().unwrap();
        assert!(owner.join().await.is_err(),"Stop does not manufacture a missing error-observation receipt");
        sqlx::query("DROP TRIGGER reject_contract_observation").execute(&pool).await.unwrap();
        owner.join().await.unwrap();
        owner.join().await.unwrap();
        assert!(owner.settlements.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::Acquire),1);
        assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,
            nomifun_agent_session::TurnReceiptStatus::Cancelled);
        let records: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_tool_settled'")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(records.len(),1);
        let value: Value = serde_json::from_str(&records[0]).unwrap();
        assert_eq!(value["event"]["contract_rejection"]["kind"],"owner_call_mismatch");
        assert_eq!(value["event"]["call_id"],"queued-call");
        assert!(value["event"]["result"].is_null());
        owner.discard_closed_observations().unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn oversized_canonical_return_is_rejected_without_clipping_or_pinning_cleanup() {
        struct OversizedOwner { calls: Arc<std::sync::atomic::AtomicUsize> }
        #[async_trait]
        impl EngineToolInvoker for OversizedOwner {
            async fn invoke(&self, invocation: EngineToolInvocation, _: CancellationToken)
                -> Result<EngineToolResult, EngineToolError> {
                self.calls.fetch_add(1,Ordering::AcqRel);
                Ok(EngineToolResult::text(invocation.call.call_id.clone(),
                    "x".repeat(nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES), false))
            }
        }
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let owner = tools(Arc::new(OversizedOwner { calls: calls.clone() }));
        let (journal,pool) = super::super::engine_journal::test_fixture().await;
        owner.bind_turn("turn".into(),journal).unwrap();
        assert!(matches!(owner.invoke(cancellation_invocation(),CancellationToken::new()).await,
            Err(EngineToolError::InvalidContract(message)) if message.contains("HOST_TOOL_RETURN_CONTRACT_REJECTED")));
        owner.close_turn().unwrap();
        owner.join().await.unwrap();
        assert_eq!(calls.load(Ordering::Acquire),1);
        let raw: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_tool_settled'")
            .fetch_one(&pool).await.unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["event"]["contract_rejection"]["kind"],"canonical_result_too_large");
        assert!(value["event"]["result"].is_null());
        assert!(raw.len()<4096);
        assert!(value["event"]["contract_rejection"]["original_return"]["bytes"].as_u64().unwrap()
            > nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES as u64);
        owner.discard_closed_observations().unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn large_owner_return_commits_exactly_before_live_delivery() {
        struct LargeOwner(EngineToolResult);
        #[async_trait]
        impl EngineToolInvoker for LargeOwner {
            async fn invoke(&self, _: EngineToolInvocation, _: CancellationToken)
                -> Result<EngineToolResult, EngineToolError> { Ok(self.0.clone()) }
        }
        let original = EngineToolResult::text("queued-call".into(),
            serde_json::json!({"cursor":"original-source","result":{"body":"代码😀\\\"\n".repeat(12_000)}}).to_string(), false);
        assert!(serde_json::to_vec(&original).unwrap().len() > 64 * 1024);
        let owner = tools(Arc::new(LargeOwner(original.clone())));
        let (journal,pool) = super::super::engine_journal::test_fixture().await;
        let store = journal.canonical_store();
        let invocation = cancellation_invocation();
        let session = invocation.agent_session_id.clone();
        let operation = invocation.turn_operation_id.clone();
        owner.bind_turn("turn".into(),journal).unwrap();
        assert_eq!(owner.invoke(invocation,CancellationToken::new()).await.unwrap(), original);
        let facts = store.native_recovery_facts(&session,&operation).await.unwrap();
        let recorded = facts.event_payloads.values().find_map(|payload| {
            let event = payload.get("event")?;
            (event["event"] == "host_tool_settled").then(|| event["result"].clone())
        }).expect("original owner settlement committed before delivery");
        assert_eq!(serde_json::from_value::<EngineToolResult>(recorded).unwrap(), original);
        owner.close_turn().unwrap();
        owner.join().await.unwrap();
        owner.discard_closed_observations().unwrap();
        pool.close().await;
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
    pub fn new(inner: Arc<dyn EngineToolInvoker>) -> Self {
        Self {
            inner,
            tasks: EngineTaskGroup::new(128).expect("fixed engine task bound is valid"),
            execution: Arc::new(tokio::sync::RwLock::new(())),
            turn: Mutex::new(None),
            settlements: Arc::new(Mutex::new(BTreeMap::new())),
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
        if has_unconfirmed_settlements(&self.settlements)?
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
        let joined = self.tasks.join().await;
        let pending = self.settlements.lock().map_err(|_| failure("settlement owner lock poisoned"))?
            .iter().map(|(operation, settlement)| (operation.clone(), settlement.clone())).collect::<Vec<_>>();
        let mut first_failure = None;
        for (operation, settlement) in pending {
            if let Err(error) = commit_tool_settlement(&self.settlements, &self.unobserved, &operation, &settlement).await {
                first_failure.get_or_insert(error);
            }
        }
        joined?;
        if let Some(error) = first_failure { return Err(error); }
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
            || !self.settlements.lock().map_err(|_| failure("settlement owner lock poisoned"))?.is_empty()
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
