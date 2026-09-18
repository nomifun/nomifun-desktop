use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CanonicalSchemaRef, CapabilityId, CorrelationId, DigestHex,
    IdempotencyKey, OperationId, PrincipalRef, ResolvedSnapshotRef, ResourceBindingId,
    StrictJsonValue, digest_payload,
};
use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ChatToolResultPart, ToolCallId};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::EngineToolError;

const MAX_TOOL_ARGUMENT_BYTES: usize = 512 * 1024;
const MAX_TOOL_RESULT_BYTES: usize = 4 * 1024 * 1024;
const TOOL_RESULT_TRUNCATION_MARKER: &str = "\n[output truncated]";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineEffectClass {
    ReadOnly,
    ManagedEffect,
    ExternalUncertainEffect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineToolBinding {
    pub model_name: String,
    pub definition: ChatToolDefinition,
    pub schema_digest: DigestHex,
    pub canonical_input_schema_ref: CanonicalSchemaRef,
    pub capability_contract_digest: DigestHex,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub resource_binding_ids: BTreeSet<ResourceBindingId>,
    pub effect_class: EngineEffectClass,
    pub parallel_safe: bool,
}

impl EngineToolBinding {
    pub fn validate(&self) -> Result<(), EngineToolError> {
        if self.model_name.trim().is_empty()
            || self.model_name.trim() != self.model_name
            || self.definition.name.trim().is_empty()
            || self.definition.name.trim() != self.definition.name
        {
            return Err(EngineToolError::InvalidContract(
                "engine tool names must not be empty".to_owned(),
            ));
        }
        if self.model_name != self.definition.name {
            return Err(EngineToolError::InvalidContract(format!(
                "model name {:?} differs from definition name {:?}",
                self.model_name, self.definition.name
            )));
        }
        let expected_schema_digest = input_schema_digest(&self.definition.input_schema)?;
        if self.schema_digest != expected_schema_digest {
            return Err(EngineToolError::ToolSchemaDigestMismatch {
                tool_name: self.model_name.clone(),
                expected: expected_schema_digest.0,
                actual: self.schema_digest.0.clone(),
            });
        }
        if self.canonical_input_schema_ref.as_ref().trim().is_empty()
            || !is_digest(&self.capability_contract_digest)
        {
            return Err(EngineToolError::InvalidContract(format!(
                "tool {} has an invalid canonical schema or capability digest",
                self.model_name
            )));
        }
        if self.capability_id.as_ref().trim().is_empty()
            || self.action_id.as_ref().trim().is_empty()
        {
            return Err(EngineToolError::InvalidContract(format!(
                "tool {} has an empty canonical capability/action",
                self.model_name
            )));
        }
        if self
            .resource_binding_ids
            .iter()
            .any(|binding_id| binding_id.as_ref().trim().is_empty())
        {
            return Err(EngineToolError::InvalidContract(format!(
                "tool {} contains an empty resource binding id",
                self.model_name
            )));
        }
        if self.parallel_safe && !matches!(self.effect_class, EngineEffectClass::ReadOnly) {
            return Err(EngineToolError::InvalidContract(format!(
                "effectful tool {} cannot be marked parallel-safe",
                self.model_name
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineToolPlan {
    bindings: BTreeMap<String, EngineToolBinding>,
}

impl EngineToolPlan {
    pub fn new(
        bindings: impl IntoIterator<Item = EngineToolBinding>,
    ) -> Result<Self, EngineToolError> {
        let mut plan = Self::default();
        for binding in bindings {
            binding.validate()?;
            if plan
                .bindings
                .insert(binding.model_name.clone(), binding)
                .is_some()
            {
                return Err(EngineToolError::InvalidContract(
                    "engine tool plan contains duplicate model tool names".to_owned(),
                ));
            }
        }
        Ok(plan)
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Combine independently host-admitted surfaces without name overrides.
    pub fn merged(&self, other: &Self) -> Result<Self, EngineToolError> {
        Self::new(
            self.bindings
                .values()
                .chain(other.bindings.values())
                .cloned(),
        )
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn binding(&self, model_name: &str) -> Option<&EngineToolBinding> {
        self.bindings.get(model_name)
    }

    pub fn model_definitions(&self) -> Vec<ChatToolDefinition> {
        self.bindings
            .values()
            .map(|binding| binding.definition.clone())
            .collect()
    }

    /// Projection only; the platform invoker independently checks the live generation.
    pub fn for_active_capabilities(&self, active: &BTreeSet<CapabilityId>) -> Self {
        Self {
            bindings: self
                .bindings
                .iter()
                .filter(|(_, binding)| active.contains(&binding.capability_id))
                .map(|(name, binding)| (name.clone(), binding.clone()))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineToolInvocation {
    pub agent_session_id: AgentSessionId,
    pub principal: PrincipalRef,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub active_set_generation: u64,
    pub turn_operation_id: OperationId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub call: ChatToolCall,
    pub binding: EngineToolBinding,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineToolResult {
    pub call_id: ToolCallId,
    pub output: Vec<ChatToolResultPart>,
    pub is_error: bool,
}

impl EngineToolResult {
    pub fn text(call_id: ToolCallId, text: impl Into<String>, is_error: bool) -> Self {
        let text = bounded_text(text.into());
        Self {
            call_id,
            output: vec![ChatToolResultPart::Text { text }],
            is_error,
        }
    }

    pub fn output_text(&self) -> String {
        self.output
            .iter()
            .filter_map(|part| match part {
                ChatToolResultPart::Text { text } => Some(text.as_str()),
                ChatToolResultPart::Image { .. } | ChatToolResultPart::Audio { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn validate_for(&self, expected: &ToolCallId) -> Result<(), EngineToolError> {
        if &self.call_id != expected {
            return Err(EngineToolError::ToolInvocation(format!(
                "tool result call id {} does not match {}",
                self.call_id.as_ref(),
                expected.as_ref()
            )));
        }
        if self.output.is_empty() {
            return Err(EngineToolError::ToolInvocation(
                "tool result output must not be empty".to_owned(),
            ));
        }
        for part in &self.output {
            match part {
                ChatToolResultPart::Text { text } if text.is_empty() => {
                    return Err(EngineToolError::ToolInvocation(
                        "tool result text must not be empty".to_owned(),
                    ));
                }
                ChatToolResultPart::Image {
                    media_type,
                    data_base64,
                }
                | ChatToolResultPart::Audio {
                    media_type,
                    data_base64,
                } if media_type.is_empty() || data_base64.is_empty() => {
                    return Err(EngineToolError::ToolInvocation(
                        "tool result media must include a type and payload".to_owned(),
                    ));
                }
                ChatToolResultPart::Text { .. }
                | ChatToolResultPart::Image { .. }
                | ChatToolResultPart::Audio { .. } => {}
            }
        }
        let serialized_size = serde_json::to_vec(&self.output)
            .map_err(|error| EngineToolError::ToolInvocation(error.to_string()))?
            .len();
        if serialized_size > MAX_TOOL_RESULT_BYTES {
            return Err(EngineToolError::ToolResultTooLarge {
                limit: MAX_TOOL_RESULT_BYTES,
            });
        }
        Ok(())
    }
}

#[async_trait]
pub trait EngineToolInvoker: Send + Sync {
    /// Run only inside a platform-owned task. Cancellation can reject before
    /// dispatch; dropping this future is not proof an effect was cancelled.
    /// The host must retain it and persist its result through caller teardown.
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError>;
}

pub fn validate_tool_argument_size(arguments: &str) -> Result<(), EngineToolError> {
    if arguments.len() > MAX_TOOL_ARGUMENT_BYTES {
        return Err(EngineToolError::ToolArgumentsTooLarge {
            limit: MAX_TOOL_ARGUMENT_BYTES,
        });
    }
    Ok(())
}

pub fn parse_completed_arguments(call: &ChatToolCall) -> Result<String, EngineToolError> {
    let json = serde_json::to_string(&call.arguments).map_err(|error| {
        EngineToolError::InvalidModelEvent(format!(
            "tool {} arguments could not be serialized: {error}",
            call.call_id.as_ref()
        ))
    })?;
    validate_tool_argument_size(&json)?;
    Ok(json)
}

pub fn input_schema_digest(schema: &StrictJsonValue) -> Result<DigestHex, EngineToolError> {
    digest_payload(schema)
        .map_err(|error| EngineToolError::InvalidContract(format!("tool schema digest: {error}")))
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

fn bounded_text(mut text: String) -> String {
    if serialized_text_size(&text) <= MAX_TOOL_RESULT_BYTES {
        return text;
    }

    let prefix_budget = MAX_TOOL_RESULT_BYTES.saturating_sub(TOOL_RESULT_TRUNCATION_MARKER.len());
    truncate_utf8(&mut text, prefix_budget);

    loop {
        let mut candidate = text.clone();
        candidate.push_str(TOOL_RESULT_TRUNCATION_MARKER);
        let serialized_size = serialized_text_size(&candidate);
        if serialized_size <= MAX_TOOL_RESULT_BYTES {
            return candidate;
        }

        let excess = serialized_size - MAX_TOOL_RESULT_BYTES;
        let next_length = text.len().saturating_sub(excess.max(1));
        if next_length == text.len() {
            text.clear();
        } else {
            truncate_utf8(&mut text, next_length);
        }
    }
}

fn serialized_text_size(text: &str) -> usize {
    serde_json::to_vec(&vec![ChatToolResultPart::Text {
        text: text.to_owned(),
    }])
    .map(|bytes| bytes.len())
    .unwrap_or(usize::MAX)
}

fn is_digest(value: &DigestHex) -> bool {
    value.as_ref().len() == 64 && value.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::StrictJsonValue;
    use nomifun_chat_model_broker::ChatToolDefinition;
    use serde_json::json;

    fn definition() -> ChatToolDefinition {
        ChatToolDefinition {
            name: "read_file".to_owned(),
            description: "read a file".to_owned(),
            input_schema: StrictJsonValue(json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            })),
            deferred: false,
        }
    }

    fn binding_with_digest(schema_digest: DigestHex) -> EngineToolBinding {
        EngineToolBinding {
            model_name: "read_file".to_owned(),
            definition: definition(),
            schema_digest,
            canonical_input_schema_ref: CanonicalSchemaRef::from("schema://workspace.files/read/input"),
            capability_contract_digest: DigestHex::from("b".repeat(64)),
            capability_id: CapabilityId::from("workspace.files"),
            action_id: ActionId::from("workspace.files/read"),
            resource_binding_ids: BTreeSet::new(),
            effect_class: EngineEffectClass::ReadOnly,
            parallel_safe: true,
        }
    }

    #[test]
    fn schema_digest_is_part_of_tool_admission() {
        let definition = definition();
        let digest = input_schema_digest(&definition.input_schema).unwrap();
        assert!(binding_with_digest(digest).validate().is_ok());
        assert!(matches!(
            binding_with_digest(DigestHex::from("a".repeat(64))).validate(),
            Err(EngineToolError::ToolSchemaDigestMismatch { .. })
        ));
    }

    #[test]
    fn text_result_is_bounded_without_invalid_utf8() {
        let result = EngineToolResult::text(
            ToolCallId::from("call-1"),
            "界".repeat(MAX_TOOL_RESULT_BYTES),
            false,
        );
        assert!(result.validate_for(&ToolCallId::from("call-1")).is_ok());
        assert!(
            result
                .output_text()
                .ends_with(TOOL_RESULT_TRUNCATION_MARKER)
        );
    }

    #[test]
    fn mismatched_tool_result_id_is_rejected() {
        let result = EngineToolResult::text(ToolCallId::from("actual"), "ok", false);
        assert!(matches!(
            result.validate_for(&ToolCallId::from("expected")),
            Err(EngineToolError::ToolInvocation(_))
        ));
    }
}
