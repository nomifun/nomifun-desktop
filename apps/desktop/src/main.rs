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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Parser;
use nomifun_app::{
    DesktopKeepAlive, DesktopServer, StartupCleanupDisposition, WebUiAsset, WebUiAssetSource,
    WebUiStatus,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

mod companion_pointer;
mod relay_pairing;
mod updater_install_context;

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

/// Keep Tauri's generated context macro at one expansion site. Production Wry
/// and the mock-runtime artifact test then consume the exact same generated
/// assets without defining platform metadata symbols twice in one test binary.
fn generated_tauri_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

/// Data root resolution, in priority order:
///
/// 1. `NOMIFUN_DATA_DIR` env — the FINAL data root, taken verbatim. This is
///    the same literal semantics as the web host and the `nomicore` bin, so
///    the value the backend re-exports for its children is a fixed point: an
///    in-app restart or auto-update relaunch that inherits it resolves to
///    the SAME directory instead of drifting. (The pre-0.3.4 shell appended
///    `/Nomi` here, which turned that inheritance into `…/Nomi/Nomi` and
///    made the post-update boot reject its own dataset.)
/// 2. The shared per-channel default from
///    `nomifun_app::cli::default_data_dir()`: stable builds use `NomiFun`,
///    non-stable builds a sibling such as `NomiFun-dev`.
///
/// `resolve_startup_data_root` then maps known self-export/default locations
/// (including values inherited from affected releases) onto the channel
/// default and runs the one-shot legacy layout migration
/// (`NomiFun/Nomi` → `NomiFun`).
fn default_data_dir() -> PathBuf {
    let requested = std::env::var_os("NOMIFUN_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(nomifun_app::cli::default_data_dir);
    nomifun_app::bootstrap::resolve_startup_data_root(requested)
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadUpdateProgress {
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    chunk_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_length: Option<u64>,
}

/// Marker prefix on `install_update` errors that mean the package was NEVER
/// handed to the installer: the slot did not hold the requested version, so
/// nothing on disk was touched and the running app is intact. The renderer must
/// recover from these (report and stay put) instead of taking the fail-closed
/// exit reserved for a failure that may have left a half-replaced app bundle.
const UPDATE_NOT_RETAINED_ERROR: &str = "NOMIFUN_UPDATE_NOT_RETAINED";

/// Smallest gap between download-progress messages pushed to the webview. The
/// plugin invokes its progress callback once per HTTP body chunk, and every
/// `Channel::send` is one `webview.eval` plus one React render downstream —
/// tens of thousands of them for a large installer, which starved the very
/// click the user was waiting to make. Bytes are accumulated between sends, so
/// coalescing changes the message RATE and never the reported total.
const UPDATE_PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(100);
/// A failed primary metadata request should reach the configured GitHub
/// endpoint promptly instead of pinning the update UI behind a dead route.
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(8);
/// Package downloads are allowed to be slow, but not indefinitely stuck. A
/// timeout on a non-GitHub source unlocks the explicit GitHub retry below.
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const GITHUB_UPDATER_ENDPOINT: &str =
    "https://github.com/nomifun/nomifun-desktop/releases/latest/download/latest.json";

/// What the native side currently holds for the in-app updater. This is the
/// single source of truth for "is an update installable right now" — the
/// renderer used to mirror it in module globals that silently drifted apart
/// from it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePackageStatus {
    /// `empty` | `downloading` | `ready` | `installing`
    state: &'static str,
    /// The version the active state refers to. Only `ready` means installable.
    version: Option<String>,
}

enum DownloadedUpdateSlot<T> {
    Empty,
    Downloading { version: String },
    Ready { version: String, payload: T },
    Installing { version: String },
}

struct DownloadedUpdateCache<T> {
    slot: Mutex<DownloadedUpdateSlot<T>>,
}

impl<T> Default for DownloadedUpdateCache<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(DownloadedUpdateSlot::Empty),
        }
    }
}

#[derive(Debug)]
enum BeginUpdateDownload {
    Start,
    /// The requested version is already downloaded and verified. Carries the
    /// retained size, measured under the SAME lock that decided this, so the
    /// answer and the size it reports can never disagree.
    AlreadyReady { retained_len: u64 },
    /// A verified package for `replaced` was discarded to make room for this
    /// download. Callers must log it: the renderer may still be offering the
    /// old version as installable.
    StartReplacingReady { replaced: String },
}

/// Why a package could not be claimed for installation. The distinction is a
/// SAFETY boundary, not cosmetics: `NotRetained`/`StillDownloading` prove the
/// installer was never handed anything, while `AlreadyInstalling` means another
/// handoff is underway and may already have moved the installed app aside — so
/// it must NOT be reported to the renderer as a recoverable "download again".
#[derive(Debug, PartialEq, Eq)]
enum TakeReadyError {
    NotRetained { version: String },
    StillDownloading { version: String },
    AlreadyInstalling { version: String },
}

impl TakeReadyError {
    fn message(&self) -> String {
        match self {
            Self::NotRetained { version } => format!("update {version} has not been downloaded"),
            Self::StillDownloading { version } => format!("update {version} is still downloading"),
            Self::AlreadyInstalling { version } => {
                format!("update {version} is already installing")
            }
        }
    }

    /// Whether an installer handoff may already be in progress for this package.
    fn handoff_may_have_started(&self) -> bool {
        matches!(self, Self::AlreadyInstalling { .. })
    }
}

impl<T> DownloadedUpdateCache<T> {
    /// `measure` reports the byte size of an already-retained payload. It is
    /// applied while the slot lock is still held so the "already downloaded"
    /// verdict and the size reported with it are one atomic observation — two
    /// separate lock acquisitions could report 0 bytes for a package that was
    /// discarded in between.
    fn begin_download(
        &self,
        version: &str,
        measure: impl FnOnce(&T) -> u64,
    ) -> Result<BeginUpdateDownload, String> {
        let mut slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        match &*slot {
            DownloadedUpdateSlot::Downloading { version: active }
            | DownloadedUpdateSlot::Installing { version: active } => {
                return Err(format!("update {active} is already being processed"));
            }
            DownloadedUpdateSlot::Ready {
                version: ready,
                payload,
            } if ready == version => {
                return Ok(BeginUpdateDownload::AlreadyReady {
                    retained_len: measure(payload),
                });
            }
            DownloadedUpdateSlot::Empty | DownloadedUpdateSlot::Ready { .. } => {}
        }
        // Note which verified package (if any) this download is about to drop.
        let replaced = match &*slot {
            DownloadedUpdateSlot::Ready { version: ready, .. } => Some(ready.clone()),
            _ => None,
        };
        *slot = DownloadedUpdateSlot::Downloading {
            version: version.to_owned(),
        };
        Ok(match replaced {
            Some(replaced) => BeginUpdateDownload::StartReplacingReady { replaced },
            None => BeginUpdateDownload::Start,
        })
    }

    /// A snapshot of what is held right now. Read-only: never mutates the slot,
    /// so the renderer can poll it to decide whether to offer "install".
    fn status(&self) -> UpdatePackageStatus {
        let slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        match &*slot {
            DownloadedUpdateSlot::Empty => UpdatePackageStatus {
                state: "empty",
                version: None,
            },
            DownloadedUpdateSlot::Downloading { version } => UpdatePackageStatus {
                state: "downloading",
                version: Some(version.clone()),
            },
            DownloadedUpdateSlot::Ready { version, .. } => UpdatePackageStatus {
                state: "ready",
                version: Some(version.clone()),
            },
            DownloadedUpdateSlot::Installing { version } => UpdatePackageStatus {
                state: "installing",
                version: Some(version.clone()),
            },
        }
    }

    fn cancel_download(&self, version: &str) {
        let mut slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        if matches!(&*slot, DownloadedUpdateSlot::Downloading { version: active } if active == version)
        {
            *slot = DownloadedUpdateSlot::Empty;
        }
    }

    fn finish_download(&self, version: &str, payload: T) -> Result<(), String> {
        let mut slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        if !matches!(&*slot, DownloadedUpdateSlot::Downloading { version: active } if active == version)
        {
            return Err(format!(
                "download state for update {version} changed before completion"
            ));
        }
        *slot = DownloadedUpdateSlot::Ready {
            version: version.to_owned(),
            payload,
        };
        Ok(())
    }

    fn take_ready(&self, version: &str) -> Result<T, TakeReadyError> {
        let mut slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        match std::mem::replace(&mut *slot, DownloadedUpdateSlot::Empty) {
            DownloadedUpdateSlot::Ready {
                version: ready,
                payload,
            } if ready == version => {
                *slot = DownloadedUpdateSlot::Installing { version: ready };
                Ok(payload)
            }
            other @ DownloadedUpdateSlot::Ready { .. } => {
                *slot = other;
                Err(TakeReadyError::NotRetained {
                    version: version.to_owned(),
                })
            }
            other @ DownloadedUpdateSlot::Downloading { .. } => {
                *slot = other;
                Err(TakeReadyError::StillDownloading {
                    version: version.to_owned(),
                })
            }
            other @ DownloadedUpdateSlot::Installing { .. } => {
                *slot = other;
                Err(TakeReadyError::AlreadyInstalling {
                    version: version.to_owned(),
                })
            }
            DownloadedUpdateSlot::Empty => Err(TakeReadyError::NotRetained {
                version: version.to_owned(),
            }),

        }
    }

    fn restore_ready(&self, version: String, payload: T) {
        let mut slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        if matches!(&*slot, DownloadedUpdateSlot::Installing { version: active } if active == &version)
        {
            *slot = DownloadedUpdateSlot::Ready { version, payload };
        }
    }

    /// Release the claim taken by [`Self::take_ready`] after the installer has
    /// accepted the package. Without this, `Installing` was a terminal state:
    /// on macOS/Linux `Update::install` RETURNS on success, so a completed
    /// install parked the slot forever and every later `begin_download` failed
    /// with "already being processed" until the app was restarted. Windows never
    /// reaches this (its installer handoff ends in `process::exit`), which is
    /// exactly why the leak went unnoticed.
    fn finish_install(&self, version: &str) {
        let mut slot = self.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        if matches!(&*slot, DownloadedUpdateSlot::Installing { version: active } if active == version)
        {
            *slot = DownloadedUpdateSlot::Empty;
        }
    }
}

struct DownloadedUpdatePackage {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
}

type DownloadedUpdateState = DownloadedUpdateCache<DownloadedUpdatePackage>;

fn checked_update_version(version: &str) -> Result<&str, String> {
    let version = version.trim();
    if version.is_empty() {
        Err("update version must not be empty".to_owned())
    } else {
        Ok(version)
    }
}

fn is_github_download_url(url: &url::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("github.com")
            || host.eq_ignore_ascii_case("release-assets.githubusercontent.com")
            || host.ends_with(".githubusercontent.com")
    })
}

fn build_desktop_updater(
    app: &tauri::AppHandle,
    shutdown_server: Arc<DesktopServer>,
    endpoints: Option<Vec<url::Url>>,
) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;

    let cleanup_app = app.clone();
    let builder = app
        .updater_builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .on_before_exit(move || {
            let verified = updater_before_exit_until_verified(
                || shutdown_server.shutdown_all_blocking(),
                || cleanup_app.cleanup_before_exit(),
                |attempt, error| {
                    tracing::error!(
                        attempt,
                        %error,
                        "updater installer handoff blocked until desktop shutdown is verified"
                    );
                },
                || std::thread::sleep(Duration::from_millis(250)),
                UPDATER_SHUTDOWN_MAX_ATTEMPTS,
            );
            if !verified {
                tracing::error!(
                    attempts = UPDATER_SHUTDOWN_MAX_ATTEMPTS,
                    "proceeding with the updater installer handoff after exhausting bounded \
                     shutdown attempts; desktop shutdown is NOT verified"
                );
            }
        });
    let builder = match endpoints {
        Some(endpoints) => builder.endpoints(endpoints).map_err(|error| error.to_string())?,
        None => builder,
    };
    builder.build().map_err(|error| error.to_string())
}

async fn check_requested_update(
    app: &tauri::AppHandle,
    shutdown_server: Arc<DesktopServer>,
    requested_version: &str,
    endpoints: Option<Vec<url::Url>>,
) -> Result<tauri_plugin_updater::Update, String> {
    let updater = build_desktop_updater(app, shutdown_server, endpoints)?;
    let mut update = updater
        .check()
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no update is currently available".to_owned())?;
    if update.version != requested_version {
        return Err(format!(
            "available update version changed from {requested_version} to {}",
            update.version
        ));
    }
    update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);
    Ok(update)
}

