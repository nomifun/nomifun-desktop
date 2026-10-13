//! Fingerprint the code supplied by a macOS Chrome application bundle. The
//! launcher is a small executable; Chrome actually loads the versioned framework
//! and helper applications, so a launcher-only SHA cannot bind that release.

use super::HeadlessPageError;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 2048;
const MAX_RELEASE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn contents(path: &Path) -> Option<&Path> {
    if path.file_name()? != "Google Chrome" {
        return None;
    }
    let macos = path.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || contents.parent()?.extension()? != "app"
    {
        return None;
    }
    Some(contents)
}

/// A non-bundle test/runtime binary retains the generic single-file SHA.
pub(super) fn fingerprint(path: &Path) -> Option<Result<String, HeadlessPageError>> {
    contents(path).map(|contents| fingerprint_bundle(path, contents))
}

fn append_file(
    files: &mut Vec<PathBuf>,
    path: PathBuf,
    bundle_root: &Path,
) -> Result<(), HeadlessPageError> {
    if files.len() >= MAX_FILES
        || !path.is_file()
        || !path
            .canonicalize()
            .map_err(|_| HeadlessPageError::Unavailable)?
            .starts_with(bundle_root)
    {
        return Err(HeadlessPageError::Unavailable);
    }
    files.push(path);
    Ok(())
}

fn append_tree(
    files: &mut Vec<PathBuf>,
    directory: &Path,
    bundle_root: &Path,
    depth: usize,
) -> Result<(), HeadlessPageError> {
    if depth > 12
        || !directory
            .canonicalize()
            .map_err(|_| HeadlessPageError::Unavailable)?
            .starts_with(bundle_root)
    {
        return Err(HeadlessPageError::Unavailable);
    }
    for entry in std::fs::read_dir(directory).map_err(|_| HeadlessPageError::Unavailable)? {
        let entry = entry.map_err(|_| HeadlessPageError::Unavailable)?;
        let kind = entry
            .file_type()
            .map_err(|_| HeadlessPageError::Unavailable)?;
        if kind.is_dir() {
            append_tree(files, &entry.path(), bundle_root, depth + 1)?;
        } else {
            // Files may use in-bundle aliases (e.g. CodeResources). Directory
            // symlinks are not followed, preventing loops and external trees.
            append_file(files, entry.path(), bundle_root)?;
        }
    }
    Ok(())
}

