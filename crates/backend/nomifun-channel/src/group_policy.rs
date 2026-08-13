//! In-process linearization fence for per-bot group admission policy.
//!
//! A policy writer holds the exclusive permit while the durable policy and
//! its non-direct sessions/queues are replaced atomically. Group admission
//! and queue delivery hold a shared permit until their durable turn handoff,
//! so an update cannot return while work admitted by the old policy can still
//! create or use a channel session.

use std::fmt;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// Per-plugin policy gates shared by the manager, inbound loop, and queue drain.
///
/// Entries intentionally remain for the process lifetime. The number of bot
/// rows is bounded, and retaining entries avoids an ABA race while replacing a
/// lock for a plugin id.
#[derive(Default)]
pub struct GroupPolicyFence {
    gates: DashMap<String, Arc<RwLock<()>>>,
}

impl GroupPolicyFence {
    fn gate(&self, plugin_id: &str) -> Arc<RwLock<()>> {
        self.gates
            .entry(plugin_id.to_owned())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    pub async fn read(&self, plugin_id: &str) -> GroupPolicyReadPermit {
        GroupPolicyReadPermit {
            plugin_id: plugin_id.to_owned(),
            _guard: self.gate(plugin_id).read_owned().await,
        }
    }

    pub async fn write(&self, plugin_id: &str) -> GroupPolicyWritePermit {
        GroupPolicyWritePermit {
            plugin_id: plugin_id.to_owned(),
            _guard: self.gate(plugin_id).write_owned().await,
        }
    }
}

/// Capability proving that this task is inside one plugin's admission fence.
pub struct GroupPolicyReadPermit {
    plugin_id: String,
    _guard: OwnedRwLockReadGuard<()>,
}

impl GroupPolicyReadPermit {
    pub fn is_for(&self, plugin_id: &str) -> bool {
        self.plugin_id == plugin_id
    }
}

impl fmt::Debug for GroupPolicyReadPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupPolicyReadPermit")
            .field("plugin_id", &self.plugin_id)
            .finish_non_exhaustive()
    }
}

/// Exclusive capability used by policy mutation and retirement.
pub struct GroupPolicyWritePermit {
    plugin_id: String,
    _guard: OwnedRwLockWriteGuard<()>,
}

impl fmt::Debug for GroupPolicyWritePermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupPolicyWritePermit")
            .field("plugin_id", &self.plugin_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn writer_waits_for_existing_admission_and_blocks_new_admission() {
        let fence = Arc::new(GroupPolicyFence::default());
        let first = fence.read("plugin-a").await;

        let writer_fence = Arc::clone(&fence);
        let mut writer = tokio::spawn(async move { writer_fence.write("plugin-a").await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut writer)
                .await
                .is_err()
        );

        let next_fence = Arc::clone(&fence);
        let mut next = tokio::spawn(async move { next_fence.read("plugin-a").await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut next)
                .await
                .is_err(),
            "tokio's fair lock must not bypass a queued writer"
        );

        drop(first);
        let writer_permit = tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .unwrap()
            .unwrap();
        assert!(!next.is_finished());
        drop(writer_permit);
        tokio::time::timeout(Duration::from_secs(1), next)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn different_plugins_do_not_block_each_other() {
        let fence = GroupPolicyFence::default();
        let _writer = fence.write("plugin-a").await;
        tokio::time::timeout(Duration::from_millis(100), fence.read("plugin-b"))
            .await
            .expect("an unrelated plugin must have an independent gate");
    }
}
