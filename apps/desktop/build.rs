#[path = "../build-support/ui_build_manifest.rs"]
mod ui_build_manifest;

fn main() {
    // Tauri's dev/custom-protocol mode is the source of truth here. In
    // particular, `tauri build --debug` has PROFILE=debug but still embeds and
    // serves production frontendDist assets, so it needs an exact build ID.
    ui_build_manifest::embed_frontend_build_id_if("nomifun-desktop", !tauri_build::is_dev());
    tauri_build::build()
}
