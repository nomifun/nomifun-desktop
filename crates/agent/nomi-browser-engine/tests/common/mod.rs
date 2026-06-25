//! observe 集成测试共享接线（`#[ignore]`，本机/打包 chrome）。
//!
//! 任务 5 的契约母本（launch→connect→run_attach_loop→enable_auto_attach→createTarget→arm→
//! call）已被复制过一次；任务 6 起所有 observe 集成测试**复用**本模块的 helper，勿再复制母本。
//!
//! 与契约母本（`observe_fixtures.rs::observe_inject_contract_iframe`）的关系：那个测试手动接线
//! 注入侧的 `incrementalAriaSnapshot` 单帧调用，验证**注入契约**（aria 形态快照）；本模块走更高层
//! ——直接建一个 [`CdpBackend`] 并 navigate 到 fixture，让被测对象就是 `engine.observe()` 全链
//! （逐帧缝合 + 脱敏 + 代际翻新 ref 表）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(observe_)'
//! 首跑写 `.snap.new`；`cargo insta accept`（或手动改名 .snap）接受为基线。
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

#![allow(dead_code)] // 不同集成测试文件用到的 helper 子集不同；未用项不报警。

use std::sync::Arc;

use nomi_browser_engine::backend::CdpBackend;
use nomi_browser_engine::firewall::{EgressApprover, FirewallConfig, HostResolver};
use nomi_browser_engine::launch::{launch_chrome, LaunchConfig};

/// fixture 的 file:// URL。`file://` + 单个前导斜杠 + POSIX 路径：`CARGO_MANIFEST_DIR`
/// 在 unix 是 `/abs`（已带前导斜杠）、在 windows 是 `C:/abs`（需补一个），故仅在缺失时补斜杠。
/// 旧写法 `file:///{manifest}` 在 unix 上产生**四**斜杠（`file:////Users/...`）——chrome 容忍但
/// 回报时归一成三斜杠，导致 navigate 的 redirect 判定（请求 url != final url）误报 redirected。
pub fn fixture_url(name: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    let abs = if manifest.starts_with('/') {
        manifest
    } else {
        format!("/{manifest}")
    };
    format!("file://{abs}/tests/fixtures/{name}")
}

/// 解析 chrome（env NOMIFUN_CHROME_BINARY > 打包 > 数据目录 > 下载兜底）+ 托管 launch（headless）
/// + 自建 transport connect + flatten auto-attach + 取一个 page session，建一个 [`CdpBackend`]。
///
/// `profile`：本测试专属 user-data-dir 后缀，避免并发测试争用同一 profile。
/// 返回的 backend 持有 chrome 进程句柄——Drop 即清理整棵进程树（kill_on_drop）；测试结束自然回收。
///
/// 不挂下载沙箱（`download_dir=None`）——下载相关集成测试用
/// [`build_backend_for_fixture_with_downloads`]。evaluate 全权 OFF（默认 default-deny）。
/// 防火墙用 `FirewallConfig::default()`（IP 封禁开 + 跨域 POST 门控开 = 现行为）。
pub async fn build_backend_for_fixture(profile: &str) -> CdpBackend {
    build_backend_for_fixture_inner(profile, None, false, false, FirewallConfig::default(), None, None, None)
        .await
}

/// 同 [`build_backend_for_fixture`]，但 **evaluate 全权模式 ON**（`EngineConfig.evaluate_full_power
/// = true` 的等价 test seam）——F1-sec 验「全权 LIVE 开 → act(Evaluate) 放行」。
pub async fn build_backend_for_fixture_full_power(profile: &str) -> CdpBackend {
    build_backend_for_fixture_inner(profile, None, true, false, FirewallConfig::default(), None, None, None)
        .await
}

/// **SD-6 persistent-login mutex 验证 test seam**：`evaluate_full_power = true` +
/// `evaluate_persistent_login` = 调用方指定。用于验证 persistent-login LIVE 值穿透到
/// `EvaluateGate.persistent_login`——`persistent_login=true` 时互斥生效（full_power+persistent
/// → Blocked）；`persistent_login=false` 时全权正常放行（控制组）。
pub async fn build_backend_for_fixture_persistent_login(
    profile: &str,
    persistent_login: bool,
) -> CdpBackend {
    build_backend_for_fixture_inner(
        profile,
        None,
        true,  // evaluate_full_power = ON (to test the mutex)
        persistent_login,
        FirewallConfig::default(),
        None,
        None,
        None,
    )
    .await
}

/// **W4d 持久登录验证 test seam**：同 [`build_backend_for_fixture`]，但额外注入
/// `storage_state`（上层从 vault `load_storage_state` 解出的登录态 JSON）——引擎在 page
/// 建好后 `restore_cookies`（+`restore_local_storage`）**启动注入**灌登录态。配合 W4d 集成测试验
/// 「跨引擎/会话持久登录」（会话 A capture+save vault → 会话 B 新引擎 load vault → 此 seam 注入 →
/// 登录态恢复存活）。`storage_state=None` 走零注入（现行为）。
///
/// Always runs on the default browser context (no per-pet isolation).
pub async fn build_backend_for_fixture_with_storage_state(
    profile: &str,
    storage_state: Option<serde_json::Value>,
) -> CdpBackend {
    build_backend_for_fixture_inner(
        profile,
        None,
        false,
        false,
        FirewallConfig::default(),
        None,
        storage_state,
        None,
    )
    .await
}

