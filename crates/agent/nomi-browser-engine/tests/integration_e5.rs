//! **P2 E5：出口防火墙端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 `Fetch.enable` 拦截链路在**真请求路径**上生效（`spawn_fetch_firewall_loop`
//! 配合 `is_blocked_ip` 与跨域 POST 门控）。全程 `file://` fixture，无需服务器——fetch 到外部
//! host / 元数据 IP 即触发 `Fetch.requestPaused`。
//!
//! 1. **IP 封禁 enforcement**（`firewall_blocks_metadata_ip`）：fetch POST 到云元数据 IP
//!    `169.254.169.254`。防火墙命中 `is_blocked_ip` → `Fetch.failRequest{BlockedByClient}` → fetch
//!    **快速 reject**（而非离线超时挂起）。测试据「快速 reject」（远短于网络超时）证明是被防火墙阻断。
//! 2. **跨域 POST 拦截 + 预览不含值**（纯逻辑已在 `firewall::tests` 穷尽覆盖：`build_post_preview` /
//!    `decide` 的 `GatePost` 断言 host/size/字段名且**绝不含字段值**）；此处的 `#[ignore]` 集成只额外
//!    确认「跨域 POST fetch 经过拦截 handler 被处理（continue 放行而非永久挂起）」——即防火墙循环对
//!    跨域 POST 也触发了（离线无法连通，但请求不被永久卡住即证拦截链路活着）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=C:\Program Files\Google\Chrome\Application\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(firewall) | test(egress) | test(blocked_ip)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

use std::time::Duration;

use nomi_browser_engine::backend::CdpBackend;
use nomi_browser_engine::BrowserEngine;
use serde_json::Value;

mod common;