fn fingerprint_bundle(path: &Path, contents: &Path) -> Result<String, HeadlessPageError> {
    let root = contents
        .canonicalize()
        .map_err(|_| HeadlessPageError::Unavailable)?;
    let versions = contents.join("Frameworks/Google Chrome Framework.framework/Versions");
    let canonical_versions = versions
        .canonicalize()
        .map_err(|_| HeadlessPageError::Unavailable)?;
    let current = versions.join("Current");
    let version = current
        .canonicalize()
        .map_err(|_| HeadlessPageError::Unavailable)?;
    if !canonical_versions.starts_with(&root)
        || version.parent() != Some(canonical_versions.as_path())
    {
        return Err(HeadlessPageError::Unavailable);
    }
    let mut files = Vec::new();
    append_file(&mut files, path.to_path_buf(), &root)?;
    append_file(&mut files, contents.join("Info.plist"), &root)?;
    append_file(&mut files, current.join("Google Chrome Framework"), &root)?;
    let helpers = current.join("Helpers");
    let mut has_general_helper = false;
    for entry in std::fs::read_dir(&helpers).map_err(|_| HeadlessPageError::Unavailable)? {
        let entry = entry.map_err(|_| HeadlessPageError::Unavailable)?;
        let kind = entry
            .file_type()
            .map_err(|_| HeadlessPageError::Unavailable)?;
        if kind.is_file() || kind.is_symlink() {
            // Chrome can start direct native helpers such as Crashpad without
            // changing the launcher/framework. The independent updater app is
            // not a browser runtime dependency and is not traversed below.
            append_file(&mut files, entry.path(), &root)?;
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(HeadlessPageError::Unavailable);
        };
        if name.starts_with("Google Chrome Helper") && name.ends_with(".app") {
            let helper = entry.path();
            let executable_name = name
                .strip_suffix(".app")
                .ok_or(HeadlessPageError::Unavailable)?;
            if !kind.is_dir()
                || !helper
                    .join("Contents/MacOS")
                    .join(executable_name)
                    .is_file()
            {
                return Err(HeadlessPageError::Unavailable);
            }
            has_general_helper |= name == "Google Chrome Helper.app";
            append_tree(&mut files, &helper, &root, 0)?;
        }
    }
    if !has_general_helper {
        return Err(HeadlessPageError::Unavailable);
    }
    // Versioned native libraries can change without changing the launcher.
    let libraries = current.join("Libraries");
    if libraries.exists() {
        append_tree(&mut files, &libraries, &root, 0)?;
    }
    // V8 context snapshots and packed runtime resources are also loaded by the
    // framework; changes to them must invalidate the same installed release.
    let resources = current.join("Resources");
    if resources.exists() {
        append_tree(&mut files, &resources, &root, 0)?;
    }
    files.sort();
    let mut digest = Sha256::new();
    digest.update(b"nomifun-macos-chrome-release-v1\0");
    let version_name = version
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(HeadlessPageError::Unavailable)?;
    digest.update(version_name.len().to_le_bytes());
    digest.update(version_name.as_bytes());
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    for file in files {
        let relative = file
            .strip_prefix(contents)
            .map_err(|_| HeadlessPageError::Unavailable)?;
        let label = relative.to_str().ok_or(HeadlessPageError::Unavailable)?;
        let mut handle = std::fs::File::open(&file).map_err(|_| HeadlessPageError::Unavailable)?;
        let metadata = handle
            .metadata()
            .map_err(|_| HeadlessPageError::Unavailable)?;
        if !metadata.is_file() {
            return Err(HeadlessPageError::Unavailable);
        }
        total = total
            .checked_add(metadata.len())
            .filter(|bytes| *bytes <= MAX_RELEASE_BYTES)
            .ok_or(HeadlessPageError::Unavailable)?;
        digest.update(label.len().to_le_bytes());
        digest.update(label.as_bytes());
        digest.update(metadata.len().to_le_bytes());
        let mut read = 0u64;
        loop {
            let count = handle
                .read(&mut buffer)
                .map_err(|_| HeadlessPageError::Unavailable)?;
            if count == 0 {
                break;
            }
            read = read
                .checked_add(count as u64)
                .filter(|bytes| *bytes <= metadata.len())
                .ok_or(HeadlessPageError::Unavailable)?;
            digest.update(&buffer[..count]);
        }
        if read != metadata.len() {
            return Err(HeadlessPageError::Unavailable);
        }
    }
    if current
        .canonicalize()
        .map_err(|_| HeadlessPageError::Unavailable)?
        != version
    {
        return Err(HeadlessPageError::BindingChanged);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn bundle() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let contents = directory.path().join("Google Chrome.app/Contents");
        let entry = contents.join("MacOS/Google Chrome");
        let version =
            contents.join("Frameworks/Google Chrome Framework.framework/Versions/155.0.8059.39");
        let helper =
            version.join("Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&entry, b"launcher").unwrap();
        std::fs::write(contents.join("Info.plist"), b"metadata").unwrap();
        std::fs::write(version.join("Google Chrome Framework"), b"framework").unwrap();
        std::fs::write(&helper, b"helper").unwrap();
        symlink("155.0.8059.39", version.parent().unwrap().join("Current")).unwrap();
        (directory, entry, version)
    }

    #[test]
    fn framework_and_helper_changes_invalidate_an_unchanged_launcher() {
        let (_directory, entry, version) = bundle();
        let initial = fingerprint(&entry).unwrap().unwrap();
        assert_eq!(fingerprint(&entry).unwrap().unwrap(), initial);
        std::fs::write(
            version.join("Google Chrome Framework"),
            b"changed framework",
        )
        .unwrap();
        let changed_framework = fingerprint(&entry).unwrap().unwrap();
        assert_ne!(changed_framework, initial);
        std::fs::write(
            version.join("Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper"),
            b"changed helper",
        )
        .unwrap();
        assert_ne!(fingerprint(&entry).unwrap().unwrap(), changed_framework);
        assert_eq!(std::fs::read(entry).unwrap(), b"launcher");
    }

    #[test]
    fn direct_native_helpers_are_part_of_the_release_fingerprint() {
        let (_directory, entry, version) = bundle();
        let crashpad = version.join("Helpers/chrome_crashpad_handler");
        std::fs::write(&crashpad, b"crash handler").unwrap();
        let initial = fingerprint(&entry).unwrap().unwrap();
        std::fs::write(&crashpad, b"changed crash handler").unwrap();
        assert_ne!(fingerprint(&entry).unwrap().unwrap(), initial);
    }

    #[test]
    fn versioned_context_snapshots_and_runtime_resources_invalidate_the_release() {
        let (_directory, entry, version) = bundle();
        let snapshot = version.join("Resources/v8_context_snapshot.arm64.bin");
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, b"context snapshot").unwrap();
        let initial = fingerprint(&entry).unwrap().unwrap();
        std::fs::write(&snapshot, b"changed context snapshot").unwrap();
        assert_ne!(fingerprint(&entry).unwrap().unwrap(), initial);
    }

    #[test]
    fn missing_framework_and_framework_symlinks_outside_bundle_fail_closed() {
        let (_directory, entry, version) = bundle();
        let framework = version.join("Google Chrome Framework");
        std::fs::remove_file(&framework).unwrap();
        assert_eq!(
            fingerprint(&entry).unwrap(),
            Err(HeadlessPageError::Unavailable)
        );
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), &framework).unwrap();
        assert_eq!(
            fingerprint(&entry).unwrap(),
            Err(HeadlessPageError::Unavailable)
        );
    }

    #[test]
    fn current_version_cannot_escape_the_owned_bundle_versions() {
        let (_directory, entry, version) = bundle();
        let current = version.parent().unwrap().join("Current");
        std::fs::remove_file(&current).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), &current).unwrap();
        assert_eq!(
            fingerprint(&entry).unwrap(),
            Err(HeadlessPageError::Unavailable)
        );
    }

    #[test]
    fn standalone_runtime_does_not_gain_a_bundle_requirement() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(fingerprint(file.path()).is_none());
    }
}
