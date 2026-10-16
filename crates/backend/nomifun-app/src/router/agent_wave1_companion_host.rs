//! Persistent Companion-memory owner for bundled Wave 1 capabilities.
//!
//! The selected `companion_memory` binding identifies one canonical Companion.
//! Every read/write is delegated to `CompanionService`, whose dedicated SQLite
//! store, ownership checks, FTS maintenance and realtime events remain the only
//! product facts. Kernel PluginState is intentionally not used here.

use std::sync::Arc;

use nomifun_agent_contracts::{StrictJsonValue, TypedResourceBinding};
use nomifun_agent_domain_wave1::{
    Wave1CompanionMemoryRecallRequest, Wave1CompanionMemoryWriteRequest,
    Wave1HostContext, Wave1HostPortError,
};
use nomifun_common::{AppError, CompanionId};
use nomifun_companion::CompanionService;
use sqlx::SqlitePool;

#[path = "agent_wave1_companion_sources.rs"]
mod sources;

use super::agent_wave1_memory_receipts::{
    CompanionMemoryWriteLedger, CompanionMemoryWriteReceiptContext, ReceiptAdmission,
    ACTION_OUTCOME_UNKNOWN,
};
use super::hosted_effect_receipts::HostedEffectReceipts;

const RECALL_PER_KIND: i64 = 20;
const RECALL_CHAR_BUDGET: usize = 48 * 1024;
const MAX_RECALL_MEMORIES: usize = 120;

#[derive(Clone)]
pub(super) struct Wave1CompanionMemoryHost {
    service: Arc<CompanionService>,
    receipts: CompanionMemoryWriteLedger,
    effect_store: nomifun_agent_session::AgentSessionStore,
    source_pool: SqlitePool,
    canonical_receipts: HostedEffectReceipts,
}

impl Wave1CompanionMemoryHost {
    pub(super) fn new(service: Arc<CompanionService>, receipt_pool: SqlitePool,
        effect_store: nomifun_agent_session::AgentSessionStore) -> Self {
        Self {
            service,
            receipts: CompanionMemoryWriteLedger::new(receipt_pool.clone()),
            effect_store,
            canonical_receipts: HostedEffectReceipts::new(receipt_pool.clone()),
            source_pool: receipt_pool,
        }
    }

    pub(super) async fn recall(
        &self,
        context: Wave1HostContext,
        mut request: Wave1CompanionMemoryRecallRequest,
    ) -> Result<StrictJsonValue, Wave1HostPortError> {
        self.resolve_action_binding(&context, "read").await?;
        request.validate().map_err(Wave1HostPortError::invalid_request)?;
        // Preserve the installed default-facts behavior. Query/history requests
        // use the shared bounded dispatch and its narrower default budgets.
        let default_facts = request.query.is_none() && request.scope.as_deref().unwrap_or("facts") == "facts";
        if default_facts {
            request.per_kind.get_or_insert(RECALL_PER_KIND as usize);
            request.char_budget.get_or_insert(RECALL_CHAR_BUDGET);
        }
        let input = serde_json::to_value(&request).map_err(|error| Wave1HostPortError::invalid_request(error.to_string()))?;
        let owner = nomifun_companion::CompanionAgentCapabilityOwner::new(
            self.service.authoritative_user_id(), self.service.clone(),
        );
        let mut result = owner.invoke_target_action(&context.principal.principal_id,
            nomifun_agent_domain_wave1::COMPANION_MEMORY_RECALL_ACTION_ID, &context.resource_bindings, StrictJsonValue(input))
            .await.map_err(|error| Wave1HostPortError::new(error.code, error.message))?;
        if default_facts {
            if let Some(memories) = result.0.get_mut("memories").and_then(serde_json::Value::as_array_mut) {
                if memories.len() > MAX_RECALL_MEMORIES {
                    memories.truncate(MAX_RECALL_MEMORIES);
                    result.0["has_more"] = serde_json::json!(true);
                    result.0["truncated"] = serde_json::json!(true);
                }
            }
        }
        Ok(result)
    }

