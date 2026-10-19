//! Effective WebView2 environment verification before any external navigation.
//! Native paths and arbitrary override values never leave this module.

use nomifun_browser_platform::runtime::{
    BrowserIdentityReport, BrowserProfilePersistence, BrowserRuntimeInfo, WorkspaceError,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, PathBuf, Prefix},
    sync::OnceLock,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment7, ICoreWebView2Settings2, ICoreWebView2_13,
};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, HANDLE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetFileInformationByHandle,
        },
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_ROUTINE_FLAGS, RRF_NOEXPAND,
            RRF_RT_ANY, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
            RRF_SUBKEY_WOW6464KEY, RegGetValueW,
        },
        UI::Shell::GetCurrentProcessExplicitAppUserModelID,
    },
    core::{BOOL, HSTRING, Interface, PWSTR},
};

const ENVIRONMENT_OVERRIDES: &[&str] = &[
    "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
    "WEBVIEW2_USER_DATA_FOLDER",
    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
    "WEBVIEW2_CHANNEL_SEARCH_KIND",
    "WEBVIEW2_RELEASE_CHANNELS",
    "WEBVIEW2_RELEASE_CHANNEL_PREFERENCE",
];
const POLICY_OVERRIDES: &[&str] = &[
    "BrowserExecutableFolder",
    "UserDataFolder",
    "AdditionalBrowserArguments",
    "ChannelSearchKind",
    "ReleaseChannels",
    "ReleaseChannelPreference",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DirectoryIdentity {
    volume: u32,
    file: u64,
}

fn ordinary_native_directory(path: PathBuf) -> Result<PathBuf, WorkspaceError> {
    // `dunce` deliberately retains verbatim paths when removing their prefix
    // would change DOS filename semantics, and does not simplify UNC shares.
    // Do not blindly remove a prefix or silently choose a different directory.
    if !path.is_absolute()
        || !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_)))
        || path.to_str().is_none_or(|text| text.contains('\0'))
    {
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    Ok(path)
}

