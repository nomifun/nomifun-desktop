//! Production owners for Wave 5 product Modules.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use nomifun_agent_contracts::{
    DigestHex, StrictJsonValue, TypedResourceBinding, canonical_json_bytes,
    digest_payload,
};
use nomifun_agent_domain_wave5::{
    Wave5CapabilityOperation, Wave5HostContext, Wave5HostPort, Wave5HostPortError,
    Wave5HostRequest, WAVE5_EFFECT_OUTCOME_UNKNOWN,
};
use nomifun_api_types::{
    CreateRequirementRequest, ListRequirementsQuery, RequirementStatus,
    UpdateRequirementRequest,
};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_IDEMPOTENCY_BYTES: usize = 128;

pub(crate) struct NomiCoreWave5Host {
    authoritative_user_id: Arc<str>,
    effects: nomifun_agent_session::AgentSessionStore,
    requirements: Arc<nomifun_requirement::RequirementService>,
    execution: OnceLock<Arc<nomifun_agent_execution::AgentExecutionEngine>>,
    schedule: OnceLock<nomifun_cron::ScheduleActionOwner>,
}

impl NomiCoreWave5Host {
    pub(crate) fn new(
        authoritative_user_id: Arc<str>,
        effects: nomifun_agent_session::AgentSessionStore,
        requirements: Arc<nomifun_requirement::RequirementService>,
    ) -> Self {
        Self {
            authoritative_user_id,
            effects,
            requirements,
            execution: OnceLock::new(),
            schedule: OnceLock::new(),
        }
    }

    pub(crate) fn install_runtime_owners(
        &self,
        execution: Arc<nomifun_agent_execution::AgentExecutionEngine>,
        schedule: Arc<nomifun_cron::service::CronService>,
    ) -> Result<(), String> {
        self.execution
            .set(execution)
            .map_err(|_| "Wave 5 AgentExecution owner was already installed".to_owned())?;
        self.schedule
            .set(nomifun_cron::ScheduleActionOwner::new(schedule))
            .map_err(|_| "Wave 5 Schedule owner was already installed".to_owned())
    }

    fn execution(&self) -> Result<&Arc<nomifun_agent_execution::AgentExecutionEngine>, Wave5HostPortError> {
        self.execution.get().ok_or_else(|| {
            Wave5HostPortError::unavailable("AgentExecution owner is not installed")
        })
    }

    fn schedule(&self) -> Result<&nomifun_cron::ScheduleActionOwner, Wave5HostPortError> {
        self.schedule.get().ok_or_else(|| {
            Wave5HostPortError::unavailable("Schedule owner is not installed")
        })
    }

