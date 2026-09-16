//! Local protocol and retained-receipt regressions; no real browser is opened.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Peer {
    conn: Connection,
    seen: mpsc::UnboundedReceiver<Value>,
    worker: tokio::task::JoinHandle<()>,
}
impl Peer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, seen) = mpsc::unbounded_channel();
        let worker = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(socket).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let method = request["method"].as_str().unwrap();
                sent.send(request.clone()).unwrap();
                match method {
                    "Target.attachToTarget" => {
                        // The effect has reached the peer and the child session
                        // exists, but the response remains withheld.
                        socket
                            .send(Message::Text(
                                json!({"method":"Target.attachedToTarget","params":{
                                    "sessionId":"prepared-session","waitingForDebugger":false,
                                    "targetInfo":{"targetId":"prepared-target","type":"page"}
                                }})
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                        continue;
                    }
                    "Page.enable" => {}
                    "Page.getFrameTree" => continue,
                    other => panic!("unexpected protocol effect: {other}"),
                }
                let mut response = json!({"id":request["id"],"result":{}});
                if let Some(session) = request.get("sessionId") {
                    response["sessionId"] = session.clone();
                }
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let conn = Connection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        Self { conn, seen, worker }
    }
    async fn close(self) {
        self.conn.shutdown().await;
        self.worker.await.unwrap();
    }
}

#[tokio::test]
async fn dropping_preparation_after_attach_write_before_ack_fences_the_connection() {
    let mut peer = Peer::new().await;
    let mut attached = peer
        .conn
        .subscribe_reliable("Target.attachedToTarget", None);
    let connection = peer.conn.clone();
    let preparing = tokio::spawn(async move {
        let cancel = CancellationToken::new();
        super::super::prepare(&connection, &cancel, async {
            let mut params = AttachToTargetParams::new("prepared-target".to_owned());
            params.flatten = Some(true);
            connection
                .send(ROOT_SESSION, &params)
                .await
                .map_err(|_| Error::ExecutionFailed)
        })
        .await
    });
    assert_eq!(
        peer.seen.recv().await.unwrap()["method"],
        "Target.attachToTarget"
    );
    let event = tokio::time::timeout(Duration::from_secs(2), attached.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.params["sessionId"], "prepared-session");
    assert!(peer.conn.registry().has_session("prepared-session"));
    assert!(!preparing.is_finished());
    preparing.abort();
    assert!(preparing.await.unwrap_err().is_cancelled());
    assert!(peer.conn.registry().is_connection_closed());
    assert_eq!(peer.conn.registry().pending_callback_count(), 0);
    peer.close().await;
}

