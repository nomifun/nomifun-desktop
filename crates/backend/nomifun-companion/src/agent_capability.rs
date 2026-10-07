//! Target-generation Companion Agent actions and binding-derived persona.
//!
//! Persona is resolved from one exact `companion` resource selected by the
//! scene. The roster remains a product-management surface and is never copied
//! into an Agent grant. Only learn/evolve and explicit memory read/write are
//! callable actions.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{StrictJsonValue, TypedResourceBinding};
use nomifun_agent_domain_wave4::{
    COMPANION_MEMORY_RESOURCE_KIND, COMPANION_PERSONA,
    COMPANION_RESOURCE_KIND, Wave4CapabilityOperation,
    Wave4ContextHostPort, Wave4ContextHostRequest, Wave4HostPort,
    Wave4HostPortError, Wave4HostRequest, typed_resource_binding,
};
use nomifun_common::AppError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::CompanionService;

pub const COMPANION_MODULE_ID: &str = "companion";
pub const COMPANION_LEARN_ACTION_ID: &str = "companion/learn";
pub const COMPANION_EVOLVE_ACTION_ID: &str = "companion/evolve";
pub const COMPANION_AGENT_ACTION_IDS: [&str; 2] = [
    COMPANION_LEARN_ACTION_ID,
    COMPANION_EVOLVE_ACTION_ID,
];

pub const COMPANION_MEMORY_MODULE_ID: &str = "companion.memory";
pub const COMPANION_MEMORY_RECALL_ACTION_ID: &str = "companion.memory/recall";
pub const COMPANION_MEMORY_WRITE_ACTION_ID: &str = "companion.memory/write";
pub const COMPANION_MEMORY_ACTION_IDS: [&str; 2] = [
    COMPANION_MEMORY_RECALL_ACTION_ID,
    COMPANION_MEMORY_WRITE_ACTION_ID,
];

const COMPANION_OPERATION_FAILED: &str = "COMPANION_OPERATION_FAILED";
const COMPANION_MODEL_NOT_CONFIGURED: &str = "COMPANION_MODEL_NOT_CONFIGURED";

/// Exact resource requirement for one target Companion Action.
pub fn companion_action_resource_requirement(
    action_id: &str,
) -> Option<(&'static str, &'static str)> {
    match action_id {
        COMPANION_LEARN_ACTION_ID | COMPANION_EVOLVE_ACTION_ID => {
            Some((COMPANION_RESOURCE_KIND, "write"))
        }
        COMPANION_MEMORY_RECALL_ACTION_ID => {
            Some((COMPANION_MEMORY_RESOURCE_KIND, "read"))
        }
        COMPANION_MEMORY_WRITE_ACTION_ID => {
            Some((COMPANION_MEMORY_RESOURCE_KIND, "write"))
        }
        _ => None,
    }
}

pub fn companion_action_input_schema(action_id: &str) -> Option<Value> {
    match action_id {
        COMPANION_LEARN_ACTION_ID => Some(
            nomifun_agent_domain_wave4::action_input_schema(COMPANION_LEARN_ACTION_ID)
            .0,
        ),
        COMPANION_EVOLVE_ACTION_ID => Some(
            nomifun_agent_domain_wave4::action_input_schema(COMPANION_EVOLVE_ACTION_ID)
            .0,
        ),
        COMPANION_MEMORY_RECALL_ACTION_ID => Some(json!({
            "type": "object",
            "properties": {
                "per_kind": { "type": "integer", "minimum": 1, "maximum": 20 },
                "char_budget": { "type": "integer", "minimum": 1, "maximum": 65536 }
            },
            "additionalProperties": false
        })),
        COMPANION_MEMORY_WRITE_ACTION_ID => Some(json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "minLength": 1, "maxLength": 64 },
                "content": { "type": "string", "minLength": 1, "maxLength": 65536 },
                "tags": {
                    "type": "array",
                    "maxItems": 64,
                    "items": { "type": "string", "minLength": 1, "maxLength": 128 }
                }
            },
            "required": ["kind", "content"],
            "additionalProperties": false
        })),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CompanionPersonaContext {
    pub kind: String,
    pub companion_id: String,
    pub system_prompt: String,
}

/// Domain owner for Companion actions and scene-derived persona Context.
pub struct CompanionAgentCapabilityOwner {
    authoritative_owner_id: Arc<str>,
    service: Arc<CompanionService>,
}

impl CompanionAgentCapabilityOwner {
    pub fn new(
        authoritative_owner_id: impl Into<Arc<str>>,
        service: Arc<CompanionService>,
    ) -> Self {
        Self {
            authoritative_owner_id: authoritative_owner_id.into(),
            service,
        }
    }

    pub fn service(&self) -> &Arc<CompanionService> {
        &self.service
    }

