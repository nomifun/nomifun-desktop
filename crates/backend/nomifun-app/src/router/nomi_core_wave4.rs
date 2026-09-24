//! Nomi-core owners for the Wave 4 Companion and Channel capabilities.
//!
//! The adapter is deliberately target-bound. It receives only Kernel-validated
//! requests carrying the current Nomi Session's exact resource bindings, and
//! re-checks the installation owner before touching the existing product
//! services. No default Companion, Channel, chat, or legacy summon fallback is
//! permitted.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, Weak};

#[cfg(test)]
use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    AgentSessionId, CanonicalSchemaRef, CapabilityId, CorrelationId,
    DigestHex, OperationId, PrincipalRef, ResolvedSnapshotRef, ScopeKey,
    StrictJsonValue, TypedResourceBinding, digest_payload,
};
use nomifun_agent_domain_wave4::{
    CHANNEL_GROUP_POLICY, CHANNEL_PAIRING, CHANNEL_RECEIVE,
    CHANNEL_RESOURCE_KIND,
    Wave4CapabilityOperation, Wave4ContextHostPort, Wave4HostPort,
    Wave4HostPortError, Wave4HostRequest, Wave4TurnMiddlewareHostPort,
    Wave4TurnMiddlewareHostRequest,
};
#[cfg(test)]
use nomifun_agent_domain_wave4::COMPANION_RESOURCE_KIND;
use nomifun_channel::error::ChannelError;
use nomifun_channel::{
    ChannelAgentCapabilityOwner, ChannelCustomerBindingAuthority,
    ChannelSceneIngressPort,
};
use nomifun_channel::group_policy::GroupPolicyFence;
use nomifun_channel::manager::ChannelManager;
use nomifun_channel::message_service::ChannelMessageService;
use nomifun_channel::pairing::PairingService;
#[cfg(test)]
use nomifun_channel::types::UnifiedOutgoingMessage;
use nomifun_common::AppError;
use sqlx::{Row, SqlitePool};

const RESOURCE_NOT_FOUND: &str = "RESOURCE_NOT_FOUND";
const WAVE4_IDEMPOTENCY_CONFLICT: &str = "WAVE4_IDEMPOTENCY_CONFLICT";
const WAVE4_ACTION_IN_PROGRESS: &str = "WAVE4_ACTION_IN_PROGRESS";
const WAVE4_ACTION_OUTCOME_UNKNOWN: &str =
    nomifun_agent_domain_wave4::WAVE4_ACTION_OUTCOME_UNKNOWN;
const WAVE4_IDEMPOTENCY_LEDGER_FAILED: &str = "WAVE4_IDEMPOTENCY_LEDGER_FAILED";
const CHANNEL_DELIVERY_FAILED: &str = "CHANNEL_DELIVERY_FAILED";
const CHANNEL_NOT_CONNECTED: &str = "CHANNEL_NOT_CONNECTED";
const COMPANION_OPERATION_FAILED: &str = "COMPANION_OPERATION_FAILED";
const COMPANION_MODEL_NOT_CONFIGURED: &str =
    "COMPANION_MODEL_NOT_CONFIGURED";

/// The action subset that may be exposed as Nomi model Tools.
pub(crate) fn nomi_core_wave4_tool_capability_ids() -> BTreeSet<CapabilityId> {
    [
        nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID,
        nomifun_agent_domain_wave4::COMPANION_MODULE_ID,
    ]
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

pub(crate) fn nomi_core_wave4_context_capability_ids() -> BTreeSet<CapabilityId> {
    [
        nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID,
        nomifun_agent_domain_wave4::COMPANION_MODULE_ID,
        nomifun_agent_domain_wave4::CUSTOMER_SERVICE_MODULE_ID,
    ]
    .into_iter()
    .map(CapabilityId::from)
    .collect()
}

pub(crate) fn nomi_core_wave4_lifecycle_capability_ids() -> BTreeSet<CapabilityId> {
    BTreeSet::new()
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NomiCoreWave4Support {
    pub companion_context: bool,
    pub companion_actions: bool,
    pub channel_ingress: bool,
    pub channel_actions: bool,
    pub channel_pairing: bool,
    pub channel_group_policy: bool,
}

#[cfg(test)]
impl NomiCoreWave4Support {
    pub(crate) fn capability_is_ready(self, capability_id: &str) -> bool {
        match capability_id {
            nomifun_agent_domain_wave4::COMPANION_MODULE_ID => self.companion_actions,
            CHANNEL_RECEIVE => self.channel_ingress,
            nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID => self.channel_actions,
            CHANNEL_PAIRING => self.channel_pairing,
            CHANNEL_GROUP_POLICY => self.channel_group_policy,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ReplayKey {
    principal_id: String,
    agent_session_id: String,
    capability_id: String,
    idempotency_key: String,
}

#[derive(Debug)]
enum ReplayAdmission {
    Execute(Wave4ActionExecutionGuard),
    Return(Result<StrictJsonValue, Wave4HostPortError>),
}

#[derive(Clone, Debug)]
struct Wave4DurableActionLedger {
    pool: SqlitePool,
    process_lease_id: Arc<str>,
}

impl Wave4DurableActionLedger {
    fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            process_lease_id: Arc::from(nomifun_common::generate_id()),
        }
    }

    #[cfg(test)]
    fn with_lease(pool: SqlitePool, lease_id: &str) -> Self {
        Self {
            pool,
            process_lease_id: Arc::from(lease_id),
        }
    }

    async fn admit(
        &self,
        request: &Wave4HostRequest,
        input: &StrictJsonValue,
    ) -> Result<ReplayAdmission, Wave4HostPortError> {
        let request_digest = digest_payload(&serde_json::json!({
            "action_id": &request.context.action_id,
            "resource_bindings": &request.context.resource_bindings,
            "input": input,
        }))
            .map_err(|error| Wave4HostPortError::invalid_request(error.to_string()))?;
        let key = ReplayKey {
            principal_id: request.context.principal.principal_id.clone(),
            agent_session_id: request.context.agent_session_id.as_ref().to_owned(),
            capability_id: request.context.capability_id.as_ref().to_owned(),
            idempotency_key: request.context.idempotency_key.as_ref().to_owned(),
        };
        let now = nomifun_common::now_ms();
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO nomi_wave4_action_receipts(
                owner_user_id, agent_session_id, capability_id, idempotency_key,
                request_digest, state, process_lease_id, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, 'in_flight', ?, ?, ?)",
        )
        .bind(&key.principal_id)
        .bind(&key.agent_session_id)
        .bind(&key.capability_id)
        .bind(&key.idempotency_key)
        .bind(request_digest.as_ref())
        .bind(self.process_lease_id.as_ref())
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?;
        if inserted.rows_affected() == 1 {
            return Ok(ReplayAdmission::Execute(Wave4ActionExecutionGuard {
                ledger: self.clone(),
                key,
                armed: true,
            }));
        }

        let row = sqlx::query(
            "SELECT request_digest, state, process_lease_id, output_json,
                    error_code, error_message
             FROM nomi_wave4_action_receipts
             WHERE owner_user_id = ? AND agent_session_id = ?
               AND capability_id = ? AND idempotency_key = ?",
        )
        .bind(&key.principal_id)
        .bind(&key.agent_session_id)
        .bind(&key.capability_id)
        .bind(&key.idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(ledger_error)?
        .ok_or_else(|| {
            Wave4HostPortError::new(
                WAVE4_IDEMPOTENCY_LEDGER_FAILED,
                "Wave 4 receipt disappeared after idempotency conflict",
            )
        })?;
        let existing_digest: String = row.try_get("request_digest").map_err(ledger_error)?;
        if existing_digest != request_digest.as_ref() {
            return Err(Wave4HostPortError::new(
                WAVE4_IDEMPOTENCY_CONFLICT,
                "Wave 4 idempotency key was reused with different input",
            ));
        }
        let state: String = row.try_get("state").map_err(ledger_error)?;
        let result = match state.as_str() {
            "completed" => {
                let output: String = row.try_get("output_json").map_err(ledger_error)?;
                let value = serde_json::from_str(&output).map_err(|error| {
                    Wave4HostPortError::new(
                        WAVE4_IDEMPOTENCY_LEDGER_FAILED,
                        format!("persisted Wave 4 output is invalid: {error}"),
                    )
                })?;
                Ok(StrictJsonValue(value))
            }
            "failed" => {
                let code: String = row.try_get("error_code").map_err(ledger_error)?;
                let message: String = row.try_get("error_message").map_err(ledger_error)?;
                Err(Wave4HostPortError::new(code, message))
            }
            "outcome_unknown" => Err(outcome_unknown()),
            "in_flight" => {
                let lease: String = row.try_get("process_lease_id").map_err(ledger_error)?;
                if lease == self.process_lease_id.as_ref() {
                    Err(Wave4HostPortError::new(
                        WAVE4_ACTION_IN_PROGRESS,
                        "the original Wave 4 action is still in flight",
                    ))
                } else {
                    sqlx::query(
                        "UPDATE nomi_wave4_action_receipts
                         SET state = 'outcome_unknown', process_lease_id = NULL,
                             updated_at = ?
                         WHERE owner_user_id = ? AND agent_session_id = ?
                           AND capability_id = ? AND idempotency_key = ?
                           AND state = 'in_flight' AND process_lease_id = ?",
                    )
                    .bind(now)
                    .bind(&key.principal_id)
                    .bind(&key.agent_session_id)
                    .bind(&key.capability_id)
                    .bind(&key.idempotency_key)
                    .bind(lease)
                    .execute(&self.pool)
                    .await
                    .map_err(ledger_error)?;
                    Err(outcome_unknown())
                }
            }
            other => {
                return Err(Wave4HostPortError::new(
                    WAVE4_IDEMPOTENCY_LEDGER_FAILED,
                    format!("persisted Wave 4 receipt has invalid state {other}"),
                ));
            }
        };
        Ok(ReplayAdmission::Return(result))
    }

    async fn settle(
        &self,
        guard: &mut Wave4ActionExecutionGuard,
        result: &Result<StrictJsonValue, Wave4HostPortError>,
    ) -> Result<(), Wave4HostPortError> {
        let now = nomifun_common::now_ms();
        let changed = match result {
            Ok(output) => {
                let output = serde_json::to_string(&output.0).map_err(|error| {
                    Wave4HostPortError::new(
                        WAVE4_IDEMPOTENCY_LEDGER_FAILED,
                        format!("Wave 4 output could not be persisted: {error}"),
                    )
                })?;
                sqlx::query(
                    "UPDATE nomi_wave4_action_receipts
                     SET state = 'completed', process_lease_id = NULL,
                         output_json = ?, updated_at = ?
                     WHERE owner_user_id = ? AND agent_session_id = ?
                       AND capability_id = ? AND idempotency_key = ?
                       AND state = 'in_flight' AND process_lease_id = ?",
                )
                .bind(output)
                .bind(now)
                .bind(&guard.key.principal_id)
                .bind(&guard.key.agent_session_id)
                .bind(&guard.key.capability_id)
                .bind(&guard.key.idempotency_key)
                .bind(self.process_lease_id.as_ref())
                .execute(&self.pool)
                .await
                .map_err(ledger_error)?
            }
            Err(error) if error.code == WAVE4_ACTION_OUTCOME_UNKNOWN => {
                sqlx::query(
                    "UPDATE nomi_wave4_action_receipts
                     SET state = 'outcome_unknown', process_lease_id = NULL,
                         updated_at = ?
                     WHERE owner_user_id = ? AND agent_session_id = ?
                       AND capability_id = ? AND idempotency_key = ?
                       AND state = 'in_flight' AND process_lease_id = ?",
                )
                .bind(now)
                .bind(&guard.key.principal_id)
                .bind(&guard.key.agent_session_id)
                .bind(&guard.key.capability_id)
                .bind(&guard.key.idempotency_key)
                .bind(self.process_lease_id.as_ref())
                .execute(&self.pool)
                .await
                .map_err(ledger_error)?
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE nomi_wave4_action_receipts
                     SET state = 'failed', process_lease_id = NULL,
                         error_code = ?, error_message = ?, updated_at = ?
                     WHERE owner_user_id = ? AND agent_session_id = ?
                       AND capability_id = ? AND idempotency_key = ?
                       AND state = 'in_flight' AND process_lease_id = ?",
                )
                .bind(&error.code)
                .bind(&error.message)
                .bind(now)
                .bind(&guard.key.principal_id)
                .bind(&guard.key.agent_session_id)
                .bind(&guard.key.capability_id)
                .bind(&guard.key.idempotency_key)
                .bind(self.process_lease_id.as_ref())
                .execute(&self.pool)
                .await
                .map_err(ledger_error)?
            }
        };
        if changed.rows_affected() != 1 {
            return Err(Wave4HostPortError::new(
                WAVE4_ACTION_OUTCOME_UNKNOWN,
                "Wave 4 action finished after its durable receipt ownership was lost",
            ));
        }
        guard.armed = false;
        Ok(())
    }

    async fn mark_outcome_unknown(
        &self,
        key: &ReplayKey,
    ) -> Result<(), Wave4HostPortError> {
        sqlx::query(
            "UPDATE nomi_wave4_action_receipts
             SET state = 'outcome_unknown', process_lease_id = NULL, updated_at = ?
             WHERE owner_user_id = ? AND agent_session_id = ?
               AND capability_id = ? AND idempotency_key = ?
               AND state = 'in_flight' AND process_lease_id = ?",
        )
        .bind(nomifun_common::now_ms())
        .bind(&key.principal_id)
        .bind(&key.agent_session_id)
        .bind(&key.capability_id)
        .bind(&key.idempotency_key)
        .bind(self.process_lease_id.as_ref())
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?;
        Ok(())
    }

    async fn purge_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<u64, Wave4HostPortError> {
        Ok(sqlx::query(
            "DELETE FROM nomi_wave4_action_receipts
             WHERE owner_user_id = ? AND agent_session_id = ?",
        )
        .bind(owner_id)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?
        .rows_affected())
    }

    async fn reclaim_orphaned_sessions(&self) -> Result<u64, Wave4HostPortError> {
        Ok(sqlx::query(
            "DELETE FROM nomi_wave4_action_receipts
             WHERE NOT EXISTS (
                 SELECT 1 FROM agent_sessions session
                 WHERE session.agent_session_id = nomi_wave4_action_receipts.agent_session_id
                   AND session.state = 'live'
                   AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user'
                   AND json_extract(session.owner_ref_json, '$.principal_id') = nomi_wave4_action_receipts.owner_user_id
             )",
        )
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?
        .rows_affected())
    }
}

#[derive(Debug)]
struct Wave4ActionExecutionGuard {
    ledger: Wave4DurableActionLedger,
    key: ReplayKey,
    armed: bool,
}

impl Drop for Wave4ActionExecutionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let ledger = self.ledger.clone();
        let key = self.key.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = ledger.mark_outcome_unknown(&key).await {
                    tracing::error!(
                        error = %error,
                        "Wave 4 cancelled action receipt could not be marked outcome-unknown"
                    );
                }
            });
        }
    }
}