/// Accumulate `chunk` into `buffered` and decide whether to publish now.
///
/// Returns `Some(bytes)` — everything buffered since the last publish, INCLUDING
/// this chunk — once `min_interval` has elapsed, and `None` while bytes should
/// keep accumulating. The renderer adds each reported value to a running total,
/// so callers MUST flush whatever is left in `buffered` when the download ends
/// or the reported total finishes short of the real byte count.
fn coalesce_progress_chunk(
    buffered: &std::sync::atomic::AtomicU64,
    last_sent: &mut Instant,
    chunk: u64,
    min_interval: Duration,
    now: Instant,
) -> Option<u64> {
    use std::sync::atomic::Ordering;

    // fetch_add returns the PREVIOUS value, so add the chunk back to get the
    // post-add buffer.
    let pending = buffered.fetch_add(chunk, Ordering::Relaxed) + chunk;
    if now.duration_since(*last_sent) < min_interval {
        return None;
    }
    buffered.store(0, Ordering::Relaxed);
    *last_sent = now;
    Some(pending)
}

async fn download_verified_update(
    update: &tauri_plugin_updater::Update,
    on_event: tauri::ipc::Channel<DownloadUpdateProgress>,
) -> Result<Vec<u8>, String> {
    let download_progress = on_event.clone();
    let download_finished = on_event.clone();
    // Bytes seen since the last message, shared with the completion callback
    // so the coalesced remainder is always flushed and the renderer's running
    // total ends up exact.
    let buffered = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let buffered_tail = Arc::clone(&buffered);
    let observed_length = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observed_length_tail = Arc::clone(&observed_length);
    let mut last_sent = Instant::now();
    let bytes = update
        .download(
            move |chunk_length, content_length| {
                use std::sync::atomic::Ordering;
                if let Some(total) = content_length {
                    observed_length.store(total, Ordering::Relaxed);
                }
                let Some(pending) = coalesce_progress_chunk(
                    &buffered,
                    &mut last_sent,
                    chunk_length as u64,
                    UPDATE_PROGRESS_MIN_INTERVAL,
                    Instant::now(),
                ) else {
                    return;
                };
                let _ = download_progress.send(DownloadUpdateProgress {
                    phase: "downloading",
                    chunk_length: Some(pending as usize),
                    content_length,
                });
            },
            move || {
                use std::sync::atomic::Ordering;
                let total = observed_length_tail.load(Ordering::Relaxed);
                let content_length = (total > 0).then_some(total);
                // Flush whatever the interval swallowed. The final "downloaded"
                // event is emitted only after the plugin verifies the signature.
                let tail = buffered_tail.swap(0, Ordering::Relaxed);
                if tail > 0 {
                    let _ = download_finished.send(DownloadUpdateProgress {
                        phase: "downloading",
                        chunk_length: Some(tail as usize),
                        content_length,
                    });
                }
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    let _ = on_event.send(DownloadUpdateProgress {
        phase: "downloaded",
        chunk_length: None,
        content_length: Some(bytes.len() as u64),
    });
    Ok(bytes)
}

/// Download and verify the exact update selected by the renderer, retaining the
/// native Update handle and verified bytes together until installation. This is
/// the only command that performs package network I/O.
#[tauri::command]
async fn download_update(
    app: tauri::AppHandle,
    server: tauri::State<'_, Arc<DesktopServer>>,
    downloaded: tauri::State<'_, DownloadedUpdateState>,
    version: String,
    on_event: tauri::ipc::Channel<DownloadUpdateProgress>,
) -> Result<(), String> {
    let requested_version = checked_update_version(&version)?.to_owned();
    match downloaded.begin_download(&requested_version, |package| package.bytes.len() as u64)? {
        BeginUpdateDownload::AlreadyReady { retained_len } => {
            // Report the retained size so the renderer paints a truthful
            // "already complete" frame rather than 100% of nothing.
            let _ = on_event.send(DownloadUpdateProgress {
                phase: "downloaded",
                chunk_length: None,
                content_length: Some(retained_len),
            });
            return Ok(());
        }
        BeginUpdateDownload::StartReplacingReady { replaced } => {
            tracing::warn!(
                replaced = %replaced,
                requested = %requested_version,
                "discarding a verified update package to download a different version"
            );
        }
        BeginUpdateDownload::Start => {}
    }

    let _ = on_event.send(DownloadUpdateProgress {
        phase: "checking",
        chunk_length: None,
        content_length: None,
    });

    let result = async {
        let primary_update = check_requested_update(
            &app,
            server.inner().clone(),
            &requested_version,
            None,
        )
        .await?;
        let primary_url = primary_update.download_url.clone();
        let primary_download = download_verified_update(&primary_update, on_event.clone()).await;
        let (update, bytes) = match primary_download {
            Ok(bytes) => (primary_update, bytes),
            Err(primary_error) if !is_github_download_url(&primary_url) => {
                tracing::warn!(
                    url = %primary_url,
                    error = %primary_error,
                    "primary update download failed; retrying the same version from GitHub"
                );
                let _ = on_event.send(DownloadUpdateProgress {
                    phase: "retrying",
                    chunk_length: None,
                    content_length: None,
                });
                let github_endpoint =
                    url::Url::parse(GITHUB_UPDATER_ENDPOINT).map_err(|error| error.to_string())?;
                let github_update = check_requested_update(
                    &app,
                    server.inner().clone(),
                    &requested_version,
                    Some(vec![github_endpoint]),
                )
                .await
                .map_err(|fallback_error| {
                    format!(
                        "primary update download failed: {primary_error}; \
                         GitHub fallback check failed: {fallback_error}"
                    )
                })?;
                let bytes = download_verified_update(&github_update, on_event.clone())
                    .await
                    .map_err(|fallback_error| {
                        format!(
                            "primary update download failed: {primary_error}; \
                             GitHub fallback download failed: {fallback_error}"
                        )
                    })?;
                (github_update, bytes)
            }
            Err(error) => return Err(error),
        };

        downloaded.finish_download(
            &requested_version,
            DownloadedUpdatePackage { update, bytes },
        )
    }
    .await;

    if result.is_err() {
        downloaded.cancel_download(&requested_version);
    }
    result
}

/// Install only the package already downloaded and signature-verified by
/// `download_update`. A missing or mismatched package is an error: installation
/// must never hide a second download behind an install action.
#[tauri::command]
async fn install_update(
    downloaded: tauri::State<'_, DownloadedUpdateState>,
    version: String,
) -> Result<(), String> {
    // `checked_update_version` and `take_ready` are precondition checks that
    // touch nothing on disk — EXCEPT `AlreadyInstalling`, which means another
    // handoff is already underway and may have moved the installed app aside.
    // Only the provably-untouched cases get the recoverable marker; killing the
    // process for those destroys the memory-only package and forces a full
    // re-download just to retry, which is the whole "must download twice to
    // install" complaint.
    let requested_version = checked_update_version(&version)
        .map_err(|error| format!("{UPDATE_NOT_RETAINED_ERROR}: {error}"))?
        .to_owned();
    let package = match downloaded.take_ready(&requested_version) {
        Ok(package) => package,
        Err(error) if error.handoff_may_have_started() => return Err(error.message()),
        Err(error) => return Err(format!("{UPDATE_NOT_RETAINED_ERROR}: {}", error.message())),
    };

    // Past this point the installer has been handed the bytes: on Windows this
    // never returns (the plugin spawns the installer and exits), and on macOS a
    // failure can leave the app bundle half replaced. An Err here is NOT
    // recoverable in the renderer.
    if let Err(error) = package.update.install(&package.bytes) {
        downloaded.restore_ready(requested_version, package);
        return Err(error.to_string());
    }
    downloaded.finish_install(&requested_version);
    Ok(())
}

/// The authoritative answer to "does an installable update package exist right
/// now". The renderer polls this instead of mirroring the state locally.
#[tauri::command]
async fn update_package_status(
    downloaded: tauri::State<'_, DownloadedUpdateState>,
) -> Result<UpdatePackageStatus, String> {
    Ok(downloaded.status())
}

/// Bounded shutdown attempts before the updater installer handoff proceeds
/// anyway (F32). Each attempt is itself bounded (shutdown_all_blocking runs
/// two ≤31s tries), so a persistently failing cleanup stage can no longer
/// hang the update install forever with zero user-visible progress. The
/// preflight semantics stay: shutdown is always attempted, and only after
/// exhausting these attempts does the handoff continue with a loud error.
const UPDATER_SHUTDOWN_MAX_ATTEMPTS: u64 = 4;

/// Returns whether the desktop shutdown was positively verified before the
/// preserved plugin cleanup ran. Callers must log loudly on `false`.
fn updater_before_exit_until_verified<S, C, E, W>(
    shutdown: S,
    cleanup_before_exit: C,
    on_error: E,
    wait_before_retry: W,
    max_attempts: u64,
) -> bool
where
    S: FnMut() -> anyhow::Result<()>,
    C: FnOnce(),
    E: FnMut(u64, &anyhow::Error),
    W: FnMut(),
{
    let verified =
        cleanup_until_verified_bounded(shutdown, on_error, wait_before_retry, max_attempts);

    // `UpdaterExt::updater_builder()` installs this cleanup by default, but
    // `on_before_exit` replaces that hook. Preserve it explicitly whether or
    // not the application-owned shutdown was verified — a hung install with
    // no escape hatch is strictly worse than an unverified handoff.
    cleanup_before_exit();
    verified
}

/// Shared bounded-then-proceed cleanup driver (the F32/F42/F33 pattern): run
/// up to `max_attempts` verification-gated cleanup attempts, waiting between
/// failed attempts, and return whether cleanup was positively verified.
/// Callers must log loudly and still make forward progress on `false` — an
/// unbounded retry that silently wedges the process is strictly worse than an
/// unverified handoff.
fn cleanup_until_verified_bounded<S, E, W>(
    mut cleanup: S,
    mut on_error: E,
    mut wait_before_retry: W,
    max_attempts: u64,
) -> bool
where
    S: FnMut() -> anyhow::Result<()>,
    E: FnMut(u64, &anyhow::Error),
    W: FnMut(),
{
    let max_attempts = max_attempts.max(1);
    for attempt in 1..=max_attempts {
        match cleanup() {
            Ok(()) => return true,
            Err(error) => {
                on_error(attempt, &error);
                if attempt < max_attempts {
                    wait_before_retry();
                }
            }
        }
    }
    false
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
/// Read local Relay pairing state without returning any bearer credentials.
#[tauri::command(rename_all = "camelCase")]
async fn relay_pairing_get_status(
    manager: tauri::State<'_, Arc<relay_pairing::RelayPairingManager>>,
    server: tauri::State<'_, Arc<DesktopServer>>,
) -> Result<relay_pairing::RelayPairingStatus, String> {
    manager
        .inner()
        .status(server.inner().clone())
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Exchange a one-shot Relay pairing envelope and start nfagent.
#[tauri::command(rename_all = "camelCase")]
async fn relay_pairing_bootstrap(
    manager: tauri::State<'_, Arc<relay_pairing::RelayPairingManager>>,
    server: tauri::State<'_, Arc<DesktopServer>>,
    pairing_envelope: String,
) -> Result<relay_pairing::RelayPairingStatus, String> {
    manager
        .inner()
        .clone()
        .bootstrap(server.inner().clone(), &pairing_envelope)
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Restart nfagent using its persisted long-lived credential.
#[tauri::command(rename_all = "camelCase")]
async fn relay_pairing_restart(
    manager: tauri::State<'_, Arc<relay_pairing::RelayPairingManager>>,
    server: tauri::State<'_, Arc<DesktopServer>>,
) -> Result<relay_pairing::RelayPairingStatus, String> {
    manager
        .inner()
        .clone()
        .restart(server.inner().clone())
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Stop nfagent while retaining restart-safe pairing state.
#[tauri::command(rename_all = "camelCase")]
async fn relay_pairing_stop(
    manager: tauri::State<'_, Arc<relay_pairing::RelayPairingManager>>,
) -> Result<relay_pairing::RelayPairingStatus, String> {
    manager
        .inner()
        .stop()
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Disconnect and remove pairing state and the agent credential directory.
#[tauri::command(rename_all = "camelCase")]
async fn relay_pairing_disconnect(
    manager: tauri::State<'_, Arc<relay_pairing::RelayPairingManager>>,
) -> Result<relay_pairing::RelayPairingStatus, String> {
    manager
        .inner()
        .disconnect()
        .await
        .map_err(|error| format!("{error:#}"))
}

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

const EXIT_PHASE_RUNNING: u8 = 0;
const EXIT_PHASE_SHUTTING_DOWN: u8 = 1;
const EXIT_PHASE_RESTARTING: u8 = 2;
const EXIT_PHASE_COMPLETE: u8 = 3;
const EXIT_PHASE_FATAL: u8 = 4;
const EXIT_PHASE_CLEANUP_FAILED: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestartCleanupOutcome {
    ContinueRestart,
    AbortRestart,
}

fn restart_cleanup_outcome(result: &anyhow::Result<()>) -> RestartCleanupOutcome {
    if result.is_ok() {
        RestartCleanupOutcome::ContinueRestart
    } else {
        RestartCleanupOutcome::AbortRestart
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownMode {
    Normal,
    Restart,
}

enum BackendRegistration {
    Starting,
    Ready(Arc<DesktopServer>),
    StoppedVerified,
    FailedVerified,
    FailedUnverified,
    FailedRetained(Arc<DesktopServer>),
}

struct RetainedStartupCleanupAuthority {
    keep_alive: Arc<DesktopKeepAlive>,
    runtime: tokio::runtime::Runtime,
    cleanup_gate: Mutex<()>,
    cleanup_verified: AtomicBool,
}

impl RetainedStartupCleanupAuthority {
    fn new(keep_alive: Arc<DesktopKeepAlive>, runtime: tokio::runtime::Runtime) -> Self {
        Self {
            keep_alive,
            runtime,
            cleanup_gate: Mutex::new(()),
            cleanup_verified: AtomicBool::new(false),
        }
    }

    fn cleanup(&self) -> anyhow::Result<()> {
        if self.cleanup_verified.load(Ordering::Acquire) {
            return Ok(());
        }
        let _gate = self
            .cleanup_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.cleanup_verified.load(Ordering::Acquire) {
            return Ok(());
        }
        self.keep_alive
            .shutdown_after_startup_failure_blocking(&self.runtime)?;
        self.cleanup_verified.store(true, Ordering::Release);
        Ok(())
    }
}

#[derive(Clone)]
enum StartupCleanup {
    /// The backend thread has not entered `DesktopServer::start` yet.  This is
    /// the only state in which the shell can prove that no backend-owned
    /// resource exists.
    NotStarted,
    /// `DesktopServer::start` has been entered, but it has not returned the
    /// authoritative server handle yet.  A failure in this window must not be
    /// treated as an already-verified cleanup: the start routine may have
    /// acquired services, listeners, or browser ownership before failing.
    StartingUnverified,
    /// `DesktopServer::start` returned a normal error after completing its
    /// internal startup-failure cleanup. Unlike a panic, this is a positive
    /// cleanup boundary: the shell may release the backend runtime and surface
    /// the startup error.
    FailedVerified,
    /// Typed startup returned an unverified failure together with the exact
    /// AppServices/environment authority needed to retry its cleanup.
    RetainedKeepAlive(Arc<DesktopKeepAlive>),
    /// Startup reached the point where the authoritative server owns cleanup.
    Server(Arc<DesktopServer>),
}

impl StartupCleanup {
    fn cleanup(&self, runtime: &tokio::runtime::Runtime) -> anyhow::Result<()> {
        match self {
            Self::NotStarted | Self::FailedVerified => Ok(()),
            Self::StartingUnverified => Err(anyhow::anyhow!(
                "embedded backend startup entered before a cleanup authority was published"
            )),
            Self::RetainedKeepAlive(keep_alive) => {
                keep_alive.shutdown_after_startup_failure_blocking(runtime)
            }
            Self::Server(server) => server.shutdown_all_blocking(),
        }
    }

    #[cfg(test)]
    fn is_verified(&self) -> bool {
        matches!(self, Self::NotStarted | Self::FailedVerified)
    }

    fn server(&self) -> Option<Arc<DesktopServer>> {
        match self {
            Self::NotStarted
            | Self::StartingUnverified
            | Self::FailedVerified
            | Self::RetainedKeepAlive(_) => None,
            Self::Server(server) => Some(server.clone()),
        }
    }

    fn retained_keep_alive(&self) -> Option<Arc<DesktopKeepAlive>> {
        match self {
            Self::RetainedKeepAlive(keep_alive) => Some(keep_alive.clone()),
            Self::NotStarted
            | Self::StartingUnverified
            | Self::FailedVerified
            | Self::Server(_) => None,
        }
    }
}

fn mark_startup_cleanup_entered(cleanup: &Mutex<StartupCleanup>) {
    *cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        StartupCleanup::StartingUnverified;
}

fn mark_startup_cleanup_failed_verified(cleanup: &Mutex<StartupCleanup>) {
    *cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        StartupCleanup::FailedVerified;
}

fn mark_startup_cleanup_retained(
    cleanup: &Mutex<StartupCleanup>,
    keep_alive: Arc<DesktopKeepAlive>,
) {
    *cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        StartupCleanup::RetainedKeepAlive(keep_alive);
}

fn mark_startup_cleanup_server(
    cleanup: &Mutex<StartupCleanup>,
    server: Arc<DesktopServer>,
) {
    *cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        StartupCleanup::Server(server);
}

/// Cross-thread state machine for Tauri's exit requests.
///
/// `ExitRequested` is delivered on the Tauri event loop, while
/// `DesktopServer::shutdown_all_async` invokes its callback on the backend
/// runtime. Keeping the state here makes the two paths single-flight and lets
/// a booting backend service an exit request that arrived before it was
/// managed by Tauri.
struct ExitCoordinator {
    phase: AtomicU8,
    restart_requested: AtomicBool,
    shutdown_started: AtomicBool,
    fatal_dialog_started: AtomicBool,
    cleanup_verified: AtomicBool,
    /// Single-flight guard for the F42 deferred exit waiter (repeated Cmd-Q
    /// while the backend is still Starting must not stack waiter threads).
    deferred_exit_wait_started: AtomicBool,
    original_code: Mutex<Option<i32>>,
    backend: Mutex<BackendRegistration>,
    retained_backend_runtimes: Mutex<Vec<tokio::runtime::Runtime>>,
    retained_startup_cleanup: Mutex<Option<Arc<RetainedStartupCleanupAuthority>>>,
    backend_changed: Condvar,
}

impl Default for ExitCoordinator {
    fn default() -> Self {
        Self {
            phase: AtomicU8::new(EXIT_PHASE_RUNNING),
            restart_requested: AtomicBool::new(false),
            shutdown_started: AtomicBool::new(false),
            fatal_dialog_started: AtomicBool::new(false),
            cleanup_verified: AtomicBool::new(false),
            deferred_exit_wait_started: AtomicBool::new(false),
            original_code: Mutex::new(None),
            backend: Mutex::new(BackendRegistration::Starting),
            retained_backend_runtimes: Mutex::new(Vec::new()),
            retained_startup_cleanup: Mutex::new(None),
            backend_changed: Condvar::new(),
        }
    }
}

impl ExitCoordinator {
    fn request_normal_exit(&self, code: Option<i32>) -> bool {
        if self.restart_requested.load(Ordering::Acquire) {
            return false;
        }
        loop {
            let phase = self.phase.load(Ordering::Acquire);
            if !matches!(phase, EXIT_PHASE_RUNNING | EXIT_PHASE_CLEANUP_FAILED) {
                return false;
            }
            if self
                .phase
                .compare_exchange(
                    phase,
                    EXIT_PHASE_SHUTTING_DOWN,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                if let Ok(mut original_code) = self.original_code.lock() {
                    *original_code = code;
                }
                return true;
            }
        }
    }

    fn request_restart(&self) -> bool {
        self.restart_requested.store(true, Ordering::Release);
        self.phase
            .compare_exchange(
                EXIT_PHASE_RUNNING,
                EXIT_PHASE_RESTARTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn shutdown_mode(&self) -> Option<ShutdownMode> {
        match self.phase.load(Ordering::Acquire) {
            EXIT_PHASE_SHUTTING_DOWN if self.restart_requested.load(Ordering::Acquire) => {
                Some(ShutdownMode::Restart)
            }
            EXIT_PHASE_SHUTTING_DOWN => Some(ShutdownMode::Normal),
            EXIT_PHASE_RESTARTING => Some(ShutdownMode::Restart),
            _ => None,
        }
    }

    fn claim_shutdown_start(&self) -> bool {
        self.shutdown_mode().is_some()
            && !self.shutdown_started.swap(true, Ordering::AcqRel)
    }

    fn mark_cleanup_verified(&self) {
        self.cleanup_verified.store(true, Ordering::Release);
        if self.phase.load(Ordering::Acquire) != EXIT_PHASE_FATAL {
            self.phase.store(EXIT_PHASE_COMPLETE, Ordering::Release);
        }
    }

    fn mark_cleanup_failed(&self) {
        self.cleanup_verified.store(false, Ordering::Release);
        if !matches!(
            self.phase.load(Ordering::Acquire),
            EXIT_PHASE_SHUTTING_DOWN | EXIT_PHASE_RESTARTING | EXIT_PHASE_FATAL
        ) {
            self.phase
                .store(EXIT_PHASE_CLEANUP_FAILED, Ordering::Release);
        }
        self.shutdown_started.store(false, Ordering::Release);
    }

    fn claim_fatal_exit(&self) -> bool {
        if self.restart_requested.load(Ordering::Acquire) {
            return false;
        }
        if !self.cleanup_verified.load(Ordering::Acquire) {
            return false;
        }
        if self.fatal_dialog_started.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.phase.store(EXIT_PHASE_FATAL, Ordering::Release);
        true
    }

    fn is_exit_allowed(&self) -> bool {
        self.cleanup_verified.load(Ordering::Acquire)
            && matches!(
                self.phase.load(Ordering::Acquire),
                EXIT_PHASE_COMPLETE | EXIT_PHASE_FATAL
            )
    }

    fn is_restart_requested(&self) -> bool {
        self.restart_requested.load(Ordering::Acquire)
    }

    fn has_pending_shutdown(&self) -> bool {
        self.shutdown_mode().is_some()
            || self.phase.load(Ordering::Acquire) == EXIT_PHASE_CLEANUP_FAILED
    }

    fn original_code(&self) -> i32 {
        self.original_code
            .lock()
            .ok()
            .and_then(|code| *code)
            .unwrap_or(0)
    }

    fn mark_no_cleanup_needed(&self) {
        self.mark_cleanup_verified();
    }

    /// Record that the backend thread failed before it could ever enter
    /// `DesktopServer::start`.  This is deliberately stronger than merely
    /// marking the backend as failed: it also proves that no startup cleanup
    /// authority was needed.
    fn mark_backend_not_started(&self) {
        self.mark_no_cleanup_needed();
        self.mark_backend_failed_verified();
    }

    /// Hide a server from all exit/request observers before a potentially
    /// blocking cleanup attempt begins.  Retain the handle separately in the
    /// failed state so retries still have authoritative ownership.
    fn mark_backend_cleanup_pending(&self, server: Arc<DesktopServer>) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(
            *backend,
            BackendRegistration::Starting | BackendRegistration::Ready(_)
        ) {
            *backend = BackendRegistration::FailedRetained(server);
        }
        self.backend_changed.notify_all();
    }

    fn register_backend(&self, server: Arc<DesktopServer>) -> bool {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(*backend, BackendRegistration::Starting) {
            *backend = BackendRegistration::Ready(server);
            self.backend_changed.notify_all();
            true
        } else {
            false
        }
    }

    fn retain_backend_runtime(&self, runtime: tokio::runtime::Runtime) {
        self.retained_backend_runtimes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(runtime);
    }

    fn release_backend_runtimes(&self) {
        let runtimes = {
            let mut retained = self
                .retained_backend_runtimes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *retained)
        };
        drop(runtimes);
    }

    fn retain_startup_cleanup_authority(
        &self,
        keep_alive: Arc<DesktopKeepAlive>,
        runtime: tokio::runtime::Runtime,
    ) -> Arc<RetainedStartupCleanupAuthority> {
        let authority = Arc::new(RetainedStartupCleanupAuthority::new(keep_alive, runtime));
        *self
            .retained_startup_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(authority.clone());
        authority
    }

    fn retained_startup_cleanup_authority(&self) -> Option<Arc<RetainedStartupCleanupAuthority>> {
        self.retained_startup_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn release_startup_cleanup_authority(
        &self,
        authority: &Arc<RetainedStartupCleanupAuthority>,
    ) {
        let mut retained = self
            .retained_startup_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retained
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, authority))
        {
            *retained = None;
        }
    }

    fn mark_backend_stopped_verified(&self) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(
            *backend,
            BackendRegistration::Starting | BackendRegistration::Ready(_)
        ) {
            *backend = BackendRegistration::StoppedVerified;
        }
        self.backend_changed.notify_all();
    }

    fn mark_backend_failed_verified(&self) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !matches!(*backend, BackendRegistration::StoppedVerified) {
            *backend = BackendRegistration::FailedVerified;
        }
        self.backend_changed.notify_all();
    }

    fn mark_backend_failed_retained(&self, server: Arc<DesktopServer>) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(
            *backend,
            BackendRegistration::Starting | BackendRegistration::Ready(_)
        ) {
            *backend = BackendRegistration::FailedRetained(server);
        }
        self.backend_changed.notify_all();
    }

    fn mark_backend_failed_unverified(&self) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(
            *backend,
            BackendRegistration::Starting | BackendRegistration::Ready(_)
        ) {
            *backend = BackendRegistration::FailedUnverified;
        }
        self.backend_changed.notify_all();
    }

    fn backend_server(&self) -> Option<Arc<DesktopServer>> {
        let backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*backend {
            BackendRegistration::Ready(server) | BackendRegistration::FailedRetained(server) => {
                Some(server.clone())
            }
            BackendRegistration::Starting
            | BackendRegistration::StoppedVerified
            | BackendRegistration::FailedVerified
            | BackendRegistration::FailedUnverified => None,
        }
    }

    fn wait_for_backend(&self, timeout: Duration) -> Option<Arc<DesktopServer>> {
        let deadline = Instant::now() + timeout;
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match &*backend {
                BackendRegistration::Ready(server)
                | BackendRegistration::FailedRetained(server) => return Some(server.clone()),
                BackendRegistration::StoppedVerified
                | BackendRegistration::FailedVerified
                | BackendRegistration::FailedUnverified => return None,
                BackendRegistration::Starting => {}
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let (next, wait_result) = self
                .backend_changed
                .wait_timeout(backend, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            backend = next;
            if wait_result.timed_out() && matches!(*backend, BackendRegistration::Starting) {
                return None;
            }
        }
    }
}

/// A restart request is a hard Tauri sentinel: unlike a normal exit it cannot
/// be cancelled with `prevent_exit`. Cleanup is therefore attempted (and
/// retried) synchronously before control returns to Tauri — but the retry is
/// BOUNDED: this path runs on the Tauri main thread, so an unbounded retry
/// against a wedged Chromium froze the UI forever and, because the
/// post-update `relaunch()` funnels into it, defeated the updater's own F32
/// bound. After [`RESTART_CLEANUP_MAX_ATTEMPTS`] the relaunch proceeds with a
/// loud error (bounded-then-proceed, the F32/F42/F33 pattern); transient
/// failures within the bound still retain the `DesktopServer` and runtime
/// authority.
fn cleanup_for_restart_until_verified(
    server: Arc<DesktopServer>,
    coordinator: &ExitCoordinator,
) {
    if restart_cleanup_bounded(coordinator, "backend shutdown", || {
        server.shutdown_all_blocking()
    }) {
        coordinator.release_backend_runtimes();
        coordinator.mark_cleanup_verified();
        coordinator.mark_backend_stopped_verified();
        tracing::info!("desktop restart cleanup verified");
    }
}

fn cleanup_startup_for_restart_until_verified(
    authority: Arc<RetainedStartupCleanupAuthority>,
    coordinator: &ExitCoordinator,
) {
    if restart_cleanup_bounded(coordinator, "startup cleanup", || authority.cleanup()) {
        coordinator.release_startup_cleanup_authority(&authority);
        coordinator.mark_cleanup_verified();
        coordinator.mark_backend_failed_verified();
        tracing::info!("desktop startup cleanup verified before restart");
    }
}

/// Bounded synchronous restart-cleanup attempts (250ms between attempts; each
/// attempt is itself bounded — `shutdown_all_blocking` runs two ≤31s tries).
/// The cleanup preflight semantics stay: cleanup is always attempted, and
/// only after exhausting these attempts does the restart handoff continue
/// with a loud error instead of freezing the main thread forever.
const RESTART_CLEANUP_MAX_ATTEMPTS: u64 = 4;

/// Run the bounded restart-cleanup attempts and return whether cleanup was
/// positively verified. On exhaustion this forces the handoff to proceed
/// (see [`allow_handoff_without_verified_cleanup`]) after logging loudly;
/// the caller only performs its success-path bookkeeping on `true`.
fn restart_cleanup_bounded<S>(
    coordinator: &ExitCoordinator,
    stage: &'static str,
    cleanup: S,
) -> bool
where
    S: FnMut() -> anyhow::Result<()>,
{
    let verified = cleanup_until_verified_bounded(
        cleanup,
        |attempt, error| {
            coordinator.mark_cleanup_failed();
            tracing::error!(
                attempt,
                %error,
                stage,
                "desktop restart cleanup is not verified; retaining authority and retrying"
            );
        },
        || std::thread::sleep(Duration::from_millis(250)),
        RESTART_CLEANUP_MAX_ATTEMPTS,
    );
    if !verified {
        tracing::error!(
            attempts = RESTART_CLEANUP_MAX_ATTEMPTS,
            stage,
            "proceeding with the restart handoff after exhausting bounded cleanup \
             attempts; desktop cleanup is NOT verified"
        );
        allow_handoff_without_verified_cleanup(coordinator);
    }
    verified
}

/// Force a pending exit/restart handoff to proceed after bounded cleanup
/// attempts were exhausted (or no cleanup authority ever became available).
/// Marks cleanup verified — the coordinator's "may proceed" signal — even
/// though the backend cleanup is NOT verified, and returns the exit code to
/// use. Callers must log loudly BEFORE calling this: an unkillable or
/// permanently frozen app is strictly worse than an unclean close (the data
/// layer is crash-safe).
fn allow_handoff_without_verified_cleanup(coordinator: &ExitCoordinator) -> i32 {
    coordinator.mark_backend_failed_unverified();
    coordinator.mark_cleanup_verified();
    coordinator.original_code()
}

/// A restart request is a non-cancellable Tauri sentinel.  If the first
/// cleanup attempt fails, do not return to Tauri immediately and do not call
/// `process::exit`: retain either the published server or typed startup
/// authority and retry within the bounded restart-cleanup attempts. An actual
/// authority loss falls back to the bounded no-authority hold; every path
/// eventually proceeds with the handoff (loudly) instead of wedging the
/// process forever.
fn abort_restart(
    app: &tauri::AppHandle,
    coordinator: &ExitCoordinator,
    reason: impl std::fmt::Display,
) {
    coordinator.mark_cleanup_failed();
    tracing::error!(%reason, "desktop restart cleanup is not verified; retrying before handoff");

    let server = app
        .try_state::<Arc<DesktopServer>>()
        .map(|state| state.inner().clone())
        .or_else(|| coordinator.backend_server())
        .or_else(|| coordinator.wait_for_backend(Duration::from_secs(30)));

    if let Some(server) = server {
        cleanup_for_restart_until_verified(server, coordinator);
    } else if let Some(authority) = coordinator.retained_startup_cleanup_authority() {
        cleanup_startup_for_restart_until_verified(authority, coordinator);
    } else {
        hold_restart_without_cleanup_authority(coordinator);
    }
}

/// How long a restart/exit request may be held while no cleanup authority is
/// available, waiting for a late-arriving backend registration. After this
/// bound the handoff proceeds with a loud error — the previous unbounded hold
/// parked the process forever with zero user-visible progress.
const RESTART_HOLD_WITHOUT_AUTHORITY_WAIT: Duration = Duration::from_secs(30);

fn hold_restart_without_cleanup_authority(coordinator: &ExitCoordinator) {
    coordinator.mark_cleanup_failed();
    tracing::error!(
        "restart request is being held because desktop cleanup authority is unavailable"
    );
    // Within the bounded hold, keep ATTEMPTING to acquire a late-arriving
    // cleanup authority instead of sleeping blindly. A terminal backend
    // registration (stopped/failed) resolves immediately: no authority can
    // appear anymore.
    if let Some(server) = coordinator.wait_for_backend(RESTART_HOLD_WITHOUT_AUTHORITY_WAIT) {
        cleanup_for_restart_until_verified(server, coordinator);
        return;
    }
    if let Some(authority) = coordinator.retained_startup_cleanup_authority() {
        cleanup_startup_for_restart_until_verified(authority, coordinator);
        return;
    }
    tracing::error!(
        wait_secs = RESTART_HOLD_WITHOUT_AUTHORITY_WAIT.as_secs(),
        "no desktop cleanup authority became available; proceeding with the exit/restart \
         handoff without cleanup verification"
    );
    allow_handoff_without_verified_cleanup(coordinator);
}

/// How long a stranded normal-exit request waits for the backend to leave the
/// `Starting` registration before it forces the exit (matches the restart
/// path's `wait_for_backend` bound).
const DEFERRED_EXIT_BACKEND_WAIT: Duration = Duration::from_secs(30);

/// F42: complete a normal exit whose backend was unavailable when
/// `ExitRequested` fired. Runs off the Tauri event loop. If the backend
/// becomes available the ordinary verified shutdown runs; if it never does
/// (stuck in `DesktopServer::start`, or already stopped/failed), the exit
/// proceeds with a loud error — an unkillable app that silently swallows
/// every quit gesture is strictly worse than an unclean close (the data
/// layer is crash-safe), and there is no cleanup authority this path could
/// have used anyway.
fn spawn_deferred_exit_shutdown(app: tauri::AppHandle, coordinator: Arc<ExitCoordinator>) {
    if coordinator
        .deferred_exit_wait_started
        .swap(true, Ordering::AcqRel)
    {
        return;
    }
    std::thread::spawn(move || {
        if let Some(server) = coordinator.wait_for_backend(DEFERRED_EXIT_BACKEND_WAIT) {
            start_shutdown_if_needed(&app, server, coordinator);
            return;
        }
        if coordinator.is_exit_allowed() {
            app.exit(coordinator.original_code());
            return;
        }
        tracing::error!(
            wait_secs = DEFERRED_EXIT_BACKEND_WAIT.as_secs(),
            "embedded backend never became available for exit cleanup; forcing exit without \
             backend cleanup"
        );
        let code = allow_exit_without_backend_cleanup(&coordinator);
        app.exit(code);
    });
}

/// Mark the exit as allowed even though no backend cleanup could run, and
/// return the exit code to use. Only the deferred-exit path may call this,
/// and only after positively establishing that no cleanup authority exists.
fn allow_exit_without_backend_cleanup(coordinator: &ExitCoordinator) -> i32 {
    coordinator.mark_cleanup_verified();
    coordinator.original_code()
}

fn start_shutdown_if_needed(
    app: &tauri::AppHandle,
    server: Arc<DesktopServer>,
    coordinator: Arc<ExitCoordinator>,
) {
    if !coordinator.claim_shutdown_start() {
        return;
    }

    let app = app.clone();
    let callback_coordinator = coordinator.clone();
    server.clone().shutdown_all_async(move |result| {
        match result {
            Ok(()) => {
                // Tauri's restart request already owns the process handoff. Calling
                // `app.exit` here would replace its RESTART_EXIT_CODE with a normal
                // exit, so only normal exits are completed by this callback.
                let restart = callback_coordinator.is_restart_requested();
                let original_code = callback_coordinator.original_code();
                callback_coordinator.mark_cleanup_verified();
                callback_coordinator.mark_backend_stopped_verified();
                if !restart {
                    app.exit(original_code);
                }
            }
            Err(error) => {
                tracing::error!(%error, "desktop backend exit cleanup failed");
                let retry_server = server.clone();
                let retry_app = app.clone();
                let retry_coordinator = callback_coordinator.clone();
                retry_server.clone().shutdown_all_async(move |retry_result| {
                    match retry_result {
                        Ok(()) => {
                            let restart = retry_coordinator.is_restart_requested();
                            retry_coordinator.mark_cleanup_verified();
                            retry_coordinator.mark_backend_stopped_verified();
                            if !restart {
                                retry_app.exit(retry_coordinator.original_code());
                            }
                        }
                        Err(retry_error) => {
                            tracing::error!(
                                first_error = %error,
                                error = %retry_error,
                                "desktop backend exit cleanup failed after retry"
                            );
                            retry_coordinator.mark_cleanup_failed();
                            retry_coordinator.mark_backend_failed_retained(server.clone());
                            tracing::error!(
                                %retry_error,
                                "cleanup authority is retained; refusing to report a successful exit"
                            );
                            retry_shutdown_until_verified(
                                retry_app,
                                server.clone(),
                                retry_coordinator,
                            );
                        }
                    }
                });
            }
        }
    });
}

/// Bounded retries for the spawned exit-cleanup thread (a normal quit whose
/// two async shutdown attempts failed, and the backend-failure teardown
/// path). `prevent_exit` has already swallowed the user's quit gesture by the
/// time this runs, so an unbounded retry against a wedged Chromium made the
/// app permanently unkillable. One second between attempts; each attempt is
/// itself bounded (`shutdown_all_blocking` runs two ≤31s tries).
const EXIT_SHUTDOWN_RETRY_MAX_ATTEMPTS: u64 = 4;

fn retry_shutdown_until_verified(
    app: tauri::AppHandle,
    server: Arc<DesktopServer>,
    coordinator: Arc<ExitCoordinator>,
) {
    std::thread::spawn(move || {
        let verified = cleanup_until_verified_bounded(
            || server.shutdown_all_blocking(),
            |attempt, error| {
                coordinator.mark_cleanup_failed();
                tracing::error!(
                    attempt,
                    %error,
                    "desktop cleanup retry is still not verified; retaining authority"
                );
            },
            || std::thread::sleep(Duration::from_secs(1)),
            EXIT_SHUTDOWN_RETRY_MAX_ATTEMPTS,
        );
        if verified {
            let restart = coordinator.is_restart_requested();
            let code = coordinator.original_code();
            coordinator.mark_cleanup_verified();
            coordinator.mark_backend_failed_verified();
            if !restart {
                app.exit(code);
            }
            coordinator.release_backend_runtimes();
            return;
        }
        // F42 sibling: after prevent_exit swallowed the quit gesture, this
        // thread is the only thing that can still complete the exit. Proceed
        // with a loud error instead of retrying forever — an unkillable app
        // is strictly worse than an unclean close.
        tracing::error!(
            attempts = EXIT_SHUTDOWN_RETRY_MAX_ATTEMPTS,
            "desktop exit cleanup never verified after bounded retries; forcing the exit \
             without verified cleanup"
        );
        let restart = coordinator.is_restart_requested();
        let code = allow_handoff_without_verified_cleanup(&coordinator);
        if !restart {
            app.exit(code);
        }
    });
}

fn retry_startup_cleanup_until_verified(
    app: tauri::AppHandle,
    authority: Arc<RetainedStartupCleanupAuthority>,
    coordinator: Arc<ExitCoordinator>,
    message: String,
) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        match authority.cleanup() {
            Ok(()) => {
                coordinator.release_startup_cleanup_authority(&authority);
                coordinator.mark_cleanup_verified();
                coordinator.mark_backend_failed_verified();
                schedule_fatal_dialog(app, coordinator, message);
                return;
            }
            Err(error) => {
                coordinator.mark_cleanup_failed();
                tracing::error!(
                    %error,
                    "desktop startup cleanup retry is still not verified; retaining authority"
                );
            }
        }
    });
}

fn show_fatal_dialog_on_main_thread(
    app: tauri::AppHandle,
    message: String,
    coordinator: Arc<ExitCoordinator>,
) {
    if !coordinator.claim_fatal_exit() {
        return;
    }

    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    let exit_app = app.clone();
    let exit_coordinator = coordinator.clone();
    app
        .dialog()
        .message(message)
        .title("NomiFun backend unavailable")
        .kind(MessageDialogKind::Error)
        .show(move |_| {
            if exit_coordinator.is_exit_allowed() {
                exit_app.exit(1);
            } else {
                tracing::error!(
                    "fatal dialog completed before cleanup verification; refusing to exit"
                );
            }
        });
}

fn schedule_fatal_dialog(
    app: tauri::AppHandle,
    coordinator: Arc<ExitCoordinator>,
    message: String,
) {
    let task_app = app.clone();
    let task_coordinator = coordinator.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        show_fatal_dialog_on_main_thread(task_app, message, task_coordinator);
    }) {
        tracing::error!(%error, "failed to dispatch backend failure dialog to the main thread");
        tracing::error!(
            "fatal dialog dispatch failed; cleanup remains fail-closed and the process is not exited"
        );
    }
}

