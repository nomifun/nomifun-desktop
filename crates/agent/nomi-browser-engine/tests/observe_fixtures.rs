//! observe 注入契约快照测试（`#[ignore]`，本机/打包 chrome）。
//!
//! 对一个**固定 HTML fixture**（`tests/fixtures/iframe.html`，file:// 加载）跑注入侧的
//! `incrementalAriaSnapshot`，把它返回的 `.full`（aria YAML，给 LLM 看的那一版）冻成 insta
//! 快照。目的：
//! - **供人审**：aria 输出形态（role/name/ref=f0e<n>）一眼可读、可在 review 里核对；
//! - **防漂移**：vendor 的 Playwright InjectedScript 升级后若 aria 序列化形态变了，快照 diff
//!   会立刻报出来（DESIGN：整包 vendor 不 fork，靠契约测试钉住外部行为）。
//!
//! 接线母本 = `src/injected.rs` 的 `inject_aria_snapshot_smoke`（八步：launch → connect →
//! run_attach_loop → enable_auto_attach → createTarget page → navigate(file://) → arm →
//! `Runtime.evaluate("document.body")` 取 objectId → `call_injected("incrementalAriaSnapshot",
//! [body, opts], by_value=true)` → 取 `.full`）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(observe_inject_contract)'
//! 首跑写 `.snap.new`；`cargo insta accept`（或手动改名 .snap）接受为基线。
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::page::{
    EnableParams as PageEnable, NavigateParams,
};
use chromiumoxide::cdp::browser_protocol::target::{CreateTargetParams, EventAttachedToTarget};
use chromiumoxide::cdp::js_protocol::runtime::{
    CallArgument, EvaluateParams, ExecutionContextId, RemoteObjectId,
};

use nomi_browser_engine::injected::InjectionManager;
use nomi_browser_engine::launch::{launch_chrome, LaunchConfig};
use nomi_browser_engine::transport::Connection;

mod common;

/// fixture 的 file:// URL（`file://` + 单前导斜杠 + POSIX 路径；unix `/abs` 已带斜杠、windows
/// `C:/abs` 补一个。旧 `file:///{manifest}` 在 unix 产四斜杠，被 chrome 归一后破坏 url 比较）。
fn fixture_url() -> String {
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    let abs = if manifest.starts_with('/') {
        manifest
    } else {
        format!("/{manifest}")
    };
    format!("file://{abs}/tests/fixtures/iframe.html")
}