/// 轮询 `window.__e5.steps.<step>` 直到被 JS 填充（fetch promise settle），返回那一步的结果对象。
/// 超时返 `None`。用 `__eval_page_world_for_test`（by-value 读，await_promise=false）反复读 window 状态。
async fn poll_step(backend: &CdpBackend, step: &str, max_ms: u64) -> Option<Value> {
    let expr = format!("window.__e5 && window.__e5.steps && window.__e5.steps.{step} || null");
    let deadline = tokio::time::Instant::now() + Duration::from_millis(max_ms);
    loop {
        // by-value RemoteObject：{type, value}。取 value；非 null 即 JS 已填充。
        if let Ok(v) = backend.__eval_page_world_for_test(&expr).await
            && let Some(val) = v.get("value")
            && !val.is_null()
        {
            return Some(val.clone());
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// IP 封禁 enforcement：fetch POST 到云元数据 IP 169.254.169.254 被防火墙 failRequest →
/// fetch 快速 reject。证明 `Fetch.enable` 拦截 + `is_blocked_ip` 在真请求路径生效。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn firewall_blocks_metadata_ip_on_real_fetch() {
    let backend = common::build_backend_for_fixture("e5-blockip").await;

    backend
        .navigate(&common::fixture_url("firewall.html"), false)
        .await
        .expect("navigate firewall.html");

    // 触发 fetch（fire-and-forget；JS 把结果填进 window.__e5.steps.blockedIp）。
    backend
        .__eval_page_world_for_test("window.__e5BlockedIpFetch(); true")
        .await
        .expect("kick off blockedIp fetch");

    let result = poll_step(&backend, "blockedIp", 15_000)
        .await
        .expect("blockedIp fetch should settle within 15s (firewall should reject it fast)");
    eprintln!("blockedIp step = {result}");

    // 被防火墙 failRequest{BlockedByClient} → fetch reject。
    let rejected = result.get("rejected").and_then(Value::as_bool).unwrap_or(false);
    let ms = result.get("ms").and_then(Value::as_f64).unwrap_or(f64::MAX);
    eprintln!("blockedIp: rejected={rejected} ms={ms:.0}");

    assert!(
        rejected,
        "fetch to metadata IP 169.254.169.254 must be REJECTED by the egress firewall \
         (Fetch.failRequest), got {result}"
    );
    // 快速 reject（防火墙阻断）vs 慢超时（无防火墙时元数据 IP 离线会挂很久）。失败请求阶段拦截近乎
    // 瞬时——给 5s 上限（远短于 TCP connect 超时的数十秒），证明是防火墙而非超时。
    assert!(
        ms < 5_000.0,
        "blocked fetch should reject FAST (firewall failRequest), not slow-timeout; got {ms:.0}ms"
    );
}

/// 跨域 POST 拦截链路活着：fetch POST（带表单 body）到跨域 host 经过拦截 handler 被处理，请求不被
/// 永久挂起。**P3-D2 后**：默认（无 `EgressApprover` 注入）下被门控请求**fail-closed**（failRequest，
/// 闭合 P2 泄漏窗口）而非 E5 旧的 continue——但 fetch 仍在合理时间内 settle（被 fail 即 reject）。关键
/// 不变量：requestPaused 对跨域 POST 触发了且请求未永久卡住（无论 continue 还是 fail，都在有界时间内
/// 应答）。批准/拒绝/超时的裁决分支由 `egress_gated_post_*` D2 测试 + 纯逻辑单测覆盖。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn firewall_intercepts_cross_origin_post_without_hanging() {
    let backend = common::build_backend_for_fixture("e5-crosspost").await;

    backend
        .navigate(&common::fixture_url("firewall.html"), false)
        .await
        .expect("navigate firewall.html");

    backend
        .__eval_page_world_for_test("window.__e5CrossOriginPost(); true")
        .await
        .expect("kick off crossPost fetch");

    // 拦截 handler 对它做出裁决（D2 默认 fail-closed → failRequest → reject）后 settle——关键是它在
    // 有界时间内 settle（不永久挂起）。
    let result = poll_step(&backend, "crossPost", 15_000)
        .await
        .expect(
            "cross-origin POST should be processed by the firewall (D2: fail-closed by default) and \
             settle, not hang forever",
        );
    eprintln!("crossPost step = {result}");
    // ok==true 即该步 JS 跑完（不管 reject 与否）——证明请求经过了拦截链路且未被永久卡。
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    assert!(
        ok,
        "cross-origin POST fetch should be processed by the interception loop and settle, got {result}"
    );
}

/// **P3-G1 注入链端到端**：注入一个**与 default 不同**的 `FirewallConfig`（关掉跨域 POST 门控）→
/// build engine → 读回引擎持有的配置，断言**注入值真的到达了引擎**（而非被硬编码 `default()` 吞掉）。
///
/// 这是 G1「链路打通」的最小充分证据：default 与自定义在**外部可观测行为**上难以快速区分（跨域 POST
/// 离线下都会 settle），故直接读回引擎构造期注入的配置快照（[`CdpBackend::firewall_config_for_test`]，
/// 与 `spawn_fetch_firewall_loop` 消费的同一份值）断言注入生效。链路：
/// `build_backend_for_fixture_with_firewall` → `from_launched(.., firewall)` →
/// `spawn_fetch_firewall_loop(conn, firewall)`（不再硬编码 default）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn injected_firewall_config_reaches_backend() {
    use nomi_browser_engine::firewall::FirewallConfig;

    // 自定义：IP 封禁仍开（SSRF 防护恒应开），但**关掉**跨域 POST 门控——与 default 明显不同。
    let custom = FirewallConfig {
        block_private_ips: true,
        gate_cross_origin_post: false,
        ..Default::default()
    };
    assert_ne!(
        custom,
        FirewallConfig::default(),
        "test sentinel must differ from default to prove injection"
    );

    // P3-D1：FirewallConfig 不再 Copy（含 Vec 域名策略字段）→ clone 进 build 调用，后续仍可断言。
    let backend = common::build_backend_for_fixture_with_firewall("g1-inject", custom.clone()).await;

    // 读回引擎构造期注入的配置——必须等于我们注入的自定义值（证明 G1 链路把注入值透传到了引擎，
    // 而非沿用硬编码 default）。
    let seen = backend.firewall_config_for_test();
    assert_eq!(
        seen, custom,
        "injected FirewallConfig must reach the engine (G1 injection chain), got {seen:?}"
    );
    // 显式反证：读回值**不等于** default（若链路仍硬编码 default，此处会失败）。
    assert_ne!(
        seen,
        FirewallConfig::default(),
        "engine must NOT be using hardcoded FirewallConfig::default() — G1 injection chain broken"
    );
}

// ── P3-D2：GatePost 悬挂等审批（批准 continue / 拒绝 fail / 预览不含值）───────────────────
//
// 这些 #[ignore] 真 chrome 测试验「被门控的跨域 POST 在引擎层**悬挂**（不立即 settle），经注入的
// EgressApprover 取裁决后 continue/fail」。悬挂机制 + always_allow + fail-closed 的**纯逻辑**已在
// firewall::tests / handle_paused_request 的 spawn 逻辑覆盖；此处的集成在真请求路径上额外确认：
// (a) approver 真被调用且收到正确预览（host + 字段名，绝不含值）；(b) 据裁决 continue/fail。

use std::sync::{Arc, Mutex};

