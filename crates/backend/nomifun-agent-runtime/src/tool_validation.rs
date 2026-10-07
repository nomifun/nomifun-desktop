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

pub(crate) struct NoExternalSchemaReads;

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
            let definition = exposed.iter().find(|item| item.name == call.name).ok_or_else(|| {
                AgentEngineError::InvalidContract("argument preflight requires an exposed tool".into())
            })?;
            let digest = input_schema_digest(&definition.input_schema)?;
            if let Some(binding) = plan.binding(&call.name).filter(|binding| digest != binding.schema_digest) {
                return Err(AgentEngineError::ToolSchemaDigestMismatch {
                    tool_name: binding.model_name.clone(),
                    expected: binding.schema_digest.as_ref().to_owned(),
                    actual: digest.as_ref().to_owned(),
                });
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
        // The issue values belong to the schema before this refusal. Every
        // paired result below is unsuccessful and will be counted once; give
        // account repair the following totals, without admitting any effect.
        let correction_counters = exposed.iter().find(|tool|tool.name == crate::completion::TOOL_NAME)
            .and_then(|tool| {
                let properties = &tool.input_schema.0["properties"];
                let tools = u32::try_from(properties["observed_tool_error_count"]["const"].as_u64()?).ok()?;
                let commands = u32::try_from(properties["observed_command_failure_count"]["const"].as_u64()?).ok()?;
                Some(json!({"observed_tool_error_count":tools.saturating_add(u32::try_from(calls.len()).unwrap_or(u32::MAX)),
                    "observed_command_failure_count":commands}))
            });
        Ok(Some(calls.iter().map(|call| {
            let issues = failures.get(&call.call_id);
            let message = if calls.len() == 1 && issues.is_some() && call.name == crate::completion::TOOL_NAME {
                "No call in this batch was executed. Repair only the completion arguments using the current advertised schema and a fresh call ID; do not rerun settled commands or tests, reset their plan steps, or infer that earlier work disappeared. Counter issue expected values already include this rejected batch; use them (equal to correction_counters_after_rejected_batch) unchanged in the next report. An intentionally nonzero test still counts. For an ineligible read/search/Git ID, describe the earlier observation separately and use unverified with no evidence for current-state claims; never substitute an unrelated eligible ID. Do not repeat observations solely to repair this account unless the user explicitly requires fresh verification. This parameter error alone does not require replanning."
            } else {
                "No call in this batch was executed. Correct the arguments using the advertised schema, then propose the batch with fresh call IDs. For unions, field issues describe the closest candidate variant; the entire original schema still applies. Earlier batches are unchanged; do not repeat their effects. This parameter error alone does not require replanning."
            };
            let mut payload = json!({
                "status":"not_executed",
                "code":if issues.is_some() { "INVALID_TOOL_ARGUMENTS" } else { "BATCH_ARGUMENTS_REJECTED" },
                "tool":call.name,
                "issues":issues,
                "message":message,
            });
            // An optional parameter supplied as "" or null is the model
            // emitting a field it does not use; name the parameters to drop
            // so the repair is omission, not guessing a value.
            if let (Some(issues), Some(definition)) =
                (issues, exposed.iter().find(|tool| tool.name == call.name))
            {
                let required = &definition.input_schema.0["required"];
                let is_required = |name: &str| required.as_array().is_some_and(|list|
                    list.iter().any(|entry| entry.as_str() == Some(name)));
                let mut omittable = Vec::new();
                for issue in issues.iter().filter_map(|issue| {
                    let path = issue["parameter_path_template"].as_str()?;
                    let name = path.strip_prefix('/')?;
                    (!name.is_empty() && !name.contains('/')).then_some((path, name))
                }) {
                    let (path, name) = issue;
                    if is_required(name) || omittable.iter().any(|seen| seen == path) { continue; }
                    let supplied = &call.arguments.0[name];
                    let failed_empty = supplied.as_str() == Some("")
                        && issues.iter().any(|issue| issue["parameter_path_template"].as_str() == Some(path)
                            && matches!(issue["schema_path"].as_str(), Some(p) if p.ends_with("/minLength") || p.ends_with("/pattern")));
                    let failed_null = supplied.is_null()
                        && issues.iter().any(|issue| issue["parameter_path_template"].as_str() == Some(path)
                            && matches!(issue["schema_path"].as_str(), Some(p) if p.ends_with("/type")));
                    if failed_empty || failed_null { omittable.push(path.to_owned()); }
                }
                if !omittable.is_empty() {
                    payload["omit_optional_parameters"] = json!(omittable);
                    payload["message"] = json!(format!(
                        "{} Omit optional parameters you do not use; never send empty strings or null for them.",
                        payload["message"].as_str().unwrap_or_default()));
                }
            }
            if let Some(counters) = &correction_counters {
                payload["correction_counters_after_rejected_batch"] = counters.clone();
            }
            // The advertised counter const/enum constants predate this
            // rejected batch; the rejection itself is counted, so a copied
            // `expected` would still be stale. Rewrite them to the corrected
            // totals carried beside them.
            if calls.len() == 1 && call.name == crate::completion::TOOL_NAME
                && let Some(counters) = &correction_counters
                && let Some(items) = payload["issues"].as_array_mut()
            {
                for issue in items {
                    let Some(path) = issue["schema_path"].as_str() else { continue };
                    let corrected = if path.ends_with("/observed_tool_error_count/const")
                        || path.ends_with("/observed_tool_error_count/enum") {
                        Some(counters["observed_tool_error_count"].clone())
                    } else if path.ends_with("/observed_command_failure_count/const")
                        || path.ends_with("/observed_command_failure_count/enum") {
                        Some(counters["observed_command_failure_count"].clone())
                    } else { None };
                    if let Some(value) = corrected { issue["expected"] = value; }
                }
            }
            if calls.len() == 1 && call.name == crate::completion::TOOL_NAME && issues.is_some()
                && let Some(definition) = exposed.iter().find(|tool| tool.name == call.name)
                && let Some(criteria) = call.arguments.0["criteria"].as_array()
            {
                let references = &definition.input_schema.0["properties"]["criteria"]["items"]["properties"]["evidence_call_ids"];
                let allowed = references["items"]["enum"].as_array();
                let none_allowed = references["maxItems"].as_u64() == Some(0);
                let affected = criteria.iter().take(16).enumerate().filter_map(|(index, criterion)| {
                    let ids = criterion["evidence_call_ids"].as_array()?;
                    ids.iter().take(8).any(|id| none_allowed || allowed.is_some_and(|values| !values.contains(id)))
                        .then_some(index)
                }).collect::<Vec<_>>();
                if !affected.is_empty() {
                    // Enum membership does not prove a claim's scope. Reflect
                    // bounded criterion positions, never raw argument values.
                    // Other eligible IDs are not replacement suggestions.
                    if let Some(items) = payload["issues"].as_array_mut() {
                        for issue in items {
                            if issue["schema_path"].as_str().is_some_and(|path|
                                path.ends_with("/properties/evidence_call_ids/items/enum")) {
                                issue.as_object_mut().expect("schema issue is an object").remove("expected");
                            }
                        }
                    }
                    payload["ineligible_evidence_criteria"] = json!(affected);
                    payload["ineligible_evidence_criterion_numbers"] = json!(affected.iter().map(|index| index + 1).collect::<Vec<_>>());
                    payload["ineligible_evidence_criterion_paths"] = json!(affected.iter().map(|index| format!("/criteria/{index}")).collect::<Vec<_>>());
                    let numbered_delivery=definition.input_schema.0["properties"]["delivery_items"].is_object();
                    payload["evidence_repair_notice"] = json!(if numbered_delivery {
                        "Repair ONLY criteria.evidence_call_ids/evidence_paths at ineligible_evidence_criterion_paths; item numbers are ONE-based, indexes ZERO-based. Keep eligible criteria unchanged. Use unverified and omit those CRITERION evidence fields when matching current evidence is absent. This does NOT invalidate delivery_items.results: its result_ref enum independently includes recorded historical data. Preserve every still-advertised delivery result, including compound items' sub-results; never delete it or mark it missing merely because its current evidence ID is ineligible. Selected exact values are published by the host, not duplicated in summary. Missing execution is different from stale verification; do not claim unperformed work or repeat settled operations. Write public labels, explanations and rationales in the SAME LANGUAGE as report.summary without internal eligibility terminology."
                    } else {
                        "Repair only the exact JSON locations in ineligible_evidence_criterion_paths. ineligible_evidence_criterion_numbers counts items from ONE; ineligible_evidence_criteria counts array indexes from ZERO. Keep already eligible criteria unchanged. Cite another ID only when its own scope supports the claim. Otherwise deliver earlier results in summary, use unverified and omit evidence fields. Do not infer unexecuted work, invent results or repeat operations. Write the rationale in the SAME LANGUAGE as report.summary and explain the actual uncertainty; do not copy the example's English rationale verbatim."
                    });
                    let chinese=call.arguments.0["summary"].as_str().is_some_and(|text|text.chars().any(|c|matches!(c as u32,0x3400..=0x9fff)));
                    payload["unverified_criterion_example"] = json!({"disposition":"unverified",
                        "rationale":if chinese {"当前状态未复核；已经取得的结果另行列出。"} else {"Current state was not rechecked; recorded results are reported separately."}});
                }
            }
            let result = AgentToolResult::text(call.call_id.clone(), payload.to_string(), true);
            (call.call_id.clone(), Ok(result))
        }).collect()))
    }
}