    pub(super) async fn write(
        &self,
        context: Wave1HostContext,
        request: Wave1CompanionMemoryWriteRequest,
    ) -> Result<StrictJsonValue, Wave1HostPortError> {
        let companion_id = self.resolve_action_binding(&context, "write").await?;
        request.validate().map_err(Wave1HostPortError::invalid_request)?;
        // Optional correction fields are omitted for an ordinary write, keeping
        // its established receipt fingerprint. CAS identity and expected version
        // become part of the fingerprint whenever a correction is requested.
        let request_json = StrictJsonValue(serde_json::to_value(&request)
            .map_err(|error| Wave1HostPortError::invalid_request(error.to_string()))?);
        let admission = self
            .receipts
            .admit(receipt_context(&context, companion_id.as_str()), &request_json)
            .await?;
        let mut guard = match admission {
            ReceiptAdmission::Execute(guard) => guard,
            ReceiptAdmission::Return(result) => {
                // A durable owner receipt is read before touching a live source
                // or performing any memory mutation. Old receipts with no
                // canonical Effect are never upgraded into fabricated Effects.
                if known_owner_receipt(&result) {
                    self.canonical_receipts.settle_wave1_replay_known(&self.effect_store, &context,
                        &request_json.0, result.is_ok(), &receipt_observation(&result)).await
                        .map_err(|_| uncertain_memory_result())?;
                }
                // Older durable receipts may contain the complete lineage.
                // Reuse their outcome without mutating the stored receipt, but
                // apply the current model presentation on every success.
                return result.and_then(project_memory_receipt);
            },
        };
        let canonical = match self.canonical_receipts.begin_wave1(&self.effect_store, &context, &request_json.0).await {
            Ok(receipt) => receipt,
            Err(error) => {
                // No domain write was dispatched. A start that committed
                // before an acknowledgement failure may only be reconciled
                // from this now-durable no-mutation receipt.
                let result = Err(Wave1HostPortError::new("COMPANION_MEMORY_EFFECT_REJECTED", error.to_string()));
                self.receipts.settle(&mut guard, &result).await?;
                self.canonical_receipts.settle_wave1_replay_known(&self.effect_store, &context,
                    &request_json.0, false, &receipt_observation(&result)).await
                    .map_err(|_| uncertain_memory_result())?;
                return result;
            }
        };
        let result = match sources::resolve(&self.effect_store, &self.source_pool, &context).await {
            Err(error) => Err(error), // Proven before the memory owner was invoked.
            Ok(sources) if sources.is_empty() => Err(Wave1HostPortError::new("COMPANION_MEMORY_SOURCE_UNAVAILABLE",
                "the current memory write has no proven canonical human input source")),
            Ok(sources) => {
                let mutation = if let (Some(memory_id), Some(expected_revision)) = (request.memory_id.as_deref(), request.expected_revision) {
                    self.service.update_memory_for_agent_cas_with_sources(companion_id.as_str(), memory_id, expected_revision,
                        &request.kind, &request.content, &sources).await
                } else {
                    self.service.add_memory_for_agent_with_sources(companion_id.as_str(), &request.kind, &request.content, &request.tags,
                        context.agent_session_id.as_ref(), context.operation_id.as_ref(), &sources).await
                };
                match mutation {
                    Ok(memory) => match strict_json(memory) {
                        Ok(result) => Ok(result),
                        Err(_) => return Err(uncertain_memory_result()),
                    },
                    Err(error) if proven_memory_rejection(&error) => Err(companion_memory_error(error)),
                    // Internal/I/O/commit failure is not evidence of rollback.
                    // The armed owner guard becomes unknown and the canonical
                    // Effect remains Pending for existing reconciliation.
                    Err(_) => return Err(uncertain_memory_result()),
                }
            }
        };
        self.receipts.settle(&mut guard, &result).await?;
        self.canonical_receipts.settle_wave1_known(&self.effect_store, canonical, result.is_ok(),
            &receipt_observation(&result)).await.map_err(|_| uncertain_memory_result())?;
        result
    }

    async fn resolve_action_binding(
        &self,
        context: &Wave1HostContext,
        operation: &str,
    ) -> Result<CompanionId, Wave1HostPortError> {
        self.resolve_binding(
            &context.principal.principal_id,
            &context.resource_bindings,
            operation,
        )
        .await
    }

    async fn resolve_binding(
        &self,
        principal_id: &str,
        bindings: &[TypedResourceBinding],
        operation: &str,
    ) -> Result<CompanionId, Wave1HostPortError> {
        if principal_id != self.service.authoritative_user_id() {
            return Err(Wave1HostPortError::new(
                "RESOURCE_OWNER_MISMATCH",
                "the Agent principal does not own this Companion dataset",
            ));
        }
        let action = match operation {
            "read" => super::agent_memory_authority::ProductMemoryAction::CompanionRecall,
            "write" => super::agent_memory_authority::ProductMemoryAction::CompanionWrite,
            _ => {
                return Err(Wave1HostPortError::invalid_request(
                    "unsupported Companion memory operation",
                ));
            }
        };
        let binding = super::agent_memory_authority::authorize_memory_resource(
            principal_id,
            action,
            bindings,
        )
        .map_err(companion_memory_error)?;
        let companion_id = CompanionId::try_from(binding.resource_id.as_ref()).map_err(|_| {
            Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                "the Companion memory resource identity is invalid",
            )
        })?;
        self.service
            .get_companion(companion_id.as_str())
            .await
            .map_err(|_| {
                Wave1HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    "the selected Companion is no longer available",
                )
            })?;
        Ok(companion_id)
    }
}

fn proven_memory_rejection(error: &AppError) -> bool {
    matches!(error, AppError::BadRequest(_) | AppError::NotFound(_) | AppError::Forbidden(_)
        | AppError::Conflict(_) | AppError::RevisionConflict(_))
}

fn known_owner_receipt(result: &Result<StrictJsonValue, Wave1HostPortError>) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => matches!(error.code.as_ref(), "INVALID_PAYLOAD" | "RESOURCE_NOT_FOUND" | "RESOURCE_OWNER_MISMATCH"
            | "COMPANION_MEMORY_REVISION_CONFLICT" | "COMPANION_MEMORY_SOURCE_UNAVAILABLE" | "COMPANION_MEMORY_EFFECT_REJECTED"),
    }
}

