use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const BUILD_MANIFEST_FILE: &str = "nomifun-build.json";
const BUILD_MANIFEST_SCHEMA: u64 = 1;
const FRONTEND_BUILD_ID_ENV: &str = "NOMIFUN_FRONTEND_BUILD_ID";
const STATIC_WEBUI_FEATURE_ENV: &str = "CARGO_FEATURE_STATIC_WEBUI";
const MANIFEST_FIELDS: [&str; 4] = [
    "api_contract_version",
    "app_version",
    "frontend_build_id",
    "schema",
];

#[allow(dead_code)] // Shared by multiple build scripts; desktop uses the explicit-mode variant.
pub fn embed_frontend_build_id(host_name: &str) {
    let profile = env::var("PROFILE").unwrap_or_default();
    let static_build_requested =
        profile == "release" || env::var_os(STATIC_WEBUI_FEATURE_ENV).is_some();
    embed_frontend_build_id_if(host_name, static_build_requested);
}

/// Embed the exact paired frontend identity when the host will serve static
/// assets, regardless of Cargo's optimization profile.
///
/// A Tauri `build --debug` still enables its production custom protocol and
/// embeds `frontendDist`; `PROFILE=debug` therefore does *not* imply a Vite/dev
/// host. The desktop build script passes `!tauri_build::is_dev()` here so fast
/// no-bundle builds receive the same identity guarantees as release bundles.
pub fn embed_frontend_build_id_if(host_name: &str, static_build_requested: bool) {
    println!("cargo:rerun-if-env-changed={STATIC_WEBUI_FEATURE_ENV}");
    if !static_build_requested {
        println!(
            "cargo:warning={host_name} is being built without static WebUI support; ignored ui/dist cannot affect API-only/Vite development"
        );
        return;
    }

    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = crate_dir.join("../../ui/dist").join(BUILD_MANIFEST_FILE);
    let contract_path = crate_dir.join("../../ui-api-contract-version.txt");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-changed={}", contract_path.display());

    let expected_contract = read_contract_version(&contract_path);
    let source = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "failed to read {}: {error}; run `bun run build:ui` before building {host_name}",
            manifest_path.display()
        )
    });
    let value: serde_json::Value = serde_json::from_str(&source).unwrap_or_else(|error| {
        panic!("invalid {}: {error}; run `bun run build:ui`", manifest_path.display())
    });
    let manifest = value
        .as_object()
        .unwrap_or_else(|| panic!("{} must contain a JSON object", manifest_path.display()));
    let actual_fields = manifest.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_fields = MANIFEST_FIELDS.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(
        actual_fields,
        expected_fields,
        "{} must contain exactly schema, app_version, api_contract_version, and frontend_build_id; run `bun run build:ui`",
        manifest_path.display()
    );
    assert_eq!(
        manifest.get("schema").and_then(serde_json::Value::as_u64),
        Some(BUILD_MANIFEST_SCHEMA),
        "{} has an unsupported schema; run `bun run build:ui`",
        manifest_path.display()
    );
    assert_eq!(
        manifest.get("app_version").and_then(serde_json::Value::as_str),
        Some(env!("CARGO_PKG_VERSION")),
        "{} app_version does not match the Rust host; run `bun run build:ui`",
        manifest_path.display()
    );
    assert_eq!(
        manifest
            .get("api_contract_version")
            .and_then(serde_json::Value::as_u64),
        Some(expected_contract),
        "{} api_contract_version does not match ui-api-contract-version.txt; run `bun run build:ui`",
        manifest_path.display()
    );
    let build_id = manifest
        .get("frontend_build_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("{} has no non-empty frontend_build_id", manifest_path.display()));
    println!("cargo:rustc-env={FRONTEND_BUILD_ID_ENV}={build_id}");
}

fn read_contract_version(path: &Path) -> u64 {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let value = source.trim();
    assert!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "{} must contain one positive integer",
        path.display()
    );
    let version = value
        .parse::<u64>()
        .unwrap_or_else(|error| panic!("invalid {}: {error}", path.display()));
    assert!(version > 0, "{} must be greater than zero", path.display());
    version
}
