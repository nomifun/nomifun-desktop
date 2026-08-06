//! Chrome for Testing 版本解析 + 平台 id 映射 + 浏览器解析（打包优先 / 下载兜底 / mac 去 quarantine）。
//!
//! 「零联网安装、不依赖 PATH」的兑现点。旧 Playwright provision 正是在此失败
//! （ENOENT / npm 不走代理），故此处直接用 `nomifun_net::http_client`（代理感知）
//! 分块下载 + 唯一 staging + 有界 zip 解压 + completion sentinel + 原子目录发布，
//! 全部自包含、不依赖外部 node / npm / PATH。
//!
//! 注：下载 / 解压 / `no_window_command` / `strip_quarantine` 的写法是
//! `nomifun-app::provision::install` 同款的**复刻**而非引用——后者位于 backend
//! 二进制 crate，agent crate 反向依赖它会造成依赖倒置，故在此本地复刻并对齐版本
//! （zip = "2" / flate2 同 workspace；本模块只需 zip，CfT 三平台包都是 .zip）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::engine::BrowserError;
use fs2::FileExt;
use tokio::io::AsyncWriteExt;

/// chrome zip 单次下载超时。CfT chrome 包 ~150MB；裸 reqwest client 无默认超时，
/// 停滞连接会永久挂起，故显式封顶（对齐 provision::install 的 600s）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// known-good JSON 抓取超时（小文件，宽松即可）。
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);

/// A corrupt/misdirected response must not fill the disk forever. Current CfT
/// Chrome archives are far below this bound, while the extracted tree is
/// larger than the archive and therefore has its own bound below.
const MAX_ARCHIVE_BYTES: u64 = 1_024 * 1_024 * 1_024;
const MAX_EXTRACTED_BYTES: u64 = 3 * 1_024 * 1_024 * 1_024;
const MAX_ZIP_ENTRIES: usize = 20_000;
/// Validation counts every directory entry (files, directories, and symlinks),
/// not just regular files. This mirrors the archive entry ceiling while also
/// bounding directories created implicitly by nested zip paths.
const MAX_PAYLOAD_TREE_ENTRIES: usize = MAX_ZIP_ENTRIES;
/// A breadth-heavy tree must not turn the depth-first directory frontier into
/// an unbounded collection of retained paths. Boxed paths have no spare
/// capacity, so this limits their encoded backing storage directly; the
/// separate entry ceiling bounds the small Vec/Box metadata overhead.
const MAX_PAYLOAD_FRONTIER_PATH_BYTES: usize = 16 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 8 * 1_024 * 1_024;
const MIN_INSTALL_BYTES: u64 = 8 * 1_024 * 1_024;
const MIN_INSTALL_FILES: u64 = 8;
const INSTALL_SENTINEL: &str = ".nomifun-cft-complete.json";
const INSTALL_SENTINEL_SCHEMA: u32 = 1;
const STAGING_SUBDIR: &str = ".staging";
const ACQUIRE_LOCK_FILE: &str = ".acquire.lock";
const ACQUIRE_LOCK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const ACQUIRE_LOCK_POLL: Duration = Duration::from_millis(100);

fn process_acquire_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct InstallSentinel {
    schema: u32,
    version: String,
    platform: String,
    payload_file_count: u64,
    payload_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreeStats {
    files: u64,
    bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct PayloadTreeLimits {
    entries: usize,
    frontier_path_bytes: usize,
}

const PAYLOAD_TREE_LIMITS: PayloadTreeLimits = PayloadTreeLimits {
    entries: MAX_PAYLOAD_TREE_ENTRIES,
    frontier_path_bytes: MAX_PAYLOAD_FRONTIER_PATH_BYTES,
};

/// The lock file lives beside all managed versions, so separate application
/// processes cannot download/extract/publish the same installation at once.
struct CrossProcessAcquireLock {
    file: File,
}

impl Drop for CrossProcessAcquireLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Cancellation safety for the only large temporary state. Normal paths
/// explicitly remove it; if a future is aborted, Drop still removes it while
/// the cross-process lock is held. A later acquisition also performs a strict
/// stale sweep before allocating another staging tree.
struct StagingDirGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingDirGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        if self.armed && self.path.exists() {
            std::fs::remove_dir_all(&self.path)?;
        }
        self.armed = false;
        Ok(())
    }
}

impl Drop for StagingDirGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = std::fs::remove_dir_all(&self.path) {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %error,
                    "failed to clean cancelled CfT staging directory; next acquire will retry"
                );
            }
        }
    }
}

/// 把 (os, arch) 映射到 Chrome for Testing 的 platform id。
pub fn cft_platform_id(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("win64"),
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("linux", "x86_64") => Some("linux64"),
        _ => None,
    }
}

#[derive(serde::Deserialize)]
struct KnownGood {
    versions: Vec<VerEntry>,
}
#[derive(serde::Deserialize)]
struct VerEntry {
    version: String,
    downloads: Downloads,
}
#[derive(serde::Deserialize)]
struct Downloads {
    chrome: Vec<Dl>,
}
#[derive(serde::Deserialize)]
struct Dl {
    platform: String,
    url: String,
}

/// 从 known-good-versions-with-downloads JSON 里挑指定 version+platform 的 chrome 下载 url。
pub fn pick_chrome_url(json: &str, version: &str, platform: &str) -> Option<String> {
    let kg: KnownGood = serde_json::from_str(json).ok()?;
    kg.versions
        .into_iter()
        .find(|v| v.version == version)?
        .downloads
        .chrome
        .into_iter()
        .find(|d| d.platform == platform)
        .map(|d| d.url)
}

/// 钉死的 Chromium 版本（build 期固化用同一版本，运行时只校验存在）。
//
// 已对照 Chrome for Testing `last-known-good-versions.json` 的 channels.Stable.version
// 核对（截至 2026-06-17）；该版本号属真实存在的稳定 CfT 通道版本，非占位值。
pub const PINNED_CHROME_VERSION: &str = "149.0.7827.155";
pub const KNOWN_GOOD_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/known-good-versions-with-downloads.json";

/// 用户显式指定 Chrome 可执行绝对路径的环境变量（最高优先级）。
pub const CHROME_BINARY_ENV: &str = "NOMIFUN_CHROME_BINARY";

/// 数据目录下安放下载浏览器的子目录名。布局：
/// `<data_dir>/nomifun-browser/<version>/chrome-<platform>/...`。
const BROWSER_SUBDIR: &str = "nomifun-browser";

/// **浏览器来源**：用户在设置里选的「浏览器模式」之来源维度（与「静默/可见」正交）。
///
/// - [`ChromeSource::Managed`]（默认）：内置/下载的 Chrome for Testing 优先 —— 现行为，
///   零依赖、可离线、版本钉死可控。
/// - [`ChromeSource::System`]（「我的浏览器」）：用户系统里装的 Chrome/Edge **本体**优先
///   （版本/指纹与日常一致），未探到则**优雅回退** Managed 顺序。**仍配专属
///   `--user-data-dir` 起独立托管实例**——红线不变：绝不碰用户真实 profile（登录态由
///   持久登录保险库单独维护，见 lib.rs `storage_state`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChromeSource {
    /// 内置/下载的 Chrome for Testing 优先（现行默认）。
    #[default]
    Managed,
    /// 系统已装 Chrome/Edge 本体优先；未找到回退 Managed。
    System,
}

impl ChromeSource {
    /// 从 `client_preferences` / `[tools.browser] source` 的字符串解析。
    /// `"system"`（大小写/空白不敏感）→ [`ChromeSource::System`]；其余（含空串/未知/
    /// `"managed"`）→ [`ChromeSource::Managed`]（default-safe：坏值静默退回默认，不阻断启动）。
    pub fn from_source_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "system" => ChromeSource::System,
            _ => ChromeSource::Managed,
        }
    }
}

