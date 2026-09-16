use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

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
async fn user_inventory_has_opaque_choices_and_granted_output_is_single_tab_only() {
    let (browser, peer) = peer().await;
    let inventory = browser.tabs_for_user().await.unwrap();
    assert_eq!(inventory.tabs.len(), 2);
    assert!(
        !serde_json::to_string(&inventory)
            .unwrap()
            .contains("raw-target")
    );
    assert_eq!(
        peer.commands.lock().unwrap().len(),
        2,
        "inventory never attaches pages"
    );
    let grant = browser
        .grant_tab(&inventory.tabs[0].choice_id)
        .await
        .unwrap();
    let result = browser.granted_tab_metadata(&grant).await.unwrap();
    let repeated = browser
        .grant_tab(&inventory.tabs[0].choice_id)
        .await
        .unwrap();
    let other = browser
        .grant_tab(&inventory.tabs[1].choice_id)
        .await
        .unwrap();
    assert!(grant.same_target(&repeated));
    assert!(!grant.same_target(&other));
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(encoded.contains("Selected app"));
    assert!(!encoded.contains("Private unrelated") && !encoded.contains("raw-target"));
    assert_eq!(result.tab_id, grant.id());
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn raw_target_url_ordinal_unknown_and_expired_choices_do_not_issue_commands() {
    let (browser, peer) = peer().await;
    let first = browser.tabs_for_user().await.unwrap();
    let fresh = browser.tabs_for_user().await.unwrap();
    let before = peer.commands.lock().unwrap().len();
    for denied in [
        "raw-target-A",
        "https://fixture.test/a",
        "0",
        "missing",
        &first.tabs[0].choice_id,
    ] {
        assert_eq!(
            browser.grant_tab(denied).await.err(),
            Some(AttachError::StaleSelection)
        );
    }
    assert_eq!(peer.commands.lock().unwrap().len(), before);
    browser.grant_tab(&fresh.tabs[0].choice_id).await.unwrap();
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn a_grant_from_another_connection_is_rejected_before_protocol_io() {
    let (first, _first_peer) = peer().await;
    let (second, second_peer) = peer().await;
    let choice = first.tabs_for_user().await.unwrap().tabs.remove(0);
    let grant = first.grant_tab(&choice.choice_id).await.unwrap();
    let before = second_peer.commands.lock().unwrap().len();
    assert_eq!(
        second.granted_tab_metadata(&grant).await.err(),
        Some(AttachError::TabNotAuthorized)
    );
    assert_eq!(second_peer.commands.lock().unwrap().len(), before);
    first.disconnect().await.unwrap();
    second.disconnect().await.unwrap();
}

#[tokio::test]
async fn navigation_or_closure_between_offer_and_selection_requires_refresh() {
    let (browser, peer) = peer().await;
    let inventory = browser.tabs_for_user().await.unwrap();
    peer.targets.lock().unwrap()[0]["url"] = json!("https://changed.test/new-account");
    assert_eq!(
        browser.grant_tab(&inventory.tabs[0].choice_id).await.err(),
        Some(AttachError::StaleSelection)
    );
    peer.targets.lock().unwrap().remove(1);
    assert_eq!(
        browser.grant_tab(&inventory.tabs[1].choice_id).await.err(),
        Some(AttachError::StaleSelection)
    );
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn closed_granted_tab_never_falls_back_to_another_page() {
    let (browser, peer) = peer().await;
    let inventory = browser.tabs_for_user().await.unwrap();
    let grant = browser
        .grant_tab(&inventory.tabs[0].choice_id)
        .await
        .unwrap();
    peer.targets.lock().unwrap().remove(0);
    assert_eq!(
        browser.granted_tab_metadata(&grant).await.err(),
        Some(AttachError::StaleSelection)
    );
    let commands = peer.commands.lock().unwrap();
    assert_eq!(
        commands.last().unwrap()["params"]["targetId"],
        "raw-target-A"
    );
    drop(commands);
    browser.disconnect().await.unwrap();
    assert_eq!(
        browser.granted_tab_metadata(&grant).await.err(),
        Some(AttachError::ConnectionFailed)
    );
}

#[tokio::test]
async fn user_navigation_to_a_privileged_document_cannot_extend_a_page_grant() {
    use nomifun_browser_platform::system_browser::{
        SystemBrowserCommand, SystemBrowserRuntimeError,
    };
    let (browser, peer) = peer().await;
    let choice = browser.tabs_for_user().await.unwrap().tabs.remove(0);
    let grant = browser.grant_tab(&choice.choice_id).await.unwrap();
    peer.targets.lock().unwrap()[0]["url"] = json!("chrome://settings");
    assert_eq!(
        browser
            .execute_granted(
                &grant,
                SystemBrowserCommand::Observe {
                    tab_id: grant.id().into()
                },
                &tokio_util::sync::CancellationToken::new()
            )
            .await
            .err(),
        Some(SystemBrowserRuntimeError::TabDenied)
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
async fn inventory_filters_privileged_targets_and_limits_do_not_preserve_old_choices() {
    let (browser, peer) = peer().await;
    peer.targets.lock().unwrap().extend([
        target("settings", "Settings", "chrome://settings"),
        target("file", "File", "file:///private"),
        target("credentials", "Secret", "https://user:password@fixture.test"),
        json!({"type":"service_worker","targetId":"worker","title":"Worker","url":"https://fixture.test"}),
    ]);
    let initial = browser.tabs_for_user().await.unwrap();
    assert_eq!(initial.tabs.len(), 2);
    *peer.targets.lock().unwrap() = vec![target("many", "A", "https://fixture.test"); MAX_TABS + 1];
    assert_eq!(
        browser.tabs_for_user().await.err(),
        Some(AttachError::InventoryLimit)
    );
    assert_eq!(
        browser.grant_tab(&initial.tabs[0].choice_id).await.err(),
        Some(AttachError::StaleSelection)
    );
    browser.disconnect().await.unwrap();
}

#[tokio::test]
async fn disconnect_waits_for_operation_clones_and_blocks_queued_inventory() {
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
    let listing = tokio::spawn(async move { queued.tabs_for_user().await });
    drop(in_flight);
    drop(operation);
    close.await.unwrap().unwrap();
    assert_eq!(
        listing.await.unwrap().err(),
        Some(AttachError::ConnectionFailed)
    );
}

#[tokio::test]
async fn synchronous_disconnect_fence_interrupts_an_unanswered_inventory_read() {
    let (browser, peer) = peer().await;
    peer.targets.lock().unwrap()[0]["pause_inventory"] = json!(true);
    let browser = Arc::new(browser);
    let listing_browser = browser.clone();
    let listing = tokio::spawn(async move { listing_browser.tabs_for_user().await });
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
