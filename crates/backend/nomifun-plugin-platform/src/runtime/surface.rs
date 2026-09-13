use getrandom::getrandom;
use nomifun_agent_contracts::digest_bytes;
use nomifun_api_types::is_preview_capability;

use crate::runtime::{PluginRuntimeM1ApplicationError, PluginRuntimeSurfaceAsset};

const SURFACE_CAPABILITY_BYTES: usize = 32;

pub fn issue_surface_capability() -> Result<String, PluginRuntimeM1ApplicationError> {
    let mut bytes = [0u8; SURFACE_CAPABILITY_BYTES];
    getrandom(&mut bytes).map_err(|error| {
        PluginRuntimeM1ApplicationError::Invalid(format!(
            "failed to generate Surface capability: {error}"
        ))
    })?;
    Ok(hex::encode(bytes))
}

pub fn surface_capability_digest(
    capability: &str,
) -> Result<String, PluginRuntimeM1ApplicationError> {
    if !is_preview_capability(capability) {
        return Err(PluginRuntimeM1ApplicationError::NotFound);
    }
    Ok(digest_bytes(capability.as_bytes()).0)
}

pub fn content_type_for_surface_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

pub fn surface_asset_response(
    asset: PluginRuntimeSurfaceAsset,
) -> (String, Vec<u8>) {
    (
        content_type_for_surface_path(&asset.normalized_relative_path).to_owned(),
        asset.bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_capabilities_are_opaque_and_only_digest_is_persistable() {
        let first = issue_surface_capability().unwrap();
        let second = issue_surface_capability().unwrap();
        assert_eq!(first.len(), SURFACE_CAPABILITY_BYTES * 2);
        assert_ne!(first, second);
        assert_eq!(surface_capability_digest(&first).unwrap().len(), 64);
        assert_ne!(
            surface_capability_digest(&first).unwrap(),
            surface_capability_digest(&second).unwrap()
        );
        assert!(matches!(
            surface_capability_digest("not-a-capability"),
            Err(PluginRuntimeM1ApplicationError::NotFound)
        ));
    }

    #[test]
    fn surface_content_types_are_nosniff_friendly() {
        assert_eq!(
            content_type_for_surface_path("ui/index.html"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type_for_surface_path("ui/app.mjs"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type_for_surface_path("ui/blob.bin"),
            "application/octet-stream"
        );
    }
}