/// 解压目录内可执行文件相对该平台解压根（`chrome-<platform>/`）的子路径。
///
/// 注意 CfT 的 zip 顶层目录就是 `chrome-<platform>/`，故这里返回的是**含**该顶层
/// 目录的相对路径——与 [`extract_zip_into`] 保留顶层目录的行为一致。
fn chrome_exe_subpath(platform: &str) -> Option<&'static str> {
    match platform {
        "win64" => Some("chrome-win64/chrome.exe"),
        // CfT 的 mac 包内是一个 `.app` bundle；真正可执行在 Contents/MacOS 下。
        // TODO(verify-macos): mac .app 内可执行路径需实机核对，见
        // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md
        "mac-arm64" => {
            Some("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        }
        "mac-x64" => {
            Some("chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        }
        // TODO(verify-linux): linux64 可执行子路径需实机核对，见上同文件。
        "linux64" => Some("chrome-linux64/chrome"),
        _ => None,
    }
}

/// env 覆写：`NOMIFUN_CHROME_BINARY` 指向的绝对路径，存在即用。两种 [`ChromeSource`] 下
/// 都最高优先（用户显式指定的二进制永远赢）。`exists` 注入文件存在判定（测试可注入假值）。
fn env_chrome_path(
    env_get: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let p = PathBuf::from(env_get(CHROME_BINARY_ENV)?);
    exists(&p).then_some(p)
}

/// 托管 Chrome for Testing：打包目录优先、其次数据目录（运行时已下载）。**不含** env、
/// **不含**系统浏览器（那两者由 [`resolve_local_chrome`] 按 source 编排）。`exists` 注入式，可单测。
fn cft_chrome_path(
    platform: &str,
    bundled_dir: Option<&Path>,
    data_dir: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let sub = chrome_exe_subpath(platform)?;
    // 打包资源目录：<bundled>/chrome-<platform>/...
    if let Some(bundled) = bundled_dir {
        let cand = bundled.join(sub);
        if exists(&cand) {
            return Some(cand);
        }
    }
    // 数据目录（运行时已下载）：<data>/nomifun-browser/<version>/chrome-<platform>/...
    let version_dir = data_dir.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION);
    let cand = version_dir.join(sub);
    (exists(&cand) && managed_install_is_complete(platform, &version_dir)).then_some(cand)
}

/// 纯优先级查找：按 env > 打包目录 > 数据目录 顺序找**已存在**的 chrome 可执行（**不含**系统
/// 浏览器探测）。生产路径已改用按 source 编排的 [`resolve_local_chrome`]；此函数保留给
/// 「纯 CfT 解析」相关的单测（如下载兜底触发条件断言需**不**短路到系统浏览器），故仅测试编译。
#[cfg(test)]
fn resolve_chrome_path_in(
    platform: &str,
    env_get: impl Fn(&str) -> Option<String>,
    bundled_dir: Option<&Path>,
    data_dir: &Path,
) -> Option<PathBuf> {
    let exists = |p: &Path| p.is_file();
    env_chrome_path(&env_get, exists)
        .or_else(|| cft_chrome_path(platform, bundled_dir, data_dir, exists))
}

/// 按 [`ChromeSource`] 编排的**纯优先级解析**（不下载、不触网，`exists`/`env_get` 注入式可单测）。
///
/// - env 覆写在**两种 source** 下都最高优先。
/// - [`ChromeSource::System`]（「我的浏览器」）：系统 Chrome/Edge 优先，未找到回退托管 CfT。
/// - [`ChromeSource::Managed`]（默认）：托管 CfT 优先，未找到回退系统浏览器（保持现行为）。
///
/// 返回 `None` → 本地无任何可用 chrome，交 [`resolve_chrome_path_with_source`] 走下载兜底。
fn resolve_local_chrome(
    platform: &str,
    os: &str,
    source: ChromeSource,
    env_get: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
    bundled_dir: Option<&Path>,
    data_dir: &Path,
) -> Option<PathBuf> {
    if let Some(p) = env_chrome_path(&env_get, &exists) {
        return Some(p);
    }
    let cft = || cft_chrome_path(platform, bundled_dir, data_dir, &exists);
    let sys = || detect_system_browser_in(os, &env_get, &exists);
    match source {
        ChromeSource::System => sys().or_else(cft),
        ChromeSource::Managed => cft().or_else(sys),
    }
}

/// 当前平台上**系统已装** Chromium 系浏览器（Chrome 优先、Edge 兜底）的候选可执行
/// 绝对路径列表，按优先级排序。纯函数（注入式 `env_get`，便于单测）。
///
/// 设计意图（呼应 DESIGN v1「复用系统浏览器二进制 + 专属 user-data-dir」）：多数用户
/// 机器已装 Chrome、Win10/11 必装 Edge——直接复用其**二进制**即可零下载、离线、绕过
/// CfT 下载被墙/无网的失败。永远配专属 `--user-data-dir` 起独立托管实例（launch.rs
/// 红线：绝不碰用户 profile）。Edge 亦为 Chromium，CDP 与 [`crate::switches`] 硬化开关
/// 通用（switches 删的仅 Edge 自更新项，不影响 CDP 启动）。
fn system_browser_candidates(os: &str, env_get: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    match os {
        "windows" => {
            // 环境变量展开（注入式读取）；缺失则回退惯例绝对路径。
            let pf = env_get("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
            let pf86 =
                env_get("ProgramFiles(x86)").unwrap_or_else(|| r"C:\Program Files (x86)".into());
            let lad = env_get("LocalAppData");
            // Chrome 优先：全局（64/32 位安装位）+ 每用户安装（LocalAppData）。
            out.push(PathBuf::from(&pf).join(r"Google\Chrome\Application\chrome.exe"));
            out.push(PathBuf::from(&pf86).join(r"Google\Chrome\Application\chrome.exe"));
            if let Some(lad) = &lad {
                out.push(PathBuf::from(lad).join(r"Google\Chrome\Application\chrome.exe"));
            }
            // Edge 兜底（Win10/11 预装；通常装在 Program Files (x86)）。
            out.push(PathBuf::from(&pf86).join(r"Microsoft\Edge\Application\msedge.exe"));
            out.push(PathBuf::from(&pf).join(r"Microsoft\Edge\Application\msedge.exe"));
            if let Some(lad) = &lad {
                out.push(PathBuf::from(lad).join(r"Microsoft\Edge\Application\msedge.exe"));
            }
        }
        // TODO(verify-macos): mac 系统浏览器路径需实机核对，见 PLATFORM-VERIFICATION.md。
        "macos" => {
            out.push(PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ));
            if let Some(home) = env_get("HOME") {
                out.push(
                    PathBuf::from(home)
                        .join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
                );
            }
            out.push(PathBuf::from(
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ));
            out.push(PathBuf::from(
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            ));
        }
        // TODO(verify-linux): linux 系统浏览器路径需实机核对，见 PLATFORM-VERIFICATION.md。
        "linux" => {
            for p in [
                "/usr/bin/google-chrome",
                "/usr/bin/google-chrome-stable",
                "/opt/google/chrome/chrome",
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/snap/bin/chromium",
                "/usr/bin/microsoft-edge",
                "/usr/bin/microsoft-edge-stable",
            ] {
                out.push(PathBuf::from(p));
            }
        }
        _ => {}
    }
    out
}

/// 探测系统已装 Chromium 系浏览器，返回首个**存在**的可执行（Chrome 优先、Edge 兜底）。
/// `exists` 注入文件存在判定（测试可注入假值）；真实调用方传 `|p| p.is_file()`。
fn detect_system_browser_in(
    os: &str,
    env_get: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    system_browser_candidates(os, env_get)
        .into_iter()
        .find(|p| exists(p))
}

/// 解析当前平台的 Chrome 可执行绝对路径（**托管来源** [`ChromeSource::Managed`]）。
/// 保留此签名以免改动既有 ~10 处调用点；来源可选的新入口见
/// [`resolve_chrome_path_with_source`]。
///
/// 优先级（高→低）：env > 打包 CfT > 已下载 CfT > 系统 Chrome/Edge > 下载 CfT。
pub async fn resolve_chrome_path(
    data_dir: &Path,
    bundled_dir: Option<&Path>,
) -> Result<PathBuf, BrowserError> {
    resolve_chrome_path_with_source(data_dir, bundled_dir, ChromeSource::Managed).await
}

/// 解析当前平台的 Chrome 可执行绝对路径，按 [`ChromeSource`] 编排来源优先级。
///
/// - env 覆写（`NOMIFUN_CHROME_BINARY`）在两种 source 下都最高优先；
/// - [`ChromeSource::System`]：系统 Chrome/Edge 优先 → 回退托管 CfT（打包/已下载）→ 下载兜底；
/// - [`ChromeSource::Managed`]：打包 CfT → 已下载 CfT → 系统 Chrome/Edge → 下载兜底（现行为）。
///
/// 下载兜底只在**本地与系统都无任何 Chromium 系浏览器**时触发（钉死版本 + 代理感知 client）。
/// 调用方（Task 7）负责提供 `data_dir`（应用数据目录）与 `bundled_dir`（Tauri resource dir）。
pub async fn resolve_chrome_path_with_source(
    data_dir: &Path,
    bundled_dir: Option<&Path>,
    source: ChromeSource,
) -> Result<PathBuf, BrowserError> {
    let platform = cft_platform_id(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        BrowserError::Unsupported {
            capability: "chrome-for-testing".into(),
            hint: format!(
                "no Chrome for Testing build for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        }
    })?;

    // 1-4：env / 系统浏览器 / 打包 CfT / 已下载 CfT，顺序由 source 决定，存在即返回。
    // 永远配专属 user-data-dir 起独立托管实例（launch.rs 红线：绝不碰用户 profile）。
    if let Some(p) = resolve_local_chrome(
        platform,
        std::env::consts::OS,
        source,
        |k| std::env::var(k).ok(),
        |p| p.is_file(),
        bundled_dir,
        data_dir,
    ) {
        return Ok(p);
    }

    // 5：下载兜底（本地无 CfT 且系统无任何 Chromium 系浏览器时的最后手段）。下载的是 CfT。
    download_chrome(platform, data_dir).await?;

    // 下载+解压后确认可执行就位（下载落在数据目录的托管 CfT）。
    cft_chrome_path(platform, bundled_dir, data_dir, |p: &Path| p.is_file()).ok_or_else(|| {
        BrowserError::Other(format!(
            "chrome executable missing after download into {}",
            data_dir.display()
        ))
    })
}

/// 下载钉死版本 chrome 到 `<data_dir>/nomifun-browser/<version>/`。
///
/// 安装协议：进程内 single-flight → 跨进程文件锁 → 清理 stale staging → 流式下载到
/// 唯一 staging → 有界解压与整树校验 → 写 completion sentinel → 同文件系统 rename 原子发布。
/// 最终目录在 sentinel 和完整 payload 同时就绪前不会被解析器看见。
async fn download_chrome(platform: &str, data_dir: &Path) -> Result<(), BrowserError> {
    let other = |e: String| BrowserError::Other(e);
    let _process_gate = tokio::time::timeout(ACQUIRE_LOCK_TIMEOUT, process_acquire_gate().lock())
        .await
        .map_err(|_| other("timed out waiting for in-process CfT acquire single-flight".into()))?;
    let browser_root = data_dir.join(BROWSER_SUBDIR);
    let version_dir = data_dir.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION);
    std::fs::create_dir_all(&browser_root)
        .map_err(|e| other(format!("mkdir {}: {e}", browser_root.display())))?;
    let _cross_process_lock = acquire_cross_process_lock(&browser_root).await?;

    cleanup_stale_staging(&browser_root).map_err(|e| {
        other(format!(
            "clean stale Chrome for Testing staging under {}: {e}",
            browser_root.display()
        ))
    })?;
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| other(format!("mkdir {}: {e}", version_dir.display())))?;

    // Double-check only after both locks. Another process may have completed the
    // install while this caller waited. A complete legacy install is adopted
    // only after full-tree validation and only when the old .zip/.part is gone.
    if prepare_existing_install(platform, &version_dir)? {
        return Ok(());
    }

    // Metadata/download are deliberately inside the gates: at most one archive
    // body and one extracted staging tree exist per process, and the filesystem
    // lock extends that guarantee to processes sharing this data directory.
    let url = fetch_chrome_url(platform).await?;
    let staging_path = create_unique_staging_dir(&browser_root, platform)
        .map_err(|e| other(format!("create unique CfT staging directory: {e}")))?;
    let mut staging = StagingDirGuard::new(staging_path.clone());
    let zip_path = staging_path.join("chrome.zip");
    download_to(&url, &zip_path).await?;

    let payload_dir = staging_path.join("payload");
    // This deliberately stays in the owning future instead of a detached
    // spawn_blocking task. Acquisition is a one-time slow path; synchronous
    // extraction means task cancellation cannot release the filesystem lock
    // and delete staging while an orphan worker is still writing into it.
    extract_zip_into(&zip_path, &payload_dir)
        .map_err(|e| other(format!("extract Chrome for Testing zip: {e}")))?;
    std::fs::remove_file(&zip_path)
        .map_err(|e| other(format!("remove staged archive {}: {e}", zip_path.display())))?;

    let stats = validate_install_payload(platform, &payload_dir)
        .map_err(|e| other(format!("validate staged Chrome for Testing: {e}")))?;
    write_install_sentinel(platform, &payload_dir, stats)
        .map_err(|e| other(format!("write Chrome for Testing completion sentinel: {e}")))?;

    let staged_install = install_root(platform, &payload_dir);
    let final_install = install_root(platform, &version_dir);
    if final_install.exists() {
        if managed_install_is_complete(platform, &version_dir) {
            staging.cleanup().map_err(|e| {
                other(format!(
                    "remove redundant CfT staging {}: {e}",
                    staging_path.display()
                ))
            })?;
            return Ok(());
        }
        // This should be impossible for cooperating processes because the lock
        // is held. Never overwrite an unexpected incomplete tree: fail closed
        // so a foreign writer cannot be hidden by our publication.
        return Err(other(format!(
            "Chrome for Testing publish collision at {}",
            final_install.display()
        )));
    }
    if let Err(error) = std::fs::rename(&staged_install, &final_install) {
        // A lockless/weak-lock filesystem can still race between the existence
        // check and rename. A fully verified winner is success; anything else
        // remains a fail-closed collision.
        if managed_install_is_complete(platform, &version_dir) {
            staging.cleanup().map_err(|e| {
                other(format!(
                    "remove redundant CfT staging {}: {e}",
                    staging_path.display()
                ))
            })?;
            return Ok(());
        }
        return Err(other(format!(
            "atomically publish Chrome for Testing {} -> {}: {error}",
            staged_install.display(),
            final_install.display()
        )));
    }

    if !managed_install_is_complete(platform, &version_dir) {
        return Err(other(format!(
            "published Chrome for Testing failed completion verification at {}",
            final_install.display()
        )));
    }

    // mac：去 quarantine，免 Gatekeeper 首次执行拦截。仅 mac，cfg 隔离。
    #[cfg(target_os = "macos")]
    {
        // TODO(verify-macos): xattr 去 quarantine 路径需实机核对，见
        // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md
        if let Some(sub) = chrome_exe_subpath(platform) {
            // 对 .app bundle 根递归去属性即可。
            let app = version_dir.join(sub);
            // sub 形如 chrome-mac-arm64/...app/Contents/MacOS/exe；取到 .app 根。
            let app_root = app
                .ancestors()
                .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
                .map(Path::to_path_buf)
                .unwrap_or(app);
            strip_quarantine(&app_root);
        }
    }

    staging.cleanup().map_err(|e| {
        other(format!(
            "remove completed CfT staging {}: {e}",
            staging_path.display()
        ))
    })?;
    Ok(())
}

async fn acquire_cross_process_lock(
    browser_root: &Path,
) -> Result<CrossProcessAcquireLock, BrowserError> {
    let lock_path = browser_root.join(ACQUIRE_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| {
            BrowserError::Other(format!("open CfT acquire lock {}: {e}", lock_path.display()))
        })?;
    let deadline = tokio::time::Instant::now() + ACQUIRE_LOCK_TIMEOUT;
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(CrossProcessAcquireLock { file }),
            Err(error) if lock_is_contended(&error) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(BrowserError::Other(format!(
                        "timed out waiting for CfT acquire lock {}",
                        lock_path.display()
                    )));
                }
                // Polling a non-blocking OS lock keeps cancellation exact: no
                // detached spawn_blocking waiter can accumulate after callers
                // abandon their Browser task.
                tokio::time::sleep(ACQUIRE_LOCK_POLL).await;
            }
            Err(error) => {
                return Err(BrowserError::Other(format!(
                    "lock CfT acquire lock {}: {error}",
                    lock_path.display()
                )));
            }
        }
    }
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    match (error.raw_os_error(), expected.raw_os_error()) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => error.kind() == expected.kind(),
    }
}

