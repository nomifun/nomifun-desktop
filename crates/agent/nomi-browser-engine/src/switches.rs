//! Chromium 启动开关表 —— 静态硬化基线（零后台出站 / 容器防崩 / 截图可复现）。
//!
//! 移植自 Playwright Apache-2.0 源码：
//! `packages/playwright-core/src/server/chromium/chromiumSwitches.ts`
//! - 仓库：<https://github.com/microsoft/playwright>
//! - 取源 URL：<https://raw.githubusercontent.com/microsoft/playwright/main/packages/playwright-core/src/server/chromium/chromiumSwitches.ts>
//! - 固定 commit（DESIGN §24）：`4b1b9d681f8a7b1dffafa973ef705f28661d4607`（2026-04-14）
//! - 移植日期：2026-06-17
//!
//! Copyright 2017 Google Inc. All rights reserved.
//! Modifications copyright (c) Microsoft Corporation.
//! Licensed under the Apache License, Version 2.0（许可署名见上，详见 LICENSE）。
//!
//! 移植取舍（本引擎 `browserName` 固定 chromium，非 Edge、非 android）：
//! - **删 Edge 专属**：`--disable-edgeupdater`、`--edge-skip-compat-layer-relaunch`，
//!   以及 disable-features 中的 `msForceBrowserSignIn`、
//!   `msEdgeUpdateLaunchServicesPreferredVersion`（Edge 行为）。
//!   注：`AutoDeElevate` 经核对 PW 源是 Chromium 项（非 Edge），已**保留**——忠实移植。
//! - **去重**：`--disable-search-engine-choice-screen` 在 PW 源出现两次（幂等），本表保留一次。
//! - **删 android 分支**：PW `options?.android ? '' : '--disable-sync'` →
//!   非 android 恒取 `--disable-sync`。
//! - `--disable-dev-shm-usage`：PW 标为容器/Linux 相关，故仅 Linux 下 push
//!   （防容器 `/dev/shm` 过小致 Chromium 崩溃）。
//! - **不含动态开关**：`--headless` / `--no-sandbox` / `--remote-debugging-port` /
//!   `--user-data-dir` / `--window-*` 由 launch（Task 7）按 display/容器/headful
//!   动态决定，本表只出静态硬化基线。
//! - **安全红线**：`--disable-back-forward-cache` 对请求拦截正确性必须
//!   （避免 `page.goBack()` 时主请求绕过拦截），勿因性能删。

/// PW `disabledFeatures` 列表中与 Chromium 相关、非 Edge 的项。
///
/// 拼成单个 `--disable-features=A,B,C`。每项的来源 issue/PR 见 PW 源注释。
/// 已删 Edge 专属项：`msForceBrowserSignIn`、`msEdgeUpdateLaunchServicesPreferredVersion`
/// （`AutoDeElevate` 是 Chromium 项非 Edge，已保留）。
const DISABLED_FEATURES: &[&str] = &[
    // https://github.com/microsoft/playwright/issues/14047
    "AvoidUnnecessaryBeforeUnloadCheckSync",
    // https://github.com/microsoft/playwright/issues/38568
    "BoundaryEventDispatchTracksNodeRemoval",
    "DestroyProfileOnBrowserClose",
    // https://github.com/microsoft/playwright/pull/13854
    "DialMediaRouteProvider",
    "GlobalMediaControls",
    // https://github.com/microsoft/playwright/pull/27605
    "HttpsUpgrades",
    // 隐藏地址栏 Lens 功能（在非官方构建里本就不工作）。
    "LensOverlay",
    // https://github.com/microsoft/playwright/pull/8162
    "MediaRouter",
    // https://github.com/microsoft/playwright/issues/28023
    "PaintHolding",
    // https://github.com/microsoft/playwright/issues/32230
    "ThirdPartyStoragePartitioning",
    // https://github.com/microsoft/playwright/issues/16126
    "Translate",
    // Chromium 项（非 Edge）：https://issues.chromium.org/issues/435410220
    "AutoDeElevate",
    // https://github.com/microsoft/playwright/issues/37714
    "RenderDocument",
    // 阻止启动时下载 optimization hints。
    "OptimizationHints",
];

