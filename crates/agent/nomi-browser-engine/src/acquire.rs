//! Chrome for Testing 版本解析 + 平台 id 映射 + 浏览器解析（打包优先 / 下载兜底 / mac 去 quarantine）。
//!
//! 「零联网安装、不依赖 PATH」的兑现点。旧 Playwright provision 正是在此失败
//! （ENOENT / npm 不走代理），故此处直接用 `nomifun_net::http_client`（代理感知）
//! 下载 + `.part`→rename + zip 解压，全部自包含、不依赖外部 node / npm / PATH。
//!
//! 注：下载 / 解压 / `no_window_command` / `strip_quarantine` 的写法是
//! `nomifun-app::provision::install` 同款的**复刻**而非引用——后者位于 backend
//! 二进制 crate，agent crate 反向依赖它会造成依赖倒置，故在此本地复刻并对齐版本
//! （zip = "2" / flate2 同 workspace；本模块只需 zip，CfT 三平台包都是 .zip）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::engine::BrowserError;

/// chrome zip 单次下载超时。CfT chrome 包 ~150MB；裸 reqwest client 无默认超时，
/// 停滞连接会永久挂起，故显式封顶（对齐 provision::install 的 600s）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// known-good JSON 抓取超时（小文件，宽松即可）。
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);

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

/// 纯优先级查找：按 env > 打包目录 > 数据目录 顺序找**已存在**的 chrome 可执行。
/// 不下载、不触网，故 Windows 可单测。`None` 时交给 [`resolve_chrome_path`] 走下载兜底。
///
/// - `env_get`：注入的环境变量读取器（测试可注入假值）。
/// - `bundled_dir`：Tauri resource dir（build 期固化），其下找 `chrome-<platform>/...`。
/// - `data_dir`：应用数据目录，其下找 `nomifun-browser/<version>/chrome-<platform>/...`。
fn resolve_chrome_path_in(
    platform: &str,
    env_get: impl Fn(&str) -> Option<String>,
    bundled_dir: Option<&Path>,
    data_dir: &Path,
) -> Option<PathBuf> {
    // 1. env 显式指定的绝对路径，存在即用（最高优先）。
    if let Some(p) = env_get(CHROME_BINARY_ENV) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }

    let sub = chrome_exe_subpath(platform)?;

    // 2. 打包资源目录：<bundled>/chrome-<platform>/...
    if let Some(bundled) = bundled_dir {
        let cand = bundled.join(sub);
        if cand.is_file() {
            return Some(cand);
        }
    }

    // 3. 数据目录（运行时已下载）：<data>/nomifun-browser/<version>/chrome-<platform>/...
    let cand = data_dir
        .join(BROWSER_SUBDIR)
        .join(PINNED_CHROME_VERSION)
        .join(sub);
    if cand.is_file() {
        return Some(cand);
    }

    None
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

/// 解析当前平台的 Chrome 可执行绝对路径。
///
/// 优先级（高→低）：
/// 1. `NOMIFUN_CHROME_BINARY` env（绝对路径，存在即用）；
/// 2. 打包资源目录 `bundled_dir/chrome-<platform>/...`（build 期固化）；
/// 3. 数据目录 `<data_dir>/nomifun-browser/<version>/chrome-<platform>/...`（运行时已下载）；
/// 4. **系统已装 Chrome/Edge**（[`detect_system_browser_in`]，Chromium 系，零下载/离线）；
/// 5. 都没有 → 用钉死版本 + 代理感知 client 下载 CfT 到数据目录并解压，再返回。
///
/// 调用方（Task 7）负责提供 `data_dir`（应用数据目录）与 `bundled_dir`（Tauri
/// resource dir），本 crate 不直接依赖任何应用数据目录来源，保持解耦。
pub async fn resolve_chrome_path(
    data_dir: &Path,
    bundled_dir: Option<&Path>,
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

    // 1-3：env / 打包 CfT / 已下载 CfT，存在即直接返回。
    if let Some(p) = resolve_chrome_path_in(platform, |k| std::env::var(k).ok(), bundled_dir, data_dir) {
        return Ok(p);
    }

    // 4：系统已装 Chrome/Edge（Chromium 系）。多数机器已装 Chrome、Win10/11 必装 Edge——
    // 复用其二进制即可零下载、离线、绕过 CfT 下载被墙/无网失败。永远配专属 user-data-dir
    // 起独立托管实例（launch.rs 红线：绝不碰用户 profile）。
    if let Some(p) = detect_system_browser_in(
        std::env::consts::OS,
        |k| std::env::var(k).ok(),
        |p| p.is_file(),
    ) {
        return Ok(p);
    }

    // 5：下载兜底（系统无任何 Chromium 系浏览器、且未打包/未下载 CfT 时的最后手段）。
    download_chrome(platform, data_dir).await?;

    // 下载+解压后再走一次纯解析，确认可执行就位。
    resolve_chrome_path_in(platform, |k| std::env::var(k).ok(), bundled_dir, data_dir).ok_or_else(|| {
        BrowserError::Other(format!(
            "chrome executable missing after download into {}",
            data_dir.display()
        ))
    })
}