use nomi_browser_engine::firewall::{
    EgressApprover, EgressVerdict, FirewallConfig, HostResolver, PostPreview,
};

/// 记录型审批者：捕获收到的预览（验 approver 真被调用 + 预览形态），按构造时给定的裁决应答。
struct RecordingApprover {
    verdict: EgressVerdict,
    seen: Arc<Mutex<Vec<PostPreview>>>,
}

#[async_trait::async_trait]
impl EgressApprover for RecordingApprover {
    async fn approve_egress(&self, preview: &PostPreview) -> EgressVerdict {
        self.seen.lock().unwrap().push(preview.clone());
        self.verdict
    }
}

/// **Fake DNS resolver**（SD-1 测试隔离）：把任意 host 解析到一个固定 IP 列表——完全不碰真实网络。
/// 跨域 POST 的目标域要先过 DNS→IP SSRF 守卫才到 approver；用 fake 把探针域映射到**公网 IP**（守卫
/// 放行→抵 approver）或**私网 IP**（守卫 fail-closed→approver 之前就 Block）以精确验这一关键交互。
/// （此前 resolver 在 cdp.rs 硬编码 TokioResolver，离线伪域 NXDOMAIN 被守卫提前 fail-closed，approver
/// 永不被咨询——SD-1 落在 P3-D2 之后静默打破了这两个 approver 测试。）
struct FakeResolver {
    ips: Vec<std::net::IpAddr>,
}

#[async_trait::async_trait]
impl HostResolver for FakeResolver {
    async fn resolve(&self, _host: &str) -> std::io::Result<Vec<std::net::IpAddr>> {
        Ok(self.ips.clone())
    }
}

/// 把探针域映射到一个**公网 IP**（93.184.216.34 = example.com 真实 IP，非私网/回环/链路本地）的 fake
/// resolver → SSRF 守卫放行 → 跨域 POST 抵达 approver（生产里真实 exfil 目标正是解析到公网 IP）。
fn public_ip_resolver() -> Arc<dyn HostResolver> {
    Arc::new(FakeResolver {
        ips: vec!["93.184.216.34".parse().unwrap()],
    })
}

/// 批准（Continue）→ 被门控的跨域 POST 被 continueRequest 放行（离线下仍会因无网络 reject，但关键是
/// 经审批后被放行、approver 收到了正确预览）。预览 host 命中目标、含字段名、**绝不含字段值**（安全红线）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn egress_gated_post_approved_continues_and_approver_sees_redacted_preview() {
    let seen = Arc::new(Mutex::new(Vec::<PostPreview>::new()));
    let approver: Arc<dyn EgressApprover> = Arc::new(RecordingApprover {
        verdict: EgressVerdict::Continue,
        seen: seen.clone(),
    });
    let backend = common::build_backend_for_fixture_with_egress(
        "d2-approve",
        FirewallConfig::default(),
        Some(approver),
        Some(public_ip_resolver()),
    )
    .await;

    backend
        .navigate(&common::fixture_url("firewall.html"), false)
        .await
        .expect("navigate firewall.html");
    backend
        .__eval_page_world_for_test("window.__e5CrossOriginPost(); true")
        .await
        .expect("kick off crossPost fetch");

    // 悬挂 → 审批（Continue）→ continueRequest → fetch settle（离线 reject）。批准后请求被放行，
    // 故在有界时间内 settle。
    let result = poll_step(&backend, "crossPost", 15_000)
        .await
        .expect("approved cross-origin POST should be released by the approver and settle");
    assert!(result.get("ok").and_then(Value::as_bool).unwrap_or(false), "step ran: {result}");

    // approver 真被调用且收到正确预览（host = 跨域目标），且预览**绝不含字段值**（hunter2/alice）。
    let previews = seen.lock().unwrap().clone();
    assert!(!previews.is_empty(), "approver must have been consulted for the gated cross-origin POST");
    let p = &previews[0];
    assert_eq!(p.host, "e5-cross-origin-probe.example.com", "preview host = cross-origin target");
    assert!(p.field_names.iter().any(|n| n == "username"), "preview should carry field NAMES: {p:?}");
    let serialized = serde_json::to_string(p).unwrap();
    assert!(!serialized.contains("hunter2"), "preview MUST NOT contain field VALUE: {serialized}");
    assert!(!serialized.contains("alice"), "preview MUST NOT contain field VALUE: {serialized}");
}

