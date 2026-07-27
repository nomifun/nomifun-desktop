//! 托管启动 Chromium：经 [`nomi_process_runtime::ChildProcessBuilder`] spawn 解析到的 chrome，传随机
//! 调试端口（`--remote-debugging-port=0`，OS 分配）+ **专属 user-data-dir**（红线：永不
//! 碰用户 profile）+ [`crate::switches::chromium_switches`] 全量硬化开关，然后**轮询
//! `<user-data-dir>/DevToolsActivePort`** 拿到实际端口与 browser ws 路径，拼出
//! `ws://127.0.0.1:<port><path>` 交给 [`crate::transport::Connection`] connect。
//!
//! 为何读 DevToolsActivePort 而非 HTTP `/json/version`：免一次 HTTP（无需 `trust_env(false)`
//! 绕代理）、无需解析 JSON、且是 chrome 端口就绪的**权威信号**（文件出现即端口在监听）。
//!
//! 进程托管：`Builder::spawn_with_cleanup` 同时返回 direct-child handle 与三平台整树
//! cleanup proof（Windows Job / Unix watchdog）。生命周期 owner 必须同时持有二者，并且只有
//! direct child 已回收且 cleanup proof 完成后，才能报告 Chromium 已停止。
//!
//! headless 决策：[`crate::display::display_available`] 为 false（无显示器：无头 server /
//! CI / SSH 无 X）→ 强制 `--headless=new`。headful 时给 `--window-position`（非主屏角）+
//! `--window-size`，避免遮主屏。

use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

use crate::engine::BrowserError;

/// 轮询 DevToolsActivePort 文件的最长等待（chrome 冷启 + 端口监听就绪）。仅 Windows ws 路径用。
#[cfg(windows)]
const PORT_FILE_TIMEOUT: Duration = Duration::from_secs(30);
/// 轮询间隔。仅 Windows ws 路径用。
#[cfg(windows)]
const PORT_FILE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 托管启动配置。`resolve_chrome_path`（Task 6）得到的可执行 + 专属数据目录 + headful。
#[derive(Clone)]
pub struct LaunchConfig {
    /// chrome 可执行绝对路径（来自 [`crate::acquire::resolve_chrome_path`]）。
    pub chrome_path: PathBuf,
    /// **专属** user-data-dir（红线：绝不指向用户真实 profile）。launch 会确保其存在。
    pub user_data_dir: PathBuf,
    /// 是否带可见窗口。注意：`display_available()==false` 时本标志被忽略，强制 headless。
    pub headful: bool,
}

impl std::fmt::Debug for LaunchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchConfig")
            .field("chrome_path_configured", &true)
            .field("user_data_dir_configured", &true)
            .field("headful", &self.headful)
            .finish()
    }
}

/// 一次成功启动的产物：托管的 child handle（保活=保证退出清理）+ CDP 连接运输。
pub struct Launched {
    /// Chromium direct child + exact whole-tree cleanup proof.
    pub child: nomi_process_runtime::ManagedChildProcess,
    /// CDP 连接运输（Unix=管道 / Windows=ws url）。
    pub transport: LaunchTransport,
}

impl Launched {
    pub(crate) fn into_managed(self) -> (nomi_process_runtime::ManagedChildProcess, LaunchTransport) {
        (self.child, self.transport)
    }
}

/// Force-stop a launched browser through its single authoritative lifecycle
/// operation. A failed or cancelled attempt leaves the same exact authority
/// available for a later retry.
pub(crate) async fn terminate_launched_process_tree(
    process: &mut nomi_process_runtime::ManagedChildProcess,
) -> Result<(), BrowserError> {
    process.shutdown().await.map_err(|error| {
        tracing::warn!(
            target: "nomi_browser_engine::launch",
            error_kind = ?error.kind(),
            "managed Chromium process-tree cleanup could not be proven"
        );
        BrowserError::Other("managed Chromium process-tree cleanup could not be proven".into())
    })
}

fn launch_error_after_cleanup(
    primary: BrowserError,
    cleanup: Result<(), BrowserError>,
) -> BrowserError {
    if cleanup.is_ok() {
        primary
    } else {
        BrowserError::Other(
            "browser launch failed and process-tree cleanup could not be proven".into(),
        )
    }
}