#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn observe_inject_contract_iframe() {
    // 1) resolve chrome（env NOMIFUN_CHROME_BINARY > 打包 > 数据目录 > 下载兜底）+ launch headless。
    let chrome = nomi_browser_engine::acquire::resolve_chrome_path(
        &std::env::temp_dir().join("nomifun-browser-data"),
        None,
    )
    .await
    .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
    let cfg = LaunchConfig {
        chrome_path: chrome,
        user_data_dir: std::env::temp_dir().join("nomifun-observe-contract-profile"),
        headful: false,
    };
    let launched = launch_chrome(&cfg, true).await.expect("launch chrome");

    // 2) connect + 先 run_attach_loop 后 enable_auto_attach（顺序铁律，否则首批 attach 事件丢）。
    let _child = launched.child; // 保活 chrome（drop 即清理）。
    let conn = Connection::connect_launched(launched.transport)
        .await
        .expect("connect");
    let _attach_loop = conn.run_attach_loop();
    conn.enable_auto_attach().await.expect("auto attach");

    // 3) 取一个 page session（createTarget about:blank + 等其 attachedToTarget）。
    let mut attached = conn.subscribe(EventAttachedToTarget::IDENTIFIER, None);
    let create = CreateTargetParams::new("about:blank");
    let cr = conn
        .send::<CreateTargetParams>(nomi_browser_engine::transport::ROOT_SESSION, &create)
        .await
        .expect("createTarget");
    let target_id = cr["targetId"].as_str().expect("targetId").to_string();
    let page_session = loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), attached.recv())
            .await
            .expect("attach timeout")
            .expect("attach recv");
        if let Ok(att) = serde_json::from_value::<EventAttachedToTarget>(ev.params.clone()) {
            let tid: String = att.target_info.target_id.clone().into();
            if tid == target_id && att.target_info.r#type == "page" {
                break String::from(att.session_id);
            }
        }
    };

    // 4) navigate 到固定 fixture（file://，含 h1/button/textbox/iframe，aria 形态稳定）。
    conn.send::<PageEnable>(&page_session, &PageEnable::default())
        .await
        .expect("Page.enable");
    let mut load_rx = conn.subscribe("Page.loadEventFired", Some(&page_session));
    conn.send::<NavigateParams>(&page_session, &NavigateParams::new(fixture_url()))
        .await
        .expect("navigate");
    let _ = tokio::time::timeout(Duration::from_secs(15), load_rx.recv()).await;

    // 5) arm 注入管线 + 等主 frame 的 utility-world context 就绪（主 frameId = page targetId）。
    let mgr = InjectionManager::new(conn.clone(), page_session.clone());
    let _ctx_loop = mgr.arm().await.expect("arm injection");
    let frame_id = target_id.clone();
    let mut ctx_id = None;
    for _ in 0..50 {
        if let Ok(id) = mgr.context_id_for(&frame_id) {
            ctx_id = Some(id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ctx_id = ctx_id.expect("utility world context never registered for main frame");

    // 6) 取 document.body 的 objectId（同 utility world 的元素句柄），作 incrementalAriaSnapshot 的 node。
    let mut body_eval = EvaluateParams::new("document.body".to_string());
    body_eval.context_id = Some(ExecutionContextId::new(ctx_id));
    body_eval.return_by_value = Some(false);
    let body_res = conn
        .send::<EvaluateParams>(&page_session, &body_eval)
        .await
        .expect("evaluate document.body");
    let body_obj_id = body_res["result"]["objectId"]
        .as_str()
        .expect("document.body objectId")
        .to_string();

    // 7) call_injected incrementalAriaSnapshot(body, {mode:ai, refPrefix:f0, depth:12})；node 走
    //    objectId，opts 走 by-value；返回 by-value（result.value 是 {full, iframeRefs, ...}）。
    let node_arg = CallArgument {
        object_id: Some(RemoteObjectId::new(body_obj_id)),
        ..Default::default()
    };
    let opts_arg = CallArgument {
        value: Some(serde_json::json!({"mode": "ai", "refPrefix": "f0", "depth": 12})),
        ..Default::default()
    };
    let result = mgr
        .call_injected(
            &frame_id,
            "incrementalAriaSnapshot",
            vec![node_arg, opts_arg],
            true,
        )
        .await
        .expect("call_injected incrementalAriaSnapshot");

    // 8) 取 .full（aria YAML 字符串）并快照。aria YAML 只含页面结构（generic/button/textbox/iframe
    //    + ref=f0e<n>），不含随机 world 名，故无需归一。
    let full = result["value"]["full"]
        .as_str()
        .expect("incrementalAriaSnapshot result.value.full is a string")
        .to_string();
    assert!(!full.trim().is_empty(), "aria .full must be non-empty");

    insta::assert_snapshot!(full);
}

// ═══════════════════════════════════════════════════════════════════════════
// 任务 6：CdpBackend::observe 全链集成测试（`#[ignore]`，本机/打包 chrome）。
// 复用 tests/common 的 build_backend_for_fixture helper（勿再复制契约母本）。
// 被测对象是 engine.observe()：逐帧 incrementalAriaSnapshot → 缝合 → 脱敏 → 代际翻新 ref 表。
// ═══════════════════════════════════════════════════════════════════════════

use nomi_browser_engine::{BrowserEngine, ObserveOpts};

/// 同进程 iframe 缝合：navigate iframe.html（srcdoc iframe 是同进程，应能缝）→ observe →
/// 快照 yaml + 断言含父帧 ref f0e*；若同进程子帧缝上则含子内容（缩进的 Inner 链接）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn observe_iframe_stitched() {
    let engine = common::build_backend_for_fixture("iframe").await;
    engine
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate iframe.html");
    let obs = engine
        .observe(&ObserveOpts::default())
        .await
        .expect("observe");

    eprintln!("=== observe_iframe_stitched yaml ===\n{}\n=== end ===", obs.yaml);
    // 父帧 ref（f0e*）必现。
    assert!(obs.yaml.contains("f0e"), "expected parent-frame ref f0e*:\n{}", obs.yaml);
    // <data> 包裹。
    assert!(obs.yaml.contains("<data"), "expected <data> wrap:\n{}", obs.yaml);
    // 同进程 srcdoc 子帧应缝入（含其链接文本 Inner）；srcdoc iframe 是同进程，应能缝。
    //
    // 快照里的 origin 是本机绝对 file:// 路径——快照前归一成稳定占位符，使快照可移植
    // （不钉死 checkout 路径），同时仍审 <data origin=…> 包裹 + 缝合结构。
    let normalized = normalize_fixture_origin(&obs.yaml, "iframe.html");
    insta::assert_snapshot!("observe_iframe_stitched", normalized);
}

/// 把 yaml 里本机绝对 `origin="file:///.../tests/fixtures/<name>"` 归一成
/// `origin="file://<FIXTURE>/<name>"`，让 insta 快照不钉死 checkout 路径。
fn normalize_fixture_origin(yaml: &str, name: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    // Chrome reports file:// origins with exactly ONE leading slash before the
    // POSIX path (unix: `file:///abs`; windows: `file:///C:/abs`). But
    // CARGO_MANIFEST_DIR is `/abs` on unix and `C:/abs` on windows — so prepend a
    // slash only when it doesn't already start with one. The old unconditional
    // `file:///{manifest}` produced FOUR slashes on unix (`file:////Users/...`),
    // never matched the reported origin, and leaked the absolute checkout path
    // into the snapshot. This makes the placeholder fire on every host.
    let abs = if manifest.starts_with('/') {
        manifest
    } else {
        format!("/{manifest}")
    };
    let actual = format!("file://{abs}/tests/fixtures/{name}");
    let placeholder = format!("file://<FIXTURE>/{name}");
    yaml.replace(&actual, &placeholder)
}

/// open shadow 可见、closed shadow 不可见（D11）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn observe_shadow_open_visible_closed_absent() {
    let engine = common::build_backend_for_fixture("shadow").await;
    engine
        .navigate(&common::fixture_url("shadow.html"), false)
        .await
        .expect("navigate shadow.html");
    let obs = engine
        .observe(&ObserveOpts::default())
        .await
        .expect("observe");

    eprintln!("=== observe_shadow yaml ===\n{}\n=== end ===", obs.yaml);
    assert!(
        obs.yaml.contains("OpenShadowBtn"),
        "open shadow content must be visible:\n{}",
        obs.yaml
    );
    assert!(
        !obs.yaml.contains("ClosedShadowBtn"),
        "closed shadow content must NOT be visible:\n{}",
        obs.yaml
    );
}