/// 拒绝（Fail）→ 被门控的跨域 POST 被 failRequest（fail-closed，泄漏窗口闭合）→ fetch reject。关键
/// 不变量：approver 返回 Fail 后请求被**阻断**（不放行），且请求在有界时间内 settle（不永久挂起）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn egress_gated_post_denied_fails_closed() {
    let seen = Arc::new(Mutex::new(Vec::<PostPreview>::new()));
    let approver: Arc<dyn EgressApprover> = Arc::new(RecordingApprover {
        verdict: EgressVerdict::Fail,
        seen: seen.clone(),
    });
    let backend = common::build_backend_for_fixture_with_egress(
        "d2-deny",
        FirewallConfig::default(),
        Some(approver),
        Some(public_ip_resolver()),
    )
    .await;

    backend
        .navigate(&common::fixture_url("firewall.html"), false)
        .await
        .expect("navigate firewall.html");
    backend
        .__eval_page_world_for_test("window.__e5CrossOriginPost(); true")
        .await
        .expect("kick off crossPost fetch");

    let result = poll_step(&backend, "crossPost", 15_000)
        .await
        .expect("denied cross-origin POST should be failed-closed and settle (reject), not hang");
    eprintln!("denied crossPost = {result}");
    // 被 failRequest → reject。fixture 在 reject 分支记 rejected:true（且 ok:true）。
    assert!(result.get("ok").and_then(Value::as_bool).unwrap_or(false), "step ran: {result}");
    assert!(
        result.get("rejected").and_then(Value::as_bool).unwrap_or(false),
        "a denied (fail-closed) cross-origin POST must be REJECTED (failRequest), not allowed: {result}"
    );
    assert!(!seen.lock().unwrap().is_empty(), "approver must have been consulted before failing closed");
}

/// **SSRF 守卫优先于 approver**（SD-1 × P3-D2 交互不变量，回归锁）：跨域 POST 的目标域解析到**私网 IP**
/// → DNS→IP SSRF 守卫在 approver **之前**就硬 Block（failRequest）→ approver **永不**被咨询。这是有意
/// 的安全次序——绝不让人去「批准」一个发往内网/元数据 IP 的出口（即便审批者会点同意也轮不到它）。
///
/// 此测试同时是上面两个 approver 测试静默回归的**根因锁**：SD-1 落在 P3-D2 之后，离线伪域走真实
/// DNS 返 NXDOMAIN → 守卫 fail-closed 提前 Block → approver 没被咨询。当时 resolver 在引擎里硬编码、
/// 测试无从注入,故这条「守卫先于 approver」的交互从未被覆盖,回归才会静默。现 resolver 可注入后补此锁。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn egress_gated_post_to_private_ip_is_ssrf_blocked_before_approver() {
    let seen = Arc::new(Mutex::new(Vec::<PostPreview>::new()));
    // 即便审批者会 Continue，私网目标也应在它之前被 SSRF 守卫拦下 → 它根本不该被咨询。
    let approver: Arc<dyn EgressApprover> = Arc::new(RecordingApprover {
        verdict: EgressVerdict::Continue,
        seen: seen.clone(),
    });
    // fake resolver：把探针域解析到**私网 IP**（10.0.0.5 ∈ RFC1918）→ check_dns_ssrf fail-closed Block。
    let resolver: Arc<dyn HostResolver> = Arc::new(FakeResolver {
        ips: vec!["10.0.0.5".parse().unwrap()],
    });
    let backend = common::build_backend_for_fixture_with_egress(
        "d2-ssrf-precedence",
        FirewallConfig::default(),
        Some(approver),
        Some(resolver),
    )
    .await;

    backend
        .navigate(&common::fixture_url("firewall.html"), false)
        .await
        .expect("navigate firewall.html");
    backend
        .__eval_page_world_for_test("window.__e5CrossOriginPost(); true")
        .await
        .expect("kick off crossPost fetch");

    let result = poll_step(&backend, "crossPost", 15_000)
        .await
        .expect("SSRF-blocked cross-origin POST should fail-closed and settle (reject), not hang");
    eprintln!("ssrf-precedence crossPost = {result}");
    assert!(result.get("ok").and_then(Value::as_bool).unwrap_or(false), "step ran: {result}");
    // 目标解析到私网 IP → SSRF 守卫 failRequest → fetch reject。
    assert!(
        result.get("rejected").and_then(Value::as_bool).unwrap_or(false),
        "a cross-origin POST whose target resolves to a PRIVATE IP must be SSRF-blocked (rejected): {result}"
    );
    // 核心不变量：守卫先于 approver——私网目标在 approver 之前被 Block，approver 永不被咨询。
    assert!(
        seen.lock().unwrap().is_empty(),
        "SSRF guard MUST block BEFORE consulting the approver (never ask a human to approve egress to a private IP)"
    );
}
