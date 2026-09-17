use nomifun_browser_platform::{
    bound_resource::BoundBrowserProviderResource,
    product::{
        BrowserCapabilityAction, BrowserProviderDescriptor, BrowserResourceBinding,
        BrowserSessionAuthority,
    },
    run_guard::{BrowserInputState, NativeInputGate, RunAdmissionError},
    runtime::*,
    workspace::{BrowserResource, BrowserResourceService},
};

struct BrowserFixture {
    navigations: AtomicUsize,
    runtime: AgentRuntimeState,
    locked: AtomicBool,
    unlock_after_terminal: AtomicBool,
    hold_lock: AtomicBool,
    lock_entered: tokio::sync::Semaphore,
    lock_release: tokio::sync::Semaphore,
}

#[tokio::test]
async fn retired_project_browser_options_do_not_create_a_browser_tool() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".nomi.toml"), "[tools.browser]\nenabled = true\nheadless = false\n").unwrap();
    let mut config = make_test_config();
    config.session_directory = root.path().join("sessions");
    let workspace = root.path().to_string_lossy().into_owned();
    let cli = nomi_config::config::CliArgs {
        project_dir: Some(root.path().to_path_buf()),
        provider: Some(config.provider.clone()),
        api_key: Some(config.api_key.clone()),
        base_url: None,
        model: Some(config.model.clone()),
        max_tokens: None,
        max_turns: None,
        system_prompt: None,
        profile: None,
    };
    let resolved = nomi_config::config::Config::resolve(&cli).unwrap();
    assert!(serde_json::to_value(&resolved.tools).unwrap().get("browser").is_none(),
        "old options are no longer part of the resolved configuration");
    let agent = NomiAgentManager::new(
        nomifun_common::ConversationId::new().into_string(), workspace, config,
        None, None, None, None, Vec::new(), None, None, Vec::new(), None,
    ).await.unwrap();
    assert!(!agent.engine.lock().await.tool_names().iter().any(|name| name == "Browser"),
        "only the native Workspace may supply the conversation Browser tool");
    agent.kill_and_wait(None).await.unwrap();
}

struct BrowserFixtureFactory(Arc<BrowserFixture>);

#[async_trait::async_trait]
impl BrowserRuntimeFactory for BrowserFixtureFactory {
    async fn create(
        &self,
        _: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        Ok(self.0.clone())
    }
}

#[async_trait::async_trait]
impl NativeInputGate for BrowserFixture {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.locked.store(true, Ordering::SeqCst);
        if self.hold_lock.load(Ordering::SeqCst) {
            self.lock_entered.add_permits(1);
            self.lock_release.acquire().await.unwrap().forget();
        }
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        Ok(())
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        self.unlock_after_terminal.store(
            self.runtime.status() == Some(ConversationStatus::Finished),
            Ordering::SeqCst,
        );
        self.locked.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl BrowserRuntime for BrowserFixture {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> {
        None
    }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        Ok(BrowserRuntimeSnapshot {
            downloads: vec![],
            runtime_generation: 1,
            revision: 1,
            active_tab_id: None,
            tabs: vec![],
        })
    }
    async fn execute(
        &self,
        command: BrowserTabCommand,
        _: tokio_util::sync::CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if matches!(
            command,
            BrowserTabCommand::Create { .. } | BrowserTabCommand::Navigate { .. }
        ) {
            self.navigations.fetch_add(1, Ordering::SeqCst);
        }
        self.snapshot().await
    }
    async fn close(&self) -> Result<(), WorkspaceError> {
        Ok(())
    }
}

async fn attach_browser(
    agent: &mut NomiAgentManager,
) -> (Arc<BrowserResource>, Arc<BrowserFixture>) {
    let fixture = Arc::new(BrowserFixture {
        navigations: AtomicUsize::new(0),
        runtime: agent.runtime.clone(),
        locked: AtomicBool::new(false),
        unlock_after_terminal: AtomicBool::new(false),
        hold_lock: AtomicBool::new(false),
        lock_entered: tokio::sync::Semaphore::new(0),
        lock_release: tokio::sync::Semaphore::new(0),
    });
    let service = BrowserResourceService::new(Arc::new(BrowserFixtureFactory(fixture.clone())));
    let provider = BrowserProviderDescriptor::managed("managed-test", "managed-test-lock").unwrap();
    let binding = BrowserResourceBinding::new(
        "browser-binding",
        "browser-resource",
        "owner",
        provider,
        BrowserCapabilityAction::all().map(BrowserCapabilityAction::resource_operation),
    )
    .unwrap();
    let authority = BrowserSessionAuthority::new(
        "owner",
        "conv-auto-continue",
        BrowserCapabilityAction::all(),
        binding,
    )
    .unwrap();
    let workspace = service
        .ensure(
            authority,
            BrowserProfile::Ephemeral,
        )
        .await
        .unwrap();
    workspace
        .user_command(BrowserTabCommand::Create {
            url: "http://localhost:3000".into(),
        })
        .await
        .unwrap();
    agent.browser_resource = Some(BoundBrowserProviderResource::Managed(workspace.clone()));
    (workspace, fixture)
}

fn message() -> SendMessageData {
    SendMessageData {
        content: "Explain this page".into(),
        msg_id: "browser-lifecycle-message".into(),
        source_message_id: None,
        files: vec![],
        inject_skills: vec![],
        origin: None,
    }
}