/// CDP 连接运输。**Unix 生产用 `--remote-debugging-pipe`**（fd3/fd4；浏览器在本进程死亡——含
/// SIGKILL——时,内核关闭继承的 fd → Chromium 自行退出,跨平台父死自清的最优解,见
/// docs/superpowers/specs/browser-use/2026-06-19-macos-pdeath-pipe-transport-design.md）。
/// **Windows 生产用 ws url**（port + DevToolsActivePort + Job Object 清理；pipe 在 Windows 走继承
/// HANDLE,复杂且 Job Object 已内核级清理,故不转）。
pub enum LaunchTransport {
    /// Unix `--remote-debugging-pipe`：`cmd_writer`=我们写命令的管道写端（chrome 在 fd3 读）,
    /// `resp_reader`=我们读响应的管道读端（chrome 在 fd4 写）。交给 [`crate::transport::Connection::connect_pipe`]。
    #[cfg(unix)]
    Pipe {
        cmd_writer: std::os::fd::OwnedFd,
        resp_reader: std::os::fd::OwnedFd,
    },
    /// `ws://127.0.0.1:<port>/devtools/browser/<uuid>`，交给 [`crate::transport::Connection::connect`]。
    Ws { ws_url: String },
}

/// 构造 chrome 启动参数（纯函数，便于单测）。
///
/// - CDP 运输开关：Unix=`--remote-debugging-pipe`（fd3/fd4,浏览器父死自退）；Windows=
///   `--remote-debugging-port=0`（OS 分配 + DevToolsActivePort）。
/// - `--user-data-dir=<dir>`：专属数据目录（红线：非用户 profile）。
/// - [`crate::switches::chromium_switches`] 全量静态硬化开关。
/// - `--no-first-run` / `--no-default-browser-check`：免首启向导/默认浏览器询问。
/// - `--headless=new`：仅当 `force_headless`（无显示器或显式 headless）。
/// - headful（`!force_headless`）：`--window-position` + `--window-size`（非主屏角）。
/// - `--no-startup-window`：不自动开启动窗口（消除冗余 about:blank；受控页由 backend
///   `Target.createTarget` 单独建）。靠 `--remote-debugging-port` 触发的 REMOTE_DEBUGGING
///   keep-alive 保进程存活、不无窗口自退。
///
/// `force_headless` 由调用方按 `display_available()` 与 `LaunchConfig::headful` 算好后传入，
/// 使本函数保持纯逻辑、无平台/环境探测，单测可在任意宿主断言。
pub fn build_chrome_args(user_data_dir: &Path, force_headless: bool) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // CDP 运输开关：Unix 用 `--remote-debugging-pipe`（fd3/fd4；浏览器在父死/管道 EOF 时自退,
    // 免疫 SIGKILL,见设计文档）;Windows 用 `--remote-debugging-port=0`（OS 分配 + DevToolsActivePort）。
    #[cfg(unix)]
    args.push("--remote-debugging-pipe".into());
    #[cfg(windows)]
    args.push("--remote-debugging-port=0".into());

    args.push(format!("--user-data-dir={}", user_data_dir.display()));

    // 静态硬化基线（零后台出站 / 容器防崩 / 截图可复现；Linux 含 dev-shm）。
    args.extend(crate::switches::chromium_switches());

    args.push("--no-first-run".into());
    args.push("--no-default-browser-check".into());

    if force_headless {
        // 无显示器强制无头；`=new` 是现代 headless（非旧 --headless），CDP 截图可用。
        args.push("--headless=new".into());
    } else {
        // headful：摆到非主屏角、给定窗口尺寸，避免遮挡主屏中心。
        args.push("--window-position=80,80".into());
        args.push("--window-size=1280,800".into());
    }

    // Linux 容器内 sandbox 常因缺 user-namespace 而启动失败；回退 --no-sandbox。
    // TODO(verify-linux): 容器 sandbox 探测/回退需实机核对（当前为无条件回退，偏保守），
    // 见 docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
    #[cfg(target_os = "linux")]
    args.push("--no-sandbox".into());

    // **不自动开启动窗口/标签**：消除冗余的命令行起始标签——受控页由 backend
    // `Target.createTarget("about:blank")` 单独建（[`crate::backend::cdp`]），命令行再开一个就是
    // 多余的孤儿空白标签。改用 `--no-startup-window` 让 chrome 启动时不开任何窗口/标签。
    //
    // 为何不会因「无窗口」自退、也不影响 launch 轮询：本函数恒传 `--remote-debugging-port`
    // （上面），命中 Chromium 的 keep-alive 受支持组合——`(kNoStartupWindow || kHeadless) &&
    // (kRemoteDebuggingPort || kRemoteDebuggingPipe)` → `ScopedKeepAlive(REMOTE_DEBUGGING)`
    // 拴住进程直到显式 `Browser.close`（见 chrome/browser/devtools/chrome_devtools_manager_
    // delegate.cc）；且 DevToolsActivePort 在 socket bind 成功即写、与有无 window 无关（见
    // content/browser/devtools/devtools_http_handler.cc）→ launch_chrome 的端口轮询不受影响。
    // 平台无关的 Chromium 通用开关（keep-alive 逻辑同源、仅排除 ChromeOS，本仓不支持）。
    // TODO(verify-macos/linux): mac/linux 真机各冒烟一次确认（本机仅 Windows 已验），见
    // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
    args.push("--no-startup-window".into());

    args
}

