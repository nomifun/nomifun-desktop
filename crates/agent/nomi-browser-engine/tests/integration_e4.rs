//! **P2 E4：下载沙箱 + MOTW 端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证：
//! 1. **下载落隔离 workspace**：navigate fixture（含 `download` 属性链接）→ observe → 取链接 ref
//!    → `act(Click)` → 文件落进 per-pet workspace 的 `downloads/` 子目录（**非用户 Downloads**）+
//!    size>0。`allowAndName` 下文件名是下载 GUID。
//! 2. **Win MOTW**：下载完成后该文件有 `Zone.Identifier` ADS 且含 `ZoneId=3`（`std::fs::read` 那个
//!    ADS 路径校验）。mac/linux 跳过 MOTW 断言（write_motw 空实现，登记在 PLATFORM-VERIFICATION.md）。
//!
//! 可执行 denylist 红线**拒绝判定**走纯逻辑单测（`download::tests`，不需真浏览器）——这里只验「良性
//! 文件真落盘 + MOTW 标记」的端到端链路。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=C:\Program Files\Google\Chrome\Application\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(download_) | test(motw)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

use std::time::Duration;

use nomi_browser_engine::progress::Progress;
use nomi_browser_engine::{ActSpec, BrowserEngine, ObserveOpts};

mod common;