fn shutdown_then_show_fatal(
    app: tauri::AppHandle,
    server: Arc<DesktopServer>,
    coordinator: Arc<ExitCoordinator>,
    message: String,
) {
    let cleanup_server = server.clone();
    let retry_server = server.clone();
    cleanup_server.cleanup_all_async(move |result| match result {
        Ok(()) => {
            coordinator.mark_cleanup_verified();
            coordinator.mark_backend_failed_verified();
            schedule_fatal_dialog(app, coordinator, message);
        }
        Err(error) => {
            coordinator.mark_cleanup_failed();
            coordinator.mark_backend_failed_retained(retry_server.clone());
            tracing::error!(
                %error,
                "desktop backend cleanup after setup failure failed; retaining authority"
            );
            retry_cleanup_then_show_fatal(app, retry_server, coordinator, message);
        }
    });
}

fn retry_cleanup_then_show_fatal(
    app: tauri::AppHandle,
    server: Arc<DesktopServer>,
    coordinator: Arc<ExitCoordinator>,
    message: String,
) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        match server.shutdown_all_blocking() {
            Ok(()) => {
                coordinator.mark_cleanup_verified();
                coordinator.mark_backend_failed_verified();
                coordinator.release_backend_runtimes();
                schedule_fatal_dialog(app, coordinator, message);
                return;
            }
            Err(error) => {
                coordinator.mark_cleanup_failed();
                coordinator.mark_backend_failed_retained(server.clone());
                tracing::error!(
                    %error,
                    "setup-failure cleanup retry is still not verified; retaining authority"
                );
            }
        }
    });
}