/// 解析 DevToolsActivePort 文件内容 → `(port, ws_path)`。
///
/// chrome 在 `--remote-debugging-port=0` 下把实际监听信息写进
/// `<user-data-dir>/DevToolsActivePort`：
///   - 第 1 行：端口号（如 `54213`）；
///   - 第 2 行：browser ws 路径（如 `/devtools/browser/4f1c-...`）。
///
/// 返回 `Err(Other)` 给出明确诊断（行数不足 / 端口非数字）；不 panic。
pub fn parse_devtools_active_port(content: &str) -> Result<(u16, String), BrowserError> {
    let mut lines = content.lines();
    let port_line = lines
        .next()
        .ok_or_else(|| BrowserError::Other("DevToolsActivePort empty (no port line)".into()))?;
    let ws_path = lines
        .next()
        .ok_or_else(|| BrowserError::Other("DevToolsActivePort missing ws-path line".into()))?;

    // Never include either line in the error. The second line is a
    // browser-scoped WebSocket path and may contain a secret token; the first
    // line is caller-controlled file content and is not useful to a caller.
    let port: u16 = port_line
        .trim()
        .parse()
        .map_err(|_| BrowserError::Other("DevToolsActivePort contained an invalid port".into()))?;
    if port == 0 {
        return Err(BrowserError::Other(
            "DevToolsActivePort reported port 0 (not yet bound)".into(),
        ));
    }

    let ws_path = ws_path.trim().to_string();
    if !ws_path.starts_with('/') {
        return Err(BrowserError::Other(
            "DevToolsActivePort contained an invalid browser path".into(),
        ));
    }
    Ok((port, ws_path))
}

/// 由端口 + ws 路径拼出 browser ws url（loopback v4）。
pub fn build_ws_url(port: u16, ws_path: &str) -> String {
    format!("ws://127.0.0.1:{port}{ws_path}")
}

fn safe_profile_prepare_error() -> BrowserError {
    BrowserError::Other("browser launch could not prepare its profile".into())
}

fn safe_profile_ownership_error() -> BrowserError {
    BrowserError::Other("browser launch ownership preflight failed".into())
}

fn safe_chromium_spawn_error() -> BrowserError {
    BrowserError::Other("browser launch could not start Chromium".into())
}

fn safe_devtools_timeout_error() -> BrowserError {
    BrowserError::Other("browser launch timed out waiting for DevToolsActivePort".into())
}

/// Keep the Chromium test escape hatch on an exact allowlist.
///
/// Release builds do not read `NOMI_CHROME_EXTRA_ARGS` at all. In debug
/// builds, only the switches required by the OOPIF fixture are accepted.
/// An allowlist is intentional here: Chromium has many aliases and related
/// switches that can change profile ownership, CDP exposure, extensions,
/// sandboxing, or other security-sensitive behavior.
fn filtered_extra_chrome_args(extra: &str) -> Vec<String> {
    #[cfg(debug_assertions)]
    {
        const OOPIF_HOST_RESOLVER_RULES: &str =
            "--host-resolver-rules=MAP *.nomitest 127.0.0.1";

        extra
            .lines()
            .map(str::trim)
            .filter(|arg| !arg.is_empty())
            .filter_map(|arg| {
                if arg == OOPIF_HOST_RESOLVER_RULES || arg == "--site-per-process" {
                    Some(arg.to_owned())
                } else {
                    // Never echo a rejected value: it may contain a path,
                    // endpoint, or another secret supplied by the caller.
                    tracing::warn!(
                        target: "nomi_browser_engine::launch",
                        "ignored unsupported NOMI_CHROME_EXTRA_ARGS entry"
                    );
                    None
                }
            })
            .collect()
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = extra;
        Vec::new()
    }
}

