//! Explicit debug-only admission for disposable macOS lifecycle validation.
//! Ordinary startup never selects this path or skips shared Keyboard ownership.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

struct Fixture {
    root: PathBuf,
    data: PathBuf,
    work: PathBuf,
    token: String,
}

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

fn plain_directory(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(metadata.is_dir(), "fixture directory must not be a link or file");
    // SAFETY: geteuid reads the current process identity and has no side effects.
    anyhow::ensure!(metadata.uid() == unsafe { libc::geteuid() }, "fixture directory must belong to the current user");
    Ok(())
}

fn validate_fixture(root: &Path, token: &str, data: &Path, work: &Path, temporary: &Path) -> anyhow::Result<Fixture> {
    anyhow::ensure!(root.is_absolute() && data.is_absolute() && work.is_absolute(),
        "fixture root/data/work paths must be absolute");
    let parsed = uuid::Uuid::parse_str(token)?;
    anyhow::ensure!(parsed.get_version_num() == 7 && parsed.to_string() == token, "fixture token must be canonical UUIDv7");
    plain_directory(root)?;
    let root = fs::canonicalize(root)?;
    let temporary = fs::canonicalize(temporary)?;
    let expected_name = format!("nomifun-lifecycle-{token}");
    anyhow::ensure!(root.parent() == Some(temporary.as_path())
        && root.file_name().and_then(|name| name.to_str()) == Some(expected_name.as_str()),
        "fixture must be the exact random direct Temp child");
    let sentinel = root.join("fixture-sentinel");
    let metadata = fs::symlink_metadata(&sentinel)?;
    // The marker is read only after its exact disposable directory is proven.
    anyhow::ensure!(metadata.is_file() && metadata.nlink() == 1
        && metadata.len() == token.len() as u64, "fixture sentinel must be an exact regular file");
    anyhow::ensure!(fs::read_to_string(&sentinel)? == token, "fixture sentinel mismatch");
    plain_directory(data)?;
    plain_directory(work)?;
    let data = fs::canonicalize(data)?;
    let work = fs::canonicalize(work)?;
    anyhow::ensure!(data == root.join("data") && work == root.join("work"),
        "fixture data/work must be exact direct children of its disposable root");
    Ok(Fixture { root, data, work, token: token.to_owned() })
}

fn validate_compiled_work_dir(compiled: Option<&str>, work: &Path) -> anyhow::Result<()> {
    if let Some(compiled) = compiled.filter(|value| !value.is_empty()) {
        anyhow::ensure!(Path::new(compiled).is_absolute()
            && fs::canonicalize(compiled)? == work,
            "compiled GUI work directory conflicts with the disposable fixture");
    }
    Ok(())
}

/// Runs after descriptor admission, before Keyboard, PATH, storage, or workers.
/// Invalid opt-in fails closed without touching shared registration or data.
pub(super) fn prepare_launch() -> anyhow::Result<()> {
    let root = std::env::var_os("NOMIFUN_LIFECYCLE_FIXTURE_DIR");
    let token = std::env::var_os("NOMIFUN_LIFECYCLE_FIXTURE_TOKEN");
    let (root, token) = match (root, token) {
        (None, None) => return Ok(()),
        (Some(root), Some(token)) => (PathBuf::from(root), token.into_string()
            .map_err(|_| anyhow::anyhow!("fixture token must be UTF-8"))?),
        _ => anyhow::bail!("fixture root and token must be provided together"),
    };
    let mode = std::env::var("NOMIFUN_LIFECYCLE_FIXTURE_MODE").unwrap_or_else(|_| "appkit".into());
    anyhow::ensure!(matches!(mode.as_str(), "appkit" | "sigint" | "sigterm" | "parent-death" | "cmd-q"),
        "unsupported macOS fixture mode");
    let data = PathBuf::from(std::env::var_os("NOMIFUN_DATA_DIR")
        .ok_or_else(|| anyhow::anyhow!("fixture data root required"))?);
    let work = PathBuf::from(std::env::var_os("NOMIFUN_WORK_DIR")
        .ok_or_else(|| anyhow::anyhow!("fixture work root required"))?);
    let fixture = validate_fixture(&root, &token, &data, &work, &std::env::temp_dir())?;
    validate_compiled_work_dir(option_env!("NOMIFUN_BROWSER_GUI_WORK_DIR"), &fixture.work)?;
    let receipt = serde_json::json!({
        "host_pid": std::process::id(), "token": fixture.token,
        "data": fixture.data, "work": fixture.work, "shared_keyboard_started": false,
    });
    // Never overwrite another run's receipt. A partial/failed write cannot be
    // admitted by the external driver, which validates this complete JSON.
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(fixture.root.join("macos-lifecycle-fixture.json"))?;
    file.write_all(&serde_json::to_vec(&receipt)?)?;
    file.sync_all()?;
    FIXTURE.set(fixture).map_err(|_| anyhow::anyhow!("fixture launch already prepared"))?;
    Ok(())
}

