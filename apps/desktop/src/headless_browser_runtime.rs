//! Installed Headless browser supply for search and Knowledge rendering. Inspect metadata only; never
//! launch a browser, inspect login state, or query the web during app startup.
use std::path::{Path, PathBuf};
#[cfg(windows)]
use windows::{
    Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    },
    core::PCWSTR,
};

#[cfg(windows)]
fn candidates() -> Vec<PathBuf> {
    candidate_paths(
        ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from),
    )
}
#[cfg(windows)]
fn candidate_paths(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths = vec![];
    for root in roots {
        let path = root.join("Google/Chrome/Application/chrome.exe");
        if path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(windows)]
fn product(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // The APIs parse an on-disk PE resource; they do not load/execute Chrome.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(name.as_ptr()), None);
        if size == 0 || size > 1024 * 1024 {
            return None;
        }
        let mut bytes = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(name.as_ptr()), None, size, bytes.as_mut_ptr().cast()).ok()?;
        let mut pointer = std::ptr::null_mut();
        let mut length = 0u32;
        if !VerQueryValueW(
            bytes.as_ptr().cast(),
            windows::core::w!("\\"),
            &mut pointer,
            &mut length,
        )
        .as_bool()
        {
            return None;
        }
        let start = pointer as usize;
        let base = bytes.as_ptr() as usize;
        let required = std::mem::size_of::<VS_FIXEDFILEINFO>();
        if length < (required as u32)
            || start < base
            || start.checked_add(required)? > base + bytes.len()
        {
            return None;
        }
        let info = std::ptr::read_unaligned(pointer.cast::<VS_FIXEDFILEINFO>());
        if info.dwSignature != 0xFEEF04BD {
            return None;
        }
        let major = info.dwProductVersionMS >> 16;
        if major < 120 {
            return None;
        }
        Some(format!(
            "Chrome/{major}.{}.{}.{}",
            info.dwProductVersionMS & 0xffff,
            info.dwProductVersionLS >> 16,
            info.dwProductVersionLS & 0xffff
        ))
    }
}

#[cfg(target_os = "macos")]
fn candidates() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    candidate_paths(roots)
}

#[cfg(target_os = "macos")]
fn candidate_paths(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths = vec![];
    for root in roots {
        let path = root.join("Google Chrome.app/Contents/MacOS/Google Chrome");
        if path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(target_os = "macos")]
fn product(path: &Path) -> Option<String> {
    use std::{io::Read, os::unix::fs::PermissionsExt};

    if !path.is_absolute() || path.file_name()? != "Google Chrome" {
        return None;
    }
    let executable = std::fs::metadata(path).ok()?;
    if !executable.is_file() || executable.permissions().mode() & 0o111 == 0 {
        return None;
    }
    let macos = path.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    // Read both XML and binary property lists in-process. NSBundle loading or a
    // Chrome --version subprocess is unnecessary and would violate startup's
    // metadata-only contract. Bound the source before parsing untrusted bytes.
    let metadata_path = contents.join("Info.plist");
    let metadata = std::fs::metadata(&metadata_path).ok()?;
    if !metadata.is_file()
        || metadata.len() > 1024 * 1024
        || !metadata_path
            .canonicalize()
            .ok()?
            .starts_with(contents.canonicalize().ok()?)
    {
        return None;
    }
    let file = std::fs::File::open(metadata_path).ok()?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let plist = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    let dictionary = plist.as_dictionary()?;
    if dictionary.get("CFBundleIdentifier")?.as_string()? != "com.google.Chrome"
        || dictionary.get("CFBundleExecutable")?.as_string()? != "Google Chrome"
    {
        return None;
    }
    let version = dictionary.get("CFBundleShortVersionString")?.as_string()?;
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 4
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || part.parse::<u16>().is_err()
        })
        || parts[0].parse::<u16>().ok()? < 120
    {
        return None;
    }
    Some(format!("Chrome/{version}"))
}

