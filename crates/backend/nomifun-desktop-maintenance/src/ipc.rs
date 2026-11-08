//! Private, bounded IPC used only for short-lived application maintenance.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{ffi::OsStr, fs::{self, File, OpenOptions}, io::{BufRead, BufReader, Read, Write}, net::TcpStream, path::{Component, Path, PathBuf}, sync::OnceLock};

const IPC_LIMIT: u64 = 32768;
#[derive(Clone, Copy)]
enum RuntimeKind { Application, Device }

/// Process-local selection, never an environment override inherited by an
/// installer helper. Release/helper callers leave this at PublishedDefaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeInitialization {
    PublishedDefaults,
    Development { profile: PathBuf, application: PathBuf, device: PathBuf },
}
static RUNTIME_CONFIGURATION: OnceLock<RuntimeInitialization> = OnceLock::new();

fn absolute_profile(profile: &Path, cwd: &Path) -> Result<PathBuf> {
    let absolute = if profile.is_absolute() { profile.to_path_buf() } else { cwd.join(profile) };
    ensure!(absolute.is_absolute(), "development data profile must resolve to an absolute path");
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(normalized.components().next_back(), Some(Component::Normal(_))) { normalized.pop(); }
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    ensure!(normalized.file_name().is_some(), "development data profile cannot be a filesystem root");
    #[cfg(windows)]
    let normalized = PathBuf::from(normalized.to_str().context("development profile identity must be valid Unicode")?.to_lowercase());
    Ok(normalized)
}

fn development_configuration(channel: Option<&OsStr>, data_profile: Option<&OsStr>, base: &Path, cwd: &Path) -> Result<RuntimeInitialization> {
    if channel != Some(OsStr::new("dev")) { return Ok(RuntimeInitialization::PublishedDefaults); }
    let profile = data_profile.filter(|value| !value.is_empty() && !value.to_string_lossy().trim().is_empty())
        .context("NOMI_CHANNEL=dev requires NOMIFUN_DATA_DIR; use bun run dev or provide its explicit data profile")?;
    let profile = absolute_profile(Path::new(profile), cwd)?;
    ensure!(base.is_absolute(), "development IPC base must be absolute");
    let fingerprint = format!("{:x}", Sha256::digest(profile.as_os_str().as_encoded_bytes()));
    let namespace = base.join("NomiFun-dev-runtime").join(fingerprint);
    #[cfg(windows)]
    let overlaps = PathBuf::from(namespace.to_string_lossy().to_lowercase()).starts_with(PathBuf::from(profile.to_string_lossy().to_lowercase()));
    #[cfg(not(windows))]
    let overlaps = namespace.starts_with(&profile);
    ensure!(!overlaps, "development IPC namespace must remain outside the data profile");
    Ok(RuntimeInitialization::Development { profile, application: namespace.join("application"), device: namespace.join("device") })
}

fn initialize_configuration(slot: &OnceLock<RuntimeInitialization>, requested: RuntimeInitialization) -> Result<RuntimeInitialization> {
    let selected = slot.get_or_init(|| requested.clone());
    ensure!(selected == &requested, "maintenance runtime scope is already initialized and cannot be changed");
    Ok(selected.clone())
}

/// Call only from the development application entry point, before any IPC or
/// headless maintenance operation. Selection is idempotent for the same profile
/// and immutable after first use. This does not create files or change env vars.
pub fn configure_development_runtime() -> Result<RuntimeInitialization> {
    let channel = std::env::var_os("NOMI_CHANNEL");
    let data_profile = std::env::var_os("NOMIFUN_DATA_DIR");
    let requested = if channel.as_deref() == Some(OsStr::new("dev")) {
        let cwd = std::env::current_dir().context("resolve development launch directory")?;
        development_configuration(channel.as_deref(), data_profile.as_deref(), &runtime_base()?, &cwd)?
    } else { RuntimeInitialization::PublishedDefaults };
    initialize_configuration(&RUNTIME_CONFIGURATION, requested)
}

pub fn is_development_runtime() -> bool {
    matches!(RUNTIME_CONFIGURATION.get(), Some(RuntimeInitialization::Development { .. }))
}

fn configured_directory(configuration: &RuntimeInitialization, kind: RuntimeKind) -> Option<PathBuf> {
    match (configuration, kind) {
        (RuntimeInitialization::Development { application, .. }, RuntimeKind::Application) => Some(application.clone()),
        (RuntimeInitialization::Development { device, .. }, RuntimeKind::Device) => Some(device.clone()),
        _ => None,
    }
}

fn runtime_path(base: &Path, platform: &str, kind: RuntimeKind) -> PathBuf {
    let leaf = match (platform, kind) {
        ("macos", RuntimeKind::Application) => "NomiFun/app-maintenance",
        ("macos", RuntimeKind::Device) => "NomiFun/external-keyboard",
        // Released Windows installers and Linux package hooks coordinate here.
        // Preserve their existing endpoint/lock identity, not a second ledger.
        _ => "NomiFun/keyboard-service",
    };
    base.join(leaf)
}

