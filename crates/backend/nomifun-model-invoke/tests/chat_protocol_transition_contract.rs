//! The same transition fixture is exercised by the settings editor's TS mirror.
use nomifun_model_invoke::transition_chat_provider_params;

#[test]
fn provider_params_match_the_shared_protocol_transition_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("fixtures/chat_protocol_transitions.json")).unwrap();
    assert_eq!(fixture["version"],1);
    let cases = fixture["cases"].as_array().expect("transition cases");
    assert!(!cases.is_empty());
    for case in cases {
        let result = transition_chat_provider_params(case["params"].as_object().expect("params object"),
            case["source"].as_str().unwrap(),case["target"].as_str().unwrap());
        let mut actual = serde_json::to_value(result).unwrap();
        let mut expected = case["expected"].clone();
        for value in [&mut actual, &mut expected] {
            value["issues"].as_array_mut().unwrap().sort_by(|left,right|
                left["parameter"].as_str().cmp(&right["parameter"].as_str())
                    .then_with(|| left["kind"].as_str().cmp(&right["kind"].as_str())));
        }
        assert_eq!(actual,expected,"case {}",case["name"]);
    }
}
