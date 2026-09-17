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

use super::agent_wave1_memory_receipts::{
    CompanionMemoryWriteLedger, CompanionMemoryWriteReceiptContext, ReceiptAdmission,
};

const RECALL_PER_KIND: i64 = 20;
const RECALL_CHAR_BUDGET: usize = 48 * 1024;
const MAX_RECALL_MEMORIES: usize = 120;

#[derive(Clone)]
pub(super) struct Wave1CompanionMemoryHost {
    service: Arc<CompanionService>,
    receipts: CompanionMemoryWriteLedger,
}

impl Wave1CompanionMemoryHost {
    pub(super) fn new(service: Arc<CompanionService>, receipt_pool: SqlitePool) -> Self {
        Self {
            service,
            receipts: CompanionMemoryWriteLedger::new(receipt_pool),
        }
    }

    pub(super) async fn recall(
        &self,
        context: Wave1HostContext,
        request: Wave1CompanionMemoryRecallRequest,
    ) -> Result<StrictJsonValue, Wave1HostPortError> {
        let companion_id = self.resolve_action_binding(&context, "read").await?;
        let mut memories = self
            .service
            .recall_memories_for_agent(
                companion_id.as_str(),
                i64::try_from(request.per_kind.unwrap_or(RECALL_PER_KIND as usize))
                    .unwrap_or(RECALL_PER_KIND),
                request.char_budget.unwrap_or(RECALL_CHAR_BUDGET),
            )
            .await
            .map_err(companion_memory_error)?;
        memories.truncate(MAX_RECALL_MEMORIES);
        Ok(StrictJsonValue(serde_json::json!({
            "companion_id": companion_id.as_str(),
            "memories": memories,
        })))
    }

    pub(super) async fn write(
        &self,
        context: Wave1HostContext,
        request: Wave1CompanionMemoryWriteRequest,
    ) -> Result<StrictJsonValue, Wave1HostPortError> {
        let companion_id = self.resolve_action_binding(&context, "write").await?;
        let request_json = StrictJsonValue(serde_json::json!({
            "kind": &request.kind,
            "content": &request.content,
            "tags": &request.tags,
        }));
        let admission = self
            .receipts
            .admit(receipt_context(&context, companion_id.as_str()), &request_json)
            .await?;
        let mut guard = match admission {
            ReceiptAdmission::Execute(guard) => guard,
            ReceiptAdmission::Return(result) => return result,
        };
        let result = self
            .service
            .add_memory(
                &request.kind,
                &request.content,
                &request.tags,
                Some(companion_id.as_str()),
            )
            .await
            .map_err(companion_memory_error)
            .and_then(strict_json);
        self.receipts.settle(&mut guard, &result).await?;
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
    serde_json::to_value(memory)
        .map(StrictJsonValue)
        .map_err(|error| {
            Wave1HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("Companion memory result could not be encoded: {error}"),
            )
        })
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
            "CAPABILITY_UNAVAILABLE",
            "Companion memory changed concurrently",
        ),
        _ => (
            "CAPABILITY_UNAVAILABLE",
            "Companion memory storage is unavailable",
        ),
    };
    Wave1HostPortError::new(code, message)
}
