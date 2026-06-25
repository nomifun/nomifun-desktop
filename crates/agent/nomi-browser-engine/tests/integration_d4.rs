//! **P2 D4：back/forward/reload/switch_frame 端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 D4（DESIGN §12 导航 / §13 Target-Tab + 裁决⑤ settle 复用 / ⑧ POST reload→IRREVERSIBLE）：
//! - **back/forward**：navigate A → navigate B → `act(Back)` → URL/内容回 A → `act(Forward)` → 回 B；
//!   settle 复用 D2（load_state 正确）；首页再 back / 末页再 forward → 良性「无更多历史」success=true。
//! - **reload**：navigate → `act(Reload)` → 页面重载（load_state 达 Load/NetworkIdle，不报错）。
//! - **switch_frame**：含 iframe 的 fixture → observe 看到 iframe ref → `act(SwitchFrame{ref})` 进 iframe
//!   → 页面级动作（get_page_text）作用于 **iframe 内容**（读到 IFRAME_INNER_MARKER 而非 MAIN_DOC_MARKER）；
//!   切回主帧（switch_frame "main"）→ get_page_text 又读到主帧内容。
//!
//! 复用 `tests/common` 的 `build_backend_for_fixture`（勿再复制契约母本）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all \
//!     -E 'test(back) | test(forward) | test(reload) | test(switch_frame) | test(history)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。
//!
//! 真实结果（本机首跑会 eprintln 出 URL 回退 / iframe 文本——填回任务汇报）。

use std::time::Duration;

use nomi_browser_engine::actions::ActSpec;
use nomi_browser_engine::progress::Progress;
use nomi_browser_engine::{BrowserEngine, LoadState, ObserveOpts};

mod common;

/// 动作级 Progress（充裕 deadline，集成测试不为超时挂死；abort 仍按事件源触发）。
fn act_progress() -> Progress {
    Progress::new(Duration::from_secs(60))
}

/// **back/forward 端到端**：A→B→back（回 A）→forward（回 B），settle 复用 D2，load_state 正确；
/// 末页再 forward / 首页再 back → 良性「无更多历史」success=true 不报错。一个测试覆盖全 history 链
/// （建一次 chrome 最省资源）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn history_back_forward_roundtrip_and_benign_edges() {
    let backend = common::build_backend_for_fixture("d4-history").await;
    let url_a = common::fixture_url("page-a.html");
    let url_b = common::fixture_url("page-b.html");
    let p = act_progress();

    // navigate A → B（建一段历史 [A, B]，当前在 B）。
    let nav_a = backend.navigate(&url_a, false).await.expect("navigate A");
    assert!(nav_a.final_url.contains("page-a"), "should be on A: {}", nav_a.final_url);
    let nav_b = backend.navigate(&url_b, false).await.expect("navigate B");
    assert!(nav_b.final_url.contains("page-b"), "should be on B: {}", nav_b.final_url);

    // ── 末页 forward → 良性「无更多历史」（在 B 是末页）──
    let fwd_edge = backend.act(&ActSpec::Forward, &p).await.expect("forward at end");
    eprintln!("=== forward-at-end === success={} changed={} msg={}", fwd_edge.success, fwd_edge.effect.changed, fwd_edge.message);
    assert!(fwd_edge.success, "forward at last page must be benign success (no more history)");
    assert!(!fwd_edge.effect.changed, "forward at end must not change page");

    // ── back → 回 A（settle 复用 D2，load_state 达可读稳态）──
    let back = backend.act(&ActSpec::Back, &p).await.expect("back to A");
    eprintln!("=== back === success={} changed={} msg={}", back.success, back.effect.changed, back.message);
    assert!(back.success && back.effect.changed, "back from B must change to A");
    // current url 应回到 A（用 get_page_text 验内容也行；这里查 active tab url 经 observe 的 url 字段）。
    let obs_a = backend.observe(&ObserveOpts::default()).await.expect("observe after back");
    eprintln!("after-back url = {:?}", obs_a.url);
    assert!(
        obs_a.url.as_deref().unwrap_or("").contains("page-a"),
        "after back, page must be A, got url={:?}",
        obs_a.url
    );
    // 内容也应是 A（PAGE_A_MARKER）。
    let text_a = backend.act(&ActSpec::GetPageText, &p).await.expect("get_page_text A");
    assert!(text_a.message.contains("PAGE_A_MARKER"), "back must show A content: {}", text_a.message);

    // ── forward → 回 B ──
    let fwd = backend.act(&ActSpec::Forward, &p).await.expect("forward to B");
    eprintln!("=== forward === success={} changed={} msg={}", fwd.success, fwd.effect.changed, fwd.message);
    assert!(fwd.success && fwd.effect.changed, "forward from A must change back to B");
    let obs_b = backend.observe(&ObserveOpts::default()).await.expect("observe after forward");
    eprintln!("after-forward url = {:?}", obs_b.url);
    assert!(
        obs_b.url.as_deref().unwrap_or("").contains("page-b"),
        "after forward, page must be B, got url={:?}",
        obs_b.url
    );

    // ── back 到首页边界 → 良性「无更多历史」success=true changed=false ──
    // 注：浏览器启动时已有一个 `about:blank` 初始历史 entry（launch + createTarget），故真实历史是
    // [about:blank, page-a, page-b]——page-a 并非 idx 0。逐步 back 直到撞到首页边界（changed=false），
    // 全程必须 success=true（每一格 back 都良性、绝不报错、绝不 panic），且边界态 changed=false。
    let mut hit_edge = false;
    for i in 0..6 {
        let b = backend.act(&ActSpec::Back, &p).await.expect("back step");
        eprintln!("=== back-step {i} === success={} changed={} msg={}", b.success, b.effect.changed, b.message);
        assert!(b.success, "every back must be benign success (step {i}): {}", b.message);
        if !b.effect.changed {
            hit_edge = true;
            break;
        }
    }
    assert!(
        hit_edge,
        "backing to the first history entry must eventually reach the benign 'no more history' edge (success=true, changed=false)"
    );
}