fn install_root(platform: &str, version_like_dir: &Path) -> PathBuf {
    version_like_dir.join(format!("chrome-{platform}"))
}

fn create_unique_staging_dir(browser_root: &Path, platform: &str) -> std::io::Result<PathBuf> {
    let root = browser_root.join(STAGING_SUBDIR);
    std::fs::create_dir_all(&root)?;
    for _ in 0..16 {
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|e| std::io::Error::other(format!("secure random staging id: {e}")))?;
        let path = root.join(format!(
            "{}-{platform}-{}-{}",
            PINNED_CHROME_VERSION,
            std::process::id(),
            hex::encode(random)
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique CfT staging directory",
    ))
}

fn cleanup_stale_staging(browser_root: &Path) -> std::io::Result<()> {
    let root = browser_root.join(STAGING_SUBDIR);
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        remove_any_path(&path)?;
    }
    Ok(())
}

fn remove_any_path(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Returns true when a complete installation already exists. Old releases
/// wrote directly into the final tree without a sentinel; those trees are
/// adopted only after the archive sidecars are absent and the entire payload
/// passes the same bounded validation as a fresh extraction.
fn prepare_existing_install(
    platform: &str,
    version_dir: &Path,
) -> Result<bool, BrowserError> {
    if managed_install_is_complete(platform, version_dir) {
        cleanup_legacy_archive_sidecars(platform, version_dir)?;
        return Ok(true);
    }

    let final_install = install_root(platform, version_dir);
    let legacy_zip = version_dir.join(format!("chrome-{platform}.zip"));
    let legacy_part = version_dir.join(format!("chrome-{platform}.part"));
    let sentinel = final_install.join(INSTALL_SENTINEL);
    if final_install.is_dir()
        && !sentinel.exists()
        && !legacy_zip.exists()
        && !legacy_part.exists()
    {
        if let Ok(stats) = validate_install_payload(platform, version_dir) {
            write_install_sentinel(platform, version_dir, stats).map_err(|e| {
                BrowserError::Other(format!(
                    "adopt validated legacy CfT install at {}: {e}",
                    final_install.display()
                ))
            })?;
            if managed_install_is_complete(platform, version_dir) {
                return Ok(true);
            }
        }
    }

    // A marker with mismatching stats, an old partial archive, or an invalid
    // tree is never considered usable merely because chrome.exe exists.
    remove_any_path(&final_install).map_err(|e| {
        BrowserError::Other(format!(
            "remove incomplete CfT install {}: {e}",
            final_install.display()
        ))
    })?;
    cleanup_legacy_archive_sidecars(platform, version_dir)?;
    Ok(false)
}

fn cleanup_legacy_archive_sidecars(
    platform: &str,
    version_dir: &Path,
) -> Result<(), BrowserError> {
    for path in [
        version_dir.join(format!("chrome-{platform}.zip")),
        version_dir.join(format!("chrome-{platform}.part")),
    ] {
        remove_any_path(&path).map_err(|e| {
            BrowserError::Other(format!("remove stale CfT sidecar {}: {e}", path.display()))
        })?;
    }
    Ok(())
}

fn managed_install_is_complete(platform: &str, version_dir: &Path) -> bool {
    let install = install_root(platform, version_dir);
    let sentinel_path = install.join(INSTALL_SENTINEL);
    let metadata = match std::fs::symlink_metadata(&sentinel_path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= 16 * 1024 => metadata,
        _ => return false,
    };
    if metadata.len() == 0 {
        return false;
    }
    let raw = match std::fs::read(&sentinel_path) {
        Ok(raw) => raw,
        Err(_) => return false,
    };
    let sentinel: InstallSentinel = match serde_json::from_slice(&raw) {
        Ok(sentinel) => sentinel,
        Err(_) => return false,
    };
    if sentinel.schema != INSTALL_SENTINEL_SCHEMA
        || sentinel.version != PINNED_CHROME_VERSION
        || sentinel.platform != platform
    {
        return false;
    }
    match validate_install_payload(platform, version_dir) {
        Ok(stats) => {
            stats.files == sentinel.payload_file_count && stats.bytes == sentinel.payload_bytes
        }
        Err(_) => false,
    }
}

fn validate_install_payload(platform: &str, version_like_dir: &Path) -> std::io::Result<TreeStats> {
    let install = install_root(platform, version_like_dir);
    if !install.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("install root missing: {}", install.display()),
        ));
    }

    let required_files: &[&str] = match platform {
        "win64" => &[
            "chrome.exe",
            "chrome.dll",
            "chrome_elf.dll",
            "icudtl.dat",
            "resources.pak",
        ],
        "linux64" => &["chrome", "chrome_sandbox", "icudtl.dat", "resources.pak"],
        "mac-arm64" | "mac-x64" => &[
            "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "Google Chrome for Testing.app/Contents/Info.plist",
        ],
        _ => {
            return Err(std::io::Error::other(format!(
                "unsupported CfT platform {platform}"
            )))
        }
    };
    for relative in required_files {
        let path = install.join(relative);
        let metadata = std::fs::metadata(&path).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("required CfT file missing {}: {e}", path.display()),
            )
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(std::io::Error::other(format!(
                "required CfT file is empty or not regular: {}",
                path.display()
            )));
        }
    }
    if matches!(platform, "mac-arm64" | "mac-x64") {
        for relative in [
            "Google Chrome for Testing.app/Contents/Frameworks",
            "Google Chrome for Testing.app/Contents/Resources",
        ] {
            let path = install.join(relative);
            if !path.is_dir() {
                return Err(std::io::Error::other(format!(
                    "required CfT directory missing: {}",
                    path.display()
                )));
            }
        }
    }

    let stats = payload_tree_stats(&install)?;
    if stats.files < MIN_INSTALL_FILES || stats.bytes < MIN_INSTALL_BYTES {
        return Err(std::io::Error::other(format!(
            "CfT payload is implausibly small: {} files / {} bytes",
            stats.files, stats.bytes
        )));
    }
    Ok(stats)
}