    async fn invoke_inner(
        &self,
        request: Wave5HostRequest,
    ) -> Result<StrictJsonValue, Wave5HostPortError> {
        request.validate()?;
        let context = request.context;
        match request.operation {
            Wave5CapabilityOperation::AgentDelegate { input } => {
                let input: CollaborationInput = decode(input)?;
                self.run_effect(
                    &context,
                    &StrictJsonValue(json!({"goal": input.goal})),
                    None,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                    async {
                        let execution = self
                            .execution()?
                            .collaborate_from_session(
                                &context.principal.principal_id,
                                context.agent_session_id.as_ref(),
                                input.goal,
                                false,
                            )
                            .await
                            .map_err(app_error)?;
                        encode(json!({
                            "execution_id": execution.execution_id,
                            "status": execution.status,
                            "mode": "delegate",
                        }))
                    },
                )
                .await
            }
            Wave5CapabilityOperation::AgentFork { input } => {
                let input: CollaborationInput = decode(input)?;
                self.run_effect(
                    &context,
                    &StrictJsonValue(json!({"goal": input.goal})),
                    None,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                    async {
                        let execution = self
                            .execution()?
                            .collaborate_from_session(
                                &context.principal.principal_id,
                                context.agent_session_id.as_ref(),
                                input.goal,
                                true,
                            )
                            .await
                            .map_err(app_error)?;
                        encode(json!({
                            "execution_id": execution.execution_id,
                            "status": execution.status,
                            "mode": "fork",
                        }))
                    },
                )
                .await
            }
            Wave5CapabilityOperation::ScheduleList { input } => {
                let input: nomifun_cron::ScheduleListInput = decode(input)?;
                let (authority, binding) = schedule_authority(&context)?;
                let output = self
                    .schedule()?
                    .list(&authority, &schedule_context(&context), input)
                    .await
                    .map_err(schedule_error)?;
                let _ = binding;
                encode(output)
            }
            Wave5CapabilityOperation::ScheduleCreate { input } => {
                let typed: nomifun_cron::ScheduleCreateInput = decode(input.clone())?;
                let (authority, binding) = schedule_authority(&context)?;
                self.run_effect(
                    &context,
                    &input,
                    Some(&binding),
                    nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
                    async {
                        encode(
                            self.schedule()?
                                .create(&authority, &schedule_context(&context), typed)
                                .await
                                .map_err(schedule_error)?,
                        )
                    },
                )
                .await
            }
            Wave5CapabilityOperation::ScheduleUpdate { input } => {
                let typed: nomifun_cron::ScheduleUpdateInput = decode(input.clone())?;
                let (authority, binding) = schedule_authority(&context)?;
                self.run_effect(
                    &context,
                    &input,
                    Some(&binding),
                    nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
                    async {
                        encode(
                            self.schedule()?
                                .update(&authority, &schedule_context(&context), typed)
                                .await
                                .map_err(schedule_error)?,
                        )
                    },
                )
                .await
            }
            Wave5CapabilityOperation::ScheduleDelete { input } => {
                let typed: nomifun_cron::ScheduleDeleteInput = decode(input.clone())?;
                let (authority, binding) = schedule_authority(&context)?;
                self.run_effect(
                    &context,
                    &input,
                    Some(&binding),
                    nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
                    async {
                        encode(
                            self.schedule()?
                                .delete(&authority, &schedule_context(&context), typed)
                                .await
                                .map_err(schedule_error)?,
                        )
                    },
                )
                .await
            }
            Wave5CapabilityOperation::RequirementsRead { input } => {
                self.require_installation_owner(&context)?;
                let input: RequirementReadInput = decode(input)?;
                match input {
                    RequirementReadInput::Get { requirement_id } => {
                        encode(self.requirements.get(&requirement_id).await.map_err(app_error)?)
                    }
                    RequirementReadInput::List {
                        tag,
                        status,
                        query,
                        page,
                        page_size,
                    } => {
                        let status = status.map(parse_requirement_status).transpose()?;
                        encode(
                            self.requirements
                                .list(&ListRequirementsQuery {
                                    tag,
                                    status,
                                    conversation_id: None,
                                    q: query,
                                    order_by: None,
                                    order: None,
                                    page,
                                    page_size,
                                })
                                .await
                                .map_err(app_error)?,
                        )
                    }
                }
            }
            Wave5CapabilityOperation::RequirementsWrite { input } => {
                self.require_installation_owner(&context)?;
                let typed: RequirementWriteInput = decode(input.clone())?;
                self.run_effect(
                    &context,
                    &input,
                    None,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                    async {
                        match typed {
                            RequirementWriteInput::Create { title, content, tag } => {
                                let requirement = self.requirements
                                    .create(CreateRequirementRequest {
                                        title,
                                        content: content.unwrap_or_default(),
                                        tag,
                                        order_key: None,
                                        status: None,
                                        created_by: Some("agent".to_owned()),
                                        attachments: Vec::new(),
                                    })
                                    .await
                                    .map_err(app_error)?;
                                encode(requirement_receipt(&requirement))
                            }
                            RequirementWriteInput::Update {
                                requirement_id,
                                title,
                                content,
                                tag,
                            } => {
                                let requirement = self.requirements
                                    .update(
                                        &requirement_id,
                                        UpdateRequirementRequest {
                                            title,
                                            content,
                                            tag,
                                            order_key: None,
                                            status: None,
                                            completion_note: None,
                                            add_attachments: Vec::new(),
                                            remove_attachment_ids: Vec::new(),
                                        },
                                    )
                                    .await
                                    .map_err(app_error)?;
                                encode(requirement_receipt(&requirement))
                            }
                            RequirementWriteInput::Delete { requirement_id } => {
                                self.requirements
                                    .delete(&requirement_id)
                                    .await
                                    .map_err(app_error)?;
                                encode(json!({"requirement_id": requirement_id, "deleted": true}))
                            }
                        }
                    },
                )
                .await
            }
            Wave5CapabilityOperation::RequirementsStatus { input } => {
                self.require_installation_owner(&context)?;
                let typed: RequirementStatusInput = decode(input.clone())?;
                let status = parse_requirement_status(typed.status)?;
                self.run_effect(
                    &context,
                    &input,
                    None,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                    async {
                        let requirement = self.requirements
                                .set_status_for_agent_session(
                                    &typed.requirement_id,
                                    context.agent_session_id.as_ref(),
                                    status,
                                    typed.completion_note,
                                )
                                .await
                                .map_err(app_error)?;
                        encode(requirement_receipt(&requirement))
                    },
                )
                .await
            }
            Wave5CapabilityOperation::RequirementsClaim { input } => {
                self.require_installation_owner(&context)?;
                let typed: RequirementClaimInput = decode(input.clone())?;
                self.run_effect(
                    &context,
                    &input,
                    None,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                    async {
                        let requirement = self.requirements
                                .claim_next_for_agent_session(
                                    &typed.tag,
                                    context.agent_session_id.as_ref(),
                                )
                                .await
                                .map_err(app_error)?;
                        encode(json!({
                            "requirement": requirement.as_ref().map(requirement_receipt),
                        }))
                    },
                )
                .await
            }
        }
    }

