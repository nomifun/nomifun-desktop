//! Windows Headless browser supply for search and Knowledge rendering. Inspect metadata only; never
//! launch a browser, inspect login state, or query the web during app startup.
use std::path::{Path, PathBuf};
use windows::{
    Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    },
    core::PCWSTR,
};

fn candidates() -> Vec<PathBuf> {
    candidate_paths(
        ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from),
    )
}
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

pub(crate) async fn prepare(host: &mut nomifun_app::DesktopHostServices) {
    if host.local_web_search.is_some() && host.headless_render.is_some() {
        return;
    }
    let installed = tokio::task::spawn_blocking(|| {
        candidates()
            .into_iter()
            .find_map(|path| product(&path).map(|version| (path, version)))
    })
    .await;
    let Ok(Some((path, version))) = installed else {
        return;
    };
    if host.set_browser_release(path, version).await.is_err() {
        // No process was created, so this optional dependency failure has no
        // browser cleanup authority to lose or transfer to desktop startup.
        tracing::warn!("Headless browser capabilities are unavailable: installed Chrome metadata could not be admitted");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_paths_are_absolute_deduplicated_and_do_not_search_path() {
        let root = PathBuf::from("C:/Known Install");
        assert_eq!(
            candidate_paths([PathBuf::from("relative"), root.clone(), root.clone()]),
            vec![root.join("Google/Chrome/Application/chrome.exe")]
        );
    }
    #[test]
    fn missing_or_non_pe_files_are_not_browser_releases() {
        let directory = tempfile::tempdir().unwrap();
        assert!(product(&directory.path().join("missing.exe")).is_none());
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(product(file.path()).is_none());
    }
    #[tokio::test]
    #[ignore = "requires an installed Windows Chrome; starts one isolated local protocol probe"]
    async fn installed_release_metadata_matches_live_product() {
        let mut host = nomifun_app::DesktopHostServices::default();
        prepare(&mut host).await;
        let provider = host.local_web_search.expect("installed Chrome admission");
        let (path, expected) = candidates()
            .into_iter()
            .find_map(|path| product(&path).map(|version| (path, version)))
            .unwrap();
        let actual = nomi_browser_engine::headless_page::probe_runtime(path)
            .await
            .unwrap();
        assert_eq!(expected, actual);
        assert_eq!(provider.binding().browser_product, actual);
    }
    #[tokio::test]
    #[ignore="requires installed Windows Chrome and public network access"]
    async fn discovered_release_searches_public_sources() {
        let mut host=nomifun_app::DesktopHostServices::default();
        prepare(&mut host).await;
        let provider=host.local_web_search.expect("installed Chrome supply");
        let results=provider.search("Tauri WebView2 documentation",3,tokio_util::sync::CancellationToken::new()).await.unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|result|result.citation_id.starts_with("nomi-local-search-")));
    }
    #[tokio::test]
    #[ignore="requires installed Windows Chrome; mismatched product must not reach the web"]
    async fn supplied_product_mismatch_is_rejected_before_search() {
        let (path,_) = candidates().into_iter().find_map(|path| product(&path).map(|version|(path,version))).unwrap();
        let mut host=nomifun_app::DesktopHostServices::default();
        host.set_browser_release(path,"Chrome/120.0.0.0".into()).await.unwrap();
        match host.local_web_search.unwrap().search("must not be sent",1,tokio_util::sync::CancellationToken::new()).await {
            Err(error)=>assert_eq!(error.code(),"NOMI_LOCAL_WEBSEARCH_BINDING_CHANGED"),
            Ok(_)=>panic!("mismatched installed release was used"),
        }
    }
}