    /// Resolve a selected Companion scene into server-authored resources.
    /// Persona read authority is derived unconditionally from the scene;
    /// learn/evolve and memory operations come only from exact Action IDs.
    pub async fn resolve_scene_bindings(
        &self,
        principal_id: &str,
        companion_id: &str,
        allowed_action_ids: &BTreeSet<String>,
    ) -> Result<Vec<TypedResourceBinding>, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        let companion = self
            .service
            .get_companion(companion_id)
            .await
            .map_err(map_service_error)?;

        let mut companion_operations = BTreeSet::from(["read".to_owned()]);
        let mut memory_operations = BTreeSet::new();
        for action_id in allowed_action_ids {
            let (resource_kind, operation) =
                companion_action_resource_requirement(action_id).ok_or_else(|| {
                    Wave4HostPortError::resource_binding_invalid(format!(
                        "Companion scene cannot derive authority from undeclared Action {action_id}",
                    ))
                })?;
            match resource_kind {
                COMPANION_RESOURCE_KIND => {
                    companion_operations.insert(operation.to_owned());
                }
                COMPANION_MEMORY_RESOURCE_KIND => {
                    memory_operations.insert(operation.to_owned());
                }
                _ => unreachable!("canonical Companion resource requirement"),
            }
        }

        let mut bindings = vec![typed_resource_binding(
            format!("companion:{}", companion.companion_id),
            COMPANION_RESOURCE_KIND,
            companion.companion_id.clone(),
            principal_id.to_owned(),
            companion_operations,
        )];
        if !memory_operations.is_empty() {
            bindings.push(typed_resource_binding(
                format!("companion-memory:{}", companion.companion_id),
                COMPANION_MEMORY_RESOURCE_KIND,
                companion.companion_id,
                principal_id.to_owned(),
                memory_operations,
            ));
        }
        Ok(bindings)
    }

    /// Derive only the selected Companion's persona. The install-wide roster is
    /// deliberately not model Context and cannot be requested as an Action.
    pub async fn persona_context(
        &self,
        principal_id: &str,
        bindings: &[TypedResourceBinding],
    ) -> Result<CompanionPersonaContext, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        let binding = exact_binding(
            bindings,
            principal_id,
            COMPANION_RESOURCE_KIND,
            "read",
        )?;
        let platform = binding
            .typed_parameters
            .get("channel_platform")
            .map(String::as_str);
        let prompt = self
            .service
            .build_bound_system_prompt(binding.resource_id.as_ref(), platform)
            .await
            .map_err(map_service_error)?;
        if prompt.trim().is_empty() || prompt.chars().count() > 65_536 {
            return Err(Wave4HostPortError::new(
                COMPANION_OPERATION_FAILED,
                "bound companion persona is empty or exceeds the 65536-character Context limit",
            ));
        }
        Ok(CompanionPersonaContext {
            kind: "companion_persona".to_owned(),
            companion_id: binding.resource_id.as_ref().to_owned(),
            system_prompt: prompt,
        })
    }

    /// Invoke one target Module Action. The returned durable run/memory ID is
    /// the domain receipt; the canonical Agent effect ledger owns replay and
    /// marks a dropped in-flight future unknown.
    pub async fn invoke_target_action(
        &self,
        principal_id: &str,
        action_id: &str,
        bindings: &[TypedResourceBinding],
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        if !input.0.is_object() {
            return Err(Wave4HostPortError::invalid_request(
                "Companion Action input must be an object",
            ));
        }
        let (resource_kind, operation) = companion_action_resource_requirement(action_id)
            .ok_or_else(|| {
                Wave4HostPortError::action_operation_mismatch(format!(
                    "undeclared Companion Action {action_id}",
                ))
            })?;
        let binding = exact_binding(bindings, principal_id, resource_kind, operation)?;
        self.service
            .get_companion(binding.resource_id.as_ref())
            .await
            .map_err(map_service_error)?;

        match action_id {
            COMPANION_LEARN_ACTION_ID => {
                validate_optional_reason(&input)?;
                let result = self
                    .service
                    .run_learn_now(binding.resource_id.as_ref())
                    .await
                    .map_err(map_service_error)?;
                validate_run_status(&result.status, result.error.as_deref())?;
                serde_json::to_value(result)
                    .map(StrictJsonValue)
                    .map_err(|error| {
                        Wave4HostPortError::new(COMPANION_OPERATION_FAILED, error.to_string())
                    })
            }
            COMPANION_EVOLVE_ACTION_ID => {
                validate_optional_reason(&input)?;
                let result = self
                    .service
                    .run_evolve_now(binding.resource_id.as_ref())
                    .await
                    .map_err(map_service_error)?;
                validate_run_status(&result.status, result.error.as_deref())?;
                Ok(StrictJsonValue(json!({
                    "evolve_run_id": result.evolve_run_id,
                    "started_at": result.started_at,
                    "finished_at": result.finished_at,
                    "status": result.status,
                    "events_processed": result.events_processed,
                    "patterns_found": result.patterns_found,
                    "drafts_created": result.drafts_created,
                    "error": result.error,
                })))
            }
            COMPANION_MEMORY_RECALL_ACTION_ID => {
                let input: MemoryRecallInput = parse_input(input.0)?;
                let memories = self
                    .service
                    .recall_memories_for_agent(
                        binding.resource_id.as_ref(),
                        input.per_kind.unwrap_or(8),
                        input.char_budget.unwrap_or(32 * 1024),
                    )
                    .await
                    .map_err(map_service_error)?;
                Ok(StrictJsonValue(json!({
                    "companion_id": binding.resource_id,
                    "memories": memories,
                })))
            }
            COMPANION_MEMORY_WRITE_ACTION_ID => {
                let input: MemoryWriteInput = parse_input(input.0)?;
                validate_memory_write_input(&input)?;
                let memory = self
                    .service
                    .add_memory(
                        &input.kind,
                        &input.content,
                        &input.tags,
                        Some(binding.resource_id.as_ref()),
                    )
                    .await
                    .map_err(map_service_error)?;
                Ok(StrictJsonValue(json!({
                    "companion_id": binding.resource_id,
                    "receipt_id": memory.memory_id.clone(),
                    "memory": memory,
                })))
            }
            _ => unreachable!("validated Companion Action"),
        }
    }

    fn require_owner(&self, principal_id: &str) -> Result<(), Wave4HostPortError> {
        if principal_id != self.authoritative_owner_id.as_ref() {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "companion resource belongs to another installation owner",
            ));
        }
        Ok(())
    }

    async fn invoke_wave4(
        &self,
        request: Wave4HostRequest,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        request.validate()?;
        if request.context.principal.principal_kind != "user" {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "Companion Actions require a user principal",
            ));
        }
        self.require_owner(&request.context.principal.principal_id)?;
        let (action_id, input) = match request.operation {
            Wave4CapabilityOperation::CompanionLearn { input } => {
                (COMPANION_LEARN_ACTION_ID, input)
            }
            Wave4CapabilityOperation::CompanionEvolve { input } => {
                (COMPANION_EVOLVE_ACTION_ID, input)
            }
            Wave4CapabilityOperation::CompanionMemoryRecall { input } => {
                (COMPANION_MEMORY_RECALL_ACTION_ID, input)
            }
            Wave4CapabilityOperation::CompanionMemoryWrite { input } => {
                (COMPANION_MEMORY_WRITE_ACTION_ID, input)
            }
            _ => {
                return Err(Wave4HostPortError::action_operation_mismatch(
                    "Companion owner received a foreign operation",
                ));
            }
        };
        self.invoke_target_action(
            &request.context.principal.principal_id,
            action_id,
            &request.context.resource_bindings,
            input,
        )
        .await
    }
}

