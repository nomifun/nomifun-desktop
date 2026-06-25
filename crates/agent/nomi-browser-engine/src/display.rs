//! 显示器探测：上层据此决定是否强制 headless 启动 Chromium。
//!
//! 无显示器（典型场景：无头 Linux 服务器、CI、SSH 无 X 转发）时
//! [`display_available`] 返回 `false`，调用方据此强制 `--headless`。
//!
//! 三平台分派：
//! - **Linux**：env 判定，与 `nomi-a11y` 的 `detect_caps`
//!   （`crates/agent/nomi-a11y/src/linux/actor.rs`）**同源判据（简化版）** ——
//!   `$WAYLAND_DISPLAY` 优先于 `$DISPLAY`。a11y 还参考 `$XDG_SESSION_TYPE`、用
//!   `var_os`；此处对「是否有显示」这个布尔目的取等价的简化子集即可。纯逻辑
//!   helper 不带 cfg 门控，故其单测可在任意宿主（含 Windows 开发机）真跑。
//! - **Windows**：`GetSystemMetrics(SM_CMONITORS)` 监视器数 > 0，
//!   复用 `windows` 0.61 绑定（与 nomi-a11y / nomi-computer 对齐）。
//! - **macOS**：`CGDisplay::active_display_count() > 0`，
//!   复用 `core-graphics` 0.25（与 nomi-a11y 对齐）提供的安全封装。

/// 是否存在可用的图形显示。无显示器时上层强制 headless 启动。
///
/// TODO(verify-linux): 纯逻辑 helper 已在 Windows 开发机单测；但**真实 env 分派**
/// （此分支）需在 Linux 上验证：有 X/Wayland→true、纯 headless server→false。
/// 见 docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
#[cfg(target_os = "linux")]
pub fn display_available() -> bool {
    display_available_from(|k| std::env::var(k).ok())
}

/// 纯逻辑：给定读 env 的闭包，判断是否存在图形显示。Wayland 优先于 X11。
/// 不带 cfg 门控 —— 这样它的单测能在任意宿主（含 Windows 开发机）运行。
/// 在非 Linux 构建里仅测试消费它，故 allow(dead_code)。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn display_available_from(get: impl Fn(&str) -> Option<String>) -> bool {
    get("WAYLAND_DISPLAY").is_some() || get("DISPLAY").is_some()
}

/// 是否存在可用的图形显示。Windows 上恒有桌面会话即返回 `true`
/// （`SM_CMONITORS` 监视器数 > 0）。
#[cfg(target_os = "windows")]
pub fn display_available() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};

    // SAFETY: GetSystemMetrics 只读全局显示指标，无前置条件。
    // SM_CMONITORS 返回当前桌面上的显示器数量（无显示器时为 0）。
    let monitors = unsafe { GetSystemMetrics(SM_CMONITORS) };
    monitors > 0
}

/// 是否存在可用的图形显示（macOS：活动显示器数 > 0）。
/// 无头/无显示 Mac（罕见）或显示全部休眠时为 `false`。
///
/// TODO(verify-macos): core-graphics `CGDisplay::active_display_count` 在 Windows
/// 宿主无法编译验证；需在 Mac 上 `cargo build/nextest -p nomi-browser-engine` 确认
/// 编译+链接+行为。见 docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
#[cfg(target_os = "macos")]
pub fn display_available() -> bool {
    use core_graphics::display::CGDisplay;

    // active_display_count() 是 core-graphics 对 CGGetActiveDisplayList 的安全封装：
    // 先以 max_displays=0 调用拿到数量。失败（无窗口服务器等）按无显示处理。
    CGDisplay::active_display_count()
        .map(|n| n > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_display_env_means_unavailable() {
        assert!(!display_available_from(|_| None));
    }

    #[test]
    fn x11_display_present_means_available() {
        assert!(display_available_from(
            |k| if k == "DISPLAY" { Some(":0".into()) } else { None }
        ));
    }

    #[test]
    fn wayland_display_present_means_available() {
        assert!(display_available_from(|k| if k == "WAYLAND_DISPLAY" {
            Some("wayland-0".into())
        } else {
            None
        }));
    }

    #[test]
    fn wayland_preferred_when_both_present() {
        // 两者皆在时仍返回 available；helper 短路在 Wayland 上。
        assert!(display_available_from(|k| match k {
            "WAYLAND_DISPLAY" => Some("wayland-0".into()),
            "DISPLAY" => Some(":0".into()),
            _ => None,
        }));
    }
}
