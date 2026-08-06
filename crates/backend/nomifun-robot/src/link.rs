//! Transport-agnostic robot links.
//!
//! The session core consumes [`Frame`]s and never learns whether they arrived
//! over a LAN WebSocket or (future) a multiplexed relay tunnel. A
//! [`RobotLinkSource`] owns its own accept loop and hands authenticated
//! [`AcceptedLink`]s to the gateway over a channel, so adding the relay is a
//! new source implementation and zero changes here.

use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc;

/// One wire frame in either direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Bytes),
}

/// Who is on the other end, resolved during authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotIdentity {
    /// Device-Id header (MAC address).
    pub robot_id: String,
    /// Client-Id header (firmware NVS UUID).
    pub client_id: String,
    /// Human-readable peer description for logs (IP, or relay tunnel id).
    pub peer: String,
}

/// Why a link operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("link closed")]
    Closed,
    #[error("transport error: {0}")]
    Transport(String),
}

/// Write half of a link.
#[async_trait::async_trait]
pub trait RobotLinkSink: Send {
    async fn send(&mut self, frame: Frame) -> Result<(), LinkError>;
    async fn close(&mut self);
}

/// Read half of a link.
#[async_trait::async_trait]
pub trait RobotLinkStream: Send {
    async fn next(&mut self) -> Option<Result<Frame, LinkError>>;
}

/// An authenticated link ready for a session actor.
pub struct AcceptedLink {
    pub identity: RobotIdentity,
    pub sink: Box<dyn RobotLinkSink>,
    pub stream: Box<dyn RobotLinkStream>,
}

/// A producer of authenticated links.
///
/// LAN is push-driven (axum hands us an upgraded socket), so `LanWsSource::run`
/// drains an internal queue; a relay source's `run` dials outbound and
/// demultiplexes. Both shapes fit this one interface.
#[async_trait::async_trait]
pub trait RobotLinkSource: Send + Sync {
    /// Stable name for logs.
    fn name(&self) -> &'static str;
    /// Run until shutdown, sending every accepted link to `accept`.
    async fn run(self: Arc<Self>, accept: mpsc::Sender<AcceptedLink>) -> anyhow::Result<()>;
}

/// An in-memory link pair for tests: writes to the sink surface on the stream.
#[cfg(test)]
pub(crate) fn fake_pair() -> (FakeSink, FakeStream) {
    let (tx, rx) = mpsc::channel(16);
    (FakeSink { tx: Some(tx) }, FakeStream { rx })
}

#[cfg(test)]
pub(crate) struct FakeSink {
    tx: Option<mpsc::Sender<Frame>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl RobotLinkSink for FakeSink {
    async fn send(&mut self, frame: Frame) -> Result<(), LinkError> {
        let tx = self.tx.as_ref().ok_or(LinkError::Closed)?;
        tx.send(frame).await.map_err(|_| LinkError::Closed)
    }

    async fn close(&mut self) {
        self.tx = None;
    }
}

#[cfg(test)]
pub(crate) struct FakeStream {
    rx: mpsc::Receiver<Frame>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl RobotLinkStream for FakeStream {
    async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
        self.rx.recv().await.map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct CountingSource;

    #[async_trait::async_trait]
    impl RobotLinkSource for CountingSource {
        fn name(&self) -> &'static str {
            "counting"
        }

        async fn run(
            self: Arc<Self>,
            accept: tokio::sync::mpsc::Sender<AcceptedLink>,
        ) -> anyhow::Result<()> {
            let (sink, stream) = fake_pair();
            accept
                .send(AcceptedLink {
                    identity: RobotIdentity {
                        robot_id: "aa:bb:cc:dd:ee:ff".into(),
                        client_id: "cid".into(),
                        peer: "192.168.1.9".into(),
                    },
                    sink: Box::new(sink),
                    stream: Box::new(stream),
                })
                .await
                .map_err(|_| anyhow::anyhow!("receiver dropped"))?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_source_hands_accepted_links_to_the_gateway_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let source = Arc::new(CountingSource);
        assert_eq!(source.name(), "counting");
        source.clone().run(tx).await.unwrap();

        let link = rx.recv().await.expect("one link accepted");
        assert_eq!(link.identity.robot_id, "aa:bb:cc:dd:ee:ff");
        assert_eq!(link.identity.peer, "192.168.1.9");
    }

    #[tokio::test]
    async fn fake_pair_moves_frames_both_ways() {
        let (mut sink, mut stream) = fake_pair();
        sink.send(Frame::Text("hi".into())).await.unwrap();
        // The test double loops sink writes back into its own stream.
        let got = stream.next().await.unwrap().unwrap();
        assert_eq!(got, Frame::Text("hi".into()));

        sink.close().await;
        assert!(stream.next().await.is_none(), "closing ends the stream");
    }
}