impl Wave4HostPort for CompanionAgentCapabilityOwner {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move { self.invoke_wave4(request).await })
    }
}

/// Transitional Wave4 adapter. It intentionally supports persona only. A
/// roster request fails closed because roster is product administration, not
/// Agent Context.
impl Wave4ContextHostPort for CompanionAgentCapabilityOwner {
    fn contribute<'a>(
        &'a self,
        request: Wave4ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<StrictJsonValue>, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            request.validate()?;
            if request.capability_id.as_ref() != COMPANION_PERSONA {
                return Err(Wave4HostPortError::action_operation_mismatch(
                    "Companion roster is not an Agent Context contribution",
                ));
            }
            if request.principal.principal_kind != "user" {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "Companion persona requires a user principal",
                ));
            }
            let context = self
                .persona_context(
                    &request.principal.principal_id,
                    &request.resource_bindings,
                )
                .await?;
            serde_json::to_value(context)
                .map(StrictJsonValue)
                .map(Some)
                .map_err(|error| {
                    Wave4HostPortError::new(COMPANION_OPERATION_FAILED, error.to_string())
                })
        })
    }
}

fn exact_binding<'a>(
    bindings: &'a [TypedResourceBinding],
    principal_id: &str,
    resource_kind: &str,
    operation: &str,
) -> Result<&'a TypedResourceBinding, Wave4HostPortError> {
    let mut matches = bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == resource_kind);
    let binding = matches.next().ok_or_else(|| {
        Wave4HostPortError::resource_not_bound(format!(
            "Companion Action requires resource kind {resource_kind}",
        ))
    })?;
    if matches.next().is_some() {
        return Err(Wave4HostPortError::resource_binding_invalid(format!(
            "Companion Action received duplicate resource kind {resource_kind}",
        )));
    }
    if binding.owner_id != principal_id {
        return Err(Wave4HostPortError::resource_owner_mismatch(format!(
            "Companion resource {} belongs to {}, not {principal_id}",
            binding.binding_id.as_ref(),
            binding.owner_id,
        )));
    }
    let allowed_parameters = binding.resource_kind.as_ref() == COMPANION_RESOURCE_KIND
        && binding
            .typed_parameters
            .keys()
            .all(|key| key == "channel_platform");
    if binding.resource_id.as_ref().trim().is_empty()
        || binding.connection_config_ref.is_some()
        || (!binding.typed_parameters.is_empty() && !allowed_parameters)
        || binding
            .operations
            .iter()
            .any(|operation| !matches!(operation.as_str(), "read" | "write"))
        || !binding.operations.contains(operation)
    {
        return Err(Wave4HostPortError::resource_not_bound(format!(
            "Companion resource {resource_kind} does not grant operation {operation}",
        )));
    }
    Ok(binding)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompanionRunInput {
    #[serde(default)]
    reason: Option<String>,
}