/// 代际单调递增（无导航两次 observe）+ 导航后旧 ref 在新表 resolve 不到（generation 跳变隔离）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn observe_generation_increments_and_renews() {
    let engine = common::build_backend_for_fixture("gen").await;
    engine
        .navigate(&common::fixture_url("shadow.html"), false)
        .await
        .expect("navigate");
    let obs1 = engine.observe(&ObserveOpts::default()).await.expect("observe1");
    let obs2 = engine.observe(&ObserveOpts::default()).await.expect("observe2");
    // 无导航两次 observe：代际单调 +1。
    assert_eq!(
        obs2.generation.0,
        obs1.generation.0 + 1,
        "generation must increment per observe: {:?} -> {:?}",
        obs1.generation,
        obs2.generation
    );
    // 导航到不同页再 observe：代际继续递增（旧代际 ref 不再属于新表）。
    engine
        .navigate(&common::fixture_url("secrets.html"), false)
        .await
        .expect("navigate2");
    let obs3 = engine.observe(&ObserveOpts::default()).await.expect("observe3");
    assert!(
        obs3.generation.0 > obs2.generation.0,
        "generation must advance after navigation: {:?} -> {:?}",
        obs2.generation,
        obs3.generation
    );
    // 每代 entries 应非空（页面有可操作元素：button 等）。
    assert!(!obs1.entries.is_empty(), "obs1 entries should be non-empty");
    assert!(!obs3.entries.is_empty(), "obs3 entries should be non-empty");
}

/// 脱敏：secrets.html 的明文 sk-/Bearer token 不出现在 yaml，且整体被 <data> 包裹。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn observe_redacts_secrets() {
    let engine = common::build_backend_for_fixture("secrets").await;
    engine
        .navigate(&common::fixture_url("secrets.html"), false)
        .await
        .expect("navigate secrets.html");
    let obs = engine
        .observe(&ObserveOpts::default())
        .await
        .expect("observe");

    eprintln!("=== observe_redacts_secrets yaml ===\n{}\n=== end ===", obs.yaml);
    assert!(obs.yaml.contains("<data"), "expected <data> wrap:\n{}", obs.yaml);
    // 明文 secret 实体不得出现。
    assert!(
        !obs.yaml.contains("sk-ABCDEFGHIJ0123456789xyzQRSTUV"),
        "plaintext sk- token leaked:\n{}",
        obs.yaml
    );
    assert!(
        !obs.yaml.contains("abcdef0123456789ABCDEFghij"),
        "plaintext Bearer token leaked:\n{}",
        obs.yaml
    );
    // D5（Critical）：password <input value="hun]ter2sk"> 的明文 value 不得进 YAML。
    // 序列化层（observe）按 DOM type=password 收 ref 后宿主侧抹掉内联 value（短/低熵口令
    // 正则脱敏救不了，必须靠 type=password 信号置空）。**value 含 `]`** 压测确定性锚点 bug：
    // 旧 blank_inline_value 用全行 rfind(']') → 锚点落进 value 内部漏抹 → 明文泄露；改 ref-token
    // 锚后必不泄露。
    assert!(
        !obs.yaml.contains("hun]ter2"),
        "password value (with ]) leaked into observe yaml:\n{}",
        obs.yaml
    );
    // 脱敏占位符应在（确认确实脱敏，而非该文本压根没进 aria 树）。
    assert!(
        obs.yaml.contains("[REDACTED_SECRET]"),
        "expected redaction placeholder (text must have entered the aria tree):\n{}",
        obs.yaml
    );
}

