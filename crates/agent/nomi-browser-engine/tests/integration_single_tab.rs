//! **单受控标签验证**（`#[ignore]`，本机/打包 chrome）：证明 `--no-startup-window` 消除了
//! 命令行冗余 about:blank 启动标签——启动后浏览器里**恰好一个** `type=="page"` 的 target
//! （引擎 `Target.createTarget` 出来的受控页），不再有命令行起始标签那个孤儿空白页。
//!
//! 旧行为 = 命令行 `about:blank` + createTarget 受控页 = **2** 个 page；
//! 新行为 = 仅 createTarget 受控页 = **1** 个 page。
//!
//! 精确跑 headless smoke（PowerShell，本机 Windows 有系统 Chrome）：
//!   $env:NOMIFUN_CHROME_BINARY='C:\Program Files\Google\Chrome\Application\chrome.exe'
//!   cargo test -p nomi-browser-engine --test integration_single_tab
//!     single_tab_headless_has_exactly_one_page_target -- --ignored --exact
//! headful 用例会开一个可见的真 Chrome 窗口（验 keep-alive 与前台 target 都生效）。
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

mod common;

use common::{
    build_backend_for_fixture_at_profile, build_backend_for_fixture_headful,
    build_backend_for_fixture_in_temp_profile, cleanup_temp_profile,
};
use nomi_browser_engine::BrowserEngine;
use nomi_browser_engine::profile::OWNERSHIP_MARKER_FILE;

/// headless：`--no-startup-window` + `--headless=new` 下,启动后恰好一个受控 page。
#[tokio::test]
#[ignore = "需本机 chrome：headless 启动后恰好一个受控 page（无命令行冗余 about:blank）"]
async fn single_tab_headless_has_exactly_one_page_target() {
    let (backend, profile_guard) =
        build_backend_for_fixture_in_temp_profile("single-tab-headless").await;
    let n = backend
        .page_target_count_for_test()
        .await
        .expect("Target.getTargets");
    assert_eq!(
        n, 1,
        "--no-startup-window 后应恰好一个受控 page（createTarget），无命令行 about:blank 孤儿，实得 {n}"
    );
    drop(backend);
    cleanup_temp_profile(profile_guard).await;
}

/// headful（关键风险路径）：`--no-startup-window` 下 chrome 被 REMOTE_DEBUGGING keep-alive 拴住、
/// **不无窗口自退**（launch_chrome 不报 "chrome exited before DevTools port"）+ 恰好一个受控 page
/// + 受控页可正常导航（active_target 链路不受影响）。会开一个可见 chrome 窗口。
#[tokio::test]
#[ignore = "需本机 chrome+显示器：headful --no-startup-window keep-alive + 单受控页 + navigate"]
async fn single_tab_headful_keepalive_one_page_and_navigates() {
    // build_backend_for_fixture_headful 内部 launch_chrome(cfg, force_headless=false)；若
    // --no-startup-window 在 headful 下无窗口自退,这一步就会 panic（"launch headful chrome"）——
    // 即评估唯一未经本机证实的风险点,跑通即证伪。
    let backend = build_backend_for_fixture_headful("single-tab-headful").await;

    let n = backend
        .page_target_count_for_test()
        .await
        .expect("Target.getTargets");
    assert_eq!(n, 1, "headful --no-startup-window 后应恰好一个受控 page，实得 {n}");

    // 受控页可正常导航（active_target 解引用链路健康）。
    let nav = backend
        .navigate("about:blank", false)
        .await
        .expect("navigate on the single controlled page should succeed");
    assert!(
        nav.final_url.contains("blank") || nav.final_url.contains("about"),
        "unexpected final_url after navigate: {}",
        nav.final_url
    );
}

#[tokio::test]
#[ignore = "需本机 chrome：standalone backend Drop 精确清进程与稳定 profile runtime artifacts"]
async fn standalone_backend_drop_cleans_stable_profile_and_allows_relaunch() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("standalone-stable-profile");
    let sentinel = profile.join("Default").join("stable-state.json");
    std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    std::fs::write(&sentinel, br#"{"keep":true}"#).unwrap();

    for round in 0..2 {
        let backend = build_backend_for_fixture_at_profile(profile.clone()).await;
        let marker_path = profile.join(OWNERSHIP_MARKER_FILE);
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
        let root_pid = marker["browser"]["pid"]
            .as_u64()
            .expect("ownership marker browser pid") as u32;
        assert!(marker_path.is_file());
        #[cfg(windows)]
        assert!(profile.join("DevToolsActivePort").is_file());

        drop(backend);

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
                let root_gone = system
                    .process(sysinfo::Pid::from_u32(root_pid))
                    .is_none();
                if root_gone
                    && !marker_path.exists()
                    && !profile.join("DevToolsActivePort").exists()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("standalone cleanup did not finish in round {round}"));
        assert_eq!(std::fs::read(&sentinel).unwrap(), br#"{"keep":true}"#);
    }
}

/// **父死自清（`--remote-debugging-pipe` keystone）**：控制进程一死,内核关闭继承的 fd3/fd4 →
/// Chromium 的 DevTools 管道读到 EOF → 浏览器**自行退出**。这是跨平台父死安全网的最优解（含
/// SIGKILL——内核关 fd 不需父进程跑任何代码,见 docs/superpowers/specs/browser-use/2026-06-19-
/// macos-pdeath-pipe-transport-design.md）。本测试在同进程内**隔离出该机制**：连上后关闭**我们这端
/// 的命令管道**（drop Connection → drop pipe Sender,chrome fd3 读到 EOF）但**不** kill child；用
/// `try_wait` 探测(不触发 `kill_on_drop`——child 句柄全程持有)断言 chrome 数秒内自退。
#[cfg(unix)]
#[tokio::test]
#[ignore = "需本机 chrome：验 --remote-debugging-pipe 下父死/管道 EOF → chrome 自退"]
async fn chrome_self_exits_when_command_pipe_closes() {
    use nomi_browser_engine::launch::{launch_chrome, LaunchConfig};
    use std::time::Duration;

    let chrome = nomi_browser_engine::acquire::resolve_chrome_path(
        &std::env::temp_dir().join("nomifun-browser-data"),
        None,
    )
    .await
    .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
    let cfg = LaunchConfig {
        chrome_path: chrome,
        user_data_dir: std::env::temp_dir().join("nomifun-pipe-selfexit-profile"),
        headful: false,
    };
    let launched = launch_chrome(&cfg, true).await.expect("launch chrome (pipe)");
    // 持有不可拆分的精确清理 guard，隔离验证「管道 EOF 致自退」机制本身。
    let (mut process, conn) = launched.connect().await.expect("connect over pipe");
    // 确认管道双向可用（命令发得出、回包收得到）后再测关闭。
    conn.enable_auto_attach().await.expect("auto attach over pipe");
    assert!(
        process.child_mut().try_wait().expect("try_wait").is_none(),
        "chrome should be running before we close the pipe"
    );

    // 模拟父死：关闭我们这端的命令管道（drop Connection → drop Sender,chrome fd3 读到 EOF）,不 kill。
    drop(conn);

    // chrome 应在数秒内自退。try_wait 探测(不 kill、不阻塞)；kill_on_drop 因 child 仍被持有而未触发。
    let mut exited = false;
    for _ in 0..100 {
        if process.child_mut().try_wait().expect("try_wait").is_some() {
            exited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        exited,
        "chrome 必须在命令管道关闭后自退（--remote-debugging-pipe 的父死自清,SIGKILL 等价）"
    );
}