/// 托管启动 chrome 并返回 child + CDP 连接运输。
///
/// 流程：确保 user-data-dir 存在 → scrub 脏 profile → 清 stale Singleton → 起 chrome。
/// **Unix**：`--remote-debugging-pipe`,经 fd3/fd4 即时连（无端口轮询；浏览器在父死/管道 EOF 时
/// 自退）;**Windows**：`--remote-debugging-port=0` + 轮询 DevToolsActivePort 拿端口/ws 路径。
/// `force_headless` 由调用方按 display 算好。
pub async fn launch_chrome(
    config: &LaunchConfig,
    force_headless: bool,
) -> Result<Launched, BrowserError> {
    // user-data-dir 必须存在（专属目录；红线已在 config 构造处保证非用户 profile）。
    std::fs::create_dir_all(&config.user_data_dir).map_err(|_| safe_profile_prepare_error())?;

    // Ownership must be resolved before touching Preferences, Singleton files,
    // or any other profile state. A live owner or in-progress recovery makes
    // the whole launch fail closed.
    let ownership_claim =
        crate::profile::prepare_ownership_marker_for_launch(&config.user_data_dir).map_err(
            |_| safe_profile_ownership_error(),
        )?;

    // **脏 profile 根治（keystone）**：上次 chrome 必被硬杀（kill_on_drop / Job Object / app 同步
    // exit），profile.exit_type 停在 "Crashed" → 下次启动弹「未正确关闭 / 恢复页面?」气泡 + 跑会话
    // 恢复（异常启动路径更易崩）。spawn 前（chrome 此刻必未运行）best-effort 洗回 "Normal"，是覆盖
    // 所有退出路径（含 crash/断电）的唯一可靠层。见 crate::profile 模块文档。
    if let Err(e) = crate::profile::scrub_crash_markers(&config.user_data_dir) {
        tracing::warn!(
            target: "nomi_browser_engine::launch",
            error_kind = ?e.kind(),
            "profile crash-marker scrub failed (best-effort; launch continues)"
        );
    }
    // mac/linux：顺手清 stale Singleton* 三件套（Windows 因 FILE_FLAG_DELETE_ON_CLOSE 无需）。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    crate::profile::clear_stale_singleton(&config.user_data_dir);

    let mut args = build_chrome_args(&config.user_data_dir, force_headless);

    // The environment escape hatch is compiled out of release builds.
    // Debug builds still use the exact allowlist above; arbitrary Chromium
    // switches must never be able to replace the managed profile or CDP
    // transport, load extensions, or weaken sandbox/security settings.
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("NOMI_CHROME_EXTRA_ARGS") {
        args.extend(filtered_extra_chrome_args(&extra));
    }

    #[cfg(unix)]
    {
        launch_chrome_pipe(config, &args, &ownership_claim).await
    }
    #[cfg(windows)]
    {
        match launch_chrome_ws(config, &args, &ownership_claim).await {
            Ok(v) => Ok(v),
            Err(first) if should_retry_with_startup_page(&first, &args) => {
                tracing::warn!(
                    target: "nomi_browser_engine::launch",
                    error = %first,
                    "chrome exited before DevTools port was ready; retrying with an explicit startup page"
                );
                crate::profile::prepare_ownership_marker_for_retry(
                    &config.user_data_dir,
                    &ownership_claim,
                )
                .map_err(|_| {
                    BrowserError::Other(
                        "browser launch ownership preflight failed before retry".into(),
                    )
                })?;
                let fallback_args = chrome_args_with_startup_page(&args);
                launch_chrome_ws(config, &fallback_args, &ownership_claim)
                    .await
                    .map_err(|_| {
                        BrowserError::Other(
                            "browser launch retry with startup page failed".into(),
                        )
                    })
            }
            Err(e) => Err(e),
        }
    }
}

