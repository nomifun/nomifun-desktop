// Prevent an extra console window on Windows in release builds. Debug builds
// keep the console so backend `tracing` logs are visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `nomifun-desktop` — the Tauri shell (replaces the Electron shell).
//!
//! Core idea (the whole point of this rewrite): there is NO spawned backend
//! binary. The unified Rust backend (`nomifun-app`, ex-`nomicore`) is linked
//! into THIS process and started in-process on a localhost port. The webview
//! loads the bundled SPA (`ui/dist`) and talks to `http://127.0.0.1:<port>/api`
//! exactly as it does today — so the renderer's ~295 HTTP calls are unchanged.
//!
//! ┌── nomifun-desktop (this process) ──────────────────────────┐
//! │  Tauri shell (window/tray/dialog/deep-link/updater)         │
//! │  └─ tokio task: nomifun_app embedded axum on 127.0.0.1:<p>  │
//! │  WebView2/WKWebView/WebKitGTK ── HTTP ──▶ 127.0.0.1:<p>/api │
//! └────────────────────────────────────────────────────────────┘

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use clap::Parser;
use nomifun_app::{DesktopServer, WebUiAsset, WebUiAssetSource, WebUiStatus};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

mod memory_panel_window;
mod companion_pointer;
mod updater_install_context;

/// Private process-handoff variable carrying the desktop shell's already
/// resolved data directory across `tauri-plugin-process` relaunches.
///
/// `NOMIFUN_DATA_DIR` cannot serve this purpose: for compatibility, the
/// desktop interprets an externally supplied value as a parent and appends
/// `/Nomi`, while the embedded backend later exports the effective directory
/// back through that same variable for child services. A relaunched desktop
/// inherits the latter value and would append `/Nomi` a second time.
const DESKTOP_EFFECTIVE_DATA_DIR_ENV: &str =
    "NOMIFUN_DESKTOP_EFFECTIVE_DATA_DIR";

/// Build the webview initialization script. Injects the loopback backend port
/// (`window.__backendPort`), the OS tag, and the per-boot local-trust secret
/// (`window.__nomiLocalTrust`). Also installs `fetch` AND `XMLHttpRequest`
/// interceptors that attach the trust header to EVERY request bound for the
/// backend — so any code path (httpBridge, configService, raw `fetch`/XHR,
/// multipart uploads with progress events, …) is trusted without per-call
/// instrumentation, while requests to external origins are untouched (the
/// secret never leaks off-box). Runs before any page script.
pub(crate) fn webui_init_script(port: u16, trust_secret: &str) -> String {
    // `{:?}` emits properly quoted/escaped JS string literals.
    format!(
        r#"window.__backendPort = {port}; window.__os = {os:?}; window.__nomiLocalTrust = {secret:?};
(function () {{
  var secret = {secret:?};
  var origin = "http://127.0.0.1:" + {port};
  if (!secret) return;
  function isBackend(url) {{
    url = url || "";
    return url.indexOf(origin) === 0 || url.charAt(0) === "/";
  }}
  if (window.fetch && !window.__nomiFetchPatched) {{
    window.__nomiFetchPatched = true;
    var origFetch = window.fetch.bind(window);
    window.fetch = function (input, init) {{
      try {{
        var url = typeof input === "string" ? input : (input && input.url) || "";
        if (isBackend(url)) {{
          init = init || {{}};
          var h = new Headers((init && init.headers) || undefined);
          if (!h.has("x-nomi-local-trust")) h.set("x-nomi-local-trust", secret);
          init.headers = h;
          if (typeof input !== "string") input = url;
        }}
      }} catch (e) {{}}
      return origFetch(input, init);
    }};
  }}
  var XHR = window.XMLHttpRequest;
  if (XHR && XHR.prototype && !window.__nomiXhrPatched) {{
    window.__nomiXhrPatched = true;
    var proto = XHR.prototype;
    var origOpen = proto.open;
    var origSetHeader = proto.setRequestHeader;
    var origSend = proto.send;
    proto.open = function (method, url) {{
      this.__nomiUrl = url;
      return origOpen.apply(this, arguments);
    }};
    proto.setRequestHeader = function (name) {{
      try {{ (this.__nomiHeaders || (this.__nomiHeaders = {{}}))[String(name).toLowerCase()] = true; }} catch (e) {{}}
      return origSetHeader.apply(this, arguments);
    }};
    proto.send = function () {{
      try {{
        if (isBackend(this.__nomiUrl) && !(this.__nomiHeaders && this.__nomiHeaders["x-nomi-local-trust"])) {{
          this.setRequestHeader("x-nomi-local-trust", secret);
        }}
      }} catch (e) {{}}
      return origSend.apply(this, arguments);
    }};
  }}
}})();"#,
        os = std::env::consts::OS,
        secret = trust_secret,
        port = port,
    )
}

/// Resolve an optional, non-empty `NOMIFUN_WEBUI_DIST` override once. Empty
/// environment values are treated as unset so they cannot accidentally disable
/// the canonical embedded frontend.
fn configured_webui_dist_override() -> Option<PathBuf> {
    normalize_webui_dist_override(std::env::var_os("NOMIFUN_WEBUI_DIST"))
}

