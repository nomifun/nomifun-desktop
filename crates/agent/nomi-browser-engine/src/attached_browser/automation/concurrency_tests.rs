//! Real local protocol transport with two target owners; no browser is opened.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct ProtocolPeer {
    worker: tokio::task::JoinHandle<()>,
    first_read: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Drop for ProtocolPeer {
    fn drop(&mut self) { self.worker.abort(); }
}

fn target(id: &str) -> Value {
    json!({"targetId":id,"type":"page","title":id,"url":"https://fixture.test/",
        "attached":true,"canAccessOpener":false})
}

async fn fixture() -> (Arc<AttachedBrowser>, ProtocolPeer, GrantedTab, GrantedTab) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let first_read = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let started = first_read.clone(); let released = release.clone(); let captured = requests.clone();
    let worker = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(socket).await.unwrap();
        let mut held = None;
        loop {
            let request: Value = tokio::select! {
                permit = released.acquire(), if held.is_some() => {
                    permit.unwrap().forget();
                    let request: Value = held.take().unwrap();
                    let reply = json!({"id":request["id"],"sessionId":request["sessionId"],"result":{
                        "frameTree":{"frame":{"id":"frame-a","loaderId":"loader-a","url":"https://fixture.test/"}}
                    }});
                    if socket.send(Message::Text(reply.to_string().into())).await.is_err() { break; }
                    continue;
                },
                request = socket.next() => {
                    let Some(Ok(Message::Text(request))) = request else { break; };
                    serde_json::from_str(&request).unwrap()
                },
            };
            captured.lock().unwrap().push(request.clone());
            let method = request["method"].as_str().unwrap();
            let result = match method {
                "Target.getTargets" => json!({"targetInfos":[target("target-a"),target("target-b")]}),
                "Target.getTargetInfo" => json!({"targetInfo":target(request["params"]["targetId"].as_str().unwrap())}),
                "Page.enable" | "Page.bringToFront" | "Target.detachFromTarget" => json!({}),
                "Page.getFrameTree" if request["sessionId"] == "session-a" => {
                    assert!(held.is_none(), "same-target work must not dispatch a second native read");
                    held = Some(request);
                    started.notify_one();
                    continue;
                },
                "Page.getFrameTree" => json!({"frameTree":{"frame":{
                    "id":"frame-b","loaderId":"loader-b","url":"https://fixture.test/"
                }}}),
                // End each read after its independent transport work without
                // fabricating a semantic observation or running page scripts.
                "Page.createIsolatedWorld" => Value::Null,
                other => panic!("unexpected concurrency fixture protocol method: {other}"),
            };
            let mut reply = if result.is_null() {
                json!({"id":request["id"],"error":{"code":-32000,"message":"fixture: observation preparation rejected"}})
            } else { json!({"id":request["id"],"result":result}) };
            if let Some(session) = request.get("sessionId") { reply["sessionId"] = session.clone(); }
            if socket.send(Message::Text(reply.to_string().into())).await.is_err() { break; }
        }
    });
    let connection = Connection::connect(&format!("ws://{address}")).await.unwrap();
    connection.registry().register_session("session-a", "page");
    connection.registry().register_session("session-b", "page");
    let browser = Arc::new(AttachedBrowser {
        state: Arc::new(Mutex::new(crate::attached_browser::AttachedState {
            connection: Some(connection), retirement: None, pending: Default::default(), retirement_pause: None,
            automation: [
                ("target-a".into(), TabAutomation { session: "session-a".into(), ..Default::default() }),
                ("target-b".into(), TabAutomation { session: "session-b".into(), ..Default::default() }),
            ].into(),
        })),
        operations: Arc::new(tokio::sync::RwLock::new(())),
        targets: Mutex::new(Default::default()), inputs: Arc::new(tokio::sync::Mutex::new(())),
        incarnation: "concurrency-fixture".into(), browser_identity: [0; 32], chromium_major: 144,
    });
    let tabs = browser.tabs_for_provider().await.unwrap();
    let first = tabs.iter().find(|tab| tab.info.title == "target-a").unwrap().grant.clone();
    let second = tabs.iter().find(|tab| tab.info.title == "target-b").unwrap().grant.clone();
    (browser, ProtocolPeer { worker, first_read, release, requests }, first, second)
}

