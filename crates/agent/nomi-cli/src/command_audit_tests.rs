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
async fn session_init_failure_runs_shutdown_before_starting_the_reader() {
    let temp = tempfile::tempdir().unwrap();
    let blocked_directory = temp.path().join("not-a-directory");
    std::fs::write(&blocked_directory, b"occupied").unwrap();
    let mut config = session_config("http://127.0.0.1:1".into());
    config.session.enabled = true;
    config.session.directory = blocked_directory.to_str().unwrap().into();

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_json_stream_mode_with_reader(
            config,
            temp.path().to_str().unwrap(),
            None,
            None,
            || panic!("reader must not start after session initialization fails"),
        ),
    )
    .await
    .expect("initialization cleanup timed out")
    .expect_err("session directory is a file");

    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert!(temp.path().join("shutdown-proof.txt").is_file());
}

struct FailingEmitter {
    fails: fn(&ProtocolEvent) -> bool,
    stream_ends: std::sync::atomic::AtomicUsize,
}

impl FailingEmitter {
    fn new(fails: fn(&ProtocolEvent) -> bool) -> Self {
        Self { fails, stream_ends: std::sync::atomic::AtomicUsize::new(0) }
    }
}

impl ProtocolEmitter for FailingEmitter {
    fn emit(&self, event: &ProtocolEvent) -> std::io::Result<()> {
        if matches!(event, ProtocolEvent::StreamEnd { .. }) {
            self.stream_ends.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if (self.fails)(event) {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "output disconnected"))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn sink_ready_failure_stops_before_starting_input_and_runs_shutdown() {
    let temp = tempfile::tempdir().unwrap();

    let writer = Arc::new(FailingEmitter::new(|event| matches!(event, ProtocolEvent::Ready { .. })));
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_json_stream_mode_with_output(
            session_config("http://127.0.0.1:1".into()),
            temp.path().to_str().unwrap(),
            None,
            None,
            || panic!("reader must not start after Ready write fails"),
            writer,
        ),
    ).await.expect("ready failure cleanup timed out").expect_err("broken Ready must fail");
    assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), std::io::ErrorKind::BrokenPipe);
    assert!(temp.path().join("shutdown-proof.txt").is_file());
}



#[tokio::test]
async fn cli_owned_output_errors_run_shutdown_and_preserve_io_error() {
    for command in [
        ProtocolCommand::Ping,
        ProtocolCommand::Message {
            msg_id: "test".into(),
            // A real successful AgentResult without a model/HTTP fixture.
            content: "/help".into(),
        },
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.send(command).await.unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run_json_stream_mode_with_output(
                session_config("http://127.0.0.1:1".into()),
                temp.path().to_str().unwrap(),
                None,
                None,
                || rx,
                Arc::new(FailingEmitter::new(|event| matches!(event, ProtocolEvent::Pong | ProtocolEvent::StreamEnd { .. }))),
            ),
        )
        .await
        .expect("output failure did not stop the command loop")
        .expect_err("broken output must not become a successful exit");

        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe,
        );
        assert!(temp.path().join("shutdown-proof.txt").is_file());
        // Keep input open until after exit: EOF cannot account for shutdown.
        assert!(tx.send(ProtocolCommand::Ping).await.is_err());
    }
}

#[tokio::test]
async fn sink_callback_failure_stops_idle_and_same_poll_completed_turns() {
    for (command, fails) in [
        (ProtocolCommand::SetConfig {
            model: Some("other-model".into()), thinking: None, thinking_budget: None,
            effort: None, compaction: None,
        }, (|event: &ProtocolEvent| matches!(event, ProtocolEvent::ConfigChanged { .. })) as fn(&ProtocolEvent) -> bool),
        (ProtocolCommand::Message { msg_id: "test".into(), content: "/help".into() },
            |event: &ProtocolEvent| matches!(event, ProtocolEvent::Info { .. })),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(command).await.unwrap();
        let writer = Arc::new(FailingEmitter::new(fails));
        let error = tokio::time::timeout(std::time::Duration::from_secs(10),
            run_json_stream_mode_with_output(
                session_config("http://127.0.0.1:1".into()), temp.path().to_str().unwrap(),
                None, None, || rx, writer.clone(),
            ),
        ).await.expect("sink failure must wake command processing").expect_err("sink failure must propagate");
        assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), std::io::ErrorKind::BrokenPipe);
        assert!(temp.path().join("shutdown-proof.txt").is_file());
        assert!(tx.send(ProtocolCommand::Ping).await.is_err());
        assert_eq!(writer.stream_ends.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn cli_output_failure_cancels_an_active_request_and_runs_shutdown() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for fails in [
        (|event: &ProtocolEvent| matches!(event, ProtocolEvent::Pong)) as fn(&ProtocolEvent) -> bool,
        |event: &ProtocolEvent| matches!(event, ProtocolEvent::TextDelta { .. }),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.send(ProtocolCommand::Message { msg_id: "test".into(), content: "hello".into() }).await.unwrap();
        let writer = Arc::new(FailingEmitter::new(fails));
        let session = run_json_stream_mode_with_output(
            session_config(endpoint), temp.path().to_str().unwrap(), None, None, || rx, writer.clone(),
        );
        let partial_response = async {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(connection.read(&mut request).await.unwrap() > 0);
            let frame = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n";
            connection.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{frame}\r\n", frame.len()
            ).as_bytes()).await.unwrap();
            tx.send(ProtocolCommand::Ping).await.unwrap();
            connection // Keep provider and command input open: only output failure may finish.
        };
        let (result, _connection) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(session, partial_response)
        }).await.expect("output failure did not cancel the active request");
        let error = result.expect_err("output failure must reach the caller");
        assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), std::io::ErrorKind::BrokenPipe);
        assert!(temp.path().join("shutdown-proof.txt").is_file());
        assert!(tx.send(ProtocolCommand::Ping).await.is_err());
        assert_eq!(writer.stream_ends.load(std::sync::atomic::Ordering::Relaxed), 0);
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
