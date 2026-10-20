// Copyright (c) 2025 Cua AI, Inc. MIT License; see LICENSE.
// Swift runtime linking and Windows DPI resources adapted from trycua/cua
// libs/cua-driver/rust/crates/cua-driver/build.rs at
// 4d959bbfa57f0c89f310330e849500b222084194.

fn main() {
    println!("cargo:rustc-env=CUA_DRIVER_GIT_SHA=4d959bbfa57f0c89f310330e849500b222084194");
    println!("cargo:rerun-if-changed=nomifun-computer-worker.manifest");
    println!("cargo:rerun-if-changed=nomifun-computer-worker.rc");
    #[cfg(target_os = "windows")]
    embed_resource::compile("nomifun-computer-worker.rc", embed_resource::NONE);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        emit_sdk_framework_search_path();
        emit_swift_runtime_link_args();
    }
}
fn emit_sdk_framework_search_path() {
    if let Some(sdk_root) = active_macos_sdk_root() {
        println!(
            "cargo:rustc-link-search=framework={}",
            sdk_root.join("System/Library/Frameworks").display()
        );
    }
}

fn emit_swift_runtime_link_args() {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    let mut dirs = BTreeSet::new();

    // `swiftc` links against the SDK's Swift TBDs on current Xcode releases.
    // The toolchain runtime directories below contain compatibility libraries,
    // but are not guaranteed to contain the full Swift runtime surface.
    if let Some(sdk_root) = active_macos_sdk_root() {
        let sdk_swift_dir = sdk_root.join("usr/lib/swift");
        if sdk_swift_dir.is_dir() {
            println!("cargo:rustc-link-search=native={}", sdk_swift_dir.display());
        }
    }

    if let Ok(out) = std::process::Command::new("xcode-select")
        .arg("-p")
        .output()
    {
        if out.status.success() {
            let xcode_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let developer_dir = PathBuf::from(xcode_path);
            for sub in [
                "Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
                "Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx",
                "Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-6.2/macosx",
                "usr/lib/swift/macosx",
                "usr/lib/swift-5.5/macosx",
                "usr/lib/swift-6.2/macosx",
            ] {
                dirs.insert(developer_dir.join(sub));
            }
        }
    }

    if let Ok(out) = Command::new("xcrun").args(["--find", "swiftc"]).output() {
        if out.status.success() {
            let swiftc = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
            if let Some(usr_dir) = swiftc.parent().and_then(Path::parent) {
                for sub in [
                    "lib/swift/macosx",
                    "lib/swift-5.5/macosx",
                    "lib/swift-6.2/macosx",
                ] {
                    dirs.insert(usr_dir.join(sub));
                }
            }
        }
    }

    for dir in dirs {
        if dir.is_dir() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
        }
    }
}

fn active_macos_sdk_root() -> Option<std::path::PathBuf> {
    std::env::var_os("SDKROOT")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            let out = std::process::Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
                .filter(|path| path.is_dir())
        })
}
