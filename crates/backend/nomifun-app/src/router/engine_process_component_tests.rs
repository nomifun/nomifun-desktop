#[tokio::test]
async fn component_preparation_rejections_do_not_dispatch_user_processes() {
    let root = tempfile::tempdir().unwrap();
    let (journal, _) = super::super::engine_journal::test_fixture().await;
    let scope = EngineProcessScope::new(root.path(), journal).unwrap();
    for (input, code) in [
        (serde_json::json!({"component":"unknown-component"}), "COMPONENT_NOT_DECLARED"),
        (serde_json::json!({"component":"officecli","env":{"OFFICECLI_NO_AUTO_RESIDENT":"conflicting-value"}}), "COMPONENT_ENVIRONMENT_CONFLICT"),
        (serde_json::json!({"component":"officecli","cwd":"../outside"}), "INVALID_PAYLOAD"),
    ] {
        let observation = scope.invoke(StrictJsonValue(input), code).await.unwrap().0;
        assert_eq!(observation["schema"], "nomifun.process-start-observation.v1");
        assert_eq!(observation["state"], "not_started");
        assert_eq!(observation["user_code_started"], false);
        assert_eq!(observation["code"], code);
        assert!(observation.get("process_id").is_none());
        assert!(!observation.to_string().contains("conflicting-value"));
    }
    let state = scope.state.lock().await;
    assert!(state.operations.is_empty());
    assert!(state.sessions.is_empty());
    assert!(state.is_quiescent());
}

#[test]
fn component_error_classification_does_not_change_ordinary_bun_errors() {
    let unavailable = nomifun_runtime::ToolError::Unavailable { tool:"bun", override_variable:"NOMIFUN_BUN_PATH" };
    assert_eq!(runtime_resolution_error(&unavailable).code, "COMPONENT_PREPARATION_REQUIRED");
    assert_eq!(component_resolution_error(nomifun_runtime::ToolId::Bun, &unavailable).code, "COMPONENT_UNAVAILABLE");
    let invalid = nomifun_runtime::ToolError::Verification { tool:"officecli", reason:"api_key=example-private-token".into() };
    let component = component_resolution_error(nomifun_runtime::ToolId::OfficeCli, &invalid);
    assert_eq!(component.code, "COMPONENT_VERIFICATION_FAILED");
    assert!(!component.message.contains("example-private-token"));
}

#[tokio::test]
async fn component_cancellation_is_unstarted_and_does_not_close_other_suppliers() {
    let root = tempfile::tempdir().unwrap();
    let (journal, _) = super::super::engine_journal::test_fixture().await;
    let scope = EngineProcessScope::new(root.path(), journal).unwrap();
    scope.close_admission();
    let observation = scope.invoke(StrictJsonValue(serde_json::json!({"component":"officecli"})), "cancelled-component").await.unwrap().0;
    assert_eq!(observation["state"], "not_started");
    assert_eq!(observation["code"], "CANCELLED");
    assert_eq!(observation["user_code_started"], false);
    assert!(scope.state.lock().await.operations.is_empty());
}