fn outcome_unknown() -> Wave4HostPortError {
    Wave4HostPortError::new(
        WAVE4_ACTION_OUTCOME_UNKNOWN,
        "the original Wave 4 action may have produced an effect; automatic replay is forbidden",
    )
}

fn ledger_error(error: impl std::fmt::Display) -> Wave4HostPortError {
    Wave4HostPortError::new(
        WAVE4_IDEMPOTENCY_LEDGER_FAILED,
        format!("Wave 4 durable idempotency ledger failed: {error}"),
    )
}

fn action_input(operation: &Wave4CapabilityOperation) -> &StrictJsonValue {
    match operation {
        Wave4CapabilityOperation::ChannelReply { input }
        | Wave4CapabilityOperation::ChannelSend { input }
        | Wave4CapabilityOperation::CompanionLearn { input }
        | Wave4CapabilityOperation::CompanionEvolve { input }
        | Wave4CapabilityOperation::CompanionMemoryRecall { input }
        | Wave4CapabilityOperation::CompanionMemoryWrite { input }
        | Wave4CapabilityOperation::CustomerServiceNotesRead { input }
        | Wave4CapabilityOperation::CustomerServiceNotesWrite { input }
        | Wave4CapabilityOperation::CustomerServiceHandoff { input }
        | Wave4CapabilityOperation::RobotVision { input }
        | Wave4CapabilityOperation::RobotDisplay { input }
        | Wave4CapabilityOperation::RobotMotion { input }
        | Wave4CapabilityOperation::RobotDevice { input } => input,
    }
}

fn exact_binding<'a>(
    bindings: &'a [TypedResourceBinding],
    resource_kind: &str,
) -> Result<&'a TypedResourceBinding, Wave4HostPortError> {
    bindings
        .iter()
        .find(|binding| binding.resource_kind.as_ref() == resource_kind)
        .ok_or_else(|| {
            Wave4HostPortError::resource_not_bound(format!(
                "target resource kind {resource_kind} is not bound"
            ))
        })
}

fn ensure_installation_owner(
    authoritative_user_id: &str,
    actual: &str,
) -> Result<(), Wave4HostPortError> {
    if actual != authoritative_user_id {
        return Err(Wave4HostPortError::resource_owner_mismatch(
            "Wave 4 request principal is not the installation owner",
        ));
    }
    Ok(())
}

fn app_error(error: AppError) -> Wave4HostPortError {
    let code = match &error {
        AppError::NotFound(_) => RESOURCE_NOT_FOUND,
        AppError::ProviderUnavailable(_) => COMPANION_MODEL_NOT_CONFIGURED,
        AppError::Conflict(_) | AppError::RevisionConflict(_) => {
            "COMPANION_OPERATION_BUSY"
        }
        _ => COMPANION_OPERATION_FAILED,
    };
    Wave4HostPortError::new(code, error.to_string())
}

fn channel_error(error: ChannelError) -> Wave4HostPortError {
    let code = match &error {
        ChannelError::PluginNotFound(_) | ChannelError::SessionNotFound(_) => {
            RESOURCE_NOT_FOUND
        }
        ChannelError::UserNotAuthorized(_) => {
            nomifun_agent_domain_wave4::WAVE4_RESOURCE_OWNER_MISMATCH
        }
        ChannelError::InvalidPluginType(_) | ChannelError::InvalidConfig(_) => {
            nomifun_agent_domain_wave4::WAVE4_INVALID_REQUEST
        }
        _ => CHANNEL_DELIVERY_FAILED,
    };
    Wave4HostPortError::new(code, error.to_string())
}

struct ChannelRuntimeOwners {
    manager: Arc<ChannelManager>,
    pairing_service: Arc<PairingService>,
    group_policy_fence: Arc<GroupPolicyFence>,
    repository: Arc<dyn nomifun_db::IChannelRepository>,
    customer_service: Arc<nomifun_customer_service::CustomerServiceService>,
}

struct CanonicalChannelSceneIngress {
    owner: Weak<NomiCoreChannelWave4Owner>,
}

#[async_trait::async_trait]
impl ChannelSceneIngressPort for CanonicalChannelSceneIngress {
    async fn bind_scene(
        &self,
        channel_plugin_id: &str,
        companion_id: &str,
        agent_session_id: &str,
    ) -> Result<(), Wave4HostPortError> {
        let owner = self.owner.upgrade().ok_or_else(|| {
            Wave4HostPortError::unavailable("Channel scene owner has shut down")
        })?;
        let ingress = owner.ingress.get().ok_or_else(|| {
            Wave4HostPortError::unavailable(
                "Nomi-core Channel ingress owner has not completed startup",
            )
        })?;
        ingress
            .bind_agent_session_ingress(
                channel_plugin_id,
                companion_id,
                agent_session_id,
            )
            .await
            .map_err(channel_error)
    }