/// Prepare only the path spelling passed to a new WebView2 environment.
///
/// The selected ProfileStore path remains the identity/cleanup owner. A fresh
/// Session directory is created there before canonicalization, then both
/// spellings must resolve to the same directory object. Ordinary, popup-shared
/// and maintenance views must keep verifying that original owner after native
/// creation; this helper cannot prove what an environment override selects.
///
/// A native fixture on Runtime 155.0.4283.45 reproduced persistent HTTP Cookie
/// loss with a verbatim UDF input and persistence with its ordinary alias.
/// Therefore unresolved verbatim paths fail before creating a native view.
/// UNC, long and special-name UDF compatibility is not asserted by this fix.
pub(crate) fn prepare_native_data_directory(
    expected: &Path,
) -> Result<PathBuf, WorkspaceError> {
    if !expected.is_absolute()
        || !matches!(expected.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    std::fs::create_dir_all(expected).map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
    let (_expected_file, expected_id) = directory_identity(expected)?;
    let native = ordinary_native_directory(
        dunce::canonicalize(expected).map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?,
    )?;
    let (_native_file, native_id) = directory_identity(&native)?;
    if native_id != expected_id {
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    Ok(native)
}

fn directory_identity(path: &Path) -> Result<(File, DirectoryIdentity), WorkspaceError> {
    if !path.is_absolute() {
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    // Keep both directory handles while comparing their object identities.
    // Delete sharing permits renames; these are not locks on path spelling.
    // File identity accepts case, extended-path and short-name aliases.
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    Ok((file, DirectoryIdentity {
        volume: info.dwVolumeSerialNumber,
        file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    }))
}

fn verify_profile(expected: &Path, actual: &Path) -> Result<DirectoryIdentity, WorkspaceError> {
    let (_expected_file, expected_id) = directory_identity(expected)?;
    let (_actual_file, actual_id) = directory_identity(actual)?;
    if expected_id != actual_id {
        // Never silently use, delete or repair the override-selected profile.
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    Ok(expected_id)
}

fn verify_effective_profile(expected: &Path, actual: &Path) -> Result<DirectoryIdentity, WorkspaceError> {
    // Inspect the native getter before normalization. An override can select
    // the same directory object through a verbatim spelling and reproduce the
    // measured Cookie loss despite the normalized SDK constructor argument.
    // Never rewrite an override or canonicalize away its effective spelling.
    let actual = ordinary_native_directory(actual.to_path_buf())?;
    let (_actual_file, actual_id) = directory_identity(&actual)?;
    let canonical = ordinary_native_directory(
        dunce::canonicalize(&actual).map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?,
    )?;
    // The raw getter handle stays open while expected/canonical handles are
    // compared. None of these checks creates or repairs the selected directory.
    let expected_id = verify_profile(expected, &canonical)?;
    if actual_id != expected_id {
        return Err(WorkspaceError::ProfileCleanupInvalid);
    }
    Ok(expected_id)
}

fn profile_token_with_salt(identity: DirectoryIdentity, salt: &[u8; 32]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nomifun.browser.effective-profile-diagnostic.v1\0");
    digest.update(salt);
    digest.update(identity.volume.to_be_bytes());
    digest.update(identity.file.to_be_bytes());
    hex::encode(digest.finalize())
}

fn profile_token(identity: DirectoryIdentity) -> Option<String> {
    // Correlates tabs in this process only. It cannot expose the path or become
    // a persistent installation/user identity in copied diagnostic reports.
    static SALT: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    SALT.get_or_init(|| {
        let mut salt = [0; 32];
        getrandom::getrandom(&mut salt).ok().map(|()| salt)
    }).as_ref().map(|salt| profile_token_with_salt(identity, salt))
}

fn version_and_channel(raw: &str) -> (Option<String>, Option<String>) {
    if raw.len() > 80 { return (None, None); }
    let mut parts = raw.split_whitespace();
    let Some(version) = parts.next() else { return (None, None); };
    let numbers: Vec<_> = version.split('.').collect();
    if numbers.len() != 4 || numbers.iter().any(|number| {
        number.is_empty() || number.len() > 10 || !number.bytes().all(|byte| byte.is_ascii_digit())
    }) { return (None, None); }
    let channel = match parts.next() {
        Some(channel) if channel.eq_ignore_ascii_case("beta") => Some("beta".to_owned()),
        Some(channel) if channel.eq_ignore_ascii_case("dev") => Some("dev".to_owned()),
        Some(channel) if channel.eq_ignore_ascii_case("canary") => Some("canary".to_owned()),
        None => None, // An absent suffix is not proof of a stable channel.
        _ => return (None, None),
    };
    if parts.next().is_some() { return (None, None); }
    (Some(version.to_owned()), channel)
}

fn registry_presence(root: HKEY, key: &str, name: &str, view: REG_ROUTINE_FLAGS) -> Option<bool> {
    let key = HSTRING::from(key);
    let name = HSTRING::from(name);
    let mut size = 0;
    // A size-only query: arbitrary argument, path and credential values are
    // neither read nor expanded into process memory or diagnostics.
    let result = unsafe {
        RegGetValueW(root, &key, &name, RRF_RT_ANY | RRF_NOEXPAND | view,
            None, None, Some(&mut size))
    };
    match result {
        ERROR_SUCCESS => Some(true),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Some(false),
        _ => None,
    }
}

fn combine_presence(values: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for value in values {
        match value {
            Some(true) => return Some(true),
            Some(false) => {},
            None => unknown = true,
        }
    }
    if unknown { None } else { Some(false) }
}

fn policy_overrides_present() -> Option<bool> {
    let executable = std::env::current_exe().ok()?;
    let application = executable.file_name()?.to_str()?;
    let mut applications = vec![application.to_owned()];
    if let Ok(raw) = unsafe { GetCurrentProcessExplicitAppUserModelID() } {
        let model_id = super::event_string(raw, 512).filter(|name| !name.is_empty())?;
        if !applications.contains(&model_id) { applications.push(model_id); }
    }
    let mut values = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY] {
            for option in POLICY_OVERRIDES {
                let key = format!("Software\\Policies\\Microsoft\\Edge\\WebView2\\{option}");
                for application in &applications {
                    values.push(registry_presence(root, &key, application, view));
                }
                // WebView2 does not apply the global UserDataFolder wildcard.
                if *option != "UserDataFolder" {
                    values.push(registry_presence(root, &key, "*", view));
                }
            }
        }
    }
    combine_presence(values)
}

/// Debug fixture admission only, before either shell or managed controller
/// creation. Never read override values or change ordinary product policy.
#[cfg(debug_assertions)]
pub(crate) fn isolated_fixture_environment_proven() -> bool {
    ENVIRONMENT_OVERRIDES.iter().all(|name| std::env::var_os(name).is_none())
        && policy_overrides_present() == Some(false)
}

fn os_dword(name: &str) -> Option<u32> {
    let key = HSTRING::from("Software\\Microsoft\\Windows NT\\CurrentVersion");
    let name = HSTRING::from(name);
    let mut value = 0_u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    (unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            None, Some((&mut value as *mut u32).cast()), Some(&mut size))
    } == ERROR_SUCCESS).then_some(value)
}

