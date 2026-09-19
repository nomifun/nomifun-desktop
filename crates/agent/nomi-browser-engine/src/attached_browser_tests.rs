use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[cfg(target_os = "macos")]
#[test]
fn macos_install_connection_uses_chromes_documented_user_data_root() {
    assert_eq!(
        macos_chrome_port_file(Path::new("/Users/alice")),
        Path::new("/Users/alice")
            .join("Library/Application Support/Google/Chrome/DevToolsActivePort")
    );
}

#[test]
fn endpoint_is_loopback_only_and_never_treated_as_a_url() {
    assert_eq!(
        parse_endpoint(b"9222\n/devtools/browser/abc-123\n").unwrap(),
        "ws://127.0.0.1:9222/devtools/browser/abc-123"
    );
    assert_eq!(
        parse_endpoint(b"9222\r\n/devtools/browser/abc_123\r\n").unwrap(),
        "ws://127.0.0.1:9222/devtools/browser/abc_123"
    );
    for input in [
        "",
        "0\n/devtools/browser/a",
        "65536\n/devtools/browser/a",
        "9222garbage\n/devtools/browser/a",
        "-1\n/devtools/browser/a",
        "9222\nws://example.test/",
        "9222\n/devtools/browser/a?token=secret",
        "9222\n/devtools/browser/../page/a",
        "9222\n/devtools/browser/%2f",
        "9222\n/devtools/browser/",
        "9222\n/devtools/page/a",
        "9222\n/devtools/browser/a\nextra",
        " 9222\n/devtools/browser/a",
        "9222\n/devtools/browser/a#secret",
        "9222\n/devtools/browser/a@remote",
        "9222\n/devtools/browser/中文",
    ] {
        assert_eq!(
            parse_endpoint(input.as_bytes()),
            Err(AttachError::InvalidEndpoint),
            "{input:?}"
        );
    }
    assert_eq!(
        parse_endpoint(&vec![b'0'; 1025]),
        Err(AttachError::InvalidEndpoint)
    );
}

async fn peer(
    product: &'static str,
    close_after_probe: bool,
) -> (
    tempfile::TempDir,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("DevToolsActivePort"),
        format!(
            "{}\n/devtools/browser/fixture\n",
            listener.local_addr().unwrap().port()
        ),
    )
    .unwrap();
    // The same disposable directory includes sentinel login/profile bytes. The
    // connector has no authority to read, modify, copy, or delete these files.
    std::fs::write(dir.path().join("Cookies"), b"fixture-login-do-not-touch").unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&commands);
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        while let Some(Ok(message)) = ws.next().await {
            match message {
                Message::Text(text) => {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    seen.lock()
                        .unwrap()
                        .push(request["method"].as_str().unwrap().into());
                    assert_eq!(request["method"], "Browser.getVersion");
                    assert!(request.get("sessionId").is_none());
                    ws.send(Message::Text(
                        json!({"id":request["id"], "result":{"product":product}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                    if close_after_probe {
                        ws.close(None).await.unwrap();
                        break;
                    }
                }
                Message::Close(_) => {
                    let _ = ws.flush().await;
                    break;
                }
                _ => {}
            }
        }
        // The listener is still live, standing in for the external browser.
        // A reconnect attempt would succeed here and fail this assertion.
        assert!(
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err()
        );
    });
    (dir, commands, task)
}

#[tokio::test]
async fn attach_and_disconnect_never_launch_attach_pages_or_close_browser() {
    let (dir, commands, task) = peer("Chrome/152.0.0.0", false).await;
    let browser = AttachedBrowser::connect_port_file(&dir.path().join("DevToolsActivePort"))
        .await
        .unwrap();
    assert_eq!(browser.chromium_major(), 152);
    assert!(browser.is_connected());
    browser.disconnect().await.unwrap();
    assert!(!browser.is_connected());
    browser.disconnect().await.unwrap();
    task.await.unwrap();
    assert_eq!(*commands.lock().unwrap(), ["Browser.getVersion"]);
    assert_eq!(
        std::fs::read(dir.path().join("Cookies")).unwrap(),
        b"fixture-login-do-not-touch"
    );
    assert!(dir.path().join("DevToolsActivePort").is_file());
}

#[tokio::test]
async fn dropping_the_owner_closes_only_its_socket() {
    let (dir, commands, task) = peer("Chrome/152.0.0.0", false).await;
    let browser = AttachedBrowser::connect_port_file(&dir.path().join("DevToolsActivePort"))
        .await
        .unwrap();
    drop(browser);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*commands.lock().unwrap(), ["Browser.getVersion"]);
    assert_eq!(
        std::fs::read(dir.path().join("Cookies")).unwrap(),
        b"fixture-login-do-not-touch"
    );
}