    async fn release_scene(
        &self,
        agent_session_id: &str,
    ) -> Result<usize, Wave4HostPortError> {
        let owner = self.owner.upgrade().ok_or_else(|| {
            Wave4HostPortError::unavailable("Channel scene owner has shut down")
        })?;
        owner.unbind_session_ingress(agent_session_id).await
    }
}

struct CanonicalChannelCustomerBinding {
    service: Arc<nomifun_customer_service::CustomerServiceService>,
}

#[async_trait::async_trait]
impl ChannelCustomerBindingAuthority for CanonicalChannelCustomerBinding {
    async fn bound_customer_agent(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<String>, Wave4HostPortError> {
        self.service
            .binding_for_plugin(channel_plugin_id)
            .await
            .map_err(app_error)
    }
}

async fn load_channel_resource(
    runtime: &ChannelRuntimeOwners,
    binding: &TypedResourceBinding,
) -> Result<nomifun_db::models::ChannelPluginRow, Wave4HostPortError> {
    let plugin_id = binding.resource_id.as_ref();
    let plugin = runtime
        .repository
        .get_plugin(plugin_id)
        .await
        .map_err(|error| {
            Wave4HostPortError::new(
                CHANNEL_DELIVERY_FAILED,
                format!("channel resource lookup failed: {error}"),
            )
        })?
        .ok_or_else(|| {
            Wave4HostPortError::new(
                RESOURCE_NOT_FOUND,
                format!("bound Channel resource {plugin_id} does not exist"),
            )
        })?;
    if !plugin.enabled {
        return Err(Wave4HostPortError::new(
            CHANNEL_NOT_CONNECTED,
            format!("bound Channel resource {plugin_id} is disabled"),
        ));
    }
    match plugin.owner_domain.as_str() {
        nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION => {
            if binding.typed_parameters.contains_key("cs_agent_id") {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "companion-owned Channel binding carries customer-service ownership",
                ));
            }
            let bound_companion_id = binding
                .typed_parameters
                .get("companion_id")
                .ok_or_else(|| {
                    Wave4HostPortError::resource_not_bound(
                        "companion-owned Channel binding has no frozen companion_id",
                    )
                })?;
            if plugin.companion_id.as_deref()
                != Some(bound_companion_id.as_str())
            {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    format!(
                        "bound companion Channel resource {plugin_id} was reassigned after this Session binding was resolved"
                    ),
                ));
            }
        }
        nomifun_db::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE => {
            if binding.typed_parameters.contains_key("companion_id")
                || plugin.companion_id.is_some()
            {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "customer-service Channel binding carries Companion ownership",
                ));
            }
            let bound_cs_agent_id = binding
                .typed_parameters
                .get("cs_agent_id")
                .ok_or_else(|| {
                    Wave4HostPortError::resource_not_bound(
                        "customer-service Channel binding has no frozen cs_agent_id",
                    )
                })?;
            let current = runtime
                .customer_service
                .binding_for_plugin(plugin_id)
                .await
                .map_err(app_error)?;
            if current.as_deref() != Some(bound_cs_agent_id.as_str()) {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    format!(
                        "bound customer-service Channel resource {plugin_id} was reassigned after this Session binding was resolved"
                    ),
                ));
            }
        }
        other => {
            return Err(Wave4HostPortError::resource_owner_mismatch(format!(
                "bound Channel resource {plugin_id} has unsupported owner domain {other}"
            )));
        }
    }
    Ok(plugin)
}

/// Host-authenticated activation request for Channel EventSource, Transport,
/// and TurnMiddleware contributions. It is assembled by the Nomi resource
/// resolver from one compiled Session, never from model input.
#[derive(Clone, Debug)]
pub(crate) struct NomiCoreChannelLifecycleRequest {
    pub(crate) principal: PrincipalRef,
    pub(crate) agent_session_id: AgentSessionId,
    pub(crate) operation_id: OperationId,
    pub(crate) correlation_id: CorrelationId,
    pub(crate) resolved_snapshot_ref: ResolvedSnapshotRef,
    pub(crate) registry_generation: u64,
    pub(crate) registry_digest: DigestHex,
    pub(crate) capability_id: CapabilityId,
    pub(crate) state_scope_key: ScopeKey,
    pub(crate) resource_bindings: Vec<TypedResourceBinding>,
    pub(crate) schema_ref: Option<CanonicalSchemaRef>,
    pub(crate) turn_input: StrictJsonValue,
}

impl NomiCoreChannelLifecycleRequest {
    fn validate(&self) -> Result<(), Wave4HostPortError> {
        let fields = [
            ("principal.kind", self.principal.principal_kind.as_str()),
            ("principal.id", self.principal.principal_id.as_str()),
            ("agent_session_id", self.agent_session_id.as_ref()),
            ("operation_id", self.operation_id.as_ref()),
            ("correlation_id", self.correlation_id.as_ref()),
            (
                "snapshot_id",
                self.resolved_snapshot_ref.snapshot_id.as_ref(),
            ),
            (
                "snapshot_digest",
                self.resolved_snapshot_ref.snapshot_digest.as_ref(),
            ),
            ("registry_digest", self.registry_digest.as_ref()),
            ("state_scope_key", self.state_scope_key.as_ref()),
        ];
        if let Some((field, _)) = fields
            .iter()
            .find(|(_, value)| value.trim().is_empty())
        {
            return Err(Wave4HostPortError::invalid_request(format!(
                "{field} must be non-empty"
            )));
        }
        if self.registry_generation == 0 {
            return Err(Wave4HostPortError::invalid_request(
                "registry_generation must identify a published generation",
            ));
        }
        if !self.turn_input.0.is_object() {
            return Err(Wave4HostPortError::invalid_request(
                "Channel lifecycle turn_input must be a JSON object",
            ));
        }
        if !matches!(
            self.capability_id.as_ref(),
            CHANNEL_RECEIVE | CHANNEL_PAIRING | CHANNEL_GROUP_POLICY
        ) {
            return Err(Wave4HostPortError::action_operation_mismatch(
                "Channel lifecycle owner received a Tool or foreign capability",
            ));
        }
        let binding = exact_binding(
            &self.resource_bindings,
            CHANNEL_RESOURCE_KIND,
        )?;
        if binding.owner_id != self.principal.principal_id {
            return Err(Wave4HostPortError::resource_owner_mismatch(format!(
                "bound Channel resource belongs to {}, not {}",
                binding.owner_id, self.principal.principal_id
            )));
        }
        let required_operation = match self.capability_id.as_ref() {
            CHANNEL_RECEIVE => "receive",
            CHANNEL_PAIRING | CHANNEL_GROUP_POLICY => "manage",
            _ => unreachable!("validated lifecycle capability"),
        };
        if !binding.operations.contains(required_operation) {
            return Err(Wave4HostPortError::resource_not_bound(format!(
                "{} requires operation {required_operation} on the bound Channel",
                self.capability_id.as_ref()
            )));
        }
        if self.schema_ref.is_some() {
            return Err(Wave4HostPortError::invalid_request(
                "Channel scene lifecycle is binding-derived and cannot carry an Agent capability schema",
            ));
        }
        Ok(())
    }
}

/// Late-bound Channel owner. The object is installed in the Nomi provider
/// before the router becomes reachable; the Nomi Kernel and Catalog retain the
/// same Arc for the process lifetime.
pub(crate) struct NomiCoreChannelWave4Owner {
    authoritative_user_id: Arc<str>,
    runtime: OnceLock<ChannelRuntimeOwners>,
    ingress: OnceLock<Arc<ChannelMessageService>>,
    target: OnceLock<Arc<ChannelAgentCapabilityOwner>>,
    ledger: Arc<Wave4DurableActionLedger>,
}

impl NomiCoreChannelWave4Owner {
    fn new(
        authoritative_user_id: Arc<str>,
        _companion_service: Arc<nomifun_companion::CompanionService>,
        ledger: Arc<Wave4DurableActionLedger>,
    ) -> Self {
        Self {
            authoritative_user_id,
            runtime: OnceLock::new(),
            ingress: OnceLock::new(),
            target: OnceLock::new(),
            ledger,
        }
    }

    pub(crate) fn install(
        self: &Arc<Self>,
        manager: Arc<ChannelManager>,
        pairing_service: Arc<PairingService>,
        repository: Arc<dyn nomifun_db::IChannelRepository>,
        customer_service: Arc<nomifun_customer_service::CustomerServiceService>,
    ) -> Result<(), String> {
        let group_policy_fence = manager.group_policy_fence();
        self.runtime
            .set(ChannelRuntimeOwners {
                manager: Arc::clone(&manager),
                pairing_service,
                group_policy_fence: Arc::clone(&group_policy_fence),
                repository: Arc::clone(&repository),
                customer_service: Arc::clone(&customer_service),
            })
            .map_err(|_| "Nomi-core Wave 4 Channel owner is already installed".to_owned())?;
        let target = ChannelAgentCapabilityOwner::new(
            Arc::clone(&self.authoritative_user_id),
            manager,
            repository,
            group_policy_fence,
        )
        .with_ingress(Arc::new(CanonicalChannelSceneIngress {
            owner: Arc::downgrade(self),
        }))
        .with_customer_binding_authority(Arc::new(
            CanonicalChannelCustomerBinding {
                service: customer_service,
            },
        ));
        self.target
            .set(Arc::new(target))
            .map_err(|_| "Nomi-core target Channel owner is already installed".to_owned())
    }