fn os_build() -> Option<String> {
    let key = HSTRING::from("Software\\Microsoft\\Windows NT\\CurrentVersion");
    let name = HSTRING::from("CurrentBuildNumber");
    let mut buffer = [0_u16; 32];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    if unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None, Some(buffer.as_mut_ptr().cast()), Some(&mut size))
    } != ERROR_SUCCESS { return None; }
    let count = buffer.iter().position(|unit| *unit == 0)?;
    let value = String::from_utf16(&buffer[..count]).ok()?;
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentityUnavailable {
    NativePropertyUnavailable,
    NativePropertyReadFailed,
    InvalidNativeProperty,
}

impl IdentityUnavailable {
    fn reason(self) -> &'static str {
        match self {
            Self::NativePropertyUnavailable => "native_property_unavailable",
            Self::NativePropertyReadFailed => "native_property_read_failed",
            Self::InvalidNativeProperty => "invalid_native_property",
        }
    }
}

fn native_identity(user_agent: String) -> Result<BrowserIdentityReport, IdentityUnavailable> {
    // Native configuration is the measured identity. Never synthesize UA from
    // the runtime version, evaluate JavaScript, or silently truncate the value.
    if user_agent.is_empty() || user_agent.len() > 512
        || !user_agent.is_ascii() || user_agent.chars().any(char::is_control) {
        return Err(IdentityUnavailable::InvalidNativeProperty);
    }
    Ok(BrowserIdentityReport {
        profile_id: "windows-webview2-native-v1".into(),
        policy_revision: 1,
        engine_family: "chromium".into(),
        effective_user_agent: user_agent,
        advertised_compatibility_version: "unknown".into(),
        version_source: "native_property".into(),
    })
}

fn identity_diagnostic(
    result: Result<BrowserIdentityReport, IdentityUnavailable>,
) -> Option<BrowserIdentityReport> {
    match result {
        Ok(identity) => Some(identity),
        Err(unavailable) => {
            // This optional measurement must not prevent ordinary navigation.
            // Log a fixed reason only, never an unvalidated native UA value.
            tracing::warn!(reason = unavailable.reason(), "Native browser identity diagnostic is unavailable");
            None
        }
    }
}

