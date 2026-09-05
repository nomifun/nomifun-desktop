//! Typed Session boundary used by Cron execution.

#[cfg(test)]
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::AgentSessionId;
#[cfg(test)]
use nomifun_ai_agent::AgentRuntimeRegistry;
#[cfg(test)]
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ResolvedPresetSnapshot,
};
#[cfg(test)]
use nomifun_api_types::SendMessageRequest;
use nomifun_common::{AgentType, AppError, ProviderWithModel};
#[cfg(test)]
use nomifun_conversation::service::{
    BackgroundTurnReconciliationDisposition, PublicTurnDeliveryState,
};
#[cfg(test)]
use nomifun_conversation::service::BackgroundTurnRuntimePreparation;
#[cfg(test)]
use nomifun_conversation::ConversationService;

/// One Cron turn handed to the canonical Session owner.
///
/// Cron supplies only the user-visible message and the server-owned per-run
/// overlay (for example the Cron id and selected skills). The Session port
/// resolves the authoritative agent type, model, delegation policy, workspace
/// identity and conversation creation timestamp from its own projection before
/// it constructs any runtime preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnMessage {
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
    pub agent_type: AgentType,
    pub model: Option<ProviderWithModel>,
    pub workspace: String,
    pub cron_job_id: Option<String>,
    pub temp_workspace_id: Option<String>,
    pub skills: Vec<String>,
    pub agent_name: Option<String>,
    pub cli_path: Option<String>,
    pub custom_agent_id: Option<String>,
    pub preset_id: Option<String>,
    pub preset_revision: Option<i64>,
    pub preset_snapshot: Option<ResolvedPresetSnapshot>,
}

pub type CronScheduledSession = CronSessionProjection;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSessionCronBindingRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub cron_job_id: String,
}

/// Stable terminal receipt owned by the Cron boundary.  The consumer never
/// needs the Conversation repository row or runtime handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
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
    Completed(CronTurnDelivery),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnReceiptQuery {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
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

#[cfg(test)]
pub(crate) fn turn_delivery_from_conversation(
    delivery: nomifun_conversation::IdempotentMessageDelivery,
) -> CronTurnDelivery {
    CronTurnDelivery {
        message_id: delivery.message_id,
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    }
}

#[cfg(test)]
pub(crate) fn turn_state_from_conversation(
    state: PublicTurnDeliveryState,
) -> CronTurnReceiptState {
    match state {
        PublicTurnDeliveryState::Missing => CronTurnReceiptState::Missing,
        PublicTurnDeliveryState::Accepted { message_id } => {
            CronTurnReceiptState::Accepted { message_id }
        }
        PublicTurnDeliveryState::Completed(delivery) => {
            CronTurnReceiptState::Completed(turn_delivery_from_conversation(delivery))
        }
    }
}

#[cfg(test)]
pub(crate) const fn turn_reconciliation_from_conversation(
    disposition: BackgroundTurnReconciliationDisposition,
) -> CronTurnReconciliation {
    match disposition {
        BackgroundTurnReconciliationDisposition::LiveExactOwnerWait => {
            CronTurnReconciliation::LiveExactOwnerWait
        }
        BackgroundTurnReconciliationDisposition::ReconciledOrTerminalReRead => {
            CronTurnReconciliation::ReconciledOrTerminalReRead
        }
        BackgroundTurnReconciliationDisposition::ExternalProofRequiredFailClosed => {
            CronTurnReconciliation::ExternalProofRequiredFailClosed
        }
        BackgroundTurnReconciliationDisposition::StaleConflict => {
            CronTurnReconciliation::StaleConflict
        }
    }
}

#[cfg(test)]
pub(crate) fn session_handle_from_response(
    response: ConversationResponse,
) -> Result<CronSessionHandle, AppError> {
    let workspace = response
        .extra
        .get("workspace")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|workspace| !workspace.is_empty())
        .ok_or_else(|| {
            AppError::Conflict(format!(
                "AgentSession {} has no canonical workspace",
                response.conversation_id
            ))
        })?
        .to_owned();
    let agent_session_id = AgentSessionId::from(response.conversation_id);
    nomifun_common::validate_uuidv7(agent_session_id.as_ref()).map_err(|error| {
        AppError::Conflict(format!(
            "AgentSession identity is not canonical UUIDv7: {error}"
        ))
    })?;
    Ok(CronSessionHandle {
        agent_session_id,
        workspace,
    })
}