/// 返回 Chromium 启动开关的静态硬化基线。
///
/// 仅含静态项；display/容器/headful 相关的动态开关由 launch 层决定。
/// 顺序忠实于 PW 源（删去 Edge/android 项后保持相对次序）。
pub fn chromium_switches() -> Vec<String> {
    // https://source.chromium.org/chromium/chromium/src/+/main:testing/variations/README.md
    let mut switches: Vec<String> = vec![
        "--disable-field-trial-config".into(),
        "--disable-background-networking".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-backgrounding-occluded-windows".into(),
        // 避免 page.goBack() 时主请求绕过拦截等意外。安全红线，勿删。
        "--disable-back-forward-cache".into(),
        "--disable-breakpad".into(),
        "--disable-client-side-phishing-detection".into(),
        "--disable-component-extensions-with-background-pages".into(),
        // 避免启动后无谓的网络活动。
        "--disable-component-update".into(),
        "--no-default-browser-check".into(),
        "--disable-default-apps".into(),
    ];

    // PW 标为容器/Linux 相关：仅 Linux 下 push，防容器 /dev/shm 过小致崩溃。
    #[cfg(target_os = "linux")]
    switches.push("--disable-dev-shm-usage".into());

    switches.extend([
        "--disable-extensions".into(),
        format!("--disable-features={}", DISABLED_FEATURES.join(",")),
        // 新的 CDP 截图取面，保证截图可复现（PW 默认开，除非 legacy env）。
        "--enable-features=CDPScreenshotNewSurface".into(),
        "--allow-pre-commit-input".into(),
        "--disable-hang-monitor".into(),
        "--disable-ipc-flooding-protection".into(),
        "--disable-popup-blocking".into(),
        "--disable-prompt-on-repost".into(),
        "--disable-renderer-backgrounding".into(),
        "--force-color-profile=srgb".into(),
        "--metrics-recording-only".into(),
        "--no-first-run".into(),
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        // https://chromium-review.googlesource.com/c/chromium/src/+/2436773
        "--no-service-autorun".into(),
        "--export-tagged-pdf".into(),
        // https://chromium-review.googlesource.com/c/chromium/src/+/4853540
        // （PW 源列两次，此处去重为一次；该开关幂等。）
        "--disable-search-engine-choice-screen".into(),
        // https://issues.chromium.org/41491762
        "--unsafely-disable-devtools-self-xss-warnings".into(),
        // 关闭 Chrome for Testing 在持久化上下文里可见的 infobar。
        "--disable-infobars".into(),
        // 非 android 恒带（PW 在 android 时省略），避免 ephemeral 上下文菜单崩溃。
        "--disable-sync".into(),
    ]);

    switches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_security_hardening_switches() {
        let s = chromium_switches();
        assert!(s.iter().any(|x| x == "--disable-background-networking"));
        assert!(s.iter().any(|x| x == "--disable-component-update"));
        assert!(s.iter().any(|x| x == "--disable-back-forward-cache"));
        assert!(s.iter().any(|x| x.starts_with("--disable-features=")));
    }

    #[test]
    fn includes_all_named_baseline_switches() {
        let s = chromium_switches();
        for expected in [
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-back-forward-cache",
            "--force-color-profile=srgb",
            "--password-store=basic",
            "--use-mock-keychain",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-default-apps",
            "--disable-popup-blocking",
            "--metrics-recording-only",
            "--disable-hang-monitor",
            "--disable-sync",
        ] {
            assert!(
                s.iter().any(|x| x == expected),
                "missing expected switch: {expected}"
            );
        }
    }

    #[test]
    fn disable_features_contains_expected_entries() {
        let s = chromium_switches();
        let df = s
            .iter()
            .find(|x| x.starts_with("--disable-features="))
            .expect("has disable-features");
        for entry in [
            "Translate",
            "HttpsUpgrades",
            "OptimizationHints",
            "ThirdPartyStoragePartitioning",
            "AutoDeElevate", // Chromium 项（非 Edge），忠实移植保留
        ] {
            assert!(df.contains(entry), "disable-features missing: {entry}");
        }
    }

    #[test]
    fn disable_features_excludes_edge_specific() {
        let s = chromium_switches();
        let df = s
            .iter()
            .find(|x| x.starts_with("--disable-features="))
            .expect("has disable-features");
        // Edge 专属 feature 必须已删。
        for edge in [
            "msForceBrowserSignIn",
            "msEdgeUpdateLaunchServicesPreferredVersion",
        ] {
            assert!(!df.contains(edge), "edge-specific feature leaked: {edge}");
        }
    }

    #[test]
    fn no_dynamic_or_edge_switches() {
        let s = chromium_switches();
        // 动态开关不得混入静态表。
        assert!(!s.iter().any(|x| x.starts_with("--headless")));
        assert!(!s.iter().any(|x| x.starts_with("--no-sandbox")));
        assert!(!s.iter().any(|x| x.starts_with("--remote-debugging-port")));
        assert!(!s.iter().any(|x| x.starts_with("--user-data-dir")));
        assert!(!s.iter().any(|x| x.starts_with("--window-")));
        // Edge 专属开关必须已删。
        assert!(!s.iter().any(|x| x == "--disable-edgeupdater"));
        assert!(!s.iter().any(|x| x == "--edge-skip-compat-layer-relaunch"));
    }

    #[test]
    fn no_empty_switches() {
        // PW 用 .filter(Boolean) 去空串；Rust 侧不应产生空串。
        let s = chromium_switches();
        assert!(s.iter().all(|x| !x.is_empty()));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn dev_shm_usage_only_on_linux() {
        let s = chromium_switches();
        assert!(!s.iter().any(|x| x == "--disable-dev-shm-usage"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dev_shm_usage_present_on_linux() {
        let s = chromium_switches();
        assert!(s.iter().any(|x| x == "--disable-dev-shm-usage"));
    }
}