struct BackendRuntimeFailure<R> {
    error: String,
    runtime: Option<R>,
}

fn backend_run_failure(
    run: std::thread::Result<anyhow::Result<()>>,
) -> Option<String> {
    match run {
        Ok(Ok(())) => return None,
        Ok(Err(error)) => Some(format!("{error:#}")),
        Err(panic) => panic
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .or_else(|| Some("backend thread panicked".to_owned())),
    }
}

fn finish_backend_runtime<R, F>(
    run: std::thread::Result<anyhow::Result<()>>,
    runtime: R,
    cleanup: F,
) -> Option<BackendRuntimeFailure<R>>
where
    F: FnOnce(&R) -> anyhow::Result<()>,
{
    let mut error = backend_run_failure(run)?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cleanup(&runtime))) {
        Ok(Ok(())) => {
            drop(runtime);
            Some(BackendRuntimeFailure {
                error,
                runtime: None,
            })
        }
        Ok(Err(cleanup_error)) => {
            error = format!("{error}; backend cleanup also failed: {cleanup_error:#}");
            Some(BackendRuntimeFailure {
                error,
                runtime: Some(runtime),
            })
        }
        Err(panic) => {
            let panic_error = backend_run_failure(Err(panic))
                .unwrap_or_else(|| "backend cleanup panicked".to_owned());
            error = format!("{error}; backend cleanup panicked: {panic_error}");
            Some(BackendRuntimeFailure {
                error,
                runtime: Some(runtime),
            })
        }
    }
}