fn bounded_schema_value(value: &Value) -> Option<Value> {
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 1024).then(|| value.clone())
}

pub(crate) fn declaration_issues(schema:&Value,arguments:&Value)->Result<Vec<Value>,String> {
    let validator=jsonschema::options().with_retriever(NoExternalSchemaReads).build(schema)
        .map_err(|_|"Exact action tool schema is invalid or needs external references".to_owned())?;
    let mut issues=Vec::new();
    for error in validator.iter_errors(arguments).take(MAX_ISSUES_PER_CALL) {collect_issues(&error,schema,0,&mut issues);}
    Ok(issues)
}

#[cfg(test)]
mod exact_declaration_tests {
    use super::*;
    #[test]
    fn native_shape_feedback_locates_files_without_echoing_private_input() {
        let schema=crate::standard_tools::standard_agent_tool_exposures().into_iter()
            .find(|tool|tool.definition.name=="apply_patch").unwrap().definition.input_schema.0;
        let issues=declaration_issues(&schema,&json!({"path":"PRIVATE_PATH","hunks":[{"text":"PRIVATE_CONTENT"}]})).unwrap();
        assert!(!issues.is_empty());let text=serde_json::to_string(&issues).unwrap();
        assert!(text.contains("files"));assert!(!text.contains("PRIVATE_PATH")&&!text.contains("PRIVATE_CONTENT"));
    }
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
    let segments=path.split('/').collect::<Vec<_>>();
    let mut location=Vec::new();
    for (index,segment) in segments.iter().enumerate() {
        if *segment=="properties" && let Some(name)=segments.get(index+1) { location.push((*name).to_owned()); }
        else if *segment=="items" { location.push("*".to_owned()); }
    }
    issue["parameter_path_template"]=json!(format!("/{}",location.join("/")));
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
    fn empty_or_null_optional_parameters_are_named_for_omission() {
        let (plan, exposed) = fixture(json!({"type":"object","required":["path"],"properties":{
            "path":{"type":"string"},
            "draft_id":{"type":"string","minLength":1},
            "plugin_id":{"type":"string"},
            "note":{"type":"string","minLength":1}
        }}));
        let mut validators = ToolArgumentValidators::default();
        let results = validators.reject_invalid_batch(&[call("open", json!({
            "path":"a","draft_id":"","plugin_id":null,"note":"ok"
        }))], &plan, &exposed).unwrap().unwrap();
        let payload:Value = serde_json::from_str(&results[0].1.as_ref().unwrap().output_text()).unwrap();
        assert_eq!(payload["omit_optional_parameters"], json!(["/draft_id","/plugin_id"]), "{payload}");
        assert!(payload["message"].as_str().unwrap().contains(
            "Omit optional parameters you do not use; never send empty strings or null for them."), "{payload}");
        assert!(validators.reject_invalid_batch(&[call("fixed", json!({"path":"a","note":"x"}))],
            &plan, &exposed).unwrap().is_none());
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
    fn completion_feedback_identifies_stale_criteria_without_offering_unrelated_ids() {
        let mut definition=crate::completion::definition();
        definition.input_schema.0["properties"]["observed_tool_error_count"]["const"]=json!(1);
        definition.input_schema.0["properties"]["observed_command_failure_count"]["const"]=json!(1);
        definition.input_schema.0["properties"]["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"]=
            json!(["current-directory","current-test-zero","current-test-one"]);
        let supported=|ids:Vec<&str>|json!({"disposition":"supported","evidence_call_ids":ids,"rationale":"PRIVATE_MODEL_INTERPRETATION"});
        let mut report=call("report",json!({"summary":"PRIVATE_REPORT_SUMMARY","observed_tool_error_count":1,
            "observed_command_failure_count":1,"criteria":[supported(vec!["current-directory"]),
                supported(vec!["PRIVATE_STALE_READ"]),supported(vec!["PRIVATE_SEARCH_ONE","PRIVATE_SEARCH_TWO"]),
                supported(vec!["PRIVATE_STATUS","PRIVATE_DIFF"]),supported(vec!["current-test-zero","current-test-one"])]}));
        report.name=crate::completion::TOOL_NAME.into();
        let before=report.arguments.clone();
        let mut validators=ToolArgumentValidators::default();
        let rejected=validators.reject_invalid_batch(std::slice::from_ref(&report),&AgentToolPlan::default(),std::slice::from_ref(&definition))
            .unwrap().unwrap();
        let text=rejected[0].1.as_ref().unwrap().output_text();
        let payload:Value=serde_json::from_str(&text).unwrap();
        assert_eq!(payload["ineligible_evidence_criteria"],json!([1,2,3]),"the model must be told which criteria failed");
        assert_eq!(payload["ineligible_evidence_criterion_numbers"],json!([2,3,4]),"public item numbers start at one");
        assert_eq!(payload["ineligible_evidence_criterion_paths"],json!(["/criteria/1","/criteria/2","/criteria/3"]),
            "repair targets must identify exact JSON positions without private values");
        assert_eq!(payload["status"],"not_executed");
        assert_eq!(payload["correction_counters_after_rejected_batch"],json!({"observed_tool_error_count":2,"observed_command_failure_count":1}));
        for issue in payload["issues"].as_array().unwrap() {
            assert!(issue.get("expected").is_none(),"a list of other observations is not a scope-correct repair");
            assert_eq!(issue["parameter_path_template"],"/criteria/*/evidence_call_ids/*");
        }
        assert!(!text.contains("PRIVATE_") && !text.contains("current-directory"));
        assert_eq!(report.arguments,before,"feedback must not silently rewrite the rejected report");
        let template=payload["unverified_criterion_example"].clone();
        assert_eq!(template["disposition"],"unverified");
        assert!(template.get("evidence_call_ids").is_none() && template.get("evidence_paths").is_none());
        let mut corrected=report.clone();corrected.call_id="fresh-report".into();
        corrected.arguments.0["summary"]=json!("Earlier file/search/Git results are reported; later state was not rechecked.");
        corrected.arguments.0["observed_tool_error_count"]=json!(2);
        for index in [1,2,3] {corrected.arguments.0["criteria"][index]=template.clone();}
        definition.input_schema.0["properties"]["observed_tool_error_count"]["const"]=json!(2);
        assert!(validators.reject_invalid_batch(&[corrected],&AgentToolPlan::default(),&[definition]).unwrap().is_none());
    }

    #[test]
    fn numbered_evidence_repair_preserves_independent_historical_delivery() {
        let mut definition=crate::completion::definition();
        definition.input_schema.0["properties"]["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"]=json!(["current"]);
        definition.input_schema.0["properties"]["delivery_items"]=json!({"type":"array","items":{"type":"object",
            "properties":{"results":{"type":"array","items":{"type":"object","properties":{
                "result_ref":{"type":"string","enum":["historic-head","historic-git"]}}}}}}});
        let mut report=call("report",json!({"summary":"历史实值仍要完整交付。","criteria":[{
            "disposition":"supported","evidence_call_ids":["historic-git"],"rationale":"PRIVATE_INTERPRETATION"}],
            "delivery_items":[{"results":[{"result_ref":"historic-head"},{"result_ref":"historic-git"}]}]}));
        report.name=crate::completion::TOOL_NAME.into();
        let original=report.arguments.clone();
        let mut validators=ToolArgumentValidators::default();
        let rejected=validators.reject_invalid_batch(std::slice::from_ref(&report),&AgentToolPlan::default(),std::slice::from_ref(&definition)).unwrap().unwrap();
        let feedback:Value=serde_json::from_str(&rejected[0].1.as_ref().unwrap().output_text()).unwrap();
        let notice=feedback["evidence_repair_notice"].as_str().unwrap();
        assert!(notice.contains("ONLY criteria.evidence_call_ids/evidence_paths") && notice.contains("does NOT invalidate delivery_items.results"));
        assert!(notice.contains("never delete it or mark it missing") && !notice.contains("deliver earlier results in summary"));
        assert!(feedback["unverified_criterion_example"]["rationale"].as_str().unwrap().contains("已经取得的结果"));
        assert!(!feedback.to_string().contains("PRIVATE_INTERPRETATION"));
        assert_eq!(report.arguments,original,"only the model can repair arguments; the host preserves data");
        report.arguments.0["criteria"][0]=feedback["unverified_criterion_example"].clone();
        assert!(validators.reject_invalid_batch(std::slice::from_ref(&report),&AgentToolPlan::default(),std::slice::from_ref(&definition)).unwrap().is_none());
        assert_eq!(report.arguments.0["delivery_items"],original.0["delivery_items"]);
    }

    #[test]
    fn rejected_completion_repairs_the_account_without_replaying_settled_work() {
        let mut definition = crate::completion::definition();
        definition.input_schema.0["properties"]["observed_tool_error_count"]["const"] = json!(1);
        definition.input_schema.0["properties"]["observed_command_failure_count"]["const"] = json!(1);
        definition.input_schema.0["properties"]["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"] = json!(["current-command"]);
        let mut invalid = call("bad-report", json!({"summary":"PRIVATE_REPORT_BODY",
            "observed_tool_error_count":0,"criteria":[{"disposition":"supported",
                "evidence_call_ids":["stale-file-id"],"rationale":"PRIVATE_RATIONALE"}]}));
        invalid.name = crate::completion::TOOL_NAME.into();
        let mut validators = ToolArgumentValidators::default();
        let rejected = validators.reject_invalid_batch(&[invalid], &AgentToolPlan::default(), &[definition.clone()])
            .unwrap().unwrap();
        let text = rejected[0].1.as_ref().unwrap().output_text();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["correction_counters_after_rejected_batch"], json!({
            "observed_tool_error_count":2,"observed_command_failure_count":1
        }));
        assert!(text.contains("Counter issue expected values already include this rejected batch"));
        assert!(text.contains("INVALID_TOOL_ARGUMENTS"));
        assert!(text.contains("/criteria/*/evidence_call_ids/*"));
        assert!(text.contains("/observed_tool_error_count"));
        assert!(!text.contains("PRIVATE_REPORT_BODY") && !text.contains("PRIVATE_RATIONALE") && !text.contains("stale-file-id"));
        assert!(text.contains("Repair only the completion arguments"));
        assert!(text.contains("do not rerun settled commands or tests"));
        assert!(text.contains("unverified with no evidence"));
        let mut corrected = call("fixed-report",json!({"summary":"Earlier read observed; later state not rechecked.",
            "observed_tool_error_count":1,"criteria":[{"disposition":"unverified","rationale":"No current file evidence"}]}));
        corrected.name = crate::completion::TOOL_NAME.into();
        assert!(validators.reject_invalid_batch(&[corrected], &AgentToolPlan::default(), &[definition]).unwrap().is_none());
    }