/// **Unix**：`--remote-debugging-pipe` 启动。建两条匿名管道,经 [`nomi_process_runtime::ChildProcessBuilder::inherit_fds`]
/// 把 chrome 端装到 fd3（读命令）/fd4（写响应）；我们持另两端交 [`crate::transport::Connection::connect_pipe`]。
/// 无端口轮询——管道即时可用,且浏览器在父死/管道 EOF 时自退（免疫 SIGKILL）。
#[cfg(unix)]
async fn launch_chrome_pipe(
    config: &LaunchConfig,
    args: &[String],
    ownership_claim: &crate::profile::ProfileLaunchClaim,
) -> Result<Launched, BrowserError> {
    // pipe_in：父写命令 → chrome 读（fd3）。pipe_out：chrome 写响应（fd4）→ 父读。
    let (chrome_cmd_read, our_cmd_write) = make_pipe()?;
    let (our_resp_read, chrome_resp_write) = make_pipe()?;

    let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&config.chrome_path);
    builder
        .args(args)
        // chrome 的 stdout/stderr 我们不消费；null 掉避免污染父进程控制台。
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // chrome `--remote-debugging-pipe`：fd3 读命令、fd4 写响应。
        .inherit_fds(vec![(3, chrome_cmd_read), (4, chrome_resp_write)]);

    let mut process = builder.spawn_managed().map_err(|e| {
        tracing::debug!(
            target: "nomi_browser_engine::launch",
            error_kind = ?e.kind(),
            "managed Chromium spawn failed"
        );
        safe_chromium_spawn_error()
    })?;
    commit_browser_ownership(config, ownership_claim, &mut process).await?;

    // 快速失败：给 chrome 一小会儿；若立即退出（坏开关 / 缺依赖）立即报错,不必等首条 CDP 命令超时。
    tokio::time::sleep(Duration::from_millis(120)).await;
    if let Ok(Some(status)) = process.child_mut().try_wait() {
        let primary = BrowserError::Other(format!(
            "chrome exited immediately after spawn (bad flags / missing deps?) status {status}"
        ));
        let cleanup = terminate_launched_process_tree(&mut process).await;
        return Err(launch_error_after_cleanup(primary, cleanup));
    }

    Ok(Launched {
        child: process,
        transport: LaunchTransport::Pipe {
            cmd_writer: our_cmd_write,
            resp_reader: our_resp_read,
        },
    })
}