/// Current application maintenance. On macOS no historical keyboard state is
/// created, queried or migrated; old state lives in a separate directory.
pub fn runtime_dir() -> Result<PathBuf> { runtime_directory(RuntimeKind::Application) }

/// Optional serial admission, separate from modern macOS application lifetime.
pub fn device_runtime_dir() -> Result<PathBuf> { runtime_directory(RuntimeKind::Device) }

fn runtime_base() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA unavailable")?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME unavailable")?
        .join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = if let Some(root) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(root)
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME unavailable")?
            .join(".local/state")
    };
    Ok(base)
}

fn runtime_directory(kind: RuntimeKind) -> Result<PathBuf> {
    // A standalone helper never invokes the development initializer, even when
    // its parent shell happens to carry development environment variables.
    let configuration = RUNTIME_CONFIGURATION.get_or_init(|| RuntimeInitialization::PublishedDefaults);
    let dir = match configured_directory(configuration, kind) {
        Some(path) => path,
        None => runtime_path(&runtime_base()?, std::env::consts::OS, kind),
    };
    fs::create_dir_all(&dir)?;
    ensure!(
        !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "runtime directory is a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            fs::metadata(&dir)?.uid() == unsafe { libc_getuid() },
            "runtime directory owner mismatch"
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}
#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
pub(crate) fn private_create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
pub(crate) fn send_line<T: Serialize>(stream: &mut TcpStream, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() < IPC_LIMIT as usize, "IPC message too large");
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}
pub(crate) fn read_line<T: for<'de> Deserialize<'de>>(
    reader: &mut BufReader<TcpStream>,
) -> Result<T> {
    let mut line = String::new();
    let n = reader.take(IPC_LIMIT + 1).read_line(&mut line)?;
    ensure!(
        n > 0 && n as u64 <= IPC_LIMIT && line.ends_with('\n'),
        "IPC closed or message limit exceeded"
    );
    Ok(serde_json::from_str(&line)?)
}
pub(crate) fn token_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn development_paths() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join("nomifun-ipc-path-plan-only");
        let cwd = base.join("checkout");
        (base, cwd)
    }

    #[test]
    fn development_generated_profile_owns_distinct_ipc_outside_its_dataset() {
        let (base, cwd) = development_paths();
        let profile = base.join("NomiFun-dev-schema-fixture");
        let plan = development_configuration(Some(OsStr::new("dev")), Some(profile.as_os_str()), &base, &cwd).unwrap();
        let RuntimeInitialization::Development { profile: selected, application, device } = plan else { panic!("development scope") };
        assert!(selected.is_absolute());
        assert_eq!(selected.file_name().unwrap().to_string_lossy().to_lowercase(), "nomifun-dev-schema-fixture");
        assert_ne!(application, device);
        assert!(application.starts_with(base.join("NomiFun-dev-runtime")));
        assert!(!application.starts_with(&profile));
        assert!(!device.starts_with(&profile));
        assert_eq!(application.parent(), device.parent());
        assert_eq!(application.parent().unwrap().file_name().unwrap().len(), 64);
    }

    #[test]
    fn development_explicit_relative_profile_resolves_against_launch_cwd() {
        let (base, cwd) = development_paths();
        let relative = Path::new("profiles").join("..").join("voice-dev");
        let plan = development_configuration(Some(OsStr::new("dev")), Some(relative.as_os_str()), &base, &cwd).unwrap();
        let expected = cwd.join("voice-dev");
        let absolute = development_configuration(Some(OsStr::new("dev")), Some(expected.as_os_str()), &base, Path::new("")).unwrap();
        assert_eq!(plan, absolute);
    }

    #[cfg(windows)]
    #[test]
    fn development_windows_profile_case_does_not_split_process_identity() {
        let (base, cwd) = development_paths();
        let upper = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("C:\\Data\\NomiFun-Dev")), &base, &cwd).unwrap();
        let lower = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("c:\\data\\nomifun-dev")), &base, &cwd).unwrap();
        assert_eq!(upper, lower);
        let slot = OnceLock::new();
        initialize_configuration(&slot, upper.clone()).unwrap();
        assert_eq!(initialize_configuration(&slot, lower).unwrap(), upper);
    }

    #[test]
    fn development_explicit_profiles_cannot_share_endpoint_or_device_locks() {
        let (base, cwd) = development_paths();
        let one = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("profiles/one")), &base, &cwd).unwrap();
        let two = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("profiles/two")), &base, &cwd).unwrap();
        for role in [RuntimeKind::Application, RuntimeKind::Device] {
            assert_ne!(configured_directory(&one, role), configured_directory(&two, role));
            for platform in ["windows", "macos", "linux"] {
                assert_ne!(configured_directory(&one, role).unwrap(), runtime_path(&base, platform, role));
            }
        }
    }

    #[test]
    fn development_scope_is_immutable_and_same_profile_initialization_is_idempotent() {
        let (base, cwd) = development_paths();
        let one = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("one")), &base, &cwd).unwrap();
        let two = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("two")), &base, &cwd).unwrap();
        let slot = OnceLock::new();
        assert_eq!(initialize_configuration(&slot, one.clone()).unwrap(), one);
        assert_eq!(initialize_configuration(&slot, one.clone()).unwrap(), one);
        assert!(initialize_configuration(&slot, two).is_err());
        assert!(initialize_configuration(&slot, RuntimeInitialization::PublishedDefaults).is_err());
        assert_eq!(slot.get(), Some(&one));
    }

    #[test]
    fn development_selection_after_published_ipc_use_is_rejected() {
        let (base, cwd) = development_paths();
        let slot = OnceLock::new();
        slot.get_or_init(|| RuntimeInitialization::PublishedDefaults);
        let plan = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("profile")), &base, &cwd).unwrap();
        assert!(initialize_configuration(&slot, plan).is_err());
        assert_eq!(slot.get(), Some(&RuntimeInitialization::PublishedDefaults));
    }

    #[test]
    fn development_channel_never_falls_back_to_production_when_profile_is_missing() {
        let (base, cwd) = development_paths();
        for profile in [None, Some(OsStr::new("")), Some(OsStr::new("  "))] {
            assert!(development_configuration(Some(OsStr::new("dev")), profile, &base, &cwd).is_err());
        }
        assert_eq!(development_configuration(None, None, &base, &cwd).unwrap(), RuntimeInitialization::PublishedDefaults);
        assert_eq!(development_configuration(Some(OsStr::new("stable")), Some(OsStr::new("ignored")), &base, &cwd).unwrap(), RuntimeInitialization::PublishedDefaults);
    }

    #[test]
    fn development_runtime_cannot_live_inside_a_relocated_data_profile() {
        let (base, cwd) = development_paths();
        assert!(development_configuration(Some(OsStr::new("dev")), Some(base.as_os_str()), &base, &cwd).is_err());
        let forbidden_profile = base.join("NomiFun-dev-runtime");
        assert!(development_configuration(Some(OsStr::new("dev")), Some(forbidden_profile.as_os_str()), &base, &cwd).is_err());
    }

    #[test]
    fn standalone_helper_without_initializer_keeps_published_defaults_despite_a_dev_plan() {
        let (base, cwd) = development_paths();
        let _inherited_dev_plan = development_configuration(Some(OsStr::new("dev")), Some(OsStr::new("profile")), &base, &cwd).unwrap();
        let helper = OnceLock::new();
        let selected = helper.get_or_init(|| RuntimeInitialization::PublishedDefaults);
        for role in [RuntimeKind::Application, RuntimeKind::Device] {
            assert!(configured_directory(selected, role).is_none());
            for platform in ["windows", "linux"] {
                assert_eq!(runtime_path(&base, platform, role), base.join("NomiFun/keyboard-service"));
            }
        }
    }

    #[test]
    fn macos_runtime_roles_are_distinct_from_legacy_and_published_other_platforms_stay_stable() {
        let base = Path::new("/private-fixture");
        assert_eq!(runtime_path(base, "macos", RuntimeKind::Application), base.join("NomiFun/app-maintenance"));
        assert_eq!(runtime_path(base, "macos", RuntimeKind::Device), base.join("NomiFun/external-keyboard"));
        for platform in ["windows", "linux"] {
            for kind in [RuntimeKind::Application, RuntimeKind::Device] {
                assert_eq!(runtime_path(base, platform, kind), base.join("NomiFun/keyboard-service"));
            }
        }
    }

    #[test]
    fn macos_application_admission_and_stop_ignore_busy_malformed_legacy_runtime() {
        use fs2::FileExt;
        use crate::maintenance::{self, MaintenanceGuard, ResumeIntent};
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("NomiFun/keyboard-service");
        let modern = runtime_path(root.path(), "macos", RuntimeKind::Application);
        fs::create_dir_all(&legacy).unwrap(); fs::create_dir_all(&modern).unwrap();
        let mut old_locks = Vec::new();
        for name in ["maintenance.lock", "desktop.lock", "owner.lock"] {
            let lock = File::options().create(true).truncate(false).read(true).write(true).open(legacy.join(name)).unwrap();
            FileExt::try_lock_exclusive(&lock).unwrap(); old_locks.push(lock);
        }
        for name in ["desktop.json", "owner.json"] { fs::write(legacy.join(name), b"invalid old runtime data").unwrap(); }
        let admission = maintenance::admit_startup(&modern).unwrap();
        drop(admission);
        let mut intent = ResumeIntent::default();
        let guard = MaintenanceGuard::prepare_application_with_intent_checked_in(
            modern.clone(), std::time::Duration::from_millis(50), &mut intent, |_| Ok(()),
        ).unwrap();
        assert_eq!(intent, ResumeIntent::default());
        assert!(!modern.join("owner.lock").exists());
        assert!(!modern.join("owner.json").exists());
        for name in ["desktop.json", "owner.json"] { assert_eq!(fs::read(legacy.join(name)).unwrap(), b"invalid old runtime data"); }
        drop(guard); drop(old_locks);
    }
}
