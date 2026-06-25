//! **P3 W4c：localStorage storage_state 捕获/恢复 端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 W4c（localStorage（origin-bound）捕获/恢复机制）：
//! - **localStorage 跨 capture/restore 往返保真（origin-bound，核心验收）**：在引擎 A 页面
//!   `setItem` 几个键 → `capture_local_storage` 读回（origin + 键值保真）→ 在**新**引擎 B
//!   navigate 到同 origin → `restore_local_storage` 灌进去 → 页面读回 `getItem` 验值生效。
//! - **origin-bound**：捕获/恢复都绑**当前页面 origin**（localStorage 同源分区，无法跨 origin 全局取/设）；
//!   restore 只灌 state 中 origin == 当前页面 origin 的那份（不跨 origin 误写）。
//! - **默认 context 零回归**：capture/restore 走默认 context 页面，机制仍工作。
//!
//! **W4c 不验 IndexedDB（best-effort/TODO，`OriginStorage.index_db` 恒 None）也不验磁盘 vault 持久化
//! （W4d）**。只走**内存往返**（capture → StorageState → restore）+ origin-bound 注入。
//!
//! **为何用真实 `https://example.com` 而非 file:// fixture**：localStorage 在 `file://` origin 上
//! Chrome 行为不稳（origin 形态 `null`/`file://` 视版本而异，且 storage 可能被分区/禁用）；`https://
//! example.com` 是稳定可达的真实 origin（与 W4b cookie 测试同选），localStorage 在其上可靠工作——
//! 这是「origin-bound 往返保真」最贴近真实登录态的验证场景。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(w4c)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop + disposeOnDetach 自动清）。
//!
//! 真实结果（本机首跑 eprintln 出捕到的 localStorage——填回任务汇报）。

use nomi_browser_engine::storage_state::{OriginStorage, StorageState};
use nomi_browser_engine::BrowserEngine;

mod common;

const ORIGIN: &str = "https://example.com";

/// 在当前页面（默认 page world）`setItem` 一组键值（测试 seam：模拟「页面写了 localStorage 登录态」）。
async fn seed_local_storage(
    backend: &nomi_browser_engine::backend::CdpBackend,
    pairs: &[(&str, &str)],
) {
    // 用 JSON 安全编码键值，逐键 setItem。
    let pairs_owned: Vec<[&str; 2]> = pairs.iter().map(|(k, v)| [*k, *v]).collect();
    let pairs_json = serde_json::to_string(&pairs_owned).expect("json pairs");
    let script = format!(
        "(() => {{ const pairs = {pairs_json}; for (const [k, v] of pairs) localStorage.setItem(k, v); return localStorage.length; }})()"
    );
    let r = backend
        .__eval_page_world_for_test(&script)
        .await
        .expect("seed localStorage setItem");
    eprintln!("=== seeded localStorage, length now = {:?}", r.get("value"));
}

