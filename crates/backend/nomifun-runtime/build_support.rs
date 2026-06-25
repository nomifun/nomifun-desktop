//! Pure helpers for `build.rs`. Kept separate so they can be unit-tested
//! without running the build script itself.
//!
//! Exposed to `build.rs` via `#[path = "build_support.rs"] mod build_support;`.

/// Whether this target has an embedded bun asset.
pub fn has_embedded_bun(target: &str) -> bool {
    target != "aarch64-pc-windows-msvc"
}

/// Map a Rust target triple + variant to the bun release asset filename.
/// Returns None for targets without a bun prebuild.
pub fn asset_name_for(target: &str, variant: &str) -> Option<String> {
    let (platform, arch) = match target {
        "x86_64-apple-darwin" => ("darwin", "x64"),
        "aarch64-apple-darwin" => ("darwin", "aarch64"),
        "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => ("linux", "x64"),
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => ("linux", "aarch64"),
        "x86_64-pc-windows-msvc" | "x86_64-pc-windows-gnu" => ("windows", "x64"),
        _ => return None,
    };
    let suffix = if variant == "baseline" { "-baseline" } else { "" };
    Some(format!("bun-{platform}-{arch}{suffix}.zip"))
}

/// Compose the GitHub download URL for a given bun version + asset.
pub fn download_url(version: &str, asset: &str) -> String {
    format!("https://github.com/oven-sh/bun/releases/download/bun-v{version}/{asset}")
}

/// Expected filename of the bun executable extracted from the zip.
pub fn bun_exe_name(target: &str) -> &'static str {
    if target.contains("windows") { "bun.exe" } else { "bun" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_embedded_bun_false_only_for_win_arm64() {
        assert!(has_embedded_bun("x86_64-apple-darwin"));
        assert!(has_embedded_bun("aarch64-apple-darwin"));
        assert!(has_embedded_bun("x86_64-unknown-linux-gnu"));
        assert!(has_embedded_bun("aarch64-unknown-linux-gnu"));
        assert!(has_embedded_bun("x86_64-pc-windows-msvc"));
        assert!(!has_embedded_bun("aarch64-pc-windows-msvc"));
    }

    #[test]
    fn asset_name_default_variants() {
        assert_eq!(
            asset_name_for("x86_64-apple-darwin", "default").as_deref(),
            Some("bun-darwin-x64.zip")
        );
        assert_eq!(
            asset_name_for("aarch64-apple-darwin", "default").as_deref(),
            Some("bun-darwin-aarch64.zip")
        );
        assert_eq!(
            asset_name_for("x86_64-unknown-linux-gnu", "default").as_deref(),
            Some("bun-linux-x64.zip")
        );
        assert_eq!(
            asset_name_for("aarch64-unknown-linux-gnu", "default").as_deref(),
            Some("bun-linux-aarch64.zip")
        );
        assert_eq!(
            asset_name_for("x86_64-pc-windows-msvc", "default").as_deref(),
            Some("bun-windows-x64.zip")
        );
    }

    #[test]
    fn asset_name_baseline_variant() {
        assert_eq!(
            asset_name_for("x86_64-unknown-linux-gnu", "baseline").as_deref(),
            Some("bun-linux-x64-baseline.zip")
        );
    }

    #[test]
    fn asset_name_none_for_win_arm64() {
        assert!(asset_name_for("aarch64-pc-windows-msvc", "default").is_none());
    }

    #[test]
    fn download_url_format() {
        assert_eq!(
            download_url("1.1.38", "bun-darwin-x64.zip"),
            "https://github.com/oven-sh/bun/releases/download/bun-v1.1.38/bun-darwin-x64.zip"
        );
    }

    #[test]
    fn bun_exe_name_platform_specific() {
        assert_eq!(bun_exe_name("x86_64-apple-darwin"), "bun");
        assert_eq!(bun_exe_name("x86_64-unknown-linux-gnu"), "bun");
        assert_eq!(bun_exe_name("x86_64-pc-windows-msvc"), "bun.exe");
        assert_eq!(bun_exe_name("aarch64-pc-windows-msvc"), "bun.exe");
    }
}
