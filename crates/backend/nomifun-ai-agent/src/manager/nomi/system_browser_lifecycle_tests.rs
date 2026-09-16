use nomifun_browser_platform::system_browser::SystemBrowserHost;
use std::time::Duration;

async fn attach_system_browser(
    agent: &mut NomiAgentManager,
) -> Arc<crate::system_browser::tests::Host> {
    let host = crate::system_browser::tests::fixture(Some(agent.runtime.clone()));
    agent.system_browser_session = Some(
        crate::system_browser::SystemBrowserSession::bind(
            host.clone(),
            host.binding(),
            "user",
            "conversation",
        )
        .await
        .unwrap(),
    );
    host
}
fn message() -> SendMessageData {
    SendMessageData {
        content: "Inspect my authorized browser tab".into(),
        msg_id: "system-browser-message".into(),
        source_message_id: None,
        files: vec![],
        inject_skills: vec![],
        origin: None,
    }
}

#[tokio::test]
async fn model_system_browser_call_is_independent_and_finishes_after_terminal() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolUse {
                id: "system-tabs".into(),
                name: "nomi_system_browser".into(),
                input: serde_json::json!({"operation":"tabs"}),
                extra: None,
            },
            LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            },
        ],
        vec![
            LlmEvent::TextDelta("Connect the browser and authorize a tab first.".into()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            },
        ],
    ]));
    let mut agent = make_agent_with_provider(provider.clone());
    let host = attach_system_browser(&mut agent).await;
    assert!(agent.browser_workspace.is_none());
    assert!(agent.engine.get_mut().registry_mut().register(Box::new(
        crate::system_browser::SystemBrowserTool::new(agent.system_browser_turn.clone())
    )));
    agent.send_message(message()).await.unwrap();
    assert_eq!(host.0.invokes.load(Ordering::Relaxed), 1);
    assert_eq!(provider.calls(), 2);
    assert!(host.0.settled_before_terminal.load(Ordering::Acquire));
    assert!(host.0.finished_after_terminal.load(Ordering::Acquire));
    assert_eq!(
        *host.0.events.lock().unwrap(),
        vec!["workspace", "begin", "settle", "finish"]
    );
}

#[tokio::test]
async fn stopping_system_browser_run_cancels_then_settles_before_terminal() {
    let provider = Arc::new(BlockingProvider::new());
    let mut agent = make_agent_with_provider(provider.clone());
    let host = attach_system_browser(&mut agent).await;
    let agent = Arc::new(agent);
    let running = tokio::spawn({
        let agent = agent.clone();
        async move { agent.send_message(message()).await }
    });
    provider.called.acquire().await.unwrap().forget();
    agent.cancel().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let events = host.0.events.lock().unwrap();
    let cancelled = events.iter().position(|event| *event == "cancel").unwrap();
    let settled = events.iter().position(|event| *event == "settle").unwrap();
    let finished = events.iter().position(|event| *event == "finish").unwrap();
    assert!(cancelled < settled && settled < finished);
    assert!(host.0.settled_before_terminal.load(Ordering::Acquire));
    assert!(host.0.finished_after_terminal.load(Ordering::Acquire));
}

#[tokio::test]
async fn provider_error_settles_system_browser_and_finishes_after_error_terminal() {
    let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Error(
        "fixture provider error".into(),
    )]]));
    let mut agent = make_agent_with_provider(provider);
    let host = attach_system_browser(&mut agent).await;
    assert!(agent.send_message(message()).await.is_err());
    assert_eq!(agent.runtime.status(), Some(ConversationStatus::Finished));
    assert!(host.0.settled_before_terminal.load(Ordering::Acquire));
    assert!(host.0.finished_after_terminal.load(Ordering::Acquire));
}
