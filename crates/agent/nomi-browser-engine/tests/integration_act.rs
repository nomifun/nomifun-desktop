//! **P2 命脉：act 反查链端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 `f<seq>e<n>` ref → DOM element objectId 的反查 + objectGroup 生命周期 + role 二次校验：
//! navigate fixture（含 button "Submit order"）→ `observe()`（填 ref 表 + 武装注入侧的
//! `_lastAriaSnapshotForQuery.elements` 缓存）→ 从 entries 取按钮 ref → `resolve_ref` 拿回非空
//! objectId（层②，vendored aria-ref selector engine）+ role 比对通过（层③）→ 不存在/过期 ref
//! → NodeStale（层①）→ `release_act_group_by_ref` 释放后不报错。
//!
//! 复用 `tests/common` 的 `build_backend_for_fixture`（勿再复制契约母本）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(resolve_ref)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

use nomi_browser_engine::actionability::CheckResult;
use nomi_browser_engine::input::Point;
use nomi_browser_engine::{BrowserError, BrowserEngine, ObserveOpts};

mod common;
/// 端到端：observe → resolve_ref（层②③）成功拿 objectId + role 校验通过；stale ref → 层① NodeStale；
/// release 后不报错。一个测试覆盖全反查链（建一次 chrome，最省资源）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn resolve_ref_roundtrip_and_stale_and_release() {
    let backend = common::build_backend_for_fixture("act").await;
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate iframe.html");

    // observe 填 ref 表 + 武装注入侧 elements 缓存（act 反查的前置）。
    let obs = backend
        .observe(&ObserveOpts::default())
        .await
        .expect("observe");
    eprintln!("=== resolve_ref entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?} frame_seq={}", e.r#ref, e.role, e.name, e.frame_seq);
    }

    // 取按钮 "Submit order" 的 ref（fixture 固定有此 button）。
    let button = obs
        .entries
        .iter()
        .find(|e| e.role == "button" && e.name == "Submit order")
        .expect("fixture should expose a button \"Submit order\"");
    let button_ref = button.r#ref.clone();
    eprintln!("button ref = {button_ref}");

    // ── 反查成功：拿回非空 objectId（层②）+ group 命名（objectGroup）+ role 校验通过（层③）──
    let handle = backend
        .resolve_ref(&button_ref, 0)
        .await
        .expect("resolve_ref should resolve a live button");
    eprintln!("resolved objectId = {} group = {}", handle.object_id, handle.group);
    assert!(!handle.object_id.is_empty(), "object_id must be non-empty");
    assert_eq!(handle.group, "act-0", "objectGroup must be act-<seq>");

    // ── 不存在 ref → 层① NodeStale（不进浏览器，table resolve 即拦下）──
    let stale = backend.resolve_ref("f9e99999", 1).await;
    eprintln!("stale lookup = {stale:?}");
    assert!(
        matches!(stale, Err(BrowserError::NodeStale { .. })),
        "non-existent ref must be NodeStale, got {stale:?}"
    );

    // ── release：释放刚才 act-0 组的句柄，不报错（幂等）。再释放一次（已空组）仍不报错。──
    backend.release_act_group_by_ref(&button_ref, 0).await;
    backend.release_act_group_by_ref(&button_ref, 0).await;

    // release 后该 group 的 objectId 已失效：对失效句柄再 call 会被 CDP 报错（"Could not find object"），
    // 但这通过 InjectError → BrowserError 体现，不 panic。这里只断言 resolve 仍能拿**新**句柄（注入侧
    // elements 缓存未失效，只是宿主释放了上次的 RemoteObject 引用）。
    let handle2 = backend
        .resolve_ref(&button_ref, 2)
        .await
        .expect("resolve_ref after release should still resolve (cache intact, fresh handle)");
    assert!(!handle2.object_id.is_empty());
    assert_eq!(handle2.group, "act-2");
    backend.release_act_group_by_ref(&button_ref, 2).await;
}

/// role 漂移校验（层③）：手动构造一个 RefRecord，其 role 与活元素实际 role 不符 → NodeStale。
/// 用 `resolve_ref_to_object`（吃 RefRecord）直接喂入伪造 role，验证层③ 真的拦截（而非只过层②）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn resolve_ref_role_mismatch_is_stale() {
    use nomi_browser_engine::aria_ref::RefRecord;

    let backend = common::build_backend_for_fixture("act-role").await;
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate");
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    let button = obs
        .entries
        .iter()
        .find(|e| e.role == "button" && e.name == "Submit order")
        .expect("button");

    // 用按钮的真 ref，但 RefRecord 谎称 role=link（模拟 backendNodeId 复用后 ref 映射到异角色元素）。
    // session_id/frame_id 取主 page session（按钮在主帧）。D1：page_session_id/main_frame_id 现 async。
    let forged = RefRecord {
        session_id: backend.page_session_id().await.expect("active tab session"),
        frame_id: backend.main_frame_id().await.expect("active tab main frame"),
        full_ref: button.r#ref.clone(),
        role: "link".into(), // ← 谎报：实际是 button
        name: button.name.clone(),
    };
    let res = backend.resolve_ref_to_object(&forged, 7).await;
    eprintln!("role-mismatch resolve = {res:?}");
    assert!(
        matches!(res, Err(BrowserError::NodeStale { .. })),
        "role mismatch must be NodeStale (layer③), got {res:?}"
    );
}

/// **B3 actionability 五检查 `check_states`（visible/stable/enabled/editable）三态 + editable 两路径**
/// 端到端（一个测试覆盖全状态，建一次 chrome 最省资源）。
///
/// fixture `actionability.html` 含 normal button / disabled button / readonly input / 普通 textbox /
/// heading（非可编辑）。navigate → observe 拿各元素 ref → `resolve_ref` 拿 ObjectHandle →
/// `check_states`：
/// - normal button `["visible","stable","enabled"]` → `Pass`；
/// - disabled button `["visible","stable","enabled"]` → `Missing("enabled")`；
/// - readonly input `["editable"]` → `Missing("editable")`（**可重试**，区别于不可编辑特例）；
/// - heading `["editable"]` → `Err(Blocked)`（元素类型根本不支持编辑，NonRecoverable，**禁重试**）；
/// - 普通 textbox observe 后用 `__eval_page_world_for_test` 改 `display:none`，再 `check_states(["visible"])`
///   → `Missing("visible")`（ref 已分配但现已不可见）。
///
/// insta 锚 `CheckResult` 的 Debug 快照（三态判定的稳定契约）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn check_states_three_states_and_editable_two_paths() {
    let backend = common::build_backend_for_fixture("act-check-states").await;
    backend
        .navigate(&common::fixture_url("actionability.html"), false)
        .await
        .expect("navigate actionability.html");

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== check_states entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }

    let find = |role: &str, name: &str| -> String {
        obs.entries
            .iter()
            .find(|e| e.role == role && e.name == name)
            .unwrap_or_else(|| panic!("fixture should expose {role:?} {name:?}"))
            .r#ref
            .clone()
    };

    // ── normal button → Pass ──────────────────────────────────────────────
    let normal_ref = find("button", "Submit order");
    let normal = backend.resolve_ref(&normal_ref, 0).await.expect("resolve normal");
    let normal_res = backend
        .check_states(&normal, &["visible", "stable", "enabled"])
        .await
        .expect("check normal");
    eprintln!("normal button check = {normal_res:?}");

    // ── disabled button → Missing("enabled") ──────────────────────────────
    let disabled_ref = find("button", "Disabled action");
    let disabled = backend.resolve_ref(&disabled_ref, 1).await.expect("resolve disabled");
    let disabled_res = backend
        .check_states(&disabled, &["visible", "stable", "enabled"])
        .await
        .expect("check disabled");
    eprintln!("disabled button check = {disabled_res:?}");

    // ── readonly input → Missing("editable")（可重试，区别于不可编辑特例的 Blocked）──
    let readonly_ref = find("textbox", "Locked");
    let readonly = backend.resolve_ref(&readonly_ref, 2).await.expect("resolve readonly");
    let readonly_res = backend
        .check_states(&readonly, &["editable"])
        .await
        .expect("check readonly");
    eprintln!("readonly input editable check = {readonly_res:?}");

    // ── heading（非可编辑元素）editable → Err(Blocked)（NonRecoverable，禁重试）──
    let heading_ref = find("heading", "Actionability");
    let heading = backend.resolve_ref(&heading_ref, 3).await.expect("resolve heading");
    let heading_res = backend.check_states(&heading, &["editable"]).await;
    eprintln!("heading editable check = {heading_res:?}");
    let heading_blocked = matches!(heading_res, Err(BrowserError::Blocked { .. }));

    // ── 普通 textbox：observe 后改为 display:none → visible 检查 Missing("visible") ──
    let email_ref = find("textbox", "Email");
    let email = backend.resolve_ref(&email_ref, 4).await.expect("resolve email");
    // 在页面 world 把 Email 输入框隐藏（ref 已分配，但 checkElementStates 现场重读其可见性）。
    backend
        .__eval_page_world_for_test(
            "(() => { \
              const label = [...document.querySelectorAll('label')].find(l => l.textContent.includes('Email')); \
              const input = label && label.querySelector('input'); \
              if (input) input.style.display = 'none'; \
              return !!input; })()",
        )
        .await
        .expect("hide email input");
    let hidden_res = backend
        .check_states(&email, &["visible"])
        .await
        .expect("check hidden");
    eprintln!("hidden input visible check = {hidden_res:?}");

    // 释放各动作组（best-effort）。
    for (r, s) in [
        (&normal_ref, 0),
        (&disabled_ref, 1),
        (&readonly_ref, 2),
        (&heading_ref, 3),
        (&email_ref, 4),
    ] {
        backend.release_act_group_by_ref(r, s).await;
    }

    // 直接断言关键三态 + 两路径（不只靠快照，便于失败时定位）。
    assert_eq!(normal_res, CheckResult::Pass, "normal button must Pass");
    assert_eq!(
        disabled_res,
        CheckResult::Missing("enabled".into()),
        "disabled button must Missing(enabled)"
    );
    assert_eq!(
        readonly_res,
        CheckResult::Missing("editable".into()),
        "readonly input must Missing(editable) (recoverable, NOT Blocked)"
    );
    assert!(
        heading_blocked,
        "non-editable heading must be Blocked (NonRecoverable), got {heading_res:?}"
    );
    assert_eq!(
        hidden_res,
        CheckResult::Missing("visible".into()),
        "hidden input must Missing(visible)"
    );

    // insta 契约：锚三态 + editable 两路径的稳定判定。
    let summary = format!(
        "normal(button,visible+stable+enabled)   = {normal_res:?}\n\
         disabled(button,visible+stable+enabled)  = {disabled_res:?}\n\
         readonly(input,editable)                 = {readonly_res:?}\n\
         non_editable(heading,editable)           = Err(Blocked)={heading_blocked}\n\
         hidden(input,visible)                    = {hidden_res:?}"
    );
    insta::assert_snapshot!("check_states_contract", summary);
}

