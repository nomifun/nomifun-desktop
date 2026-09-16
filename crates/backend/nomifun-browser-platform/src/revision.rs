//! Coalescing change notifications. Consumers always reload a complete snapshot.

#[derive(Debug)]
pub struct BrowserRevision(tokio::sync::watch::Sender<u64>);

impl Default for BrowserRevision {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(0).0)
    }
}

impl BrowserRevision {
    pub fn current(&self) -> u64 {
        *self.0.borrow()
    }
    pub fn bump(&self) {
        self.0.send_modify(|revision| *revision += 1);
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.subscribe()
    }
}