fn complete_main_thread_setup(
    app: tauri::AppHandle,
    server: Arc<DesktopServer>,
    coordinator: Arc<ExitCoordinator>,
    pairing_data_dir: PathBuf,
) -> anyhow::Result<()> {
    if let Some(error) = server.current_failure() {
        return Err(anyhow::anyhow!("embedded backend failed before window setup: {error}"));
    }

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
    if !app.manage(server.clone()) {
        return Err(anyhow::anyhow!(
            "desktop backend state was already registered"
        ));
    }
    let pairing_manager = Arc::new(
        relay_pairing::RelayPairingManager::new(pairing_data_dir)
            .context("failed to initialize Relay pairing state")?,
    );
    if !app.manage(pairing_manager.clone()) {
        return Err(anyhow::anyhow!(
            "Relay pairing state was already registered"
        ));
    }
    // Restore the managed agent after the backend/webview state is published.
    // The restore path is best-effort: a transient Relay outage leaves the
    // persisted metadata visible and nfagent can be restarted from the UI.
    let restore_server = server.clone();
    tauri::async_runtime::spawn(async move {
        pairing_manager.restore(restore_server).await;
    });
    if coordinator.has_pending_shutdown() {
        if coordinator.is_restart_requested() {
            // Tauri deliberately ignores `prevent_exit` for the restart
            // sentinel.  Complete cleanup before allowing the restart event
            // to hand control back to Tauri.
            let result = server.shutdown_all_blocking();
            match restart_cleanup_outcome(&result) {
                RestartCleanupOutcome::ContinueRestart => {
                    coordinator.mark_cleanup_verified();
                    coordinator.mark_backend_stopped_verified();
                }
                RestartCleanupOutcome::AbortRestart => {
                    let error = result
                        .err()
                        .unwrap_or_else(|| anyhow::anyhow!("desktop restart cleanup failed"));
                    abort_restart(
                        &app,
                        &coordinator,
                        format!("desktop restart cleanup failed: {error:#}"),
                    );
                }
            }
        } else {
            start_shutdown_if_needed(&app, server, coordinator);
        }
        return Ok(());
    }
    let win_builder =
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
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
    if let Some(error) = server.current_failure() {
        return Err(anyhow::anyhow!(
            "embedded backend failed during window setup: {error}"
        ));
    }
    win_builder.build()?;

    // System tray. Closing the main window HIDES it here instead of
    // quitting (see the CloseRequested handler in on_window_event); the
    // process truly exits only via the tray's "退出" item. Left-click the
    // icon to bring the window back; right-click for the Show/Quit menu.
    // Labels are English fallbacks, adopted from the renderer's locale via
    // `set_tray_labels` once it mounts (the renderer always loads before
    // the user can close, so the first menu open is already localized).
    let tray_show = MenuItem::with_id(&app, "tray-show", "Show NomiFun", true, None::<&str>)?;
    let tray_quit = MenuItem::with_id(&app, "tray-quit", "Quit", true, None::<&str>)?;
    let tray_menu = Menu::with_items(&app, &[&tray_show, &tray_quit])?;
    if !app.manage(TrayMenuItems {
        show: tray_show.clone(),
        quit: tray_quit.clone(),
    }) {
        return Err(anyhow::anyhow!(
            "tray menu state was already registered"
        ));
    }
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
    tray_builder.build(&app)?;

    // Desktop-companion windows are NOT created here anymore. They are
    // multi-companion and dynamic: the main window's useCompanionWindowsSync hook
    // invokes `sync_companion_windows` (above) on boot and on companion
    // created/deleted/config-updated events, reconciling one
    // transparent always-on-top `companion-{companion_id}` window per enabled companion.

    // Wire deep-link open-url events to a Tauri event the renderer can
    // `listen()` to. `register_all()` is best-effort (some platforms / dev
    // contexts need it; ignore the error if it fails).
    let handle = app.clone();
    let _ = app.deep_link().register_all();
    app.deep_link().on_open_url(move |event| {
        let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
        let _ = handle.emit("deep-link://received", urls);
    });
    Ok(())
}

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

#[cfg(any(test, target_os = "macos"))]
fn should_show_main_window_for_macos_reopen(_has_visible_windows: bool) -> bool {
    true
}

fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            let Some(coordinator) = app
                .try_state::<Arc<ExitCoordinator>>()
                .map(|state| state.inner().clone())
            else {
                return;
            };

            if code == Some(tauri::RESTART_EXIT_CODE) {
                // Tauri documents that prevent_exit() is ignored for the
                // restart sentinel. Mark the window as explicitly quitting
                // and start cleanup immediately; the Tauri restart machinery
                // retains ownership of the final process handoff.
                app.state::<QuitFlag>().0.store(true, Ordering::SeqCst);
                coordinator.request_restart();
                if let Some(server) = app
                    .try_state::<Arc<DesktopServer>>()
                    .map(|state| state.inner().clone())
                    .or_else(|| coordinator.backend_server())
                    .or_else(|| coordinator.wait_for_backend(Duration::from_secs(30)))
                {
                    let result = server.shutdown_all_blocking();
                    match restart_cleanup_outcome(&result) {
                        RestartCleanupOutcome::ContinueRestart => {
                            coordinator.mark_cleanup_verified();
                            coordinator.mark_backend_stopped_verified();
                        }
                        RestartCleanupOutcome::AbortRestart => {
                            let error = result.err().unwrap_or_else(|| {
                                anyhow::anyhow!("desktop restart cleanup failed")
                            });
                            abort_restart(
                                app,
                                &coordinator,
                                format!("desktop restart cleanup failed: {error:#}"),
                            );
                        }
                    }
                } else {
                    abort_restart(
                        app,
                        &coordinator,
                        "embedded backend did not become available within 30 seconds",
                    );
                }
                return;
            }

            if coordinator.is_exit_allowed() {
                return;
            }

            // Keep every normal request blocked until the async cleanup
            // callback has requested the final exit. This is intentionally
            // never the blocking shutdown_all_blocking path.
            api.prevent_exit();
            app.state::<QuitFlag>().0.store(true, Ordering::SeqCst);
            coordinator.request_normal_exit(code);
            if let Some(server) = app
                .try_state::<Arc<DesktopServer>>()
                .map(|state| state.inner().clone())
                .or_else(|| coordinator.backend_server())
            {
                start_shutdown_if_needed(app, server, coordinator);
            } else {
                // F42: the backend may be wedged inside DesktopServer::start
                // (registration stuck at Starting), and prevent_exit above has
                // already swallowed this quit gesture. Never strand it: wait
                // for the backend off the event loop, then either run the
                // normal shutdown or force the exit with a loud error.
                spawn_deferred_exit_shutdown(app.clone(), coordinator);
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
    specs: Vec<CompanionWindowSpec>,
) -> Result<(), String> {
    let init_script = webui_init_script(server.loopback_port(), server.local_trust_secret());
    let app_for_task = app.clone();
    run_on_main_thread_task(
        move |task| app.run_on_main_thread(task).map_err(|e| e.to_string()),
        move || reconcile_companion_windows(app_for_task, init_script, specs),
    )
    .await
}

