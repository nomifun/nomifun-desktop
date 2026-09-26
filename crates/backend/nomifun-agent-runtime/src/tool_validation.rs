//! Model-facing argument preflight. Validate a whole proposed batch before
//! admitting any of its effects. This is not Kernel permission or owner proof.
use std::collections::BTreeMap;

use jsonschema::{Retrieve, Uri, Validator, ValidationError, error::ValidationErrorKind};
use nomifun_agent_contracts::DigestHex;
use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ToolCallId};
use serde_json::{Value, json};

use crate::{AgentEngineError, AgentToolPlan, AgentToolResult, input_schema_digest};

const MAX_CACHED_SCHEMAS: usize = 256;
const MAX_ISSUES_PER_CALL: usize = 8;

struct NoExternalSchemaReads;

impl Retrieve for NoExternalSchemaReads {
    fn retrieve(&self, _: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("tool schemas must carry their referenced definitions locally".into())
    }
}

#[derive(Default)]
pub(crate) struct ToolArgumentValidators {
    validators: BTreeMap<DigestHex, Validator>,
}

impl ToolArgumentValidators {
    pub(crate) fn reject_invalid_batch(
        &mut self,
        calls: &[ChatToolCall],
        plan: &AgentToolPlan,
        exposed: &[ChatToolDefinition],
    ) -> Result<Option<Vec<(ToolCallId, Result<AgentToolResult, AgentEngineError>)>>, AgentEngineError> {
        let mut failures = BTreeMap::new();
        for call in calls {
            // Engine controls have their own transactional state validation.
            // In particular a rejected completion replacement must still reach
            // its owner, which deliberately invalidates the prior report.
            let Some(binding) = plan.binding(&call.name) else { continue; };
            let definition = exposed.iter().find(|item| item.name == call.name).ok_or_else(|| {
                AgentEngineError::InvalidContract("argument preflight requires an exposed tool".into())
            })?;
            let digest = input_schema_digest(&definition.input_schema)?;
            if digest != binding.schema_digest {
                return Err(AgentEngineError::InvalidContract("model tool schema differs from the frozen binding".into()));
            }
            if !self.validators.contains_key(&digest) {
                let validator = jsonschema::options().with_retriever(NoExternalSchemaReads)
                    .build(&definition.input_schema.0).map_err(|_| {
                        // Schema errors may embed URI credentials or defaults.
                        AgentEngineError::InvalidContract("exposed tool input schema is invalid or has unresolved external references".into())
                    })?;
                if self.validators.len() >= MAX_CACHED_SCHEMAS {
                    // A cache limit is not a lifetime limit on valid tool use.
                    self.validators.clear();
                }
                self.validators.insert(digest.clone(), validator);
            }
            let mut issues = Vec::new();
            for error in self.validators[&digest].iter_errors(&call.arguments.0).take(MAX_ISSUES_PER_CALL) {
                collect_issues(&error, &definition.input_schema.0, 0, &mut issues);
            }
            if !issues.is_empty() {
                failures.insert(call.call_id.clone(), issues);
            }
        }
        if failures.is_empty() { return Ok(None); }
        Ok(Some(calls.iter().map(|call| {
            let issues = failures.get(&call.call_id);
            let result = AgentToolResult::text(call.call_id.clone(), json!({
                "status":"not_executed",
                "code":if issues.is_some() { "INVALID_TOOL_ARGUMENTS" } else { "BATCH_ARGUMENTS_REJECTED" },
                "tool":call.name,
                "issues":issues,
                "message":"No call in this batch was executed. Correct the arguments using the advertised schema, then propose the batch with fresh call IDs. For unions, field issues describe the closest candidate variant; the entire original schema still applies. Earlier batches are unchanged; do not repeat their effects. This parameter error alone does not require replanning.",
            }).to_string(), true);
            (call.call_id.clone(), Ok(result))
        }).collect()))
    }
}

fn bounded_schema_value(value: &Value) -> Option<Value> {
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 1024).then(|| value.clone())
}

