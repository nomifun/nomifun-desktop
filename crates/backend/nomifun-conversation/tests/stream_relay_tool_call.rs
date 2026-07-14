use std::sync::Arc;

use nomifun_ai_agent::{
    AgentStreamEvent,
    protocol::events::{FinishEventData, ToolCallEventData, ToolCallStatus},
};
use nomifun_common::now_ms;
use nomifun_conversation::stream_relay::StreamRelay;
use nomifun_db::{
    IConversationRepository, SortOrder, SqliteConversationRepository, init_database_memory, models::ConversationRow,
};
use nomifun_realtime::WebSocketManager;
use serde_json::json;
use tokio::sync::broadcast;

async fn setup_repo() -> (Arc<SqliteConversationRepository>, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let repo = Arc::new(SqliteConversationRepository::new(db.pool().clone()));
    let now = now_ms();
    repo.create(&ConversationRow {
        id: 1,
        user_id: "system_default_user".into(),
        name: "Tool call test".into(),
        r#type: "nomi".into(),
        extra: "{}".into(),
        delegation_policy: "automatic".into(),
        execution_model_pool: None,
        decision_policy: "automatic".into(),
        execution_template_id: None,
        model: None,
        status: Some("running".into()),
        source: Some("nomifun".into()),
        channel_chat_id: None,
        pinned: false,
        pinned_at: None,
        cron_job_id: None,
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap();

    (repo, db)
}

#[tokio::test]
async fn run_tool_call_with_empty_call_id_is_not_persisted() {
    let (repo, _db) = setup_repo().await;
    let bus = Arc::new(WebSocketManager::new());
    let (tx, _) = broadcast::channel(64);

    let relay = StreamRelay::new(
        "1".into(),
        "asst-1".into(),
        "system_default_user".into(),
        repo.clone(),
        bus,
        None,
    );

    let rx = tx.subscribe();
    tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
        call_id: "".into(),
        name: "Glob".into(),
        args: json!({"pattern": "*.rs"}),
        status: ToolCallStatus::Running,
        input: Some(json!({"pattern": "*.rs"})),
        output: None,
        description: None,
    }))
    .unwrap();
    tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

    relay.consume(rx).await;

    let messages = repo.get_messages(1, 1, 100, SortOrder::Asc).await.unwrap();

    assert!(
        messages.items.iter().all(|row| row.r#type != "tool_call"),
        "empty call_id tool_call must not be persisted"
    );
}