    fn require_installation_owner(
        &self,
        context: &Wave5HostContext,
    ) -> Result<(), Wave5HostPortError> {
        ensure_installation_owner(
            self.authoritative_user_id.as_ref(),
            &context.principal.principal_id,
        )
    }

    async fn run_effect<F>(
        &self,
        context: &Wave5HostContext,
        input: &StrictJsonValue,
        binding: Option<&TypedResourceBinding>,
        strategy: nomifun_agent_session::EffectStrategy,
        invoke: F,
    ) -> Result<StrictJsonValue, Wave5HostPortError>
    where
        F: Future<Output = Result<StrictJsonValue, Wave5HostPortError>>,
    {
        match begin_effect(&self.effects, context, binding, input, strategy).await? {
            EffectAdmission::Replay(output) => Ok(output),
            EffectAdmission::Reserved(reservation) => {
                let result = invoke.await;
                match &result {
                    Ok(output) => finish_effect(
                        &reservation,
                        EffectCompletion::Succeeded(output),
                    )
                    .await?,
                    Err(error) if error.code == WAVE5_EFFECT_OUTCOME_UNKNOWN => {
                        finish_effect(&reservation, EffectCompletion::Uncertain(error)).await?
                    }
                    Err(error) => {
                        finish_effect(&reservation, EffectCompletion::Failed(error)).await?
                    }
                }
                result
            }
        }
    }
}

