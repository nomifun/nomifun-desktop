use super::*;
use futures_util::{SinkExt, StreamExt};
use nomifun_browser_platform::attached_browser::{
    AttachedBrowserCommand, AttachedBrowserRuntimeError,
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

struct Peer {
    _directory: tempfile::TempDir,
    targets: Arc<Mutex<Vec<Value>>>,
    commands: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn target(id: &str, title: &str, url: &str) -> Value {
    json!({"targetId":id,"type":"page","title":title,"url":url})
}

async fn peer() -> (AttachedBrowser, Peer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let port_file = directory.path().join("DevToolsActivePort");
    std::fs::write(
        &port_file,
        format!(
            "{}\n/devtools/browser/fixture",
            listener.local_addr().unwrap().port()
        ),
    )
    .unwrap();
    let targets = Arc::new(Mutex::new(vec![
        target("raw-target-A", "Selected app", "https://fixture.test/a"),
        target(
            "raw-target-B",
            "Private unrelated page",
            "https://private.test/b",
        ),
    ]));
    let commands = Arc::new(Mutex::new(Vec::new()));
    let (peer_targets, peer_commands) = (targets.clone(), commands.clone());
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let request: Value = serde_json::from_str(&text).unwrap();
            peer_commands.lock().unwrap().push(request.clone());
            let result = match request["method"].as_str().unwrap() {
                "Browser.getVersion" => json!({"product":"Chrome/152.0.0.0"}),
                "Target.getTargets" => {
                    let pause = peer_targets
                        .lock()
                        .unwrap()
                        .first()
                        .is_some_and(|target| target["pause_inventory"] == true);
                    if pause {
                        std::future::pending::<()>().await;
                    }
                    json!({"targetInfos":peer_targets.lock().unwrap().clone()})
                }
                "Target.getTargetInfo" => {
                    let found = peer_targets
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|tab| tab["targetId"] == request["params"]["targetId"])
                        .cloned();
                    json!({"targetInfo":found})
                }
                other => panic!("unexpected command {other}"),
            };
            ws.send(Message::Text(
                json!({"id":request["id"],"result":result})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        }
    });
    let browser = AttachedBrowser::connect_port_file(&port_file)
        .await
        .unwrap();
    (
        browser,
        Peer {
            _directory: directory,
            targets,
            commands,
            task,
        },
    )
}

#[tokio::test]
async fn installation_provider_inventory_mints_opaque_handles_without_attaching_pages() {
    let (browser, peer) = peer().await;
    let tabs = browser.tabs_for_provider().await.unwrap();
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].info.title, "Selected app");
    assert_eq!(tabs[1].info.title, "Private unrelated page");
    for tab in &tabs {
        assert_eq!(tab.info.tab_id, tab.grant.id());
        assert!(!tab.info.tab_id.contains("raw-target"));
        let key = tab.grant.target_key();
        assert_eq!(key.len(), 64);
        assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    assert_eq!(
        peer.commands.lock().unwrap().len(),
        2,
        "inventory only probes version and lists current targets"
    );
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn handle_from_another_installation_connection_is_rejected_before_protocol_io() {
    let (first, _first_peer) = peer().await;
    let (second, second_peer) = peer().await;
    let grant = first
        .tabs_for_provider()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .grant;
    let before = second_peer.commands.lock().unwrap().len();
    assert_eq!(
        second
            .execute_granted(
                &grant,
                AttachedBrowserCommand::Observe {
                    tab_id: grant.id().into(),
                },
                &CancellationToken::new(),
            )
            .await,
        Err(AttachedBrowserRuntimeError::TabDenied)
    );
    assert_eq!(second_peer.commands.lock().unwrap().len(), before);
    first.disconnect().await.unwrap();
    second.disconnect().await.unwrap();
}

#[tokio::test]
async fn closed_or_privileged_target_never_falls_back_to_another_page() {
    let (browser, peer) = peer().await;
    let grant = browser
        .tabs_for_provider()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .grant;
    peer.targets.lock().unwrap().remove(0);
    assert_eq!(
        browser
            .execute_granted(
                &grant,
                AttachedBrowserCommand::Observe {
                    tab_id: grant.id().into(),
                },
                &CancellationToken::new(),
            )
            .await,
        Err(AttachedBrowserRuntimeError::TabDenied)
    );
    peer.targets
        .lock()
        .unwrap()
        .insert(0, target("raw-target-A", "Settings", "chrome://settings"));
    assert_eq!(
        browser
            .execute_granted(
                &grant,
                AttachedBrowserCommand::Observe {
                    tab_id: grant.id().into(),
                },
                &CancellationToken::new(),
            )
            .await,
        Err(AttachedBrowserRuntimeError::TabDenied)
    );
    assert!(peer.commands.lock().unwrap().iter().all(|command| {
        !command["method"]
            .as_str()
            .unwrap()
            .contains("attachToTarget")
    }));
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn provider_inventory_filters_privileged_targets_and_is_bounded() {
    let (browser, peer) = peer().await;
    peer.targets.lock().unwrap().extend([
        target("settings", "Settings", "chrome://settings"),
        target("file", "File", "file:///private"),
        target(
            "credentials",
            "Secret",
            "https://user:password@fixture.test",
        ),
        json!({"type":"service_worker","targetId":"worker","title":"Worker","url":"https://fixture.test"}),
    ]);
    assert_eq!(browser.tabs_for_provider().await.unwrap().len(), 2);

    *peer.targets.lock().unwrap() =
        vec![target("many", "A", "https://fixture.test"); MAX_TABS + 1];
    assert_eq!(
        browser.tabs_for_provider().await.err(),
        Some(AttachError::InventoryLimit)
    );
    *peer.targets.lock().unwrap() = vec![
        target("duplicate", "A", "https://fixture.test/a"),
        target("duplicate", "B", "https://fixture.test/b"),
    ];
    assert_eq!(
        browser.tabs_for_provider().await.err(),
        Some(AttachError::ConnectionFailed)
    );
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn disconnect_waits_for_operations_and_blocks_queued_provider_inventory() {
    let (browser, _peer) = peer().await;
    let browser = Arc::new(browser);
    let operation = browser.operations.lock().await;
    let in_flight = browser.current_connection().unwrap();
    let closing = browser.clone();
    let mut close = tokio::spawn(async move { closing.disconnect().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while browser.is_connected() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut close)
            .await
            .is_err()
    );
    let queued = browser.clone();
    let listing = tokio::spawn(async move { queued.tabs_for_provider().await });
    drop(in_flight);
    drop(operation);
    close.await.unwrap().unwrap();
    assert_eq!(
        listing.await.unwrap().err(),
        Some(AttachError::ConnectionFailed)
    );
}

#[tokio::test]
async fn synchronous_disconnect_fence_interrupts_unanswered_provider_inventory() {
    let (browser, peer) = peer().await;
    peer.targets.lock().unwrap()[0]["pause_inventory"] = json!(true);
    let browser = Arc::new(browser);
    let listing_browser = browser.clone();
    let listing = tokio::spawn(async move { listing_browser.tabs_for_provider().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while peer.commands.lock().unwrap().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    browser.request_disconnect();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), listing)
            .await
            .unwrap()
            .unwrap()
            .err(),
        Some(AttachError::ConnectionFailed)
    );
    browser.disconnect().await.unwrap();
}
