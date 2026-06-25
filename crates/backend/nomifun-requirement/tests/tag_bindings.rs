//! Tests for `RequirementService::tag_bindings`: enumerates conversation +
//! terminal AutoWork bindings, grouped by tag, only for enabled ones.

use std::sync::Arc;

use nomifun_db::models::ConversationRow;
use nomifun_db::{
    CreateTerminalParams, IConversationRepository, IRequirementRepository, ITerminalRepository,
    SqliteConversationRepository, SqliteRequirementRepository, SqliteTerminalRepository, init_database_memory,
};
use nomifun_realtime::EventBroadcaster;
use nomifun_requirement::{RequirementEventEmitter, RequirementService};

#[derive(Default)]
struct NoopBroadcaster;
impl EventBroadcaster for NoopBroadcaster {
    fn broadcast(&self, _event: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
}

fn conv(id: i64, name: &str, autowork_json: &str) -> ConversationRow {
    ConversationRow {
        id,
        user_id: "user_1".into(),
        name: name.into(),
        r#type: "nomi".into(),
        extra: autowork_json.into(),
        model: None,
        status: Some("pending".into()),
        source: None,
        channel_chat_id: None,
        pinned: false,
        pinned_at: None,
        cron_job_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

#[tokio::test]
async fn groups_enabled_conversation_and_terminal_bindings_by_tag() {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('user_1', 'tester', 'h', 0, 0)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let conv_repo: Arc<dyn IConversationRepository> = Arc::new(SqliteConversationRepository::new(pool.clone()));
    let term_repo: Arc<dyn ITerminalRepository> = Arc::new(SqliteTerminalRepository::new(pool.clone()));
    let req_repo: Arc<dyn IRequirementRepository> = Arc::new(SqliteRequirementRepository::new(pool.clone()));

    // Two conversations enabled on tag "x", one disabled, one with no autowork.
    // The id field is ignored on insert (SQLite mints the PK).
    conv_repo
        .create(&conv(1, "Alpha A", r#"{"autowork":{"enabled":true,"tag":"x"}}"#))
        .await
        .unwrap();
    conv_repo
        .create(&conv(2, "Alpha B", r#"{"autowork":{"enabled":true,"tag":"x"}}"#))
        .await
        .unwrap();
    conv_repo
        .create(&conv(
            3,
            "Disabled",
            r#"{"autowork":{"enabled":false,"tag":"x"}}"#,
        ))
        .await
        .unwrap();
    conv_repo.create(&conv(4, "No autowork", "{}")).await.unwrap();

    // One terminal enabled on tag "y". The id is minted by SQLite and returned.
    let term = term_repo
        .create(&CreateTerminalParams {
            name: "Term One".into(),
            cwd: "/tmp".into(),
            command: "claude".into(),
            args: "[]".into(),
            env: None,
            backend: Some("claude".into()),
            mode: None,
            cols: 80,
            rows: 24,
            user_id: "user_1".into(),
        })
        .await
        .unwrap();
    let term_id = term.id;
    term_repo
        .update_autowork(term_id, Some(r#"{"enabled":true,"tag":"y"}"#))
        .await
        .unwrap();

    let svc = RequirementService::new(req_repo, RequirementEventEmitter::new(Arc::new(NoopBroadcaster)))
        .with_conversation_repo(conv_repo)
        .with_terminal_repo(term_repo);
    Box::leak(Box::new(db));

    let groups = svc.tag_bindings("user_1").await.unwrap();

    // tag "x" has the two enabled conversations; "y" has the terminal. Disabled +
    // no-autowork conversations are excluded.
    let x = groups.iter().find(|g| g.tag == "x").expect("tag x present");
    assert_eq!(x.bindings.len(), 2);
    let mut names: Vec<&str> = x.bindings.iter().map(|b| b.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["Alpha A", "Alpha B"]);

    let y = groups.iter().find(|g| g.tag == "y").expect("tag y present");
    assert_eq!(y.bindings.len(), 1);
    assert_eq!(y.bindings[0].target_id, term_id.to_string());

    // No "active" run_state without a live orchestrator (route enriches that).
    assert!(
        groups
            .iter()
            .flat_map(|g| &g.bindings)
            .all(|b| b.run_state == nomifun_api_types::AutoWorkRunState::Idle)
    );
}