/// **B4 double hit-target 三步舞（actionability 第⑤检查 receivesEvents）** 端到端（一个测试覆盖
/// 正常→Ok / 遮挡→Blocked，建一次 chrome 最省资源）。
///
/// fixture `modal-overlay.html`：`#reachable`（modal 内，z-index 高于遮罩，点它真命中自己）+
/// `#covered`（页面背景，被全屏 `#overlay` 遮罩盖住，点它命中 overlay）。两按钮 fixed 定位到确定
/// 坐标，命中点硬编码（不依赖 quad 计算，那是 B5）。navigate → observe 拿按钮 ref → `resolve_ref`
/// 拿 ObjectHandle，然后在硬编码命中点上：
/// - **可达按钮**（(120,64) 在 #reachable 内）：`expect_hit_target` → `Ok`（未被遮挡）；
///   `hit_setup`（mouse, block_all=false）→ `Ok(HitInterceptor)`（预检通过，拿到 interceptor 句柄）；
///   `hit_stop`（B4 无真点击，无事件触发）→ `Ok`（stop 内 `result || 'done'` 保守放行）；
/// - **被遮挡按钮**（(120,224) 在 #covered 内，但被 overlay 盖住）：`expect_hit_target` →
///   `Err(Blocked)`（hitTargetDescription 指向 overlay）；`hit_setup` 预检短路 → `Err(Blocked)`
///   （reason 指向 overlay，无需走点击）。
///
/// insta 锚 `expectHitTarget`/`hit_setup` 的判定 + 遮挡按钮的 `hitTargetDescription`（reason 原文，
/// 即 `expectHitTarget`/setup 短路返回的 JSON 的 load-bearing 部分）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn hit_target_setup_stop_and_expect_occlusion() {
    let backend = common::build_backend_for_fixture("act-hit-target").await;
    backend
        .navigate(&common::fixture_url("modal-overlay.html"), false)
        .await
        .expect("navigate modal-overlay.html");

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== hit_target entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }

    let find = |role: &str, name: &str| -> String {
        obs.entries
            .iter()
            .find(|e| e.role == role && e.name == name)
            .unwrap_or_else(|| panic!("fixture should expose {role:?} {name:?}"))
            .r#ref
            .clone()
    };

    // 命中点：与 fixture 的 fixed 定位一致（中心点；不走 quad 计算，那是 B5）。
    let reachable_point = Point { x: 120.0, y: 64.0 }; // #reachable left:40 top:40 w:160 h:48
    let covered_point = Point { x: 120.0, y: 224.0 }; // #covered  left:40 top:200 w:160 h:48

    // ── 可达按钮（modal 内，未被遮挡）─────────────────────────────────────────
    let reachable_ref = find("button", "Reachable in modal");
    let reachable = backend.resolve_ref(&reachable_ref, 0).await.expect("resolve reachable");

    // expect_hit_target：未被遮挡 → Ok。
    let reachable_expect = backend.expect_hit_target(&reachable, reachable_point).await;
    eprintln!("reachable expect_hit_target = {reachable_expect:?}");

    // hit_setup（mouse, block_all=false）：预检通过 → 拿到 interceptor 句柄（by_value=false 保活）。
    let reachable_setup = backend
        .hit_setup(&reachable, "mouse", reachable_point, false)
        .await
        .expect("hit_setup on reachable button should装上拦截器");
    eprintln!(
        "reachable hit_setup interceptor objectId = {} group = {}",
        reachable_setup.object_id, reachable_setup.group
    );
    assert!(!reachable_setup.object_id.is_empty(), "interceptor objectId 非空");
    assert_eq!(reachable_setup.group, "act-0", "interceptor 句柄归元素同组 act-0");

    // hit_stop（B4 无真点击，无事件触发）→ stop 内 `result || 'done'` 保守放行 → Ok。
    let reachable_stop = backend.hit_stop(&reachable_setup).await;
    eprintln!("reachable hit_stop (no click) = {reachable_stop:?}");

    // ── 被遮挡按钮（页面背景，被全屏 overlay 盖住）──────────────────────────────
    let covered_ref = find("button", "Behind overlay");
    let covered = backend.resolve_ref(&covered_ref, 1).await.expect("resolve covered");

    // expect_hit_target：命中 overlay → Err(Blocked)，reason 指向 overlay。
    let covered_expect = backend.expect_hit_target(&covered, covered_point).await;
    eprintln!("covered expect_hit_target = {covered_expect:?}");

    // hit_setup 预检短路：已被遮挡 → Err(Blocked)（reason 指向 overlay，无需走点击）。
    let covered_setup = backend.hit_setup(&covered, "mouse", covered_point, false).await;
    eprintln!("covered hit_setup (preliminary short-circuit) = {covered_setup:?}");

    // 释放各动作组（best-effort）。
    backend.release_act_group_by_ref(&reachable_ref, 0).await;
    backend.release_act_group_by_ref(&covered_ref, 1).await;

    // ── 直接断言（便于失败时定位）────────────────────────────────────────────
    assert!(
        reachable_expect.is_ok(),
        "reachable expect_hit_target must Ok, got {reachable_expect:?}"
    );
    assert!(
        reachable_stop.is_ok(),
        "reachable hit_stop (no click) must Ok, got {reachable_stop:?}"
    );
    let covered_expect_reason = match &covered_expect {
        Err(BrowserError::Blocked { reason }) => reason.clone(),
        other => panic!("covered expect_hit_target must Blocked, got {other:?}"),
    };
    assert!(
        covered_expect_reason.contains("overlay"),
        "covered expect_hit_target Blocked reason should指向 overlay, got {covered_expect_reason:?}"
    );
    let covered_setup_reason = match &covered_setup {
        Err(BrowserError::Blocked { reason }) => reason.clone(),
        other => panic!("covered hit_setup must Blocked (preliminary short-circuit), got {other:?}"),
    };
    assert!(
        covered_setup_reason.contains("overlay"),
        "covered hit_setup Blocked reason should指向 overlay, got {covered_setup_reason:?}"
    );

    // insta 契约：锚三原语判定 + 遮挡 hitTargetDescription（reason 即 expectHitTarget/setup 短路
    // JSON 的 load-bearing 部分）。可达侧只锚 Ok（objectId 是运行时随机，不入快照）。
    let summary = format!(
        "reachable.expect_hit_target((120,64))    = {}\n\
         reachable.hit_setup(mouse,block=false)   = Ok(interceptor handle, group=act-0)\n\
         reachable.hit_stop(no click)             = {}\n\
         covered.expect_hit_target((120,224))     = Err(Blocked: {covered_expect_reason})\n\
         covered.hit_setup(preliminary)           = Err(Blocked: {covered_setup_reason})",
        if reachable_expect.is_ok() { "Ok" } else { "ERR" },
        if reachable_stop.is_ok() { "Ok" } else { "ERR" },
    );
    insta::assert_snapshot!("hit_target_contract", summary);
}

/// **B5 输入合成端到端**（getContentQuads 几何取点 → dispatchMouseEvent / insertText /
/// dispatchKeyEvent，全程禁 DPR）。一个测试覆盖：点击聚焦 + 文本键入读回 + 按钮点击态变 + 组合键，
/// 建一次 chrome 最省资源。
///
/// fixture `input-synth.html`：`#field`（text input，包在 label "Name"）+ `#toggle`（button，点击改
/// 自身文案 "Not clicked" → "Clicked"）。navigate → observe 拿 input/button ref → `resolve_ref`
/// 拿 ObjectHandle，然后：
/// - **input 聚焦 + 键入**：`element_content_quads`（真实 getContentQuads，CSS 像素）→
///   `viewport_size` → `pick_click_point`（B1 几何）→ `click_at`（dispatchMouseEvent，零 DPR）聚焦
///   → `type_text("hello")`（insertText）→ 读回 `#field.value == "hello"`；
/// - **按钮点击**：同样几何取点 → `click_at` → 读回 `#toggle.textContent == "Clicked"`（验证点击真
///   分发到元素，几何点准）；
/// - **组合键**：再次点 input 聚焦 → `type_text("xyz")` → `key_combo("Ctrl+A")` 全选 → `type_text("Z")`
///   覆盖选区 → 读回 `#field.value == "Z"`（验证 Ctrl+A 真触发选区，否则会是 "xyzZ"）。
///
/// 本机 Windows 真跑：
///   set NOMIFUN_CHROME_BINARY=...\chrome.exe
///   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(input_synth)'
/// 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn input_synth_click_focus_type_and_key_combo() {
    let backend = common::build_backend_for_fixture("act-input-synth").await;
    backend
        .navigate(&common::fixture_url("input-synth.html"), false)
        .await
        .expect("navigate input-synth.html");

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== input_synth entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }

    let find = |role: &str, name: &str| -> String {
        obs.entries
            .iter()
            .find(|e| e.role == role && e.name == name)
            .unwrap_or_else(|| panic!("fixture should expose {role:?} {name:?}"))
            .r#ref
            .clone()
    };

    // 几何取点：element_content_quads（真实 getContentQuads，CSS 像素）→ viewport_size →
    // pick_click_point（B1）→ Point。零 DPR：坐标原样从 quad 流到 dispatchMouseEvent。
    async fn click_point_for(
        backend: &nomi_browser_engine::backend::CdpBackend,
        h: &nomi_browser_engine::actionability::ObjectHandle,
    ) -> Point {
        let quads = backend.element_content_quads(h).await.expect("getContentQuads");
        eprintln!("  quads = {quads:?}");
        let (vw, vh) = backend.viewport_size().await.expect("viewport_size");
        eprintln!("  viewport = ({vw}, {vh})");
        nomi_browser_engine::input::pick_click_point(&quads, vw, vh)
            .expect("pick_click_point should find an in-viewport center")
    }

    // 读 #field.value（页面 world）。
    let read_field_value = || async {
        backend
            .__eval_page_world_for_test("document.getElementById('field').value")
            .await
            .expect("read field value")
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    // 读 #toggle.textContent（页面 world）。
    let read_toggle_text = || async {
        backend
            .__eval_page_world_for_test("document.getElementById('toggle').textContent")
            .await
            .expect("read toggle text")
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };

    // ── input 聚焦 + 文本键入 ────────────────────────────────────────────────
    let field_ref = find("textbox", "Name");
    let field = backend.resolve_ref(&field_ref, 0).await.expect("resolve field");
    let field_point = click_point_for(&backend, &field).await;
    eprintln!("field click point = {field_point:?}");
    backend.click_at(field_point).await.expect("click field (focus)");
    backend.type_text("hello").await.expect("insert_text hello");
    let field_value = read_field_value().await;
    eprintln!("field value after type = {field_value:?}");

    // ── 按钮点击：态变 ───────────────────────────────────────────────────────
    let toggle_ref = find("button", "Not clicked");
    let toggle = backend.resolve_ref(&toggle_ref, 1).await.expect("resolve toggle");
    let toggle_point = click_point_for(&backend, &toggle).await;
    eprintln!("toggle click point = {toggle_point:?}");
    let toggle_before = read_toggle_text().await;
    backend.click_at(toggle_point).await.expect("click toggle");
    let toggle_after = read_toggle_text().await;
    eprintln!("toggle text: before={toggle_before:?} after={toggle_after:?}");

    // ── 组合键 Ctrl+A 全选 → 覆盖 ────────────────────────────────────────────
    // 重新 observe（按钮文案变了，旧代际 ref 仍可能有效，但稳妥起见重定位 field）。重新点 field 聚焦。
    let field2 = backend.resolve_ref(&field_ref, 2).await.expect("resolve field again");
    let field_point2 = click_point_for(&backend, &field2).await;
    backend.click_at(field_point2).await.expect("re-focus field");
    // 现 field 已有 "hello"；再追加 "xyz" → "helloxyz"。
    backend.type_text("xyz").await.expect("insert xyz");
    let before_combo = read_field_value().await;
    eprintln!("field before Ctrl+A = {before_combo:?}");
    // 全选 → 输入 "Z" 覆盖整段 → value == "Z"。用 `ControlOrMeta+A`（跨平台加速键：mac=Meta、其它=Ctrl）；
    // mac 经 macEditingCommands 补 `commands=["selectAll"]` 才真全选（裸 Meta/Ctrl+A 在 headless 不触发编辑
    // 命令,且裸 Ctrl+A 在 mac headless 会挂起后续 CDP）。若全选没触发,会是 before_combo + "Z"。
    backend.key_combo("ControlOrMeta+A").await.expect("key_combo ControlOrMeta+A");
    backend.type_text("Z").await.expect("insert Z (replace selection)");
    let after_combo = read_field_value().await;
    eprintln!("field after ControlOrMeta+A + 'Z' = {after_combo:?}");

    // 释放各动作组（best-effort）。
    backend.release_act_group_by_ref(&field_ref, 0).await;
    backend.release_act_group_by_ref(&toggle_ref, 1).await;
    backend.release_act_group_by_ref(&field_ref, 2).await;

    // ── 直接断言（便于失败时定位）────────────────────────────────────────────
    assert_eq!(field_value, "hello", "insert_text should write 'hello' into focused input");
    assert_eq!(toggle_before, "Not clicked", "toggle should start 'Not clicked'");
    assert_eq!(
        toggle_after, "Clicked",
        "dispatch_click should真分发到按钮 (geometry point准), toggling text to 'Clicked'"
    );
    assert_eq!(
        after_combo, "Z",
        "ControlOrMeta+A 应全选(mac 经 macEditingCommands selectAll),其后 'Z' 覆盖整段 (was {before_combo:?})"
    );
}