/// Reads the public native UA property of this exact existing view. It neither
/// changes identity nor claims that any website accepted this browser. Unlike
/// mandatory profile verification, unavailable identity metadata is non-fatal.
pub(crate) async fn inspect_identity(
    view: &tauri::Webview,
) -> Option<BrowserIdentityReport> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let scheduled = view.with_webview(move |platform| {
        let result = (|| {
            let core = unsafe { platform.controller().CoreWebView2() }
                .map_err(|_| IdentityUnavailable::NativePropertyUnavailable)?;
            let settings = unsafe { core.Settings() }
                .map_err(|_| IdentityUnavailable::NativePropertyUnavailable)?;
            let settings = settings.cast::<ICoreWebView2Settings2>()
                .map_err(|_| IdentityUnavailable::NativePropertyUnavailable)?;
            let mut raw = PWSTR::null();
            let received = unsafe { settings.UserAgent(&mut raw) };
            let user_agent = super::event_string(raw, 512);
            received.map_err(|_| IdentityUnavailable::NativePropertyReadFailed)?;
            native_identity(user_agent.ok_or(IdentityUnavailable::InvalidNativeProperty)?)
        })();
        let _ = tx.send(result);
    });
    identity_diagnostic(match scheduled {
        Ok(()) => rx.await.unwrap_or(Err(IdentityUnavailable::NativePropertyReadFailed)),
        Err(_) => Err(IdentityUnavailable::NativePropertyUnavailable),
    })
}