    #[test]
    fn counter_rejection_issue_expected_is_already_the_corrected_total() {
        // The rejection itself counts as a tool error, so the advertised
        // const/enum constants are stale inside the rejection issues; they
        // must publish the corrected totals a copy of `expected` can satisfy.
        let mut definition = crate::completion::definition();
        definition.input_schema.0["properties"]["observed_tool_error_count"]["const"] = json!(2);
        definition.input_schema.0["properties"]["observed_tool_error_count"]["enum"] = json!([2]);
        definition.input_schema.0["properties"]["observed_command_failure_count"]["const"] = json!(1);
        definition.input_schema.0["properties"]["observed_command_failure_count"]["enum"] = json!([1]);
        let mut stale = call("stale-report", json!({"summary":"PRIVATE_REPORT_BODY",
            "observed_tool_error_count":1,"observed_command_failure_count":1,
            "criteria":[{"disposition":"unverified","rationale":"PRIVATE_RATIONALE"}]}));
        stale.name = crate::completion::TOOL_NAME.into();
        let mut validators = ToolArgumentValidators::default();
        let rejected = validators.reject_invalid_batch(&[stale], &AgentToolPlan::default(), &[definition.clone()])
            .unwrap().unwrap();
        let payload: Value = serde_json::from_str(&rejected[0].1.as_ref().unwrap().output_text()).unwrap();
        assert_eq!(payload["correction_counters_after_rejected_batch"], json!({
            "observed_tool_error_count":3,"observed_command_failure_count":1}));
        let counter_issues = payload["issues"].as_array().unwrap().iter()
            .filter(|issue| issue["schema_path"].as_str().unwrap_or_default()
                .contains("/observed_tool_error_count/"))
            .collect::<Vec<_>>();
        assert!(!counter_issues.is_empty(), "counter-only rejection must still report its issues: {payload}");
        for issue in &counter_issues {
            assert_eq!(issue["expected"], json!(3),
                "a copied expected must satisfy the next advertised constant: {payload}");
        }
        // The next advertised constant is the corrected total; copying the
        // issue's expected repairs the account on the first retry.
        definition.input_schema.0["properties"]["observed_tool_error_count"]["const"] = json!(3);
        definition.input_schema.0["properties"]["observed_tool_error_count"]["enum"] = json!([3]);
        let mut fixed = call("fresh-report", json!({"summary":"PRIVATE_REPORT_BODY",
            "observed_tool_error_count":payload["correction_counters_after_rejected_batch"]["observed_tool_error_count"],
            "observed_command_failure_count":1,
            "criteria":[{"disposition":"unverified","rationale":"PRIVATE_RATIONALE"}]}));
        fixed.name = crate::completion::TOOL_NAME.into();
        assert!(validators.reject_invalid_batch(&[fixed], &AgentToolPlan::default(), &[definition]).unwrap().is_none());
    }

