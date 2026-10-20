use sha2::{Digest, Sha256};
use std::{env, path::PathBuf, process::Command};

fn main() {
    let platform = env::var("CARGO_CFG_TARGET_OS").unwrap();
    if !matches!(platform.as_str(), "macos" | "windows" | "linux") {
        return;
    }
    let sources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/macos-system-execution");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut identity = Sha256::new();
    fn hash_file(root: &std::path::Path, path: &std::path::Path, hash: &mut Sha256) {
        println!("cargo:rerun-if-changed={}", path.display());
        let label = path
            .strip_prefix(root)
            .expect("component input is repository owned")
            .to_string_lossy();
        let bytes = std::fs::read(path).expect("system execution identity input");
        hash.update((label.len() as u64).to_le_bytes());
        hash.update(label.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    fn hash_sources(root: &std::path::Path, path: &std::path::Path, hash: &mut Sha256) {
        println!("cargo:rerun-if-changed={}", path.display());
        let mut children = std::fs::read_dir(path)
            .expect("system execution source inventory")
            .map(|entry| entry.expect("source entry").path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            if child.is_dir() {
                hash_sources(root, &child, hash);
            } else if matches!(
                child.extension().and_then(|v| v.to_str()),
                Some("rs" | "swift" | "toml" | "plist")
            ) {
                hash_file(root, &child, hash);
            }
        }
    }
    for relative in [
        "Cargo.toml",
        "Cargo.lock",
        "crates/shared/nomi-system-execution/Cargo.toml",
        "crates/shared/nomi-process-runtime/Cargo.toml",
        "crates/shared/nomi-native-files/Cargo.toml",
    ] {
        hash_file(&root, &root.join(relative), &mut identity);
    }
    // Installation and native admission are part of the component identity,
    // along with the build logic that selects and fingerprints these inputs.
    for relative in [
        "apps/desktop/linux-system-execution/com.nomifun.system-execution.policy",
        "apps/desktop/linux-system-execution/install.sh",
        "crates/shared/nomi-system-execution/build.rs",
    ] {
        hash_file(&root, &root.join(relative), &mut identity);
    }
    for relative in [
        "crates/shared/nomi-system-execution/src",
        "crates/shared/nomi-process-runtime/src",
        "crates/shared/nomi-native-files/src",
        "apps/desktop/macos-system-execution",
    ] {
        hash_sources(&root, &root.join(relative), &mut identity);
    }
    if platform == "windows" {
        for relative in [
            "apps/desktop/src/main.rs", "apps/web/src/main.rs",
            "crates/backend/nomifun-app/src/main.rs", "crates/backend/nomifun-app/src/lib.rs",
            "crates/backend/nomifun-app/Cargo.toml",
        ] {
            hash_file(&root, &root.join(relative), &mut identity);
        }
    }
    identity.update(env::var("TARGET").expect("build target"));
    identity.update(env!("CARGO_PKG_VERSION"));
    println!(
        "cargo:rustc-env=NOMIFUN_SYSTEM_EXECUTION_BUILD_ID={:x}",
        identity.finalize()
    );
    if platform != "macos" {
        return;
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS system execution architecture {other}"),
    };
    for name in ["Protocol.swift", "Client.swift"] {
        println!("cargo:rerun-if-changed={}", sources.join(name).display());
    }
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-swift-version",
            "5",
            "-O",
            "-module-name",
            "NomiFunSystemExecutionClient",
            "-target",
            &format!("{arch}-apple-macosx14.0"),
        ])
        .arg(sources.join("Protocol.swift"))
        .arg(sources.join("Client.swift"))
        .arg("-o")
        .arg(out.join("libnomifun_system_client.a"))
        .status()
        .expect("macOS Swift compiler is required for native system execution");
    assert!(
        status.success(),
        "native macOS system execution bridge failed to compile"
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=nomifun_system_client");
    for framework in ["Foundation", "ServiceManagement", "Security", "AppKit"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    // Swift's ABI-stable runtime is supplied by the minimum supported macOS.
    let sdk = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .expect("macOS SDK path");
    assert!(sdk.status.success(), "macOS SDK path lookup failed");
    let sdk = PathBuf::from(
        String::from_utf8(sdk.stdout)
            .expect("UTF-8 SDK path")
            .trim(),
    );
    // Select the SDK's ABI-stable system dylib stubs before any toolchain
    // back-deployment libraries. Their install names are /usr/lib/swift/*,
    // so every downstream Rust binary/test works without a transitive rpath
    // (Cargo does not propagate rustc-link-arg from an rlib dependency).
    println!(
        "cargo:rustc-link-search=native={}",
        sdk.join("usr/lib/swift").display()
    );
    println!("cargo:rustc-link-lib=dylib=swiftCore");
    println!("cargo:rustc-link-lib=dylib=swift_Concurrency");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