fn reconcile_companion_windows(
    app: tauri::AppHandle,
    init_script: String,
    specs: Vec<CompanionWindowSpec>,
) -> Result<(), String> {
    use std::collections::HashSet;

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

fn main() -> std::process::ExitCode {
    // If a terminal agent CLI spawned this shell as an MCP stdio bridge
    // (`current_exe() mcp-requirement-stdio` etc.), run that helper and exit
    // BEFORE any runtime init, single-instance handling, or window creation.
    // Every host binary must honor these or the injected declaration tools
    // (requirement_complete / team / guide) never appear in the agent's session.
    if let Some(code) = nomifun_app::commands::run_mcp_stdio_subcommand_if_present() {
        return code;
    }

    // Env mutation + runtime init BEFORE Tauri builds its runtime/threads,
    // mirroring the nomicore bin's ordering. `default_data_dir` resolves the
    // effective root (literal NOMIFUN_DATA_DIR or the channel default) and
    // runs the one-shot legacy layout migration (`NomiFun/Nomi` → `NomiFun`,
    // see nomifun_app::bootstrap::resolve_startup_data_root). v3 remains a
    // hard dataset cut for the historical pre-v3 temp-rooted dataset: the
    // backend quarantines any incompatible dataset found at the current root.
    let data_dir = default_data_dir();
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

    // F48: publish Tauri's authoritative resource-dir resolution for the
    // backend's bundled Chrome-for-Testing discovery. macOS .app bundles place
    // resources in Contents/Resources while the executable lives in
    // Contents/MacOS, so the backend's exe-relative fallback alone can never
    // see a packaged Chrome there. The backend crate has no Tauri dependency;
    // this env var is the seam (see nomifun_app::browser_resource).
    let tauri_context = generated_tauri_context();
    if let Ok(resource_dir) = tauri::utils::platform::resource_dir(
        tauri_context.package_info(),
        &tauri::Env::default(),
    ) {
        // SAFETY: same single-threaded window as `enhance_process_path`
        // above — Tauri's runtime threads are only created by `.run()`.
        unsafe {
            std::env::set_var(
                nomifun_app::browser_resource::BUNDLED_CHROME_DIR_ENV,
                resource_dir.join("chrome-for-testing"),
            );
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
            let app_handle = app.handle().clone();
            let coordinator = app.state::<Arc<ExitCoordinator>>().inner().clone();

            // In dev, the desktop webview loads the live Vite server; the LAN
            // listener must proxy to the same source instead of stale assets.
            // In production, the embedded assets below are canonical.
            let dev_frontend_url: Option<String> = if tauri::is_dev() {
                app.config()
                    .build
                    .dev_url
                    .as_ref()
                    .map(|url| url.to_string())
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
                    match resolve_embedded_webui_assets(app) {
                        Ok(source) => source,
                        Err(error) => {
                            coordinator.mark_no_cleanup_needed();
                            coordinator.mark_backend_failed_verified();
                            let message = format!(
                                "NomiFun could not resolve its embedded WebUI assets: {error:#}"
                            );
                            tracing::error!(%error, "failed to resolve embedded desktop WebUI assets");
                            schedule_fatal_dialog(app_handle, coordinator, message);
                            return Ok(());
                        }
                    }
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
                match resolve_webui_spa_dir(app, explicit_dist_override.as_deref()) {
                    Ok(spa_dir) => spa_dir,
                    Err(error) => {
                        coordinator.mark_no_cleanup_needed();
                        coordinator.mark_backend_failed_verified();
                        let message =
                            format!("NomiFun could not resolve its bundled WebUI assets: {error:#}");
                        tracing::error!(%error, "failed to resolve desktop WebUI assets");
                        schedule_fatal_dialog(app_handle, coordinator, message);
                        return Ok(());
                    }
                }
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

            // Backend startup is intentionally asynchronous. Blocking this
            // setup callback would block Tauri's event loop and prevent both
            // startup-failure dialogs and exit requests from being processed.
            let status_emit_handle = app_handle.clone();
            let setup_app_handle = app_handle.clone();
            let failure_app_handle = app_handle.clone();
            let supervisor_coordinator = coordinator.clone();
            let failure_coordinator = coordinator.clone();
            let spawn_result = std::thread::Builder::new()
                .name("nomifun-backend".into())
                .spawn(move || {
                    let startup_cleanup = Arc::new(Mutex::new(StartupCleanup::NotStarted));
                    let startup_cleanup_for_run = Arc::clone(&startup_cleanup);
                    let runtime = match tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            failure_coordinator.mark_backend_not_started();
                            let error = format!("failed to build backend runtime: {error}");
                            tracing::error!(error = %error, "embedded backend exited with error");
                            schedule_fatal_dialog(
                                failure_app_handle,
                                failure_coordinator,
                                error,
                            );
                            return;
                        }
                    };
                    mark_startup_cleanup_entered(&startup_cleanup);
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || -> anyhow::Result<()> {
                            runtime.block_on(async move {
                                let (server, keep_alive) = match DesktopServer::start_with_outcome(
                                    &cli,
                                    &merged_path,
                                    spa_dir,
                                    dev_frontend_url,
                                    webui_asset_source,
                                )
                                .await
                                {
                                    Ok(started) => started,
                                    Err(mut error) => {
                                        match error.cleanup_disposition() {
                                            StartupCleanupDisposition::Verified => {
                                                mark_startup_cleanup_failed_verified(
                                                    &startup_cleanup_for_run,
                                                );
                                            }
                                            StartupCleanupDisposition::Unverified => {
                                                if let Some(keep_alive) =
                                                    error.take_retained_keep_alive()
                                                {
                                                    mark_startup_cleanup_retained(
                                                        &startup_cleanup_for_run,
                                                        Arc::new(keep_alive),
                                                    );
                                                } else {
                                                    tracing::error!(
                                                        "desktop startup reported unverified cleanup without a retained authority"
                                                    );
                                                }
                                            }
                                        }
                                        return Err(error.into_inner());
                                    }
                                };
                                mark_startup_cleanup_server(
                                    &startup_cleanup_for_run,
                                    server.clone(),
                                );
                                let mut status_rx = server.subscribe_status();
                                let mut failure_rx = server.subscribe_failure();
                                let mut shutdown_rx = server.subscribe_shutdown();
                                if let Some(error) = failure_rx.borrow_and_update().clone() {
                                    return Err(anyhow::Error::msg(error));
                                }
                                if !supervisor_coordinator.register_backend(server.clone()) {
                                    return Err(anyhow::anyhow!(
                                        "embedded backend completed startup after its registration was already closed"
                                    ));
                                }

                                let setup_app = setup_app_handle.clone();
                                let setup_server = server.clone();
                                let setup_coordinator = supervisor_coordinator.clone();
                                let setup_pairing_data_dir = cli.data_dir.clone();
                                if let Err(error) = setup_app_handle.run_on_main_thread(move || {
                                    let setup_coordinator_for_failure = setup_coordinator.clone();
                                    if let Err(error) = complete_main_thread_setup(
                                        setup_app.clone(),
                                        setup_server.clone(),
                                        setup_coordinator,
                                        setup_pairing_data_dir,
                                    ) {
                                        tracing::error!(
                                            error = %error,
                                            "failed to complete desktop window setup"
                                        );
                                        shutdown_then_show_fatal(
                                            setup_app,
                                            setup_server,
                                            setup_coordinator_for_failure,
                                            format!(
                                                "NomiFun could not initialize its desktop window: {error:#}"
                                            ),
                                        );
                                    }
                                }) {
                                    shutdown_then_show_fatal(
                                        setup_app_handle.clone(),
                                        server.clone(),
                                        supervisor_coordinator.clone(),
                                        format!(
                                            "NomiFun could not dispatch desktop window setup: {error}"
                                        ),
                                    );
                                }

                                if let Some(error) = failure_rx.borrow_and_update().clone() {
                                    return Err(anyhow::Error::msg(error));
                                }
                                if *shutdown_rx.borrow_and_update() {
                                    drop(keep_alive);
                                    return Ok(());
                                }
                                loop {
                                    tokio::select! {
                                        biased;
                                        changed = failure_rx.changed() => {
                                            changed.context(
                                                "desktop backend failure monitor closed unexpectedly",
                                            )?;
                                            if let Some(error) = failure_rx.borrow_and_update().clone() {
                                                return Err(anyhow::Error::msg(error));
                                            }
                                        }
                                        changed = shutdown_rx.changed() => {
                                            changed.context(
                                                "desktop backend shutdown monitor closed unexpectedly",
                                            )?;
                                            if *shutdown_rx.borrow_and_update() {
                                                drop(keep_alive);
                                                return Ok(());
                                            }
                                        }
                                        changed = status_rx.changed() => {
                                            changed.context(
                                                "desktop backend status monitor closed unexpectedly",
                                            )?;
                                            let status = status_rx.borrow_and_update().clone();
                                            let _ = status_emit_handle.emit(
                                                "webui://status-changed",
                                                status,
                                            );
                                        }
                                    }
                                }
                            })
                        },
                    ));
                    let startup_cleanup_snapshot = startup_cleanup
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    let Some(failure) =
                        finish_backend_runtime(run, runtime, |runtime| {
                            if let Some(server) = startup_cleanup_snapshot.server() {
                                failure_coordinator.mark_backend_cleanup_pending(server);
                            }
                            let result = startup_cleanup_snapshot.cleanup(runtime);
                            if result.is_ok() {
                                mark_startup_cleanup_failed_verified(&startup_cleanup);
                            }
                            result
                        })
                    else {
                        return;
                    };
                    let BackendRuntimeFailure { error, runtime } = failure;
                    let startup_cleanup = startup_cleanup
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    if let Some(runtime) = runtime {
                        failure_coordinator.mark_cleanup_failed();
                        tracing::error!(
                            error = %error,
                            "embedded backend cleanup is unverified; retaining runtime authority"
                        );
                        if let Some(server) = startup_cleanup.server() {
                            failure_coordinator.mark_backend_failed_retained(server.clone());
                            failure_coordinator.retain_backend_runtime(runtime);
                            retry_shutdown_until_verified(
                                failure_app_handle,
                                server,
                                failure_coordinator,
                            );
                        } else if let Some(keep_alive) = startup_cleanup.retained_keep_alive() {
                            let authority = failure_coordinator
                                .retain_startup_cleanup_authority(keep_alive, runtime);
                            failure_coordinator.mark_backend_failed_unverified();
                            retry_startup_cleanup_until_verified(
                                failure_app_handle,
                                authority,
                                failure_coordinator,
                                error,
                            );
                        } else {
                            failure_coordinator.retain_backend_runtime(runtime);
                            failure_coordinator.mark_backend_failed_unverified();
                            hold_restart_without_cleanup_authority(&failure_coordinator);
                        }
                    } else {
                        // The runtime is returned as `None` only when the
                        // cleanup closure completed successfully. That result,
                        // not the pre-cleanup enum variant, is the positive
                        // teardown proof.
                        failure_coordinator.mark_cleanup_verified();
                        failure_coordinator.mark_backend_failed_verified();
                        tracing::error!(error = %error, "embedded backend exited with error");
                        schedule_fatal_dialog(failure_app_handle, failure_coordinator, error);
                    }
                });
            if let Err(error) = spawn_result {
                coordinator.mark_backend_not_started();
                tracing::error!(%error, "failed to spawn embedded backend thread");
                schedule_fatal_dialog(
                    app_handle,
                    coordinator,
                    format!("NomiFun could not start its embedded backend thread: {error}"),
                );
            }
            Ok(())
        })
        // The ~38 OS-shell commands (window controls, tray, zoom, get-path,
        // feedback, auto-update status) register here as #[tauri::command]s (P3).
        .manage(AwakeState(Mutex::new(None)))
        .manage(QuitFlag(AtomicBool::new(false)))
        .manage(Arc::new(ExitCoordinator::default()))
        .manage(DownloadedUpdateState::default())
        .invoke_handler(tauri::generate_handler![
            download_update,
            install_update,
            update_package_status,
            companion_pointer::get_companion_local_pointer,
            updater_install_context::get_updater_install_context,
            sync_companion_windows,
            webui_get_status,
            webui_start,
            webui_stop,
            relay_pairing_get_status,
            relay_pairing_bootstrap,
            relay_pairing_restart,
            relay_pairing_stop,
            relay_pairing_disconnect,
            set_keep_awake,
            set_tray_labels
        ])
        // Close-to-tray is now the DEFAULT (and only) close behavior. Closing the
        // main window (titlebar ×, OS close, Alt+F4) hides it to the tray instead
        // of quitting — the agent, scheduled tasks, and companions keep running in
        // the background. The process exits ONLY via the tray's "退出" item, which
        // arms QuitFlag and calls app.exit(0); with the flag set we let the close
        // proceed. Process termination is coordinated exclusively by
        // `ExitRequested`, so window destruction cannot overwrite a restart
        // sentinel or bypass browser/backend cleanup.
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
                _ => {}
            }
        })
        .build(tauri_context)
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

    #[test]
    fn downloaded_update_cache_separates_download_from_install() {
        let cache = DownloadedUpdateCache::default();

        assert!(matches!(
            cache.begin_download("0.3.8", |_| 0),
            Ok(BeginUpdateDownload::Start)
        ));
        assert!(cache.begin_download("0.3.8", |_| 0).is_err());
        cache.finish_download("0.3.8", "verified bytes").unwrap();

        assert!(matches!(
            cache.begin_download("0.3.8", |_| 0),
            Ok(BeginUpdateDownload::AlreadyReady { .. })
        ));
        assert_eq!(cache.take_ready("0.3.8").unwrap(), "verified bytes");
        // While the installer is being handed the bytes the slot is claimed, so a
        // concurrent download or a second install must still be refused.
        assert!(cache.begin_download("0.3.8", |_| 0).is_err());
        assert!(cache.take_ready("0.3.8").is_err());
    }

    #[test]
    fn completed_install_releases_the_slot_for_the_next_update() {
        // `Installing` used to be a terminal sink: the only way out was an install
        // ERROR (restore_ready). On macOS/Linux `install()` returns Ok, so a
        // successful install parked the slot in Installing for the rest of the
        // process — after which EVERY later download was rejected with
        // "already being processed" and the user could not update again without
        // restarting the app.
        let cache = DownloadedUpdateCache::default();
        cache.begin_download("0.4.2", |_| 0).unwrap();
        cache.finish_download("0.4.2", "verified bytes").unwrap();
        let payload = cache.take_ready("0.4.2").unwrap();
        assert_eq!(payload, "verified bytes");

        cache.finish_install("0.4.2");

        assert_eq!(cache.status().state, "empty");
        assert!(matches!(
            cache.begin_download("0.4.3", |_| 0),
            Ok(BeginUpdateDownload::Start)
        ));
    }

    #[test]
    fn finish_install_only_releases_the_version_that_was_claimed() {
        let cache = DownloadedUpdateCache::default();
        cache.begin_download("0.4.2", |_| 0).unwrap();
        cache.finish_download("0.4.2", "verified bytes").unwrap();
        let _payload = cache.take_ready("0.4.2").unwrap();

        // A stale completion for another version must not release this claim.
        cache.finish_install("0.4.1");
        assert_eq!(cache.status().state, "installing");
        assert_eq!(cache.status().version.as_deref(), Some("0.4.2"));
    }

    #[test]
    fn status_reports_the_installable_version_only_while_ready() {
        let cache = DownloadedUpdateCache::<&str>::default();
        assert_eq!(cache.status().state, "empty");
        assert_eq!(cache.status().version, None);

        cache.begin_download("0.4.2", |_| 0).unwrap();
        let downloading = cache.status();
        assert_eq!(downloading.state, "downloading");
        // The version is reported for every active state so the renderer can pin
        // its metadata handle to the package being produced, but only `ready`
        // means "installable right now".
        assert_eq!(downloading.version.as_deref(), Some("0.4.2"));

        cache.finish_download("0.4.2", "bytes").unwrap();
        let ready = cache.status();
        assert_eq!(ready.state, "ready");
        assert_eq!(ready.version.as_deref(), Some("0.4.2"));
    }

    #[test]
    fn replacing_a_ready_package_reports_what_it_discarded() {
        // Starting a download for a different version silently threw away an
        // already-verified package while the renderer still believed it was
        // installable; the caller must at least be able to log it.
        let cache = DownloadedUpdateCache::default();
        cache.begin_download("0.4.2", |_| 0).unwrap();
        cache.finish_download("0.4.2", "verified bytes").unwrap();

        match cache.begin_download("0.4.3", |_| 0) {
            Ok(BeginUpdateDownload::StartReplacingReady { replaced }) => {
                assert_eq!(replaced, "0.4.2");
            }
            other => panic!("expected the discarded package to be reported, got {other:?}"),
        }
    }

    #[test]
    fn coalescing_progress_never_loses_or_duplicates_a_byte() {
        use std::sync::atomic::{AtomicU64, Ordering};

        // Synthetic clock so the test does not depend on wall time.
        let base = Instant::now();
        let min = Duration::from_millis(100);
        let buffered = AtomicU64::new(0);
        let mut last_sent = base;

        // (chunk bytes, milliseconds since `base`) — several chunks inside one
        // window, one that crosses it, then a sub-window tail that never crosses.
        let chunks: [(u64, u64); 7] = [
            (1_000, 10),
            (2_000, 40),
            (3_000, 90),
            (4_000, 150),
            (5_000, 160),
            (6_000, 260),
            (7_000, 300),
        ];

        let mut emitted = Vec::new();
        for (chunk, at_ms) in chunks {
            if let Some(bytes) = coalesce_progress_chunk(
                &buffered,
                &mut last_sent,
                chunk,
                min,
                base + Duration::from_millis(at_ms),
            ) {
                emitted.push(bytes);
            }
        }
        // The remainder the interval swallowed, flushed the way the download's
        // completion callback does.
        let tail = buffered.swap(0, Ordering::Relaxed);

        let total: u64 = chunks.iter().map(|(chunk, _)| chunk).sum();
        let reported: u64 = emitted.iter().sum::<u64>() + tail;
        assert_eq!(
            reported, total,
            "the renderer's running total must equal the real byte count"
        );
        // The point of coalescing: far fewer messages than chunks.
        assert!(
            emitted.len() < chunks.len(),
            "expected coalescing, got {} messages for {} chunks",
            emitted.len(),
            chunks.len()
        );
        // 150ms crosses the first window (1000+2000+3000+4000), 260ms the second
        // (5000+6000); 7000 arrives 40ms later and stays buffered as the tail.
        assert_eq!(emitted, vec![10_000, 11_000]);
        assert_eq!(tail, 7_000);
    }

    #[test]
    fn a_single_chunk_after_the_interval_publishes_immediately() {
        use std::sync::atomic::AtomicU64;

        let base = Instant::now();
        let buffered = AtomicU64::new(0);
        let mut last_sent = base;
        assert_eq!(
            coalesce_progress_chunk(
                &buffered,
                &mut last_sent,
                512,
                Duration::from_millis(100),
                base + Duration::from_millis(101),
            ),
            Some(512)
        );
    }

    #[test]
    fn github_download_detection_only_suppresses_redundant_fallbacks() {
        assert!(is_github_download_url(
            &url::Url::parse(
                "https://github.com/nomifun/nomifun-desktop/releases/download/v0.8.0/app.exe"
            )
            .unwrap()
        ));
        assert!(is_github_download_url(
            &url::Url::parse("https://release-assets.githubusercontent.com/asset").unwrap()
        ));
        assert!(!is_github_download_url(
            &url::Url::parse("https://cdn.crabnebula.app/asset/01ABC").unwrap()
        ));
    }

    #[test]
    fn already_ready_measures_the_retained_package_under_the_same_lock() {
        // The size travels with the verdict: measuring it in a second lock
        // acquisition let a concurrent take/replace turn it into 0, which is how
        // the UI ended up painting "100% of 0.0 KB".
        let cache = DownloadedUpdateCache::default();
        cache.begin_download("0.4.2", |_| 0).unwrap();
        cache.finish_download("0.4.2", vec![0u8; 1234]).unwrap();

        match cache.begin_download("0.4.2", |payload: &Vec<u8>| payload.len() as u64) {
            Ok(BeginUpdateDownload::AlreadyReady { retained_len }) => {
                assert_eq!(retained_len, 1234);
            }
            other => panic!("expected the retained size to be reported, got {other:?}"),
        }
    }

    #[test]
    fn an_install_already_in_flight_is_never_reported_as_recoverable() {
        // SAFETY BOUNDARY. `NotRetained`/`StillDownloading` prove the installer
        // was handed nothing. `AlreadyInstalling` does not: on macOS the running
        // .app has already been renamed aside by then, so telling the renderer
        // "just download again" hands the user a live Install button on top of a
        // half-replaced app instead of terminating.
        let cache = DownloadedUpdateCache::<&str>::default();
        assert_eq!(
            cache.take_ready("0.4.2"),
            Err(TakeReadyError::NotRetained {
                version: "0.4.2".to_owned()
            })
        );
        assert!(!TakeReadyError::NotRetained {
            version: "0.4.2".to_owned()
        }
        .handoff_may_have_started());
        assert!(!TakeReadyError::StillDownloading {
            version: "0.4.2".to_owned()
        }
        .handoff_may_have_started());
        assert!(TakeReadyError::AlreadyInstalling {
            version: "0.4.2".to_owned()
        }
        .handoff_may_have_started());

        cache.begin_download("0.4.2", |_| 0).unwrap();
        assert_eq!(
            cache.take_ready("0.4.2"),
            Err(TakeReadyError::StillDownloading {
                version: "0.4.2".to_owned()
            })
        );
        cache.finish_download("0.4.2", "bytes").unwrap();
        cache.take_ready("0.4.2").unwrap();
        // Slot is now Installing: a second claim must say so, not "not retained".
        assert_eq!(
            cache.take_ready("0.4.2"),
            Err(TakeReadyError::AlreadyInstalling {
                version: "0.4.2".to_owned()
            })
        );
    }

    #[test]
    fn downloaded_update_cache_rejects_wrong_version_without_losing_ready_package() {
        let cache = DownloadedUpdateCache::default();
        assert!(matches!(
            cache.begin_download("0.3.8", |_| 0),
            Ok(BeginUpdateDownload::Start)
        ));
        cache.finish_download("0.3.8", vec![1, 2, 3]).unwrap();

        assert!(cache.take_ready("0.3.9").is_err());
        let payload = cache.take_ready("0.3.8").unwrap();
        cache.restore_ready("0.3.8".to_owned(), payload);
        assert_eq!(cache.take_ready("0.3.8").unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn failed_update_download_can_be_retried() {
        let cache = DownloadedUpdateCache::<Vec<u8>>::default();
        assert!(matches!(
            cache.begin_download("0.3.8", |_| 0),
            Ok(BeginUpdateDownload::Start)
        ));
        cache.cancel_download("0.3.8");
        assert!(matches!(
            cache.begin_download("0.3.8", |_| 0),
            Ok(BeginUpdateDownload::Start)
        ));
    }

    #[test]
    fn updater_before_exit_retries_until_shutdown_is_verified_then_cleans_up_once() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let attempts = Arc::new(AtomicU8::new(0));

        let shutdown_events = events.clone();
        let shutdown_attempts = attempts.clone();
        let cleanup_events = events.clone();
        let error_events = events.clone();
        let wait_events = events.clone();

        let verified = updater_before_exit_until_verified(
            move || {
                let attempt = shutdown_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                shutdown_events
                    .lock()
                    .unwrap()
                    .push(format!("shutdown:{attempt}"));
                if attempt < 3 {
                    Err(anyhow::anyhow!("fixture shutdown failure {attempt}"))
                } else {
                    Ok(())
                }
            },
            move || cleanup_events.lock().unwrap().push("cleanup".to_owned()),
            move |attempt, _| {
                error_events
                    .lock()
                    .unwrap()
                    .push(format!("error:{attempt}"));
            },
            move || wait_events.lock().unwrap().push("wait".to_owned()),
            UPDATER_SHUTDOWN_MAX_ATTEMPTS,
        );

        assert!(verified);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "shutdown:1",
                "error:1",
                "wait",
                "shutdown:2",
                "error:2",
                "wait",
                "shutdown:3",
                "cleanup",
            ]
        );
    }

    #[test]
    fn updater_before_exit_is_bounded_and_still_cleans_up_when_shutdown_never_verifies() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let shutdown_events = events.clone();
        let cleanup_events = events.clone();
        let error_events = events.clone();
        let wait_events = events.clone();

        // F32: a persistently failing cleanup stage must not hang the update
        // install forever — the loop stops at the cap, preserves the plugin
        // cleanup exactly once, and reports the unverified shutdown.
        let verified = updater_before_exit_until_verified(
            move || {
                shutdown_events.lock().unwrap().push("shutdown".to_owned());
                Err(anyhow::anyhow!("fixture shutdown failure"))
            },
            move || cleanup_events.lock().unwrap().push("cleanup".to_owned()),
            move |attempt, _| {
                error_events
                    .lock()
                    .unwrap()
                    .push(format!("error:{attempt}"));
            },
            move || wait_events.lock().unwrap().push("wait".to_owned()),
            3,
        );

        assert!(!verified);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "shutdown", "error:1", "wait", "shutdown", "error:2", "wait", "shutdown",
                "error:3", "cleanup",
            ]
        );
    }

    #[test]
    fn deferred_exit_without_backend_cleanup_allows_the_exit_with_original_code() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.request_normal_exit(Some(5)));
        assert!(!coordinator.is_exit_allowed());

        // F42: with no backend ever becoming available, the stranded quit
        // gesture must still complete instead of being dropped forever.
        let code = allow_exit_without_backend_cleanup(&coordinator);

        assert_eq!(code, 5);
        assert!(coordinator.is_exit_allowed());
    }

    #[test]
    fn restart_cleanup_is_bounded_and_forces_the_handoff_when_never_verified() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.request_restart());
        let attempts = Arc::new(AtomicU8::new(0));
        let cleanup_attempts = attempts.clone();

        // A wedged Chromium on the restart path must not freeze the Tauri
        // main thread forever: the loop stops at the cap and the relaunch
        // handoff proceeds with unverified cleanup.
        let verified = restart_cleanup_bounded(&coordinator, "test", move || {
            cleanup_attempts.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!("fixture cleanup failure"))
        });

        assert!(!verified);
        assert_eq!(
            u64::from(attempts.load(Ordering::SeqCst)),
            RESTART_CLEANUP_MAX_ATTEMPTS,
            "cleanup must still be attempted for every bounded retry"
        );
        assert!(
            coordinator.is_exit_allowed(),
            "the restart handoff must proceed after exhausting the bounded attempts"
        );
    }

    #[test]
    fn restart_cleanup_verifies_within_the_bound_without_forcing_the_handoff() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.request_restart());
        let attempts = Arc::new(AtomicU8::new(0));
        let cleanup_attempts = attempts.clone();

        let verified = restart_cleanup_bounded(&coordinator, "test", move || {
            if cleanup_attempts.fetch_add(1, Ordering::SeqCst) + 1 < 3 {
                Err(anyhow::anyhow!("fixture transient cleanup failure"))
            } else {
                Ok(())
            }
        });

        assert!(verified);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert!(
            !coordinator.is_exit_allowed(),
            "success-path bookkeeping (mark_cleanup_verified) is owned by the caller"
        );
    }

    #[test]
    fn restart_hold_without_cleanup_authority_is_bounded_and_proceeds() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.request_restart());
        // Terminal backend registration: no cleanup authority can appear
        // anymore, so the bounded hold must resolve immediately instead of
        // parking the restart forever.
        coordinator.mark_backend_failed_unverified();

        let start = Instant::now();
        hold_restart_without_cleanup_authority(&coordinator);

        assert!(
            start.elapsed() < RESTART_HOLD_WITHOUT_AUTHORITY_WAIT,
            "a terminal registration must not wait out the full authority hold"
        );
        assert!(
            coordinator.is_exit_allowed(),
            "the restart handoff must proceed after the bounded no-authority hold"
        );
    }

    #[test]
    fn exhausted_cleanup_handoff_reports_the_original_exit_code_and_allows_exit() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.request_normal_exit(Some(9)));
        coordinator.mark_cleanup_failed();

        let code = allow_handoff_without_verified_cleanup(&coordinator);

        assert_eq!(code, 9);
        assert!(coordinator.is_exit_allowed());
        assert!(
            coordinator.backend_server().is_none(),
            "no observer may re-acquire the backend after the forced handoff"
        );
    }

    #[test]
    fn exit_coordinator_keeps_the_first_normal_exit_code_and_starts_once() {
        let coordinator = ExitCoordinator::default();

        assert!(coordinator.request_normal_exit(Some(23)));
        assert!(!coordinator.request_normal_exit(Some(99)));
        assert_eq!(coordinator.original_code(), 23);
        assert_eq!(coordinator.shutdown_mode(), Some(ShutdownMode::Normal));
        assert!(coordinator.claim_shutdown_start());
        assert!(!coordinator.claim_shutdown_start());

        coordinator.mark_cleanup_verified();
        assert!(coordinator.is_exit_allowed());
    }

    #[test]
    fn exit_coordinator_upgrades_an_inflight_normal_exit_to_restart() {
        let coordinator = ExitCoordinator::default();

        assert!(coordinator.request_normal_exit(Some(7)));
        assert!(
            !coordinator.request_restart(),
            "the normal shutdown already owns the phase transition"
        );
        assert!(coordinator.is_restart_requested());
        assert_eq!(coordinator.shutdown_mode(), Some(ShutdownMode::Restart));
        assert_eq!(
            coordinator.original_code(),
            7,
            "upgrading to restart must not overwrite the original normal exit code"
        );
    }

    #[test]
    fn restart_cleanup_failure_aborts_relaunch() {
        let failed: anyhow::Result<()> = Err(anyhow::anyhow!("fixture cleanup failure"));
        let succeeded: anyhow::Result<()> = Ok(());

        assert_eq!(
            restart_cleanup_outcome(&failed),
            RestartCleanupOutcome::AbortRestart
        );
        assert_eq!(
            restart_cleanup_outcome(&succeeded),
            RestartCleanupOutcome::ContinueRestart
        );
    }

    #[test]
    fn exit_coordinator_reports_a_fatal_exit_only_once() {
        let coordinator = ExitCoordinator::default();

        coordinator.mark_no_cleanup_needed();
        assert!(coordinator.claim_fatal_exit());
        assert!(!coordinator.claim_fatal_exit());
        coordinator.mark_cleanup_verified();
        assert!(
            coordinator.is_exit_allowed(),
            "normal completion must not disarm an already-fatal exit"
        );
        assert!(!coordinator.request_normal_exit(Some(0)));
    }

    #[test]
    fn exit_coordinator_unblocks_early_exit_when_backend_startup_fails() {
        let coordinator = ExitCoordinator::default();

        coordinator.mark_no_cleanup_needed();
        coordinator.mark_backend_failed_verified();
        assert!(coordinator.wait_for_backend(Duration::from_secs(1)).is_none());
        assert!(coordinator.backend_server().is_none());
    }

    #[test]
    fn startup_cleanup_not_started_is_verified_and_has_no_server() {
        let cleanup = StartupCleanup::NotStarted;
        let runtime = tokio::runtime::Runtime::new().expect("build test runtime");

        assert!(cleanup.cleanup(&runtime).is_ok());
        assert!(cleanup.is_verified());
        assert!(cleanup.server().is_none());
    }

    #[test]
    fn startup_cleanup_starting_is_fail_closed_and_unverified() {
        let cleanup = StartupCleanup::StartingUnverified;
        let runtime = tokio::runtime::Runtime::new().expect("build test runtime");

        let error = cleanup
            .cleanup(&runtime)
            .expect_err("startup without published cleanup authority must fail closed");
        assert!(format!("{error:#}").contains("cleanup authority"));
        assert!(!cleanup.is_verified());
        assert!(cleanup.server().is_none());
    }

    #[test]
    fn startup_cleanup_normal_start_failure_is_verified_and_has_no_server() {
        let cleanup = StartupCleanup::FailedVerified;
        let runtime = tokio::runtime::Runtime::new().expect("build test runtime");

        assert!(cleanup.cleanup(&runtime).is_ok());
        assert!(cleanup.is_verified());
        assert!(cleanup.server().is_none());
    }

    #[test]
    fn startup_cleanup_entered_helper_transitions_shared_state() {
        let cleanup = Mutex::new(StartupCleanup::NotStarted);

        mark_startup_cleanup_entered(&cleanup);
        assert!(matches!(
            &*cleanup.lock().unwrap(),
            StartupCleanup::StartingUnverified
        ));
    }

    #[test]
    fn startup_cleanup_failed_helper_only_verifies_a_returned_start_error() {
        let cleanup = Mutex::new(StartupCleanup::NotStarted);

        mark_startup_cleanup_entered(&cleanup);
        mark_startup_cleanup_failed_verified(&cleanup);
        assert!(matches!(
            &*cleanup.lock().unwrap(),
            StartupCleanup::FailedVerified
        ));
    }

    #[test]
    fn backend_run_error_without_a_started_server_is_preserved() {
        let error = backend_run_failure(Ok(Err(anyhow::anyhow!("fixture backend error"))))
            .expect("an error result must remain fatal");

        assert!(error.contains("fixture backend error"));
    }

    #[test]
    fn backend_run_panic_without_a_started_server_is_preserved() {
        let panic = std::panic::catch_unwind(|| panic!("fixture backend panic"));
        let error = backend_run_failure(panic.map(|_| Ok(())))
            .expect("a panic result must remain fatal");

        assert!(error.contains("fixture backend panic"));
    }

    #[test]
    fn backend_run_success_needs_no_fatal_cleanup() {
        assert!(backend_run_failure(Ok(Ok(()))).is_none());
    }

    #[test]
    fn backend_failure_cleanup_runs_before_runtime_is_dropped() {
        struct RuntimeDropProbe {
            runtime: tokio::runtime::Runtime,
            dropped: Arc<AtomicBool>,
        }

        impl Drop for RuntimeDropProbe {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::Release);
            }
        }

        let runtime_dropped = Arc::new(AtomicBool::new(false));
        let cleanup_called = Arc::new(AtomicBool::new(false));
        let cleanup_called_for_fn = Arc::clone(&cleanup_called);
        let runtime_dropped_for_fn = Arc::clone(&runtime_dropped);
        let runtime = RuntimeDropProbe {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("build probe Tokio runtime"),
            dropped: Arc::clone(&runtime_dropped),
        };

        let error = finish_backend_runtime(
            Ok(Err(anyhow::anyhow!("fixture backend failure"))),
            runtime,
            move |runtime| {
                assert!(
                    !runtime_dropped_for_fn.load(Ordering::Acquire),
                    "backend cleanup must run while its Tokio runtime is still alive"
                );
                let (tx, rx) = std::sync::mpsc::channel();
                runtime.runtime.spawn(async move {
                    tx.send(()).expect("send runtime probe completion");
                });
                rx.recv_timeout(Duration::from_secs(1))
                    .expect("cleanup must run while the Tokio runtime is usable");
                cleanup_called_for_fn.store(true, Ordering::Release);
                Ok(())
            },
        )
        .expect("backend failure must remain fatal");

        assert!(error.error.contains("fixture backend failure"));
        assert!(cleanup_called.load(Ordering::Acquire));
        assert!(runtime_dropped.load(Ordering::Acquire));
    }

    #[test]
    fn backend_panic_cleanup_runs_before_runtime_is_dropped() {
        struct RuntimeDropProbe {
            runtime: tokio::runtime::Runtime,
            dropped: Arc<AtomicBool>,
        }

        impl Drop for RuntimeDropProbe {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::Release);
            }
        }

        let panic = std::panic::catch_unwind(|| panic!("fixture backend panic"));
        let runtime_dropped = Arc::new(AtomicBool::new(false));
        let runtime_dropped_for_fn = Arc::clone(&runtime_dropped);
        let runtime = RuntimeDropProbe {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("build probe Tokio runtime"),
            dropped: Arc::clone(&runtime_dropped),
        };

        let error = finish_backend_runtime(panic.map(|_| Ok(())), runtime, move |runtime| {
            assert!(
                !runtime_dropped_for_fn.load(Ordering::Acquire),
                "panic cleanup must run while its Tokio runtime is still alive"
            );
            let (tx, rx) = std::sync::mpsc::channel();
            runtime.runtime.spawn(async move {
                tx.send(()).expect("send runtime probe completion");
            });
            rx.recv_timeout(Duration::from_secs(1))
                .expect("panic cleanup must run while the Tokio runtime is usable");
            Ok(())
        })
        .expect("backend panic must remain fatal");

        assert!(error.error.contains("fixture backend panic"));
        assert!(runtime_dropped.load(Ordering::Acquire));
    }

    #[test]
    fn backend_failure_preserves_shutdown_error_context() {
        let error = finish_backend_runtime(
            Ok(Err(anyhow::anyhow!("fixture backend failure"))),
            (),
            |_| Err(anyhow::anyhow!("fixture shutdown failure")),
        )
        .expect("backend failure must remain fatal");

        assert!(error.error.contains("fixture backend failure"));
        assert!(error.error.contains("fixture shutdown failure"));
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
            // The generated-context gate sets the requirement variable and runs:
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
