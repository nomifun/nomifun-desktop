//! **P2 D2：navigate settle 升级端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 D2 的成熟导航判定（DESIGN §12 + 裁决⑤）：
//! - 普通页 navigate → load_state 达 Load/NetworkIdle（视页面是否真静默）。
//! - networkidle 永不空闲页（长轮询）→ networkidle 短 cap（~4s）降级返 Load，**不卡 30s**。
//! - SPA history.pushState 软导航 → navigatedWithinDocument 降级判定（不重新等 load）。
//! - 真站（HTTP）→ http_status==200 填充；redirect 用 URL-normalize 不误判 trailing-slash。
//!
//! 复用 `tests/common` 的 `build_backend_for_fixture`（勿再复制契约母本）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(nav)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。
//!
//! 真实结果（本机首跑会 eprintln 出 load_state/http_status/耗时——填回任务汇报）。

use std::time::Instant;

use nomi_browser_engine::{BrowserEngine, LoadState};

mod common;

/// 普通 file:// 页 navigate：settle 阶梯走通，load_state 达 Load（file:// 无 HTTP 故 http_status
/// 可能为 None），耗时远小于 30s 总超时。act-c1.html 是静态页（脚本只挂事件，无持续网络）→ 应在
/// networkidle 短 cap 内达到 NetworkIdle，或至少 Load。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn nav_normal_page_reaches_load_or_networkidle() {
    let backend = common::build_backend_for_fixture("nav-normal").await;

    let t0 = Instant::now();
    let nav = backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate act-c1.html");
    let elapsed = t0.elapsed();

    eprintln!(
        "=== nav_normal_page === final_url={} http_status={:?} redirected={} load_state={} elapsed={:?}",
        nav.final_url, nav.http_status, nav.redirected, nav.load_state, elapsed
    );

    // 静态页：应达 Load 或 NetworkIdle（不会停在 commit/DCL——本页极快）。
    assert!(
        matches!(nav.load_state, LoadState::Load | LoadState::NetworkIdle),
        "static page should reach Load/NetworkIdle, got {}",
        nav.load_state
    );
    // file:// 自身不算 redirect（归一化比较：请求 url == final url）。
    assert!(!nav.redirected, "file:// self-nav must not be flagged redirect");
    // 整个 navigate 应远小于 30s（即便 networkidle cap 触发也只 +4s）。
    assert!(
        elapsed.as_secs() < 15,
        "navigate took too long ({elapsed:?}); networkidle cap must not blow up nav timeout"
    );
}

/// networkidle 永不空闲页（长轮询 fixture）：networkidle 短 cap（~4s）到点降级返 Load，**绝不**卡到
/// 30s。这是裁决⑤ 的核心不变量——长轮询/SSE/WS 站永不 idle 也不能拖垮 navigate。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn nav_never_idle_page_degrades_to_load_within_cap() {
    let backend = common::build_backend_for_fixture("nav-never-idle").await;

    let t0 = Instant::now();
    let nav = backend
        .navigate(&common::fixture_url("never-idle.html"), false)
        .await
        .expect("navigate never-idle.html");
    let elapsed = t0.elapsed();

    eprintln!(
        "=== nav_never_idle === load_state={} elapsed={:?} (networkidle cap degrade expected)",
        nav.load_state, elapsed
    );

    // 永不空闲 → networkidle 等不到 → 降级返 Load（良性，不报错）。
    assert_eq!(
        nav.load_state,
        LoadState::Load,
        "never-idle page must degrade to Load (not NetworkIdle)"
    );
    // 关键不变量：cap 独立，总耗时 ~ load + 4s cap，远小于 30s。给宽松上限 20s 防慢机器 flaky，
    // 但必须显著小于 30s 才能证明「cap 没并入 nav 超时」。
    assert!(
        elapsed.as_secs() < 20,
        "networkidle cap must be independent of 30s nav timeout; elapsed={elapsed:?}"
    );
}

/// SPA 软导航：本 fixture load 后自动 history.pushState 改 URL（navigatedWithinDocument，无新文档）。
/// navigate 应识别软导航降级路径（不重新等 load 超时），良性返回成功。
///
/// 注意：navigate 的初始文档 load 与之后的软导航是两件事——navigate 返回时通常已达 Load（初始文档
/// 的 load 先到）。本测试主要验证「不报错、final_url 反映软导航后的 URL（若软导航在 navigate 返回
/// 前发生）」+ 不卡死。软导航的「不重新等 load」降级路径在 run_settle 内（若软导航先于 load 到达）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn nav_spa_soft_navigation_is_benign() {
    let backend = common::build_backend_for_fixture("nav-spa").await;

    let t0 = Instant::now();
    let nav = backend
        .navigate(&common::fixture_url("spa-softnav.html"), false)
        .await
        .expect("navigate spa-softnav.html");
    let elapsed = t0.elapsed();

    eprintln!(
        "=== nav_spa === final_url={} load_state={} redirected={} elapsed={:?}",
        nav.final_url, nav.load_state, nav.redirected, elapsed
    );

    // 软导航是良性态：navigate 成功（不 Err），不卡死。
    assert!(
        elapsed.as_secs() < 15,
        "SPA nav must not hang on load timeout; elapsed={elapsed:?}"
    );
    // load_state 至少达 DOMContentLoaded（DOM 已构建）；多数情况达 Load（初始文档 load 先到）。
    assert!(
        matches!(
            nav.load_state,
            LoadState::DomContentLoaded | LoadState::Load | LoadState::NetworkIdle
        ),
        "SPA page should reach at least DOMContentLoaded, got {}",
        nav.load_state
    );
}

/// 真站（HTTP）：http_status==200 填充 + redirect URL-normalize 不误判。仅当设了
/// `NOMIFUN_NAV_HTTP_TEST=1`（避免离线环境 flaky）才真跑——否则 eprintln 跳过原因后返回。
///
/// 用 https://example.com（稳定、无重定向、明确 200）。验证：
/// - http_status == Some(200)（D2 从主帧 Document responseReceived 取到）。
/// - example.com → example.com/（trailing-slash）归一化后**不**算 redirect（裸 != 会误报）。
#[tokio::test]
#[ignore = "需本机/打包 chrome + 网络：set NOMIFUN_CHROME_BINARY + NOMIFUN_NAV_HTTP_TEST=1 后 --run-ignored all"]
async fn nav_http_status_and_redirect_normalize() {
    if std::env::var("NOMIFUN_NAV_HTTP_TEST").ok().as_deref() != Some("1") {
        eprintln!("=== nav_http_status === SKIPPED (set NOMIFUN_NAV_HTTP_TEST=1 to run online)");
        return;
    }
    let backend = common::build_backend_for_fixture("nav-http").await;

    let t0 = Instant::now();
    let nav = backend
        .navigate("https://example.com", false)
        .await
        .expect("navigate example.com");
    let elapsed = t0.elapsed();

    eprintln!(
        "=== nav_http_status === final_url={} http_status={:?} redirected={} load_state={} elapsed={:?}",
        nav.final_url, nav.http_status, nav.redirected, nav.load_state, elapsed
    );

    assert_eq!(nav.http_status, Some(200), "example.com should return HTTP 200");
    // example.com → example.com/（浏览器补 trailing slash）：归一化比较**不**算 redirect。
    assert!(
        !nav.redirected,
        "trailing-slash difference must not be flagged as redirect (URL-normalize), final_url={}",
        nav.final_url
    );
    assert!(nav.final_url.contains("example.com"));
}
