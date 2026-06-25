//! **P3 W4b：cookie storage_state 捕获/恢复 端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 W4b（cookie 捕获/恢复机制）：
//! - **往返保真**：restore 一条全字段 cookie（含 sameSite + 持久 expires）→
//!   capture 读回 → 名/值/域/sameSite/expires 全保真（**核心验收：cookie 跨 capture/restore 往返保真**）。
//! - **登录态可灌**：restore 的 cookie 真写进 cookie store（capture 读回即证「灌入生效」，
//!   且 navigate 到该域后仍在——浏览器接受了它为该域的 cookie，跨导航存活 = 登录态可经 restore 灌入）。
//! - **默认 context 零回归**：capture/restore 走默认 context（不传 browserContextId），机制仍工作。
//!
//! **W4b 不验 localStorage/IndexedDB（W4c）也不验磁盘 vault 持久化（W4d）**。本测试只走**内存往返**
//! （capture → StorageState → restore）。
//!
//! **为何用 `Storage.getCookies/setCookies` 而非 `Network.getAllCookies`**：见
//! [`nomi_browser_engine::backend::CdpBackend::capture_cookies`] 文档——本 chromiumoxide_cdp 版本无
//! `Network.getAllCookies`，且 `Network.*` 是 session 级无法按 browserContextId 取/设；`Storage.*` 支持
//! `browserContextId`，是正确的 CDP 面。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(w4b)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop + disposeOnDetach 自动清）。
//!
//! 真实结果（本机首跑 eprintln 出捕到的 cookie——填回任务汇报）。

use nomi_browser_engine::storage_state::{SameSite, StorageState, StorageStateCookie};
use nomi_browser_engine::BrowserEngine;

mod common;

/// 造一条**全字段非默认**的 storage_state cookie（持久 + sameSite=Lax + secure），灌进 context 后
/// 应能原样 capture 回。`name`/`value` 用易辨识串便于断言。
///
/// **注**：不带 CHIPS partitionKey——分区 cookie 的 `Storage.setCookies` 接受依赖 chrome 的 CHIPS 支持
/// 与 topLevelSite 形态，集成里灌非分区 cookie 验主路径（partitionKey 的纯逻辑往返已在
/// `storage_state::tests` 钉死；分区 cookie 真灌入留 W4d/真站验）。
fn login_cookie(name: &str, value: &str) -> StorageStateCookie {
    StorageStateCookie {
        name: name.into(),
        value: value.into(),
        // example.com（与下方 navigate 目标同域，验「登录态灌入该域」）。
        domain: ".example.com".into(),
        path: "/".into(),
        // 持久 cookie（远未来过期），非 session——跨导航存活。
        expires: 4_102_444_800.0, // 2100-01-01
        http_only: false,         // 非 httpOnly：若要 document.cookie 验也读得到（本测试以 capture 为主）。
        secure: false,
        session: false,
        same_site: Some(SameSite::Lax),
        priority: nomi_browser_engine::storage_state::Priority::Medium,
        source_scheme: nomi_browser_engine::storage_state::SourceScheme::NonSecure,
        source_port: -1,
        partition_key: None,
    }
}

/// 在一组 capture 回的 cookie 里按 name 找。
fn find<'a>(state: &'a StorageState, name: &str) -> Option<&'a StorageStateCookie> {
    state.cookies.iter().find(|c| c.name == name)
}

/// **往返保真 + 登录态可灌**：restore 一条全字段 cookie → capture 读回保真 →
/// navigate 到该域后再 capture 仍在（浏览器接受为该域 cookie，跨导航存活 = 登录态灌入生效）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4b_cookie_round_trip_and_login_state_loadable() {
    let backend = common::build_backend_for_fixture("w4b-rt").await;

    // 1) restore：把一条登录态 cookie 灌进默认 context。
    let state = StorageState {
        cookies: vec![login_cookie("nomi_session", "login-token-abc123")],
        ..Default::default()
    };
    backend
        .restore_cookies(&state)
        .await
        .expect("restore_cookies into default context");

    // 2) capture：读回——名/值/域/sameSite/expires 全保真（核心验收：往返保真）。
    let captured = backend.capture_cookies().await.expect("capture_cookies");
    eprintln!(
        "=== w4b captured {} cookie(s) === {:?}",
        captured.cookies.len(),
        captured.cookies
    );
    let c = find(&captured, "nomi_session")
        .expect("restored cookie must be captured back (login state loaded into context)");
    assert_eq!(c.value, "login-token-abc123", "cookie value must round-trip");
    assert_eq!(c.domain, ".example.com", "cookie domain must round-trip");
    assert_eq!(c.path, "/", "cookie path must round-trip");
    assert_eq!(c.same_site, Some(SameSite::Lax), "sameSite must round-trip (not lost/defaulted)");
    assert!(!c.session, "persistent cookie must not be a session cookie");
    // 持久 cookie：expires 是一个真实的未来时间戳（非 -1/session）。**注**：Chrome 把远未来过期时间
    // 钳到「~400 天上限」（cookie max-age 政策，RFC 6265bis），故灌 2100 年会被钳到约一年后——这是
    // 浏览器正确行为，不是丢字段。验「仍是持久 cookie（expires>0 且远在未来）」而非精确值。
    assert!(
        c.expires > 1_700_000_000.0,
        "persistent cookie must keep a real future expiry (Chrome clamps far-future to ~400d), got {}",
        c.expires
    );

    // 3) 登录态可灌：navigate 到该域后 cookie 仍在 context（浏览器接受为 example.com 的 cookie，
    //    跨导航存活）——这就是「登录态经 restore 灌入、后续请求带得上」的真实信号。
    let nav = backend
        .navigate("https://example.com", false)
        .await
        .expect("navigate to example.com");
    eprintln!("=== w4b nav after restore === final_url={}", nav.final_url);
    let after_nav = backend.capture_cookies().await.expect("capture after navigate");
    assert!(
        find(&after_nav, "nomi_session").is_some(),
        "restored cookie must survive navigation (login state loadable into the domain): {:?}",
        after_nav.cookies
    );
}

/// **默认 context 零回归**：capture/restore 走默认 context（不传
/// browserContextId），机制仍工作（restore→capture 往返）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4b_default_context_capture_restore_works() {
    let backend = common::build_backend_for_fixture("w4b-default").await;

    let state = StorageState {
        cookies: vec![login_cookie("default_ctx_cookie", "v-default")],
        ..Default::default()
    };
    backend.restore_cookies(&state).await.expect("restore default context");
    let captured = backend.capture_cookies().await.expect("capture default context");
    eprintln!("=== w4b default-context captured {} cookie(s) ===", captured.cookies.len());
    let c = find(&captured, "default_ctx_cookie")
        .expect("default-context cookie must round-trip");
    assert_eq!(c.value, "v-default");
}