/// **P3-G1 注入链验证 test seam**：同 [`build_backend_for_fixture`]，但注入一个**自定义**
/// [`FirewallConfig`]（而非 `default()`）。配合 [`CdpBackend::firewall_config_for_test`] 断言
/// 「自定义配置经 from_launched 真的注入到引擎、未被硬编码 default 吞掉」。
pub async fn build_backend_for_fixture_with_firewall(
    profile: &str,
    firewall: FirewallConfig,
) -> CdpBackend {
    build_backend_for_fixture_inner(profile, None, false, false, firewall, None, None, None).await
}

/// **P3-D2 验证 test seam**：注入自定义 [`FirewallConfig`] **+ 一个出口审批通道**
/// （[`EgressApprover`]）。配合跨域 POST fixture 验「被门控请求悬挂等裁决 → 批准 continue / 拒绝
/// fail / 超时 / 无通道 fail-closed」。`egress_approver=None` 走引擎 fail-closed 默认（验泄漏窗口闭合）。
///
/// **SD-1 交互**：`dns_resolver` 注入式——跨域 POST 的目标域要先过 DNS→IP SSRF 守卫才到 approver。
/// 测试须注入一个把探针域映射到**公网 IP** 的 fake resolver，否则真实 DNS 对离线伪域返 NXDOMAIN →
/// 守卫 fail-closed 在 approver **之前**就 Block（approver 永不被咨询）。`None` = 默认 TokioResolver。
pub async fn build_backend_for_fixture_with_egress(
    profile: &str,
    firewall: FirewallConfig,
    egress_approver: Option<Arc<dyn EgressApprover>>,
    dns_resolver: Option<Arc<dyn HostResolver>>,
) -> CdpBackend {
    build_backend_for_fixture_inner(profile, None, false, false, firewall, egress_approver, None, dns_resolver).await
}

/// 同 [`build_backend_for_fixture`]，但挂 E4 下载沙箱（`Browser.setDownloadBehavior(allowAndName)`
/// 落 `download_dir` + 下载事件循环打 MOTW）。返回 `(backend, download_dir)`——下载测试用 download_dir
/// 校验落盘文件 + Zone.Identifier ADS。
pub async fn build_backend_for_fixture_with_downloads(
    profile: &str,
) -> (CdpBackend, std::path::PathBuf) {
    let download_dir = std::env::temp_dir().join(format!("nomifun-dl-{profile}-downloads"));
    // 干净起点：清旧目录残留（上次跑的 GUID 文件），再建。
    let _ = std::fs::remove_dir_all(&download_dir);
    std::fs::create_dir_all(&download_dir).expect("create download dir");
    let backend = build_backend_for_fixture_inner(
        profile,
        Some(download_dir.to_string_lossy().into_owned()),
        false,
        false,
        FirewallConfig::default(),
        None,
        None,
        None,
    )
    .await;
    (backend, download_dir)
}

/// **单标签验证 test seam（headful）**：同 [`build_backend_for_fixture`] 但 **headful**
/// （`force_headless=false`）——验 `--no-startup-window` 在 headful 下被 REMOTE_DEBUGGING
/// keep-alive 拴住（chrome 不无窗口自退、launch 不报 "exited before DevTools port"）+ 恰好
/// 一个受控 page。会开一个**可见** chrome 窗口（需本机有显示器；本机 Windows 有）。
pub async fn build_backend_for_fixture_headful(profile: &str) -> CdpBackend {
    let chrome = nomi_browser_engine::acquire::resolve_chrome_path(
        &std::env::temp_dir().join("nomifun-browser-data"),
        None,
    )
    .await
    .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
    let cfg = LaunchConfig {
        chrome_path: chrome,
        user_data_dir: std::env::temp_dir().join(format!("nomifun-observe-{profile}-profile")),
        headful: true,
    };
    // force_headless=false → headful（带可见窗口）。这是 --no-startup-window keep-alive 的风险路径。
    let launched = launch_chrome(&cfg, false).await.expect("launch headful chrome");
    CdpBackend::from_launched(
        launched,
        true, // headful
        true, // display_available
        None,
        None, // workspace_dir: headful fixture 不上传
        false,
        false, // evaluate_persistent_login
        FirewallConfig::default(),
        None,
        None,
        nomi_browser_engine::KnownSecretValues::default(),
        None, // dns_resolver: 默认 TokioResolver
    )
    .await
    .expect("build CdpBackend (headful)")
}

