use super::*;
use serde_json::json;

#[tokio::test]
async fn hook_values_are_data_not_shell_source() {
    let temp = tempfile::tempdir().unwrap();
    let config = HooksConfig {
        post_tool_use: vec![HookDef {
            name: "echo-input".into(),
            tool_match: vec![],
            file_match: vec![],
            command: r#"echo "${TOOL_INPUT_FILE_PATH}""#.into(),
            timeout_ms: 10_000,
        }],
        ..Default::default()
    };
    let engine = HookEngine::new(config, temp.path().to_path_buf());
    for value in [
        "$(echo HOOK_INJECTION)",
        "a \"quoted\" path; # literal",
        "${TOOL_NAME}",
    ] {
        let messages = engine
            .run_post_tool_use("Read", &json!({"file_path": value}), "")
            .await;
        assert_eq!(messages, [format!("[hook:echo-input] {value}")]);
    }
}
