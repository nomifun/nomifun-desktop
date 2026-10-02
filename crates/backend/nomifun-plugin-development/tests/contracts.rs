use nomifun_plugin_development::{ACTIONS, CREATE_ACTIONS, MODULE_ID, action_id, input_schema, schema_ref, resolve_schema};
use serde_json::json;

#[test]
fn creation_actions_are_specific_and_resolve_the_exact_published_schema() {
    for required in CREATE_ACTIONS {
        let name=required.strip_prefix("plugin.development/").unwrap();
        assert!(ACTIONS.contains(&name));
        let schema=input_schema(name);
        let reference=schema_ref(&action_id(name),"input",&schema);
        assert_eq!(resolve_schema(&reference).unwrap().0,schema);
        assert_eq!(schema["additionalProperties"],false);
    }
    assert!(CREATE_ACTIONS.contains(&"plugin.development/test_ui"));
    assert!(!CREATE_ACTIONS.contains(&"plugin.development/delete"));
    assert_eq!(MODULE_ID,"plugin.development");
}

#[test]
fn model_cannot_supply_permission_approval_or_claim_a_ui_case_passed() {
    let install=jsonschema::validator_for(&input_schema("install")).unwrap();
    assert!(install.validate(&json!({
        "draft_id":"draft","expected_revision":1,"verification_digest":"digest",
        "permission_confirmation_id":"token_seen_elsewhere",
    })).is_err());
    let ui=jsonschema::validator_for(&input_schema("test_ui")).unwrap();
    assert!(ui.validate(&json!({
        "draft_id":"draft","expected_revision":1,"case_name":"todo",
        "steps":[{"operation":"count","selector":"li","value":1}],"passed":true,
    })).is_err());
    assert!(ui.validate(&json!({
        "draft_id":"draft","expected_revision":1,"case_name":"todo",
        "steps":[{"operation":"evaluate","code":"return true"}],
    })).is_err());
}

#[test]
fn requirements_link_cases_and_require_actual_restart_evidence() {
    let mut plan=json!({"summary":"Persist a value","output_key":"store","outputs":[{"key":"store","kind":"headless"}],
        "features":[{"description":"Read saved data","case_names":["read"],"requires_persistence":true}],
        "cases":{"read":{"kind":"action","action":"read","input":{},"expected_output":{"value":"saved"}}}});
    assert!(nomifun_plugin_development::validate_plan(&plan).is_err());
    plan["cases"]["read"]["restart"]=json!(true);
    nomifun_plugin_development::validate_plan(&plan).unwrap();
    let mut report=json!({"plan":plan,"cases":{"read":{"passed":true}}});
    assert!(!nomifun_plugin_development::plan_evidence_complete(&report));
    report["cases"]["read"]["persistence_checked"]=json!(true);
    assert!(nomifun_plugin_development::plan_evidence_complete(&report));
    report["plan"]["features"][0]["case_names"]=json!(["missing"]);
    assert!(nomifun_plugin_development::validate_plan(&report["plan"]).is_err());
}