fn normalize_webui_dist_override(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Resolve the bundled SPA directory (`ui/dist`) served to remote browsers by
/// the LAN listener. Checks an already-normalized explicit override, then
/// resource-dir layouts (production bundle), then dev-tree relatives. A
/// candidate must contain `index.html` to be accepted; `None` means remote
/// browsers get the API only (logged as a warning).
fn validate_webui_spa_candidate(path: &std::path::Path, production: bool) -> anyhow::Result<bool> {
    validate_webui_spa_candidate_with_build_id(
        path,
        production,
        option_env!("NOMIFUN_FRONTEND_BUILD_ID"),
    )
}

fn validate_webui_spa_candidate_with_build_id(
    path: &std::path::Path,
    production: bool,
    expected_build_id: Option<&str>,
) -> anyhow::Result<bool> {
    if !path.join("index.html").is_file() {
        return Ok(false);
    }
    if production {
        let expected_build_id = expected_build_id.context(
            "this production desktop host has no exact frontend build identity; run `bun run build:ui` and rebuild the desktop application",
        )?;
        nomifun_app::bootstrap::validate_webui_dist(
            path,
            env!("CARGO_PKG_VERSION"),
            Some(expected_build_id),
        )?;
    }
    Ok(true)
}

fn resolve_webui_spa_dir(
    app: &tauri::App,
    explicit_override: Option<&Path>,
) -> anyhow::Result<Option<PathBuf>> {
    let production = !tauri::is_dev();
    if let Some(path) = explicit_override {
        if validate_webui_spa_candidate(path, production)? {
            return Ok(Some(path.to_path_buf()));
        }
        anyhow::bail!(
            "NOMIFUN_WEBUI_DIST={} does not contain a valid WebUI app shell",
            path.display()
        );
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("webui-dist"));
        candidates.push(res.join("dist"));
        candidates.push(res.join("ui").join("dist"));
        // Tauri encodes `..` segments of a resource path as `_up_`.
        candidates.push(res.join("_up_").join("_up_").join("ui").join("dist"));
    }
    candidates.push(PathBuf::from("ui/dist"));
    candidates.push(PathBuf::from("../../ui/dist"));
    candidates.push(PathBuf::from("../ui/dist"));
    for candidate in candidates {
        if validate_webui_spa_candidate(&candidate, production)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

/// Snapshot Tauri's production asset resolver into the backend's host-agnostic
/// WebUI source. A custom-protocol build already embeds `frontendDist` in the
/// executable, so this is available to release bundles *and* `tauri build
/// --debug --no-bundle` without relying on a platform-specific resource path.
///
/// Every asset is resolved exactly once here. The immutable snapshot uses
/// ref-counted bytes, bounds decompression work to startup, and lets the Tauri
/// resolver (which owns an AppManager reference) drop before the backend is put
/// back into Tauri managed state.
///
/// Development deliberately returns `None`: the LAN listener proxies to the
/// same live Vite server as the desktop webview instead.
fn resolve_embedded_webui_assets<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> anyhow::Result<Option<WebUiAssetSource>> {
    if tauri::is_dev() {
        return Ok(None);
    }

    let resolver = app.asset_resolver();
    let asset_keys = resolver
        .iter()
        .map(|(key, _)| key.into_owned())
        .collect::<Vec<_>>();
    if !asset_keys
        .iter()
        .any(|key| key.trim_start_matches('/') == "index.html")
    {
        return Ok(None);
    }

    // Use the LAN listener's HTTP scheme explicitly. During setup no webview
    // exists yet, so AssetResolver::get() would infer HTTPS from all(empty).
    let mut assets = Vec::with_capacity(asset_keys.len());
    let mut manifest_bytes = None;
    let mut uncompressed_bytes = 0usize;
    for key in asset_keys {
        let key = key.trim_start_matches('/').to_owned();
        let asset = resolver
            .get_for_scheme(format!("/{key}"), false)
            .with_context(|| format!("failed to resolve embedded WebUI asset {key}"))?;
        let asset =
            WebUiAsset::new(asset.bytes, asset.mime_type).with_csp_header(asset.csp_header);
        validate_webui_snapshot_asset(&key, &asset)?;
        uncompressed_bytes = uncompressed_bytes.saturating_add(asset.bytes.len());
        if key == nomifun_app::bootstrap::UI_BUILD_MANIFEST_FILE {
            manifest_bytes = Some(asset.bytes.clone());
        }
        assets.push((key, asset));
    }

    // Keep the exact host/frontend pairing invariant for embedded assets too.
    // The manifest is selected from the exact embedded key set, so a missing
    // manifest cannot be hidden by Tauri's normal index.html SPA fallback.
    let expected_build_id = option_env!("NOMIFUN_FRONTEND_BUILD_ID").context(
        "this production desktop host has no exact frontend build identity; run `bun run build:ui` and rebuild the desktop application",
    )?;
    let manifest_bytes = manifest_bytes.context("embedded WebUI build manifest is missing")?;
    nomifun_app::bootstrap::validate_webui_manifest_bytes(
        &manifest_bytes,
        "embedded frontendDist/nomifun-build.json",
        env!("CARGO_PKG_VERSION"),
        Some(expected_build_id),
    )?;

    tracing::info!(
        asset_count = assets.len(),
        uncompressed_bytes,
        "snapshotted embedded WebUI assets"
    );
    Ok(Some(WebUiAssetSource::new(assets)))
}

fn validate_webui_snapshot_asset(key: &str, asset: &WebUiAsset) -> anyhow::Result<()> {
    anyhow::ensure!(
        !asset
            .csp_header
            .as_deref()
            .is_some_and(|value| value.contains("'nonce-")),
        "embedded WebUI asset {key} uses a per-response CSP nonce, which cannot be safely reused from an immutable snapshot; configure a hash-only CSP or disable CSP nonce injection"
    );
    Ok(())
}

fn should_resolve_filesystem_webui(
    explicit_dist_override: bool,
    has_dev_frontend: bool,
    has_embedded_frontend: bool,
) -> bool {
    !has_dev_frontend && (explicit_dist_override || !has_embedded_frontend)
}

/// Data root resolution, in priority order:
///
/// 1. The private effective-path handoff inherited from a patched desktop
///    process relaunch.
/// 2. `NOMIFUN_DATA_DIR` env — explicit override; the shell appends `/Nomi`
///    (semantics unchanged since the Electron era). The sole compatibility
///    exception is a canonical path that already proves a complete v3 data
///    root, as exported by an older embedded backend before an update relaunch.
/// 3. The shared per-channel default from `nomifun_app::cli::default_data_dir()`:
///    stable builds use the `Nomi` leaf; non-stable builds use a suffixed
///    sibling such as `Nomi-dev`. The web host and the `nomicore` bin resolve
///    to the same directory when built for the same channel, while dev state
///    remains isolated from the installed stable app. The historic system-temp
///    stable location remains an extreme fallback (see `relocate.rs`).
fn default_data_dir() -> PathBuf {
    resolve_desktop_data_dir(
        std::env::var_os(DESKTOP_EFFECTIVE_DATA_DIR_ENV),
        std::env::var_os("NOMIFUN_DATA_DIR"),
    )
}

fn resolve_desktop_data_dir(
    inherited_effective_dir: Option<std::ffi::OsString>,
    external_parent_override: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(dir) = inherited_effective_dir.filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = external_parent_override {
        let dir = PathBuf::from(dir);
        // The first launch after upgrading from a build without the private
        // handoff variable still inherits the effective path that its embedded
        // backend exported through NOMIFUN_DATA_DIR. Recognize only a
        // canonical, real directory whose complete v3 receipt/generation/db
        // tuple proves that it is already a data root. Anything missing,
        // malformed, linked, or merely named "Nomi" keeps the historical
        // external parent + /Nomi behavior.
        if is_proven_inherited_effective_data_root(&dir) {
            return dir;
        }
        return dir.join("Nomi");
    }
    nomifun_app::cli::default_data_dir()
}

fn is_proven_inherited_effective_data_root(candidate: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(candidate) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    let Ok(canonical) = std::fs::canonicalize(candidate) else {
        return false;
    };
    if canonical != candidate {
        return false;
    }

    // Passing the data root itself as the probe work root deliberately accepts
    // WorkRootMismatch: both statuses are reached only after the receipt,
    // UUIDv7 storage generation, and regular database file have all validated.
    // This keeps upgrades working when the user selected a distinct work root.
    matches!(
        nomifun_common::factory_reset::inspect_v3_dataset_receipt(
            candidate, candidate,
        ),
        Ok(
            nomifun_common::factory_reset::DatasetReceiptStatus::Current
                | nomifun_common::factory_reset::DatasetReceiptStatus::WorkRootMismatch
        )
    )
}

/// Publish the resolved directory before Tauri or the backend starts threads.
/// Tauri's process-plugin relaunch inherits this private value, so the next
/// desktop process can distinguish an effective path from the public
/// parent-directory override without guessing from the path's basename.
fn publish_effective_data_dir_for_relaunch(data_dir: &Path) {
    // SAFETY: called at the start of `main`, before runtime initialization or
    // Tauri/backend thread creation.
    unsafe {
        std::env::set_var(DESKTOP_EFFECTIVE_DATA_DIR_ENV, data_dir);
    }
}

/// Updater scaffold: ask the configured update endpoint whether a newer signed
/// release is available. Invoked from the renderer via
/// `invoke("check_for_updates")`. Returns the new version string, or `null` if
/// up to date. Inert until `plugins.updater.endpoints` in tauri.conf.json serves
/// a valid `latest.json` signed with the project key
/// (see apps/desktop/updater/README.md).
#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(update.version)),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Desired desktop-companion window, one per companion (multi-companion, spec §4.6).
#[derive(serde::Deserialize)]
struct CompanionWindowSpec {
    companion_id: String,
    enabled: bool,
}

type MainThreadTask = Box<dyn FnOnce() + Send + 'static>;

pub(crate) async fn run_on_main_thread_task<D, F>(dispatch: D, work: F) -> Result<(), String>
where
    D: FnOnce(MainThreadTask) -> Result<(), String>,
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    dispatch(Box::new(move || {
        let _ = tx.send(work());
    }))?;
    rx.await
        .map_err(|_| "main-thread task did not return a result".to_string())?
}

// ---- WebUI / LAN remote-access lifecycle commands -------------------------
// The embedded backend always serves the app's own webview on loopback. These
// commands toggle the SEPARATE on-demand LAN listener (`0.0.0.0`) so remote
// browsers on the same network can reach the app by IP (with login). They are
// async so they run on Tauri's async runtime — never blocking the main thread.

/// Current WebUI/LAN serving status (running, port, LAN IP, URL).
///
/// Async because it resolves the persisted admin identity fresh from the DB
/// (via `status_snapshot`), so the panel shows the real username / password-set
/// state even when the LAN listener is stopped (e.g. right after a restart).
#[tauri::command]
async fn webui_get_status(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<WebUiStatus, String> {
    let server = server.inner().clone();
    Ok(server.status_snapshot().await)
}

/// Start LAN serving (bind `0.0.0.0:25808`, fallback port if taken).
#[tauri::command]
async fn webui_start(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<WebUiStatus, String> {
    let server = server.inner().clone();
    Ok(server.start_lan().await)
}

/// Stop LAN serving (the loopback listener / desktop webview are unaffected).
#[tauri::command]
async fn webui_stop(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<WebUiStatus, String> {
    let server = server.inner().clone();
    Ok(server.stop_lan().await)
}

/// 持有当前生效的系统防休眠 assertion;`None`=允许休眠。
/// Drop `KeepAwake` 即释放 assertion,所以进程退出/关闭开关都能干净恢复正常电源行为。
/// Managed state holding the active OS sleep-inhibitor assertion (None = sleep allowed).
struct AwakeState(Mutex<Option<keepawake::KeepAwake>>);

/// 获取"保持唤醒"的 OS assertion:仅阻止系统空闲休眠(PreventUserIdleSystemSleep),
/// **不**阻止显示器空闲关闭 —— 等价 `caffeinate -i`(而非 `-di`)。电脑保持活动时屏幕仍可正常熄屏,
/// 既省电也避免长时间常亮对屏幕(尤其 OLED)的损耗;熄屏不影响定时任务运行。
/// `set_keep_awake` 与回归测试共用此单一来源。
/// Acquire the keep-awake assertion: inhibit system idle sleep only, while letting the display
/// sleep normally (≈ `caffeinate -i`, not `-di`) — saves power and avoids screen wear, and the
/// display turning off does NOT pause scheduled tasks. Single source shared with the test.
fn acquire_keep_awake() -> Result<keepawake::KeepAwake, String> {
    keepawake::Builder::default()
        .display(false) // 不持有 PreventUserIdleDisplaySleep:允许显示器空闲关闭(省电 + 护屏)
        .idle(true) // PreventUserIdleSystemSleep:系统保持唤醒;电池供电时同样生效
        .sleep(false) // PreventSystemSleep:已废弃 + 电池下被忽略,显式关闭
        .reason("NomiFun keep-awake enabled")
        .app_name("NomiFun")
        .app_reverse_domain("com.nomifun.desktop")
        .create()
        .map_err(|e| format!("failed to acquire keep-awake assertion: {e}"))
}

/// 开启/关闭"保持唤醒":开盖状态下阻止系统空闲休眠,但允许显示器照常熄屏(等价 `caffeinate -i`)。
/// macOS 硬限制:合盖属于"强制休眠"(forced sleep),任何 IOKit assertion 都拦不住(参见 Apple QA1340);
/// 合盖仍要运行,只能 clamshell 模式(插电 + 外接显示器 + 外接键鼠)或 root 级 `pmset disablesleep 1`。
/// 早先还持有 PreventUserIdleDisplaySleep(display=true)强制屏幕常亮,会阻止显示器关闭、徒增屏幕损耗,
/// 现已去掉;PreventSystemSleep(sleep)自 macOS 10.9 起已废弃且电池下被忽略,同样不用。
/// Keep-awake: with the lid OPEN, inhibit idle system sleep but let the display sleep (~`caffeinate -i`).
/// Lid-close is forced sleep that no assertion can block; the old display-on assertion (which blocked
/// the monitor from turning off) and the deprecated PreventSystemSleep are both gone.
#[tauri::command]
fn set_keep_awake(enabled: bool, state: tauri::State<'_, AwakeState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if enabled {
        if guard.is_none() {
            *guard = Some(acquire_keep_awake()?);
        }
    } else {
        *guard = None; // Drop 释放 assertion / Drop releases the assertion.
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod keep_awake_tests {
    use super::acquire_keep_awake;
    use std::process::Command;
    use std::thread::sleep;
    use std::time::Duration;

    /// 回归测试:保持唤醒必须只阻止系统空闲休眠,绝不阻止显示器关闭。
    /// 用真实 IOKit assertion + `pmset -g assertions` 验证 —— 只看本测试进程(按 pid 过滤)
    /// 自己持有的 assertion,因此不受同时运行的 App 实例或 `caffeinate` 干扰。
    /// Regression: keep-awake must hold PreventUserIdleSystemSleep but NOT
    /// PreventUserIdleDisplaySleep (the latter is what stops the monitor from turning off).
    #[test]
    fn holds_system_idle_assertion_but_not_display() {
        let handle = acquire_keep_awake().expect("acquire keep-awake assertion");
        let owner = format!("pid {}(", std::process::id());

        // assertion 注册是同步的,但留一点重试余量以防极偶发的可见性延迟。
        let mut ours: Vec<String> = Vec::new();
        for _ in 0..10 {
            let out = Command::new("pmset")
                .args(["-g", "assertions"])
                .output()
                .expect("run `pmset -g assertions`");
            ours = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.contains(&owner))
                .map(str::to_owned)
                .collect();
            if ours.iter().any(|l| l.contains("PreventUserIdleSystemSleep")) {
                break;
            }
            sleep(Duration::from_millis(50));
        }

        assert!(
            ours.iter().any(|l| l.contains("PreventUserIdleSystemSleep")),
            "keep-awake should hold PreventUserIdleSystemSleep; our assertions: {ours:?}"
        );
        assert!(
            !ours.iter().any(|l| l.contains("PreventUserIdleDisplaySleep")),
            "keep-awake must NOT hold PreventUserIdleDisplaySleep (it blocks the display from \
             turning off); our assertions: {ours:?}"
        );

        drop(handle);
    }
}

/// 关闭=收到托盘的护栏标志。仅在「真正退出」(托盘「退出」/`app.exit`)前置真,届时主窗口的
/// `CloseRequested` 处理停止拦截、放行关闭;默认关闭手势(标题栏 ×、系统关闭、Alt+F4)保持假,
/// 因此一律隐藏到托盘而非退出进程。
/// Set true just before a real quit so the main window's CloseRequested handler stops
/// intercepting; default close gestures leave it false and therefore hide to tray.
struct QuitFlag(AtomicBool);

/// 托盘菜单两项的句柄,留存以便前端在 UI 语言就绪后本地化标签(见 `set_tray_labels`)。
/// 创建时用英文兜底,确保渲染层挂载前托盘已可用。
/// Handles to the two tray menu items so the renderer can localize their labels once the
/// UI locale is known; built with English fallbacks so the tray works before the UI mounts.
struct TrayMenuItems {
    show: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

/// 把主窗口从托盘(或其他窗口背后)唤回:隐藏则显示、最小化则还原,并聚焦。
/// Bring the main window back from the tray: show if hidden, restore if minimized, then focus.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn should_show_main_window_for_macos_reopen(_has_visible_windows: bool) -> bool {
    true
}

fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        // Real app exit: tray-quit's `app.exit(0)`, the `Destroyed`→`exit(0)`
        // path, macOS Cmd-Q, and last-window-closed all surface here. Close-to-tray
        // uses `api.prevent_close()` in the `CloseRequested` handler so the window
        // is merely hidden and this event NEVER fires for it — which makes it safe
        // to wipe every terminal session here (kill PTYs + delete rows) with no
        // QuitFlag guard. Blocks briefly (≤3s) so the wipe finishes before exit.
        tauri::RunEvent::ExitRequested { .. } => {
            if let Some(server) = app.try_state::<Arc<DesktopServer>>() {
                server.shutdown_terminals_blocking();
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { has_visible_windows, .. } => {
            if should_show_main_window_for_macos_reopen(has_visible_windows) {
                show_main_window(app);
            }
        }
        _ => {
            let _ = app;
        }
    }
}

/// 本地化原生托盘菜单。渲染层在挂载时及语言切换时调用,传入翻译后的 `tray.showWindow` /
/// `tray.quit` 文案——Rust 侧无法自行解析 i18n,故创建时用英文兜底,随后采纳这些标签。
/// Localize the native tray menu: the renderer hands over translated labels on mount and on
/// language change (Rust can't resolve i18n itself, so it ships English fallbacks first).
#[tauri::command]
fn set_tray_labels(
    show: String,
    quit: String,
    items: tauri::State<'_, TrayMenuItems>,
) -> Result<(), String> {
    items.show.set_text(show).map_err(|e| e.to_string())?;
    items.quit.set_text(quit).map_err(|e| e.to_string())?;
    Ok(())
}

/// Reconcile the native desktop-companion window set (labels `companion-{companion_id}`) against
/// the desired specs sent by the main window (useCompanionWindowsSync):
///   - `companion-*` windows whose companion is gone or disabled → close;
///   - enabled companions without a window → create, hidden — the companion page shows the
///     window itself once its config loads (window autonomy, unchanged);
///   - windows already matching a desired spec are left untouched.
/// Async on purpose: creating a webview from a *sync* command can deadlock on
/// Windows (wry limitation).
#[tauri::command]
async fn sync_companion_windows(
    app: tauri::AppHandle,
    server: tauri::State<'_, Arc<DesktopServer>>,
    memory_panel: tauri::State<'_, memory_panel_window::MemoryPanelWindowState>,
    specs: Vec<CompanionWindowSpec>,
) -> Result<(), String> {
    let init_script = webui_init_script(server.loopback_port(), server.local_trust_secret());
    let enabled_ids = specs
        .iter()
        .filter(|spec| spec.enabled)
        .map(|spec| spec.companion_id.clone())
        .collect();
    let hide_memory_panel = memory_panel.invalidate_owner_unless(&enabled_ids);
    let memory_panel_for_task = memory_panel.inner().clone();
    let app_for_task = app.clone();
    run_on_main_thread_task(
        move |task| app.run_on_main_thread(task).map_err(|e| e.to_string()),
        move || reconcile_companion_windows(app_for_task, init_script, specs, hide_memory_panel, memory_panel_for_task),
    )
    .await
}

fn reconcile_companion_windows(
    app: tauri::AppHandle,
    init_script: String,
    specs: Vec<CompanionWindowSpec>,
    hide_memory_panel: bool,
    memory_panel: memory_panel_window::MemoryPanelWindowState,
) -> Result<(), String> {
    use std::collections::HashSet;

    if hide_memory_panel {
        memory_panel.run_if_empty(|| {
            if let Some(window) = app.get_webview_window(memory_panel_window::MEMORY_PANEL_LABEL) { let _ = window.hide(); }
            Ok(())
        })?;
    }

    let known: HashSet<String> = specs
        .iter()
        .map(|s| format!("companion-{}", s.companion_id))
        .collect();
    let desired: HashSet<String> = specs
        .iter()
        .filter(|s| s.enabled)
        .map(|s| format!("companion-{}", s.companion_id))
        .collect();

    // Reconcile existing companion windows. Disabling a companion HIDES (keeps)
    // its window rather than closing it: destroy-then-recreate raced with the
    // async close/lingering and could leave a re-enabled companion with NO
    // visible window ("隐藏后再点显示，桌面伙伴再也起不来"). Only a companion that no
    // longer exists at all (deleted) is closed/destroyed.
    for (label, window) in app.webview_windows() {
        if !label.starts_with("companion-") {
            continue;
        }
        if !known.contains(&label) {
            if let Err(e) = window.close() {
                tracing::warn!(error = %e, label = %label, "failed to close removed companion window");
            }
        } else if !desired.contains(&label) {
            if let Err(e) = window.hide() {
                tracing::warn!(error = %e, label = %label, "failed to hide disabled companion window");
            }
        }
    }

    // Ensure every enabled companion has a VISIBLE window: show the existing one
    // (it may be hidden from a previous disable / right-click hide), or create it
    // (hidden; the companion page shows itself once its config loads).
    for spec in specs.iter().filter(|s| s.enabled) {
        let label = format!("companion-{}", spec.companion_id);
        if let Some(window) = app.get_webview_window(&label) {
            // Only show a window that is actually HIDDEN. Tauri's `show()` maps to
            // tao `set_visible(true)` → `makeKeyAndOrderFront`, which makes the
            // window the macOS *key* window — stealing keyboard focus from the
            // main window — and it does this even when the window is already
            // visible (no `isVisible()` short-circuit anywhere in the chain). This
            // sync also fires on the MAIN window's own `focus` event
            // (useCompanionWindowsSync), so an unconditional `show()` turned every
            // main-window refocus into a focus-steal loop ("点按钮/打字总被夺焦").
            // Re-showing an already-visible companion has no visible effect anyway,
            // so skipping it is purely the removal of the unwanted re-key. An
            // `is_visible()` error biases toward showing (visibility correctness
            // outweighs a rare extra steal — a companion that won't appear is the
            // worse bug).
            if !window.is_visible().unwrap_or(false) {
                if let Err(e) = window.show() {
                    tracing::warn!(error = %e, label = %label, "failed to show enabled companion window");
                }
            }
            continue;
        }
        let url = format!("index.html#/companion?companion_id={}", spec.companion_id);
        let builder =
            tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
                // Placeholder title for the brief pre-load frame; the companion page
                // overwrites it with the companion's custom name once its profile loads
                // (see setTitle in pages/companion/index.tsx). Never the lowercase engine id.
                .title("NomiFun")
                // Matches DEFAULT_DESK (characters/index.ts): figure + minimal chrome,
                // no reserved bubble headroom (the page grows the window on demand).
                // Keeping these in sync avoids a visible startup resize for built-ins.
                .inner_size(240.0, 214.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .visible(false)
                .initialization_script(&init_script);
        // Show the freshly-built window from here rather than relying SOLELY on
        // the companion page's self-show (applyWindowState): on a FIRST enable
        // (no window existed, so we land in this create branch) the page init
        // can stall/fail (transient boot 5xx in its Promise.all, the configReady
        // gate, a wry build hiccup) and — since no later event re-enters the
        // create branch — the window would stay hidden forever ("开启桌面显示但
        // 桌面伙伴不出现"). The page's own show() is idempotent and still handles
        // position correction + later enable/disable toggles. A failed build
        // self-heals on the next sync.
        match builder.build() {
            Ok(window) => {
                // 只有能在整窗穿透后重新查询本地光标的后端才允许启动为穿透态。
                // native Wayland 无全局指针查询，必须默认捕获，避免伙伴永久不可操作。
                if companion_pointer::supports_initial_pointer_passthrough(&window) {
                    if let Err(e) = window.set_ignore_cursor_events(true) {
                        tracing::warn!(error = %e, label = %label, "failed to set startup companion click-through");
                    }
                }
                if let Err(e) = window.show() {
                    tracing::warn!(error = %e, label = %label, "failed to show created companion window");
                }
            }
            Err(e) => tracing::warn!(error = %e, label = %label, "failed to create companion window"),
        }
    }
    Ok(())
}

/// Keep Tauri's generated context macro at one expansion site. Production Wry
/// and the mock-runtime artifact test then consume the exact same generated
/// assets without defining platform metadata symbols twice in one test binary.
fn generated_tauri_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

fn main() -> std::process::ExitCode {
    // If an ACP agent CLI spawned this shell as an MCP stdio bridge
    // (`current_exe() mcp-requirement-stdio` etc.), run that helper and exit
    // BEFORE any runtime init, single-instance handling, or window creation.
    // Every host binary must honor these or the injected declaration tools
    // (requirement_complete / team / guide) never appear in the agent's session.
    if let Some(code) = nomifun_app::commands::run_mcp_stdio_subcommand_if_present() {
        return code;
    }

    // Env mutation + runtime init BEFORE Tauri builds its runtime/threads,
    // mirroring the nomicore bin's ordering. v3 is a hard dataset cut, so the
    // desktop must never copy or reopen the historical temp-rooted dataset
    // before the shared dataset gate runs. The backend will quarantine any
    // incompatible dataset already present at the current data root.
    let data_dir = default_data_dir();
    publish_effective_data_dir_for_relaunch(&data_dir);
    nomifun_runtime::init(&data_dir);
    // SAFETY: no worker threads exist yet (Tauri's runtime is built by .run()).
    let merged_path = unsafe { nomifun_runtime::enhance_process_path() };

    // Backend config. The desktop does NOT use `--local`: `DesktopServer::start`
    // runs the backend under `TrustLocalToken` (trusts only its own webview via
    // a per-boot secret) so the LAN listener can require login. Only the data
    // dir + log level flow from here; the listeners bind their own ports.
    let mut cli = nomifun_app::cli::Cli::parse_from(["nomifun-desktop"]);
    cli.data_dir = data_dir;
    // Opt-in verbose backend logging without a custom build, e.g.
    //   NOMI_LOG_LEVEL=debug            (everything)
    //   NOMI_LOG_LEVEL=info             (default)
    // At `debug`, the `nomi_providers` target logs the outgoing request body and
    // each SSE chunk, and `nomi_mcp` logs MCP connect results — exactly what is
    // needed to diagnose a provider/gateway stall. Console output appears in the
    // terminal that launched `tauri dev`; it is also written to the log files
    // under {data-dir}/logs/.
    if let Ok(level) = std::env::var("NOMI_LOG_LEVEL") {
        let level = level.trim();
        if !level.is_empty() {
            cli.log_level = Some(level.to_owned());
        }
    }

    let app = tauri::Builder::default()
        // single-instance MUST be the first plugin. With its `deep-link` feature
        // enabled (see Cargo.toml), it forwards a second instance's argv into the
        // deep-link plugin BEFORE invoking this callback, so `on_open_url` (wired
        // in setup) fires on its own. We still use the callback to surface the
        // existing window: a second launch usually means the app is hidden in the
        // tray (close-to-tray), so bring it back instead of silently no-op'ing.
        // (Schemes are statically configured in tauri.conf.json, so there is no
        // need to re-parse argv here.)
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            // In dev, the desktop webview loads the live vite dev server; serving
            // the (stale) bundled `ui/dist` to remote browsers would desync them
            // from the desktop. So in dev the LAN listener proxies the SPA to vite
            // instead. In production this is None and embedded assets are served.
            let dev_frontend_url: Option<String> = if tauri::is_dev() {
                app.config()
                    .build
                    .dev_url
                    .as_ref()
                    .map(|u| u.to_string())
                    .or_else(|| Some("http://localhost:5173".to_string()))
            } else {
                None
            };
            let explicit_dist_override = configured_webui_dist_override();
            // Production custom-protocol builds already contain frontendDist in
            // the executable. This is the canonical, cross-platform source for
            // remote WebUI requests and also covers --no-bundle fast builds.
            // An explicit directory override keeps its historical precedence.
            let webui_asset_source =
                if dev_frontend_url.is_none() && explicit_dist_override.is_none() {
                    resolve_embedded_webui_assets(app)?
                } else {
                    None
                };
            // Only probe platform resource/cwd layouts when explicitly requested
            // or when an older/custom host has no embedded frontend. A stale
            // sidecar must never block a valid canonical embedded distribution.
            let spa_dir = if should_resolve_filesystem_webui(
                explicit_dist_override.is_some(),
                dev_frontend_url.is_some(),
                webui_asset_source.is_some(),
            ) {
                resolve_webui_spa_dir(app, explicit_dist_override.as_deref())?
            } else {
                None
            };
            if spa_dir.is_none()
                && dev_frontend_url.is_none()
                && webui_asset_source.is_none()
            {
                tracing::warn!(
                    "WebUI app shell not found — remote browsers would receive the API but no app shell"
                );
            }

            // Hand the backend control handle (port + trust secret + LAN
            // lifecycle) back to this thread once the loopback listener is bound.
            let (boot_tx, boot_rx) = std::sync::mpsc::channel::<Arc<DesktopServer>>();
            let backend_err_handle = app.handle().clone();
            let status_emit_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("nomifun-backend".into())
                .spawn(move || {
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
                        let rt = tokio::runtime::Builder::new_multi_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| anyhow::anyhow!("failed to build backend runtime: {e}"))?;
                        rt.block_on(async move {
                            let (server, _keep_alive) =
                                DesktopServer::start(
                                    &cli,
                                    &merged_path,
                                    spa_dir,
                                    dev_frontend_url,
                                    webui_asset_source,
                                )
                                .await?;
                            // Unblock the main thread's window build.
                            let _ = boot_tx.send(server.clone());
                            // Forward LAN status changes to the renderer. Holding
                            // `server` + `_keep_alive` here keeps the backend (and
                            // this runtime) alive for the process lifetime.
                            let mut rx = server.subscribe_status();
                            while rx.changed().await.is_ok() {
                                let status = rx.borrow().clone();
                                let _ = status_emit_handle.emit("webui://status-changed", status);
                            }
                            drop(_keep_alive);
                            Ok::<(), anyhow::Error>(())
                        })
                    }));
                    let error = match run {
                        Ok(Ok(())) => return, // clean shutdown
                        Ok(Err(e)) => format!("{e:#}"),
                        Err(panic) => panic
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "backend thread panicked".to_owned()),
                    };
                    tracing::error!(error = %error, "embedded backend exited with error");
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    backend_err_handle
                        .dialog()
                        .message(error)
                        .title("NomiFun backend failed to start")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    backend_err_handle.exit(1);
                })
                .expect("failed to spawn backend thread");

            // Wait for the backend to bind its loopback listener (or fail) before
            // building the window — the init script needs the port + trust
            // secret. A recv error means the backend failed; it has already shown
            // a dialog and will exit, so we just stop building the window.
            let Ok(server) = boot_rx.recv() else {
                return Ok(());
            };
            let loopback_port = server.loopback_port();

            // Build the main window programmatically so we can inject the backend
            // port + local-trust secret via an INITIALIZATION SCRIPT — it runs
            // before any page script, so the renderer's first `getBaseUrl()` (and
            // its trust-header attach) always see them. Race-free (unlike
            // eval-after-load).
            //
            // Frameless on Windows/Linux: the React titlebar draws its own
            // min/max/close (via @tauri-apps/api/window) on the same row as the
            // app's nav buttons. macOS keeps native traffic-light buttons via the
            // Overlay title-bar style, with content extending under the bar.
            // resizable defaults to true, so edge-resize + Snap are retained even
            // without decorations on Windows.
            let init_script = webui_init_script(loopback_port, server.local_trust_secret());
            app.manage(server);
            let win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
                    .title("NomiFun")
                    .inner_size(1280.0, 832.0)
                    .min_inner_size(880.0, 600.0)
                    .initialization_script(&init_script);
            // macOS: Overlay makes the titlebar transparent + extends content under
            // it, but it does NOT hide the native title text. With the title still
            // set to "NomiFun", AppKit draws that string next to the traffic lights,
            // overlapping the React sidebar toggle. `hidden_title(true)` maps to
            // `setTitleVisibility(Hidden)` so the OS keeps the title for menus /
            // Mission Control while leaving the titlebar visually empty.
            //
            // Vertically center the traffic lights on the React toolbar's button
            // line. The React titlebar (`.app-titlebar--mac`, height 45px in
            // ui/.../titlebar.css) centers its 36px buttons at y≈22.5px from the
            // window top, but AppKit's default places the 16px lights at center
            // y≈16px — ~6.5px too high. tao's `inset_traffic_lights` (view.rs)
            // makes `y` the height of the button *container* (16 + y) and
            // bottom-anchors the lights in it with a ~10px margin, so the lights'
            // center-from-top works out to (y - 2). To land the center at 22.5px:
            // y = 24.5 (empirically verified via the Accessibility API).
            //
            // Horizontally, `x` is the left edge of the close button's frame
            // (tao sets `rect.origin.x = x` per button). That frame is 14px
            // wide with the visible 12px circle centered in it (1px each
            // side), so the circle's left gap from the window edge is x + 1.
            // Balance that gap with the vertical whitespace around the lights:
            // (45 - 12) / 2 = 16.5px above/below the circle, hence
            // x = 16.5 - 1 = 15.5. (AppKit's native ~8px inset assumes a 28px
            // titlebar and looks glued to the corner in a 45px one; Apple's
            // own apps use ~16-20px in tall toolbars.) The lights then span up
            // to zoom's right edge at 15.5 + 2*20 + 14 = 69.5px, still clear
            // of the React menu, which starts at 84px (8px titlebar padding +
            // 76px margin-left in Titlebar/index.tsx).
            // (`traffic_light_position` requires Overlay + decorations:true, both set.)
            #[cfg(target_os = "macos")]
            let win_builder = win_builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .traffic_light_position(tauri::LogicalPosition::new(15.5, 24.5));
            #[cfg(not(target_os = "macos"))]
            let win_builder = win_builder.decorations(false);
            win_builder.build()?;

            // System tray. Closing the main window HIDES it here instead of
            // quitting (see the CloseRequested handler in on_window_event); the
            // process truly exits only via the tray's "退出" item. Left-click the
            // icon to bring the window back; right-click for the Show/Quit menu.
            // Labels are English fallbacks, adopted from the renderer's locale via
            // `set_tray_labels` once it mounts (the renderer always loads before
            // the user can close, so the first menu open is already localized).
            let tray_show = MenuItem::with_id(app, "tray-show", "Show NomiFun", true, None::<&str>)?;
            let tray_quit = MenuItem::with_id(app, "tray-quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&tray_show, &tray_quit])?;
            app.manage(TrayMenuItems {
                show: tray_show.clone(),
                quit: tray_quit.clone(),
            });
            let mut tray_builder = TrayIconBuilder::with_id("nomi-tray")
                .tooltip("NomiFun")
                .menu(&tray_menu)
                // Left-click is reserved for "surface the window"; the menu is
                // right-click only (otherwise a left-click would both pop the menu
                // AND try to show the window).
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "tray-show" => show_main_window(app),
                    "tray-quit" => {
                        // Arm the quit guard FIRST, then exit — the CloseRequested
                        // handler checks this flag and stops hiding-to-tray.
                        app.state::<QuitFlag>().0.store(true, Ordering::SeqCst);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });
            // Reuse the app's bundled window icon for the tray (no extra asset).
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            tray_builder.build(app)?;

            // Desktop-companion windows are NOT created here anymore. They are
            // multi-companion and dynamic: the main window's useCompanionWindowsSync hook
            // invokes `sync_companion_windows` (above) on boot and on companion
            // created/deleted/config-updated events, reconciling one
            // transparent always-on-top `companion-{companion_id}` window per enabled companion.

            // Wire deep-link open-url events to a Tauri event the renderer can
            // `listen()` to. `register_all()` is best-effort (some platforms /
            // dev contexts need it; ignore the error if it fails).
            let handle = app.handle().clone();
            let _ = app.deep_link().register_all();
            app.deep_link().on_open_url(move |event| {
                let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                let _ = handle.emit("deep-link://received", urls);
            });
            Ok(())
        })
        // The ~38 OS-shell commands (window controls, tray, zoom, get-path,
        // feedback, auto-update status) register here as #[tauri::command]s (P3).
        .manage(AwakeState(Mutex::new(None)))
        .manage(QuitFlag(AtomicBool::new(false)))
        .manage(memory_panel_window::MemoryPanelWindowState::default())
        .invoke_handler(tauri::generate_handler![
            check_for_updates,
            companion_pointer::get_companion_local_pointer,
            updater_install_context::get_updater_install_context,
            sync_companion_windows,
            memory_panel_window::prepare_companion_memory_panel,
            memory_panel_window::place_companion_memory_panel,
            memory_panel_window::show_companion_memory_panel,
            memory_panel_window::hide_companion_memory_panel,
            webui_get_status,
            webui_start,
            webui_stop,
            set_keep_awake,
            set_tray_labels
        ])
        // Close-to-tray is now the DEFAULT (and only) close behavior. Closing the
        // main window (titlebar ×, OS close, Alt+F4) hides it to the tray instead
        // of quitting — the agent, scheduled tasks, and companions keep running in
        // the background. The process exits ONLY via the tray's "退出" item, which
        // arms QuitFlag and calls app.exit(0); with the flag set we let the close
        // proceed and the Destroyed arm tears the process down (the always-on-top
        // companion windows would otherwise keep it — and a floating companion —
        // alive after the main window is gone).
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let quitting = window.app_handle().state::<QuitFlag>().0.load(Ordering::SeqCst);
                    if !quitting {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    window.app_handle().exit(0);
                }
                _ => {}
            }
        })
        .build(generated_tauri_context())
        .expect("error while building tauri application");

    // `Builder::run(context)` installs an empty app-level event callback. Build
    // manually so a Dock click after close-to-tray can surface the hidden main window.
    app.run(handle_run_event);

    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};

    fn write_v3_data_root_identity(data: &Path, work: &Path) {
        let generation = nomifun_common::generate_id();
        fs::write(data.join("nomifun-backend.db"), b"sqlite fixture")
            .expect("write database");
        fs::write(data.join("storage-generation"), &generation)
            .expect("write storage generation");
        fs::write(
            data.join(
                nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE,
            ),
            serde_json::to_vec(&serde_json::json!({
                "contract_version":
                    nomifun_common::factory_reset::V3_DATASET_CONTRACT_VERSION,
                "generation": generation,
                "work_root": work.display().to_string(),
                "work_root_binding_required": false,
                "installed_at": 1
            }))
            .expect("serialize receipt"),
        )
        .expect("write receipt");
    }

    #[test]
    fn external_desktop_data_dir_keeps_parent_override_semantics() {
        assert_eq!(
            resolve_desktop_data_dir(
                None,
                Some(std::ffi::OsString::from("custom-parent")),
            ),
            PathBuf::from("custom-parent").join("Nomi")
        );
    }

    #[test]
    fn first_relaunch_from_an_old_build_recognizes_a_proven_v3_data_root() {
        let data = tempfile::tempdir().expect("create data root");
        let work = tempfile::tempdir().expect("create external work root");
        let data = fs::canonicalize(data.path()).expect("canonicalize data root");
        let work = fs::canonicalize(work.path()).expect("canonicalize work root");
        write_v3_data_root_identity(&data, &work);

        assert!(is_proven_inherited_effective_data_root(&data));
        assert_eq!(
            resolve_desktop_data_dir(
                None,
                Some(data.clone().into_os_string()),
            ),
            data,
            "the first patched process must not turn an old backend export into Nomi/Nomi"
        );
    }

    #[test]
    fn unproven_directory_named_nomi_remains_an_external_parent_override() {
        let fixture = tempfile::tempdir().expect("create fixture root");
        let parent = fixture.path().join("Nomi");
        fs::create_dir(&parent).expect("create explicitly named parent");
        let parent = fs::canonicalize(parent).expect("canonicalize parent");
        fs::write(parent.join("nomifun-backend.db"), b"sqlite fixture")
            .expect("write ambiguous database");
        fs::write(
            parent.join("storage-generation"),
            nomifun_common::generate_id(),
        )
        .expect("write ambiguous generation");
        fs::write(
            parent.join(
                nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE,
            ),
            br#"{"contract_version":3,"generation":"not-a-uuid"}"#,
        )
        .expect("write malformed receipt");

        assert!(!is_proven_inherited_effective_data_root(&parent));
        assert_eq!(
            resolve_desktop_data_dir(
                None,
                Some(parent.clone().into_os_string()),
            ),
            parent.join("Nomi")
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn linked_v3_data_root_fails_closed_as_an_external_parent_override() {
        let data = tempfile::tempdir().expect("create real data root");
        let work = tempfile::tempdir().expect("create work root");
        let data = fs::canonicalize(data.path()).expect("canonicalize data root");
        let work = fs::canonicalize(work.path()).expect("canonicalize work root");
        write_v3_data_root_identity(&data, &work);

        let links = tempfile::tempdir().expect("create alias parent");
        let alias = links.path().join("data-alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&data, &alias).expect("create data-root symlink");
        #[cfg(windows)]
        junction::create(&data, &alias).expect("create data-root junction");

        assert!(!is_proven_inherited_effective_data_root(&alias));
        assert_eq!(
            resolve_desktop_data_dir(
                None,
                Some(alias.clone().into_os_string()),
            ),
            alias.join("Nomi")
        );
    }

    #[test]
    fn relaunch_uses_explicit_effective_data_dir_without_appending_nomi() {
        let effective = PathBuf::from("custom-parent").join("Nomi");
        assert_eq!(
            resolve_desktop_data_dir(
                Some(effective.clone().into_os_string()),
                // The embedded backend republishes its effective data dir
                // through this public variable before Tauri relaunches.
                Some(effective.clone().into_os_string()),
            ),
            effective
        );
    }

    #[test]
    fn internal_effective_data_dir_is_not_inferred_from_a_path_basename() {
        let effective = PathBuf::from("canonicalized").join("dataset-root");
        assert_eq!(
            resolve_desktop_data_dir(
                Some(effective.clone().into_os_string()),
                Some(std::ffi::OsString::from("different-backend-export")),
            ),
            effective
        );
    }

    #[test]
    fn empty_internal_effective_data_dir_does_not_shadow_external_override() {
        assert_eq!(
            resolve_desktop_data_dir(
                Some(std::ffi::OsString::new()),
                Some(std::ffi::OsString::from("custom-parent")),
            ),
            PathBuf::from("custom-parent").join("Nomi")
        );
    }

    #[test]
    fn macos_reopen_surfaces_main_window_when_no_windows_are_visible() {
        assert!(should_show_main_window_for_macos_reopen(false));
    }

    #[test]
    fn macos_reopen_surfaces_main_window_even_when_companion_window_is_visible() {
        assert!(should_show_main_window_for_macos_reopen(true));
    }

    #[test]
    fn production_spa_candidate_rejects_a_manifestless_bundle() {
        let dist = tempfile::tempdir().expect("create dist fixture");
        fs::write(dist.path().join("index.html"), "<!doctype html>").expect("write index");

        let error = validate_webui_spa_candidate_with_build_id(
            dist.path(),
            true,
            Some("paired-test-build"),
        )
            .expect_err("production candidate must require a matching build manifest");
        assert!(format!("{error:#}").contains("nomifun-build.json"));
    }

    #[test]
    fn production_spa_candidate_rejects_a_host_without_an_exact_build_id() {
        let dist = tempfile::tempdir().expect("create dist fixture");
        fs::write(dist.path().join("index.html"), "<!doctype html>").expect("write index");

        let error = validate_webui_spa_candidate_with_build_id(dist.path(), true, None)
            .expect_err("an unpaired desktop host must never serve production assets");
        assert!(format!("{error:#}").contains("exact frontend build identity"));
    }

    #[test]
    fn embedded_webui_skips_stale_filesystem_candidates() {
        assert!(!should_resolve_filesystem_webui(false, false, true));
    }

    #[test]
    fn empty_webui_dist_override_does_not_disable_embedded_assets() {
        assert!(normalize_webui_dist_override(Some("".into())).is_none());
        assert_eq!(
            normalize_webui_dist_override(Some("custom-dist".into())),
            Some(PathBuf::from("custom-dist"))
        );
    }

    #[test]
    fn embedded_webui_snapshot_rejects_reusable_csp_nonces() {
        let asset = WebUiAsset::new(b"<html></html>".to_vec(), "text/html")
            .with_csp_header(Some("script-src 'nonce-1234'".to_string()));
        let error = validate_webui_snapshot_asset("index.html", &asset)
            .expect_err("per-response nonces cannot be cached for process lifetime");
        assert!(format!("{error:#}").contains("per-response CSP nonce"));
    }

    #[test]
    fn generated_custom_protocol_context_contains_a_valid_embedded_webui_snapshot() {
        if tauri::is_dev() {
            // Normal desktop tests intentionally use Vite/no embedded assets.
            // The three-platform generated-context gate sets the requirement
            // variable and runs this test with:
            //   cargo test -p nomifun-desktop --features tauri/custom-protocol \
            //     generated_custom_protocol_context_contains_a_valid_embedded_webui_snapshot
            assert!(
                std::env::var_os("NOMIFUN_REQUIRE_EMBEDDED_WEBUI_TEST").is_none(),
                "embedded WebUI gate requires tauri/custom-protocol, but Tauri is still in dev mode"
            );
            return;
        }

        let app = tauri::test::mock_builder()
            .build(generated_tauri_context())
            .expect("build Tauri mock app with generated production context");
        let source = resolve_embedded_webui_assets(&app)
            .expect("validate and snapshot generated frontendDist");
        assert!(
            source.is_some(),
            "custom-protocol context must embed an index.html app shell"
        );
    }

    #[test]
    fn explicit_webui_dist_keeps_precedence_in_production() {
        assert!(should_resolve_filesystem_webui(true, false, true));
    }

    #[test]
    fn filesystem_webui_remains_a_fallback_when_embedded_assets_are_missing() {
        assert!(should_resolve_filesystem_webui(false, false, false));
    }

    #[test]
    fn vite_development_never_probes_stale_webui_filesystem_assets() {
        assert!(!should_resolve_filesystem_webui(false, true, false));
        assert!(!should_resolve_filesystem_webui(true, true, false));
    }

    #[tokio::test]
    async fn main_thread_task_runner_executes_work_inside_dispatcher() {
        let dispatcher_thread = Arc::new(Mutex::new(None));
        let work_thread = Arc::new(Mutex::new(None));
        let dispatcher_thread_seen = dispatcher_thread.clone();
        let work_thread_seen = work_thread.clone();

        run_on_main_thread_task(
            move |task| {
                *dispatcher_thread_seen.lock().unwrap() = Some(std::thread::current().id());
                let handle = std::thread::spawn(move || task());
                handle.join().expect("dispatched task should not panic");
                Ok(())
            },
            move || {
                *work_thread_seen.lock().unwrap() = Some(std::thread::current().id());
                Ok(())
            },
        )
        .await
        .expect("task should complete");

        let dispatcher_thread = dispatcher_thread.lock().unwrap().unwrap();
        let work_thread = work_thread.lock().unwrap().unwrap();
        assert_ne!(
            dispatcher_thread, work_thread,
            "work must run inside the dispatcher task, not on the caller thread"
        );
    }
}