pub(crate) async fn prepare(host: &mut nomifun_app::DesktopHostServices) {
    if host.local_web_search.is_some() && host.headless_render.is_some() {
        return;
    }
    let installed = tokio::task::spawn_blocking(|| {
        candidates()
            .into_iter()
            .filter_map(|path| product(&path).map(|version| (path, version)))
            .collect::<Vec<_>>()
    })
    .await;
    let installed = match installed {
        Ok(installed) => installed,
        Err(_) => {
            tracing::warn!(
                "Headless browser capabilities are unavailable: installed Chrome discovery did not complete"
            );
            return;
        }
    };
    let mut admission_failed = false;
    for (path, version) in installed {
        if host.set_browser_release(path, version).await.is_ok() {
            return;
        }
        admission_failed = true;
    }
    if admission_failed {
        // No process was created, so this optional dependency failure has no
        // browser cleanup authority to lose or transfer to desktop startup.
        tracing::warn!(
            "Headless browser capabilities are unavailable: installed Chrome metadata could not be admitted"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn candidate_paths_are_absolute_deduplicated_and_do_not_search_path() {
        let root = PathBuf::from("C:/Known Install");
        assert_eq!(
            candidate_paths([PathBuf::from("relative"), root.clone(), root.clone()]),
            vec![root.join("Google/Chrome/Application/chrome.exe")]
        );
    }
    #[cfg(windows)]
    #[test]
    fn missing_or_non_pe_files_are_not_browser_releases() {
        let directory = tempfile::tempdir().unwrap();
        assert!(product(&directory.path().join("missing.exe")).is_none());
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(product(file.path()).is_none());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn candidate_paths_are_absolute_deduplicated_and_include_user_applications() {
        let system = PathBuf::from("/Applications");
        let user = PathBuf::from("/Users/alice/Applications");
        assert_eq!(
            candidate_paths([
                PathBuf::from("relative"),
                system.clone(),
                user.clone(),
                system.clone()
            ]),
            vec![
                system.join("Google Chrome.app/Contents/MacOS/Google Chrome"),
                user.join("Google Chrome.app/Contents/MacOS/Google Chrome"),
            ]
        );
    }
    #[cfg(target_os = "macos")]
    fn bundle_metadata_fixture(version: &str, binary_plist: bool) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("Google Chrome.app/Contents/MacOS/Google Chrome");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Executable marker only: these metadata tests never launch the file.
        std::fs::write(&path, b"metadata fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("CFBundleIdentifier".into(), "com.google.Chrome".into());
        dictionary.insert("CFBundleExecutable".into(), "Google Chrome".into());
        dictionary.insert("CFBundleShortVersionString".into(), version.into());
        let metadata_path = path.parent().unwrap().parent().unwrap().join("Info.plist");
        let value = plist::Value::Dictionary(dictionary);
        if binary_plist {
            value.to_file_binary(metadata_path).unwrap();
        } else {
            value.to_file_xml(metadata_path).unwrap();
        }
        (directory, path)
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_metadata_admits_xml_and_binary_plists_without_launching() {
        for binary_plist in [false, true] {
            let (_directory, path) = bundle_metadata_fixture("155.0.8059.39", binary_plist);
            assert_eq!(product(&path).as_deref(), Some("Chrome/155.0.8059.39"));
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_metadata_rejects_invalid_or_old_versions_and_untrusted_bundle_identity() {
        use std::os::unix::fs::PermissionsExt;
        for version in [
            "119.0.1.0",
            "155.0.1",
            "155.0.1.0.0",
            "155.0.1.bad",
            "155.0.65536.0",
            "155..1.0",
        ] {
            let (_directory, path) = bundle_metadata_fixture(version, false);
            assert!(product(&path).is_none(), "{version}");
        }
        let (_directory, path) = bundle_metadata_fixture("155.0.8059.39", true);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(product(&path).is_none());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let metadata_path = path.parent().unwrap().parent().unwrap().join("Info.plist");
        let mut value = plist::Value::from_file(&metadata_path).unwrap();
        value
            .as_dictionary_mut()
            .unwrap()
            .insert("CFBundleIdentifier".into(), "com.example.Other".into());
        value.to_file_binary(&metadata_path).unwrap();
        assert!(product(&path).is_none());
        std::fs::write(&metadata_path, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(product(&path).is_none());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_metadata_rejects_fifo_and_external_plist_symlinks_before_opening() {
        use std::os::unix::fs::symlink;
        let (_directory, path) = bundle_metadata_fixture("155.0.8059.39", false);
        let metadata_path = path.parent().unwrap().parent().unwrap().join("Info.plist");
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::copy(&metadata_path, outside.path()).unwrap();
        std::fs::remove_file(&metadata_path).unwrap();
        symlink(outside.path(), &metadata_path).unwrap();
        assert!(product(&path).is_none());
        std::fs::remove_file(&metadata_path).unwrap();
        assert!(
            std::process::Command::new("/usr/bin/mkfifo")
                .arg(&metadata_path)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            product(&path).is_none(),
            "a FIFO must never be opened during startup discovery"
        );
    }
    #[tokio::test]
    #[ignore = "requires an installed Chrome; starts one isolated local protocol probe"]
    async fn installed_release_metadata_matches_live_product() {
        let mut host = nomifun_app::DesktopHostServices::default();
        prepare(&mut host).await;
        let provider = host.local_web_search.expect("installed Chrome admission");
        let expected = provider.binding().browser_product.clone();
        let mut admitted = None;
        for path in candidates() {
            if product(&path).as_deref() == Some(expected.as_str())
                && nomi_browser_engine::headless_page::binary_digest(path.clone())
                    .await
                    .is_ok_and(|digest| digest == provider.binding().browser_binary_digest)
            {
                admitted = Some(path);
                break;
            }
        }
        let path = admitted.expect("the release admitted by prepare");
        let actual = nomi_browser_engine::headless_page::probe_runtime(path)
            .await
            .unwrap();
        assert_eq!(expected, actual);
        assert_eq!(provider.binding().browser_product, actual);
    }
    #[tokio::test]
    #[ignore = "requires installed Chrome and public network access"]
    async fn discovered_release_searches_public_sources() {
        let mut host = nomifun_app::DesktopHostServices::default();
        prepare(&mut host).await;
        let provider = host.local_web_search.expect("installed Chrome supply");
        let results = provider
            .search(
                "Tauri desktop browser documentation",
                3,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.citation_id.starts_with("nomi-local-search-"))
        );
    }
    #[tokio::test]
    #[ignore = "requires installed Chrome; mismatched product must not reach the web"]
    async fn supplied_product_mismatch_is_rejected_before_search() {
        let (path, _) = candidates()
            .into_iter()
            .find_map(|path| product(&path).map(|version| (path, version)))
            .unwrap();
        let mut host = nomifun_app::DesktopHostServices::default();
        host.set_browser_release(path, "Chrome/120.0.0.0".into())
            .await
            .unwrap();
        match host
            .local_web_search
            .unwrap()
            .search(
                "must not be sent",
                1,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
        {
            Err(error) => assert_eq!(error.code(), "WEB_RESEARCH_LOCAL_BINDING_CHANGED"),
            Ok(_) => panic!("mismatched installed release was used"),
        }
    }
}