/// (unix) 建一条匿名管道 → `(读端, 写端)`,两端都设 `FD_CLOEXEC`。chrome 端经 Builder 的 dup2
/// shuffle 在 fd3/4 上清掉 CLOEXEC 以 survive exec；我们这端保持 CLOEXEC,绝不漏进 chrome 或其它 spawn。
#[cfg(unix)]
fn make_pipe() -> Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd), BrowserError> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe(2) 成功时向数组写入恰好两个新建 owned fd。
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(BrowserError::Other(format!(
            "pipe(2): {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: pipe(2) 刚返回两个独占 fd,所有权移交 OwnedFd（drop 即 close）。
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    set_cloexec(&read)?;
    set_cloexec(&write)?;
    Ok((read, write))
}

#[cfg(unix)]
fn set_cloexec(fd: &std::os::fd::OwnedFd) -> Result<(), BrowserError> {
    use std::os::fd::AsRawFd;
    let raw = fd.as_raw_fd();
    // SAFETY: F_GETFD/F_SETFD 在一个 owned fd 上,无前置条件。
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFD) };
    if flags < 0 {
        return Err(BrowserError::Other(format!(
            "fcntl F_GETFD: {}",
            std::io::Error::last_os_error()
        )));
    }
    if unsafe { libc::fcntl(raw, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(BrowserError::Other(format!(
            "fcntl F_SETFD: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// **Windows**：`--remote-debugging-port=0` 启动,轮询 DevToolsActivePort 拿端口 + ws 路径,拼 ws url。
#[cfg(windows)]
async fn launch_chrome_ws(
    config: &LaunchConfig,
    args: &[String],
    ownership_claim: &crate::profile::ProfileLaunchClaim,
) -> Result<Launched, BrowserError> {
    // 删旧 DevToolsActivePort：复用目录时避免轮询读到上次启动的陈旧端口/路径。
    let port_file = config.user_data_dir.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);

    let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&config.chrome_path);
    builder
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut process = builder.spawn_managed().map_err(|e| {
        tracing::debug!(
            target: "nomi_browser_engine::launch",
            error_kind = ?e.kind(),
            "managed Chromium spawn failed"
        );
        safe_chromium_spawn_error()
    })?;
    commit_browser_ownership(config, ownership_claim, &mut process).await?;

    // 轮询 DevToolsActivePort 直到出现且可解析，或 child 提前退出，或超时。
    let deadline = Instant::now() + PORT_FILE_TIMEOUT;
    loop {
        if let Ok(Some(status)) = process.child_mut().try_wait() {
            let primary = BrowserError::Other(format!(
                "chrome exited before DevTools port was ready (status {status})"
            ));
            let cleanup = terminate_launched_process_tree(&mut process).await;
            return Err(launch_error_after_cleanup(primary, cleanup));
        }
        if let Ok(content) = std::fs::read_to_string(&port_file) {
            if let Ok((port, ws_path)) = parse_devtools_active_port(&content) {
                let ws_url = build_ws_url(port, &ws_path);
                return Ok(Launched {
                    child: process,
                    transport: LaunchTransport::Ws { ws_url },
                });
            }
        }
        if Instant::now() >= deadline {
            let cleanup = terminate_launched_process_tree(&mut process).await;
            return Err(launch_error_after_cleanup(
                safe_devtools_timeout_error(),
                cleanup,
            ));
        }
        tokio::time::sleep(PORT_FILE_POLL_INTERVAL).await;
    }
}

async fn commit_browser_ownership(
    config: &LaunchConfig,
    ownership_claim: &crate::profile::ProfileLaunchClaim,
    process: &mut nomi_process_runtime::ManagedChildProcess,
) -> Result<(), BrowserError> {
    let Some(_) = process.id() else {
        let cleanup = terminate_launched_process_tree(process).await;
        return Err(launch_error_after_cleanup(
            BrowserError::Other("spawned browser exited before ownership commit".into()),
            cleanup,
        ));
    };
    if crate::profile::write_browser_ownership_marker(
        ownership_claim,
        &config.user_data_dir,
        &config.chrome_path,
        process.child(),
    )
    .await
    .is_err()
    {
        tracing::warn!(
            target: "nomi_browser_engine::launch",
            "browser ownership marker commit failed; terminating the unowned process tree"
        );
        let cleanup = terminate_launched_process_tree(process).await;
        return Err(launch_error_after_cleanup(
            BrowserError::Other("browser ownership commit failed".into()),
            cleanup,
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn should_retry_with_startup_page(error: &BrowserError, args: &[String]) -> bool {
    args.iter().any(|a| a == "--no-startup-window")
        && matches!(
            error,
            BrowserError::Other(message)
                if message.contains("chrome exited before DevTools port was ready")
        )
}

#[cfg(windows)]
fn chrome_args_with_startup_page(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = args
        .iter()
        .filter(|a| a.as_str() != "--no-startup-window")
        .cloned()
        .collect();
    out.push("about:blank".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_include_port_user_data_dir_and_hardening() {
        let dir = Path::new("/tmp/nomi-udd");
        let args = build_chrome_args(dir, false);

        // 运输开关随平台：Unix=--remote-debugging-pipe（fd3/fd4 自死），Windows=--remote-debugging-port=0。
        #[cfg(unix)]
        assert!(
            args.iter().any(|a| a == "--remote-debugging-pipe"),
            "missing --remote-debugging-pipe flag: {args:?}"
        );
        #[cfg(windows)]
        assert!(
            args.iter().any(|a| a == "--remote-debugging-port=0"),
            "missing --remote-debugging-port=0 flag: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "--user-data-dir=/tmp/nomi-udd"),
            "missing user-data-dir: {args:?}"
        );
        // 硬化基线关键项必须透传。
        assert!(args.iter().any(|a| a == "--disable-background-networking"));
        assert!(args.iter().any(|a| a == "--disable-component-update"));
        assert!(args.iter().any(|a| a.starts_with("--disable-features=")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
        assert!(args.iter().any(|a| a == "--no-default-browser-check"));
        // 不自动开启动窗口（消除冗余命令行 about:blank；受控页由 backend createTarget 建）。
        assert!(args.iter().any(|a| a == "--no-startup-window"));
        assert!(
            !args.iter().any(|a| a == "about:blank"),
            "命令行不应再带 about:blank 起始页（受控页由 createTarget 建）: {args:?}"
        );
    }

    #[test]
    fn extra_chrome_args_are_fail_closed_for_security_sensitive_switches() {
        let extra = [
            "--user-data-dir=C:\\attacker\\profile",
            "--profile-directory=Default",
            "--remote-debugging-port=9222",
            "--remote-debugging-address=0.0.0.0",
            "--remote-debugging-pipe",
            "--load-extension=C:\\attacker\\extension",
            "--disable-extensions-except=C:\\attacker\\extension",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--disable-web-security",
            "--allow-running-insecure-content",
            "--disable-features=IsolateOrigins",
            "--enable-features=NetworkServiceInProcess",
            "--proxy-server=http://attacker.invalid:8080",
            "--host-resolver-rules=MAP * 0.0.0.0",
            "https://attacker.invalid",
            "--site-per-process",
            "--host-resolver-rules=MAP *.nomitest 127.0.0.1",
        ]
        .join("\n");

        let filtered = filtered_extra_chrome_args(&extra);

        #[cfg(debug_assertions)]
        assert_eq!(
            filtered,
            vec![
                "--site-per-process".to_string(),
                "--host-resolver-rules=MAP *.nomitest 127.0.0.1".to_string(),
            ]
        );

        #[cfg(not(debug_assertions))]
        assert!(
            filtered.is_empty(),
            "release builds must not accept ambient Chromium switches: {filtered:?}"
        );

        for rejected in [
            "--user-data-dir=",
            "--profile-directory=",
            "--remote-debugging-port=",
            "--remote-debugging-address=",
            "--remote-debugging-pipe",
            "--load-extension=",
            "--disable-extensions-except=",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--disable-web-security",
            "--allow-running-insecure-content",
            "--disable-features=",
            "--enable-features=",
            "--proxy-server=",
            "--host-resolver-rules=MAP * 0.0.0.0",
            "https://attacker.invalid",
        ] {
            assert!(
                !filtered.iter().any(|arg| arg == rejected),
                "sensitive or unapproved switch was accepted: {rejected}; got {filtered:?}"
            );
        }
    }

    #[test]
    fn extra_chrome_args_cannot_override_managed_profile_or_transport() {
        let managed_profile = Path::new("/managed/profile");
        let mut args = build_chrome_args(managed_profile, true);
        args.extend(filtered_extra_chrome_args(
            "--user-data-dir=/attacker/profile\n\
             --profile-directory=Attacker\n\
             --remote-debugging-port=9222\n\
             --remote-debugging-address=0.0.0.0\n\
             --load-extension=/attacker/extension\n\
             --no-sandbox\n\
             --disable-web-security\n\
             --site-per-process",
        ));

        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("--user-data-dir="))
                .count(),
            1,
            "managed profile must remain unique: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--user-data-dir=/attacker/")),
            "untrusted profile override must be absent: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--profile-directory=")),
            "profile-directory must not be caller-controlled: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--remote-debugging-port=9222")),
            "remote debugging port must remain managed: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--remote-debugging-address=")),
            "remote debugging bind address must remain managed: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--load-extension=")),
            "extensions must not be caller-controlled: {args:?}"
        );
        assert!(
            args.iter().all(|arg| arg != "--disable-web-security"),
            "security weakening switch must be absent: {args:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_retry_args_replace_no_startup_window_with_startup_page() {
        let args = vec![
            "--remote-debugging-port=0".to_string(),
            "--no-startup-window".to_string(),
            "--disable-background-networking".to_string(),
        ];
        let fallback = chrome_args_with_startup_page(&args);
        assert!(!fallback.iter().any(|a| a == "--no-startup-window"));
        assert!(fallback.iter().any(|a| a == "about:blank"));
        assert!(should_retry_with_startup_page(
            &BrowserError::Other(
                "chrome exited before DevTools port was ready (status exit code: 0)".into()
            ),
            &args
        ));
    }

    #[test]
    fn headless_flag_only_when_forced() {
        let dir = Path::new("/tmp/x");
        let headless = build_chrome_args(dir, true);
        assert!(
            headless.iter().any(|a| a == "--headless=new"),
            "force_headless must add --headless=new: {headless:?}"
        );
        // headless 时不该有 headful 的窗口摆位开关。
        assert!(!headless.iter().any(|a| a.starts_with("--window-position")));

        let headful = build_chrome_args(dir, false);
        assert!(
            !headful.iter().any(|a| a == "--headless=new"),
            "headful must NOT add --headless=new: {headful:?}"
        );
        // headful 时给窗口摆位/尺寸。
        assert!(headful.iter().any(|a| a.starts_with("--window-position")));
        assert!(headful.iter().any(|a| a.starts_with("--window-size")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_container_falls_back_to_no_sandbox() {
        // TODO(verify-linux): 当前无条件回退 --no-sandbox（偏保守）；容器探测见
        // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
        let args = build_chrome_args(Path::new("/tmp/x"), true);
        assert!(
            args.iter().any(|a| a == "--no-sandbox"),
            "linux must add --no-sandbox: {args:?}"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_has_no_no_sandbox() {
        let args = build_chrome_args(Path::new("/tmp/x"), true);
        assert!(!args.iter().any(|a| a == "--no-sandbox"));
    }

    #[test]
    fn parse_active_port_two_lines() {
        let content = "54213\n/devtools/browser/4f1c0a2b-aaaa-bbbb-cccc-ddddeeeeffff\n";
        let (port, path) = parse_devtools_active_port(content).unwrap();
        assert_eq!(port, 54213);
        assert_eq!(
            path,
            "/devtools/browser/4f1c0a2b-aaaa-bbbb-cccc-ddddeeeeffff"
        );
        assert_eq!(
            build_ws_url(port, &path),
            "ws://127.0.0.1:54213/devtools/browser/4f1c0a2b-aaaa-bbbb-cccc-ddddeeeeffff"
        );
    }

    #[test]
    fn parse_active_port_trims_whitespace() {
        // chrome 可能不带末尾换行；也容忍行内多余空白。
        let content = "  9333  \n  /devtools/browser/x  ";
        let (port, path) = parse_devtools_active_port(content).unwrap();
        assert_eq!(port, 9333);
        assert_eq!(path, "/devtools/browser/x");
    }

    #[test]
    fn parse_active_port_rejects_missing_lines() {
        assert!(parse_devtools_active_port("").is_err());
        assert!(parse_devtools_active_port("54213").is_err()); // 缺第二行
    }

    #[test]
    fn parse_active_port_rejects_bad_port() {
        assert!(parse_devtools_active_port("notaport\n/devtools/browser/x").is_err());
        assert!(parse_devtools_active_port("0\n/devtools/browser/x").is_err()); // 0=未绑定
    }

    #[test]
    fn active_port_parse_errors_do_not_echo_endpoint_material() {
        let private_port_line = "12345-private-port-sentinel";
        let error = parse_devtools_active_port(&format!(
            "{private_port_line}\n/devtools/browser/private-token"
        ))
        .unwrap_err()
        .to_string();
        assert!(!error.contains(private_port_line));
        assert!(!error.contains("private-token"));
        assert!(!error.contains("12345"));

        let private_ws_path = "ws://127.0.0.1:12345/devtools/browser/private-token";
        let error = parse_devtools_active_port(&format!("9333\n{private_ws_path}"))
            .unwrap_err()
            .to_string();
        assert!(!error.contains(private_ws_path));
        assert!(!error.contains("private-token"));
        assert!(!error.contains("12345"));
    }

    #[test]
    fn parse_active_port_rejects_non_absolute_ws_path() {
        assert!(parse_devtools_active_port("9333\ndevtools/browser/x").is_err());
    }

    #[test]
    fn launch_boundary_errors_do_not_echo_private_paths_or_endpoints() {
        let profile_path = r"C:\secret\profile";
        let chrome_path = r"C:\secret\Chrome\chrome.exe";
        let ws_endpoint = "ws://127.0.0.1:12345/devtools/browser/private-token";
        let errors = [
            safe_profile_prepare_error(),
            safe_profile_ownership_error(),
            safe_chromium_spawn_error(),
            safe_devtools_timeout_error(),
        ];

        for error in errors {
            let display = error.to_string();
            assert!(!display.contains(profile_path));
            assert!(!display.contains(chrome_path));
            assert!(!display.contains(ws_endpoint));
            assert!(!display.contains("private-token"));
            assert!(!display.contains("12345"));
        }
    }

    #[test]
    fn launch_config_debug_does_not_echo_private_paths() {
        let chrome_path = "LAUNCH-CHROMIUM-PATH-SENTINEL";
        let profile_path = "LAUNCH-PROFILE-PATH-SENTINEL";
        let config = LaunchConfig {
            chrome_path: PathBuf::from(chrome_path),
            user_data_dir: PathBuf::from(profile_path),
            headful: true,
        };

        let debug = format!("{config:?}");
        assert!(!debug.contains(chrome_path), "{debug}");
        assert!(!debug.contains(profile_path), "{debug}");
        assert!(debug.contains("chrome_path_configured"));
        assert!(debug.contains("user_data_dir_configured"));
        assert!(debug.contains("headful: true"));
    }

    #[tokio::test]
    async fn profile_prepare_failure_does_not_echo_the_profile_or_executable_path() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("private-profile-sentinel");
        std::fs::write(&profile, b"not a directory").unwrap();
        let chrome = temp.path().join("private-chrome-sentinel");
        let config = LaunchConfig {
            chrome_path: chrome.clone(),
            user_data_dir: profile.clone(),
            headful: false,
        };

        let error = match launch_chrome(&config, true).await {
            Ok(_) => panic!("a regular file cannot be used as a browser profile"),
            Err(error) => error.to_string(),
        };
        assert!(!error.contains(&profile.display().to_string()));
        assert!(!error.contains(&chrome.display().to_string()));
        assert!(!error.contains("private-profile-sentinel"));
        assert!(!error.contains("private-chrome-sentinel"));
    }

    #[tokio::test]
    async fn spawn_failure_does_not_echo_the_profile_or_executable_path() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("private-profile-sentinel");
        let chrome = temp.path().join("private-chrome-sentinel");
        let config = LaunchConfig {
            chrome_path: chrome.clone(),
            user_data_dir: profile.clone(),
            headful: false,
        };

        let error = match launch_chrome(&config, true).await {
            Ok(_) => panic!("a nonexistent Chromium executable cannot launch"),
            Err(error) => error.to_string(),
        };
        assert!(!error.contains(&profile.display().to_string()));
        assert!(!error.contains(&chrome.display().to_string()));
        assert!(!error.contains("private-profile-sentinel"));
        assert!(!error.contains("private-chrome-sentinel"));
    }
}