/// Walk a CfT payload without collecting a directory snapshot. Callers receive
/// only scalar statistics, so a later verification cannot retain or multiply a
/// previous walk's frontier. Symlinks remain leaf entries and are never
/// followed.
fn payload_tree_stats(root: &Path) -> std::io::Result<TreeStats> {
    payload_tree_stats_with_limits(root, PAYLOAD_TREE_LIMITS)
}

fn payload_tree_stats_with_limits(
    root: &Path,
    limits: PayloadTreeLimits,
) -> std::io::Result<TreeStats> {
    let root_path_bytes = root.as_os_str().as_encoded_bytes().len();
    if root_path_bytes > limits.frontier_path_bytes {
        return Err(std::io::Error::other(format!(
            "CfT payload traversal frontier exceeds {} path bytes",
            limits.frontier_path_bytes
        )));
    }

    // `into_boxed_path` removes PathBuf spare capacity. Therefore the tracked
    // encoded lengths bound the actual path-content backing allocations held
    // by this frontier; `limits.entries` independently bounds pointer metadata.
    let mut pending: Vec<(Box<Path>, usize)> = vec![(
        root.to_path_buf().into_boxed_path(),
        root_path_bytes,
    )];
    let mut frontier_path_bytes = root_path_bytes;
    let mut entries = 0_usize;
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    while let Some((dir, dir_path_bytes)) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            // Check a conservative joined-path upper bound before allocating a
            // PathBuf. A directory entry name cannot contain a separator, so
            // `dir + separator + name` is exact or one byte too large when
            // `dir` already ends in a separator.
            let entry_path_upper_bound = dir_path_bytes
                .checked_add(1)
                .and_then(|bytes| {
                    bytes.checked_add(file_name.as_os_str().as_encoded_bytes().len())
                })
                .ok_or_else(|| std::io::Error::other("CfT payload entry path-byte overflow"))?;
            if entry_path_upper_bound > limits.frontier_path_bytes {
                return Err(std::io::Error::other(format!(
                    "CfT payload entry path exceeds {} bytes",
                    limits.frontier_path_bytes
                )));
            }
            entries = entries
                .checked_add(1)
                .ok_or_else(|| std::io::Error::other("CfT payload entry-count overflow"))?;
            if entries > limits.entries {
                return Err(std::io::Error::other(format!(
                    "CfT payload exceeds validation entry limit {}",
                    limits.entries
                )));
            }
            // DirEntry::metadata is explicitly non-following for symlinks and
            // avoids constructing a full path for ordinary file leaves.
            let metadata = entry.metadata()?;
            let is_root_sentinel = dir.as_ref() == root
                && file_name.as_os_str() == INSTALL_SENTINEL;
            if is_root_sentinel {
                if !metadata.is_file() {
                    let path = dir.join(&file_name);
                    return Err(std::io::Error::other(format!(
                        "CfT completion sentinel is not a regular file: {}",
                        path.display()
                    )));
                }
                continue;
            }
            if metadata.is_dir() {
                let path = dir.join(&file_name);
                let entry_path_bytes = path.as_os_str().as_encoded_bytes().len();
                debug_assert!(entry_path_bytes <= entry_path_upper_bound);
                let next_frontier_path_bytes = frontier_path_bytes
                    .checked_add(entry_path_bytes)
                    .ok_or_else(|| {
                        std::io::Error::other("CfT payload frontier path-byte overflow")
                    })?;
                if next_frontier_path_bytes > limits.frontier_path_bytes {
                    return Err(std::io::Error::other(format!(
                        "CfT payload traversal frontier exceeds {} path bytes",
                        limits.frontier_path_bytes
                    )));
                }
                pending.push((path.into_boxed_path(), entry_path_bytes));
                frontier_path_bytes = next_frontier_path_bytes;
                continue;
            }
            if !metadata.is_file() && !metadata.file_type().is_symlink() {
                let path = dir.join(&file_name);
                return Err(std::io::Error::other(format!(
                    "unsupported entry type in CfT payload: {}",
                    path.display()
                )));
            }
            files = files
                .checked_add(1)
                .ok_or_else(|| std::io::Error::other("CfT payload file-count overflow"))?;
            bytes = bytes
                .checked_add(metadata.len())
                .ok_or_else(|| std::io::Error::other("CfT payload size overflow"))?;
            if bytes > MAX_EXTRACTED_BYTES {
                return Err(std::io::Error::other("CfT payload exceeds validation bounds"));
            }
        }
        drop(dir);
        frontier_path_bytes = frontier_path_bytes
            .checked_sub(dir_path_bytes)
            .ok_or_else(|| std::io::Error::other("CfT payload frontier accounting underflow"))?;
    }
    Ok(TreeStats { files, bytes })
}

