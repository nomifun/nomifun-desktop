use std::time::Instant;

use super::{FINALIZE_DEDUP_WINDOW, TeammateManager};

impl TeammateManager {
    pub fn begin_finalize(&self, conversation_id: &str) -> bool {
        let now = Instant::now();
        let should_proceed = !matches!(
            self.finalized_turns.get(conversation_id),
            Some(entry) if now.duration_since(*entry.value()) < FINALIZE_DEDUP_WINDOW
        );
        if should_proceed {
            self.finalized_turns.insert(conversation_id.to_owned(), now);
            let map = self.finalized_turns.clone();
            let key = conversation_id.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(FINALIZE_DEDUP_WINDOW).await;
                map.remove(&key);
            });
        }
        should_proceed
    }

    pub fn clear_finalized_turn(&self, conversation_id: &str) {
        self.finalized_turns.remove(conversation_id);
    }
}
