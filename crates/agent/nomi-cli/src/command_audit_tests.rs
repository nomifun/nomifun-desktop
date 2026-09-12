use super::*;

fn session_config(endpoint: String) -> Config {
    let file = config::ConfigFile::default();
    Config {
        provider_label: "openai".into(),
        provider: config::ProviderType::OpenAI,
        api_key: "review-test-key".into(),
        base_url: endpoint,
        model: "review-test".into(),
        output_max_tokens: Some(100),
        max_turns: Some(1),
        system_prompt: Some("Local test".into()),
        project_instructions: config::ProjectInstructionsConfig {
            project_doc_max_bytes: 0,
            ..Default::default()
        },
        thinking: None,
        prompt_caching: false,
        compat: nomi_config::compat::ProviderCompat::openai_defaults(),
        tools: file.tools,
        session: config::SessionConfig {
            enabled: false,
            ..Default::default()
        },
        compact: file.compact,
        plan: file.plan,
        file_cache: file.file_cache,
        hooks: nomi_config::hooks::HooksConfig {
            stop: vec![nomi_config::hooks::HookDef {
                name: "shutdown-proof".into(),
                tool_match: vec![],
                file_match: vec![],
                command: "echo stopped > shutdown-proof.txt".into(),
                timeout_ms: 5000,
            }],
            ..Default::default()
        },
        bedrock: None,
        vertex: None,
        mcp: file.mcp,
        logging: file.logging,
    }
}

#[tokio::test]
async fn stop_before_first_message_runs_the_common_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(ProtocolCommand::Stop).await.unwrap();
    drop(tx);
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_json_stream_mode_with_reader(
            session_config("http://127.0.0.1:1".into()),
            temp.path().to_str().unwrap(),
            None,
            None,
            || rx,
        ),
    )
    .await
    .expect("shutdown timed out")
    .unwrap();
    assert!(temp.path().join("shutdown-proof.txt").is_file());
}

#[tokio::test]
async fn input_eof_cancels_an_active_request_and_runs_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(ProtocolCommand::Message {
        msg_id: "test".into(),
        content: "hello".into(),
    })
    .await
    .unwrap();
    let session = run_json_stream_mode_with_reader(
        session_config(endpoint),
        temp.path().to_str().unwrap(),
        None,
        None,
        || rx,
    );
    let disconnect = async {
        let (connection, _) = listener.accept().await.unwrap();
        drop(tx); // EOF only after the real provider request is in flight.
        connection // Keep the server side open; EOF must cancel the request.
    };
    let (result, _connection) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(session, disconnect)
    })
    .await
    .expect("EOF did not stop the active request");
    result.unwrap();
    assert!(temp.path().join("shutdown-proof.txt").is_file());
}

#[test]
fn queued_config_updates_preserve_partial_updates_and_order() {
    let mut pending = Vec::new();
    let updates = [
        (
            Some("new-model".into()),
            Some("enabled".into()),
            Some(12000),
            None,
            None,
        ),
        (None, None, None, Some("high".into()), Some("off".into())),
        (
            None,
            Some("disabled".into()),
            None,
            Some(String::new()),
            None,
        ),
    ];
    for update in &updates {
        assert!(queue_config_update(&mut pending, update.clone()));
    }
    assert_eq!(pending, updates);
}

#[test]
fn config_queue_rejects_overflow_without_displacing_accepted_updates() {
    let mut pending = Vec::new();
    for index in 0..MAX_PENDING_CONFIG_UPDATES {
        assert!(queue_config_update(
            &mut pending,
            (Some(index.to_string()), None, None, None, None)
        ));
    }
    let before = pending.clone();
    assert!(!queue_config_update(
        &mut pending,
        (Some("overflow".into()), None, None, None, None)
    ));
    assert_eq!(pending, before);
}
