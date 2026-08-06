//! LAN WebSocket source.
//!
//! LAN links are push-driven: axum hands us an already-upgraded socket from a
//! request handler. To fit the pull-shaped [`RobotLinkSource`] contract (which a
//! future outbound relay source needs), the handler `offer`s links into a queue
//! and `run` drains it.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use crate::link::{AcceptedLink, LinkError, RobotLinkSource};

/// Handle given to HTTP handlers so they can hand off upgraded sockets.
#[derive(Clone)]
pub struct LanLinkAcceptor {
    tx: mpsc::Sender<AcceptedLink>,
}

impl LanLinkAcceptor {
    /// Hand an authenticated link to the gateway.
    pub async fn offer(&self, link: AcceptedLink) -> Result<(), LinkError> {
        self.tx.send(link).await.map_err(|_| LinkError::Closed)
    }
}

/// The LAN source: drains what handlers offered.
pub struct LanWsSource {
    rx: Mutex<mpsc::Receiver<AcceptedLink>>,
}

impl LanWsSource {
    /// Build the source and the handle its HTTP handlers use.
    pub fn new() -> (Arc<Self>, LanLinkAcceptor) {
        let (tx, rx) = mpsc::channel(8);
        (Arc::new(Self { rx: Mutex::new(rx) }), LanLinkAcceptor { tx })
    }
}

#[async_trait::async_trait]
impl RobotLinkSource for LanWsSource {
    fn name(&self) -> &'static str {
        "lan-ws"
    }

    async fn run(self: Arc<Self>, accept: mpsc::Sender<AcceptedLink>) -> anyhow::Result<()> {
        let mut rx = self.rx.lock().await;
        while let Some(link) = rx.recv().await {
            if accept.send(link).await.is_err() {
                break;
            }
        }
        Ok(())
    }
}
