//! Real Linux sandbox evidence through the product's pipe/process launcher.
//! Run explicitly with NOMIFUN_CHROME_BINARY; no download or normal profile use.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::target::{AttachToTargetParams, CreateTargetParams};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use nomi_browser_engine::launch::{LaunchConfig, launch_chrome};
use nomi_browser_engine::transport::ROOT_SESSION;

#[tokio::test]
#[ignore = "requires Linux sandbox prerequisites and explicit NOMIFUN_CHROME_BINARY"]
async fn linux_renderer_is_sandboxed_through_product_launcher() {
    let chrome = PathBuf::from(
        std::env::var_os("NOMIFUN_CHROME_BINARY")
            .expect("set NOMIFUN_CHROME_BINARY to an isolated Linux Chrome executable"),
    );
    assert!(chrome.is_absolute() && chrome.is_file());
    let profile = tempfile::tempdir().unwrap();
    let launched = launch_chrome(
        &LaunchConfig {
            chrome_path: chrome,
            user_data_dir: profile.path().to_owned(),
            headful: false,
        },
        true,
    )
    .await
    .expect("launch sandboxed Chromium through the product");
    let (mut process, connection) = launched.connect().await.expect("connect CDP pipe");

    // Inspect Chromium's own OS sandbox report, not just the absence of a flag.
    tokio::time::timeout(Duration::from_secs(20), async {
        let created = connection
            .send(ROOT_SESSION, &CreateTargetParams::new("chrome://sandbox"))
            .await
            .unwrap();
        let mut attach =
            AttachToTargetParams::new(created["targetId"].as_str().unwrap().to_owned());
        attach.flatten = Some(true);
        let attached = connection.send(ROOT_SESSION, &attach).await.unwrap();
        let session = attached["sessionId"].as_str().unwrap();
        let mut evaluate = EvaluateParams::new("document.body ? document.body.innerText : ''");
        evaluate.return_by_value = Some(true);
        loop {
            let result = connection.send(session, &evaluate).await.unwrap();
            let text = result["result"]["value"].as_str().unwrap_or_default();
            if text.contains("adequately sandboxed") {
                assert!(text.contains("You are adequately sandboxed"), "{text}");
                assert!(
                    text.lines().any(|line| {
                        line.split_whitespace().collect::<Vec<_>>()
                            == ["Seccomp-BPF", "sandbox", "Yes"]
                    }),
                    "{text}"
                );
                println!("Linux Chromium sandbox report:\n{text}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("sandbox status page must become available");

    // Pipe EOF stops Chromium; the retained product guard still owns whole-tree
    // cleanup and only then removes this launch's exact profile markers.
    connection.shutdown().await;
    drop(connection);
    tokio::time::timeout(Duration::from_secs(10), process.child_mut().wait())
        .await
        .expect("Chromium exits after pipe EOF")
        .unwrap();
    drop(process);
    let marker = profile
        .path()
        .join(nomi_browser_engine::profile::OWNERSHIP_MARKER_FILE);
    tokio::time::timeout(Duration::from_secs(10), async {
        while marker.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("product cleanup guard removes the exact runtime marker");
}
