//! Optional Agent module for developing one Unified Plugin through ordinary
//! conversation tools. Plugin instances and runtime remain owned by Plugin Core.
#![forbid(unsafe_code)]

use std::{collections::BTreeSet, sync::Arc};
use async_trait::async_trait;
use nomifun_agent_contracts::*;
use nomifun_agent_domain_support::{CapabilitySpec, PackageSpec};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};
use serde_json::{Value, json};

pub const MODULE_ID: &str = "plugin.development";
pub const PACKAGE_ID: &str = "nomifun.plugin-development";
pub const CREATE_ACTIONS: &[&str] = &[
    "plugin.development/list", "plugin.development/open", "plugin.development/read",
    "plugin.development/plan",
    "plugin.development/apply", "plugin.development/check", "plugin.development/preview",
    "plugin.development/test_action", "plugin.development/test_ui", "plugin.development/install", "plugin.development/inspect",
];
pub const ACTIONS: &[&str] = &[
    "list", "open", "read", "plan", "apply", "check", "preview", "test_action", "test_ui", "install",
    "inspect", "configure", "enable", "export", "trash", "restore", "delete", "discard",
    "close_preview",
];
pub const DESCRIPTION: &str = "Plugin and small-app development / 插件与小程序。Create, edit, verify, install and manage one NomiFun Unified Plugin through this conversation. Start with list to obtain the authoritative SDK guide, then open a managed draft and plan every required output, feature and exact case before editing. Use read/apply for incremental files, check for actual diagnostics, preview for real UI or headless execution, and test_action for required business cases. Repair failures within this task. Install only the exact verified revision and inspect the installed result. A file tree, successful startup, or prose saying done is NOT a usable Plugin delivery. Missing configuration or execution trust requires user input; do not fabricate APIs, credentials, tests or successful results. User-installed instances remain independent of this development module.";

#[async_trait]
pub trait PluginDevelopmentHost: Send + Sync {
    async fn invoke(&self, context: CapabilityInvocationContext, input: Value) -> Result<Value, String>;
}

struct Handler(Arc<dyn PluginDevelopmentHost>);
#[async_trait]
impl CapabilityHandler for Handler {
    async fn invoke(&self, context: CapabilityInvocationContext, input: StrictJsonValue) -> Result<StrictJsonValue, KernelError> {
        if context.capability_id.as_ref() != MODULE_ID
            || !ACTIONS.iter().any(|action| context.action_id.as_ref() == action_id(action))
        {
            return Err(KernelError::CapabilityExecution { reason: "Unknown Plugin development action".into() });
        }
        let action = context.action_id.as_ref().strip_prefix("plugin.development/").expect("checked action");
        if action == "plan" {
            let diagnostics = plan_case_diagnostics(&input.0["plan"]);
            if !diagnostics.is_empty() {
                return Err(KernelError::CapabilityExecution {
                    reason: bounded(&format!("PLUGIN_INVALID_INPUT: {}", diagnostics.join("; ")), 2048),
                });
            }
        }
        let validator = jsonschema::validator_for(&input_schema(action))
            .map_err(|error| KernelError::CapabilityExecution { reason: error.to_string() })?;
        if let Some(reason) = validation_message(&validator, &input.0) {
            return Err(KernelError::CapabilityExecution { reason });
        }
        if serde_json::to_vec(&input.0).map_or(true, |bytes| bytes.len() > 4 * 1024 * 1024) {
            return Err(KernelError::CapabilityExecution { reason: "Plugin tool input exceeds 4 MiB".into() });
        }
        self.0.invoke(context, input.0).await.map(StrictJsonValue)
            .map_err(|reason| KernelError::CapabilityExecution { reason })
    }
}

pub fn action_id(action: &str) -> String { format!("{MODULE_ID}/{action}") }

/// Model-supplied values can be arbitrarily large; bound diagnostics on a
/// char boundary so a rejection stays small instead of echoing the input.
fn bounded(text: &str, max_chars: usize) -> String {
    let mut bounded: String = text.chars().take(max_chars).collect();
    if text.chars().nth(max_chars).is_some() {
        bounded.push('…');
    }
    bounded
}

/// Up to 8 schema issues as "path: message", so one rejection names every
/// actual problem instead of forcing repeated blind retries.
fn validation_message(validator: &jsonschema::Validator, instance: &Value) -> Option<String> {
    let mut issues = validator.iter_errors(instance).take(8).peekable();
    issues.peek()?;
    Some(bounded(&format!("PLUGIN_INVALID_INPUT: {}", issues
        .map(|error| bounded(&format!("{}: {}", error.instance_path(), error), 256))
        .collect::<Vec<_>>().join("; ")), 2048))
}