#[tokio::test]
async fn incompatible_browser_disconnects_without_fallback() {
    for product in [
        "Chrome/143.0.0.0",
        "HeadlessChrome/152.0.0.0",
        "private-invalid-product",
    ] {
        let (dir, commands, task) = peer(product, false).await;
        let error = AttachedBrowser::connect_port_file(&dir.path().join("DevToolsActivePort"))
            .await
            .err()
            .unwrap();
        assert_eq!(error, AttachError::UnsupportedBrowser);
        assert!(!error.to_string().contains(product));
        task.await.unwrap();
        assert_eq!(*commands.lock().unwrap(), ["Browser.getVersion"]);
    }
}

#[tokio::test]
async fn lost_browser_connection_never_reconnects() {
    let (dir, commands, task) = peer("Chrome/152.0.0.0", true).await;
    // A close racing the initial version result may prevent publication; either
    // outcome must remain terminal, never auto-connect a replacement socket.
    if let Ok(browser) =
        AttachedBrowser::connect_port_file(&dir.path().join("DevToolsActivePort")).await
    {
        tokio::time::timeout(Duration::from_secs(5), async {
            while browser.is_connected() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        browser.disconnect().await.unwrap();
    }
    task.await.unwrap();
    assert_eq!(*commands.lock().unwrap(), ["Browser.getVersion"]);
}

#[tokio::test]
async fn missing_or_malformed_discovery_file_is_read_only_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("DevToolsActivePort");
    assert_eq!(
        AttachedBrowser::connect_port_file(&path).await.err(),
        Some(AttachError::NotReady)
    );
    assert!(!path.exists());
    for input in [b"private-fixture-cookie".to_vec(), vec![b'p'; 1025]] {
        std::fs::write(&path, &input).unwrap();
        assert_eq!(
            AttachedBrowser::connect_port_file(&path).await.err(),
            Some(AttachError::InvalidEndpoint)
        );
        assert_eq!(std::fs::read(&path).unwrap(), input);
    }
}

#[tokio::test]
async fn cancellation_during_initial_probe_drops_the_socket_without_browser_cleanup() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let port_file = dir.path().join("DevToolsActivePort");
    std::fs::write(
        &port_file,
        format!(
            "{}\n/devtools/browser/fixture",
            listener.local_addr().unwrap().port()
        ),
    )
    .unwrap();
    let (probe_tx, probe_rx) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let request = ws.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&request).unwrap()["method"],
            "Browser.getVersion"
        );
        probe_tx.send(()).unwrap();
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
        {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {}
            other => panic!("unexpected command after cancellation: {other:?}"),
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err()
        );
    });
    let connecting =
        tokio::spawn(async move { AttachedBrowser::connect_port_file(&port_file).await });
    probe_rx.await.unwrap();
    connecting.abort();
    assert!(connecting.await.is_err());
    peer.await.unwrap();
}

#[tokio::test]
async fn disconnect_disposes_tcp_even_if_peer_never_acknowledges_websocket_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let port_file = dir.path().join("DevToolsActivePort");
    std::fs::write(
        &port_file,
        format!(
            "{}\n/devtools/browser/fixture",
            listener.local_addr().unwrap().port()
        ),
    )
    .unwrap();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let request: Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        ws.send(Message::Text(
            json!({"id":request["id"],"result":{"product":"Chrome/152.0.0.0"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        assert!(matches!(ws.next().await, Some(Ok(Message::Close(_)))));
        // Do not flush the queued close acknowledgment: observe physical TCP
        // retirement, not merely receipt of a WebSocket control message.
        let mut byte = [0u8];
        let eof = tokio::time::timeout(Duration::from_secs(5), ws.get_mut().read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(eof, Ok(0)) || eof.is_err());
    });
    let browser = AttachedBrowser::connect_port_file(&port_file)
        .await
        .unwrap();
    browser.disconnect().await.unwrap();
    // Keep the wrapper alive: disconnect itself must release the socket.
    assert!(!browser.is_connected());
    peer.await.unwrap();
    drop(browser);
}

#[tokio::test]
async fn cancelled_disconnect_preserves_one_joinable_physical_retirement() {
    let (dir, commands, peer) = peer("Chrome/152.0.0.0", false).await;
    let browser = Arc::new(
        AttachedBrowser::connect_port_file(&dir.path().join("DevToolsActivePort"))
            .await
            .unwrap(),
    );
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    browser.state.lock().unwrap().retirement_pause = Some((started_tx, resume_rx));
    let first_browser = Arc::clone(&browser);
    let first = tokio::spawn(async move { first_browser.disconnect().await });
    started_rx.await.unwrap();
    assert!(!browser.is_connected());
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let second_browser = Arc::clone(&browser);
    let mut second = tokio::spawn(async move { second_browser.disconnect().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut second)
            .await
            .is_err()
    );
    resume_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    browser.disconnect().await.unwrap();
    peer.await.unwrap();
    assert_eq!(*commands.lock().unwrap(), ["Browser.getVersion"]);
}