    fn target(&self) -> Result<&Arc<ChannelAgentCapabilityOwner>, Wave4HostPortError> {
        self.target.get().ok_or_else(|| {
            Wave4HostPortError::unavailable(
                "Nomi-core target Channel owner has not completed startup",
            )
        })
    }

    pub(crate) fn install_ingress(
        &self,
        message_service: Arc<ChannelMessageService>,
    ) -> Result<(), String> {
        self.ingress
            .set(message_service)
            .map_err(|_| "Nomi-core Wave 4 Channel ingress is already installed".to_owned())
    }

    async fn unbind_session_ingress(
        &self,
        agent_session_id: &str,
    ) -> Result<usize, Wave4HostPortError> {
        let ingress = self.ingress.get().ok_or_else(|| {
            Wave4HostPortError::unavailable(
                "Nomi-core Channel ingress owner has not completed startup",
            )
        })?;
        Ok(ingress
            .unbind_agent_session_ingress(agent_session_id)
            .await)
    }

    #[cfg(test)]
    pub(crate) fn support(&self) -> NomiCoreWave4Support {
        let ready = self.runtime.get().is_some();
        NomiCoreWave4Support {
            companion_context: false,
            companion_actions: false,
            channel_ingress: ready && self.ingress.get().is_some(),
            channel_actions: ready,
            channel_pairing: ready,
            channel_group_policy: ready,
        }
    }

    /// Activate one non-Tool Channel contribution for an exact Nomi Session.
    ///
    /// `channel.transport/receive` proves the bound bot is live on the process message
    /// loop; `channel.transport/pairing` reads the real pairing owner; and
    /// `channel.transport/group_policy` reads under the same per-bot fence used by
    /// inbound admission and policy writes.
    pub(crate) async fn activate_lifecycle(
        &self,
        request: NomiCoreChannelLifecycleRequest,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        request.validate()?;
        ensure_installation_owner(
            self.authoritative_user_id.as_ref(),
            &request.principal.principal_id,
        )?;
        let binding = exact_binding(
            &request.resource_bindings,
            CHANNEL_RESOURCE_KIND,
        )?;
        let runtime = self.runtime.get().ok_or_else(|| {
            Wave4HostPortError::unavailable(
                "Nomi-core Channel lifecycle owner has not completed startup",
            )
        })?;
        let plugin_id = binding.resource_id.as_ref();
        // The policy snapshot must be read after any in-flight writer commits.
        let _policy = if request.capability_id.as_ref() == CHANNEL_GROUP_POLICY {
            Some(runtime.group_policy_fence.read(plugin_id).await)
        } else {
            None
        };
        let plugin = load_channel_resource(runtime, binding).await?;
        match request.capability_id.as_ref() {
            CHANNEL_RECEIVE => {
                if !runtime.manager.is_plugin_running(plugin_id) {
                    return Err(Wave4HostPortError::new(
                        CHANNEL_NOT_CONNECTED,
                        format!(
                            "bound Channel resource {plugin_id} is not connected to the live message loop"
                        ),
                    ));
                }
                if plugin.owner_domain
                    == nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION
                {
                    let companion_id = binding
                        .typed_parameters
                        .get("companion_id")
                        .expect("validated companion Channel binding");
                    self.ingress
                        .get()
                        .ok_or_else(|| {
                            Wave4HostPortError::unavailable(
                                "Nomi-core Channel ingress owner has not completed startup",
                            )
                        })?
                        .bind_agent_session_ingress(
                            plugin_id,
                            companion_id,
                            request.agent_session_id.as_ref(),
                        )
                        .await
                        .map_err(channel_error)?;
                }
                Ok(StrictJsonValue(serde_json::json!({
                    "kind": "channel_receive",
                    "channel_plugin_id": plugin_id,
                    "platform": plugin.r#type,
                    "agent_session_id": request.agent_session_id,
                    "state": "active",
                })))
            }
            CHANNEL_PAIRING => {
                let pending_count = runtime
                    .pairing_service
                    .get_pending_pairings()
                    .await
                    .map_err(channel_error)?
                    .into_iter()
                    .filter(|pairing| {
                        pairing.channel_plugin_id.as_deref() == Some(plugin_id)
                    })
                    .count();
                Ok(StrictJsonValue(serde_json::json!({
                    "kind": "channel_pairing",
                    "channel_plugin_id": plugin_id,
                    "pending_count": pending_count,
                    "state": "active",
                })))
            }
            CHANNEL_GROUP_POLICY => {
                Ok(StrictJsonValue(serde_json::json!({
                    "kind": "channel_group_policy",
                    "channel_plugin_id": plugin_id,
                    "mode": plugin.group_access_mode,
                    "state": "active",
                })))
            }
            _ => unreachable!("validated lifecycle capability"),
        }
    }

}

impl Wave4HostPort for NomiCoreChannelWave4Owner {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            request.validate()?;
            ensure_installation_owner(
                self.authoritative_user_id.as_ref(),
                &request.context.principal.principal_id,
            )?;
            let input = action_input(&request.operation);
            match self.ledger.admit(&request, input).await? {
                ReplayAdmission::Return(result) => result,
                ReplayAdmission::Execute(mut guard) => {
                    let result = self.target()?.invoke(request.clone()).await;
                    self.ledger.settle(&mut guard, &result).await?;
                    result
                }
            }
        })
    }
}

impl Wave4TurnMiddlewareHostPort for NomiCoreChannelWave4Owner {
    fn apply<'a>(
        &'a self,
        request: Wave4TurnMiddlewareHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move {
            request.validate()?;
            self.target()?.apply(request).await
        })
    }
}

#[cfg(test)]
pub(crate) fn combined_support(
    channel: &NomiCoreChannelWave4Owner,
) -> NomiCoreWave4Support {
    let mut support = channel.support();
    support.companion_context = true;
    support.companion_actions = true;
    support
}

/// One Nomi-core composition object for the two Wave 4 packages that share
/// Companion/Channel product state.
pub(crate) struct NomiCoreWave4Owners {
    pub(crate) channel: Arc<NomiCoreChannelWave4Owner>,
    action_port: Arc<dyn Wave4HostPort>,
    context_port: Arc<dyn Wave4ContextHostPort>,
    ledger: Arc<Wave4DurableActionLedger>,
}

impl NomiCoreWave4Owners {
    pub(crate) fn new(
        authoritative_user_id: Arc<str>,
        companion_service: Arc<nomifun_companion::CompanionService>,
        pool: SqlitePool,
    ) -> Self {
        let ledger = Arc::new(Wave4DurableActionLedger::new(pool));
        let companion = Arc::new(nomifun_companion::CompanionAgentCapabilityOwner::new(
            Arc::clone(&authoritative_user_id),
            Arc::clone(&companion_service),
        ));
        let channel = Arc::new(NomiCoreChannelWave4Owner::new(
            authoritative_user_id,
            Arc::clone(&companion_service),
            Arc::clone(&ledger),
        ));
        let action_port = nomifun_agent_domain_wave4::composed_host_port(
            nomifun_agent_domain_wave4::Wave4OwnerBindings::default()
                .with_companion(
                    Arc::clone(&companion) as Arc<dyn Wave4HostPort>
                )
                .with_channel(
                    Arc::clone(&channel) as Arc<dyn Wave4HostPort>
                ),
        );
        let context_port =
            nomifun_agent_domain_wave4::composed_context_host_port(
                nomifun_agent_domain_wave4::Wave4ContextOwnerBindings::default()
                    .with_companion(
                        Arc::clone(&companion)
                            as Arc<dyn Wave4ContextHostPort>,
                    ),
            );
        Self {
            channel,
            action_port,
            context_port,
            ledger,
        }
    }

    pub(crate) fn registrations(
        &self,
    ) -> Result<Vec<nomifun_agent_kernel::PluginRegistration>, String> {
        Ok(vec![
            nomifun_agent_domain_wave4::channel_registration_with_all_host_ports(
                Arc::clone(&self.action_port),
                Arc::clone(&self.context_port),
                Arc::clone(&self.channel) as Arc<dyn Wave4TurnMiddlewareHostPort>,
            )?,
            nomifun_agent_domain_wave4::companion_registration_with_host_ports(
                Arc::clone(&self.action_port),
                Arc::clone(&self.context_port),
            )?,
        ])
    }

    pub(crate) fn install_channel(
        &self,
        manager: Arc<ChannelManager>,
        pairing_service: Arc<PairingService>,
        repository: Arc<dyn nomifun_db::IChannelRepository>,
        customer_service: Arc<nomifun_customer_service::CustomerServiceService>,
    ) -> Result<(), String> {
        self.channel
            .install(manager, pairing_service, repository, customer_service)
    }

    pub(crate) fn install_channel_ingress(
        &self,
        message_service: Arc<ChannelMessageService>,
    ) -> Result<(), String> {
        self.channel.install_ingress(message_service)
    }