/// Exact Session operations needed by the Cron domain.
#[async_trait]
pub trait CronSessionPort: Send + Sync {
    async fn get_session(
        &self,
        query: &CronSessionLookup,
    ) -> Result<CronSessionProjection, AppError>;

    async fn lookup_scheduled_sessions(
        &self,
        query: &CronScheduledSessionLookup,
    ) -> Result<Vec<CronScheduledSession>, AppError>;

    /// Legacy HTTP projection retained for the existing Cron conversations
    /// endpoint. Core scheduling and execution must use
    /// `lookup_scheduled_sessions`/`get_session`.
    async fn list_conversation_responses_for_cron(
        &self,
        query: &CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError>;

    async fn bind_cron_relation(
        &self,
        request: &CronSessionCronBindingRequest,
    ) -> Result<(), AppError>;

    async fn read_turn_receipt(
        &self,
        query: &CronTurnReceiptQuery,
    ) -> Result<CronTurnReceiptState, AppError>;

    async fn reconcile_turn_receipt(
        &self,
        request: &CronTurnReconciliationRequest,
    ) -> Result<CronTurnReconciliation, AppError>;

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<ResolvedPresetSnapshot>,
        creation_key: &str,
    ) -> Result<CronSessionHandle, AppError>;

    async fn prepare_runtime_and_send(
        &self,
        request: CronRuntimePreparationRequest,
    ) -> Result<CronPreparedTurnDelivery, AppError>;

    async fn delivery_result(
        &self,
        query: &CronTurnDeliveryQuery,
    ) -> Result<Option<CronTurnDelivery>, AppError>;
}