/// Must run before the first target navigation, including native popup binding.
/// This inspects the existing environment only; it creates no process/request.
pub(crate) async fn inspect(
    view: &tauri::Webview,
    expected_directory: PathBuf,
    persistence: BrowserProfilePersistence,
) -> Result<BrowserRuntimeInfo, WorkspaceError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| {
            let environment = platform.environment();
            let environment7 = environment.cast::<ICoreWebView2Environment7>()
                .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
            let mut raw = PWSTR::null();
            let received = unsafe { environment7.UserDataFolder(&mut raw) };
            let directory = super::event_string(raw, 32768);
            received.map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
            let directory = directory.filter(|path| !path.is_empty())
                .ok_or(WorkspaceError::ProfileCleanupInvalid)?;
            let identity = verify_effective_profile(&expected_directory, Path::new(&directory))?;
            let core = unsafe { platform.controller().CoreWebView2() }
                .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
            let profile = unsafe { core.cast::<ICoreWebView2_13>()
                .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?.Profile() }
                .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
            let mut private = BOOL::default();
            unsafe { profile.IsInPrivateModeEnabled(&mut private) }
                .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
            if private.as_bool() != (persistence == BrowserProfilePersistence::Ephemeral) {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
            let mut raw = PWSTR::null();
            let received = unsafe { environment.BrowserVersionString(&mut raw) };
            let version = super::event_string(raw, 80);
            let (browser_version, browser_channel) = if received.is_ok() {
                version.as_deref().map(version_and_channel).unwrap_or_default()
            } else { (None, None) };
            let os_build = os_build();
            let os_version = match (os_dword("CurrentMajorVersionNumber"), os_dword("CurrentMinorVersionNumber")) {
                (Some(major), Some(minor)) => format!("{major}.{minor}"),
                _ => "unknown".to_owned(),
            };
            Ok(BrowserRuntimeInfo {
                os_version,
                os_build,
                browser_version,
                browser_channel,
                profile_persistence: Some(persistence),
                profile_identity_token: profile_token(identity),
                environment_overrides_present: Some(ENVIRONMENT_OVERRIDES.iter()
                    .any(|name| std::env::var_os(name).is_some())),
                policy_overrides_present: policy_overrides_present(),
                ..Default::default()
            })
        })();
        let _ = tx.send(result);
    }).map_err(|_| WorkspaceError::NativeUnavailable)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_udf_preparation_creates_only_the_selected_fresh_profile_directory() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let expected = root.join("browser-v4").join("agent-sessions").join("selected-session");
        assert!(!expected.exists());
        let native = prepare_native_data_directory(&expected).unwrap();
        assert!(expected.is_dir());
        assert!(matches!(native.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_))));
        assert_eq!(verify_profile(&expected, &native), verify_profile(&expected, &expected));
        assert!(!root.join("other-session").exists());
    }

    #[test]
    fn native_udf_preparation_preserves_existing_data_and_directory_identity() {
        let root = tempfile::tempdir().unwrap();
        let expected = root.path().join("selected-session");
        std::fs::create_dir(&expected).unwrap();
        let sentinel = expected.join("controlled-existing-data");
        std::fs::write(&sentinel, b"retain exact profile").unwrap();
        let expected = std::fs::canonicalize(expected).unwrap();
        let first = prepare_native_data_directory(&expected).unwrap();
        let second = prepare_native_data_directory(&expected).unwrap();
        assert_eq!(first, second);
        assert_eq!(verify_profile(&expected, &first), verify_profile(&expected, &second));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"retain exact profile");
    }

    #[test]
    fn native_udf_preparation_rejects_unsupported_roots_before_filesystem_access() {
        for input in [r"relative\profile", r"C:relative\profile", r"\profile",
            r"\\server\share\profile", r"\\?\UNC\server\share\profile", r"\\.\C:\profile"] {
            assert_eq!(prepare_native_data_directory(Path::new(input)),
                Err(WorkspaceError::ProfileCleanupInvalid));
        }
    }

    #[test]
    fn native_udf_preparation_does_not_trim_extended_special_names() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let expected = root.join("controlled-trailing-dot.");
        std::fs::create_dir(&expected).unwrap();
        let sentinel = expected.join("retained-data");
        std::fs::write(&sentinel, b"do not relocate").unwrap();
        assert_eq!(prepare_native_data_directory(&expected),
            Err(WorkspaceError::ProfileCleanupInvalid));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"do not relocate");
        assert!(!root.join("controlled-trailing-dot").exists());
    }

    #[test]
    fn unresolved_native_udf_representations_are_not_prefix_stripped() {
        for input in [r"\\?\UNC\server\share\profile".to_owned(),
            r"\\?\C:\controlled-trailing-space ".to_owned(),
            format!(r"\\?\C:\{}\{}", "a".repeat(180), "b".repeat(180))] {
            let path = PathBuf::from(input);
            assert_eq!(dunce::simplified(&path), path.as_path());
            assert_eq!(ordinary_native_directory(path), Err(WorkspaceError::ProfileCleanupInvalid));
        }
    }

    #[test]
    fn effective_profile_rejects_verbatim_getter_even_when_directory_identity_matches() {
        let root = tempfile::tempdir().unwrap();
        let expected = std::fs::canonicalize(root.path()).unwrap();
        let ordinary = dunce::canonicalize(&expected).unwrap();
        assert!(verify_profile(&expected, &ordinary).is_ok());
        assert!(verify_profile(&expected, &expected).is_ok());
        let sentinel = expected.join("retained-profile-data");
        std::fs::write(&sentinel, b"preserve source").unwrap();
        let error = verify_effective_profile(&expected, &expected).unwrap_err();
        assert_eq!(error, WorkspaceError::ProfileCleanupInvalid);
        assert!(!error.to_string().contains(&expected.to_string_lossy().to_string()));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve source");
    }

    #[test]
    fn effective_profile_allows_matching_ordinary_getter_and_rejects_other_session() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first-session");
        let second = root.path().join("second-session");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let expected = std::fs::canonicalize(&first).unwrap();
        let first = dunce::canonicalize(&first).unwrap();
        let second = dunce::canonicalize(&second).unwrap();
        assert_eq!(verify_effective_profile(&expected, &first), verify_profile(&expected, &first));
        assert_eq!(verify_effective_profile(&expected, &second), Err(WorkspaceError::ProfileCleanupInvalid));
    }

    #[test]
    fn effective_profile_verification_rejects_other_profile_without_reporting_paths() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first-session");
        let second = root.path().join("second-session");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        assert!(verify_profile(&first, &first).is_ok());
        assert_eq!(verify_profile(&first, &second), Err(WorkspaceError::ProfileCleanupInvalid));
        let error = verify_profile(&first, &second).unwrap_err().to_string();
        assert!(!error.contains("first-session") && !error.contains("second-session"));
    }

    #[test]
    fn effective_profile_verification_compares_directory_objects_not_path_spelling() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("ProfileWithCase");
        std::fs::create_dir(&first).unwrap();
        let case_alias = root.path().join("profilewithcase");
        // Case-sensitive NTFS directories need not resolve the alias.
        if case_alias.exists() { assert!(verify_profile(&first, &case_alias).is_ok()); }
        assert!(verify_profile(&first, &std::fs::canonicalize(&first).unwrap()).is_ok());
        assert_eq!(verify_profile(Path::new("relative"), &first), Err(WorkspaceError::ProfileCleanupInvalid));
    }

    #[test]
    fn profile_tokens_are_process_salted_and_contain_no_profile_path() {
        let first = DirectoryIdentity { volume: 1, file: 2 };
        let second = DirectoryIdentity { volume: 1, file: 3 };
        let token = profile_token_with_salt(first, &[7; 32]);
        assert_eq!(token, profile_token_with_salt(first, &[7; 32]));
        assert_ne!(token, profile_token_with_salt(second, &[7; 32]));
        assert_ne!(token, profile_token_with_salt(first, &[8; 32]));
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn native_version_reports_only_known_numeric_version_and_channel() {
        assert_eq!(version_and_channel("134.0.3124.51"), (Some("134.0.3124.51".into()), None));
        assert_eq!(version_and_channel("134.0.3124.51 dev"), (Some("134.0.3124.51".into()), Some("dev".into())));
        assert_eq!(version_and_channel("134.0.3124.51 Beta"), (Some("134.0.3124.51".into()), Some("beta".into())));
        for raw in ["", "134.0.1", "134.0.bad.1", "134.0.1.2 --proxy-server=secret", "134.0.1.2 dev extra"] {
            assert_eq!(version_and_channel(raw), (None, None));
        }
    }

    #[test]
    fn native_identity_preserves_the_measured_property_without_guessing_a_version() {
        let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/134.0.0.0 Safari/537.36 Edg/134.0.0.0";
        let identity = native_identity(user_agent.into()).unwrap();
        assert_eq!(identity.effective_user_agent, user_agent);
        assert_eq!(identity.engine_family, "chromium");
        assert_eq!(identity.version_source, "native_property");
        assert_eq!(identity.advertised_compatibility_version, "unknown");
        for value in [String::new(), "bad\r\nheader".into(), "bad\0header".into(), "x".repeat(513)] {
            assert_eq!(native_identity(value), Err(IdentityUnavailable::InvalidNativeProperty));
        }
    }

    #[test]
    fn optional_identity_diagnostics_preserve_measurements_and_degrade_every_failure() {
        let measured = native_identity("controlled-native-user-agent".into()).unwrap();
        assert_eq!(identity_diagnostic(Ok(measured.clone())), Some(measured));
        for unavailable in [IdentityUnavailable::NativePropertyUnavailable,
            IdentityUnavailable::NativePropertyReadFailed, IdentityUnavailable::InvalidNativeProperty] {
            assert_eq!(identity_diagnostic(Err(unavailable)), None);
        }
        assert_eq!(identity_diagnostic(native_identity("x".repeat(513))), None);
    }

    #[test]
    fn unreadable_policy_is_unknown_unless_a_matching_override_is_proven() {
        assert_eq!(combine_presence([Some(false), Some(false)]), Some(false));
        assert_eq!(combine_presence([None, Some(false)]), None);
        assert_eq!(combine_presence([None, Some(true)]), Some(true));
    }
}