/// **B6 detach 事件源 → progress.abort → act 立即返**（端到端，建一次 chrome）。
///
/// navigate fixture → 建一个**大 deadline**（3600s）的 [`Progress`] → `arm_act_abort` 派生子
/// Progress + 装临时 detach/crash 监听 → 在子 Progress 上并发跑一个**永挂** `run_act_with_retry`
/// op（永不成功、永不出错；唯一能结束它的就是 progress.abort）。另一处用
/// `__close_page_target_for_test()` 关掉 page target（模拟「用户关标签页」），CDP 随之发
/// `Target.detachedFromTarget`（sessionId == 本 page session）→ 监听任务命中 →
/// `progress.abort(PageClosed)` → 进行中的 `run_act_with_retry`（在 `progress.race` 上）**立即**
/// 以 `BrowserError::TargetClosed` 返回。
///
/// **关键断言**：① act 返 `TargetClosed`（abort→errmap 链通）；② 返回耗时**远早于** 3600s deadline
/// （这里用 `tokio::time::timeout(10s)` 兜底——若没接线，op 会永挂到 10s 兜底超时而非秒级返回）。
///
/// 本机 Windows 真跑：
///   set NOMIFUN_CHROME_BINARY=...\chrome.exe
///   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(detach_aborts)'
/// 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn detach_aborts_in_flight_act_far_before_timeout() {
    use nomi_browser_engine::progress::Progress;
    use std::future::pending;
    use std::time::{Duration, Instant};

    let backend = common::build_backend_for_fixture("act-detach-abort").await;
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate iframe.html");

    // 大 deadline：若 detach 接线没生效，op 永挂，只能靠下面 10s 兜底超时暴露（≠ 秒级返回）。
    let parent = Progress::new(Duration::from_secs(3600));
    // 装 detach/crash 监听 + 派生子 Progress（act 跑在 child 上；guard 持监听任务，drop 收摊）。
    // D1：main_frame_id / arm_act_abort 现 async（经 active tab 解引用）。
    let main_frame_id = backend.main_frame_id().await.expect("active tab main frame");
    let (child, _guard) = backend
        .arm_act_abort(&parent, &main_frame_id)
        .await
        .expect("arm_act_abort on active tab");

    let started = Instant::now();

    // 并发：A) 在 child 上跑永挂 op 的重试编排；B) 稍候关掉 page target 触发 detach。
    // 用 join 而非 spawn：两者都借 backend/child（无需 'static）。op 永挂 → 唯一出口是 abort。
    let act_fut = async {
        nomi_browser_engine::actions::run_act_with_retry::<_, _, ()>(
            &child,
            false, // 非不可逆：但 op 永挂，重试逻辑无从触发，唯一出口是 progress.abort。
            |_attempt| async { pending().await },
        )
        .await
    };
    let trigger_fut = async {
        // 让 op 真正进入等待（observe 不需要；直接小睡即可），再关 target 触发 detach。
        tokio::time::sleep(Duration::from_millis(200)).await;
        backend
            .__close_page_target_for_test()
            .await
            .expect("close page target (trigger detach)");
    };

    // 10s 兜底：接线生效则 act 秒级返回；没生效则 op 永挂到这里超时（断言会失败并指出未接线）。
    let act_result = tokio::time::timeout(Duration::from_secs(10), async {
        let (act_out, _trigger) = tokio::join!(act_fut, trigger_fut);
        act_out
    })
    .await;

    let elapsed = started.elapsed();
    eprintln!("detach→abort act returned in {elapsed:?}: {act_result:?}");

    let act_out = act_result.expect(
        "act_with_retry did NOT return within 10s after page detach — \
         detach→progress.abort wiring is broken (op hung to fallback timeout)",
    );

    // ① 分类：page target 关闭 → PageClosed → errmap → TargetClosed。
    assert!(
        matches!(act_out, Err(BrowserError::TargetClosed)),
        "detach must surface TargetClosed via progress.abort(PageClosed), got {act_out:?}"
    );
    // ② 远早于 3600s deadline：秒级（含 200ms 触发延迟 + 调度）。给 5s 上限足够宽松又能证明非走 deadline。
    assert!(
        elapsed < Duration::from_secs(5),
        "abort must fire near-immediately (≪ 3600s deadline), took {elapsed:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// C1：click / type / set_value 三分支端到端（经公开 `act()` trait 方法，串 B2-B6）。
// 一个测试覆盖三动作 + 登录表单 + verify 读回（建一次 chrome 最省资源）；stale ref 单独一个。
// 本机 Windows 真跑：
//   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(c1_)'
// ═══════════════════════════════════════════════════════════════════════════

/// **C1 click + type + set_value + 登录表单全链**（经 `act()`，verify 读回真值）。
///
/// fixture `act-c1.html`：`#name`（text input）+ `#notes`（textarea）+ `#toggle`（button，点击改文案
/// "Not clicked"→"Clicked"）+ 登录表单（`#username`/`#password` input + `#submit` button，submit 时把
/// 用户名写进 `#login-status` 的 role=status 标记）。navigate → observe → 经 `act()` 执行：
/// - **type**：`Type{name_ref, Literal("Ada Lovelace")}` → 读回 `#name.value == "Ada Lovelace"`；
/// - **set_value**：`SetValue{notes_ref, "multi\nline"}` → 读回 `#notes.value` == 设的值；
/// - **click**：`Click{toggle_ref}` → 读回 `#toggle.textContent == "Clicked"`（点击真分发）；
/// - **登录表单**：type 用户名 → type 密码（Secret）→ click submit → 读回 `#login-status` == "submitted:<user>"。
///
/// 每步断言 `ActResult.success == true` + `Effect.changed`（type/set_value 的 before≠after value）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c1_click_type_set_value_and_login_form() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::{ActSpec, TypeInput};
    use std::time::Duration;

    let backend = common::build_backend_for_fixture("c1-actions").await;
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate act-c1.html");

    // 每个动作给宽松 deadline（真 chrome 几何/注入耗时；远小于此）。
    let new_progress = || Progress::new(Duration::from_secs(30));

    // observe 拿 ref 表（act 反查的前置）。
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== c1 entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }
    let find = |role: &str, name: &str| -> String {
        obs.entries
            .iter()
            .find(|e| e.role == role && e.name == name)
            .unwrap_or_else(|| panic!("fixture should expose {role:?} {name:?}"))
            .r#ref
            .clone()
    };

    // 页面 world 读 DOM 值的小工具。
    let read = |expr: &'static str| {
        let backend = &backend;
        async move {
            backend
                .__eval_page_world_for_test(expr)
                .await
                .expect("eval read")
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        }
    };

    // ── type：#name ───────────────────────────────────────────────────────────
    let name_ref = find("textbox", "Name");
    let p = new_progress();
    let type_res = backend
        .act(&ActSpec::Type { r#ref: name_ref.clone(), text: TypeInput::Literal("Ada Lovelace".into()) }, &p)
        .await
        .expect("act type #name");
    let name_value = read("document.getElementById('name').value").await;
    eprintln!("type result = {type_res:?}; #name.value = {name_value:?}");
    assert!(type_res.success, "type must succeed");
    assert_eq!(name_value, "Ada Lovelace", "type should write the literal into #name");
    assert!(type_res.effect.changed, "type Effect.changed must be true (value changed)");

    // ── set_value：#notes（textarea，含换行——set_value 是整体设值快路径）────────────
    let notes_ref = find("textbox", "Notes");
    let p = new_progress();
    let set_res = backend
        .act(&ActSpec::SetValue { r#ref: notes_ref.clone(), value: "multi\nline notes".into(), secret: false }, &p)
        .await
        .expect("act set_value #notes");
    let notes_value = read("document.getElementById('notes').value").await;
    eprintln!("set_value result = {set_res:?}; #notes.value = {notes_value:?}");
    assert!(set_res.success, "set_value must succeed");
    assert_eq!(notes_value, "multi\nline notes", "set_value should set the textarea value verbatim");
    assert!(set_res.effect.changed, "set_value Effect.changed must be true");

    // ── click：#toggle 态变 ────────────────────────────────────────────────────
    let toggle_ref = find("button", "Not clicked");
    let toggle_before = read("document.getElementById('toggle').textContent").await;
    let p = new_progress();
    let click_res = backend
        .act(&ActSpec::Click { r#ref: toggle_ref.clone() }, &p)
        .await
        .expect("act click #toggle");
    let toggle_after = read("document.getElementById('toggle').textContent").await;
    eprintln!("click result = {click_res:?}; toggle: {toggle_before:?} -> {toggle_after:?}");
    assert!(click_res.success, "click must succeed");
    assert_eq!(toggle_before, "Not clicked", "toggle starts 'Not clicked'");
    assert_eq!(toggle_after, "Clicked", "click should真分发到按钮 toggling text to 'Clicked'");

    // ── 登录表单（验收点）：type user → type pass(Secret) → click submit → 标记态变 ──
    // 表单元素文案变了不影响（这些 ref 同一代际仍有效——没导航，没重拍）。
    let user_ref = find("textbox", "Username");
    let pass_ref = find("textbox", "Password");
    let submit_ref = find("button", "Sign in");

    let p = new_progress();
    backend
        .act(&ActSpec::Type { r#ref: user_ref.clone(), text: TypeInput::Literal("ada".into()) }, &p)
        .await
        .expect("act type username");
    let p = new_progress();
    let pass_res = backend
        .act(&ActSpec::Type { r#ref: pass_ref.clone(), text: TypeInput::Secret("s3cr3t-pw".into()) }, &p)
        .await
        .expect("act type password (secret)");
    // secret 安全契约：ActResult.message 不含明文，Effect 锚点不含值。
    assert!(!pass_res.message.contains("s3cr3t-pw"), "secret value must not leak into message: {pass_res:?}");
    assert!(pass_res.effect.before_anchor.is_none() && pass_res.effect.after_anchor.is_none(),
        "secret Effect anchors must be None (no value capture), got {:?}", pass_res.effect);

    let p = new_progress();
    let submit_res = backend
        .act(&ActSpec::Click { r#ref: submit_ref.clone() }, &p)
        .await
        .expect("act click submit");
    let login_status = read("document.getElementById('login-status').textContent").await;
    eprintln!("submit result = {submit_res:?}; #login-status = {login_status:?}");
    assert!(submit_res.success, "submit click must succeed");
    assert_eq!(login_status, "submitted:ada", "login form submit should fire with typed username");

    // 实际读回汇总（贴进汇报）。
    eprintln!(
        "=== C1 READBACK SUMMARY ===\n\
         type #name.value      = {name_value:?}\n\
         set_value #notes.value= {notes_value:?}\n\
         click #toggle text    = {toggle_before:?} -> {toggle_after:?}\n\
         login #login-status   = {login_status:?}\n\
         password secret leak? = message={:?} anchors={:?}",
        pass_res.message, pass_res.effect
    );
}

/// **F2 verify-after-act 富锚点 + set_value secret 锚点抑制**（经 `act()`，本机 Windows 真跑）。
///
/// fixture `act-c1.html`：复用 `#password`（set_value secret 写入，验锚点抑制）+ 新增 `#agree`
/// （checkbox，验 click 富锚点 checked 翻转 → changed=true，URL 不变也能判）+ `#noop`（无副作用按钮，
/// 验失败/无变化 → changed=false 如实，非静默）。navigate → observe → 经 `act()`：
/// - **set_value secret**（`SetValue{password_ref, "s3cr3t-pw", secret:true}`）：**关键安全回归**——
///   `Effect.before_anchor`/`after_anchor` 必须为 `None`（不采 read-back 明文），`message` 不含明文；
///   读回 `#password.value` == 设的值（证 secret 真写入，只是不进锚点）；
/// - **checkbox click**（`Click{agree_ref}`）：富锚点捕 `checked` 翻转 → `changed=true`，且 after_anchor
///   含 `checked:true`（不导航也判 changed——F2 富锚点主体）；
/// - **no-op click**（`Click{noop_ref}`）：不改态不导航 → `changed=false` 如实（never assume executed==succeeded）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn f2_verify_anchors_and_set_value_secret_suppression() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::ActSpec;
    use std::time::Duration;

    let backend = common::build_backend_for_fixture("f2-verify").await;
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate act-c1.html");

    let new_progress = || Progress::new(Duration::from_secs(30));

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== f2 entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }
    let find = |role: &str, name: &str| -> String {
        obs.entries
            .iter()
            .find(|e| e.role == role && e.name == name)
            .unwrap_or_else(|| panic!("fixture should expose {role:?} {name:?}"))
            .r#ref
            .clone()
    };
    let read = |expr: &'static str| {
        let backend = &backend;
        async move {
            backend
                .__eval_page_world_for_test(expr)
                .await
                .expect("eval read")
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        }
    };

    // ── set_value secret（安全红线）：anchor=None + message 不含明文 + 真写入 ─────────────
    let pass_ref = find("textbox", "Password");
    let p = new_progress();
    let secret_res = backend
        .act(
            &ActSpec::SetValue {
                r#ref: pass_ref.clone(),
                value: "s3cr3t-setvalue-pw".into(),
                secret: true,
            },
            &p,
        )
        .await
        .expect("act set_value (secret)");
    let pass_value = read("document.getElementById('password').value").await;
    eprintln!("set_value secret = {secret_res:?}; #password.value = {pass_value:?}");
    assert!(secret_res.success, "set_value secret must succeed");
    // **关键安全回归**：锚点必须为 None（不采 read-back 明文）。
    assert!(
        secret_res.effect.before_anchor.is_none() && secret_res.effect.after_anchor.is_none(),
        "set_value secret Effect anchors MUST be None (no value capture), got {:?}",
        secret_res.effect
    );
    // message 不含明文。
    assert!(
        !secret_res.message.contains("s3cr3t-setvalue-pw"),
        "set_value secret value must not leak into message: {}",
        secret_res.message
    );
    // 真写入（证只是不进锚点，值确实 fill 进了控件）。
    assert_eq!(pass_value, "s3cr3t-setvalue-pw", "set_value secret should fill the password verbatim");

    // ── checkbox click 富锚点：checked 翻转 → changed=true + after_anchor 含 checked:true ──
    let agree_ref = find("checkbox", "I agree");
    let p = new_progress();
    let cb_res = backend
        .act(&ActSpec::Click { r#ref: agree_ref.clone() }, &p)
        .await
        .expect("act click checkbox");
    let agree_checked = read("String(document.getElementById('agree').checked)").await;
    eprintln!("checkbox click = {cb_res:?}; #agree.checked = {agree_checked:?}");
    assert!(cb_res.success, "checkbox click must succeed");
    assert_eq!(agree_checked, "true", "checkbox should be checked after click");
    // 富锚点：URL 没变，但 checked false→true 让 changed=true（F2 富锚点主体）。
    assert!(
        cb_res.effect.changed,
        "checkbox click Effect.changed must be true via checked anchor (no nav), got {:?}",
        cb_res.effect
    );
    let after_checked = cb_res
        .effect
        .after_anchor
        .as_ref()
        .and_then(|v| v.get("checked"))
        .and_then(|v| v.as_bool());
    assert_eq!(
        after_checked,
        Some(true),
        "after_anchor must carry checked:true; effect = {:?}",
        cb_res.effect
    );

    // ── no-op click：不改态不导航 → changed=false 如实（never assume executed==succeeded）──
    let noop_ref = find("button", "No-op");
    let p = new_progress();
    let noop_res = backend
        .act(&ActSpec::Click { r#ref: noop_ref.clone() }, &p)
        .await
        .expect("act click no-op");
    eprintln!("no-op click = {noop_res:?}");
    assert!(noop_res.success, "no-op click still dispatches (success=true)");
    assert!(
        !noop_res.effect.changed,
        "no-op click must report changed=false truthfully, got {:?}",
        noop_res.effect
    );

    eprintln!(
        "=== F2 READBACK SUMMARY ===\n\
         set_value secret: anchors={:?} (MUST be None,None) message-leak?={}\n\
         #password.value (written, not in anchor) = {pass_value:?}\n\
         checkbox click changed={} after_checked={:?}\n\
         no-op click changed={}",
        (secret_res.effect.before_anchor.as_ref(), secret_res.effect.after_anchor.as_ref()),
        secret_res.message.contains("s3cr3t-setvalue-pw"),
        cb_res.effect.changed,
        after_checked,
        noop_res.effect.changed,
    );
}

/// **C1 stale ref → NodeStale（文案引导 re-observe）**：用一个不存在/过期的 ref 调 `act()`，应在层①
/// （纯 Rust，不进浏览器）即返 [`BrowserError::NodeStale`]，且 Display 文案含「stale」（引导模型重拍）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c1_stale_ref_returns_node_stale() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::ActSpec;
    use std::time::Duration;

    let backend = common::build_backend_for_fixture("c1-stale").await;
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate");
    // 即便没 observe（表为空）/ 用伪 ref，都应是 NodeStale（层①）。
    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(&ActSpec::Click { r#ref: "f9e99999".into() }, &p)
        .await;
    eprintln!("stale act = {res:?}");
    match res {
        Err(BrowserError::NodeStale { .. }) => {
            // 文案应引导 re-observe（Display 含 "stale"）。
            let msg = format!("{}", BrowserError::NodeStale { generation: 0 });
            assert!(msg.contains("stale"), "NodeStale Display should guide re-observe, got {msg:?}");
        }
        other => panic!("stale ref must surface NodeStale (layer①, no browser), got {other:?}"),
    }
}

/// **C1 不可编辑元素 type → Blocked（禁重试）**：对一个非可编辑元素（heading）跑 `Type`，
/// check_states(editable) 的不可编辑特例 → check_states 返 Err(Blocked) → C1 用
/// `classify_editable_check_err` 判 Fatal（禁重试，**非** classify_browser_err 的 Retryable）→
/// `act()` 返 [`BrowserError::Blocked`]（B3 语义：元素类型根本不支持编辑，NonRecoverable）。
/// 复用 `actionability.html`（含 heading "Actionability"）。**耗时断言**佐证「Fatal 立返」——绝不
/// 走完 6 槽退避（770ms sleep）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c1_type_non_editable_is_blocked() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::{ActSpec, TypeInput};
    use std::time::{Duration, Instant};

    let backend = common::build_backend_for_fixture("c1-non-editable").await;
    backend
        .navigate(&common::fixture_url("actionability.html"), false)
        .await
        .expect("navigate actionability.html");
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    let heading_ref = obs
        .entries
        .iter()
        .find(|e| e.role == "heading" && e.name == "Actionability")
        .expect("fixture should expose heading")
        .r#ref
        .clone();
    let p = Progress::new(Duration::from_secs(30));
    let started = Instant::now();
    let res = backend
        .act(&ActSpec::Type { r#ref: heading_ref, text: TypeInput::Literal("nope".into()) }, &p)
        .await;
    let elapsed = started.elapsed();
    eprintln!("type-non-editable act = {res:?} (elapsed {elapsed:?})");
    assert!(
        matches!(res, Err(BrowserError::Blocked { .. })),
        "typing into a non-editable element must be Blocked (NonRecoverable, no retry), got {res:?}"
    );
    // **禁重试佐证**：不可编辑 → Fatal 立返，绝不走完 6 槽退避（770ms sleep）。即使含一次 resolve +
    // check_states 的真 chrome 往返，单次尝试也远小于「退避全跑完」的下限。给 600ms 宽松上限：
    // 远低于 770ms 退避 sleep（更别说 + 6 次往返），又足够宽容真 chrome 单次往返抖动。
    assert!(
        elapsed < Duration::from_millis(600),
        "non-editable Type must fail fast (Fatal, no backoff); took {elapsed:?} \
         (≥770ms would mean it wrongly ran the full retry backoff)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// C2：hover / select_option / press_key / scroll / scroll_to_text 端到端（经公开 `act()`）。
// 一个测试覆盖全 5 动作 + verify 读回（建一次 chrome 最省资源）。
// 本机 Windows 真跑：
//   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(c2_)'
// ═══════════════════════════════════════════════════════════════════════════

/// **C2 hover + select_option + press_key + scroll + scroll_to_text 全链**（经 `act()`，verify 读回真态）。
///
/// fixture `c2.html`：`#hovertarget`（onmouseover 打 `data-hovered=yes` + 文案变 "Hovered"）+ `#picker`
/// （3-option select）+ `#q`（form 内输入框，Enter→submit 打 `#search-status`）+ 3000px spacer + 底部
/// `#bottommarker`（"Unique footer sentinel text"）/`#bottomtarget`。navigate → observe → 经 `act()`：
/// - **hover**：`Hover{hovertarget_ref}` → 读回 `#hovertarget[data-hovered] == "yes"`（hover 真投递）；
/// - **select_option**：`SelectOption{picker_ref, ["Second"]}` → 读回 `#picker.value == "opt2"`（按 label 命中）；
/// - **scroll**：`Scroll{Viewport, Down, 1500}` → scrollY 从 0 变大（视口真滚）；再 scroll 到底再 scroll →
///   良性不报错（success=true, changed=false）；
/// - **scroll_to_text**：`ScrollToText{"Unique footer sentinel text"}` → success=true + 底部元素进视口；
///   未命中文本（乱串）→ success=false 如实（非报错）；
/// - **press_key**：先点 `#q` 聚焦 + type → `PressKey{"Enter"}`（form 内）→ 读回 `#search-status == "searched:<q>"`。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c2_hover_select_press_key_scroll_scroll_to_text() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::{ActSpec, ScrollDir, ScrollTarget, TypeInput};
    use std::time::Duration;

    let backend = common::build_backend_for_fixture("c2-actions").await;
    backend
        .navigate(&common::fixture_url("c2.html"), false)
        .await
        .expect("navigate c2.html");

    let new_progress = || Progress::new(Duration::from_secs(30));

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== c2 entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }
    let find = |role: &str, name: &str| -> String {
        obs.entries
            .iter()
            .find(|e| e.role == role && e.name == name)
            .unwrap_or_else(|| panic!("fixture should expose {role:?} {name:?}"))
            .r#ref
            .clone()
    };

    // 页面 world 读 DOM 的小工具。
    let read = |expr: &'static str| {
        let backend = &backend;
        async move {
            backend
                .__eval_page_world_for_test(expr)
                .await
                .expect("eval read")
                .get("value")
                .map(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_default()
        }
    };

    // ── hover：onmouseover 打标记 ───────────────────────────────────────────────
    // hovertarget 是 button（aria 给它稳定 name "Hover me"）。
    let hover_ref = find("button", "Hover me");
    let p = new_progress();
    let hover_res = backend
        .act(&ActSpec::Hover { r#ref: hover_ref.clone() }, &p)
        .await
        .expect("act hover");
    let hovered = read("document.getElementById('hovertarget').getAttribute('data-hovered')").await;
    eprintln!("hover result = {hover_res:?}; data-hovered = {hovered:?}");
    assert!(hover_res.success, "hover must succeed");
    assert_eq!(hovered, "yes", "hover should真投递 mouseover, setting data-hovered=yes");

    // ── select_option：按 label "Second" 选 → value opt2 ─────────────────────────
    let picker_ref = find("combobox", "Pick");
    let p = new_progress();
    let select_res = backend
        .act(
            &ActSpec::SelectOption {
                r#ref: picker_ref.clone(),
                options: vec!["Second".into()],
            },
            &p,
        )
        .await
        .expect("act select_option");
    let picker_value = read("document.getElementById('picker').value").await;
    eprintln!("select result = {select_res:?}; #picker.value = {picker_value:?}");
    assert!(select_res.success, "select_option must succeed");
    assert_eq!(picker_value, "opt2", "select 'Second' (label) should set value to opt2");
    // ActResult.effect 自身必须反映真实 select.value 变化（守 C2 bug：旧 verify 对 <select> 读
    // textContent → 前后锚点都是 option 文案拼接 → 误报 changed=false）。before=opt1（默认选中第一项），
    // after=opt2，故 changed=true。
    assert!(
        select_res.effect.changed,
        "select effect.changed must be true (value opt1→opt2): effect = {:?}",
        select_res.effect
    );
    assert_eq!(
        select_res.effect.after_anchor.as_ref().and_then(|v| v.as_str()),
        Some("opt2"),
        "select after_anchor must be the selected option's value (opt2), not option text: effect = {:?}",
        select_res.effect
    );
    assert_eq!(
        select_res.effect.before_anchor.as_ref().and_then(|v| v.as_str()),
        Some("opt1"),
        "select before_anchor must be the prior selected value (opt1), not option text: effect = {:?}",
        select_res.effect
    );

    // ── scroll：viewport down → scrollY 变大 ────────────────────────────────────
    let scroll_y_before = read("String(window.scrollY)").await;
    let p = new_progress();
    let scroll_res = backend
        .act(
            &ActSpec::Scroll {
                target: ScrollTarget::Viewport,
                direction: ScrollDir::Down,
                amount: Some(1500.0),
            },
            &p,
        )
        .await
        .expect("act scroll down");
    let scroll_y_after = read("String(window.scrollY)").await;
    eprintln!(
        "scroll result = {scroll_res:?}; scrollY {scroll_y_before:?} -> {scroll_y_after:?}"
    );
    assert!(scroll_res.success, "scroll must succeed");
    let y_before: f64 = scroll_y_before.parse().unwrap_or(-1.0);
    let y_after: f64 = scroll_y_after.parse().unwrap_or(-1.0);
    assert!(
        y_after > y_before,
        "scroll down must increase scrollY ({y_before} -> {y_after})"
    );

    // ── scroll 到底 + 再 scroll → 良性不报错（success=true，到边界 changed=false）──
    // 先猛滚到底（一大步）。
    let p = new_progress();
    backend
        .act(
            &ActSpec::Scroll {
                target: ScrollTarget::Viewport,
                direction: ScrollDir::Down,
                amount: Some(99999.0),
            },
            &p,
        )
        .await
        .expect("scroll to bottom");
    // 已到底，再 scroll down → 良性：success=true，changed=false（无更多内容）。
    let p = new_progress();
    let at_bottom_res = backend
        .act(
            &ActSpec::Scroll {
                target: ScrollTarget::Viewport,
                direction: ScrollDir::Down,
                amount: Some(1500.0),
            },
            &p,
        )
        .await
        .expect("scroll at bottom must not error (benign)");
    eprintln!("scroll-at-bottom result = {at_bottom_res:?}");
    assert!(at_bottom_res.success, "scroll at bottom must be benign success (no error)");
    assert!(
        !at_bottom_res.effect.changed,
        "scroll at bottom should report changed=false (already at edge), got {at_bottom_res:?}"
    );

    // ── scroll_to_text：找底部 sentinel → 进视口 ────────────────────────────────
    // 先滚回顶部，确保 scroll_to_text 真做了滚动。
    backend
        .__eval_page_world_for_test("window.scrollTo(0, 0)")
        .await
        .expect("reset scroll");
    let p = new_progress();
    let s2t_res = backend
        .act(
            &ActSpec::ScrollToText {
                text: "Unique footer sentinel text".into(),
            },
            &p,
        )
        .await
        .expect("act scroll_to_text");
    // 验证底部元素进视口：bottomtarget 的 getBoundingClientRect().top < innerHeight。
    let target_in_view = read(
        "(() => { const r = document.getElementById('bottomtarget').getBoundingClientRect(); \
          return String(r.top < window.innerHeight && r.bottom > 0); })()",
    )
    .await;
    eprintln!("scroll_to_text result = {s2t_res:?}; bottomtarget in view = {target_in_view:?}");
    assert!(s2t_res.success, "scroll_to_text must succeed when text exists");
    assert_eq!(target_in_view, "true", "scroll_to_text should bring footer into viewport");

    // ── scroll_to_text 未命中：success=false 如实（非报错）─────────────────────────
    let p = new_progress();
    let miss_res = backend
        .act(
            &ActSpec::ScrollToText {
                text: "zzz-nonexistent-text-zzz".into(),
            },
            &p,
        )
        .await
        .expect("scroll_to_text miss must not error (benign)");
    eprintln!("scroll_to_text miss result = {miss_res:?}");
    assert!(!miss_res.success, "scroll_to_text miss must be success=false (not error)");

    // ── press_key Enter-in-form：点 #q 聚焦 + type → Enter submit → 标记态变 ────────
    // scroll_to_text 把页面滚到了底部，#q 在顶部已不在视口——先滚回顶部让 #q 可点。
    backend
        .__eval_page_world_for_test("window.scrollTo(0, 0)")
        .await
        .expect("reset scroll before press_key");
    // 重新 observe 不必要（没导航）；直接用 input 的 ref 点聚焦再 type。
    let q_ref = find("textbox", "Query");
    let p = new_progress();
    backend
        .act(&ActSpec::Click { r#ref: q_ref.clone() }, &p)
        .await
        .expect("click #q to focus");
    let p = new_progress();
    backend
        .act(
            &ActSpec::Type {
                r#ref: q_ref.clone(),
                text: TypeInput::Literal("hello-query".into()),
            },
            &p,
        )
        .await
        .expect("type into #q");
    // 诊断：press_key 前确认焦点确实在 #q（form 内），否则 Enter 不会触发隐式提交。
    let active_id = read("document.activeElement ? document.activeElement.id : '(none)'").await;
    let q_value = read("document.getElementById('q').value").await;
    eprintln!("before press_key: activeElement.id = {active_id:?}; #q.value = {q_value:?}");
    let p = new_progress();
    let press_res = backend
        .act(&ActSpec::PressKey { keys: "Enter".into() }, &p)
        .await
        .expect("act press_key Enter");
    let search_status = read("document.getElementById('search-status').textContent").await;
    eprintln!("press_key result = {press_res:?}; #search-status = {search_status:?}");
    assert!(press_res.success, "press_key must succeed");
    assert_eq!(
        search_status, "searched:hello-query",
        "Enter in form should fire submit with typed query"
    );

    // 实际读回汇总（贴进汇报）。
    eprintln!(
        "=== C2 READBACK SUMMARY ===\n\
         hover #hovertarget data-hovered = {hovered:?}\n\
         select #picker.value            = {picker_value:?}\n\
         scroll scrollY                  = {scroll_y_before:?} -> {scroll_y_after:?}\n\
         scroll-at-bottom changed        = {}\n\
         scroll_to_text footer in view   = {target_in_view:?}\n\
         scroll_to_text miss success     = {}\n\
         press_key #search-status        = {search_status:?}",
        at_bottom_res.effect.changed,
        miss_res.success
    );
}

/// **C2 element-target scroll（4 alignment 逃 sticky）**：长页底部元素经 `Scroll{Element{ref}}` 滚进视口。
/// fixture `c2.html` 的 `#bottomtarget`（按钮）初始在 3000px 之外。observe → `Scroll{Element{bottomtarget_ref}}`
/// → 读回该元素 getBoundingClientRect().top < innerHeight（进视口）。验证 element-target 滚动 + 4 alignment。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c2_scroll_element_into_view() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::{ActSpec, ScrollDir, ScrollTarget};
    use std::time::Duration;

    let backend = common::build_backend_for_fixture("c2-scroll-element").await;
    backend
        .navigate(&common::fixture_url("c2.html"), false)
        .await
        .expect("navigate c2.html");
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    let target_ref = obs
        .entries
        .iter()
        .find(|e| e.role == "button" && e.name == "Bottom target button")
        .expect("fixture should expose bottom target button")
        .r#ref
        .clone();

    // 确保初始不在视口（reset 到顶部）。
    backend
        .__eval_page_world_for_test("window.scrollTo(0, 0)")
        .await
        .expect("reset scroll");

    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(
            &ActSpec::Scroll {
                target: ScrollTarget::Element { r#ref: target_ref.clone() },
                direction: ScrollDir::Down,
                amount: None,
            },
            &p,
        )
        .await
        .expect("act scroll element into view");
    let in_view = backend
        .__eval_page_world_for_test(
            "(() => { const r = document.getElementById('bottomtarget').getBoundingClientRect(); \
              return r.top < window.innerHeight && r.bottom > 0; })()",
        )
        .await
        .expect("read in-view")
        .get("value")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    eprintln!("scroll-element result = {res:?}; bottomtarget in view = {in_view}");
    assert!(res.success, "scroll element must succeed");
    assert!(in_view, "scroll Element should bring the bottom button into viewport");
}

// ═══════════════════════════════════════════════════════════════════════════
// C3：只读类（get_page_text / search_page / find_elements / get_dropdown_options /
// cursor / wait / wait_for）端到端（经公开 `act()`）。**全只读零写**。
// 一个测试覆盖大部分（建一次 chrome 最省资源）；wait_for 单独一个（含计时）。
// 本机 Windows 真跑：
//   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(c3_)'
// ═══════════════════════════════════════════════════════════════════════════

/// **C3 get_page_text(脱敏) + search_page(命中/未命中) + find_elements(返 ref→可反解点中) +
/// get_dropdown_options + cursor + wait** 全链（经 `act()`）。
///
/// fixture `c3.html`：正文含 sentinel + 明文 secret（sk-/Bearer）；`button.primary` ×2 +
/// `.secondary`；`<select id=picker>`（含 selected/disabled option）。navigate → observe → 经 `act()`：
/// - **get_page_text**：返页面文本，且 **不含明文 secret**（脱敏验证）+ `<data>` 包裹（防注入）；
/// - **search_page**：grep "unique-sentinel-marker" 命中返片段；grep 乱串未命中 success=false（非报错）；
/// - **find_elements**：`button.primary` 命中 2 个 → 返 2 个 ref → **用首个 ref `act(Click)` 点中**
///   （`#p1` 文案变 + `#click-status==p1-clicked`，证 ref 真可反解端到端）；
/// - **get_dropdown_options**：`#picker` 枚举 3 个 option（含 selected=Second/disabled=Third）；
/// - **cursor**：返 pointer-cursor 元素计数 > 0；
/// - **wait**：`Wait{50}` 成功；`Wait{超大}` 被钳制（文案含 cap）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c3_page_text_search_find_dropdown_cursor_wait() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::ActSpec;
    use std::time::Duration;

    let backend = common::build_backend_for_fixture("c3-readonly").await;
    backend
        .navigate(&common::fixture_url("c3.html"), false)
        .await
        .expect("navigate c3.html");

    let new_progress = || Progress::new(Duration::from_secs(30));

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== c3 entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }

    let read = |expr: &'static str| {
        let backend = &backend;
        async move {
            backend
                .__eval_page_world_for_test(expr)
                .await
                .expect("eval read")
                .get("value")
                .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))
                .unwrap_or_default()
        }
    };

    // ── get_page_text：脱敏 + <data> 包裹 ──────────────────────────────────────
    let p = new_progress();
    let text_res = backend
        .act(&ActSpec::GetPageText, &p)
        .await
        .expect("act get_page_text");
    eprintln!("get_page_text message (first 400 chars):\n{}", &text_res.message.chars().take(400).collect::<String>());
    assert!(text_res.success, "get_page_text must succeed");
    // **脱敏验证**：明文 secret 绝不出现（sk- 串 / Bearer token）。
    assert!(
        !text_res.message.contains("sk-ABCDEFGHIJ0123456789xyzQRSTUV"),
        "get_page_text leaked the plaintext sk- secret:\n{}",
        text_res.message
    );
    assert!(
        !text_res.message.contains("abcdef0123456789ABCDEFghij"),
        "get_page_text leaked the Bearer token:\n{}",
        text_res.message
    );
    // <data> 包裹（不可信内容防注入）。
    assert!(text_res.message.contains("<data"), "page text must be <data>-wrapped: {}", text_res.message);
    // 正文 sentinel（非 secret 内容）应保留。
    assert!(
        text_res.message.contains("unique-sentinel-marker"),
        "non-secret page text should survive redaction: {}",
        text_res.message
    );

    // ── search_page：命中 + 未命中 ─────────────────────────────────────────────
    let p = new_progress();
    let search_hit = backend
        .act(&ActSpec::SearchPage { query: "unique-sentinel-marker".into() }, &p)
        .await
        .expect("act search_page hit");
    eprintln!("search hit = {search_hit:?}");
    assert!(search_hit.success, "search_page must succeed on a hit");
    assert!(search_hit.message.contains("unique-sentinel-marker"), "search hit must include the matched line");
    // 命中片段同样脱敏（即便 secret 行被命中也不泄漏）——验证 grep "Bearer" 不回明文。
    let p = new_progress();
    let search_secret = backend
        .act(&ActSpec::SearchPage { query: "Authorization".into() }, &p)
        .await
        .expect("act search_page on a line that contains a secret");
    eprintln!("search-on-secret-line = {search_secret:?}");
    assert!(
        !search_secret.message.contains("abcdef0123456789ABCDEFghij"),
        "search_page must redact secrets even when the matched line contains one:\n{}",
        search_secret.message
    );

    let p = new_progress();
    let search_miss = backend
        .act(&ActSpec::SearchPage { query: "zzz-nonexistent-zzz".into() }, &p)
        .await
        .expect("act search_page miss must not error");
    eprintln!("search miss = {search_miss:?}");
    assert!(!search_miss.success, "search_page miss must be success=false (benign, not error)");

    // ── find_elements：button.primary → 2 ref → 用首个 ref click 点中（反解验证）──────
    let p = new_progress();
    let find_res = backend
        .act(&ActSpec::FindElements { selector: "button.primary".into() }, &p)
        .await
        .expect("act find_elements");
    eprintln!("find_elements = {find_res:?}");
    assert!(find_res.success, "find_elements must succeed");
    // 提取返回的 ref（message 含 "[ref=f...]"）。
    let found_refs: Vec<String> = find_res
        .message
        .split("[ref=")
        .skip(1)
        .filter_map(|s| s.split(']').next().map(|r| r.to_string()))
        .collect();
    eprintln!("found refs = {found_refs:?}");
    assert_eq!(found_refs.len(), 2, "button.primary should match exactly 2 elements, got {found_refs:?}");
    // **关键：用 find_elements 返回的 ref act(Click) 能点中**（证 ref 真可被 resolve_ref 反解，端到端）。
    // #p1 是文档里第一个 .primary（Primary One），点击改文案 + 打 #click-status。
    let first_ref = found_refs[0].clone();
    let p = new_progress();
    let click_res = backend
        .act(&ActSpec::Click { r#ref: first_ref.clone() }, &p)
        .await
        .expect("act click on a find_elements ref");
    eprintln!("click-via-found-ref = {click_res:?}");
    assert!(click_res.success, "clicking a find_elements ref must succeed (ref reverse-resolves)");
    let click_status = read("document.getElementById('click-status').textContent").await;
    eprintln!("#click-status after clicking found ref = {click_status:?}");
    assert_eq!(
        click_status, "p1-clicked",
        "clicking the first find_elements ref must hit #p1 (proves ref is resolvable & actionable)"
    );

    // ── get_dropdown_options：枚举 3 个 option（含 selected/disabled）──────────────
    let picker_ref = obs
        .entries
        .iter()
        .find(|e| e.role == "combobox" && e.name == "Pick")
        .expect("fixture should expose the picker combobox")
        .r#ref
        .clone();
    let p = new_progress();
    let dd_res = backend
        .act(&ActSpec::GetDropdownOptions { r#ref: picker_ref.clone() }, &p)
        .await
        .expect("act get_dropdown_options");
    eprintln!("get_dropdown_options = {dd_res:?}");
    assert!(dd_res.success, "get_dropdown_options must succeed on a <select>");
    assert!(dd_res.message.contains("First") && dd_res.message.contains("Second") && dd_res.message.contains("Third"),
        "dropdown options must list all three labels: {}", dd_res.message);
    assert!(dd_res.message.contains("selected"), "dropdown must mark the selected option: {}", dd_res.message);
    assert!(dd_res.message.contains("disabled"), "dropdown must mark the disabled option: {}", dd_res.message);

    // get_dropdown_options on a non-<select> → success=false（良性）。
    let primary_ref = obs
        .entries
        .iter()
        .find(|e| e.role == "button" && e.name == "Secondary")
        .map(|e| e.r#ref.clone());
    if let Some(btn_ref) = primary_ref {
        let p = new_progress();
        let dd_bad = backend
            .act(&ActSpec::GetDropdownOptions { r#ref: btn_ref }, &p)
            .await
            .expect("get_dropdown_options on non-select must not error");
        eprintln!("get_dropdown_options on button = {dd_bad:?}");
        assert!(!dd_bad.success, "get_dropdown_options on a non-<select> must be success=false (benign)");
    }

    // ── cursor：pointer-cursor 元素计数 > 0（fixture 的 button/a 设了 cursor:pointer）──
    let p = new_progress();
    let cursor_res = backend.act(&ActSpec::Cursor, &p).await.expect("act cursor");
    eprintln!("cursor = {cursor_res:?}");
    assert!(cursor_res.success, "cursor must succeed");

    // ── wait：固定等待 + 钳制 ──────────────────────────────────────────────────
    let p = new_progress();
    let wait_res = backend.act(&ActSpec::Wait { ms: 50 }, &p).await.expect("act wait");
    eprintln!("wait = {wait_res:?}");
    assert!(wait_res.success, "wait must succeed");
    // 超大 ms 被钳制到 WAIT_MS_CAP（10s）。给这次 act 一个 > 10s 的 deadline（15s），让钳制后的
    // sleep 能在 deadline 内跑完（不被判 Timeout）。注意：这步会真睡 ~10s（钳制后的 sleep），属预期。
    let p = Progress::new(Duration::from_secs(15));
    let wait_cap = backend
        .act(&ActSpec::Wait { ms: 9_999_999 }, &p)
        .await
        .expect("act wait (capped)");
    eprintln!("wait-capped = {wait_cap:?}");
    assert!(wait_cap.success, "capped wait must succeed");
    assert!(wait_cap.message.contains("cap"), "over-limit wait must report capping: {}", wait_cap.message);

    eprintln!(
        "=== C3 READBACK SUMMARY ===\n\
         get_page_text secret-leak?     = sk:{} bearer:{}\n\
         get_page_text <data>-wrapped   = {}\n\
         search hit/miss                = hit={} miss_success={}\n\
         search-on-secret-line leak?    = {}\n\
         find_elements refs             = {found_refs:?}\n\
         click-via-found-ref status     = {click_status:?}\n\
         dropdown options listed        = First/Second/Third + selected + disabled\n\
         cursor                         = {}\n\
         wait / wait-capped             = ok / capped={}",
        text_res.message.contains("sk-ABCDEFGHIJ"),
        text_res.message.contains("abcdef0123456789ABCDEFghij"),
        text_res.message.contains("<data"),
        search_hit.success,
        search_miss.success,
        search_secret.message.contains("abcdef0123456789ABCDEFghij"),
        cursor_res.message,
        wait_cap.message.contains("cap"),
    );
}

/// **C3 wait_for：TextVisible 满足 + RefActionable 满足 + UrlContains 超时→Timeout{Action}**。
///
/// fixture `c3.html` 的 `#late` 在 1s 后注入 "delayed-content-loaded" 文本。
/// - **TextVisible**：`WaitFor{TextVisible{"delayed-content-loaded"}}` → 轮询直到 1s 后文本出现 → 成功
///   （SPA 软导航降级范式：不依赖 load 事件，靠轮询条件）；
/// - **RefActionable**：`WaitFor{RefActionable{<primary ref>}}` → 已 actionable → 立即成功；
/// - **超时**：`WaitFor{UrlContains{"never-happens"}}` 用短 deadline（这里靠 fixture 不变 URL）→
///   `Timeout{phase:Action}`（用一个永不满足的条件 + 等到 wait_for 默认 15s deadline 太久，故此处
///   直接验超时映射的形态——见纯逻辑单测 `wait_for_timeout_maps_to_action_timeout`；集成只验满足路径）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn c3_wait_for_text_visible_and_ref_actionable() {
    use nomi_browser_engine::progress::Progress;
    use nomi_browser_engine::{ActSpec, WaitCondition};
    use std::time::{Duration, Instant};

    let backend = common::build_backend_for_fixture("c3-wait-for").await;
    backend
        .navigate(&common::fixture_url("c3.html"), false)
        .await
        .expect("navigate c3.html");
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");

    // ── TextVisible：从测试侧确定性控制「延迟出现」（navigate→observe 耗时不定，fixture 的 1s
    //    setTimeout 可能已触发，故这里先把 #late 清空，再用页面侧 setTimeout 在 ~800ms 后注入
    //    一个全新 sentinel——保证 wait_for 启动时条件**未**满足，真走轮询直到出现）。
    backend
        .__eval_page_world_for_test(
            "(() => { const el = document.getElementById('late'); el.textContent = ''; \
              setTimeout(() => { el.textContent = 'polled-sentinel-xyz'; }, 800); return true; })()",
        )
        .await
        .expect("arm delayed text");
    let p = Progress::new(Duration::from_secs(30));
    let started = Instant::now();
    let tv_res = backend
        .act(
            &ActSpec::WaitFor {
                condition: WaitCondition::TextVisible { text: "polled-sentinel-xyz".into() },
            },
            &p,
        )
        .await
        .expect("act wait_for TextVisible");
    let tv_elapsed = started.elapsed();
    eprintln!("wait_for TextVisible = {tv_res:?} (elapsed {tv_elapsed:?})");
    assert!(tv_res.success, "wait_for TextVisible must succeed once the delayed text appears");
    // 文本 ~800ms 后才注入：应等到 ≥ ~600ms（容忍调度抖动），证明真在轮询直到满足（而非立即误判）。
    assert!(
        tv_elapsed >= Duration::from_millis(500),
        "wait_for should have polled until the ~800ms-delayed text appeared, took {tv_elapsed:?}"
    );

    // ── RefActionable：已可见可点的 primary button → 立即满足 ────────────────────
    let primary_ref = obs
        .entries
        .iter()
        .find(|e| e.role == "button" && e.name == "Primary One")
        .expect("fixture should expose Primary One button")
        .r#ref
        .clone();
    let p = Progress::new(Duration::from_secs(30));
    let ra_res = backend
        .act(
            &ActSpec::WaitFor {
                condition: WaitCondition::RefActionable { r#ref: primary_ref.clone() },
            },
            &p,
        )
        .await
        .expect("act wait_for RefActionable");
    eprintln!("wait_for RefActionable = {ra_res:?}");
    assert!(ra_res.success, "wait_for RefActionable must succeed for an already-actionable element");

    // ── UrlContains 满足（当前 URL 含 'c3.html'）→ 立即成功（SPA 软导航降级范式验证）──────
    let p = Progress::new(Duration::from_secs(30));
    let url_res = backend
        .act(
            &ActSpec::WaitFor {
                condition: WaitCondition::UrlContains { text: "c3.html".into() },
            },
            &p,
        )
        .await
        .expect("act wait_for UrlContains");
    eprintln!("wait_for UrlContains = {url_res:?}");
    assert!(url_res.success, "wait_for UrlContains must succeed when the URL already contains the substring");

    eprintln!(
        "=== C3 wait_for SUMMARY ===\n\
         TextVisible (1s delayed)  = success={} elapsed={tv_elapsed:?}\n\
         RefActionable             = success={}\n\
         UrlContains               = success={}",
        tv_res.success, ra_res.success, url_res.success
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// D3：tab 发现循环 + switch/close/open_link_new_tab/tabs 端到端（真 Chrome）。
// 验证：open_link_new_tab 不抢焦点（active 不变，返新 tab last4）→ 发现循环 arm 新 tab 入注册表 →
// tabs 列出 2 项 → switch 到新 tab（逻辑指针）→ 在新 tab observe 拿到其内容 → close 新 tab（active
// 自动回原 tab）→ 后续 observe 仍工作。close active 重选路径 + 无重复 arm（注册表无重复 key）一并验。
// ═══════════════════════════════════════════════════════════════════════════

/// 从 `tabs` 动作的 message 数当前纳管标签数（"N tab(s) open" 头行）。轮询新 tab 上桌用。
fn count_tabs_in_message(msg: &str) -> usize {
    // 头行形如 "2 tab(s) open ..."；空则 "no tabs open"。
    msg.split_whitespace()
        .next()
        .and_then(|w| w.parse::<usize>().ok())
        .unwrap_or(0)
}

/// 轮询 `tabs` 动作直到纳管标签数达到 `want`（发现循环 arm 是异步的），或超时返回最后一次 message。
async fn wait_until_tab_count(
    backend: &nomi_browser_engine::backend::CdpBackend,
    want: usize,
    timeout: std::time::Duration,
) -> String {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let res = backend.act_tabs().await.expect("act_tabs");
        let last = res.message;
        if count_tabs_in_message(&last) >= want {
            return last;
        }
        if std::time::Instant::now() >= deadline {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// **D3 全链**：open_link_new_tab（不抢焦点）→ 发现循环 arm → tabs 列 2 项 → switch → 新 tab observe
/// → close 新 tab → active 回原 tab → 后续 observe 仍工作。一个测试覆盖发现/tabs/switch/close/open。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn d3_open_switch_close_full_chain() {
    use nomi_browser_engine::{ActSpec, ObserveOpts};

    let backend = common::build_backend_for_fixture("d3-chain").await;
    // 原始 active tab 导航到 act-c1（含 button "Toggle me" 等可观测元素）。
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate act-c1.html");

    // 初始：tabs 列 1 项（原 active tab）。
    let t0 = backend.act_tabs().await.expect("tabs initial");
    eprintln!("=== D3 tabs (initial) ===\n{}", t0.message);
    assert_eq!(count_tabs_in_message(&t0.message), 1, "should start with exactly 1 tab");
    // 记原 active tab 的 last4（从 tabs 列表的 "(active)" 行抽）。
    let orig_active_l4 = t0
        .message
        .lines()
        .find(|l| l.contains("(active)"))
        .and_then(|l| l.split(']').next())
        .and_then(|l| l.split('[').nth(1))
        .map(|s| s.to_string())
        .expect("initial tab must be active");
    eprintln!("orig active last4 = {orig_active_l4}");

    // ── open_link_new_tab：不抢焦点（active 不变，返新 tab last4）──────────────────────
    let open_res = backend
        .act(&ActSpec::OpenLinkNewTab { url: common::fixture_url("c3.html") }, &mk_progress())
        .await
        .expect("open_link_new_tab");
    eprintln!("=== open_link_new_tab ===\n{}", open_res.message);
    assert!(open_res.success);
    let new_tab_l4 = open_res
        .effect
        .after_anchor
        .as_ref()
        .and_then(|a| a.get("new_tab"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .expect("open_link_new_tab must report the new tab's last4");
    eprintln!("new tab last4 = {new_tab_l4}");
    assert_ne!(new_tab_l4, orig_active_l4, "new tab must be a different tab");

    // **不抢焦点**：open 之后 active 仍是原 tab（发现循环 arm 新 tab 但不改 active）。
    let after_open = wait_until_tab_count(&backend, 2, std::time::Duration::from_secs(5)).await;
    eprintln!("=== tabs after open (waited for 2) ===\n{after_open}");
    assert_eq!(count_tabs_in_message(&after_open), 2, "discovery loop must arm the new tab → 2 tabs");
    // 无重复 arm：恰好 2 项（不是 3+），且原 active tab 仍 (active)。
    let active_line = after_open.lines().find(|l| l.contains("(active)")).expect("an active tab");
    assert!(
        active_line.contains(&format!("[{orig_active_l4}]")),
        "active must still be the original tab after open (no focus steal): {active_line}"
    );

    // ── switch_tab 到新 tab（逻辑指针切换）→ 在新 tab observe 拿到 c3.html 内容 ──────────
    let sw = backend
        .act(&ActSpec::SwitchTab { tab_id: new_tab_l4.clone() }, &mk_progress())
        .await
        .expect("switch_tab");
    eprintln!("=== switch_tab ===\n{}", sw.message);
    assert!(sw.success);

    let obs_new = backend.observe(&ObserveOpts::default()).await.expect("observe new tab");
    eprintln!("new tab url = {:?}", obs_new.url);
    assert!(
        obs_new.url.as_deref().map(|u| u.contains("c3.html")).unwrap_or(false),
        "after switch, observe must act on the new tab (c3.html), got {:?}",
        obs_new.url
    );

    // tabs 现在标新 tab 为 active。
    let t_after_switch = backend.act_tabs().await.expect("tabs after switch");
    let active_line2 = t_after_switch.lines_active();
    assert!(
        active_line2.contains(&format!("[{new_tab_l4}]")),
        "after switch the new tab must be active: {active_line2}"
    );

    // ── close_tab 新 tab（它是 active）→ active 自动回原 tab → 后续 observe 仍工作 ──────────
    let close = backend
        .act(&ActSpec::CloseTab { tab_id: new_tab_l4.clone() }, &mk_progress())
        .await
        .expect("close_tab active");
    eprintln!("=== close_tab (active) ===\n{}", close.message);
    assert!(close.success);
    // 重选回原 tab。
    let reselected = close
        .effect
        .after_anchor
        .as_ref()
        .and_then(|a| a.get("active_tab"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .expect("close active must report the reselected active tab");
    assert_eq!(reselected, orig_active_l4, "after closing the active tab, active must fall back to the original");

    // tabs 回 1 项；后续 observe 仍工作（作用在重选的原 tab，act-c1.html）。
    let t_final = backend.act_tabs().await.expect("tabs after close");
    eprintln!("=== tabs (final) ===\n{}", t_final.message);
    assert_eq!(count_tabs_in_message(&t_final.message), 1, "after close, back to 1 tab");
    let obs_back = backend.observe(&ObserveOpts::default()).await.expect("observe after close active");
    assert!(
        obs_back.url.as_deref().map(|u| u.contains("act-c1.html")).unwrap_or(false),
        "after closing active, observe must work on the reselected original tab (act-c1.html), got {:?}",
        obs_back.url
    );
    // 重选后能在原 tab 正常 act（click 一个按钮，验证句柄链端到端可用）。
    let obs_btn = obs_back.entries.iter().find(|e| e.role == "button");
    if let Some(btn) = obs_btn {
        let click = backend
            .act(&ActSpec::Click { r#ref: btn.r#ref.clone() }, &mk_progress())
            .await;
        eprintln!("post-reselect click {:?} = {click:?}", btn.r#ref);
        assert!(click.is_ok(), "act on the reselected tab must work, got {click:?}");
    }

    eprintln!("=== D3 full chain SUMMARY: open(no-focus-steal) → discover → tabs(2) → switch → observe(new) → close(active) → reselect(orig) → observe+act OK ===");
}

/// 小 helper：给 act 一个宽松 deadline 的 Progress。
fn mk_progress() -> nomi_browser_engine::progress::Progress {
    nomi_browser_engine::progress::Progress::new(std::time::Duration::from_secs(30))
}

/// `ActResult.message` 取 "(active)" 那一行的小工具（测试断言用）。
trait ActiveLine {
    fn lines_active(&self) -> String;
}
impl ActiveLine for nomi_browser_engine::ActResult {
    fn lines_active(&self) -> String {
        self.message
            .lines()
            .find(|l| l.contains("(active)"))
            .unwrap_or("")
            .to_string()
    }
}

/// **D3 close-active 重选 + 无重复 arm + 无残留循环**：开 2 个新 tab（共 3）→ 反复 open/close 同一组
/// 不让注册表膨胀（无重复 arm）→ switch 到一个新 tab → close 它（active）→ active 自动回某剩余 tab →
/// 后续 observe/act 仍工作 → 全部关到只剩原 tab。关 tab 后无残留循环（test 退出时 chrome 必清——由
/// kill_on_drop + 上层 PowerShell 核对）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn d3_close_active_reselect_and_no_duplicate_arm() {
    use nomi_browser_engine::{ActSpec, ObserveOpts};

    let backend = common::build_backend_for_fixture("d3-reselect").await;
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate act-c1.html");

    // 开两个新 tab（c3 + iframe）。各等其上桌。
    for url in ["c3.html", "iframe.html"] {
        backend
            .act(&ActSpec::OpenLinkNewTab { url: common::fixture_url(url) }, &mk_progress())
            .await
            .expect("open new tab");
    }
    let after_opens = wait_until_tab_count(&backend, 3, std::time::Duration::from_secs(6)).await;
    eprintln!("=== tabs after 2 opens ===\n{after_opens}");
    assert_eq!(count_tabs_in_message(&after_opens), 3, "two opens → 3 tabs total");

    // **无重复 arm**：把这 3 行的 last4 收集，去重后仍是 3（注册表无重复 key）。
    // 只解析 tab 行（"- [" 开头）；跳过头行（"... use the [id] ..." 含字面 [id]）。
    let last4s: Vec<&str> = after_opens
        .lines()
        .filter(|l| l.trim_start().starts_with("- ["))
        .filter_map(|l| l.split('[').nth(1).and_then(|s| s.split(']').next()))
        .collect();
    let mut uniq = last4s.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert_eq!(uniq.len(), last4s.len(), "no duplicate tab keys (discovery loop must not arm twice): {last4s:?}");
    assert_eq!(uniq.len(), 3, "exactly 3 distinct tabs");

    // 取一个非 active 的新 tab，switch 到它（成为 active），再 close 它（active）。
    let new_l4 = after_opens
        .lines()
        .find(|l| l.trim_start().starts_with("- [") && !l.contains("(active)"))
        .and_then(|l| l.split('[').nth(1).and_then(|s| s.split(']').next()))
        .map(|s| s.to_string())
        .expect("a non-active new tab");
    backend
        .act(&ActSpec::SwitchTab { tab_id: new_l4.clone() }, &mk_progress())
        .await
        .expect("switch to new tab");
    let closed = backend
        .act(&ActSpec::CloseTab { tab_id: new_l4.clone() }, &mk_progress())
        .await
        .expect("close active new tab");
    eprintln!("=== close active new tab ===\n{}", closed.message);
    assert!(closed.success);
    // active 重选到某剩余 tab（非被关的那个）。
    let reselected = closed
        .effect
        .after_anchor
        .as_ref()
        .and_then(|a| a.get("active_tab"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .expect("reselect reported");
    assert_ne!(reselected, new_l4, "must reselect a different tab than the closed one");

    // 后续 observe/act 仍工作（作用在重选 tab）。
    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe after close-active");
    eprintln!("post-close-active observe url = {:?}", obs.url);
    assert!(obs.url.is_some(), "observe must work on the reselected tab");

    // 关到只剩 1 个 tab：逐个 close 剩余的非原始 tab。
    loop {
        let listing = backend.act_tabs().await.expect("tabs").message;
        if count_tabs_in_message(&listing) <= 1 {
            break;
        }
        // 取任一非 active tab 关（避免每次都触发重选 churn；若只剩 active+1 则关 active 也可）。
        // 只看 tab 行（"- [" 开头），跳过头行。
        let victim = listing
            .lines()
            .filter(|l| l.trim_start().starts_with("- ["))
            .filter_map(|l| l.split('[').nth(1).and_then(|s| s.split(']').next()))
            .nth(1) // 跳过第 0 个 tab（确定性地关第 2 个）
            .map(|s| s.to_string());
        let Some(v) = victim else { break };
        backend
            .act(&ActSpec::CloseTab { tab_id: v.clone() }, &mk_progress())
            .await
            .expect("close remaining tab");
    }
    let final_tabs = backend.act_tabs().await.expect("tabs final").message;
    eprintln!("=== tabs final ===\n{final_tabs}");
    assert_eq!(count_tabs_in_message(&final_tabs), 1, "closed down to 1 tab");
    // 最后一个 tab 仍可 observe（无残留循环把它弄坏）。
    let obs_final = backend.observe(&ObserveOpts::default()).await.expect("observe final");
    assert!(obs_final.url.is_some(), "the surviving tab must still observe");
    eprintln!("=== D3 reselect/no-dup-arm SUMMARY: 3 tabs → close-active-reselect → close down to 1 → observe OK ===");
}

/// **D3 fix（裁决⑥）回归：跨 tab observe 隔离——OOPIF 循环只管 iframe，绝不把兄弟顶层 tab 当 OOPIF 子帧**。
///
/// **bug**：每个 per-tab OOPIF arm 循环订阅**全局** `Target.attachedToTarget`；旧 type 过滤
/// `ttype != "iframe" && ttype != "page"` 仍接受 `page`，于是看到**兄弟顶层 tab**（type=page、sid≠自己）
/// 就把它 arm 进自己的 `oopif_managers`，致 observe 活动 tab 时把兄弟 tab 整页内容当 OOPIF 子帧拼进来
/// （跨标签污染）。修法 = OOPIF 循环收紧只收 `iframe`（[`nomi_browser_engine::tabs::should_arm_as_oopif`]）。
///
/// **本测断言污染已消**：2 tab（原 tab=act-c1.html 标志「Sign in」/「Not clicked」；新 tab=c3.html 标志
/// 「Primary One」/「Primary Two」/sentinel）→ observe **活动 tab** → 其 observe entries 的 name **只**含
/// 本 tab 标志，**不含**另一 tab 标志（兄弟内容没被当 OOPIF 子帧拼进来）。两个方向都验（原 tab 不含 c3
/// 标志、切到新 tab 后不含 act-c1 标志）。get_page_text 同口径再验页面级文本不串味。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn d3_cross_tab_observe_isolation_no_sibling_pollution() {
    use nomi_browser_engine::{ActSpec, ObserveOpts};

    // observe entries 的全部 name 拼成一个串（便于 contains 断言）。
    fn names_blob(obs: &nomi_browser_engine::Observation) -> String {
        obs.entries
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    let backend = common::build_backend_for_fixture("d3-isolation").await;
    // 原 active tab = act-c1.html（标志：button "Sign in" / "Not clicked"）。
    backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate act-c1.html");

    // 开新 tab = c3.html（标志：button "Primary One" / "Primary Two"，正文 sentinel）。
    backend
        .act(&ActSpec::OpenLinkNewTab { url: common::fixture_url("c3.html") }, &mk_progress())
        .await
        .expect("open_link_new_tab c3");
    let after_open = wait_until_tab_count(&backend, 2, std::time::Duration::from_secs(6)).await;
    assert_eq!(count_tabs_in_message(&after_open), 2, "two tabs expected: {after_open}");

    // ── 方向 1：活动 tab 仍是原 tab（act-c1）→ observe 只含 C1 标志，不含 c3 标志 ──────────
    let obs_orig = backend.observe(&ObserveOpts::default()).await.expect("observe orig active");
    eprintln!("orig-active url={:?} names={}", obs_orig.url, names_blob(&obs_orig));
    assert!(
        obs_orig.url.as_deref().map(|u| u.contains("act-c1.html")).unwrap_or(false),
        "active tab should still be act-c1 (no focus steal), got {:?}",
        obs_orig.url
    );
    let orig_names = names_blob(&obs_orig);
    // 本 tab 的标志元素在场（observe 真拿到本 tab 内容）。
    assert!(
        orig_names.contains("Sign in") || orig_names.contains("Not clicked"),
        "orig tab observe must include its own C1 elements: {orig_names}"
    );
    // **隔离**：绝不含 c3 标志元素（兄弟 tab 内容没被当 OOPIF 子帧拼进来）。
    assert!(
        !orig_names.contains("Primary One") && !orig_names.contains("Primary Two"),
        "CROSS-TAB POLLUTION: orig tab observe must NOT include sibling c3 elements: {orig_names}"
    );
    // 页面级文本同口径再验：act-c1 文本不含 c3 的唯一 sentinel。
    let orig_text = backend.act(&ActSpec::GetPageText, &mk_progress()).await.expect("get_page_text orig");
    assert!(
        !orig_text.message.contains("unique-sentinel-marker"),
        "CROSS-TAB POLLUTION: orig tab get_page_text must NOT include c3's sentinel: {}",
        orig_text.message
    );

    // ── 方向 2：switch 到新 tab（c3）→ observe 只含 c3 标志，不含 C1 标志 ───────────────────
    let new_l4 = after_open
        .lines()
        .find(|l| l.trim_start().starts_with("- [") && !l.contains("(active)"))
        .and_then(|l| l.split('[').nth(1).and_then(|s| s.split(']').next()))
        .map(|s| s.to_string())
        .expect("the non-active new tab last4");
    backend
        .act(&ActSpec::SwitchTab { tab_id: new_l4.clone() }, &mk_progress())
        .await
        .expect("switch to c3 tab");

    let obs_new = backend.observe(&ObserveOpts::default()).await.expect("observe c3 active");
    eprintln!("c3-active url={:?} names={}", obs_new.url, names_blob(&obs_new));
    assert!(
        obs_new.url.as_deref().map(|u| u.contains("c3.html")).unwrap_or(false),
        "active tab should now be c3, got {:?}",
        obs_new.url
    );
    let new_names = names_blob(&obs_new);
    assert!(
        new_names.contains("Primary One") || new_names.contains("Primary Two"),
        "c3 tab observe must include its own elements: {new_names}"
    );
    // **隔离**：绝不含 act-c1 标志元素。
    assert!(
        !new_names.contains("Sign in") && !new_names.contains("Not clicked"),
        "CROSS-TAB POLLUTION: c3 tab observe must NOT include sibling act-c1 elements: {new_names}"
    );

    eprintln!("=== D3 cross-tab isolation SUMMARY: 2 tabs, observe(active) contains only its own elements, no sibling pollution (judgment ⑥) ===");
}

// ════════════════════════════════════════════════════════════════════════
// F1-sec: evaluate 全权 LIVE 门控端到端（E3）。默认 OFF → Unsupported；全权 ON（test seam =
// EngineConfig.evaluate_full_power=true）→ 放行真跑 JS。证「full_power LIVE 读真武装 evaluate 门」。
// 手动跑：set NOMIFUN_CHROME_BINARY 后
//   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(f1sec_evaluate)'
// ════════════════════════════════════════════════════════════════════════

use nomi_browser_engine::progress::Progress;
use nomi_browser_engine::ActSpec;

/// **F1-sec：evaluate 默认 OFF**（full_power 未 opt-in）→ `act(Evaluate)` 返
/// `Unsupported{capability:"evaluate"}`（default-deny，最高危逃生舱封死）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn f1sec_evaluate_off_by_default_real() {
    let backend = common::build_backend_for_fixture("f1sec-eval-off").await; // full_power = false
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate");

    let p = Progress::new(std::time::Duration::from_secs(20));
    let res = backend
        .act(&ActSpec::Evaluate { script: "1+1".into() }, &p)
        .await;
    eprintln!("evaluate (default OFF) result = {res:?}");
    match res {
        Err(BrowserError::Unsupported { capability, .. }) => {
            assert_eq!(capability, "evaluate", "default-off must report the evaluate capability");
        }
        other => panic!("evaluate must be Unsupported by default (default-deny), got {other:?}"),
    }
}

/// **F1-sec：evaluate 全权 ON LIVE 后放行**（test seam = EngineConfig.evaluate_full_power=true，等价
/// 用户在 System Settings opt-in full-power）→ `act(Evaluate)` 真跑 JS，`1+1` 求值结果可见。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn f1sec_evaluate_allowed_under_full_power_real() {
    let backend = common::build_backend_for_fixture_full_power("f1sec-eval-on").await; // full_power = true
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate");

    let p = Progress::new(std::time::Duration::from_secs(20));
    let res = backend
        .act(&ActSpec::Evaluate { script: "1+1".into() }, &p)
        .await;
    eprintln!("evaluate (full-power ON) result = {res:?}");
    let r = res.expect("evaluate must be allowed under full-power mode");
    assert!(r.success, "full-power evaluate should succeed");
    // after_anchor 携带 by-value 结果（1+1=2）；message 含全权模式提示。
    assert!(
        r.message.contains("full-power") || r.message.contains("evaluated"),
        "message should signal evaluate ran: {}",
        r.message
    );
    let got_two = r
        .effect
        .after_anchor
        .as_ref()
        .map(|v| v.to_string().contains('2'))
        .unwrap_or(false);
    assert!(got_two, "1+1 should evaluate to 2; after_anchor = {:?}", r.effect.after_anchor);
}

// ── SD-6: persistent-login LIVE 值接进 evaluate mutex 的端到端验证 ──────────────────────
//
// 证明 `EngineConfig.evaluate_persistent_login = true` 真的穿透到 `EvaluateGate.persistent_login
// = true`，使互斥生效（full_power + persistent_login → Blocked）。
//   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(sd6_evaluate)'

/// **SD-6：persistent_login LIVE = true + full_power = true → evaluate Blocked（互斥）**。
/// 证端到端：EngineConfig.evaluate_persistent_login 到达 EvaluateGate.persistent_login，
/// 使互斥逻辑真正生效（不再是占位 false）。
#[tokio::test]
#[ignore = "需本机 chrome：SD-6 persistent-login mutex 端到端"]
async fn sd6_evaluate_blocked_when_persistent_login_live_true() {
    let backend =
        common::build_backend_for_fixture_persistent_login("sd6-eval-mutex", true).await;
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate");
    // full_power=true + persistent_login=true → evaluate 应被互斥拦下 → Blocked
    let p = Progress::new(std::time::Duration::from_secs(20));
    let res = backend
        .act(&ActSpec::Evaluate { script: "1+1".into() }, &p)
        .await;
    eprintln!("evaluate (full_power+persistent_login) result = {res:?}");
    match res {
        Err(BrowserError::Blocked { reason }) => {
            let lc = reason.to_lowercase();
            assert!(
                lc.contains("persistent login") || lc.contains("mutually exclusive"),
                "Blocked reason must mention persistent-login mutex, got: {reason}"
            );
        }
        other => panic!(
            "evaluate with full_power+persistent_login must be Blocked (互斥), got {other:?}"
        ),
    }
}

/// **SD-6：persistent_login LIVE = false + full_power = true → evaluate allowed**。
/// 控制组：persistent_login off 时全权放行（互斥不触发）。
#[tokio::test]
#[ignore = "需本机 chrome：SD-6 控制组 persistent-login=false"]
async fn sd6_evaluate_allowed_when_persistent_login_live_false() {
    let backend =
        common::build_backend_for_fixture_persistent_login("sd6-eval-allow", false).await;
    backend
        .navigate(&common::fixture_url("iframe.html"), false)
        .await
        .expect("navigate");
    // full_power=true + persistent_login=false → evaluate 应放行
    let p = Progress::new(std::time::Duration::from_secs(20));
    let res = backend
        .act(&ActSpec::Evaluate { script: "1+1".into() }, &p)
        .await;
    eprintln!("evaluate (full_power, no persistent_login) result = {res:?}");
    let r = res.expect("evaluate must be allowed when persistent_login=false");
    assert!(r.success, "full-power evaluate should succeed");
}