fn validate_optional_reason(input: &StrictJsonValue) -> Result<(), Wave4HostPortError> {
    let parsed: CompanionRunInput = parse_input(input.0.clone())?;
    if parsed
        .reason
        .as_deref()
        .is_some_and(|reason| reason.trim().is_empty() || reason.chars().count() > 512)
    {
        return Err(Wave4HostPortError::invalid_request(
            "reason must be non-empty and at most 512 characters when supplied",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRecallInput {
    #[serde(default)]
    per_kind: Option<i64>,
    #[serde(default)]
    char_budget: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryWriteInput {
    kind: String,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
}

fn validate_memory_write_input(input: &MemoryWriteInput) -> Result<(), Wave4HostPortError> {
    if input.kind.trim().is_empty()
        || input.kind.chars().count() > 64
        || input.content.trim().is_empty()
        || input.content.chars().count() > 65_536
        || input.tags.len() > 64
        || input
            .tags
            .iter()
            .any(|tag| tag.trim().is_empty() || tag.chars().count() > 128)
    {
        return Err(Wave4HostPortError::invalid_request(
            "companion.memory/write input exceeds its bounded schema",
        ));
    }
    Ok(())
}

fn parse_input<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, Wave4HostPortError> {
    serde_json::from_value(value)
        .map_err(|error| Wave4HostPortError::invalid_request(error.to_string()))
}

fn validate_run_status(
    status: &str,
    error: Option<&str>,
) -> Result<(), Wave4HostPortError> {
    match status {
        "error" => Err(Wave4HostPortError::new(
            COMPANION_OPERATION_FAILED,
            error.unwrap_or("Companion run failed without a diagnostic"),
        )),
        "model_unconfigured" => Err(Wave4HostPortError::new(
            COMPANION_MODEL_NOT_CONFIGURED,
            "the bound Companion has no required model configured",
        )),
        _ => Ok(()),
    }
}

fn map_service_error(error: AppError) -> Wave4HostPortError {
    let code = match &error {
        AppError::NotFound(_) => "RESOURCE_NOT_FOUND",
        AppError::ProviderUnavailable(_) => COMPANION_MODEL_NOT_CONFIGURED,
        AppError::Conflict(_) | AppError::RevisionConflict(_) => "COMPANION_OPERATION_BUSY",
        _ => COMPANION_OPERATION_FAILED,
    };
    Wave4HostPortError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_actions_have_exact_resource_requirements_and_no_context_grants() {
        assert_eq!(
            companion_action_resource_requirement(COMPANION_LEARN_ACTION_ID),
            Some((COMPANION_RESOURCE_KIND, "write")),
        );
        assert_eq!(
            companion_action_resource_requirement(COMPANION_MEMORY_RECALL_ACTION_ID),
            Some((COMPANION_MEMORY_RESOURCE_KIND, "read")),
        );
        assert_eq!(
            companion_action_resource_requirement(COMPANION_MEMORY_WRITE_ACTION_ID),
            Some((COMPANION_MEMORY_RESOURCE_KIND, "write")),
        );
        for retired_context in [
            nomifun_agent_domain_wave4::COMPANION_PERSONA,
            nomifun_agent_domain_wave4::COMPANION_ROSTER,
        ] {
            assert_eq!(companion_action_resource_requirement(retired_context), None);
        }
    }

    #[test]
    fn action_inputs_are_strict_and_bounded() {
        assert!(validate_optional_reason(&StrictJsonValue(json!({}))).is_ok());
        assert!(
            validate_optional_reason(&StrictJsonValue(json!({ "reason": "because" }))).is_ok()
        );
        assert!(
            validate_optional_reason(&StrictJsonValue(json!({ "reason": "" }))).is_err()
        );
        assert!(
            validate_optional_reason(&StrictJsonValue(json!({ "unknown": true }))).is_err()
        );
    }
}