pub fn registration(host: Arc<dyn PluginDevelopmentHost>) -> Result<PluginRegistration, String> {
    const CAPS: &[CapabilitySpec] = &[CapabilitySpec::tool(MODULE_ID, EffectClass::WriteDurable, &[])];
    let base = nomifun_agent_domain_support::registration(PackageSpec {
        id: PACKAGE_ID, mount_id: "plugin-development", display_name: "Plugins and small apps",
        description: DESCRIPTION, capabilities: CAPS, supported_surfaces: &["desktop", "headless"],
    }).map_err(|error| error.to_string())?;
    let mut metadata = base.metadata;
    let port = HostPortRef { id: "host.plugin-development.invoke".into(), version: "1.0.0".into() };
    let cap = &mut metadata.manifest.payload.contributions.capabilities[0];
    cap.display.name = "Plugins and small apps / 插件与小程序".into();
    cap.display.description = DESCRIPTION.into();
    cap.contributions.host_ports = vec![port.clone()];
    cap.contributions.actions = ACTIONS.iter().map(|action| {
        let id = action_id(action);
        CapabilityActionDescriptor {
            action_id: id.as_str().into(),
            input_schema: schema_ref(&id, "input", &input_schema(action)),
            output_schema: schema_ref(&id, "output", &json!({"type":"object"})),
            effect_class: match *action {
                "list" | "read" | "inspect" => EffectClass::ReadLocal,
                "check" | "preview" | "test_action" | "test_ui" => EffectClass::ExecuteLocal,
                "delete" => EffectClass::Destructive,
                _ => EffectClass::WriteDurable,
            },
            presentation: ToolPresentationKind::FunctionTool,
        }
    }).collect();
    metadata.manifest = ArtifactEnvelope::new(metadata.manifest.payload).map_err(|error| error.to_string())?;
    metadata.registrar.allowed_operations.insert(PluginRegistrarOperation::BindHostPort);
    metadata.registrar.declared_host_ports.insert(port.id.clone());
    metadata.context.host_ports.push(HostPortBindingDescriptor {
        port, request_schema: schema_ref(MODULE_ID, "host-request", &json!({"type":"object"})),
        response_schema: schema_ref(MODULE_ID, "host-response", &json!({"type":"object"})),
    });
    let mut registration = PluginRegistration::new(metadata);
    registration.add_capability_handler(MODULE_ID.into(), Arc::new(Handler(host))).map_err(|error| error.to_string())?;
    Ok(registration)
}

pub fn schema_ref(action: &str, direction: &str, schema: &Value) -> CanonicalSchemaRef {
    format!("schema://nomifun/{action}/{direction}@1#{}", digest_payload(schema).expect("static schema").as_ref()).into()
}

pub fn resolve_schema(reference: &CanonicalSchemaRef) -> Option<StrictJsonValue> {
    ACTIONS.iter().find_map(|action| {
        let schema = input_schema(action);
        (schema_ref(&action_id(action), "input", &schema) == *reference).then_some(StrictJsonValue(schema))
    })
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}
fn string() -> Value { json!({"type":"string","minLength":1,"maxLength":32768}) }
fn revision() -> Value { json!({"type":"integer","minimum":1}) }