#[cfg(test)]
struct TestCronSessionPort {
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

#[cfg(test)]
#[async_trait]
impl CronSessionPort for TestCronSessionPort {
    async fn get_session(
        &self,
        query: &CronSessionLookup,
    ) -> Result<CronSessionProjection, AppError> {
        let row = self
            .service
            .conversation_repo()
            .get(query.agent_session_id.as_ref())
            .await?
            .filter(|row| row.user_id == query.owner_id)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "AgentSession {} not found",
                    query.agent_session_id.as_ref()
                ))
            })?;
        let response = self
            .service
            .get(&query.owner_id, query.agent_session_id.as_ref())
            .await?;
        session_projection_from_response(&query.owner_id, response, row.cron_job_id)
    }

    async fn lookup_scheduled_sessions(
        &self,
        query: &CronScheduledSessionLookup,
    ) -> Result<Vec<CronScheduledSession>, AppError> {
        self.service
            .list_by_cron_job(&query.owner_id, &query.cron_job_id)
            .await?
            .into_iter()
            .map(|response| {
                session_projection_from_response(
                    &query.owner_id,
                    response,
                    Some(query.cron_job_id.clone()),
                )
            })
            .collect()
    }

    async fn list_conversation_responses_for_cron(
        &self,
        query: &CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError> {
        self.service
            .list_by_cron_job(&query.owner_id, &query.cron_job_id)
            .await
    }

    async fn bind_cron_relation(
        &self,
        request: &CronSessionCronBindingRequest,
    ) -> Result<(), AppError> {
        let row = self
            .service
            .conversation_repo()
            .get(request.agent_session_id.as_ref())
            .await?
            .filter(|row| row.user_id == request.owner_id)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "AgentSession {} not found",
                    request.agent_session_id.as_ref()
                ))
            })?;
        if row.cron_job_id.as_deref() == Some(request.cron_job_id.as_str()) {
            return Ok(());
        }
        if let Some(existing) = row.cron_job_id {
            return Err(AppError::Conflict(format!(
                "AgentSession {} is already bound to cron job {existing}",
                request.agent_session_id.as_ref()
            )));
        }
        self.service
            .conversation_repo()
            .bind_cron_relation(
                &request.owner_id,
                request.agent_session_id.as_ref(),
                &request.cron_job_id,
                nomifun_common::now_ms(),
            )
            .await?;
        Ok(())
    }

    async fn read_turn_receipt(
        &self,
        query: &CronTurnReceiptQuery,
    ) -> Result<CronTurnReceiptState, AppError> {
        Ok(turn_state_from_conversation(self.service
            .public_turn_delivery_state(
                &query.owner_id,
                query.agent_session_id.as_ref(),
                &query.idempotency_key,
            )
            .await?))
    }

    async fn reconcile_turn_receipt(
        &self,
        request: &CronTurnReconciliationRequest,
    ) -> Result<CronTurnReconciliation, AppError> {
        Ok(turn_reconciliation_from_conversation(self.service
            .reconcile_quiescent_running_turn_for_background(
                &request.owner_id,
                request.agent_session_id.as_ref(),
                &request.idempotency_key,
                &self.runtime_registry,
            )
            .await?))
    }

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<ResolvedPresetSnapshot>,
        creation_key: &str,
    ) -> Result<CronSessionHandle, AppError> {
        match snapshot {
            Some(snapshot) => {
                let response = self.service
                    .create_from_preset_snapshot_idempotent(
                        user_id,
                        request,
                        snapshot,
                        creation_key,
                    )
                    .await?;
                session_handle_from_response(response)
            }
            None => {
                let response = self.service
                    .create_idempotent(user_id, request, creation_key)
                    .await?;
                session_handle_from_response(response)
            }
        }
    }

    async fn prepare_runtime_and_send(
        &self,
        request: CronRuntimePreparationRequest,
    ) -> Result<CronPreparedTurnDelivery, AppError> {
        let CronRuntimePreparationRequest {
            owner_id,
            agent_session_id,
            idempotency_key,
            turn,
        } = request;
        let CronTurnRequest {
            message,
            runtime:
                CronTurnRuntimePreparation {
                    overlay,
                    clear_context,
                },
        } = turn;
        let build_lease = self
            .service
            .begin_public_runtime_preparation(agent_session_id.as_ref(), &owner_id)?;
        let session = self
            .service
            .get(&owner_id, agent_session_id.as_ref())
            .await?;
        build_lease.ensure_active()?;
        let requested_skills = message.inject_skills.clone();
        let (mut runtime_options, workspace) =
            runtime_options_from_session(&owner_id, session, &overlay, &requested_skills)?;
        // The production Session owner resolves and validates this typed
        // message field inside its own preparation gate. This test-only
        // Conversation adapter mirrors that behavior without reopening the
        // arbitrary runtime-extra escape hatch.
        append_requested_skills(&mut runtime_options, &requested_skills);
        let observed = self.service
            .send_observed_background_message_with_idempotency_key(
                &owner_id,
                agent_session_id.as_ref(),
                &idempotency_key,
                send_message_request(message),
                &self.runtime_registry,
                build_lease,
                BackgroundTurnRuntimePreparation {
                    runtime_options,
                    clear_context,
                    pre_send_hook: None,
                },
            )
            .await?;
        Ok(CronPreparedTurnDelivery {
            delivery: turn_delivery_from_conversation(observed.delivery),
            workspace,
        })
    }

    async fn delivery_result(
        &self,
        query: &CronTurnDeliveryQuery,
    ) -> Result<Option<CronTurnDelivery>, AppError> {
        let request = send_message_request(query.message.clone());
        Ok(self.service
            .idempotent_delivery_result_with_idempotency_key(
                &query.owner_id,
                query.agent_session_id.as_ref(),
                &query.idempotency_key,
                &request,
            )
            .await?
            .map(turn_delivery_from_conversation))
    }
}