pub(super) fn active() -> bool { FIXTURE.get().is_some() }

#[cfg(test)]
mod tests {
    use super::*;

    struct OwnedFixture {
        temporary: tempfile::TempDir,
        root: PathBuf,
        token: String,
    }

    impl OwnedFixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let token = uuid::Uuid::now_v7().to_string();
            let root = temporary.path().join(format!("nomifun-lifecycle-{token}"));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("data")).unwrap();
            fs::create_dir(root.join("work")).unwrap();
            fs::write(root.join("fixture-sentinel"), &token).unwrap();
            Self { temporary, root, token }
        }

        fn validate(&self) -> anyhow::Result<Fixture> {
            validate_fixture(&self.root, &self.token, &self.root.join("data"),
                &self.root.join("work"), self.temporary.path())
        }
    }

    #[test]
    fn exact_disposable_identity_is_accepted_without_shared_state() {
        let owned = OwnedFixture::new();
        let fixture = owned.validate().unwrap();
        assert_eq!(fixture.root, fs::canonicalize(&owned.root).unwrap());
        assert_eq!(fixture.data, fixture.root.join("data"));
        assert_eq!(fixture.work, fixture.root.join("work"));
        assert!(!fixture.root.join("macos-lifecycle-fixture.json").exists());
        assert!(!active());
    }

    #[test]
    fn wrong_token_marker_and_temp_identity_are_rejected() {
        let owned = OwnedFixture::new();
        let other = uuid::Uuid::now_v7().to_string();
        assert!(validate_fixture(&owned.root, &other, &owned.root.join("data"),
            &owned.root.join("work"), owned.temporary.path()).is_err());
        fs::write(owned.root.join("fixture-sentinel"), format!("{}\n", owned.token)).unwrap();
        assert!(owned.validate().is_err());
        fs::write(owned.root.join("fixture-sentinel"), &owned.token).unwrap();
        assert!(validate_fixture(&owned.root, &owned.token, &owned.root.join("data"),
            &owned.root.join("work"), &owned.root).is_err());
        assert!(validate_fixture(&owned.root, "00000000-0000-4000-8000-000000000000", &owned.root.join("data"),
            &owned.root.join("work"), owned.temporary.path()).is_err());
    }

    #[test]
    fn linked_marker_root_and_data_never_gain_fixture_admission() {
        use std::os::unix::fs::symlink;
        let owned = OwnedFixture::new();
        let sentinel = owned.root.join("fixture-sentinel");
        let source = owned.temporary.path().join("external-marker");
        fs::write(&source, &owned.token).unwrap();
        fs::remove_file(&sentinel).unwrap();
        symlink(&source, &sentinel).unwrap();
        assert!(owned.validate().is_err());
        fs::remove_file(&sentinel).unwrap();
        fs::hard_link(&source, &sentinel).unwrap();
        assert!(owned.validate().is_err());
        fs::remove_file(&sentinel).unwrap();
        fs::write(&sentinel, &owned.token).unwrap();
        let alias = owned.temporary.path().join("root-alias");
        symlink(&owned.root, &alias).unwrap();
        assert!(validate_fixture(&alias, &owned.token, &owned.root.join("data"),
            &owned.root.join("work"), owned.temporary.path()).is_err());
        fs::remove_dir(owned.root.join("data")).unwrap();
        symlink(owned.root.join("work"), owned.root.join("data")).unwrap();
        assert!(owned.validate().is_err());
    }

    #[test]
    fn other_data_or_work_paths_are_rejected() {
        let owned = OwnedFixture::new();
        let other = OwnedFixture::new();
        assert!(validate_fixture(&owned.root, &owned.token, &other.root.join("data"),
            &owned.root.join("work"), owned.temporary.path()).is_err());
        assert!(validate_fixture(&owned.root, &owned.token, &owned.root.join("data"),
            &other.root.join("work"), owned.temporary.path()).is_err());
    }

    #[test]
    fn compiled_gui_work_override_cannot_escape_the_proven_fixture() {
        let owned = OwnedFixture::new();
        let fixture = owned.validate().unwrap();
        assert!(validate_compiled_work_dir(None, &fixture.work).is_ok());
        assert!(validate_compiled_work_dir(Some(""), &fixture.work).is_ok());
        assert!(validate_compiled_work_dir(fixture.work.to_str(), &fixture.work).is_ok());
        assert!(validate_compiled_work_dir(fixture.data.to_str(), &fixture.work).is_err());
        assert!(validate_compiled_work_dir(Some("relative/work"), &fixture.work).is_err());
    }
}