pub fn input_schema(action: &str) -> Value {
    let config = json!({"type":"object","description":"Actual values matching configSchema; no invented credentials."});
    let credentials = json!({"type":"object","additionalProperties":{"type":"string"},"description":"Host Credential reference IDs only; never plaintext secrets."});
    let properties = match action {
        "list" => json!({}),
        "open" => json!({"draft_id":string(),"plugin_id":string(),"expected_plugin_revision":revision(),"template":{"type":"string","enum":["agent.before_tool"]}}),
        "read" | "discard" => json!({"draft_id":string(),"expected_revision":revision()}),
        "plan" => json!({"draft_id":string(),"expected_revision":revision(),"plan":plan_schema()}),
        "apply" => json!({
            "draft_id":string(),"expected_revision":revision(),
            "files":{"type":"object","additionalProperties":{"type":"string","maxLength":1048576},"description":"Normalized package paths to full UTF-8 content; omitted files stay unchanged."},
            "delete":{"type":"array","uniqueItems":true,"items":string(),"description":"Exact package paths to delete; the manifest cannot be deleted."}
        }),
        "check" => json!({"draft_id":string(),"expected_revision":revision()}),
        "preview" => json!({"draft_id":string(),"expected_revision":revision(),"config":config,"permissions":{"type":"array","items":string(),"uniqueItems":true},"credential_bindings":credentials}),
        "test_action" => json!({"draft_id":string(),"expected_revision":revision(),"action":string(),"input":{},"expected_output":{},"case_name":string(),"restart":{"type":"boolean"}}),
        "test_ui" => json!({
            "draft_id":string(),"expected_revision":revision(),"case_name":string(),
            "steps":{"type":"array","minItems":1,"maxItems":32,"items":object(json!({
                "operation":{"type":"string","enum":["click","fill","text","count","reopen"]},
                "selector":string(),"value":{"type":["string","number"]}
            }), &["operation"])}
        }),
        "install" => json!({
            "draft_id":string(),"expected_revision":revision(),"expected_plugin_revision":revision(),
            "config":config,"credential_bindings":credentials,
            "verification_digest":string()
        }),
        "inspect" => json!({"plugin_id":string()}),
        "configure" => json!({"plugin_id":string(),"expected_revision":revision(),"config":config,"credential_bindings":{"type":"object","additionalProperties":{"type":["string","null"]}},"grants":{"type":"object","additionalProperties":{"type":"boolean"}}}),
        "enable" => json!({"plugin_id":string(),"expected_revision":revision(),"enabled":{"type":"boolean"}}),
        "export" => json!({"plugin_id":string(),"expected_revision":revision(),"destination_path":string(),"include_source":{"type":"boolean"}}),
        "trash" => json!({"plugin_id":string(),"expected_revision":revision()}),
        "delete" => json!({"plugin_id":string(),"expected_revision":revision(),"acknowledge_permanent_delete":{"type":"boolean","const":true}}),
        "restore" => json!({"plugin_id":string(),"expected_revision":revision(),"mode":{"type":"string","enum":["previous_code","previous_code_and_data","from_trash"]},"acknowledge_data_loss":{"type":"boolean"}}),
        "close_preview" => json!({"draft_id":string(),"surface_session_id":string(),"surface_generation":revision()}),
        _ => return json!({"not":{}}),
    };
    let required: &[&str] = match action {
        "list" | "open" => &[],
        "read" => &["draft_id"],
        "plan" => &["draft_id","expected_revision","plan"],
        "apply" => &["draft_id","expected_revision","files"],
        "check" | "preview" | "discard" => &["draft_id","expected_revision"],
        "test_action" => &["draft_id","expected_revision","action","input","expected_output","case_name"],
        "test_ui" => &["draft_id","expected_revision","steps","case_name"],
        "install" => &["draft_id","expected_revision","verification_digest"],
        "inspect" => &["plugin_id"],
        "configure" => &["plugin_id","expected_revision","config"],
        "enable" => &["plugin_id","expected_revision","enabled"],
        "export" => &["plugin_id","expected_revision","destination_path"],
        "delete" => &["plugin_id","expected_revision","acknowledge_permanent_delete"],
        "restore" => &["plugin_id","expected_revision","mode"],
        "close_preview" => &["draft_id","surface_session_id","surface_generation"],
        _ => &["plugin_id","expected_revision"],
    };
    let mut schema = object(properties, required);
    schema["description"] = Value::String(match action {
        "check" => "Compile/inspect the exact draft. Returns actionable diagnostics, not self-reported success.",
        "plan" => "Before editing, record every requested output, this draft's output key, required features and exact business cases. Include restart/reopen assertions for persistence. The accepted plan cannot be weakened during repair.",
        "preview" => "Run the actual draft with temporary storage; UI handshake/business tests are required before UI delivery.",
        "install" => "Save the exact verified draft. May return confirmation_required; the user must approve actual additional privileges.",
        "test_action" => "Execute a business case on the real preview Service. Planned cases retain their exact oracle; extra named cases are diagnostic probes and do not add delivery requirements.",
        "test_ui" => "Run typed DOM interactions on the real conversation preview. Planned cases retain their exact steps; extra named cases are diagnostic probes. text/count assert the exact result; include reopen followed by an assertion to verify persistence. Failure diagnostics report the step, expected and actual values. No arbitrary JavaScript.",
        "open" => "Open a managed draft for this conversation. Replays use the same draft; editing requires the exact installed revision.",
        _ => DESCRIPTION,
    }.into());
    schema
}

pub fn capability_ids() -> BTreeSet<CapabilityId> { BTreeSet::from([MODULE_ID.into()]) }

