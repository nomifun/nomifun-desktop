use super::*;

#[tokio::test]
async fn send_message_evicts_acp_task_after_terminal_error() {
    let (svc, _broadcaster, _repo, _default_task_mgr) = make_service();
    let task_mgr = Arc::new(MockTaskManager::new());
    let conv = svc.create("user_1", make_create_req()).await.unwrap();

    let scripted_agent = Arc::new(ScriptedAgent::new(
        &conv.id.to_string(),
        vec![vec![AgentStreamEvent::Error(ErrorEventData::legacy(
            "Agent completed the turn without producing visible output.",
            Some(AgentErrorCode::UnknownUpstreamError),
        ))]],
    ));
    task_mgr.insert_agent(&conv.id.to_string(), AgentInstance::Mock(scripted_agent));

    let task_mgr_dyn: Arc<dyn IWorkerTaskManager> = task_mgr.clone();
    svc.send_message("user_1", &conv.id.to_string(), make_send_req(), &task_mgr_dyn)
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if task_mgr.kill_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("ACP terminal error should evict the cached task");
    wait_for_turn_released(&svc, &conv.id.to_string()).await;

    assert_eq!(task_mgr.active_count(), 0);
    assert_eq!(
        task_mgr.kill_records(),
        vec![(conv.id.to_string(), Some(AgentKillReason::AgentErrorRecovery))]
    );
}

#[tokio::test]
async fn send_message_clears_persisted_acp_model_after_model_not_found() {
    let acp_session_repo = Arc::new(StubAcpSessionRepo::default());
    let (svc, _broadcaster, repo, _default_task_mgr) = make_service_with_resolver_and_acp_session_repo(
        Arc::new(FixedSkillResolver { names: vec![] }),
        acp_session_repo.clone(),
    );
    let task_mgr = Arc::new(MockTaskManager::new());
    let conv = svc.create("user_1", make_create_req()).await.unwrap();
    repo.update(
        conv.id,
        &ConversationRowUpdate {
            extra: Some(
                serde_json::to_string(&json!({
                    "workspace": "/project",
                    "current_model_id": "deepseek-v4-pro",
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let scripted_agent = Arc::new(ScriptedAgent::new(
        &conv.id.to_string(),
        vec![vec![AgentStreamEvent::Error(ErrorEventData::legacy(
            "The configured model was not found by the provider.",
            Some(AgentErrorCode::UserLlmProviderModelNotFound),
        ))]],
    ));
    task_mgr.insert_agent(&conv.id.to_string(), AgentInstance::Mock(scripted_agent));

    let task_mgr_dyn: Arc<dyn IWorkerTaskManager> = task_mgr.clone();
    svc.send_message("user_1", &conv.id.to_string(), make_send_req(), &task_mgr_dyn)
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let saves = acp_session_repo.runtime_state_saves();
            if saves
                .iter()
                .any(|call| call.conversation_id == conv.id.to_string() && call.current_model_id == Some(None))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("model_not_found should clear persisted ACP model");
    wait_for_turn_released(&svc, &conv.id.to_string()).await;

    assert_eq!(task_mgr.active_count(), 0);
    assert_eq!(
        acp_session_repo.runtime_state_saves(),
        vec![RuntimeStateSaveCall {
            conversation_id: conv.id.to_string(),
            current_model_id: Some(None),
        }]
    );

    let row = repo.get(conv.id).await.unwrap().unwrap();
    let extra: serde_json::Value = serde_json::from_str(&row.extra).unwrap();
    assert!(extra.get("workspace").is_some());
    assert!(
        extra.get("current_model_id").is_none(),
        "model_not_found must clear conversation.extra.current_model_id so rebuild cannot reseed stale desired model"
    );
}

#[tokio::test]
async fn send_message_does_not_clear_persisted_acp_model_for_other_terminal_errors() {
    let acp_session_repo = Arc::new(StubAcpSessionRepo::default());
    let (svc, _broadcaster, _repo, _default_task_mgr) = make_service_with_resolver_and_acp_session_repo(
        Arc::new(FixedSkillResolver { names: vec![] }),
        acp_session_repo.clone(),
    );
    let task_mgr = Arc::new(MockTaskManager::new());
    let conv = svc.create("user_1", make_create_req()).await.unwrap();

    let scripted_agent = Arc::new(ScriptedAgent::new(
        &conv.id.to_string(),
        vec![vec![AgentStreamEvent::Error(ErrorEventData::legacy(
            "Unknown upstream error.",
            Some(AgentErrorCode::UnknownUpstreamError),
        ))]],
    ));
    task_mgr.insert_agent(&conv.id.to_string(), AgentInstance::Mock(scripted_agent));

    let task_mgr_dyn: Arc<dyn IWorkerTaskManager> = task_mgr.clone();
    svc.send_message("user_1", &conv.id.to_string(), make_send_req(), &task_mgr_dyn)
        .await
        .unwrap();
    wait_for_turn_released(&svc, &conv.id.to_string()).await;

    assert_eq!(task_mgr.active_count(), 0);
    assert!(acp_session_repo.runtime_state_saves().is_empty());
}
