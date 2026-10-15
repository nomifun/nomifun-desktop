use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_channel::manager::ChannelManager;
use nomifun_db::{Database, SqliteChannelRepository, init_database_memory};
use nomifun_realtime::UserEventSink;

struct DiscardEvents;

impl UserEventSink for DiscardEvents {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
}

pub async fn manager_fixture(owner_id: &str) -> (ChannelManager, Database) {
    let db = init_database_memory().await.unwrap();
    let repo = Arc::new(SqliteChannelRepository::new(db.pool().clone()));
    let (message_tx, _message_rx) = tokio::sync::mpsc::channel(16);
    let manager = ChannelManager::new(repo, Arc::new(DiscardEvents), owner_id, [0x42; 32], message_tx);
    (manager, db)
}