/// 下载钉死版本 chrome 到 `<data_dir>/nomifun-browser/<version>/`，解压，
/// mac 上去 quarantine。代理感知（`nomifun_net::http_client`）、`.part`→rename。
async fn download_chrome(platform: &str, data_dir: &Path) -> Result<(), BrowserError> {
    let other = |e: String| BrowserError::Other(e);

    // 先取 known-good JSON，挑出钉死版本的下载 url。
    let url = fetch_chrome_url(platform).await?;

    let version_dir = data_dir.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION);
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| other(format!("mkdir {}: {e}", version_dir.display())))?;

    // 下载 zip → .part → rename（部分文件绝不冒充完成）。
    let zip_path = version_dir.join(format!("chrome-{platform}.zip"));
    download_to(&url, &zip_path).await?;

    // 解压到版本目录（CfT zip 顶层即 chrome-<platform>/）。
    extract_zip_into(&zip_path, &version_dir).map_err(|e| other(format!("extract chrome zip: {e}")))?;
    // 解压成功后删掉 zip，省空间；失败不致命。
    let _ = std::fs::remove_file(&zip_path);

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

    Ok(())
}

/// 取 known-good JSON 并挑出钉死版本+平台的 chrome 下载 url。
async fn fetch_chrome_url(platform: &str) -> Result<String, BrowserError> {
    let client = nomifun_net::http_client();
    let json = client
        .get(KNOWN_GOOD_URL)
        .timeout(METADATA_TIMEOUT)
        .send()
        .await
        .map_err(|e| BrowserError::Other(format!("GET {KNOWN_GOOD_URL}: {e}")))?
        .error_for_status()
        .map_err(|e| BrowserError::Other(format!("non-2xx from {KNOWN_GOOD_URL}: {e}")))?
        .text()
        .await
        .map_err(|e| BrowserError::Other(format!("read body {KNOWN_GOOD_URL}: {e}")))?;
    pick_chrome_url(&json, PINNED_CHROME_VERSION, platform).ok_or_else(|| {
        BrowserError::Other(format!(
            "no chrome download for version {PINNED_CHROME_VERSION} platform {platform} in known-good list"
        ))
    })
}

/// 代理感知下载 `url` 到 `dest`，经 `.part` 旁车再 rename（同 provision::install::download）。
async fn download_to(url: &str, dest: &Path) -> Result<(), BrowserError> {
    let other = |e: String| BrowserError::Other(e);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| other(format!("mkdir {}: {e}", parent.display())))?;
    }
    let client = nomifun_net::http_client();
    let resp = client
        .get(url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| other(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| other(format!("non-2xx from {url}: {e}")))?;
    let bytes = resp.bytes().await.map_err(|e| other(format!("read body {url}: {e}")))?;
    let part = dest.with_extension("part");
    std::fs::write(&part, &bytes).map_err(|e| other(format!("write {}: {e}", part.display())))?;
    std::fs::rename(&part, dest).map_err(|e| other(format!("rename into {}: {e}", dest.display())))?;
    Ok(())
}

/// 解压 zip 到 `dest_dir`，保留 zip 内顶层目录（CfT 包顶层即 `chrome-<platform>/`）。
/// 复刻 `provision::install::extract_zip`：跳过 traversal-unsafe 名、unix 保留权限位。
fn extract_zip_into(archive: &Path, dest_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest_dir)?;
    let f = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| std::io::Error::other(format!("read zip: {e}")))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| std::io::Error::other(format!("zip entry {i}: {e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            // traversal-unsafe（含 .. / 绝对路径）→ 跳过。
            continue;
        };
        let out = dest_dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut w)?;
        #[cfg(unix)]
        {
            // TODO(verify-linux): chrome 可执行位需实机核对（保留 zip 权限位），见
            // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
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
        let data_exe = touch(
            &data.join(BROWSER_SUBDIR).join(PINNED_CHROME_VERSION),
            "chrome-win64/chrome.exe",
        );

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