    #[test]
    fn control_tools_receive_nested_schema_feedback_without_private_argument_values() {
        let mut call = call("report", json!({"summary":"Ready","criteria":[{
            "disposition":"supported","rationale":42,"PRIVATE_FIELD":"PRIVATE_VALUE"
        }]}));
        call.name = crate::completion::TOOL_NAME.into();
        let results = ToolArgumentValidators::default().reject_invalid_batch(&[call],
            &AgentToolPlan::default(), &[crate::completion::definition()]).unwrap().unwrap();
        let text = results[0].1.as_ref().unwrap().output_text();
        assert!(text.contains("/criteria/*/rationale"));
        assert!(text.contains("allowed_properties"));
        assert!(!text.contains("PRIVATE_FIELD") && !text.contains("PRIVATE_VALUE"));
    }

    #[test]
    fn schema_drift_is_a_host_error_not_a_model_correction_or_new_authority() {
        let (plan, mut exposed) = fixture(json!({"type":"object"}));
        let expected = plan.binding("write_file").unwrap().schema_digest.as_ref().to_owned();
        exposed[0].input_schema = StrictJsonValue(json!({"type":"string"}));
        let actual = input_schema_digest(&exposed[0].input_schema).unwrap().as_ref().to_owned();
        let error = ToolArgumentValidators::default()
            .reject_invalid_batch(&[call("call", json!({}))], &plan, &exposed).unwrap_err();
        assert!(matches!(error, AgentEngineError::ToolSchemaDigestMismatch {
            tool_name, expected: frozen, actual: presented,
        } if tool_name == "write_file" && frozen == expected && presented == actual));
    }