/// **reload 端到端**：navigate → reload → load_state 达可读稳态（Load/NetworkIdle），success=true。
/// reload 后页面仍是同一 URL（GET 页，非 POST，故 effect.after_anchor.irreversible == false）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn reload_get_page_succeeds_and_not_irreversible() {
    let backend = common::build_backend_for_fixture("d4-reload").await;
    let url = common::fixture_url("page-a.html");
    let p = act_progress();

    backend.navigate(&url, false).await.expect("navigate");
    let reload = backend.act(&ActSpec::Reload, &p).await.expect("reload");
    eprintln!("=== reload === success={} changed={} msg={}", reload.success, reload.effect.changed, reload.message);
    assert!(reload.success, "reload of a GET page must succeed");
    // GET 页 reload → 非 IRREVERSIBLE（after_anchor.irreversible == false）。
    let irreversible = reload
        .effect
        .after_anchor
        .as_ref()
        .and_then(|a| a.get("irreversible"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    assert!(!irreversible, "GET page reload must NOT be flagged irreversible: {:?}", reload.effect.after_anchor);
    // load_state 达可读稳态（静态页）。
    let ls = reload
        .effect
        .after_anchor
        .as_ref()
        .and_then(|a| a.get("load_state"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    eprintln!("reload load_state = {ls}");
    assert!(
        ls == "load" || ls == "networkidle" || ls == "domcontentloaded",
        "reload should reach a readable load state, got {ls}"
    );
    // 内容仍是 A。
    let text = backend.act(&ActSpec::GetPageText, &p).await.expect("get_page_text");
    assert!(text.message.contains("PAGE_A_MARKER"), "reload keeps page A: {}", text.message);
    let _ = LoadState::Load; // 用到 LoadState 导入（保持与 nav 测同风格）。
}

/// **switch_frame 端到端**：含 iframe 的 fixture → observe 看到 iframe ref → switch_frame 进 iframe →
/// get_page_text 读到 **iframe 内容**（IFRAME_INNER_MARKER）而非主帧（MAIN_DOC_MARKER）；switch_frame
/// "main" 切回主帧 → get_page_text 又读到主帧内容。验证 active_frame 指针影响页面级动作。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn switch_frame_scopes_page_text_to_iframe() {
    let backend = common::build_backend_for_fixture("d4-switchframe").await;
    let p = act_progress();
    backend
        .navigate(&common::fixture_url("switch-frame.html"), false)
        .await
        .expect("navigate switch-frame.html");

    // observe 填 ref 表 + 武装注入侧缓存（switch_frame 反查的前置）。
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== switch_frame entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?} frame_seq={}", e.r#ref, e.role, e.name, e.frame_seq);
    }

    // 主帧 get_page_text：应读到 MAIN_DOC_MARKER（默认作用主帧）。
    let main_text = backend.act(&ActSpec::GetPageText, &p).await.expect("get_page_text main");
    eprintln!("=== main-frame text (truncated) ===\n{}", &main_text.message.chars().take(300).collect::<String>());
    assert!(
        main_text.message.contains("MAIN_DOC_MARKER"),
        "before switch_frame, page text must be the MAIN doc: {}",
        main_text.message
    );

    // 取 iframe 元素 ref（observe 把 iframe 元素以 role=iframe 暴露）。
    let iframe_entry = obs
        .entries
        .iter()
        .find(|e| e.role == "iframe")
        .expect("fixture should expose an iframe element ref");
    eprintln!("iframe ref = {}", iframe_entry.r#ref);

    // switch_frame 进 iframe。
    let sw = backend
        .act(&ActSpec::SwitchFrame { r#ref: iframe_entry.r#ref.clone() }, &p)
        .await
        .expect("switch_frame into iframe");
    eprintln!("=== switch_frame === success={} msg={}", sw.success, sw.message);
    assert!(sw.success, "switch_frame into a real iframe must succeed");

    // 现在 get_page_text 应作用于 iframe 内容（IFRAME_INNER_MARKER），不再是主帧。
    let iframe_text = backend.act(&ActSpec::GetPageText, &p).await.expect("get_page_text iframe");
    eprintln!("=== iframe text (truncated) ===\n{}", &iframe_text.message.chars().take(300).collect::<String>());
    assert!(
        iframe_text.message.contains("IFRAME_INNER_MARKER"),
        "after switch_frame, page text must be the IFRAME content: {}",
        iframe_text.message
    );
    assert!(
        !iframe_text.message.contains("MAIN_DOC_MARKER"),
        "after switch_frame, page text must NOT include the main doc marker: {}",
        iframe_text.message
    );

    // switch_frame "main" 切回主帧 → get_page_text 又读到主帧内容。
    let back_main = backend
        .act(&ActSpec::SwitchFrame { r#ref: "main".into() }, &p)
        .await
        .expect("switch_frame back to main");
    assert!(back_main.success, "switch_frame back to main must succeed");
    let main_again = backend.act(&ActSpec::GetPageText, &p).await.expect("get_page_text main again");
    assert!(
        main_again.message.contains("MAIN_DOC_MARKER"),
        "after switching back to main, page text must be the MAIN doc again: {}",
        main_again.message
    );
}

/// **switch_frame 到非 iframe 元素 → 良性失败**（success=false，不报错）：取一个非 iframe 元素 ref
/// （主帧 button）switch_frame → success=false（引导换 ref），不 Err、不 panic。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn switch_frame_on_non_iframe_is_benign_failure() {
    let backend = common::build_backend_for_fixture("d4-switchframe-noniframe").await;
    let p = act_progress();
    backend
        .navigate(&common::fixture_url("switch-frame.html"), false)
        .await
        .expect("navigate");
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    let button = obs
        .entries
        .iter()
        .find(|e| e.role == "button")
        .expect("fixture should expose a button");
    let res = backend
        .act(&ActSpec::SwitchFrame { r#ref: button.r#ref.clone() }, &p)
        .await
        .expect("switch_frame on non-iframe must return Ok (benign), not Err");
    eprintln!("=== switch_frame-non-iframe === success={} msg={}", res.success, res.message);
    assert!(!res.success, "switch_frame on a non-iframe element must be a benign failure (success=false)");
}
