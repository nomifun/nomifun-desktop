//! **多 origin localStorage 自动遍历恢复端到端集成**（`#[ignore]`，需 `NOMIFUN_CHROME_BINARY`）。
//!
//! 验证 `restore_all_origins` 能在一个 session 内自动遍历多 origin 并恢复 localStorage，
//! 且不与出口防火墙 loop 的 `Fetch.requestPaused` 冲突（用 `addScriptToEvaluateOnNewDocument`）。
//!
//! 手动跑：
//!   NOMIFUN_CHROME_BINARY="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(restore_all_origins)'

mod common;

use nomi_browser_engine::BrowserEngine;
use nomi_browser_engine::storage_state::{LocalStorageItem, OriginStorage, StorageState};

/// **restore_all_origins_restores_two_origins**: 2-origin localStorage restored in one session
/// without the caller pre-navigating each origin.
#[tokio::test]
#[ignore = "需 NOMIFUN_CHROME_BINARY（真 Chrome）：multi-origin localStorage auto-restore"]
async fn restore_all_origins_restores_two_origins() {
    const ORIGIN_A: &str = "https://example.com";
    const ORIGIN_B: &str = "https://www.iana.org";

    let state = StorageState {
        cookies: vec![],
        local_storage: vec![
            OriginStorage {
                origin: ORIGIN_A.into(),
                local_storage: vec![
                    LocalStorageItem { name: "key_a1".into(), value: "val_a1".into() },
                    LocalStorageItem { name: "key_a2".into(), value: "val_a2".into() },
                ],
                index_db: None,
            },
            OriginStorage {
                origin: ORIGIN_B.into(),
                local_storage: vec![
                    LocalStorageItem { name: "key_b1".into(), value: "val_b1".into() },
                ],
                index_db: None,
            },
        ],
    };

    let backend = common::build_backend_for_fixture("multi-origin-ls").await;

    // Restore all origins in one call (no manual per-origin navigate by caller).
    backend
        .restore_all_origins(&state)
        .await
        .expect("restore_all_origins must succeed");

    // ── Verify origin A ──
    backend.navigate(ORIGIN_A, false).await.expect("nav A");
    let r = backend
        .__eval_page_world_for_test("localStorage.getItem('key_a1')")
        .await
        .expect("read key_a1");
    assert_eq!(
        r.get("value").and_then(|v| v.as_str()),
        Some("val_a1"),
        "origin A key_a1 must be restored"
    );
    let r = backend
        .__eval_page_world_for_test("localStorage.getItem('key_a2')")
        .await
        .expect("read key_a2");
    assert_eq!(
        r.get("value").and_then(|v| v.as_str()),
        Some("val_a2"),
        "origin A key_a2 must be restored"
    );

    // ── Verify origin B ──
    backend.navigate(ORIGIN_B, false).await.expect("nav B");
    let r = backend
        .__eval_page_world_for_test("localStorage.getItem('key_b1')")
        .await
        .expect("read key_b1");
    assert_eq!(
        r.get("value").and_then(|v| v.as_str()),
        Some("val_b1"),
        "origin B key_b1 must be restored"
    );
    // Ensure origin B does NOT have origin A's keys (origin isolation).
    let r = backend
        .__eval_page_world_for_test("localStorage.getItem('key_a1')")
        .await
        .expect("read key_a1 from B");
    assert_eq!(
        r.get("value"),
        Some(&serde_json::Value::Null),
        "origin B must NOT have origin A's keys (origin isolation)"
    );

    eprintln!("=== PASS: restore_all_origins_restores_two_origins");
}