#[cfg(test)]
fn session_projection_from_response(
    owner_id: &str,
    response: ConversationResponse,
    cron_job_id: Option<String>,
) -> Result<CronSessionProjection, AppError> {
    let ConversationResponse {
        conversation_id,
        name,
        r#type: agent_type,
        model,
        preset_id,
        preset_revision,
        preset_snapshot,
        extra,
        ..
    } = response;
    let extra = extra.as_object().ok_or_else(|| {
        AppError::Internal(format!(
            "conversation {conversation_id} extra must be a JSON object"
        ))
    })?;
    let workspace = extra
        .get("workspace")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|workspace| !workspace.is_empty())
        .ok_or_else(|| {
            AppError::Conflict(format!(
                "AgentSession {conversation_id} has no canonical workspace"
            ))
        })?
        .to_owned();
    let agent_session_id = AgentSessionId::from(conversation_id);
    nomifun_common::validate_uuidv7(agent_session_id.as_ref()).map_err(|error| {
        AppError::Conflict(format!(
            "AgentSession identity is not canonical UUIDv7: {error}"
        ))
    })?;

    Ok(CronSessionProjection {
        agent_session_id,
        owner_id: owner_id.to_owned(),
        name,
        agent_type,
        model,
        workspace,
        cron_job_id,
        temp_workspace_id: extra
            .get("temp_workspace_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        skills: extra
            .get("skills")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        agent_name: extra
            .get("agent_name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        cli_path: extra
            .get("cli_path")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                extra
                    .get("gateway")
                    .and_then(|gateway| gateway.get("cli_path"))
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_owned),
        custom_agent_id: extra
            .get("custom_agent_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        preset_id,
        preset_revision,
        preset_snapshot,
    })
}

#[cfg(test)]
fn send_message_request(message: CronTurnMessage) -> SendMessageRequest {
    SendMessageRequest {
        content: message.content,
        files: message.files,
        inject_skills: message.inject_skills,
        hidden: message.hidden,
        origin: message.origin,
        channel_platform: message.channel_platform,
    }
}

/// Translate the latest canonical Session projection into the legacy runtime
/// preparation shape for the in-crate test adapter only.
///
/// Cron contributes one closed annotation. Workspace, managed-workspace
/// identity, skills, preset/runtime selectors, model, and delegation policy
/// all remain exactly as resolved by the Session owner while its preparation
/// lease is active.
#[cfg(test)]
fn runtime_options_from_session(
    user_id: &str,
    session: ConversationResponse,
    overlay: &CronTurnRuntimeOverlay,
    _requested_skills: &[String],
) -> Result<(AgentRuntimeBuildOptions, String), AppError> {
    let ConversationResponse {
        conversation_id,
        r#type: agent_type,
        model,
        delegation_policy,
        created_at,
        extra: mut session_extra,
        ..
    } = session;

    let session_extra = session_extra.as_object_mut().ok_or_else(|| {
        AppError::Internal(format!(
            "conversation {conversation_id} extra must be a JSON object"
        ))
    })?;
    nomifun_common::CronJobId::parse(&overlay.cron_job_id).map_err(|error| {
        AppError::BadRequest(format!("invalid Cron runtime annotation: {error}"))
    })?;
    session_extra.insert(
        "cron_job_id".to_owned(),
        serde_json::Value::String(overlay.cron_job_id.clone()),
    );

    let workspace = session_extra
        .get("workspace")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Internal(format!(
                "conversation {conversation_id} has no canonical workspace"
            ))
        })?
        .to_owned();

    Ok((
        AgentRuntimeBuildOptions {
            user_id: user_id.to_owned(),
            agent_type,
            workspace: workspace.clone(),
            model,
            conversation_id,
            delegation_policy,
            extra: session_extra.clone().into(),
            conversation_created_at: Some(created_at),
            workspace_binding_lease: None,
        },
        workspace,
    ))
}

