use std::time::Duration;

use nomifun_ai_agent::AgentStreamEvent;
use tokio::sync::broadcast;
use tracing::warn;

use super::{TeammateManager, WAKE_TIMEOUT_MS, WakePayload, WakeTimeoutHandler};
use crate::error::TeamError;

impl TeammateManager {
    pub async fn build_wake_payload(&self, slot_id: &str) -> Result<WakePayload, TeamError> {
        let agent = self.get_agent(slot_id).await?;
        let tasks = self.task_board.list_tasks(&self.team_id).await?;
        let unread = self.mailbox.read_unread(&self.team_id, slot_id).await?;
        Ok(WakePayload {
            agent,
            tasks,
            unread_messages: unread,
        })
    }

    pub fn acquire_wake_lock(&self, slot_id: &str) -> bool {
        self.active_wakes.insert(slot_id.to_owned())
    }

    pub fn release_wake_lock(&self, slot_id: &str) {
        self.active_wakes.remove(slot_id);
    }

    pub fn is_wake_active(&self, slot_id: &str) -> bool {
        self.active_wakes.contains(slot_id)
    }

    pub fn clear_wake_timeout(&self, slot_id: &str) {
        if let Some((_, handle)) = self.wake_timeouts.remove(slot_id) {
            handle.abort();
        }
    }

    pub fn arm_wake_timeout(
        &self,
        slot_id: &str,
        stream_rx: broadcast::Receiver<AgentStreamEvent>,
        on_timeout: WakeTimeoutHandler,
    ) {
        let slot_id_owned = slot_id.to_owned();
        let map = self.wake_timeouts.clone();
        let map_for_task = map.clone();
        let handle = tokio::spawn(async move {
            let mut rx = stream_rx;
            let timeout = Duration::from_millis(WAKE_TIMEOUT_MS);
            let sleep = tokio::time::sleep(timeout);
            tokio::pin!(sleep);

            let timed_out = loop {
                tokio::select! {
                    event = rx.recv() => {
                        match event {
                            Ok(AgentStreamEvent::Finish(_)) => break false,
                            Ok(AgentStreamEvent::Error(_)) => break false,
                            Err(broadcast::error::RecvError::Closed) => break false,
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(slot_id = %slot_id_owned, skipped = n, "wake watchdog lagged");
                                sleep.as_mut().reset(tokio::time::Instant::now() + timeout);
                            }
                            Ok(_) => {
                                sleep.as_mut().reset(tokio::time::Instant::now() + timeout);
                            }
                        }
                    }
                    _ = &mut sleep => break true,
                }
            };

            if timed_out {
                on_timeout(slot_id_owned.clone()).await;
            }

            map_for_task.remove(&slot_id_owned);
        });

        if let Some(old) = map.insert(slot_id.to_owned(), handle) {
            old.abort();
        }
    }
}