fn receipt_observation(result: &Result<StrictJsonValue, Wave1HostPortError>) -> serde_json::Value {
    match result {
        Ok(memory) => serde_json::json!({ "memory_id": memory.0.get("memory_id"), "revision": memory.0.get("revision") }),
        Err(error) => serde_json::json!({ "code": error.code.as_ref(), "known_no_memory_mutation": true }),
    }
}

fn uncertain_memory_result() -> Wave1HostPortError {
    Wave1HostPortError::new(ACTION_OUTCOME_UNKNOWN,
        "the memory write or its final receipt could not be proven; automatic mutation replay is forbidden")
}

fn receipt_context<'a>(
    context: &'a Wave1HostContext,
    companion_id: &'a str,
) -> CompanionMemoryWriteReceiptContext<'a> {
    CompanionMemoryWriteReceiptContext {
        owner_user_id: &context.principal.principal_id,
        agent_session_id: context.agent_session_id.as_ref(),
        module_id: context.capability_id.as_ref(),
        action_id: context.action_id.as_ref(),
        idempotency_key: context.idempotency_key.as_ref(),
        target_companion_id: companion_id,
    }
}

fn strict_json(
    memory: nomifun_companion::store::CompanionMemory,
) -> Result<StrictJsonValue, Wave1HostPortError> {
    serde_json::to_value(nomifun_companion::AgentMemoryProjection::from(memory))
        .map(StrictJsonValue)
        .map_err(|error| {
            Wave1HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("Companion memory result could not be encoded: {error}"),
            )
        })
}

fn project_memory_receipt(result: StrictJsonValue) -> Result<StrictJsonValue, Wave1HostPortError> {
    let memory = serde_json::from_value::<nomifun_companion::store::CompanionMemory>(result.0)
        .map_err(|error| Wave1HostPortError::new("CAPABILITY_UNAVAILABLE",
            format!("Companion memory receipt could not be decoded: {error}")))?;
    strict_json(memory)
}

fn companion_memory_error(error: AppError) -> Wave1HostPortError {
    let (code, message) = match error {
        AppError::BadRequest(_) => ("INVALID_PAYLOAD", "Companion memory input is invalid"),
        AppError::NotFound(_) => ("RESOURCE_NOT_FOUND", "Companion memory was not found"),
        AppError::Forbidden(_) => (
            "RESOURCE_OWNER_MISMATCH",
            "Companion memory belongs to a different owner",
        ),
        AppError::Conflict(_) | AppError::RevisionConflict(_) => (
            "COMPANION_MEMORY_REVISION_CONFLICT",
            "Companion memory changed concurrently",
        ),
        _ => (
            "CAPABILITY_UNAVAILABLE",
            "Companion memory storage is unavailable",
        ),
    };
    Wave1HostPortError::new(code, message)
}

#[cfg(test)]
mod model_projection_tests {
    use super::*;

    #[test]
    fn companion_memory_write_and_legacy_replay_outputs_do_not_grow_with_lineage() {
        let mut memory = nomifun_companion::store::CompanionMemory {
            memory_id: nomifun_common::CompanionMemoryId::new().into_string(),
            kind: "preference".into(), content: "Prefer short facts.".into(), tags: vec![],
            importance: 0.5, strength: 0.7, pinned: false, source: "agent".into(),
            status: "active".into(), created_at: 1, updated_at: 2, last_reinforced_at: 1,
            companion_id: Some(nomifun_common::CompanionId::new().into_string()), revision: 3,
            source_session_id: Some(nomifun_common::ConversationId::new().into_string()),
            source_operation_id: Some("real-primary-effect-operation".into()),
            source_event_ids: vec![], source_message_refs: vec![],
        };
        let baseline = strict_json(memory.clone()).unwrap();
        for _ in 0..1024 {
            memory.source_event_ids.push(nomifun_common::generate_id());
            memory.source_message_refs.push(nomifun_companion::store::CompanionMemoryMessageSource {
                session_id: memory.source_session_id.clone().unwrap(), message_id: nomifun_common::generate_id(),
            });
        }
        let full_receipt = StrictJsonValue(serde_json::to_value(&memory).unwrap());
        assert!(serde_json::to_vec(&full_receipt.0).unwrap().len() > 64 * 1024);
        let unchanged = memory.clone();
        let fresh = strict_json(memory.clone()).unwrap();
        assert_eq!(fresh.0, baseline.0);
        assert_eq!(project_memory_receipt(full_receipt.clone()).unwrap().0, baseline.0,
            "old durable receipts receive the same model-only presentation without rewriting them");
        assert_eq!(project_memory_receipt(fresh.clone()).unwrap().0, baseline.0);
        assert!(fresh.0.get("source_event_ids").is_none());
        assert!(fresh.0.get("source_message_refs").is_none());
        assert_eq!(full_receipt.0, serde_json::to_value(&unchanged).unwrap());
        assert_eq!(memory, unchanged);
    }
}