fn plan_action_case_schema() -> Value {
    object(json!({"kind":{"const":"action"},"action":string(),"input":{},"expected_output":{},"restart":{"type":"boolean"}}),
        &["kind","action","input","expected_output"])
}
fn plan_ui_case_schema() -> Value {
    object(json!({"kind":{"const":"ui"},"steps":input_schema("test_ui")["properties"]["steps"]}),&["kind","steps"])
}
/// Precise per-case diagnostics for the plan's oneOf case branches. Errors
/// carry their full /plan/cases/<name> path so a rejected plan names the
/// exact property instead of collapsing into an opaque oneOf failure.
fn plan_case_diagnostics(plan: &Value) -> Vec<String> {
    let mut diagnostics=Vec::new();
    let Some(cases)=plan.get("cases").and_then(Value::as_object) else { return diagnostics; };
    let action=jsonschema::validator_for(&plan_action_case_schema());
    let ui=jsonschema::validator_for(&plan_ui_case_schema());
    for (name,case) in cases {
        let validator=match case.get("kind").and_then(Value::as_str) {
            Some("action")=>action.as_ref().ok(),
            Some("ui")=>ui.as_ref().ok(),
            _=>None,
        };
        match validator {
            Some(validator)=>diagnostics.extend(validator.iter_errors(case).take(8)
                .map(|error|bounded(&format!("/plan/cases/{name}{}: {error}",error.instance_path()),256))),
            None=>diagnostics.push(bounded(&format!("/plan/cases/{name}/kind: must be \"ui\" or \"action\""),256)),
        }
    }
    diagnostics
}

pub fn plan_schema() -> Value {
    object(json!({
        "summary":string(),"output_key":string(),
        "current_conversation_case":object(json!({"action":string(),"input":{},"expected_output":{}}),&["action","input","expected_output"]),
        "outputs":{"type":"array","minItems":1,"maxItems":32,"items":object(json!({
            "key":string(),"kind":{"enum":["ui","headless","mixed"]}
        }),&["key","kind"])},
        "features":{"type":"array","minItems":1,"maxItems":32,"items":object(json!({
            "description":string(),"case_names":{"type":"array","minItems":1,"maxItems":32,"uniqueItems":true,"items":string()},
            "requires_persistence":{"type":"boolean"}
        }),&["description","case_names"])},
        "cases":{"type":"object","minProperties":1,"maxProperties":64,"propertyNames":string(),"additionalProperties":{"oneOf":[plan_action_case_schema(),plan_ui_case_schema()]}}
    }),&["summary","output_key","outputs","features","cases"])
}