fn browser(conn: Connection) -> Arc<AttachedBrowser> {
    Arc::new(AttachedBrowser {
        state: Arc::new(Mutex::new(crate::attached_browser::AttachedState {
            connection: Some(conn),
            retirement: None,
            offered_tabs: Default::default(),
            automation: Default::default(),
            pending: Default::default(),
            retirement_pause: None,
        })),
        operations: Arc::new(tokio::sync::Mutex::new(())),
        incarnation: "receipt-fixture".into(),
        browser_identity: [0; 32],
        chromium_major: 144,
    })
}
async fn disconnect_joins_original_receipts(pending_work: bool) {
    let mut peer = Peer::new().await;
    peer.conn
        .registry()
        .register_session("dialog-session", "page");
    let dialogs = script_dialogs::Dialogs::install(&peer.conn, "dialog-session")
        .await
        .unwrap();
    assert_eq!(peer.seen.recv().await.unwrap()["method"], "Page.enable");
    let (mut cleanup_entered, release_cleanup) = dialogs.hold_shutdown_receipt_for_test();
    let browser = browser(peer.conn.clone());
    let job_finished = Arc::new(AtomicBool::new(false));
    let mut release_work = None;
    if pending_work {
        let (released, waiting) = oneshot::channel();
        let (started, entered) = oneshot::channel();
        let finished = job_finished.clone();
        let pending = spawn(
            async move {
                started.send(()).unwrap();
                let _ = waiting.await;
                finished.store(true, Ordering::Release);
                Ok(json!({"original_work_finished":true}))
            },
            dialogs.clone(),
            peer.conn.clone(),
            CancellationToken::new(),
            false,
        );
        browser
            .state
            .lock()
            .unwrap()
            .pending
            .insert("fixture-target".into(), pending);
        entered.await.unwrap();
        release_work = Some(released);
    } else {
        browser.state.lock().unwrap().automation.insert(
            "fixture-target".into(),
            TabAutomation {
                dialogs: Some(dialogs.clone()),
                ..Default::default()
            },
        );
    }
    assert!(
        browser.operations.try_lock().is_ok(),
        "the original operation gate is already empty"
    );
    let mut first = tokio::spawn({
        let browser = browser.clone();
        async move { browser.disconnect().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut first)
            .await
            .is_err(),
        "disconnect must not finish before original receipts"
    );
    assert!(
        peer.conn.registry().is_connection_closed(),
        "the socket is already fenced"
    );
    if let Some(release_work) = release_work {
        assert!(!job_finished.load(Ordering::Acquire));
        assert!(
            matches!(
                cleanup_entered.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "dialog cleanup must follow the retained original job"
        );
        release_work.send(()).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), cleanup_entered)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !first.is_finished(),
        "dialog receipt is still withheld after socket shutdown"
    );
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let mut second = tokio::spawn({
        let browser = browser.clone();
        async move { browser.disconnect().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut second)
            .await
            .is_err(),
        "second caller must join the original cleanup, not an empty replacement"
    );
    release_cleanup.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if pending_work {
        assert!(job_finished.load(Ordering::Acquire));
    }
    assert!(browser.state.lock().unwrap().pending.is_empty());
    assert!(browser.state.lock().unwrap().automation.is_empty());
    browser.disconnect().await.unwrap();
    drop(dialogs);
    drop(browser);
    peer.close().await;
}

#[tokio::test]
async fn disconnect_joins_cached_dialog_receipt_after_waiter_cancellation() {
    disconnect_joins_original_receipts(false).await;
}
#[tokio::test]
async fn disconnect_joins_pending_work_then_dialog_receipt_after_waiter_cancellation() {
    disconnect_joins_original_receipts(true).await;
}
#[test]
fn eighth_answered_dialog_waiting_for_closed_is_not_a_ninth_dialog() {
    let mut answered: BTreeSet<String> = (1..8).map(|index| format!("dialog-{index}")).collect();
    assert!(!new_dialog_exceeds_budget(&answered, "dialog-8"));
    answered.insert("dialog-8".into());
    // Repeated observations while its reply ACK / Closed is outstanding must
    // join the same answer, not trip the budget and discard its receipt.
    for _ in 0..3 {
        assert!(!new_dialog_exceeds_budget(&answered, "dialog-8"));
    }
    assert!(new_dialog_exceeds_budget(&answered, "dialog-9"));
}

async fn pending_read() -> (Peer, Pending, CancellationToken, Value) {
    let mut peer = Peer::new().await;
    peer.conn
        .registry()
        .register_session("dialog-session", "page");
    let dialogs = script_dialogs::Dialogs::install(&peer.conn, "dialog-session")
        .await
        .unwrap();
    assert_eq!(peer.seen.recv().await.unwrap()["method"], "Page.enable");
    let connection = peer.conn.clone();
    let cancel = CancellationToken::new();
    let work = spawn(
        async move {
            connection
                .send("dialog-session", &GetFrameTreeParams::default())
                .await
                .map_err(|_| Error::ExecutionFailed)
        },
        dialogs,
        peer.conn.clone(),
        cancel.clone(),
        false,
    );
    let request = peer.seen.recv().await.unwrap();
    assert_eq!(request["method"], "Page.getFrameTree");
    (peer, work, cancel, request)
}

#[tokio::test]
async fn short_read_stop_preserves_connection_when_original_ack_finishes_in_grace() {
    let (peer, pending, cancel, request) = pending_read().await;
    let mut fatal = peer.conn.subscribe_fatal();
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), fatal.changed())
            .await
            .is_err(),
        "normal read cancellation must not disconnect immediately"
    );
    peer.conn.registry().dispatch_message(&json!({"id":request["id"],"sessionId":"dialog-session","result":{"original_read":true}}).to_string()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), pending.job.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, json!({"original_read":true}));
    assert!(!peer.conn.registry().is_connection_closed());
    pending.dialogs.shutdown().await.unwrap();
    peer.close().await;
}

#[tokio::test]
async fn stalled_read_stop_has_one_grace_deadline_despite_dialog_update_traffic() {
    let (peer, pending, cancel, _) = pending_read().await;
    let started = std::time::Instant::now();
    cancel.cancel();
    while !peer.conn.registry().is_connection_closed() {
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "dialog updates must not restart the grace deadline"
        );
        peer.conn.registry().dispatch_message(&json!({"method":"Page.javascriptDialogClosed","sessionId":"dialog-session","params":{}}).to_string()).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "an ordinary read receives its cooperative grace"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending.job.clone())
            .await
            .unwrap()
            .is_err()
    );
    let _ = pending.dialogs.shutdown().await;
    peer.close().await;
}

#[tokio::test]
async fn already_cancelled_pending_work_is_not_polled_and_does_not_fence() {
    let mut peer = Peer::new().await;
    peer.conn
        .registry()
        .register_session("dialog-session", "page");
    let dialogs = script_dialogs::Dialogs::install(&peer.conn, "dialog-session")
        .await
        .unwrap();
    assert_eq!(peer.seen.recv().await.unwrap()["method"], "Page.enable");
    let polled = Arc::new(AtomicBool::new(false));
    let started = polled.clone();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let pending = spawn(
        async move {
            started.store(true, Ordering::Release);
            Ok(json!({}))
        },
        dialogs.clone(),
        peer.conn.clone(),
        cancel,
        false,
    );
    assert_eq!(pending.job.clone().await, Err(Error::Cancelled));
    assert!(!polled.load(Ordering::Acquire));
    assert!(!peer.conn.registry().is_connection_closed());
    assert!(peer.seen.try_recv().is_err());
    dialogs.shutdown().await.unwrap();
    peer.close().await;
}