#[tokio::test]
async fn slow_target_read_preserves_other_target_inventory_and_release_progress() {
    let (browser, peer, first, second) = fixture().await;
    let reading = browser.clone(); let grant = first.clone();
    let first_read = tokio::spawn(async move {
        reading.execute_granted(&grant, Command::Observe { tab_id: grant.id().into() }, &CancellationToken::new()).await
    });
    tokio::time::timeout(Duration::from_secs(3), peer.first_read.notified()).await.unwrap();
    assert_eq!(browser.execute_granted(&first, Command::Navigate {
        tab_id: first.id().into(), url: "https://fixture.test/next".into(),
    }, &CancellationToken::new()).await, Err(Error::Busy));
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), browser.execute_granted(&second,
        Command::Observe { tab_id: second.id().into() }, &CancellationToken::new())).await
        .expect("the second target must reach its own native result"), Err(Error::ExecutionFailed));
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), browser.tabs_for_provider()).await
        .expect("inventory must not wait on another target's native read").unwrap().len(), 2);
    tokio::time::timeout(Duration::from_secs(3), browser.release_granted(&second)).await
        .expect("independent target cleanup must not wait on the first native read").unwrap();
    assert!(!first_read.is_finished());
    let requests = peer.requests.lock().unwrap().clone();
    assert_eq!(requests.iter().filter(|request| request["method"] == "Page.getFrameTree"
        && request["sessionId"] == "session-a").count(), 1);
    assert!(requests.iter().any(|request| request["method"] == "Target.detachFromTarget"
        && request["params"]["sessionId"] == "session-b"));
    peer.release.add_permits(1);
    assert_eq!(first_read.await.unwrap(), Err(Error::ExecutionFailed));
    browser.release_granted(&first).await.unwrap();
    assert!(browser.state.lock().unwrap().automation.is_empty());
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn disconnect_fences_independent_read_admission_and_joins_original_native_owner() {
    let (browser, peer, first, _) = fixture().await;
    let reading = browser.clone(); let grant = first.clone();
    let first_read = tokio::spawn(async move {
        reading.execute_granted(&grant, Command::Observe { tab_id: grant.id().into() }, &CancellationToken::new()).await
    });
    tokio::time::timeout(Duration::from_secs(3), peer.first_read.notified()).await.unwrap();
    let barrier = browser.operations.read().await;
    let closing = browser.clone();
    let mut disconnect = Box::pin(closing.disconnect());
    assert!(futures_util::poll!(&mut disconnect).is_pending());
    assert!(!browser.is_connected());
    tokio::time::timeout(Duration::from_secs(3), first_read).await.unwrap().unwrap().unwrap_err();
    assert!(futures_util::poll!(&mut disconnect).is_pending(), "physical disposal must retain its original barrier");
    let mut inventory = Box::pin(browser.tabs_for_provider());
    let inventory_done = match futures_util::poll!(&mut inventory) {
        std::task::Poll::Ready(result) => {
            assert_eq!(result.err(), Some(crate::attached_browser::AttachError::ConnectionFailed));
            true
        },
        std::task::Poll::Pending => false,
    };
    drop(barrier);
    tokio::time::timeout(Duration::from_secs(3), disconnect).await.unwrap().unwrap();
    if !inventory_done {
        assert_eq!(inventory.await.err(), Some(crate::attached_browser::AttachError::ConnectionFailed));
    }
    assert!(browser.state.lock().unwrap().pending.is_empty());
    assert!(browser.state.lock().unwrap().automation.is_empty());
}
