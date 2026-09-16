//! Local CDP protocol coverage for ordered dialogs and response-clock policy.
use super::*;
use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
use futures_util::poll;
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, task::Poll};
use tokio::sync::{mpsc, watch};

struct Peer {
    conn: Connection,
    packets: mpsc::UnboundedSender<Value>,
    requests: Arc<StdMutex<Vec<Value>>>,
    worker: tokio::task::JoinHandle<()>,
}
impl Peer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(StdMutex::new(vec![]));
        let captured = requests.clone();
        let (packets, mut outgoing) = mpsc::unbounded_channel::<Value>();
        let worker = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            loop {
                tokio::select! {
                    packet = outgoing.recv() => {
                        let Some(packet) = packet else { break; };
                        if socket.send(WsMessage::Text(packet.to_string().into())).await.is_err() { break; }
                    }
                    message = socket.next() => match message {
                        Some(Ok(WsMessage::Text(text))) => captured.lock().unwrap().push(serde_json::from_str(&text).unwrap()),
                        Some(Ok(WsMessage::Ping(_))) => {},
                        _ => break,
                    }
                }
            }
        });
        let conn = Connection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        Self {
            conn,
            packets,
            requests,
            worker,
        }
    }
    async fn start<F: Future<Output = CommandResult>>(
        &self,
        mut future: Pin<&mut F>,
        requests: usize,
    ) {
        // Start real socket I/O before pausing time. Keep the tested response
        // future explicitly polled so virtual time cannot outrun OS delivery.
        for _ in 0..100_000 {
            assert!(poll!(future.as_mut()).is_pending());
            if self.requests.lock().unwrap().len() >= requests {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("local protocol request was not written");
    }
    fn reply(&self, index: usize) {
        let request = self.requests.lock().unwrap()[index].clone();
        let mut reply = json!({"id":request["id"],"result":{"fixture":true}});
        if let Some(session) = request.get("sessionId") {
            reply["sessionId"] = session.clone();
        }
        self.packets.send(reply).unwrap();
    }
    async fn close(self) {
        self.conn.shutdown().await;
        self.worker.await.unwrap();
    }
}

#[tokio::test]
async fn paused_response_survives_original_deadline_and_receives_original_ack() {
    let peer = Peer::new().await;
    let (_pause, receiver) = watch::channel(true);
    let conn = peer.conn.with_response_pause(receiver);
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(conn.send(ROOT_SESSION, &params));
    peer.start(waiting.as_mut(), 1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(poll!(waiting.as_mut()).is_pending());
    peer.reply(0);
    assert_eq!(waiting.as_mut().await.unwrap(), json!({"fixture":true}));
    assert_eq!(peer.requests.lock().unwrap().len(), 1, "no command replay");
    drop(waiting);
    drop(conn);
    tokio::time::resume();
    peer.close().await;
}

#[tokio::test]
async fn resumed_response_consumes_remaining_budget_instead_of_resetting_it() {
    let peer = Peer::new().await;
    let (pause, receiver) = watch::channel(false);
    let conn = peer.conn.with_response_pause(receiver);
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(conn.send(ROOT_SESSION, &params));
    peer.start(waiting.as_mut(), 1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(12)).await;
    assert!(poll!(waiting.as_mut()).is_pending());
    pause.send(true).unwrap();
    assert!(poll!(waiting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(90)).await;
    assert!(poll!(waiting.as_mut()).is_pending());
    pause.send(false).unwrap();
    assert!(poll!(waiting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(17)).await;
    assert!(poll!(waiting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        poll!(waiting.as_mut()),
        Poll::Ready(Err(TransportError::Timeout))
    ));
    assert_eq!(peer.conn.registry().pending_callback_count(), 0);
    drop(waiting);
    drop(conn);
    tokio::time::resume();
    peer.close().await;
}

#[tokio::test]
async fn ordinary_clone_keeps_its_deadline_while_another_handle_is_paused() {
    let peer = Peer::new().await;
    let (_pause, receiver) = watch::channel(true);
    let paused = peer.conn.with_response_pause(receiver);
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(paused.send(ROOT_SESSION, &params));
    peer.start(waiting.as_mut(), 1).await;
    let normal = peer.conn.clone();
    let mut ordinary = Box::pin(normal.send(ROOT_SESSION, &params));
    peer.start(ordinary.as_mut(), 2).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(matches!(
        poll!(ordinary.as_mut()),
        Poll::Ready(Err(TransportError::Timeout))
    ));
    assert!(poll!(waiting.as_mut()).is_pending());
    peer.reply(0);
    assert!(waiting.as_mut().await.is_ok());
    drop(waiting);
    drop(ordinary);
    drop(paused);
    drop(normal);
    tokio::time::resume();
    peer.close().await;
}

#[tokio::test]
async fn dropped_pause_owner_resumes_remaining_budget() {
    let peer = Peer::new().await;
    let (pause, receiver) = watch::channel(true);
    let conn = peer.conn.with_response_pause(receiver);
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(conn.send(ROOT_SESSION, &params));
    peer.start(waiting.as_mut(), 1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(poll!(waiting.as_mut()).is_pending());
    drop(pause);
    assert!(poll!(waiting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(poll!(waiting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        poll!(waiting.as_mut()),
        Poll::Ready(Err(TransportError::Timeout))
    ));
    drop(waiting);
    drop(conn);
    tokio::time::resume();
    peer.close().await;
}

#[tokio::test]
async fn disconnect_unblocks_a_paused_response_without_waiting_for_resume() {
    let peer = Peer::new().await;
    let (_pause, receiver) = watch::channel(true);
    let conn = peer.conn.with_response_pause(receiver);
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(conn.send(ROOT_SESSION, &params));
    peer.start(waiting.as_mut(), 1).await;
    tokio::time::timeout(Duration::from_secs(2), peer.conn.shutdown())
        .await
        .unwrap();
    assert!(matches!(
        poll!(waiting.as_mut()),
        Poll::Ready(Err(TransportError::Closed))
    ));
    assert_eq!(peer.conn.registry().pending_callback_count(), 0);
    drop(waiting);
    drop(conn);
    peer.close().await;
}

const OPEN: &str = "Page.javascriptDialogOpening";

#[tokio::test]
async fn pause_never_extends_the_socket_write_deadline() {
    let peer = Peer::new().await;
    let (_pause, receiver) = watch::channel(true);
    let conn = peer.conn.with_response_pause(receiver);
    let sink = peer.conn.inner.sink.lock().await;
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(conn.send(ROOT_SESSION, &params));
    tokio::time::pause();
    assert!(poll!(waiting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(matches!(
        poll!(waiting.as_mut()),
        Poll::Ready(Err(TransportError::Timeout))
    ));
    assert!(peer.requests.lock().unwrap().is_empty());
    assert_eq!(peer.conn.registry().pending_callback_count(), 0);
    drop(waiting);
    drop(sink);
    drop(conn);
    tokio::time::resume();
    peer.close().await;
}

#[tokio::test]
async fn late_pause_does_not_resurrect_an_exhausted_response_budget() {
    let peer = Peer::new().await;
    let (pause, receiver) = watch::channel(false);
    let conn = peer.conn.with_response_pause(receiver);
    let params = GetTargetsParams::default();
    let mut waiting = Box::pin(conn.send(ROOT_SESSION, &params));
    peer.start(waiting.as_mut(), 1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    pause.send(true).unwrap();
    assert!(matches!(
        poll!(waiting.as_mut()),
        Poll::Ready(Err(TransportError::Timeout))
    ));
    drop(waiting);
    drop(conn);
    tokio::time::resume();
    peer.close().await;
}

const CLOSE: &str = "Page.javascriptDialogClosed";
#[tokio::test]
async fn reliable_sequence_preserves_open_close_order_and_deduplicates_methods() {
    let peer = Peer::new().await;
    peer.conn.registry().register_session("dialog-page", "page");
    let mut sequence = peer
        .conn
        .subscribe_reliable_sequence(&[OPEN, CLOSE, OPEN], Some("dialog-page"));
    let mut legacy = peer.conn.subscribe_reliable(OPEN, Some("dialog-page"));
    peer.packets
        .send(json!({"sessionId":"unrelated-page","method":OPEN,"params":{"number":99}}))
        .unwrap();
    for (number, method) in [OPEN, CLOSE, OPEN].into_iter().enumerate() {
        peer.packets
            .send(json!({"sessionId":"dialog-page","method":method,"params":{"number":number}}))
            .unwrap();
    }
    for (number, method) in [OPEN, CLOSE, OPEN].into_iter().enumerate() {
        let event = tokio::time::timeout(Duration::from_secs(2), sequence.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.method, method);
        assert_eq!(event.params["number"], number);
        assert_eq!(event.session_id, "dialog-page");
    }
    assert!(sequence.try_recv().is_err());
    assert_eq!(legacy.recv().await.unwrap().params["number"], 0);
    assert_eq!(legacy.recv().await.unwrap().params["number"], 2);
    peer.close().await;
}

#[tokio::test]
async fn alternating_sequence_methods_share_one_budget_and_poison_on_overflow() {
    let peer = Peer::new().await;
    let mut sequence = peer.conn.subscribe_reliable_sequence(&[OPEN, CLOSE], None);
    let mut fatal = peer.conn.subscribe_fatal();
    for number in 0..=crate::session::RELIABLE_EVENT_CAPACITY {
        peer.packets
            .send(
                json!({"method":if number % 2 == 0 {OPEN} else {CLOSE},"params":{"number":number}}),
            )
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while !peer.conn.registry().is_connection_closed() {
            fatal.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let mut count = 0;
    while sequence.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, crate::session::RELIABLE_EVENT_CAPACITY);
    assert!(peer.conn.registry().is_connection_closed());
    peer.close().await;
}

#[test]
fn invalid_sequence_methods_fail_closed_without_partial_registration() {
    for methods in [vec![], vec![""], vec![OPEN; 17]] {
        let registry = SessionRegistry::new();
        let mut receiver = registry.subscribe_reliable_sequence(&methods, None);
        assert!(registry.is_connection_closed());
        assert!(receiver.try_recv().is_err());
        assert!(!registry.has_reliable_subscriber(OPEN));
    }
}
