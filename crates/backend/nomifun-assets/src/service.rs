use std::path::{Component, Path, PathBuf};

use http::HeaderValue;
use nomifun_common::AppError;
use rust_embed::RustEmbed;
use sha2::{Digest, Sha256};

#[derive(RustEmbed)]
#[folder = "assets/logos/"]
struct LogoAssets;

/// Resolved static asset bytes plus cache metadata.
pub struct AssetFile {
    pub bytes: Vec<u8>,
    pub content_type: HeaderValue,
    pub etag: HeaderValue,
}

/// Service resolving embedded logo assets.
#[derive(Clone, Default)]
pub struct AssetService;

impl AssetService {
    /// Look up a logo asset by its route-relative path.
    pub fn get_logo(&self, asset_path: &str) -> Result<AssetFile, AppError> {
        let normalized = normalize_logo_path(asset_path)
            .ok_or_else(|| AppError::Forbidden(format!("Asset path escapes logos root: {asset_path}")))?;

        let file = LogoAssets::get(&normalized).ok_or_else(|| AppError::NotFound("Logo asset not found".into()))?;
        let bytes = file.data.into_owned();

        Ok(AssetFile {
            content_type: content_type_for_path(&normalized),
            etag: build_etag(&bytes)?,
            bytes,
        })
    }

    /// Return `true` when the request ETag already matches the asset.
    pub fn etag_matches(&self, header_value: Option<&HeaderValue>, etag: &HeaderValue) -> bool {
        let Some(header_value) = header_value else {
            return false;
        };
        let Ok(expected) = etag.to_str() else {
            return false;
        };
        let Ok(candidate) = header_value.to_str() else {
            return false;
        };

        candidate
            .split(',')
            .map(str::trim)
            .any(|value| value == "*" || value == expected)
    }
}

fn normalize_logo_path(path: &str) -> Option<String> {
    if path.contains('\\') || path.contains(':') {
        return None;
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => return None,
        }
    }

    if normalized.as_os_str().is_empty() {
        return None;
    }

    Some(normalized.to_string_lossy().replace('\\', "/"))
}

fn content_type_for_path(path: &str) -> HeaderValue {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    HeaderValue::from_str(mime.as_ref()).unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"))
}

fn build_etag(bytes: &[u8]) -> Result<HeaderValue, AppError> {
    let digest = Sha256::digest(bytes);
    HeaderValue::from_str(&format!("\"{digest:x}\"")).map_err(|error| AppError::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_logo_path_rejects_traversal() {
        assert!(normalize_logo_path("../brand/nomi.svg").is_none());
        assert!(normalize_logo_path("/etc/passwd").is_none());
        assert!(normalize_logo_path("C:\\Windows\\System32").is_none());
    }

    #[test]
    fn normalize_logo_path_preserves_nested_relative_paths() {
        assert_eq!(
            normalize_logo_path("./ai-major/claude.svg").as_deref(),
            Some("ai-major/claude.svg")
        );
    }

    #[test]
    fn get_logo_returns_bytes_and_metadata() {
        let service = AssetService;
        let asset = service.get_logo("ai-major/claude.svg").expect("claude logo present");

        assert_eq!(asset.content_type, HeaderValue::from_static("image/svg+xml"));
        assert!(!asset.bytes.is_empty());
        assert!(asset.etag.to_str().unwrap().starts_with('"'));
    }

    #[test]
    fn etag_matches_supports_exact_and_star_values() {
        let service = AssetService;
        let etag = HeaderValue::from_static("\"abc\"");

        assert!(service.etag_matches(Some(&HeaderValue::from_static("\"abc\"")), &etag));
        assert!(service.etag_matches(Some(&HeaderValue::from_static("*")), &etag));
        assert!(service.etag_matches(Some(&HeaderValue::from_static("\"def\", \"abc\"")), &etag));
        assert!(!service.etag_matches(Some(&HeaderValue::from_static("\"def\"")), &etag));
    }
}