    #[test]
    fn current_control_schemas_preflight_the_whole_platform_batch() {
        let (plan, mut exposed) = fixture(json!({"type":"object","additionalProperties":false,
            "required":["path"],"properties":{"path":{"type":"string"}}}));
        exposed.push(crate::planning::definition());
        exposed.push(crate::completion::definition());
        let control = |id: &str, name: &str, arguments| ChatToolCall {
            call_id: id.into(), name: name.into(),
            arguments: StrictJsonValue(arguments), provider_metadata: None,
        };
        assert!(ToolArgumentValidators::default().reject_invalid_batch(&[
            control("valid-plan", "update_plan", json!({"plan":[{"step":"inspect","status":"in_progress"}]})),
        ], &plan, &exposed).unwrap().is_none(), "first update_plan may omit explanation");
        for arguments in [
            json!({"explanation":"inspect"}),
            json!({"explanation":true,"plan":[{"step":"inspect","status":"in_progress"}]}),
            json!({"explanation":"inspect","plan":[{"step":"inspect","status":"in_progress"}],"unknown":true}),
        ] {
            let calls = [call("valid-write", json!({"path":"a"})), control("invalid-plan", "update_plan", arguments)];
            let results = ToolArgumentValidators::default().reject_invalid_batch(&calls, &plan, &exposed)
                .unwrap().expect("a malformed current control must hold the entire batch");
            assert_eq!(results.len(), 2);
            assert!(results.iter().all(|(_, result)| result.as_ref().unwrap().is_error));
            assert!(results[0].1.as_ref().unwrap().output_text().contains("BATCH_ARGUMENTS_REJECTED"));
            assert!(results[1].1.as_ref().unwrap().output_text().contains("INVALID_TOOL_ARGUMENTS"));
        }
        let calls = [call("valid-write", json!({"path":"a"})),
            control("invalid-report", "report_completion", json!({"summary":"done","criteria":[],"unknown":true}))];
        let results = ToolArgumentValidators::default().reject_invalid_batch(&calls, &plan, &exposed)
            .unwrap().expect("a malformed completion report must hold the entire batch");
        assert!(results.iter().all(|(_, result)| result.as_ref().unwrap().is_error));
        assert!(results.iter().all(|(_, result)| !result.as_ref().unwrap().output_text().contains("Repair only the completion arguments")),
            "a mixed batch must not imply that its unexecuted requested write can be discarded");
    }
}
