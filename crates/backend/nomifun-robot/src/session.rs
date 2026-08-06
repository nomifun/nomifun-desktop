//! One actor per connected robot.
//!
//! This task owns the read loop, the handshake, the keepalive ping, and (from
//! later tasks) the audio pipelines. It never touches a socket directly — only
//! [`AcceptedLink`] halves — so the same actor serves a LAN WebSocket today and
//! a relay tunnel later.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

use crate::link::{AcceptedLink, Frame, RobotLinkSink};
use crate::protocol::{
    DeviceMessage, ServerMessage, parse_device_message, serialize_server_message,
};
use crate::registry::RobotRegistry;
use crate::status::{RobotPhase, RobotStatusRegistry};

/// The firmware declares a link dead after 120 s of silence; ping at half that.
pub const PING_INTERVAL_SECS: u64 = 60;

/// Everything a session actor needs from the host.
#[derive(Clone)]
pub struct SessionDeps {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Outbound frames are funnelled through one writer task so the ping timer and
/// the pipelines never contend for the sink.
struct Writer {
    tx: mpsc::Sender<Frame>,
}

impl Writer {
    fn spawn(mut sink: Box<dyn RobotLinkSink>) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<Frame>(64);
        let handle = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if sink.send(frame).await.is_err() {
                    break;
                }
            }
            sink.close().await;
        });
        (Self { tx }, handle)
    }

    async fn send_json(&self, msg: &ServerMessage) {
        let _ = self
            .tx
            .send(Frame::Text(serialize_server_message(msg)))
            .await;
    }
}