/// 端到端：触发一次真实下载 → 文件落隔离 workspace/downloads + size>0；Windows 上额外验
/// `Zone.Identifier` ADS 含 ZoneId=3。一个测试覆盖「沙箱落点 + MOTW」全链（建一次 chrome）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn download_lands_in_sandbox_and_gets_motw() {
    let (backend, download_dir) =
        common::build_backend_for_fixture_with_downloads("e4").await;
    eprintln!("download_dir = {}", download_dir.display());

    backend
        .navigate(&common::fixture_url("download.html"), false)
        .await
        .expect("navigate download.html");

    // observe 填 ref 表（act Click 反查的前置）。
    let obs = backend
        .observe(&ObserveOpts::default())
        .await
        .expect("observe");
    eprintln!("=== download fixture entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }

    // 取下载链接的 ref（fixture 固定有一个 link "Download report.txt"）。
    let link = obs
        .entries
        .iter()
        .find(|e| e.role == "link" && e.name.contains("report.txt"))
        .expect("fixture should expose a download link");
    eprintln!("download link ref = {}", link.r#ref);

    // act(Click)：点击 download 链接触发下载（data: URL + download 属性 → chrome 落盘到沙箱目录）。
    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(&ActSpec::Click { r#ref: link.r#ref.clone() }, &p)
        .await;
    eprintln!("click result = {res:?}");
    // 点击本身可能因「下载导致导航被打断」返回各种良性态——不强断言 success；下载是否落盘才是验收点。

    // 轮询下载目录直到出现一个非空文件（下载异步；最长等 ~10s）。allowAndName → 文件名是 GUID。
    let mut found: Option<std::path::PathBuf> = None;
    for _ in 0..100 {
        if let Ok(rd) = std::fs::read_dir(&download_dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                // 跳过 chrome 下载中途的 .crdownload 临时文件，只认最终落盘文件。
                if path.extension().and_then(|e| e.to_str()) == Some("crdownload") {
                    continue;
                }
                if let Ok(meta) = std::fs::metadata(&path)
                    && meta.is_file()
                    && meta.len() > 0
                {
                    found = Some(path);
                    break;
                }
            }
        }
        if found.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let file = found.expect("a non-empty downloaded file should land in the sandbox download dir");
    let size = std::fs::metadata(&file).unwrap().len();
    eprintln!("downloaded file = {} ({} bytes)", file.display(), size);
    assert!(size > 0, "downloaded file must be non-empty");

    // ── 红线：落点必须在隔离 download_dir 下（绝不在用户真实 Downloads）──
    assert!(
        file.starts_with(&download_dir),
        "downloaded file must be inside the sandbox dir {}, got {}",
        download_dir.display(),
        file.display()
    );

    // ── Win MOTW：等下载循环打上 Zone.Identifier ADS（异步，downloadProgress completed 后才写）──
    #[cfg(windows)]
    {
        let ads = format!("{}:Zone.Identifier", file.display());
        let mut motw: Option<String> = None;
        for _ in 0..50 {
            if let Ok(s) = std::fs::read_to_string(&ads) {
                motw = Some(s);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let content = motw.expect("Zone.Identifier ADS should be written by the download loop (MOTW)");
        eprintln!("MOTW Zone.Identifier content = {content:?}");
        assert!(content.contains("[ZoneTransfer]"), "MOTW must have [ZoneTransfer] header: {content:?}");
        assert!(content.contains("ZoneId=3"), "MOTW must mark Internet zone (ZoneId=3): {content:?}");
    }
    // ── macOS MOTW 等价：下载文件必带 com.apple.quarantine（Gatekeeper 打开可执行/归档时校验）。
    //    两个写者：① chrome 原生 LSFileQuarantine（agent=Chrome,形态更全含源URL+事件UUID）;
    //    ② 我方 write_motw 兜底（XATTR_CREATE,仅 chrome 未落时填，agent=NomiFun）。二者异步竞争,
    //    最终 agent 不定——但**安全不变量=quarantine 存在且标 web-download（0081;）**,与 agent 无关。
    //    故断言「存在 + 0081; 标志」,agent 接受 Chrome（原生）或 NomiFun（兜底）两者。轮询等其落盘。──
    #[cfg(target_os = "macos")]
    {
        let mut q: Option<String> = None;
        for _ in 0..100 {
            let out = std::process::Command::new("/usr/bin/xattr")
                .args(["-p", "com.apple.quarantine"])
                .arg(&file)
                .output();
            if let Ok(o) = out
                && o.status.success()
            {
                q = Some(String::from_utf8_lossy(&o.stdout).trim().to_string());
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let value = q.expect("com.apple.quarantine should be present (chrome native or write_motw fallback) on macOS");
        eprintln!("macOS quarantine = {value:?}");
        assert!(
            value.starts_with("0081;"),
            "quarantine flags must mark web download (0081;...): {value:?}"
        );
        // agent 是 Chrome（chrome 原生先写）或 NomiFun（chrome 未写时我方兜底）——两者都满足
        // 「文件已被 quarantine」这一安全不变量,不耦合具体 agent（消除两写者竞争致的伪 flake）。
        assert!(
            value.contains("Chrome") || value.contains("NomiFun"),
            "quarantine agent should be Chrome (native) or NomiFun (fallback): {value:?}"
        );
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        // linux：write_motw 空实现，无内核级 quarantine 等价——不断言。见 PLATFORM-VERIFICATION.md
        // TODO(verify-linux)。
        eprintln!("linux: MOTW/quarantine is a no-op (no kernel equivalent); skip assertion");
    }

    // 清理：删下载文件（连带其 ADS）。download_dir 由 build helper 下次跑前清。
    let _ = std::fs::remove_file(&file);
}

/// **F1-sec：可执行下载红线 enforcement 端到端**。触发一次 `setup.exe` 下载 → 引擎的下载循环在
/// `Browser.downloadWillBegin` 命中 `reject_executable_download`（.exe denylist）→ `cancelDownload`
/// 取消（**fail-closed，红线，不看 session_mode**）→ 沙箱目录里**不出现**任何非空最终落盘文件。
/// 证「可执行下载在红线会话也拒」（这道判定不吃 session_mode，故即便 yolo/companion 也取消）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn executable_download_is_cancelled_red_line() {
    let (backend, download_dir) =
        common::build_backend_for_fixture_with_downloads("f1sec-exe").await;
    eprintln!("download_dir = {}", download_dir.display());

    backend
        .navigate(&common::fixture_url("download_exe.html"), false)
        .await
        .expect("navigate download_exe.html");

    let obs = backend
        .observe(&ObserveOpts::default())
        .await
        .expect("observe");
    let link = obs
        .entries
        .iter()
        .find(|e| e.role == "link" && e.name.contains("setup.exe"))
        .expect("fixture should expose an executable download link");
    eprintln!("exe download link ref = {}", link.r#ref);

    // 点击触发 .exe 下载。引擎应在 downloadWillBegin 时取消（denylist）。
    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(&ActSpec::Click { r#ref: link.r#ref.clone() }, &p)
        .await;
    eprintln!("click result = {res:?}");

    // 等待窗口：给下载循环时间收到 downloadWillBegin + 发 cancelDownload；其间反复确认沙箱目录里
    // **没有**非空 .exe 最终文件落盘（被取消 → 不应有 completed 落盘）。
    let mut leaked: Option<std::path::PathBuf> = None;
    for _ in 0..50 {
        if let Ok(rd) = std::fs::read_dir(&download_dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                // .crdownload 是中途临时文件（取消后会被清理）——只认最终落盘的非空文件。
                if path.extension().and_then(|e| e.to_str()) == Some("crdownload") {
                    continue;
                }
                if let Ok(meta) = std::fs::metadata(&path)
                    && meta.is_file()
                    && meta.len() > 0
                {
                    leaked = Some(path);
                    break;
                }
            }
        }
        if leaked.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    if let Some(p) = &leaked {
        let _ = std::fs::remove_file(p);
    }
    assert!(
        leaked.is_none(),
        "executable download (.exe) must be cancelled by the red-line (no final file should land), \
         but a file landed: {leaked:?}"
    );
}