#[cfg(test)]
fn append_requested_skills(
    runtime_options: &mut AgentRuntimeBuildOptions,
    requested_skills: &[String],
) {
    if requested_skills.is_empty() {
        return;
    }
    let Some(extra) = runtime_options.extra.as_object_mut() else {
        return;
    };
    let mut skills = extra
        .get("skills")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for skill in requested_skills {
        if !skills
            .iter()
            .any(|existing| existing.as_str() == Some(skill.as_str()))
        {
            skills.push(serde_json::Value::String(skill.clone()));
        }
    }
    extra.insert("skills".to_owned(), serde_json::Value::Array(skills));
}

/// Build the Conversation-backed adapter used only by unit tests in this
/// crate. Production composition supplies a `CronSessionPort` implementation
/// from the host-owned Session owner directly.
#[cfg(test)]
pub(crate) fn test_cron_session_port(
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> Arc<dyn CronSessionPort> {
    Arc::new(TestCronSessionPort {
        service,
        runtime_registry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{
        AgentType, ConversationStatus, DecisionPolicy, DelegationPolicy, ProviderWithModel,
    };

    const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    fn session(extra: serde_json::Value) -> ConversationResponse {
        ConversationResponse {
            conversation_id: CONVERSATION_ID.to_owned(),
            name: "scheduled".to_owned(),
            r#type: AgentType::Nomi,
            model: Some(ProviderWithModel {
                provider_id: PROVIDER_ID.to_owned(),
                model: "gpt-5".to_owned(),
                use_model: None,
            }),
            status: ConversationStatus::Finished,
            runtime: None,
            source: None,
            pinned: false,
            pinned_at: None,
            channel_chat_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            delegation_policy: DelegationPolicy::PreferParallel,
            execution_model_pool: None,
            decision_policy: DecisionPolicy::Automatic,
            execution_template_id: None,
            linked_execution_id: None,
            execution_step_id: None,
            execution_attempt_id: None,
            created_at: 42,
            modified_at: 43,
            extra,
        }
    }

    #[test]
    fn runtime_options_take_authority_from_session_projection() {
        let (options, workspace) = runtime_options_from_session(
            USER_ID,
            session(serde_json::json!({
                "workspace": "  C:/session-workspace  ",
                "delegation_policy": "disabled",
                "model": "forged",
            })),
            &CronTurnRuntimeOverlay {
                cron_job_id: "0190f5fe-7c00-7a00-8abc-012345678902".to_owned(),
            },
            &[],
        )
        .expect("valid Session projection");

        assert_eq!(options.user_id, USER_ID);
        assert_eq!(options.conversation_id, CONVERSATION_ID);
        assert_eq!(options.agent_type, AgentType::Nomi);
        assert_eq!(
            options.model.as_ref().map(|model| model.provider_id.as_str()),
            Some(PROVIDER_ID)
        );
        assert_eq!(options.delegation_policy, DelegationPolicy::PreferParallel);
        assert_eq!(options.conversation_created_at, Some(42));
        assert_eq!(options.workspace, "C:/session-workspace");
        assert_eq!(workspace, "C:/session-workspace");
        assert_eq!(
            options.extra["cron_job_id"],
            "0190f5fe-7c00-7a00-8abc-012345678902"
        );
        assert_eq!(options.extra["delegation_policy"], "disabled");
    }

    #[test]
    fn runtime_options_reject_missing_session_workspace() {
        let error = runtime_options_from_session(
            USER_ID,
            session(serde_json::json!({})),
            &CronTurnRuntimeOverlay {
                cron_job_id: "0190f5fe-7c00-7a00-8abc-012345678902".to_owned(),
            },
            &[],
        )
        .expect_err("a runtime must have a canonical workspace");
        assert!(error.to_string().contains("no canonical workspace"));
    }
}