/// Run one robot session to completion. Returns when the inbound stream ends.
pub async fn run_session(link: AcceptedLink, deps: SessionDeps) {
    let AcceptedLink {
        identity,
        sink,
        mut stream,
    } = link;
    let robot_id = identity.robot_id.clone();
    let (writer, writer_task) = Writer::spawn(sink);

    let mut session_id: Option<String> = None;
    let mut companion_id: Option<String> = None;
    let mut ping = interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping.tick().await; // the first tick is immediate; skip it

    loop {
        tokio::select! {
            _ = ping.tick() => {
                if let Some(sid) = &session_id {
                    writer.send_json(&ServerMessage::Ping { session_id: sid.clone() }).await;
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                let Ok(frame) = frame else { break };
                match frame {
                    Frame::Binary(_) if session_id.is_none() => {
                        // Wake-word audio can arrive before `listen start`; before
                        // the handshake it is simply noise.
                        continue;
                    }
                    Frame::Binary(_) => {
                        // Uplink audio handling lands in the uplink pipeline task.
                        continue;
                    }
                    Frame::Text(raw) => {
                        let message = match parse_device_message(&raw) {
                            Ok(m) => m,
                            Err(error) => {
                                tracing::warn!(%robot_id, %error, "robot: unparseable text frame");
                                continue;
                            }
                        };
                        match message {
                            DeviceMessage::Hello(hello) => {
                                let record = deps.registry.list().await.into_iter().find(|r| r.robot_id == robot_id);
                                let Some(bound) = record.as_ref().and_then(|r| r.companion_id.clone()) else {
                                    tracing::warn!(%robot_id, "robot: refusing session, not bound to a companion");
                                    break;
                                };
                                let sid = uuid::Uuid::new_v4().to_string();
                                tracing::info!(
                                    %robot_id,
                                    companion_id = %bound,
                                    session_id = %sid,
                                    protocol_version = hello.version,
                                    mcp = hello.mcp,
                                    "robot: session established"
                                );
                                writer.send_json(&ServerMessage::Hello { session_id: sid.clone() }).await;
                                deps.status
                                    .publish(&robot_id, Some(&bound), RobotPhase::Idle, now_ms())
                                    .await;
                                session_id = Some(sid);
                                companion_id = Some(bound);
                            }
                            DeviceMessage::Unknown { raw_type } => {
                                tracing::debug!(%robot_id, %raw_type, "robot: unknown message type");
                            }
                            // Listen / Abort / Mcp handling is wired by the
                            // pipeline and bridge tasks.
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let _ = companion_id;
    deps.status.mark_offline(&robot_id, now_ms()).await;
    drop(writer);
    let _ = writer_task.await;
    tracing::info!(%robot_id, "robot: session ended");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RobotEventEmitter;
    use crate::link::{
        AcceptedLink, Frame, LinkError, RobotIdentity, RobotLinkSink, RobotLinkStream,
    };
    use crate::registry::{RobotRegistry, RobotReport};
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::UserEventSink;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    struct NullSink;
    impl UserEventSink for NullSink {
        fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
    }

    /// A sink that records everything written, and a stream driven by a channel.
    struct RecordingSink(Arc<Mutex<Vec<Frame>>>);
    #[async_trait::async_trait]
    impl RobotLinkSink for RecordingSink {
        async fn send(&mut self, frame: Frame) -> Result<(), LinkError> {
            self.0.lock().unwrap().push(frame);
            Ok(())
        }
        async fn close(&mut self) {}
    }

    struct ChannelStream(mpsc::Receiver<Frame>);
    #[async_trait::async_trait]
    impl RobotLinkStream for ChannelStream {
        async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
            self.0.recv().await.map(Ok)
        }
    }

    async fn harness(
        bound: bool,
    ) -> (
        SessionDeps,
        AcceptedLink,
        mpsc::Sender<Frame>,
        Arc<Mutex<Vec<Frame>>>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (record, token) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        if bound {
            registry
                .claim(
                    record.activation_code.as_deref().unwrap(),
                    "0190f5fe-7c00-7a00-8000-0000000000aa",
                )
                .await
                .unwrap();
        }
        let _ = token;
        let status = Arc::new(crate::status::RobotStatusRegistry::new(
            RobotEventEmitter::new(Arc::new(NullSink)),
            "owner-1".to_owned(),
        ));
        let written = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel(16);
        let link = AcceptedLink {
            identity: RobotIdentity {
                robot_id: "aa:bb:cc:dd:ee:ff".into(),
                client_id: "cid".into(),
                peer: "192.168.1.9".into(),
            },
            sink: Box::new(RecordingSink(written.clone())),
            stream: Box::new(ChannelStream(rx)),
        };
        (SessionDeps { registry, status }, link, tx, written, dir)
    }

    fn texts(frames: &Arc<Mutex<Vec<Frame>>>) -> Vec<serde_json::Value> {
        frames
            .lock()
            .unwrap()
            .iter()
            .filter_map(|f| match f {
                Frame::Text(t) => serde_json::from_str(t).ok(),
                Frame::Binary(_) => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn bound_device_gets_a_server_hello_after_its_hello() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true}}"#
                .into(),
        ))
        .await
        .unwrap();
        // Closing the stream ends the session loop.
        drop(tx);
        task.await.unwrap();

        let sent = texts(&written);
        assert_eq!(sent.len(), 1, "exactly one server hello");
        assert_eq!(sent[0]["type"], "hello");
        assert_eq!(sent[0]["audio_params"]["sample_rate"], 24000);
        assert!(sent[0]["session_id"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[tokio::test]
    async fn unbound_device_is_refused_after_hello_with_no_server_hello() {
        let (deps, link, tx, written, _dir) = harness(false).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        task.await.unwrap();

        let sent = texts(&written);
        assert!(
            sent.iter().all(|m| m["type"] != "hello"),
            "an unbound robot must never get a session"
        );
    }

    #[tokio::test]
    async fn audio_before_hello_is_ignored_not_fatal() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Binary(bytes::Bytes::from_static(&[0xfc, 0x01])))
            .await
            .unwrap();
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();

        assert_eq!(
            texts(&written).len(),
            1,
            "session still established after stray audio"
        );
    }

    #[tokio::test]
    async fn unknown_message_type_does_not_end_the_session() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(r#"{"type":"brand_new_thing","x":1}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"stop"}"#.into(),
        ))
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();

        assert_eq!(texts(&written)[0]["type"], "hello");
    }

    #[tokio::test]
    async fn session_marks_offline_when_the_link_drops() {
        let (deps, link, tx, _written, _dir) = harness(true).await;
        let status = deps.status.clone();
        let task = tokio::spawn(run_session(link, deps));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();

        let snap = status.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].phase, "offline");
    }
}
