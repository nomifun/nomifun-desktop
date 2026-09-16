#[path = "../build-support/ui_build_manifest.rs"]
mod ui_build_manifest;

fn main() {
    // Tauri's dev/custom-protocol mode is the source of truth here. In
    // particular, `tauri build --debug` has PROFILE=debug but still embeds and
    // serves production frontendDist assets, so it needs an exact build ID.
    ui_build_manifest::embed_frontend_build_id_if("nomifun-desktop", !tauri_build::is_dev());
    // tauri-winres links the app resource to binaries, not Cargo examples.
    // Native browser examples also import TaskDialogIndirect from Common
    // Controls v6 and need their own embedded activation manifest.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/windows.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-examples=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::build()
}