pub fn validate_plan(plan: &Value) -> Result<(), String> {
    jsonschema::validator_for(&plan_schema()).map_err(|error|error.to_string())?
        .validate(plan).map_err(|error|error.to_string())?;
    let mut outputs=BTreeSet::new();
    for output in plan["outputs"].as_array().expect("validated") {
        let key=output["key"].as_str().expect("validated");
        if !outputs.insert(key) { return Err(format!("output '{key}': duplicate output key")); }
    }
    let output_key=plan["output_key"].as_str().expect("validated");
    if !outputs.contains(output_key) { return Err(format!("output_key '{output_key}': the draft must identify one declared output")); }
    let cases=plan["cases"].as_object().expect("validated");
    let kind=plan["outputs"].as_array().expect("validated").iter().find(|output|output["key"]==plan["output_key"])
        .expect("validated output key")["kind"].as_str().expect("validated");
    let ui=cases.values().any(|case|case["kind"]=="ui");
    let service=cases.values().any(|case|case["kind"]=="action");
    if kind=="ui" && (!ui || service) || kind=="headless" && (!service || ui) || kind=="mixed" && (!ui || !service) {
        return Err(format!("output '{output_key}' kind '{kind}': cases must cover the planned UI/Service shape"));
    }
    // The accepted plan is immutable; a non-canonical action id here would
    // make the plan permanently unsatisfiable after a manifest rename.
    for (name,case) in cases.iter() {
        if case["kind"]=="action" && let Some(action)=case["action"].as_str()
            && nomifun_agent_contracts::plugin::validate_machine_id(action,"actions.<id>",96,false).is_err() {
            return Err(format!("case {name}: action '{action}' must be a lowercase machine identifier such as trim_and_uppercase"));
        }
    }
    if let Some(action)=plan["current_conversation_case"]["action"].as_str()
        && nomifun_agent_contracts::plugin::validate_machine_id(action,"actions.<id>",96,false).is_err() {
        return Err(format!("current_conversation_case: action '{action}' must be a lowercase machine identifier such as trim_and_uppercase"));
    }
    for (name,case) in cases.iter().filter(|(_,case)|case["kind"]=="ui") {
        let steps=case["steps"].as_array().expect("validated");
        if !steps.iter().any(|step|step["operation"]=="text" || step["operation"]=="count") {
            return Err(format!("case {name}: UI cases require a concrete result assertion (text or count)"));
        }
        for (index,step) in steps.iter().enumerate() {
            if step["operation"]!="reopen" && step["selector"].as_str().is_none_or(str::is_empty) {
                return Err(format!("case {name} step {index}: UI steps require a selector"));
            }
            if matches!(step["operation"].as_str(),Some("fill"|"text")) && !step["value"].is_string()
                || step["operation"]=="count" && step["value"].as_u64().is_none() {
                return Err(format!("case {name} step {index}: UI steps require correctly typed inputs and expectations"));
            }
        }
    }
    for feature in plan["features"].as_array().expect("validated") {
        let description=feature["description"].as_str().expect("validated");
        let names=feature["case_names"].as_array().expect("validated");
        if let Some(missing)=names.iter().filter_map(Value::as_str).find(|name|!cases.contains_key(*name)) {
            return Err(format!("feature '{description}': case '{missing}' is not a declared case"));
        }
        if feature["requires_persistence"]==json!(true) && !names.iter().any(|name| {
            let case=&cases[name.as_str().expect("validated")];
            if case["kind"]=="action" { return case["restart"]==json!(true); }
            let mut reopened=false;
            case["steps"].as_array().expect("validated UI steps").iter().any(|step| {
                if step["operation"]=="reopen" { reopened=true; }
                reopened && (step["operation"]=="text" || step["operation"]=="count")
            })
        }) { return Err(format!("feature '{description}': persistence requires a case that reads after restart/reopen")); }
    }
    Ok(())
}