#[tokio::test]
async fn retained_browser_invocation_cannot_adopt_a_new_turn() {
    let provider=Arc::new(BlockingProvider::new());
    let mut agent=make_agent_with_provider(provider);
    let (workspace,fixture)=attach_browser(&mut agent).await;
    let resource = BoundBrowserProviderResource::Managed(workspace.clone());
    agent.browser_turn.begin(&resource).await.unwrap();
    let old=agent.browser_turn.current().unwrap();
    agent.browser_turn.finish().await.unwrap();
    agent.browser_turn.begin(&resource).await.unwrap();
    let before=fixture.navigations.load(Ordering::SeqCst);
    let crate::manager::nomi::browser_lifecycle::BrowserTurn::Managed(old) = old else {
        panic!("managed turn")
    };
    assert!(matches!(old.command(BrowserTabCommand::Create{url:"https://example.com".into()}).await,Err(WorkspaceError::Admission(RunAdmissionError::StaleRun))));
    assert!(matches!(old.tabs().await,Err(WorkspaceError::Admission(RunAdmissionError::StaleRun))));
    assert_eq!(before,fixture.navigations.load(Ordering::SeqCst));
    agent.browser_turn.finish().await.unwrap();
}

#[tokio::test]
async fn model_browser_call_uses_the_current_conversation_runtime() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolUse {
                id: "native-nav".into(),
                name: "Browser".into(),
                input: serde_json::json!({"operation":"navigate","url":"http://localhost:3000"}),
                extra: None,
            },
            LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            },
        ],
        vec![
            LlmEvent::TextDelta("Navigation complete.".into()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            },
        ],
    ]));
    let mut agent = make_agent_with_provider(provider.clone());
    let (workspace, fixture) = attach_browser(&mut agent).await;
    fixture.navigations.store(0, Ordering::SeqCst);
    let tool = crate::manager::nomi::browser_tool::ConversationBrowserTool::new(
        agent.browser_turn.clone(),
        [nomifun_agent_contracts::ActionId::from("browser/navigate")]
            .into_iter()
            .collect(),
        nomifun_browser_platform::product::BrowserProviderKind::Managed,
    );
    assert!(
        agent
            .engine
            .get_mut()
            .registry_mut()
            .register(Box::new(tool))
    );
    let mut data = message();
    data.content = "Open http://localhost:3000 in the browser".into();
    agent.send_message(data).await.unwrap();
    assert_eq!(fixture.navigations.load(Ordering::SeqCst), 1);
    assert_eq!(provider.calls(), 2);
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}

#[tokio::test]
async fn successful_turn_without_browser_tool_releases_input_after_its_terminal() {
    let provider = Arc::new(ScriptedProvider::new(vec![vec![
        LlmEvent::TextDelta("Hello!".into()),
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        },
    ]]));
    let mut agent = make_agent_with_provider(provider);
    assert!(agent.engine.get_mut().registry_mut().get("Browser").is_none());
    let (workspace, fixture) = attach_browser(&mut agent).await;
    let mut data = message();
    data.content = "Hello".into();
    agent.send_message(data).await.unwrap();
    assert!(fixture.unlock_after_terminal.load(Ordering::SeqCst));
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}

#[tokio::test]
async fn stopping_during_native_lock_never_starts_the_model_afterwards() {
    let provider = Arc::new(BlockingProvider::new());
    let mut agent = make_agent_with_provider(provider.clone());
    let (workspace, fixture) = attach_browser(&mut agent).await;
    fixture.hold_lock.store(true, Ordering::SeqCst);
    let agent = Arc::new(agent);
    let pending = tokio::spawn({
        let agent = agent.clone();
        async move { agent.send_message(message()).await }
    });
    fixture.lock_entered.acquire().await.unwrap().forget();
    agent.cancel().await.unwrap();
    fixture.lock_release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}

#[tokio::test]
async fn cancel_keeps_native_input_locked_until_agent_terminal() {
    let provider = Arc::new(BlockingProvider::new());
    let mut agent = make_agent_with_provider(provider.clone());
    let (workspace, fixture) = attach_browser(&mut agent).await;
    let agent = Arc::new(agent);
    let running = tokio::spawn({
        let agent = agent.clone();
        async move { agent.send_message(message()).await }
    });
    provider.called.acquire().await.unwrap().forget();
    assert!(fixture.locked.load(Ordering::SeqCst));
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::AgentRunning
    );
    assert!(matches!(
        workspace
            .user_command(BrowserTabCommand::Create {
                url: "http://localhost:3000".into()
            })
            .await,
        Err(WorkspaceError::Admission(
            RunAdmissionError::UserInputLocked
        ))
    ));
    agent.cancel().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(fixture.unlock_after_terminal.load(Ordering::SeqCst));
    assert!(!fixture.locked.load(Ordering::SeqCst));
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}

#[tokio::test]
async fn provider_failure_unlocks_only_after_the_error_terminal() {
    let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Error(
        "fixture provider error".into(),
    )]]));
    let mut agent = make_agent_with_provider(provider);
    let (workspace, fixture) = attach_browser(&mut agent).await;
    assert!(agent.send_message(message()).await.is_err());
    assert_eq!(agent.runtime.status(), Some(ConversationStatus::Finished));
    assert!(fixture.unlock_after_terminal.load(Ordering::SeqCst));
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}

#[tokio::test]
async fn unwinding_turn_retains_native_authority_until_cleanup_terminal() {
    let provider = Arc::new(BlockingProvider::new());
    let mut agent = make_agent_with_provider(provider.clone());
    let (workspace, fixture) = attach_browser(&mut agent).await;
    let agent = Arc::new(agent);
    let running = tokio::spawn({
        let agent = agent.clone();
        async move { agent.send_message(message()).await }
    });
    provider.called.acquire().await.unwrap().forget();
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert!(
        agent
            .turn_teardown_fence
            .wait_until_clear(std::time::Duration::from_secs(2))
            .await
    );
    assert!(fixture.unlock_after_terminal.load(Ordering::SeqCst));
    assert_eq!(
        workspace.snapshot().await.unwrap().run.input_state,
        BrowserInputState::UserReady
    );
}