    pub(crate) async fn purge_session_receipts(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<u64, Wave4HostPortError> {
        self.ledger.purge_session(owner_id, session_id).await
    }

    pub(crate) async fn reclaim_orphaned_receipts(
        &self,
    ) -> Result<u64, Wave4HostPortError> {
        self.ledger.reclaim_orphaned_sessions().await
    }

    pub(crate) async fn release_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), Wave4HostPortError> {
        self.channel.unbind_session_ingress(session_id).await?;
        self.purge_session_receipts(owner_id, session_id).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl nomifun_ai_agent::NomiPlatformBuiltinLifecycleInvoker
    for NomiCoreWave4Owners
{
    async fn activate(
        &self,
        request: nomifun_ai_agent::NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<StrictJsonValue, String> {
        let capability_id = request.capability.capability.id.clone();
        if !nomi_core_wave4_lifecycle_capability_ids()
            .contains(&capability_id)
        {
            return Err(format!(
                "WAVE4_ACTION_OPERATION_MISMATCH: {} is not owned by the Channel lifecycle adapter",
                capability_id.as_ref()
            ));
        }
        self.channel
            .activate_lifecycle(NomiCoreChannelLifecycleRequest {
                principal: request.principal,
                agent_session_id: request.agent_session_id,
                operation_id: request.operation_id,
                correlation_id: request.correlation_id,
                resolved_snapshot_ref: request.resolved_snapshot_ref,
                registry_generation: request.registry_generation,
                registry_digest: request.registry_digest,
                capability_id,
                state_scope_key: request.state_scope_key,
                resource_bindings: request.resource_bindings,
                schema_ref: request.schema_ref,
                turn_input: request.turn_input,
            })
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::IdempotencyKey;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000101";
    const TEST_SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000102";
    const TEST_LEASE_A: &str = "0190f5fe-7c00-7a00-8000-000000000103";
    const TEST_LEASE_B: &str = "0190f5fe-7c00-7a00-8000-000000000104";
    const TEST_LEASE_C: &str = "0190f5fe-7c00-7a00-8000-000000000105";

    struct RecordingChannelPlugin {
        status: nomifun_channel::types::PluginStatus,
        bot_info: Option<nomifun_channel::types::BotInfo>,
        sends: Arc<AtomicUsize>,
        fail_after_send: bool,
    }

    #[async_trait::async_trait]
    impl nomifun_channel::plugin::ChannelPlugin for RecordingChannelPlugin {
        async fn initialize(
            &mut self,
            config: nomifun_channel::types::PluginConfig,
            _callbacks: nomifun_channel::plugin::PluginCallbacks,
        ) -> Result<(), ChannelError> {
            self.bot_info = Some(nomifun_channel::types::BotInfo {
                id: "test-bot".to_owned(),
                username: Some("test_bot".to_owned()),
                display_name: "Test Bot".to_owned(),
            });
            self.fail_after_send = config
                .credentials
                .token
                .as_deref()
                .is_some_and(|token| token.contains("uncertain"));
            self.status = nomifun_channel::types::PluginStatus::Ready;
            Ok(())
        }

        async fn start(&mut self) -> Result<(), ChannelError> {
            self.status = nomifun_channel::types::PluginStatus::Running;
            Ok(())
        }

        async fn stop(&mut self) -> Result<(), ChannelError> {
            self.status = nomifun_channel::types::PluginStatus::Stopped;
            Ok(())
        }

        async fn send_message(
            &self,
            _chat_id: &str,
            _message: UnifiedOutgoingMessage,
        ) -> Result<String, ChannelError> {
            let call = self.sends.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_after_send {
                return Err(ChannelError::MessageSendFailed(
                    "provider connection closed after request write".to_owned(),
                ));
            }
            Ok(format!("platform-message-{call}"))
        }

        async fn edit_message(
            &self,
            _chat_id: &str,
            _message_id: &str,
            _message: UnifiedOutgoingMessage,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        fn active_user_count(&self) -> usize {
            0
        }

        fn bot_info(&self) -> Option<&nomifun_channel::types::BotInfo> {
            self.bot_info.as_ref()
        }

        fn plugin_type(&self) -> nomifun_channel::types::PluginType {
            nomifun_channel::types::PluginType::Telegram
        }

        fn status(&self) -> nomifun_channel::types::PluginStatus {
            self.status
        }

        fn last_error(&self) -> Option<&str> {
            None
        }
    }

    struct NoopCompleter;

    #[async_trait::async_trait]
    impl nomifun_companion::learner::CompanionCompleter for NoopCompleter {
        async fn complete(
            &self,
            _provider_id: &str,
            _model: &str,
            _system: &str,
            _user: &str,
            _max_tokens: u32,
        ) -> Result<String, AppError> {
            Ok("{}".to_owned())
        }
    }

    async fn companion_service(
        data_dir: &std::path::Path,
    ) -> Arc<nomifun_companion::CompanionService> {
        nomifun_companion::CompanionService::start(
            data_dir,
            Arc::new(nomifun_realtime::BroadcastEventBus::new(16)),
            TEST_OWNER,
            Arc::new(NoopCompleter),
            Arc::new(
                nomifun_skill_library::skill_service::resolve_skill_paths(
                    data_dir, data_dir,
                ),
            ),
        )
        .await
        .expect("start test Companion service")
    }

    fn customer_service(
        database: &nomifun_db::Database,
    ) -> Arc<nomifun_customer_service::CustomerServiceService> {
        Arc::new(
            nomifun_customer_service::CustomerServiceService::new(Arc::new(
                nomifun_db::SqliteCustomerServiceRepository::new(
                    database.pool().clone(),
                ),
            )),
        )
    }

    struct StaticAgentSessionPort {
        owner_user_id: String,
        session: nomifun_api_types::ConversationResponse,
    }

    #[async_trait::async_trait]
    impl nomifun_channel::ChannelSessionPort for StaticAgentSessionPort {
        async fn is_busy(&self, _session_id: &str) -> bool {
            false
        }

        async fn turn_outcome(
            &self,
            _owner_id: &str,
            _session_id: &str,
            _idempotency_key: &str,
        ) -> Result<nomifun_channel::ChannelTurnReceiptState, AppError> {
            Ok(nomifun_channel::ChannelTurnReceiptState::Missing)
        }

        async fn cancel(
            &self,
            _owner_id: &str,
            _session_id: &str,
        ) -> Result<(), AppError> {
            Err(AppError::Internal("unused test cancellation".to_owned()))
        }

        async fn list_messages(
            &self,
            _owner_id: &str,
            _session_id: &str,
            _query: nomifun_api_types::ListMessagesQuery,
        ) -> Result<nomifun_api_types::MessageListResponse, AppError> {
            Err(AppError::Internal("unused test message list".to_owned()))
        }

        async fn send_turn(
            &self,
            _owner_id: &str,
            _session_id: &str,
            _idempotency_key: &str,
            _request: nomifun_api_types::SendMessageRequest,
        ) -> Result<nomifun_channel::ChannelTurnDelivery, AppError> {
            Err(AppError::Internal("unused test turn".to_owned()))
        }

        async fn get(
            &self,
            owner_id: &str,
            session_id: &str,
        ) -> Result<nomifun_api_types::ConversationResponse, AppError> {
            if owner_id == self.owner_user_id
                && session_id == self.session.conversation_id
            {
                Ok(self.session.clone())
            } else {
                Err(AppError::NotFound(format!(
                    "AgentSession '{session_id}' not found"
                )))
            }
        }

        async fn create_idempotent(
            &self,
            _owner_id: &str,
            _request: nomifun_api_types::CreateConversationRequest,
            _creation_key: &str,
        ) -> Result<nomifun_api_types::ConversationResponse, AppError> {
            Err(AppError::Internal("unused test session create".to_owned()))
        }
    }

    fn ingress_message_service(
        database: &nomifun_db::Database,
        repository: Arc<dyn nomifun_db::IChannelRepository>,
        channel_plugin_id: &str,
        companion_id: &str,
        agent_session_id: &str,
    ) -> Arc<ChannelMessageService> {
        let session = nomifun_api_types::ConversationResponse {
            conversation_id: agent_session_id.to_owned(),
            name: "Wave 4 AgentSession".to_owned(),
            r#type: nomifun_common::AgentType::Nomi,
            model: None,
            reasoning_effort: None,
            status: nomifun_common::ConversationStatus::Finished,
            runtime: None,
            source: None,
            pinned: false,
            pinned_at: None,
            channel_chat_id: None,
            preset_id: Some("companion.default".to_owned()),
            preset_revision: Some(1),
            agent_snapshot: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            linked_execution_id: None,
            execution_step_id: None,
            execution_attempt_id: None,
            created_at: 1,
            modified_at: 1,
            extra: serde_json::json!({
                "nomi_core_session": {
                    "version": 1,
                    "kind": "agent_session",
                    "binding": {
                        "preset_revision_ref": {
                            "preset_id": "companion.default",
                            "revision": 1,
                            "revision_digest": "test-revision"
                        },
                        "resolved_snapshot_ref": {
                            "snapshot_id": "test-snapshot",
                            "snapshot_digest": "test-snapshot-digest"
                        },
                        "typed_resource_bindings": [{
                            "binding_id": "channel",
                            "resource_kind": "channel",
                            "resource_id": channel_plugin_id,
                            "owner_id": TEST_OWNER,
                            "operations": ["receive", "reply", "send"],
                            "typed_parameters": { "companion_id": companion_id }
                        }],
                        "binding_version": 1
                    }
                }
            }),
        };
        let settings = Arc::new(
            nomifun_channel::channel_settings::ChannelSettingsService::new(
                Arc::new(
                    nomifun_db::SqliteClientPreferenceRepository::new(
                        database.pool().clone(),
                    ),
                ),
            ),
        );
        Arc::new(ChannelMessageService::new(
            Arc::new(StaticAgentSessionPort {
                owner_user_id: TEST_OWNER.to_owned(),
                session,
            }),
            settings,
            repository,
            TEST_OWNER.to_owned(),
        ))
    }

    fn companion_action_request(
        capability_id: &str,
        action_id: &str,
        companion_id: &str,
        input: serde_json::Value,
    ) -> Wave4HostRequest {
        use nomifun_agent_contracts::{
            ActionId, AgentSessionId, CorrelationId, OperationId,
            PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId,
            ResourceKind, ScopeKey,
        };

        let operation = match action_id {
            nomifun_agent_domain_wave4::COMPANION_LEARN_ACTION_ID => Wave4CapabilityOperation::CompanionLearn {
                input: StrictJsonValue(input),
            },
            nomifun_agent_domain_wave4::COMPANION_EVOLVE_ACTION_ID => Wave4CapabilityOperation::CompanionEvolve {
                input: StrictJsonValue(input),
            },
            _ => panic!("unsupported test Companion action"),
        };
        Wave4HostRequest {
            context: nomifun_agent_domain_wave4::Wave4HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: TEST_OWNER.to_owned(),
                },
                agent_session_id: AgentSessionId::from(TEST_SESSION),
                operation_id: OperationId::from("operation"),
                idempotency_key: IdempotencyKey::from("same-key"),
                correlation_id: CorrelationId::from("correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(capability_id),
                action_id: ActionId::from(action_id),
                state_scope_key: ScopeKey::from("session:session"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("companion-memory"),
                    resource_kind: ResourceKind::from(COMPANION_RESOURCE_KIND),
                    resource_id: ResourceId::from(companion_id.to_owned()),
                    owner_id: TEST_OWNER.to_owned(),
                    operations: BTreeSet::from(["write".to_owned()]),
                    connection_config_ref: None,
                    typed_parameters: BTreeMap::new(),
                }],
            },
            operation,
        }
    }

    fn channel_lifecycle_request(
        capability_id: &str,
        channel_plugin_id: &str,
        companion_id: &str,
    ) -> NomiCoreChannelLifecycleRequest {
        NomiCoreChannelLifecycleRequest {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: TEST_OWNER.to_owned(),
            },
            agent_session_id: AgentSessionId::from(TEST_SESSION),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "digest".into(),
            },
            operation_id: OperationId::from("lifecycle-operation"),
            correlation_id: CorrelationId::from("lifecycle-correlation"),
            registry_generation: 1,
            registry_digest: DigestHex::from("registry-digest"),
            capability_id: CapabilityId::from(capability_id),
            state_scope_key: ScopeKey::from("session:session"),
            resource_bindings: vec![TypedResourceBinding {
                binding_id: nomifun_agent_contracts::ResourceBindingId::from(
                    "channel",
                ),
                resource_kind: nomifun_agent_contracts::ResourceKind::from(
                    CHANNEL_RESOURCE_KIND,
                ),
                resource_id: nomifun_agent_contracts::ResourceId::from(
                    channel_plugin_id.to_owned(),
                ),
                owner_id: TEST_OWNER.to_owned(),
                operations: BTreeSet::from([
                    "manage".to_owned(),
                    "receive".to_owned(),
                    "reply".to_owned(),
                    "send".to_owned(),
                ]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::from([(
                    "companion_id".to_owned(),
                    companion_id.to_owned(),
                )]),
            }],
            schema_ref: None,
            turn_input: StrictJsonValue(serde_json::json!({})),
        }
    }

    fn request(input: serde_json::Value) -> Wave4HostRequest {
        use nomifun_agent_contracts::{
            ActionId, AgentSessionId, CorrelationId, OperationId,
            PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId,
            ResourceKind, ScopeKey,
        };

        Wave4HostRequest {
            context: nomifun_agent_domain_wave4::Wave4HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: TEST_OWNER.to_owned(),
                },
                agent_session_id: AgentSessionId::from(TEST_SESSION),
                operation_id: OperationId::from("operation"),
                idempotency_key: IdempotencyKey::from("same-key"),
                correlation_id: CorrelationId::from("correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(
                    nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID,
                ),
                action_id: ActionId::from(
                    nomifun_agent_domain_wave4::CHANNEL_MESSAGING_SEND_ACTION_ID,
                ),
                state_scope_key: ScopeKey::from("session:session"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("channel"),
                    resource_kind: ResourceKind::from(CHANNEL_RESOURCE_KIND),
                    resource_id: ResourceId::from("channel-resource"),
                    owner_id: TEST_OWNER.to_owned(),
                    operations: BTreeSet::from(["send".to_owned()]),
                    connection_config_ref: None,
                    typed_parameters: BTreeMap::new(),
                }],
            },
            operation: Wave4CapabilityOperation::ChannelSend {
                input: StrictJsonValue(input),
            },
        }
    }

    #[tokio::test]
    async fn durable_replay_survives_restart_and_rejects_changed_input() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let first_ledger = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_A,
        );
        let first_request = request(serde_json::json!({
            "destination_ref": "chat",
            "text": "hello"
        }));
        let input = action_input(&first_request.operation);
        let ReplayAdmission::Execute(mut guard) = first_ledger
            .admit(&first_request, input)
            .await
            .unwrap()
        else {
            panic!("first request must execute");
        };
        let output = StrictJsonValue(serde_json::json!({"message_ref":"m1"}));
        first_ledger
            .settle(&mut guard, &Ok(output.clone()))
            .await
            .unwrap();

        let restarted = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_B,
        );
        let replay = restarted.admit(&first_request, input).await.unwrap();
        let ReplayAdmission::Return(Ok(actual)) = replay else {
            panic!("same input/key must return the original result");
        };
        assert_eq!(actual, output);

        let changed = request(serde_json::json!({
            "destination_ref": "chat",
            "text": "different"
        }));
        assert_eq!(
            restarted
                .admit(&changed, action_input(&changed.operation))
                .await
                .unwrap_err()
                .code,
            WAVE4_IDEMPOTENCY_CONFLICT
        );
        let mut changed_target = first_request.clone();
        changed_target.context.resource_bindings[0].resource_id =
            nomifun_agent_contracts::ResourceId::from("another-channel");
        assert_eq!(
            restarted
                .admit(
                    &changed_target,
                    action_input(&changed_target.operation),
                )
                .await
                .unwrap_err()
                .code,
            WAVE4_IDEMPOTENCY_CONFLICT
        );
        database.close().await;
    }

    #[tokio::test]
    async fn crashed_process_receipt_becomes_durable_outcome_unknown() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let crashed = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_A,
        );
        let request = request(serde_json::json!({
            "destination_ref": "chat",
            "text": "hello"
        }));
        let ReplayAdmission::Execute(guard) = crashed
            .admit(&request, action_input(&request.operation))
            .await
            .unwrap()
        else {
            panic!("first request must execute");
        };
        std::mem::forget(guard);

        let restarted = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_B,
        );
        let replay = restarted
            .admit(&request, action_input(&request.operation))
            .await
            .unwrap();
        let ReplayAdmission::Return(Err(error)) = replay else {
            panic!("crash replay must never execute again");
        };
        assert_eq!(error.code, WAVE4_ACTION_OUTCOME_UNKNOWN);
        let second = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_C,
        )
        .admit(&request, action_input(&request.operation))
        .await
        .unwrap();
        assert!(matches!(
            second,
            ReplayAdmission::Return(Err(ref error))
                if error.code == WAVE4_ACTION_OUTCOME_UNKNOWN
        ));
        database.close().await;
    }

    #[tokio::test]
    async fn cancelled_action_is_marked_unknown_and_never_reexecuted() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let ledger = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_A,
        );
        let request = request(serde_json::json!({
            "destination_ref": "chat",
            "text": "hello"
        }));
        let ReplayAdmission::Execute(guard) = ledger
            .admit(&request, action_input(&request.operation))
            .await
            .unwrap()
        else {
            panic!("first request must execute");
        };
        drop(guard);
        let mut state = String::new();
        for _ in 0..100 {
            state = sqlx::query_scalar(
                "SELECT state FROM nomi_wave4_action_receipts
                 WHERE owner_user_id = ? AND agent_session_id = ?
                   AND capability_id = 'channel.messaging' AND idempotency_key = 'same-key'",
            )
            .bind(TEST_OWNER)
            .bind(TEST_SESSION)
            .fetch_one(database.pool())
            .await
            .unwrap();
            if state == "outcome_unknown" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(state, "outcome_unknown");
        let replay = ledger
            .admit(&request, action_input(&request.operation))
            .await
            .unwrap();
        assert!(matches!(
            replay,
            ReplayAdmission::Return(Err(ref error))
                if error.code == WAVE4_ACTION_OUTCOME_UNKNOWN
        ));
        database.close().await;
    }

    #[tokio::test]
    async fn ledger_has_no_global_16384_entry_lockout_and_purges_by_session() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        sqlx::query(
            "WITH RECURSIVE seq(value) AS (
                 SELECT 0 UNION ALL SELECT value + 1 FROM seq WHERE value < 17000
             )
             INSERT INTO nomi_wave4_action_receipts(
                 owner_user_id, agent_session_id, capability_id, idempotency_key,
                 request_digest, state, output_json, created_at, updated_at
             )
             SELECT ?, ?, 'channel.messaging',
                    printf('bulk-%d', value), 'bulk', 'completed', '{}', 1, 1
             FROM seq",
        )
        .bind(TEST_OWNER)
        .bind(TEST_SESSION)
        .execute(database.pool())
        .await
        .unwrap();
        let ledger = Wave4DurableActionLedger::with_lease(
            database.pool().clone(),
            TEST_LEASE_A,
        );
        let request = request(serde_json::json!({
            "destination_ref": "chat",
            "text": "hello"
        }));
        let ReplayAdmission::Execute(mut guard) = ledger
            .admit(&request, action_input(&request.operation))
            .await
            .unwrap()
        else {
            panic!("a live scope must not lock out at 16384 prior receipts");
        };
        ledger
            .settle(
                &mut guard,
                &Ok(StrictJsonValue(serde_json::json!({"ok":true}))),
            )
            .await
            .unwrap();
        let removed = ledger.purge_session(TEST_OWNER, TEST_SESSION).await.unwrap();
        assert!(removed > 16_384);
        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM nomi_wave4_action_receipts",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(remaining, 0);
        database.close().await;
    }

    #[tokio::test]
    async fn support_plan_requires_the_real_channel_runtime() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let companion_dir = tempfile::tempdir().unwrap();
        let channel = NomiCoreChannelWave4Owner::new(
            Arc::from(TEST_OWNER),
            companion_service(companion_dir.path()).await,
            Arc::new(Wave4DurableActionLedger::new(
                database.pool().clone(),
            )),
        );
        let support = combined_support(&channel);
        assert!(support.companion_context);
        assert!(support.companion_actions);
        for capability in [
            CHANNEL_RECEIVE,
            nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID,
            CHANNEL_PAIRING,
            CHANNEL_GROUP_POLICY,
        ] {
            assert!(!support.capability_is_ready(capability));
        }
        database.close().await;
    }

    #[tokio::test]
    async fn channel_lifecycle_keeps_unbound_resource_distinct_from_startup() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let companion_dir = tempfile::tempdir().unwrap();
        let channel = NomiCoreChannelWave4Owner::new(
            Arc::from(TEST_OWNER),
            companion_service(companion_dir.path()).await,
            Arc::new(Wave4DurableActionLedger::new(
                database.pool().clone(),
            )),
        );
        let request = NomiCoreChannelLifecycleRequest {
            principal: nomifun_agent_contracts::PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: TEST_OWNER.to_owned(),
            },
            agent_session_id: nomifun_agent_contracts::AgentSessionId::from(
                TEST_SESSION,
            ),
            operation_id: OperationId::from("lifecycle-operation"),
            correlation_id: CorrelationId::from("lifecycle-correlation"),
            resolved_snapshot_ref:
                nomifun_agent_contracts::ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
            },
            registry_generation: 1,
            registry_digest: DigestHex::from("registry-digest"),
            capability_id: CapabilityId::from(CHANNEL_RECEIVE),
            state_scope_key: ScopeKey::from("session:session"),
            resource_bindings: Vec::new(),
            schema_ref: None,
            turn_input: StrictJsonValue(serde_json::json!({})),
        };
        let error = channel.activate_lifecycle(request).await.unwrap_err();
        assert_eq!(
            error.code,
            nomifun_agent_domain_wave4::WAVE4_RESOURCE_NOT_BOUND
        );
        assert_ne!(
            error.code,
            nomifun_agent_domain_wave4::WAVE4_HOST_PORT_UNAVAILABLE
        );
        database.close().await;
    }

    #[tokio::test]
    async fn companion_context_reads_only_the_explicit_live_binding() {
        let dir = tempfile::tempdir().unwrap();
        let service = companion_service(dir.path()).await;
        let selected = service
            .create_companion("Selected", "ink")
            .await
            .unwrap();
        let other = service
            .create_companion("Other", "bolt")
            .await
            .unwrap();
        let owner = nomifun_companion::CompanionAgentCapabilityOwner::new(
            Arc::from(TEST_OWNER),
            Arc::clone(&service),
        );
        let bindings = owner
            .resolve_scene_bindings(TEST_OWNER, &selected.companion_id, &BTreeSet::new())
            .await
            .unwrap();
        let persona = owner
            .persona_context(TEST_OWNER, &bindings)
            .await
            .unwrap();
        assert_eq!(
            persona.companion_id,
            selected.companion_id
        );
        assert!(persona.system_prompt.contains("Selected"));
        assert!(!persona.system_prompt.contains(&other.companion_id));

        let missing = nomifun_common::CompanionId::new().into_string();
        let error = owner
            .resolve_scene_bindings(TEST_OWNER, &missing, &BTreeSet::new())
            .await
            .unwrap_err();
        assert_ne!(error.code, "WAVE4_HOST_PORT_UNAVAILABLE");
    }

    #[tokio::test]
    async fn companion_actions_reach_the_real_product_owner() {
        let dir = tempfile::tempdir().unwrap();
        let service = companion_service(dir.path()).await;
        let database = nomifun_db::init_database_memory().await.unwrap();
        let companion = service
            .create_companion("Learner", "ink")
            .await
            .unwrap();
        let owner = nomifun_companion::CompanionAgentCapabilityOwner::new(
            Arc::from(TEST_OWNER),
            service,
        );
        let request = companion_action_request(
            nomifun_agent_domain_wave4::COMPANION_MODULE_ID,
            nomifun_agent_domain_wave4::COMPANION_LEARN_ACTION_ID,
            &companion.companion_id,
            serde_json::json!({}),
        );
        let first = owner.invoke(request.clone()).await.unwrap_err();
        assert_eq!(first.code, COMPANION_MODEL_NOT_CONFIGURED);
        let evolve = companion_action_request(
            nomifun_agent_domain_wave4::COMPANION_MODULE_ID,
            nomifun_agent_domain_wave4::COMPANION_EVOLVE_ACTION_ID,
            &companion.companion_id,
            serde_json::json!({}),
        );
        assert_eq!(
            owner.invoke(evolve).await.unwrap_err().code,
            COMPANION_MODEL_NOT_CONFIGURED
        );
        database.close().await;
    }

    #[tokio::test]
    async fn channel_lifecycle_reads_the_real_pairing_and_group_policy_owners() {
        use nomifun_db::IChannelRepository;

        let database = nomifun_db::init_database_memory().await.unwrap();
        let companion_dir = tempfile::tempdir().unwrap();
        let companion_service = companion_service(companion_dir.path()).await;
        let companion = companion_service
            .create_companion("Bound", "ink")
            .await
            .unwrap();
        let repository: Arc<dyn IChannelRepository> = Arc::new(
            nomifun_db::SqliteChannelRepository::new(database.pool().clone()),
        );
        let now = nomifun_common::now_ms();
        let companion_id = companion.companion_id;
        let plugin = repository
            .create_plugin(&nomifun_db::models::NewChannelPluginRow {
                r#type: "telegram".to_owned(),
                name: "test channel".to_owned(),
                enabled: true,
                config: "{}".to_owned(),
                status: Some("stopped".to_owned()),
                last_connected: None,
                companion_id: Some(companion_id.clone()),
                bot_key: Some("test-bot".to_owned()),
                owner_domain: nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION
                    .to_owned(),
                group_access_mode:
                    nomifun_db::models::CHANNEL_GROUP_ACCESS_MODE_ALLOWLIST
                        .to_owned(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let events = Arc::new(nomifun_realtime::BroadcastEventBus::new(16));
        let (message_tx, _message_rx) = tokio::sync::mpsc::channel(4);
        let manager = Arc::new(ChannelManager::new(
            Arc::clone(&repository),
            events.clone(),
            TEST_OWNER,
            [7; 32],
            message_tx,
        ));
        let pairing = Arc::new(
            PairingService::new(
                Arc::clone(&repository),
                events,
                TEST_OWNER,
            )
            .with_group_policy_fence(manager.group_policy_fence()),
        );
        let owner = Arc::new(NomiCoreChannelWave4Owner::new(
            Arc::from(TEST_OWNER),
            Arc::clone(&companion_service),
            Arc::new(Wave4DurableActionLedger::new(
                database.pool().clone(),
            )),
        ));
        let customer_service = customer_service(&database);
        owner
            .install(
                manager,
                pairing,
                Arc::clone(&repository),
                Arc::clone(&customer_service),
            )
            .unwrap();
        owner
            .install_ingress(ingress_message_service(
                &database,
                Arc::clone(&repository),
                &plugin.channel_plugin_id,
                &companion_id,
                TEST_SESSION,
            ))
            .unwrap();
        let support = owner.support();
        for capability in [
            CHANNEL_RECEIVE,
            nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID,
            CHANNEL_PAIRING,
            CHANNEL_GROUP_POLICY,
        ] {
            assert!(support.capability_is_ready(capability));
        }

        let group = owner
            .activate_lifecycle(channel_lifecycle_request(
                CHANNEL_GROUP_POLICY,
                &plugin.channel_plugin_id,
                &companion_id,
            ))
            .await
            .unwrap();
        assert_eq!(
            group.0["mode"],
            nomifun_db::models::CHANNEL_GROUP_ACCESS_MODE_ALLOWLIST
        );
        let fence = &owner.runtime.get().unwrap().group_policy_fence;
        let writer = fence.write(&plugin.channel_plugin_id).await;
        let mut waiting = Box::pin(owner.activate_lifecycle(channel_lifecycle_request(
            CHANNEL_GROUP_POLICY, &plugin.channel_plugin_id, &companion_id,
        )));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting).await.is_err());
        repository.update_plugin_group_access_mode_and_clear_non_direct_sessions(
            &plugin.channel_plugin_id, nomifun_db::models::CHANNEL_GROUP_ACCESS_MODE_DISABLED,
        ).await.unwrap();
        assert_eq!(repository.get_plugin(&plugin.channel_plugin_id).await.unwrap().unwrap().group_access_mode,
            nomifun_db::models::CHANNEL_GROUP_ACCESS_MODE_DISABLED);
        drop(writer);
        let observed = tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
            .await.unwrap().unwrap();
        assert_eq!(observed.0["mode"], nomifun_db::models::CHANNEL_GROUP_ACCESS_MODE_DISABLED);
        let pairing = owner
            .activate_lifecycle(channel_lifecycle_request(
                CHANNEL_PAIRING,
                &plugin.channel_plugin_id,
                &companion_id,
            ))
            .await
            .unwrap();
        assert_eq!(pairing.0["pending_count"], 0);
        let receive = owner
            .activate_lifecycle(channel_lifecycle_request(
                CHANNEL_RECEIVE,
                &plugin.channel_plugin_id,
                &companion_id,
            ))
            .await
            .unwrap_err();
        assert_eq!(receive.code, CHANNEL_NOT_CONNECTED);

        let mut rebound = plugin.clone();
        rebound.companion_id = Some(
            nomifun_common::CompanionId::new().into_string(),
        );
        repository.update_plugin(&rebound).await.unwrap();
        let stale = owner
            .activate_lifecycle(channel_lifecycle_request(
                CHANNEL_GROUP_POLICY,
                &plugin.channel_plugin_id,
                &companion_id,
            ))
            .await
            .unwrap_err();
        assert_eq!(
            stale.code,
            nomifun_agent_domain_wave4::WAVE4_RESOURCE_OWNER_MISMATCH
        );
        database.close().await;
    }

    #[tokio::test]
    async fn channel_send_uses_the_live_plugin_once_for_an_idempotent_replay() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let companion_dir = tempfile::tempdir().unwrap();
        let companion_service = companion_service(companion_dir.path()).await;
        let companion = companion_service
            .create_companion("Bound", "ink")
            .await
            .unwrap();
        let repository: Arc<dyn nomifun_db::IChannelRepository> = Arc::new(
            nomifun_db::SqliteChannelRepository::new(database.pool().clone()),
        );
        let events = Arc::new(nomifun_realtime::BroadcastEventBus::new(16));
        let (message_tx, _message_rx) = tokio::sync::mpsc::channel(4);
        let manager = Arc::new(ChannelManager::new(
            Arc::clone(&repository),
            events.clone(),
            TEST_OWNER,
            [9; 32],
            message_tx,
        ));
        let sends = Arc::new(AtomicUsize::new(0));
        let factory_sends = Arc::clone(&sends);
        let factory: nomifun_channel::manager::PluginFactory = Box::new(
            move |plugin_type| {
                (plugin_type == nomifun_channel::types::PluginType::Telegram)
                    .then(|| {
                        Box::new(RecordingChannelPlugin {
                            status: nomifun_channel::types::PluginStatus::Created,
                            bot_info: None,
                            sends: Arc::clone(&factory_sends),
                            fail_after_send: false,
                        }) as Box<dyn nomifun_channel::plugin::ChannelPlugin>
                    })
            },
        );
        let companion_id = companion.companion_id;
        let channel_plugin_id = manager
            .enable_plugin(
                &nomifun_channel::manager::EnableChannelSpec {
                    plugin_id: None,
                    plugin_type: Some("telegram".to_owned()),
                    companion_id: Some(companion_id.clone()),
                    owner_domain: Some(
                        nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION
                            .to_owned(),
                    ),
                },
                &serde_json::json!({
                    "credentials": { "token": "12345:test-secret" },
                    "config": null
                }),
                &factory,
            )
            .await
            .unwrap();
        let uncertain_plugin_id = manager
            .enable_plugin(
                &nomifun_channel::manager::EnableChannelSpec {
                    plugin_id: None,
                    plugin_type: Some("telegram".to_owned()),
                    companion_id: Some(companion_id.clone()),
                    owner_domain: Some(
                        nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION
                            .to_owned(),
                    ),
                },
                &serde_json::json!({
                    "credentials": { "token": "54321:uncertain" },
                    "config": null
                }),
                &factory,
            )
            .await
            .unwrap();
        let pairing = Arc::new(
            PairingService::new(
                Arc::clone(&repository),
                events,
                TEST_OWNER,
            )
            .with_group_policy_fence(manager.group_policy_fence()),
        );
        let owner = Arc::new(NomiCoreChannelWave4Owner::new(
            Arc::from(TEST_OWNER),
            Arc::clone(&companion_service),
            Arc::new(Wave4DurableActionLedger::new(
                database.pool().clone(),
            )),
        ));
        let customer_service = customer_service(&database);
        owner
            .install(
                Arc::clone(&manager),
                pairing,
                Arc::clone(&repository),
                Arc::clone(&customer_service),
            )
            .unwrap();

        let agent_session_id =
            nomifun_common::ConversationId::new().into_string();
        let message_service = ingress_message_service(
            &database,
            Arc::clone(&repository),
            &channel_plugin_id,
            &companion_id,
            &agent_session_id,
        );
        owner
            .install_ingress(Arc::clone(&message_service))
            .unwrap();
        let mut receive = channel_lifecycle_request(
            CHANNEL_RECEIVE,
            &channel_plugin_id,
            &companion_id,
        );
        receive.agent_session_id = AgentSessionId::from(agent_session_id.clone());
        owner.activate_lifecycle(receive).await.unwrap();
        assert_eq!(
            message_service
                .bound_agent_session_ingress(&channel_plugin_id)
                .await
                .as_deref(),
            Some(agent_session_id.as_str())
        );

        let mut send_request = request(serde_json::json!({
            "destination_ref": "chat-1",
            "text": "hello"
        }));
        send_request.context.resource_bindings[0].resource_id =
            nomifun_agent_contracts::ResourceId::from(channel_plugin_id.clone());
        send_request.context.resource_bindings[0].typed_parameters =
            BTreeMap::from([("companion_id".to_owned(), companion_id.clone())]);
        let first = owner.invoke(send_request.clone()).await.unwrap();
        let replay = owner.invoke(send_request.clone()).await.unwrap();
        assert_eq!(first, replay);
        assert_eq!(first.0["message_ref"], "platform-message-1");
        assert_eq!(sends.load(Ordering::SeqCst), 1);

        send_request.operation = Wave4CapabilityOperation::ChannelSend {
            input: StrictJsonValue(serde_json::json!({
                "destination_ref": "chat-1",
                "text": "changed"
            })),
        };
        assert_eq!(
            owner.invoke(send_request).await.unwrap_err().code,
            WAVE4_IDEMPOTENCY_CONFLICT
        );

        let mut uncertain = request(serde_json::json!({
            "destination_ref": "chat-2",
            "text": "may have arrived"
        }));
        uncertain.context.agent_session_id = AgentSessionId::from(
            nomifun_common::ConversationId::new().into_string(),
        );
        uncertain.context.idempotency_key = IdempotencyKey::from("uncertain-key");
        uncertain.context.resource_bindings[0].resource_id =
            nomifun_agent_contracts::ResourceId::from(uncertain_plugin_id);
        uncertain.context.resource_bindings[0].typed_parameters =
            BTreeMap::from([("companion_id".to_owned(), companion_id.clone())]);
        let error = owner.invoke(uncertain.clone()).await.unwrap_err();
        assert_eq!(error.code, WAVE4_ACTION_OUTCOME_UNKNOWN);
        let replay = owner.invoke(uncertain).await.unwrap_err();
        assert_eq!(replay.code, WAVE4_ACTION_OUTCOME_UNKNOWN);
        assert_eq!(sends.load(Ordering::SeqCst), 2);

        let cs_agent = customer_service
            .create_agent(nomifun_customer_service::CreateCsAgentInput {
                name: "Support".to_owned(),
                ..Default::default()
            })
            .await
            .unwrap();
        let customer_plugin_id = manager
            .enable_plugin(
                &nomifun_channel::manager::EnableChannelSpec {
                    plugin_id: None,
                    plugin_type: Some("telegram".to_owned()),
                    companion_id: None,
                    owner_domain: Some(
                        nomifun_db::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE
                            .to_owned(),
                    ),
                },
                &serde_json::json!({
                    "credentials": { "token": "67890:customer" },
                    "config": null
                }),
                &factory,
            )
            .await
            .unwrap();
        customer_service
            .replace_bindings(
                &cs_agent.cs_agent_id,
                vec![customer_plugin_id.clone()],
            )
            .await
            .unwrap();

        let mut customer_receive = channel_lifecycle_request(
            CHANNEL_RECEIVE,
            &customer_plugin_id,
            &companion_id,
        );
        customer_receive.resource_bindings[0].typed_parameters =
            BTreeMap::from([("cs_agent_id".to_owned(), cs_agent.cs_agent_id.clone())]);
        let customer_lifecycle = owner
            .activate_lifecycle(customer_receive)
            .await
            .unwrap();
        assert_eq!(
            customer_lifecycle.0["channel_plugin_id"],
            customer_plugin_id
        );

        let mut customer_send = request(serde_json::json!({
            "destination_ref": "customer-chat",
            "text": "support reply"
        }));
        customer_send.context.agent_session_id = AgentSessionId::from(
            nomifun_common::ConversationId::new().into_string(),
        );
        customer_send.context.idempotency_key = IdempotencyKey::from("customer-send");
        customer_send.context.resource_bindings[0].resource_id =
            nomifun_agent_contracts::ResourceId::from(customer_plugin_id.clone());
        customer_send.context.resource_bindings[0].typed_parameters =
            BTreeMap::from([("cs_agent_id".to_owned(), cs_agent.cs_agent_id.clone())]);
        let sent = owner.invoke(customer_send.clone()).await.unwrap();
        assert_eq!(sent.0["message_ref"], "platform-message-3");
        assert_eq!(owner.invoke(customer_send.clone()).await.unwrap(), sent);
        assert_eq!(sends.load(Ordering::SeqCst), 3);

        let mut wrong_cs = customer_send.clone();
        wrong_cs.context.agent_session_id = AgentSessionId::from(
            nomifun_common::ConversationId::new().into_string(),
        );
        wrong_cs.context.idempotency_key = IdempotencyKey::from("wrong-cs-key");
        wrong_cs.context.resource_bindings[0].typed_parameters =
            BTreeMap::from([("cs_agent_id".to_owned(), nomifun_common::CsAgentId::new().into_string())]);
        assert_eq!(
            owner.invoke(wrong_cs).await.unwrap_err().code,
            nomifun_agent_domain_wave4::WAVE4_RESOURCE_OWNER_MISMATCH
        );

        let mut cross_domain = customer_send;
        cross_domain.context.agent_session_id = AgentSessionId::from(
            nomifun_common::ConversationId::new().into_string(),
        );
        cross_domain.context.idempotency_key = IdempotencyKey::from("cross-domain-key");
        cross_domain.context.resource_bindings[0].typed_parameters =
            BTreeMap::from([("companion_id".to_owned(), companion_id.clone())]);
        assert_eq!(
            owner.invoke(cross_domain).await.unwrap_err().code,
            nomifun_agent_domain_wave4::WAVE4_RESOURCE_OWNER_MISMATCH
        );

        assert_eq!(
            owner
                .unbind_session_ingress(&agent_session_id)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            message_service
                .bound_agent_session_ingress(&channel_plugin_id)
                .await
                ,
            None
        );
        database.close().await;
    }
}