/// 读回当前页面某 localStorage 键（默认 page world `getItem`）。None = 键不存在（返回 JS null）。
async fn read_local_storage(
    backend: &nomi_browser_engine::backend::CdpBackend,
    key: &str,
) -> Option<String> {
    let key_json = serde_json::to_string(key).expect("json key");
    let script = format!("localStorage.getItem({key_json})");
    let r = backend
        .__eval_page_world_for_test(&script)
        .await
        .expect("read localStorage getItem");
    r.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// **核心：localStorage 跨 capture/restore 往返保真（origin-bound）**。
/// engine A：navigate example.com → seed localStorage → capture（origin + 键值保真）。
/// engine B（新引擎，独立 profile）：navigate example.com → restore A 的快照 → 页面 getItem 读回值生效。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4c_local_storage_round_trip_origin_bound() {
    // ── 捕获侧：engine A ──────────────────────────────────────────────────────
    let backend_a = common::build_backend_for_fixture("w4c-capA").await;
    let nav = backend_a
        .navigate(ORIGIN, false)
        .await
        .expect("navigate A to example.com");
    eprintln!("=== w4c capture nav === final_url={}", nav.final_url);

    // 页面写 localStorage（含特殊字符值，验往返不被吞）。
    seed_local_storage(
        &backend_a,
        &[
            ("ls_auth", "jwt.eyJhbGciOiJ9.sig-abc123"),
            ("ls_theme", "dark"),
            ("ls_json", "{\"a\":1,\"b\":\"x=y&z\"}"),
        ],
    )
    .await;

    // capture：读回当前 origin 的 localStorage（origin + 键值保真）。
    let captured = backend_a
        .capture_local_storage()
        .await
        .expect("capture_local_storage")
        .expect("page has an origin to capture");
    eprintln!(
        "=== w4c captured origin={} items={:?}",
        captured.origin, captured.local_storage
    );
    // **核心验收：捕到的 origin == 页面 origin；键值保真**。
    assert_eq!(captured.origin, ORIGIN, "captured origin must match page origin (origin-bound)");
    let find = |k: &str| {
        captured
            .local_storage
            .iter()
            .find(|i| i.name == k)
            .map(|i| i.value.as_str())
    };
    assert_eq!(find("ls_auth"), Some("jwt.eyJhbGciOiJ9.sig-abc123"), "auth value captured");
    assert_eq!(find("ls_theme"), Some("dark"), "theme value captured");
    assert_eq!(find("ls_json"), Some("{\"a\":1,\"b\":\"x=y&z\"}"), "special chars captured intact");

    // 组装一份 storage_state（只含 localStorage——本任务范围）。
    let state = StorageState {
        cookies: vec![],
        local_storage: vec![captured.clone()],
    };

    // ── 恢复侧：engine B（独立 profile，localStorage 必为空白起点）──────────────────
    let backend_b = common::build_backend_for_fixture("w4c-resB").await;
    backend_b
        .navigate(ORIGIN, false)
        .await
        .expect("navigate B to example.com");

    // 恢复前：B 的 localStorage 该键应不存在（新引擎 + 独立 profile = 干净起点）。
    assert_eq!(
        read_local_storage(&backend_b, "ls_auth").await,
        None,
        "fresh engine B must not have A's localStorage before restore"
    );

    // restore：把 A 的 localStorage 灌进 B 当前页面（origin-bound：origin 匹配 example.com）。
    backend_b
        .restore_local_storage(&state)
        .await
        .expect("restore_local_storage into B");

    // 页面读回：值生效（往返保真——这是「localStorage 登录态经 restore 灌入、页面读得到」的真实信号）。
    let auth = read_local_storage(&backend_b, "ls_auth").await;
    let theme = read_local_storage(&backend_b, "ls_theme").await;
    let json = read_local_storage(&backend_b, "ls_json").await;
    eprintln!("=== w4c restored read-back === ls_auth={auth:?} ls_theme={theme:?} ls_json={json:?}");
    assert_eq!(
        auth.as_deref(),
        Some("jwt.eyJhbGciOiJ9.sig-abc123"),
        "restored localStorage value must be readable in page (round-trip fidelity)"
    );
    assert_eq!(theme.as_deref(), Some("dark"));
    assert_eq!(json.as_deref(), Some("{\"a\":1,\"b\":\"x=y&z\"}"), "special chars survive restore");

    // re-capture 在 B：应捕到刚恢复的项（capture↔restore 对称）。
    let recap = backend_b
        .capture_local_storage()
        .await
        .expect("re-capture B")
        .expect("B has origin");
    assert!(
        recap.local_storage.iter().any(|i| i.name == "ls_auth"),
        "re-capture after restore must see the restored key: {:?}",
        recap.local_storage
    );
}

/// **origin-bound：restore 不跨 origin 误写**。state 里只有 example.com 的 localStorage；当前页面停在
/// about:blank（无 example.com origin）→ restore 是 no-op（不把 example.com 的键写进无关页面）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4c_restore_is_origin_bound_no_cross_origin_write() {
    let backend = common::build_backend_for_fixture("w4c-origin").await;

    // 当前页面在另一个 origin（fixture file://），与 state 里的 example.com 不同 origin。
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate to fixture origin");

    // state 只含 example.com 的 localStorage。
    let state = StorageState {
        cookies: vec![],
        local_storage: vec![OriginStorage::new_local_storage(
            ORIGIN,
            [("ls_cross", "should-not-leak".to_string())]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v)),
        )],
    };
    // restore：当前页面 origin != example.com → no-op（绝不把 example.com 键写进 fixture 页面）。
    backend
        .restore_local_storage(&state)
        .await
        .expect("restore is no-op when origin mismatches (origin-bound)");

    // fixture 页面不该出现 example.com 的键（origin-bound 防误写）。
    let leaked = read_local_storage(&backend, "ls_cross").await;
    eprintln!("=== w4c origin-bound no-cross-write === fixture-page ls_cross={leaked:?}");
    assert_eq!(
        leaked, None,
        "restore must NOT write example.com's localStorage into a different-origin page (origin-bound)"
    );
}

/// **默认 context 零回归**：capture/restore 走默认 context 页面，机制仍工作
/// （往返保真）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4c_default_context_local_storage_works() {
    let backend = common::build_backend_for_fixture("w4c-default").await;

    backend
        .navigate(ORIGIN, false)
        .await
        .expect("navigate default context to example.com");
    seed_local_storage(&backend, &[("ls_default", "v-default-ctx")]).await;

    let captured = backend
        .capture_local_storage()
        .await
        .expect("capture default context")
        .expect("default context page has origin");
    eprintln!(
        "=== w4c default-context captured origin={} items={:?}",
        captured.origin, captured.local_storage
    );
    assert_eq!(captured.origin, ORIGIN);
    assert!(
        captured.local_storage.iter().any(|i| i.name == "ls_default" && i.value == "v-default-ctx"),
        "default-context localStorage must round-trip: {:?}",
        captured.local_storage
    );

    // restore 同一份回当前页面（覆盖即幂等）→ 读回仍在。
    let state = StorageState {
        cookies: vec![],
        local_storage: vec![captured],
    };
    backend
        .restore_local_storage(&state)
        .await
        .expect("restore default context");
    assert_eq!(
        read_local_storage(&backend, "ls_default").await.as_deref(),
        Some("v-default-ctx"),
        "default-context restore round-trip"
    );
}