#[tokio::test]
async fn component_cannot_change_existing_process_controls_or_mix_launch_targets() {
    let root = tempfile::tempdir().unwrap();
    let (journal, _) = super::super::engine_journal::test_fixture().await;
    let scope = EngineProcessScope::new(root.path(), journal).unwrap();
    for input in [
        serde_json::json!({"operation":"poll","component":"officecli","process_id":"existing"}),
        serde_json::json!({"component":"officecli","command":"officecli"}),
        serde_json::json!({"component":"officecli","cmd":"officecli --version"}),
    ] {
        let failure = scope.invoke(StrictJsonValue(input), "invalid-component").await.unwrap_err();
        assert_eq!(failure.code, "INVALID_PAYLOAD");
    }
    assert!(scope.state.lock().await.operations.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn component_entry_does_not_change_explicit_user_programs_or_shell_environment() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("officecli");
    std::fs::write(&executable, "#!/bin/sh\nprintf 'ordinary:%s' \"$OFFICECLI_NO_AUTO_RESIDENT\"\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (journal, _) = super::super::engine_journal::test_fixture().await;
    let scope = EngineProcessScope::new(root.path(), journal).unwrap();
    for (operation, input) in [
        ("ordinary-program", serde_json::json!({"command":executable,"env":{"OFFICECLI_NO_AUTO_RESIDENT":"0"},"timeout_ms":5000})),
        ("ordinary-shell", serde_json::json!({"cmd":"./officecli","env":{"OFFICECLI_NO_AUTO_RESIDENT":"0"},"timeout_ms":5000})),
    ] {
        let output = scope.invoke(StrictJsonValue(input), operation).await.unwrap().0;
        assert_eq!(output["state"], "exited");
        assert_eq!(output["success"], true, "{operation}: {output}");
        assert_eq!(output["output"]["text"], "ordinary:0");
        assert!(output.get("component_identity").is_none());
    }
    scope.cleanup().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires the real approved embedded OfficeCLI or verified development override"]
async fn real_component_office_pptx_ignores_same_named_path_program() {
    use std::os::unix::fs::PermissionsExt;
    use std::io::Read;
    let root = tempfile::tempdir().unwrap();
    let shadow = root.path().join("officecli");
    std::fs::write(&shadow, "#!/bin/sh\ntouch WRONG_OFFICECLI_EXECUTED\nexit 97\n").unwrap();
    std::fs::set_permissions(&shadow, std::fs::Permissions::from_mode(0o755)).unwrap();
    let child_path = format!("{}:/usr/bin:/bin", root.path().display());
    let (journal, _) = super::super::engine_journal::test_fixture().await;
    let scope = EngineProcessScope::new(root.path(), journal).unwrap();
    for (index, arguments) in [
        vec!["create", "managed.pptx"],
        vec!["add", "managed.pptx", "/", "--type", "slide", "--prop", "layout=blank"],
        vec!["add", "managed.pptx", "/slide[1]", "--type", "shape", "--prop", "text=Managed component verification", "--prop", "x=2cm", "--prop", "y=2cm", "--prop", "width=20cm", "--prop", "height=3cm"],
        vec!["add", "managed.pptx", "/", "--type", "slide", "--prop", "layout=blank"],
        vec!["validate", "managed.pptx"],
    ].into_iter().enumerate() {
        let output = scope.invoke(StrictJsonValue(serde_json::json!({
            "component":"officecli", "args":arguments,
            "env":{"PATH":child_path}, "timeout_ms":30000,
        })), &format!("real-office-{index}")).await.unwrap().0;
        assert_eq!(output["state"], "exited", "{output}");
        assert_eq!(output["success"], true, "{output}");
        assert_eq!(output["component_identity"]["component"], "officecli");
        assert_eq!(output["component_identity"]["version"], "1.0.156");
        assert_eq!(output["cleanup"]["reaped"], true);
    }
    assert!(!root.path().join("WRONG_OFFICECLI_EXECUTED").exists());
    let operations_before = scope.state.lock().await.operations.len();
    let full_environment: BTreeMap<_, _> = (0..128).map(|index| (format!("FLAG_{index}"), "keep".to_owned())).collect();
    let rejected = scope.invoke(StrictJsonValue(serde_json::json!({
        "component":"officecli", "args":["--version"], "env":full_environment,
    })), "real-office-environment-budget").await.unwrap().0;
    assert_eq!(rejected["state"], "not_started");
    assert_eq!(rejected["user_code_started"], false);
    assert_eq!(rejected["code"], "INVALID_PAYLOAD");
    assert_eq!(scope.state.lock().await.operations.len(), operations_before);
    let file = std::fs::File::open(root.path().join("managed.pptx")).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.file_names().filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml")).count(), 2);
    assert!(archive.by_name("ppt/slides/slide1.xml").is_ok());
    assert!(archive.by_name("ppt/slides/slide2.xml").is_ok());
    let mut first_slide = String::new();
    archive.by_name("ppt/slides/slide1.xml").unwrap().read_to_string(&mut first_slide).unwrap();
    assert!(first_slide.contains("Managed component verification"));
    scope.cleanup().await.unwrap();
}