/// 同 [`build_backend_for_fixture_headful`] 但注入自定义 [`FirewallConfig`]。OOPIF 验证用：
/// **headful = 真浏览器进程模型**,site-isolation 才会把跨站 iframe 起成**独立渲染进程（OOPIF）**;
/// `--headless=new` 是单渲染进程,不起 OOPIF。配合 `NOMIFUN_CHROME_BINARY` + 测试侧 `NOMI_CHROME_EXTRA_ARGS`
/// （--host-resolver-rules/--site-per-process）使用。会开一个**可见** chrome 窗口（需本机有显示器）。
pub async fn build_backend_for_fixture_headful_with_firewall(
    profile: &str,
    firewall: FirewallConfig,
) -> CdpBackend {
    let chrome = nomi_browser_engine::acquire::resolve_chrome_path(
        &std::env::temp_dir().join("nomifun-browser-data"),
        None,
    )
    .await
    .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
    let cfg = LaunchConfig {
        chrome_path: chrome,
        user_data_dir: std::env::temp_dir().join(format!("nomifun-observe-{profile}-profile")),
        headful: true,
    };
    let launched = launch_chrome(&cfg, false).await.expect("launch headful chrome");
    CdpBackend::from_launched(launched, true, true, None, None, false, false, firewall, None, None, nomi_browser_engine::KnownSecretValues::default(), None)
        .await
        .expect("build CdpBackend (headful+firewall)")
}

/// **Headful + 下载沙箱 test seam**（Task 6 / `TODO(verify-headful-printToPDF)`）：同
/// [`build_backend_for_fixture_headful`]（headful，真可见窗口）但额外挂 E4 下载沙箱
/// （`download_dir`）。专供 **save_as_pdf 的 headful 校验**——验 `Page.printToPDF` 在 headful
/// Chrome 下是否仍产非空 PDF（headful 历史上对 printToPDF 有限制）。返回 `(backend, download_dir)`。
/// 会开一个**可见** chrome 窗口（需本机有显示器）。
pub async fn build_backend_for_fixture_headful_with_downloads(
    profile: &str,
) -> (CdpBackend, std::path::PathBuf) {
    let download_dir =
        std::env::temp_dir().join(format!("nomifun-dl-{profile}-headful-downloads"));
    // 干净起点：清旧目录残留，再建。
    let _ = std::fs::remove_dir_all(&download_dir);
    std::fs::create_dir_all(&download_dir).expect("create download dir");
    let chrome = nomi_browser_engine::acquire::resolve_chrome_path(
        &std::env::temp_dir().join("nomifun-browser-data"),
        None,
    )
    .await
    .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
    let cfg = LaunchConfig {
        chrome_path: chrome,
        user_data_dir: std::env::temp_dir().join(format!("nomifun-observe-{profile}-profile")),
        headful: true,
    };
    let launched = launch_chrome(&cfg, false).await.expect("launch headful chrome");
    let backend = CdpBackend::from_launched(
        launched,
        true, // headful
        true, // display_available
        Some(download_dir.to_string_lossy().into_owned()),
        None, // workspace_dir：save_as_pdf 不依赖上传沙箱
        false,
        false, // evaluate_persistent_login
        FirewallConfig::default(),
        None,
        None,
        nomi_browser_engine::KnownSecretValues::default(),
        None, // dns_resolver: 默认 TokioResolver
    )
    .await
    .expect("build CdpBackend (headful+downloads)");
    (backend, download_dir)
}

// 构造参数随安全配置增长（download/full_power/persistent/firewall/egress/storage/dns_resolver）；
// 与 build_backend / from_launched 同源的已知取舍（见 cdp.rs 的 too_many_arguments allow + EngineRuntimeParams TODO）。
#[allow(clippy::too_many_arguments)]
async fn build_backend_for_fixture_inner(
    profile: &str,
    download_dir: Option<String>,
    evaluate_full_power: bool,
    evaluate_persistent_login: bool,
    firewall: FirewallConfig,
    egress_approver: Option<Arc<dyn EgressApprover>>,
    storage_state: Option<serde_json::Value>,
    dns_resolver: Option<Arc<dyn HostResolver>>,
) -> CdpBackend {
    let chrome = nomi_browser_engine::acquire::resolve_chrome_path(
        &std::env::temp_dir().join("nomifun-browser-data"),
        None,
    )
    .await
    .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
    let cfg = LaunchConfig {
        chrome_path: chrome,
        user_data_dir: std::env::temp_dir().join(format!("nomifun-observe-{profile}-profile")),
        headful: false,
    };
    let launched = launch_chrome(&cfg, true).await.expect("launch chrome");
    // SD-2：fixture 测试默认给 temp_dir 作为 workspace（上传沙箱需要一个根；实际产品环境是 per-pet
    // workspace，此处用 temp_dir 让 temp 下创建的测试文件都能通过沙箱校验）。
    let workspace_dir = Some(std::env::temp_dir());
    // headful=false / display=false：observe 不依赖显示，capabilities 字段对集成断言无关紧要。
    CdpBackend::from_launched(
        launched,
        false,
        false,
        download_dir,
        workspace_dir,
        evaluate_full_power,
        evaluate_persistent_login,
        firewall,
        egress_approver,
        storage_state,
        nomi_browser_engine::KnownSecretValues::default(),
        dns_resolver,
    )
    .await
    .expect("build CdpBackend")
}