impl Wave5HostPort for NomiCoreWave5Host {
    fn invoke<'a>(
        &'a self,
        request: Wave5HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave5HostPortError>> + Send + 'a>> {
        Box::pin(async move { self.invoke_inner(request).await })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollaborationInput {
    goal: String,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum RequirementReadInput {
    Get { requirement_id: String },
    List {
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        page: Option<u32>,
        #[serde(default)]
        page_size: Option<u32>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum RequirementWriteInput {
    Create {
        title: String,
        #[serde(default)]
        content: Option<String>,
        tag: String,
    },
    Update {
        requirement_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        tag: Option<String>,
    },
    Delete { requirement_id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequirementStatusInput {
    requirement_id: String,
    status: String,
    #[serde(default)]
    completion_note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequirementClaimInput {
    tag: String,
}

/// Effect receipts deliberately exclude unbounded Requirement content and
/// completion notes. The model can read the exact row through
/// `requirements/read`; mutations must retain a compact, durable response so
/// a lost HTTP/tool response can be replayed with the same idempotency key.
fn requirement_receipt(requirement: &nomifun_api_types::Requirement) -> Value {
    json!({
        "requirement_id": requirement.requirement_id,
        "display_no": requirement.display_no,
        "title": requirement.title,
        "tag": requirement.tag,
        "status": requirement.status,
    })
}

fn ensure_installation_owner(
    authoritative_user_id: &str,
    principal_id: &str,
) -> Result<(), Wave5HostPortError> {
    if principal_id != authoritative_user_id {
        return Err(Wave5HostPortError::new(
            nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH,
            "Requirements actions require the authenticated installation owner",
        ));
    }
    Ok(())
}

fn parse_requirement_status(status: String) -> Result<RequirementStatus, Wave5HostPortError> {
    serde_json::from_value(Value::String(status))
        .map_err(|error| Wave5HostPortError::invalid_request(error.to_string()))
}

fn schedule_context(context: &Wave5HostContext) -> nomifun_cron::ScheduleActionContext {
    nomifun_cron::ScheduleActionContext {
        principal_id: context.principal.principal_id.clone(),
        agent_session_id: context.agent_session_id.as_ref().to_owned(),
        operation_id: context.operation_id.as_ref().to_owned(),
    }
}

fn schedule_authority(
    context: &Wave5HostContext,
) -> Result<(nomifun_cron::ScheduleAuthority, TypedResourceBinding), Wave5HostPortError> {
    let binding = exact_binding(context, nomifun_cron::SCHEDULER_RESOURCE_KIND)?;
    let resource = nomifun_cron::ScheduleResourceBinding::from_operation_names(
        binding.binding_id.as_ref(),
        binding.resource_id.as_ref(),
        binding.owner_id.clone(),
        binding.operations.iter(),
    )
    .map_err(schedule_error)?;
    let authority = nomifun_cron::ScheduleAuthority::bound(
        context.principal.principal_id.clone(),
        resource,
    )
    .map_err(schedule_error)?;
    Ok((authority, binding))
}

fn exact_binding(
    context: &Wave5HostContext,
    kind: &str,
) -> Result<TypedResourceBinding, Wave5HostPortError> {
    let mut bindings = context
        .resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == kind);
    let binding = bindings.next().cloned().ok_or_else(|| {
        Wave5HostPortError::resource_binding_invalid(format!(
            "{} requires one {kind} resource binding",
            context.capability_id.as_ref()
        ))
    })?;
    if bindings.next().is_some() {
        return Err(Wave5HostPortError::resource_binding_invalid(format!(
            "{} received multiple {kind} resource bindings",
            context.capability_id.as_ref()
        )));
    }
    Ok(binding)
}

fn decode<T: for<'de> Deserialize<'de>>(input: StrictJsonValue) -> Result<T, Wave5HostPortError> {
    serde_json::from_value(input.0)
        .map_err(|error| Wave5HostPortError::invalid_request(error.to_string()))
}

fn encode(value: impl serde::Serialize) -> Result<StrictJsonValue, Wave5HostPortError> {
    serde_json::to_value(value)
        .map(StrictJsonValue)
        .map_err(|error| Wave5HostPortError::new("WAVE5_INVALID_RESPONSE", error.to_string()))
}

fn app_error(error: nomifun_common::AppError) -> Wave5HostPortError {
    Wave5HostPortError::new("WAVE5_OWNER_REJECTED", error.to_string())
}

fn schedule_error(error: nomifun_cron::ScheduleActionError) -> Wave5HostPortError {
    match error {
        nomifun_cron::ScheduleActionError::OutcomeUnknown(message) => {
            Wave5HostPortError::new(WAVE5_EFFECT_OUTCOME_UNKNOWN, message)
        }
        other => Wave5HostPortError::new("SCHEDULE_ACTION_REJECTED", other.to_string()),
    }
}

#[derive(Clone)]
struct EffectReservation {
    store: nomifun_agent_session::AgentSessionStore,
    request: nomifun_agent_session::EffectEventRequest,
}

enum EffectAdmission {
    Replay(StrictJsonValue),
    Reserved(EffectReservation),
}

enum EffectCompletion<'a> {
    Succeeded(&'a StrictJsonValue),
    Failed(&'a Wave5HostPortError),
    Uncertain(&'a Wave5HostPortError),
}

fn effect_id(context: &Wave5HostContext) -> Result<String, Wave5HostPortError> {
    let key = context.idempotency_key.as_ref();
    if key.is_empty()
        || key.len() > MAX_IDEMPOTENCY_BYTES
        || !key.bytes().all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return Err(Wave5HostPortError::invalid_request(
            "Wave 5 effect requires a bounded visible idempotency key",
        ));
    }
    let digest = digest_payload(&json!({
        "agent_session_id": context.agent_session_id,
        "turn_id": context.turn_id,
        "idempotency_key": key,
        "module": context.capability_id,
        "action": context.action_id,
    }))
    .map_err(|error| Wave5HostPortError::invalid_request(error.to_string()))?;
    Ok(format!("wave5:{}", digest.as_ref()))
}

fn input_digest(
    context: &Wave5HostContext,
    binding: Option<&TypedResourceBinding>,
    input: &StrictJsonValue,
) -> Result<DigestHex, Wave5HostPortError> {
    digest_payload(&json!({
        "module": context.capability_id,
        "action": context.action_id,
        "binding": binding,
        "input": input.0,
    }))
    .map_err(|error| Wave5HostPortError::invalid_request(error.to_string()))
}

fn resource_key(
    context: &Wave5HostContext,
    binding: Option<&TypedResourceBinding>,
) -> Result<String, Wave5HostPortError> {
    let value = match binding {
        Some(binding) => json!({
            "owner": binding.owner_id,
            "kind": binding.resource_kind,
            "resource": binding.resource_id,
        }),
        None => json!({
            "owner": context.principal.principal_id,
            "session": context.agent_session_id,
            "module": context.capability_id,
        }),
    };
    digest_payload(&value)
        .map(|digest| digest.as_ref().to_owned())
        .map_err(|error| Wave5HostPortError::invalid_request(error.to_string()))
}

async fn begin_effect(
    store: &nomifun_agent_session::AgentSessionStore,
    context: &Wave5HostContext,
    binding: Option<&TypedResourceBinding>,
    input: &StrictJsonValue,
    strategy: nomifun_agent_session::EffectStrategy,
) -> Result<EffectAdmission, Wave5HostPortError> {
    let effect_id = effect_id(context)?;
    let input_digest = input_digest(context, binding, input)?;
    let resource_key = resource_key(context, binding)?;
    let read = || async {
        store
            .read_effect(&context.agent_session_id, &effect_id)
            .await
            .map_err(|error| Wave5HostPortError::unavailable(error.to_string()))
    };
    if let Some(record) = read().await? {
        return observe_effect(record, context, binding, &input_digest, strategy, &resource_key);
    }
    let causation = store
        .effect_causation_event_id(
            &context.agent_session_id,
            &context.turn_id,
            &context.operation_id,
            &context.capability_id,
            &context.action_id,
        )
        .await
        .map_err(|error| Wave5HostPortError::unavailable(error.to_string()))?;
    let request = nomifun_agent_session::EffectEventRequest {
        agent_session_id: context.agent_session_id.clone(),
        effect_id: effect_id.clone(),
        turn_id: context.turn_id.clone(),
        operation_id: context.operation_id.clone(),
        owner_domain: "wave5.product".to_owned(),
        capability_module: context.capability_id.clone(),
        action_id: context.action_id.clone(),
        resource_binding_id: binding.map(|binding| binding.binding_id.clone()),
        resource_key: Some(resource_key.clone()),
        input_digest: input_digest.clone(),
        recorded_at: nomifun_common::now_ms(),
        event_id: nomifun_agent_contracts::EventId::from(format!("{effect_id}:started")),
        producer_id: nomifun_agent_contracts::EventProducerId::from("capability_host"),
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
            "{effect_id}:lifecycle"
        )),
        correlation_id: nomifun_agent_contracts::CorrelationId::from(effect_id.clone()),
        strategy,
        causation_event_id: Some(causation),
        payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
            StrictJsonValue(json!({})),
        ),
    };
    match store.record_effect_started(request.clone()).await {
        Ok(_) => Ok(EffectAdmission::Reserved(EffectReservation {
            store: store.clone(),
            request,
        })),
        Err(error) => {
            if let Some(record) = read().await? {
                return observe_effect(
                    record,
                    context,
                    binding,
                    &input_digest,
                    strategy,
                    &resource_key,
                );
            }
            Err(Wave5HostPortError::unavailable(format!(
                "canonical Wave 5 effect admission failed: {error}"
            )))
        }
    }
}

fn observe_effect(
    record: nomifun_agent_session::AgentEffectRecord,
    context: &Wave5HostContext,
    binding: Option<&TypedResourceBinding>,
    input_digest: &DigestHex,
    strategy: nomifun_agent_session::EffectStrategy,
    resource_key: &str,
) -> Result<EffectAdmission, Wave5HostPortError> {
    if record.agent_session_id != context.agent_session_id
        || record.turn_id != context.turn_id
        || record.owner_domain != "wave5.product"
        || record.capability_module != context.capability_id
        || record.action_id != context.action_id
        || record.resource_binding_id.as_ref() != binding.map(|binding| &binding.binding_id)
        || record.resource_key.as_deref() != Some(resource_key)
        || record.input_digest != *input_digest
        || record.strategy != strategy
    {
        return Err(Wave5HostPortError::new(
            "IDEMPOTENCY_CONFLICT",
            "Wave 5 idempotency key was reused for different authority or input",
        ));
    }
    match record.state {
        nomifun_agent_session::AgentEffectState::Returned => {
            let result = record
                .bounded_observation
                .as_ref()
                .and_then(|value| value.get("result"))
                .cloned()
                .ok_or_else(|| {
                    Wave5HostPortError::unavailable(
                        "completed Wave 5 effect has no replayable bounded result",
                    )
                })?;
            Ok(EffectAdmission::Replay(StrictJsonValue(result)))
        }
        nomifun_agent_session::AgentEffectState::Rejected => {
            let error = record
                .bounded_observation
                .as_ref()
                .and_then(|value| value.get("error"));
            Err(Wave5HostPortError::new(
                error.and_then(|value| value.get("code")).and_then(Value::as_str)
                    .unwrap_or("WAVE5_OWNER_REJECTED"),
                error.and_then(|value| value.get("message")).and_then(Value::as_str)
                    .unwrap_or("Wave 5 owner rejected the prior invocation"),
            ))
        }
        nomifun_agent_session::AgentEffectState::Pending
        | nomifun_agent_session::AgentEffectState::Unknown
        | nomifun_agent_session::AgentEffectState::Cancelled => Err(
            Wave5HostPortError::new(
                WAVE5_EFFECT_OUTCOME_UNKNOWN,
                "prior Wave 5 effect is unsettled; inspect owner state before retrying",
            ),
        ),
    }
}

async fn finish_effect(
    reservation: &EffectReservation,
    completion: EffectCompletion<'_>,
) -> Result<(), Wave5HostPortError> {
    let (state, suffix, payload) = match completion {
        EffectCompletion::Succeeded(output) => (
            nomifun_agent_session::EffectTerminalState::Succeeded,
            "succeeded",
            json!({"result": output.0}),
        ),
        EffectCompletion::Failed(error) => (
            nomifun_agent_session::EffectTerminalState::Failed,
            "failed",
            json!({"error":{"code":error.code,"message":error.message.chars().take(2048).collect::<String>()}}),
        ),
        EffectCompletion::Uncertain(error) => (
            nomifun_agent_session::EffectTerminalState::Uncertain,
            "uncertain",
            json!({"outcome":"unknown","error":{"code":error.code,"message":error.message.chars().take(2048).collect::<String>()}}),
        ),
    };
    let bytes = canonical_json_bytes(&payload).unwrap_or_default();
    let payload = if bytes.len() <= 48 * 1024 {
        payload
    } else {
        json!({
            "observation_truncated": true,
            "digest": digest_payload(&payload).map(|value| value.as_ref().to_owned()).unwrap_or_default(),
        })
    };
    let mut request = reservation.request.clone();
    request.recorded_at = nomifun_common::now_ms();
    request.event_id = nomifun_agent_contracts::EventId::from(format!(
        "{}:{suffix}", request.effect_id
    ));
    request.producer_id = nomifun_agent_contracts::EventProducerId::from("owning_plugin");
    request.causation_event_id = Some(reservation.request.event_id.clone());
    request.payload = nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
        StrictJsonValue(payload),
    );
    reservation
        .store
        .record_effect_terminal(request, state)
        .await
        .map_err(|error| {
            Wave5HostPortError::unavailable(format!(
                "canonical Wave 5 effect terminal receipt failed: {error}"
            ))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const OTHER: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    #[test]
    fn requirements_actions_are_fenced_to_the_installation_owner() {
        assert!(ensure_installation_owner(OWNER, OWNER).is_ok());
        let error = ensure_installation_owner(OWNER, OTHER).unwrap_err();
        assert_eq!(
            error.code,
            nomifun_agent_contracts::RESOURCE_OWNER_MISMATCH
        );
    }

    #[test]
    fn requirement_mutation_receipt_is_compact_and_replayable_at_max_content() {
        let requirement = nomifun_api_types::Requirement {
            requirement_id: "0190f5fe-7c00-7a00-8000-000000000010".into(),
            display_no: 7,
            title: "large task".into(),
            content: "x".repeat(65_536),
            tag: "agent".into(),
            order_key: String::new(),
            status: RequirementStatus::Pending,
            completion_note: Some("y".repeat(65_536)),
            owner_conversation_id: None,
            owner_terminal_id: None,
            started_at: None,
            completed_at: None,
            attempt_count: 0,
            created_by: "agent".into(),
            created_at: 1,
            updated_at: 1,
            attachments: Vec::new(),
        };
        let receipt = requirement_receipt(&requirement);
        assert_eq!(receipt["requirement_id"], requirement.requirement_id);
        assert!(receipt.get("content").is_none());
        assert!(receipt.get("completion_note").is_none());
        assert!(canonical_json_bytes(&receipt).unwrap().len() < 48 * 1024);
    }
}