fn write_install_sentinel(
    platform: &str,
    version_like_dir: &Path,
    stats: TreeStats,
) -> std::io::Result<()> {
    let install = install_root(platform, version_like_dir);
    let path = install.join(INSTALL_SENTINEL);
    let temp = install.join(format!("{INSTALL_SENTINEL}.tmp"));
    let sentinel = InstallSentinel {
        schema: INSTALL_SENTINEL_SCHEMA,
        version: PINNED_CHROME_VERSION.to_string(),
        platform: platform.to_string(),
        payload_file_count: stats.files,
        payload_bytes: stats.bytes,
    };
    let encoded = serde_json::to_vec(&sentinel)
        .map_err(|e| std::io::Error::other(format!("serialize sentinel: {e}")))?;
    remove_any_path(&temp)?;
    let mut file = OpenOptions::new().create_new(true).write(true).open(&temp)?;
    file.write_all(&encoded)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, &path)?;
    Ok(())
}

/// 取 known-good JSON 并挑出钉死版本+平台的 chrome 下载 url。
async fn fetch_chrome_url(platform: &str) -> Result<String, BrowserError> {
    let client = nomifun_net::http_client();
    let mut response = client
        .get(KNOWN_GOOD_URL)
        .timeout(METADATA_TIMEOUT)
        .send()
        .await
        .map_err(|e| BrowserError::Other(format!("GET {KNOWN_GOOD_URL}: {e}")))?
        .error_for_status()
        .map_err(|e| BrowserError::Other(format!("non-2xx from {KNOWN_GOOD_URL}: {e}")))?;
    if response.content_length().is_some_and(|len| len > MAX_METADATA_BYTES) {
        return Err(BrowserError::Other(format!(
            "Chrome for Testing metadata exceeds {MAX_METADATA_BYTES} bytes"
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| BrowserError::Other(format!("read body {KNOWN_GOOD_URL}: {e}")))?
    {
        let next_len = (body.len() as u64)
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| BrowserError::Other("CfT metadata length overflow".into()))?;
        if next_len > MAX_METADATA_BYTES {
            return Err(BrowserError::Other(format!(
                "Chrome for Testing metadata exceeds {MAX_METADATA_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    let json = std::str::from_utf8(&body)
        .map_err(|e| BrowserError::Other(format!("CfT metadata is not UTF-8: {e}")))?;
    pick_chrome_url(json, PINNED_CHROME_VERSION, platform).ok_or_else(|| {
        BrowserError::Other(format!(
            "no chrome download for version {PINNED_CHROME_VERSION} platform {platform} in known-good list"
        ))
    })
}

/// 代理感知下载 `url` 到唯一 staging 内的 `dest`。响应分块直接写文件，不把整个
/// ~150MB archive 聚合进堆；`.part` 仅在 fsync 后 rename。
async fn download_to(url: &str, dest: &Path) -> Result<(), BrowserError> {
    let other = |e: String| BrowserError::Other(e);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| other(format!("mkdir {}: {e}", parent.display())))?;
    }
    let client = nomifun_net::http_client();
    let mut response = client
        .get(url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| other(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| other(format!("non-2xx from {url}: {e}")))?;
    if response.content_length().is_some_and(|len| len > MAX_ARCHIVE_BYTES) {
        return Err(other(format!(
            "Chrome for Testing archive exceeds {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    let part = dest.with_extension("part");
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&part)
            .await
            .map_err(|e| other(format!("create staged archive {}: {e}", part.display())))?;
        let mut written = 0_u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| other(format!("stream body {url}: {e}")))?
        {
            written = written
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| other("CfT archive length overflow".into()))?;
            if written > MAX_ARCHIVE_BYTES {
                return Err(other(format!(
                    "Chrome for Testing archive exceeds {MAX_ARCHIVE_BYTES} bytes"
                )));
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| other(format!("write staged archive {}: {e}", part.display())))?;
        }
        if written == 0 {
            return Err(other("Chrome for Testing archive response was empty".into()));
        }
        file.flush()
            .await
            .map_err(|e| other(format!("flush staged archive {}: {e}", part.display())))?;
        file.sync_all()
            .await
            .map_err(|e| other(format!("sync staged archive {}: {e}", part.display())))?;
        drop(file);
        tokio::fs::rename(&part, dest)
            .await
            .map_err(|e| other(format!("rename archive into {}: {e}", dest.display())))?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result
}

/// 解压 zip 到 `dest_dir`，保留 zip 内顶层目录（CfT 包顶层即 `chrome-<platform>/`）。
/// 拒绝 traversal-unsafe 名、重复输出和超过上限的 archive；unix 保留权限位。
fn extract_zip_into(archive: &Path, dest_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest_dir)?;
    let f = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| std::io::Error::other(format!("read zip: {e}")))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(std::io::Error::other(format!(
            "zip has {} entries; maximum is {MAX_ZIP_ENTRIES}",
            zip.len()
        )));
    }
    let mut extracted = 0_u64;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| std::io::Error::other(format!("zip entry {i}: {e}")))?;
        let rel = entry.enclosed_name().ok_or_else(|| {
            std::io::Error::other(format!("zip entry {i} has traversal-unsafe path"))
        })?;
        extracted = extracted
            .checked_add(entry.size())
            .ok_or_else(|| std::io::Error::other("zip extracted-size overflow"))?;
        if extracted > MAX_EXTRACTED_BYTES {
            return Err(std::io::Error::other(format!(
                "zip expands beyond {MAX_EXTRACTED_BYTES} bytes"
            )));
        }
        let out = dest_dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let entry_is_symlink = entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000);
        #[cfg(unix)]
        let unix_mode = entry.unix_mode();
        let expected = entry.size();
        if entry_is_symlink {
            #[cfg(unix)]
            {
                use std::io::Read as _;
                use std::os::unix::ffi::OsStringExt as _;

                if expected > 4 * 1024 {
                    return Err(std::io::Error::other(format!(
                        "zip symlink entry {i} target is implausibly long"
                    )));
                }
                let mut target = Vec::with_capacity(expected as usize);
                entry.read_to_end(&mut target)?;
                if target.len() as u64 != expected || target.contains(&0) {
                    return Err(std::io::Error::other(format!(
                        "zip symlink entry {i} has invalid target"
                    )));
                }
                let target = PathBuf::from(std::ffi::OsString::from_vec(target));
                if target.as_os_str().is_empty()
                    || target.is_absolute()
                    || target.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    })
                {
                    return Err(std::io::Error::other(format!(
                        "zip symlink entry {i} escapes the staging tree"
                    )));
                }
                std::os::unix::fs::symlink(target, &out)?;
                continue;
            }
            #[cfg(not(unix))]
            {
                return Err(std::io::Error::other(format!(
                    "zip entry {i} is an unexpected symlink on this platform"
                )));
            }
        }
        let mut w = OpenOptions::new().create_new(true).write(true).open(&out)?;
        let copied = std::io::copy(&mut entry, &mut w)?;
        if copied != expected {
            return Err(std::io::Error::other(format!(
                "zip entry {i} size mismatch: expected {expected}, copied {copied}"
            )));
        }
        w.flush()?;
        #[cfg(unix)]
        {
            // TODO(verify-linux): chrome 可执行位需实机核对（保留 zip 权限位），见
            // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = unix_mode {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

/// 去 `com.apple.quarantine`，免 Gatekeeper 拦截。复刻 `provision::install::strip_quarantine`：
/// 仅 mac 实做（cfg 隔离），其它平台与缺 `xattr` 时为安全 no-op。无需管理员权限。
#[cfg(target_os = "macos")]
fn strip_quarantine(path: &Path) {
    // -r 递归整 .app 树；-d 删单属性；缺属性返非零，按 benign 处理。
    let status = no_window_command("/usr/bin/xattr")
        .args(["-r", "-d", "com.apple.quarantine"])
        .arg(path)
        .status();
    match status {
        Ok(s) if s.success() => {
            tracing::debug!(path = %path.display(), "stripped com.apple.quarantine");
        }
        Ok(_) => {
            tracing::debug!(path = %path.display(), "xattr non-zero (likely no quarantine attr)");
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "xattr failed; Gatekeeper may prompt");
        }
    }
}

/// 构造永不闪控制台窗的 [`std::process::Command`]（同 provision::install::no_window_command）。
/// 仅 mac quarantine 路径用到，故 cfg 到 macos 避免别处 dead_code。off-Windows no-op。
#[cfg(target_os = "macos")]
fn no_window_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    std::process::Command::new(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_id_maps() {
        assert_eq!(cft_platform_id("windows", "x86_64"), Some("win64"));
        assert_eq!(cft_platform_id("macos", "aarch64"), Some("mac-arm64"));
        assert_eq!(cft_platform_id("macos", "x86_64"), Some("mac-x64"));
        assert_eq!(cft_platform_id("linux", "x86_64"), Some("linux64"));
        assert_eq!(cft_platform_id("freebsd", "x86_64"), None);
    }

    #[test]
    fn parse_download_url_from_known_good_json() {
        let json = r#"{"versions":[{"version":"151.0.7895.0","downloads":{"chrome":[{"platform":"linux64","url":"https://x/linux64/chrome-linux64.zip"}]}}]}"#;
        let url = pick_chrome_url(json, "151.0.7895.0", "linux64").unwrap();
        assert!(url.ends_with("chrome-linux64.zip"));
    }

    #[test]
    fn missing_version_or_platform_returns_none() {
        let json = r#"{"versions":[{"version":"1.0","downloads":{"chrome":[{"platform":"linux64","url":"u"}]}}]}"#;
        assert!(pick_chrome_url(json, "9.9", "linux64").is_none());
        assert!(pick_chrome_url(json, "1.0", "win64").is_none());
        assert!(pick_chrome_url("not json", "1.0", "linux64").is_none());
    }

    // --- Task 6: 优先级解析（纯逻辑，Windows 可跑）-----------------------------

    /// 在解压目录布局里造一个假 exe 文件（含中间目录），用 win64 子路径。
    fn touch(root: &Path, sub: &str) -> PathBuf {
        let p = root.join(sub);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"fake-chrome").unwrap();
        p
    }

    fn populate_complete_win64_version(version_dir: &Path) -> PathBuf {
        let install = install_root("win64", &version_dir);
        for name in [
            "chrome.exe",
            "chrome.dll",
            "chrome_elf.dll",
            "icudtl.dat",
            "resources.pak",
            "locales/en-US.pak",
            "v8_context_snapshot.bin",
        ] {
            touch(&install, name);
        }
        let filler = touch(&install, "test-payload.bin");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&filler)
            .unwrap()
            .set_len(MIN_INSTALL_BYTES)
            .unwrap();
        let stats = validate_install_payload("win64", &version_dir).unwrap();
        write_install_sentinel("win64", &version_dir, stats).unwrap();
        version_dir.join("chrome-win64/chrome.exe")
    }

    fn make_complete_win64_install(data_dir: &Path) -> PathBuf {
        populate_complete_win64_version(
            &data_dir.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION),
        )
    }

    #[test]
    fn env_path_wins_when_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env_exe = tmp.path().join("custom").join("chrome.exe");
        std::fs::create_dir_all(env_exe.parent().unwrap()).unwrap();
        std::fs::write(&env_exe, b"x").unwrap();

        // 即使打包目录与数据目录都有 exe，env 仍应最高优先。
        let bundled = tmp.path().join("bundled");
        touch(&bundled, "chrome-win64/chrome.exe");
        let data = tmp.path().join("data");
        touch(&data.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION), "chrome-win64/chrome.exe");

        let env_str = env_exe.to_string_lossy().to_string();
        let got = resolve_chrome_path_in(
            "win64",
            |k| (k == CHROME_BINARY_ENV).then(|| env_str.clone()),
            Some(&bundled),
            &data,
        );
        assert_eq!(got, Some(env_exe));
    }

    #[test]
    fn env_path_ignored_when_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundled = tmp.path().join("bundled");
        let bundled_exe = touch(&bundled, "chrome-win64/chrome.exe");
        let data = tmp.path().join("data");

        // env 指向不存在的文件 → 跳过，落到打包目录。
        let got = resolve_chrome_path_in(
            "win64",
            |_| Some("Z:/nope/chrome.exe".to_string()),
            Some(&bundled),
            &data,
        );
        assert_eq!(got, Some(bundled_exe));
    }

    #[test]
    fn bundled_dir_used_when_no_env() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundled = tmp.path().join("bundled");
        let bundled_exe = touch(&bundled, "chrome-win64/chrome.exe");
        // 数据目录也有，但打包目录优先。
        let data = tmp.path().join("data");
        touch(&data.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION), "chrome-win64/chrome.exe");

        let got = resolve_chrome_path_in("win64", |_| None, Some(&bundled), &data);
        assert_eq!(got, Some(bundled_exe));
    }

    #[test]
    fn data_dir_used_when_no_env_no_bundled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let data_exe = make_complete_win64_install(&data);

        // 无 env、无打包目录 → 数据目录命中。
        let got = resolve_chrome_path_in("win64", |_| None, None, &data);
        assert_eq!(got, Some(data_exe.clone()));

        // 打包目录传了但里面没有 → 仍落到数据目录。
        let empty_bundled = tmp.path().join("empty");
        std::fs::create_dir_all(&empty_bundled).unwrap();
        let got2 = resolve_chrome_path_in("win64", |_| None, Some(&empty_bundled), &data);
        assert_eq!(got2, Some(data_exe));
    }

    #[test]
    fn none_when_nothing_present_triggers_download() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        // 全空：env 无、打包目录无、数据目录无 → None（交给下载兜底）。
        assert!(resolve_chrome_path_in("win64", |_| None, None, &data).is_none());
    }

    #[test]
    fn managed_install_never_accepts_only_chrome_exe() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let version = data.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION);
        touch(&version, "chrome-win64/chrome.exe");
        assert!(!managed_install_is_complete("win64", &version));
        assert!(resolve_chrome_path_in("win64", |_| None, None, &data).is_none());
    }

    #[test]
    fn completion_sentinel_binds_full_tree_stats() {
        let tmp = tempfile::TempDir::new().unwrap();
        let version = tmp.path().join("version");
        populate_complete_win64_version(&version);
        assert!(managed_install_is_complete("win64", &version));

        std::fs::write(install_root("win64", &version).join("chrome.dll"), b"tampered")
            .unwrap();
        assert!(
            !managed_install_is_complete("win64", &version),
            "payload changes must invalidate the completion sentinel"
        );
    }

    #[test]
    fn payload_tree_counts_wide_empty_directories_against_one_entry_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("wide-empty-tree");
        std::fs::create_dir_all(&root).unwrap();
        const ALLOWED_ENTRIES: usize = 32;
        for index in 0..=ALLOWED_ENTRIES {
            std::fs::create_dir(root.join(format!("empty-{index:03}"))).unwrap();
        }

        let error = payload_tree_stats_with_limits(
            &root,
            PayloadTreeLimits {
                entries: ALLOWED_ENTRIES,
                frontier_path_bytes: MAX_PAYLOAD_FRONTIER_PATH_BYTES,
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("entry limit 32"),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn payload_tree_counts_deep_empty_directories_against_one_entry_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("deep-empty-tree");
        std::fs::create_dir_all(&root).unwrap();
        const ALLOWED_ENTRIES: usize = 16;
        let mut cursor = root.clone();
        for index in 0..=ALLOWED_ENTRIES {
            cursor = cursor.join(format!("d{index:02}"));
            std::fs::create_dir(&cursor).unwrap();
        }

        let error = payload_tree_stats_with_limits(
            &root,
            PayloadTreeLimits {
                entries: ALLOWED_ENTRIES,
                frontier_path_bytes: MAX_PAYLOAD_FRONTIER_PATH_BYTES,
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("entry limit 16"),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn payload_tree_does_not_hide_nested_sentinel_named_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("sentinel-name-tree");
        std::fs::create_dir_all(
            root.join("parent")
                .join(INSTALL_SENTINEL)
                .join("nested-empty"),
        )
        .unwrap();

        let error = payload_tree_stats_with_limits(
            &root,
            PayloadTreeLimits {
                entries: 2,
                frontier_path_bytes: MAX_PAYLOAD_FRONTIER_PATH_BYTES,
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("entry limit 2"),
            "nested sentinel-named directories must be traversed: {error}"
        );
    }

    #[test]
    fn payload_tree_bounds_long_paths_retained_in_the_frontier() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("long-frontier");
        std::fs::create_dir_all(&root).unwrap();
        let children = [
            root.join(format!("a{}", "a".repeat(120))),
            root.join(format!("b{}", "b".repeat(120))),
        ];
        for child in &children {
            std::fs::create_dir(child).unwrap();
        }
        let root_bytes = root.as_os_str().as_encoded_bytes().len();
        let largest_child_bytes = children
            .iter()
            .map(|path| path.as_os_str().as_encoded_bytes().len())
            .max()
            .unwrap();

        // The active root and either child fit, but retaining both children in
        // the breadth frontier does not. The result is independent of the
        // filesystem's directory enumeration order.
        let frontier_path_bytes = root_bytes + largest_child_bytes;
        let error = payload_tree_stats_with_limits(
            &root,
            PayloadTreeLimits {
                entries: MAX_PAYLOAD_TREE_ENTRIES,
                frontier_path_bytes,
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("payload traversal frontier exceeds"),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn representative_cft_install_remains_within_production_tree_budgets() {
        let tmp = tempfile::TempDir::new().unwrap();
        let version = tmp.path().join("representative-version");
        populate_complete_win64_version(&version);

        let stats = payload_tree_stats(&install_root("win64", &version)).unwrap();
        assert!(stats.files >= MIN_INSTALL_FILES);
        assert!(stats.bytes >= MIN_INSTALL_BYTES);
        assert!(managed_install_is_complete("win64", &version));
    }

    #[cfg(unix)]
    #[test]
    fn payload_tree_treats_directory_symlinks_as_leaf_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("payload");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("must-not-be-walked"), vec![0_u8; 4096]).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("directory-link")).unwrap();

        let stats = payload_tree_stats(&root).unwrap();
        assert_eq!(stats.files, 1, "the symlink itself is the only payload leaf");
        assert!(
            stats.bytes < 4096,
            "the traversal must not follow a directory symlink"
        );
    }

    #[test]
    fn validated_legacy_tree_is_adopted_but_active_partial_is_replaced() {
        let tmp = tempfile::TempDir::new().unwrap();
        let version = tmp.path().join("version");
        populate_complete_win64_version(&version);
        std::fs::remove_file(install_root("win64", &version).join(INSTALL_SENTINEL)).unwrap();
        assert!(prepare_existing_install("win64", &version).unwrap());
        assert!(managed_install_is_complete("win64", &version));

        std::fs::remove_file(install_root("win64", &version).join(INSTALL_SENTINEL)).unwrap();
        std::fs::write(version.join("chrome-win64.part"), b"still downloading").unwrap();
        assert!(!prepare_existing_install("win64", &version).unwrap());
        assert!(!install_root("win64", &version).exists());
        assert!(!version.join("chrome-win64.part").exists());
    }

    #[test]
    fn stale_staging_is_swept_before_new_unique_staging() {
        let tmp = tempfile::TempDir::new().unwrap();
        let browser_root = tmp.path().join(BROWSER_SUBDIR);
        let stale = browser_root.join(STAGING_SUBDIR).join("old-partial");
        touch(&stale, "payload/chrome-win64/chrome.exe");
        cleanup_stale_staging(&browser_root).unwrap();
        assert!(!stale.exists());

        let one = create_unique_staging_dir(&browser_root, "win64").unwrap();
        let two = create_unique_staging_dir(&browser_root, "win64").unwrap();
        assert_ne!(one, two);
        assert!(one.is_dir() && two.is_dir());
    }

    #[test]
    fn staged_install_is_complete_before_atomic_publication() {
        let tmp = tempfile::TempDir::new().unwrap();
        let staging_payload = tmp.path().join("staging/payload");
        populate_complete_win64_version(&staging_payload);
        let final_version = tmp.path().join("final-version");
        std::fs::create_dir_all(&final_version).unwrap();
        let staged = install_root("win64", &staging_payload);
        let final_install = install_root("win64", &final_version);
        assert!(!final_install.exists());
        std::fs::rename(&staged, &final_install).unwrap();
        assert!(managed_install_is_complete("win64", &final_version));
    }

    #[tokio::test]
    async fn cross_process_lock_serializes_acquirers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let browser_root = tmp.path().join(BROWSER_SUBDIR);
        std::fs::create_dir_all(&browser_root).unwrap();
        let first = acquire_cross_process_lock(&browser_root).await.unwrap();
        let ready = tmp.path().join("lock-child-ready");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "acquire::tests::cross_process_lock_child",
                "--ignored",
            ])
            .env("NOMIFUN_TEST_CFT_LOCK_ROOT", &browser_root)
            .env("NOMIFUN_TEST_CFT_LOCK_READY", &ready)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "lock child exited before attempting the filesystem lock"
            );
            assert!(
                tokio::time::Instant::now() < ready_deadline,
                "lock child did not reach the filesystem lock"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            child.try_wait().unwrap().is_none(),
            "child process must wait while the filesystem lock is held"
        );
        drop(first);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "lock child failed with {status}");
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill();
                panic!("child process did not acquire lock after release");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[ignore = "helper subprocess for cross_process_lock_serializes_acquirers"]
    #[tokio::test]
    async fn cross_process_lock_child() {
        let browser_root = PathBuf::from(
            std::env::var_os("NOMIFUN_TEST_CFT_LOCK_ROOT").expect("lock root env"),
        );
        let ready = PathBuf::from(
            std::env::var_os("NOMIFUN_TEST_CFT_LOCK_READY").expect("lock ready env"),
        );
        std::fs::write(ready, b"ready").unwrap();
        let lock = acquire_cross_process_lock(&browser_root).await.unwrap();
        drop(lock);
    }

    #[tokio::test]
    async fn archive_download_streams_into_staging_file() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nfirst-second",
                )
                .await
                .unwrap();
        });
        let tmp = tempfile::TempDir::new().unwrap();
        let destination = tmp.path().join("unique-staging/chrome.zip");
        download_to(&format!("http://{address}/chrome.zip"), &destination)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"first-second");
        assert!(!destination.with_extension("part").exists());
    }

    #[test]
    fn staging_guard_cleans_cancelled_partial_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let staging = tmp.path().join("cancelled-staging");
        touch(&staging, "payload/chrome-win64/chrome.exe");
        {
            let _guard = StagingDirGuard::new(staging.clone());
        }
        assert!(!staging.exists());
    }

    #[test]
    fn exe_subpath_per_platform_correct() {
        assert_eq!(chrome_exe_subpath("win64"), Some("chrome-win64/chrome.exe"));
        assert_eq!(
            chrome_exe_subpath("mac-arm64"),
            Some("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        );
        assert_eq!(
            chrome_exe_subpath("mac-x64"),
            Some("chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        );
        assert_eq!(chrome_exe_subpath("linux64"), Some("chrome-linux64/chrome"));
        assert_eq!(chrome_exe_subpath("freebsd"), None);
    }

    #[test]
    fn unknown_platform_resolves_to_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        // 平台未知 → chrome_exe_subpath 返 None → 整体 None（除非 env 命中，这里无 env）。
        assert!(resolve_chrome_path_in("nope", |_| None, None, tmp.path()).is_none());
    }

    // --- 系统浏览器探测（纯逻辑，注入 env + exists，Windows 可跑）------------------

    #[test]
    fn windows_candidates_chrome_before_edge_and_expand_env() {
        let env = |k: &str| match k {
            "ProgramFiles" => Some(r"C:\PF".to_string()),
            "ProgramFiles(x86)" => Some(r"C:\PF86".to_string()),
            "LocalAppData" => Some(r"C:\Users\me\AppData\Local".to_string()),
            _ => None,
        };
        // 归一化分隔符：`PathBuf::join` 在非 Windows 宿主用 '/' 拼接，与全反斜杠字面量不一致，
        // 故比较前统一把 '\\' 换成 '/'（纯逻辑判定在任意宿主可跑，对齐 display 同款跨平台单测设计）。
        let strs: Vec<String> = system_browser_candidates("windows", env)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        // 所有 Chrome 候选必须排在第一个 Edge 候选之前（Chrome 优先于 Edge）。
        let first_edge = strs.iter().position(|s| s.contains("msedge.exe")).unwrap();
        let last_chrome = strs.iter().rposition(|s| s.ends_with("chrome.exe")).unwrap();
        assert!(last_chrome < first_edge, "Chrome must precede Edge: {strs:?}");
        // env 展开生效（全局 Chrome / x86 Edge / 每用户 Chrome）。
        assert!(strs.iter().any(|s| s == "C:/PF/Google/Chrome/Application/chrome.exe"));
        assert!(strs.iter().any(|s| s == "C:/PF86/Microsoft/Edge/Application/msedge.exe"));
        assert!(strs
            .iter()
            .any(|s| s == "C:/Users/me/AppData/Local/Google/Chrome/Application/chrome.exe"));
    }

    #[test]
    fn windows_candidates_fall_back_to_conventional_paths_without_env() {
        // 归一化分隔符（见上一测试注释）：非 Windows 宿主 join 用 '/'。
        let strs: Vec<String> = system_browser_candidates("windows", |_| None)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(strs
            .iter()
            .any(|s| s == "C:/Program Files/Google/Chrome/Application/chrome.exe"));
        assert!(strs
            .iter()
            .any(|s| s == "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"));
        // 无 LocalAppData → 不产每用户候选（None 分支不 push）。
        assert!(!strs.iter().any(|s| s.contains("AppData/Local")));
    }

    #[test]
    fn detect_picks_first_existing_chrome_over_edge() {
        // 期望值从候选构造里取，而非硬编码字面量——非 Windows 宿主 `PathBuf::join` 用 '/'
        // 拼接，与全反斜杠字面量按 PathBuf 比较不等。同构造取值则在任意宿主可跑。
        let cands = system_browser_candidates("windows", |_| None);
        let conv_chrome = cands
            .iter()
            .find(|p| p.to_string_lossy().ends_with("chrome.exe"))
            .cloned()
            .expect("a chrome candidate");
        let conv_edge = cands
            .iter()
            .find(|p| p.to_string_lossy().contains("msedge.exe"))
            .cloned()
            .expect("an edge candidate");
        // 仅 Edge 存在 → 选 Edge。
        let edge_for_closure = conv_edge.clone();
        let got = detect_system_browser_in("windows", |_| None, move |p| *p == edge_for_closure);
        assert_eq!(got, Some(conv_edge));
        // Chrome + Edge 都存在 → 选 Chrome（优先级：首个候选即首个 chrome）。
        let got2 = detect_system_browser_in("windows", |_| None, |_| true);
        assert_eq!(got2, Some(conv_chrome));
        // 都不存在 → None（交给下载兜底）。
        let got3 = detect_system_browser_in("windows", |_| None, |_| false);
        assert!(got3.is_none());
    }

    #[test]
    fn detect_unknown_os_yields_none() {
        assert!(detect_system_browser_in("freebsd", |_| None, |_| true).is_none());
    }

    // --- ChromeSource 解析 + source 编排优先级（纯逻辑，注入 env+exists，任意宿主可跑）------

    #[test]
    fn chrome_source_from_str_and_default() {
        assert_eq!(ChromeSource::from_source_str("system"), ChromeSource::System);
        assert_eq!(ChromeSource::from_source_str("System"), ChromeSource::System);
        assert_eq!(ChromeSource::from_source_str("  SYSTEM  "), ChromeSource::System);
        assert_eq!(ChromeSource::from_source_str("managed"), ChromeSource::Managed);
        assert_eq!(ChromeSource::from_source_str(""), ChromeSource::Managed);
        assert_eq!(ChromeSource::from_source_str("garbage"), ChromeSource::Managed);
        assert_eq!(ChromeSource::default(), ChromeSource::Managed);
    }

    /// 取一个真实的系统 Chrome 候选路径（首个 chrome.exe 候选），用于注入 exists。
    fn a_system_chrome() -> PathBuf {
        system_browser_candidates("windows", |_| None)
            .into_iter()
            .find(|p| p.to_string_lossy().ends_with("chrome.exe"))
            .expect("a windows chrome candidate")
    }

    #[test]
    fn source_system_prefers_system_over_cft_managed_prefers_cft() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundled = tmp.path().join("bundled");
        let bundled_exe = bundled.join("chrome-win64/chrome.exe");
        let data = tmp.path().join("data");
        let sys = a_system_chrome();
        // 同时「存在」打包 CfT 与系统 chrome。
        let (be, se) = (bundled_exe.clone(), sys.clone());
        let exists = move |p: &Path| *p == be || *p == se;

        // System：选系统浏览器（优先于 CfT）。
        let got = resolve_local_chrome(
            "win64", "windows", ChromeSource::System, |_| None, &exists, Some(&bundled), &data,
        );
        assert_eq!(got, Some(sys));
        // Managed：选 CfT（打包）——现行为。
        let got2 = resolve_local_chrome(
            "win64", "windows", ChromeSource::Managed, |_| None, &exists, Some(&bundled), &data,
        );
        assert_eq!(got2, Some(bundled_exe));
    }

    #[test]
    fn source_system_falls_back_to_cft_when_no_system_browser() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundled = tmp.path().join("bundled");
        let bundled_exe = bundled.join("chrome-win64/chrome.exe");
        let data = tmp.path().join("data");
        let be = bundled_exe.clone();
        let exists = move |p: &Path| *p == be; // 仅 CfT 存在，无系统浏览器
        let got = resolve_local_chrome(
            "win64", "windows", ChromeSource::System, |_| None, &exists, Some(&bundled), &data,
        );
        assert_eq!(got, Some(bundled_exe));
    }

    #[test]
    fn source_managed_falls_back_to_system_when_no_cft() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let sys = a_system_chrome();
        let se = sys.clone();
        let exists = move |p: &Path| *p == se; // 仅系统浏览器存在，无 CfT
        let got = resolve_local_chrome(
            "win64", "windows", ChromeSource::Managed, |_| None, &exists, None, &data,
        );
        assert_eq!(got, Some(sys));
    }

    #[test]
    fn env_override_wins_in_both_sources() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env_exe = tmp.path().join("custom/chrome.exe");
        let bundled = tmp.path().join("bundled");
        let bundled_exe = bundled.join("chrome-win64/chrome.exe");
        let data = tmp.path().join("data");
        let sys = a_system_chrome();
        // env + CfT + 系统浏览器都「存在」。
        let (ee, be, se) = (env_exe.clone(), bundled_exe.clone(), sys.clone());
        let exists = move |p: &Path| *p == ee || *p == be || *p == se;
        let env_str = env_exe.to_string_lossy().to_string();
        let env_get = |k: &str| (k == CHROME_BINARY_ENV).then(|| env_str.clone());
        for source in [ChromeSource::System, ChromeSource::Managed] {
            let got = resolve_local_chrome(
                "win64", "windows", source, &env_get, &exists, Some(&bundled), &data,
            );
            assert_eq!(got, Some(env_exe.clone()), "env must win for {source:?}");
        }
    }

    #[test]
    fn resolve_local_none_when_nothing_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data = tmp.path().join("data");
        // 全空（无 env、无 CfT、无系统浏览器）→ None（交下载兜底）。
        for source in [ChromeSource::System, ChromeSource::Managed] {
            let got = resolve_local_chrome(
                "win64", "windows", source, |_| None, |_| false, None, &data,
            );
            assert!(got.is_none(), "expected None for {source:?}");
        }
    }

    /// 本机集成（需已装 Chrome/Edge）：验证**无 env 时**真实文件系统能探到系统浏览器。
    /// 手动跑 `cargo nextest run -p nomi-browser-engine acquire::detects_real -- --ignored`。
    #[ignore = "需本机已装 Chrome/Edge"]
    #[test]
    fn detects_real_system_browser_on_this_machine() {
        let got = detect_system_browser_in(
            std::env::consts::OS,
            |k| std::env::var(k).ok(),
            |p| p.is_file(),
        );
        assert!(got.is_some(), "no system Chrome/Edge found on this machine");
        assert!(got.unwrap().is_file());
    }

    /// 联网集成冒烟（~150MB 下载）：手动跑
    /// `cargo nextest run -p nomi-browser-engine acquire:: -- --ignored`。
    /// 直接验证**下载兜底**（绕过系统探测，否则装了 Chrome 的机器会短路到系统浏览器，
    /// 测不到下载路径）能下到并解压出可执行 chrome。
    #[ignore = "联网，下 ~150MB；手动跑"]
    #[tokio::test]
    async fn download_smoke() {
        let tmp = tempfile::TempDir::new().unwrap();
        let platform =
            cft_platform_id(std::env::consts::OS, std::env::consts::ARCH).expect("supported platform");
        download_chrome(platform, tmp.path()).await.expect("download+extract chrome");
        let path = resolve_chrome_path_in(platform, |_| None, None, tmp.path())
            .expect("resolved chrome after download");
        assert!(path.is_file(), "resolved chrome must exist at {}", path.display());
    }
}