pub fn plan_evidence_complete(report: &Value) -> bool {
    let plan=&report["plan"];
    if validate_plan(plan).is_err() { return false; }
    // Only the immutable plan defines required cases. Diagnostic probes may
    // fail without changing delivery, while every declared case remains due
    // even when no feature happens to reference it.
    if !plan["cases"].as_object().expect("validated").keys()
        .all(|name|report["cases"][name]["passed"]==json!(true)) { return false; }
    plan["features"].as_array().expect("validated").iter().all(|feature| {
        let names=feature["case_names"].as_array().expect("validated");
        names.iter().all(|name|report["cases"][name.as_str().expect("validated")]["passed"]==json!(true))
            && (feature["requires_persistence"]!=json!(true) || names.iter().any(|name|
                report["cases"][name.as_str().expect("validated")]["persistence_checked"]==json!(true)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_plan() -> Value {
        json!({
            "summary":"Deliver a todo plugin.",
            "output_key":"todo",
            "outputs":[{"key":"todo","kind":"ui"}],
            "features":[{"description":"persist","case_names":["add_todo"],"requires_persistence":true}],
            "cases":{"add_todo":{"kind":"ui","steps":[
                {"operation":"fill","selector":"#input","value":"milk"},
                {"operation":"click","selector":"#add"},
                {"operation":"reopen"},
                {"operation":"text","selector":"#list","value":"milk"}
            ]}}
        })
    }

    #[test]
    fn plan_case_diagnostics_names_the_offending_step_property() {
        let mut plan=valid_plan();
        plan["cases"]["add_todo"]["steps"][3]["description"]=json!("oops");
        let diagnostics=plan_case_diagnostics(&plan);
        assert_eq!(diagnostics.len(),1);
        assert!(diagnostics[0].starts_with("/plan/cases/add_todo/steps/3"),"{diagnostics:?}");
        assert!(diagnostics[0].contains("'description'"),"{diagnostics:?}");
    }

    #[test]
    fn plan_case_diagnostics_rejects_an_unknown_case_kind() {
        let mut plan=valid_plan();
        plan["cases"]["add_todo"]["kind"]=json!("widget");
        assert_eq!(plan_case_diagnostics(&plan),
            ["/plan/cases/add_todo/kind: must be \"ui\" or \"action\""]);
    }

    #[test]
    fn planned_action_ids_must_be_manifest_machine_ids() {
        let mut plan=valid_plan();
        plan["output_key"]=json!("service");
        plan["outputs"]=json!([{"key":"service","kind":"headless"}]);
        plan["features"]=json!([{"description":"trim","case_names":["trim"]}]);
        plan["cases"]=json!({"trim":{"kind":"action","action":"trimAndUppercase","input":{},"expected_output":{}}});
        let error=validate_plan(&plan).unwrap_err();
        assert!(error.contains("case trim") && error.contains("trimAndUppercase")
            && error.contains("lowercase machine identifier"),"{error}");
        plan["cases"]["trim"]["action"]=json!("trim_and_uppercase");
        plan["current_conversation_case"]=json!({"action":"trimAndUppercase","input":{},"expected_output":{}});
        let error=validate_plan(&plan).unwrap_err();
        assert!(error.contains("current_conversation_case") && error.contains("trimAndUppercase"),"{error}");
        plan["current_conversation_case"]["action"]=json!("trim_and_uppercase");
        validate_plan(&plan).unwrap();
    }

    #[test]
    fn a_valid_plan_passes_diagnostics_and_semantic_validation() {
        let plan=valid_plan();
        assert!(plan_case_diagnostics(&plan).is_empty());
        validate_plan(&plan).unwrap();
        let validator=jsonschema::validator_for(&input_schema("plan")).unwrap();
        assert!(validation_message(&validator,&json!({
            "draft_id":"draft-1","expected_revision":1,"plan":plan
        })).is_none());
    }

    #[test]
    fn delivery_requires_every_planned_case_but_ignores_diagnostic_failures() {
        let mut plan=valid_plan();
        plan["cases"]["extra-planned-check"]=json!({"kind":"ui","steps":[
            {"operation":"count","selector":"#list","value":1}
        ]});
        let mut report=json!({"plan":plan,"cases":{
            "add_todo":{"passed":true,"persistence_checked":true},
            "debug-probe":{"passed":false}
        }});
        assert!(!plan_evidence_complete(&report),"unreferenced planned cases are still required");
        report["cases"]["extra-planned-check"]=json!({"passed":true});
        assert!(plan_evidence_complete(&report),"a failed diagnostic does not change the plan");
        report["cases"]["add_todo"]["persistence_checked"]=json!(false);
        assert!(!plan_evidence_complete(&report),"required persistence evidence cannot be weakened");
    }

    #[test]
    fn generic_validation_reports_each_issue_with_its_path() {
        let validator=jsonschema::validator_for(&input_schema("plan")).unwrap();
        let message=validation_message(&validator,&json!({
            "expected_revision":"not an integer","plan":{"summary":"x"}
        })).unwrap();
        assert!(message.starts_with("PLUGIN_INVALID_INPUT: "),"{message}");
        assert!(message.contains("/expected_revision"),"{message}");
        assert!(message.contains("draft_id"),"{message}");
        assert!(message.contains("; "),"{message}");
    }

    #[test]
    fn validation_and_plan_diagnostics_stay_bounded_for_huge_inputs() {
        let validator=jsonschema::validator_for(&input_schema("plan")).unwrap();
        let message=validation_message(&validator,&json!({
            "draft_id":"d","expected_revision":"x".repeat(100_000),"plan":{"summary":"x"}
        })).unwrap();
        assert!(message.chars().count()<=2049,"truncation adds one ellipsis: {message}");
        assert!(message.contains("/expected_revision"),"{message}");

        let mut plan=valid_plan();
        plan["cases"]["add_todo"]["steps"][0]["value"]=json!("y".repeat(100_000));
        // A still-schema-valid step keeps the case quiet; an unknown kind
        // names it without echoing the oversized sibling values.
        plan["cases"]["add_todo"]["kind"]=json!("widget");
        let diagnostics=plan_case_diagnostics(&plan);
        assert_eq!(diagnostics.len(),1);
        assert!(diagnostics[0].chars().count()<=257,"{diagnostics:?}");
        let mut plan=valid_plan();
        plan["cases"]["add_todo"]["steps"][0]["selector"]=json!("s".repeat(100_000));
        plan["cases"]["add_todo"]["steps"][0]["description"]=json!("oops");
        let diagnostics=plan_case_diagnostics(&plan);
        assert!(!diagnostics.is_empty() && diagnostics.len()<=8);
        assert!(diagnostics.iter().all(|entry|entry.chars().count()<=257),
            "truncation adds one ellipsis: {diagnostics:?}");
        assert!(diagnostics[0].starts_with("/plan/cases/add_todo/steps/0"),"{diagnostics:?}");
    }
}