fn collect_issues(error: &ValidationError<'_>, schema: &Value, depth: usize, issues: &mut Vec<Value>) {
    if issues.len() >= MAX_ISSUES_PER_CALL { return; }
    if depth < 8 {
        let contexts = match error.kind() {
            ValidationErrorKind::OneOfNotValid { context } | ValidationErrorKind::AnyOf { context } => Some(context),
            _ => None,
        };
        if let Some(contexts) = contexts {
            // Prefer a branch whose discriminator already matches. Showing
            // the planned variant's goal error to a parallel request encourages
            // mixing two mutually exclusive contracts and repeating failures.
            let closest = contexts.iter().min_by_key(|errors| (
                errors.iter().filter(|error| matches!(error.kind(),
                    ValidationErrorKind::Constant { .. } | ValidationErrorKind::Enum { .. })).count(),
                errors.len(),
            ));
            if let Some(errors) = closest.filter(|errors| !errors.is_empty()) {
                for error in errors.iter().take(MAX_ISSUES_PER_CALL) {
                    collect_issues(error, schema, depth + 1, issues);
                }
                return;
            }
        }
    }
    // Reflect schema-owned values only, never error Display, input values or
    // dynamic input property names (which can themselves contain secrets).
    let path = error.schema_path().to_string();
    let expected = schema.pointer(&path).and_then(bounded_schema_value);
    let mut issue = json!({"schema_path":path.chars().take(1024).collect::<String>(), "expected":expected});
    if let ValidationErrorKind::Required { property } = error.kind() {
        issue["missing_property"] = bounded_schema_value(property).unwrap_or(Value::Null);
    }
    if matches!(error.kind(), ValidationErrorKind::AdditionalProperties { .. }) {
        let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        if let Some(properties) = schema.pointer(parent).and_then(|value| value.get("properties")).and_then(Value::as_object) {
            let allowed = json!(properties.keys().take(32).collect::<Vec<_>>());
            issue["allowed_properties"] = bounded_schema_value(&allowed).unwrap_or(Value::Null);
        }
    }
    issues.push(issue);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::StrictJsonValue;
    use crate::{AgentEffectClass, AgentToolBinding};

    fn fixture(schema: Value) -> (AgentToolPlan, Vec<ChatToolDefinition>) {
        let definition = ChatToolDefinition {
            name: "write_file".into(), description: "fixture".into(),
            input_schema: StrictJsonValue(schema), deferred: false,
        };
        let binding = AgentToolBinding {
            model_name: definition.name.clone(), schema_digest: input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: "schema://fixture/input".into(), capability_contract_digest: "a".repeat(64).into(),
            definition: definition.clone(), capability_id: "workspace.files".into(), action_id: "workspace.files/write".into(),
            resource_binding_ids: Default::default(), effect_class: AgentEffectClass::ManagedEffect, parallel_safe: false,
        };
        (AgentToolPlan::new([binding]).unwrap(), vec![definition])
    }

    fn call(id: &str, arguments: Value) -> ChatToolCall {
        ChatToolCall { call_id: id.into(), name: "write_file".into(), arguments: StrictJsonValue(arguments), provider_metadata: None }
    }

    #[test]
    fn one_invalid_call_holds_every_effect_and_gives_schema_not_argument_values() {
        let (plan, exposed) = fixture(json!({"type":"object","additionalProperties":false,
            "required":["path"],"properties":{"path":{"type":"string"}}}));
        let mut validators = ToolArgumentValidators::default();
        let results = validators.reject_invalid_batch(&[
            call("valid", json!({"path":"a"})),
            call("invalid", json!({"path":42,"secret":"MUST_NOT_APPEAR_IN_FEEDBACK"})),
        ], &plan, &exposed).unwrap().unwrap();
        assert!(results.iter().all(|(_, result)| result.as_ref().unwrap().is_error));
        assert!(results[0].1.as_ref().unwrap().output_text().contains("BATCH_ARGUMENTS_REJECTED"));
        let error = results[1].1.as_ref().unwrap().output_text();
        assert!(error.contains("/properties/path/type"));
        assert!(error.contains("string"));
        assert!(!error.contains("MUST_NOT_APPEAR"));
        assert_eq!(validators.validators.len(), 1);
        assert!(validators.reject_invalid_batch(&[call("fixed", json!({"path":"b"}))], &plan, &exposed).unwrap().is_none());
        assert_eq!(validators.validators.len(), 1);
    }

    #[test]
    fn local_schema_references_work_and_external_references_never_read_io() {
        let (plan, exposed) = fixture(json!({"$defs":{"input":{"type":"object","required":["path"]}},"$ref":"#/$defs/input"}));
        let mut validators = ToolArgumentValidators::default();
        assert!(validators.reject_invalid_batch(&[call("valid", json!({"path":"a"}))], &plan, &exposed).unwrap().is_none());
        for reference in ["https://example.invalid/secret", "file:///private/schema.json"] {
            let (plan, exposed) = fixture(json!({"$ref":reference}));
            let error = validators.reject_invalid_batch(&[call("invalid", json!({}))], &plan, &exposed).unwrap_err();
            assert!(matches!(error, AgentEngineError::InvalidContract(_)));
            assert!(!error.to_string().contains(reference));
        }
    }

    #[test]
    fn union_feedback_identifies_the_matching_variant_and_actual_repair_fields() {
        let (plan, exposed) = fixture(json!({"type":"object", "oneOf":[
            {"type":"object","additionalProperties":false,"required":["strategy","goal"],
                "properties":{"strategy":{"const":"planned"},"goal":{"type":"string"}}},
            {"type":"object","additionalProperties":false,"required":["strategy","tasks"],
                "properties":{"strategy":{"const":"parallel"},"tasks":{"type":"array"},"synthesize":{"type":"boolean"}}}
        ]}));
        let mut validators = ToolArgumentValidators::default();
        let results = validators.reject_invalid_batch(&[call("wrong", json!({
            "strategy":"parallel","task":[],"synthesize":"false","private_key":"DO_NOT_REFLECT"
        }))], &plan, &exposed).unwrap().unwrap();
        let text = results[0].1.as_ref().unwrap().output_text();
        assert!(text.contains("/oneOf/1"));
        assert!(!text.contains("/oneOf/0"));
        assert!(text.contains("missing_property") && text.contains("tasks"));
        assert!(text.contains("boolean"));
        assert!(text.contains("allowed_properties"));
        assert!(!text.contains("DO_NOT_REFLECT") && !text.contains("private_key"));
        assert!(validators.reject_invalid_batch(&[call("fixed", json!({
            "strategy":"parallel","tasks":[],"synthesize":false
        }))], &plan, &exposed).unwrap().is_none());
    }

    #[test]
    fn schema_drift_is_a_host_error_not_a_model_correction_or_new_authority() {
        let (plan, mut exposed) = fixture(json!({"type":"object"}));
        exposed[0].input_schema = StrictJsonValue(json!({"type":"string"}));
        assert!(ToolArgumentValidators::default().reject_invalid_batch(&[call("call", json!({}))], &plan, &exposed).is_err());
    }
}
