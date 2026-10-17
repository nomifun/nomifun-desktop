//! Full Node/npm/npx publication. The complete payload has a build-pinned identity.
use crate::managed::{contract::{ComponentAsset, ComponentSpec}, provenance_bytes};
use crate::node_distribution_contract::{self as distribution, Entry};
use crate::{ToolError, ToolId};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

const DIRECTORY: &str = "distribution-v1";
const INDEX: &str = "DISTRIBUTION.json";

#[derive(Deserialize, Serialize)]
struct Publication {
    schema_version: u32,
    version: String,
    archive_sha256: String,
    executable_sha256: String,
    entries: Vec<Entry>,
}

pub(crate) fn executable_path(directory: &Path, asset: &ComponentAsset) -> PathBuf {
    let root = directory.join(DIRECTORY);
    if asset.target.contains("windows") { root.join(&asset.executable) }
    else { root.join("bin").join(&asset.executable) }
}

fn mapped(error: std::io::Error, cancel: &CancellationToken) -> ToolError {
    if cancel.is_cancelled() { ToolError::Cancelled }
    else if error.kind() == std::io::ErrorKind::InvalidData { ToolError::InvalidAsset(error.to_string()) }
    else { ToolError::Io(error) }
}

/// Read-only validation. An old single-file cache is deliberately not a Node distribution.
pub(crate) fn validated_cached(
    directory: &Path,
    spec: &ComponentSpec,
    asset: &ComponentAsset,
    executable_sha256: &str,
    distribution_sha256: &str,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>, ToolError> {
    let root = directory.join(DIRECTORY);
    let index = root.join(INDEX);
    let metadata = match fs::symlink_metadata(&index) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 { return Ok(None); }
    let publication: Publication = match serde_json::from_slice(&fs::read(index)?) {
        Ok(publication) => publication,
        Err(_) => return Ok(None),
    };
    if publication.schema_version != 1 || publication.version != spec.version
        || publication.archive_sha256 != asset.sha256 || publication.executable_sha256 != executable_sha256
        || distribution::identity(&publication.entries).map_err(|error| mapped(error, cancel))? != distribution_sha256
    { return Ok(None); }
    let mut metadata_files = vec![INDEX, "PROVENANCE.json"];
    metadata_files.extend(crate::notices::files(ToolId::Node).iter().map(|notice| notice.filename));
    let valid = match distribution::validate(&root, &publication.entries, &metadata_files, || cancel.is_cancelled()) {
        Ok(valid) => valid,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(mapped(error, cancel)),
    };
    if !valid {
        return Ok(None);
    }
    let provenance = provenance_bytes(spec, asset, executable_sha256);
    if fs::read(root.join("PROVENANCE.json")).ok().as_deref() != Some(&provenance) { return Ok(None); }
    for notice in crate::notices::files(ToolId::Node) {
        if fs::read(root.join(notice.filename)).ok().as_deref() != Some(notice.contents) { return Ok(None); }
    }
    Ok(Some(executable_path(directory, asset)))
}

pub(crate) fn publish(
    directory: &Path,
    archive: &Path,
    spec: &ComponentSpec,
    asset: &ComponentAsset,
    executable_sha256: &str,
    distribution_sha256: &str,
    cancel: &CancellationToken,
) -> Result<PathBuf, ToolError> {
    let staging = tempfile::Builder::new().prefix(".node-extract-").suffix(".part").tempdir_in(directory)?;
    let entries = distribution::decode(archive, &asset.format, Some(staging.path()), || cancel.is_cancelled())
        .map_err(|error| mapped(error, cancel))?;
    if distribution::identity(&entries).map_err(|error| mapped(error, cancel))? != distribution_sha256 {
        return Err(ToolError::InvalidAsset("Node distribution differs from approved build identity".into()));
    }
    let executable_relative = if asset.target.contains("windows") { asset.executable.clone() } else { format!("bin/{}", asset.executable) };
    if !entries.iter().any(|entry| entry.path == executable_relative && entry.kind == "file" && entry.digest == executable_sha256) {
        return Err(ToolError::InvalidAsset("Node executable differs from approved build identity".into()));
    }
    for required in if asset.target.contains("windows") {
        ["npm.cmd", "npx.cmd", "node_modules/npm/bin/npm-cli.js", "node_modules/npm/bin/npx-cli.js"]
    } else {
        ["bin/npm", "bin/npx", "lib/node_modules/npm/bin/npm-cli.js", "lib/node_modules/npm/bin/npx-cli.js"]
    } {
        if !entries.iter().any(|entry| entry.path == required) { return Err(ToolError::InvalidAsset(format!("Node distribution lacks {required}"))); }
    }
    let publication = Publication { schema_version: 1, version: spec.version.clone(), archive_sha256: asset.sha256.clone(), executable_sha256: executable_sha256.into(), entries };
    write_file(&staging.path().join(INDEX), &serde_json::to_vec(&publication).map_err(|error| ToolError::InvalidAsset(error.to_string()))?)?;
    write_file(&staging.path().join("PROVENANCE.json"), &provenance_bytes(spec, asset, executable_sha256))?;
    for notice in crate::notices::files(ToolId::Node) { write_file(&staging.path().join(notice.filename), notice.contents)?; }
    if cancel.is_cancelled() { return Err(ToolError::Cancelled); }
    let destination = directory.join(DIRECTORY);
    // The supplier holds the cross-process asset lock. Readers never see a partially extracted
    // tree; interrupted replacement is recovered on the next preparation.
    let retired = directory.join(".node-retired.part");
    if retired.exists() { fs::remove_dir_all(&retired)?; }
    if destination.exists() { fs::rename(&destination, &retired)?; }
    let result = fs::rename(staging.path(), &destination);
    if let Err(error) = result {
        if retired.exists() { let _ = fs::rename(&retired, &destination); }
        return Err(error.into());
    }
    #[cfg(unix)] File::open(directory)?.sync_all()?;
    if retired.exists() { fs::remove_dir_all(retired)?; }
    Ok(executable_path(directory, asset))
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), ToolError> {
    use std::io::Write;
    let mut file = File::options().write(true).create_new(true).open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

/// Metadata admission for every immutable payload entry. This is intentionally
/// separate from the content hash: normal launches check metadata; first cache
/// admission and repair compare the entire tree to its compiled content pin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DistributionFingerprint {
    root: PathBuf,
    entries: Vec<(PathBuf, EntryFingerprint)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EntryFingerprint {
    size: u64,
    modified: Option<std::time::SystemTime>,
    target: Option<PathBuf>,
    #[cfg(unix)]
    unix: (u64, u64, u32, i64, i64),
    #[cfg(windows)]
    windows: (u64, u64, u32),
}

fn entry_fingerprint(path: &Path) -> Result<EntryFingerprint, ToolError> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(unix)] use std::os::unix::fs::MetadataExt;
    #[cfg(windows)] use std::os::windows::fs::MetadataExt;
    Ok(EntryFingerprint {
        size: metadata.len(), modified: metadata.modified().ok(),
        target: if metadata.file_type().is_symlink() { Some(fs::read_link(path)?) } else { None },
        #[cfg(unix)] unix: (metadata.dev(), metadata.ino(), metadata.mode(), metadata.ctime(), metadata.ctime_nsec()),
        #[cfg(windows)] windows: (metadata.creation_time(), metadata.last_write_time(), metadata.file_attributes()),
    })
}

impl DistributionFingerprint {
    /// Directory metadata participates so adding unindexed modules or replacing
    /// a launcher is detected as well as modifying an existing module.
    pub(crate) fn validate(&self) -> Result<bool, ToolError> {
        for (relative, expected) in &self.entries {
            let actual = match entry_fingerprint(&self.root.join(relative)) {
                Ok(actual) => actual,
                Err(ToolError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error),
            };
            if &actual != expected { return Ok(false); }
        }
        Ok(true)
    }
}

pub(crate) fn fingerprint(executable_path: &Path) -> Result<DistributionFingerprint, ToolError> {
    let root = if executable_path.file_name().is_some_and(|name| name == "node.exe") {
        executable_path.parent()
    } else { executable_path.parent().and_then(Path::parent) }
        .ok_or_else(|| ToolError::InvalidAsset("Node distribution path lacks its root".into()))?.to_path_buf();
    if root.file_name().is_none_or(|name| name != DIRECTORY) {
        return Err(ToolError::InvalidAsset("Node distribution is outside its published tree".into()));
    }
    let mut entries = std::collections::BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        entries.insert(relative.clone(), entry_fingerprint(&path)?);
        if entries.len() > 32_800 { return Err(ToolError::InvalidAsset("Node distribution entry limit".into())); }
        if metadata.is_dir() {
            for child in fs::read_dir(&path)? { pending.push(relative.join(child?.file_name())); }
        }
    }
    let fingerprint = DistributionFingerprint { root, entries: entries.into_iter().collect() };
    if !fingerprint.validate()? { return Err(ToolError::Changed { tool: "node" }); }
    Ok(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn fixture() -> (tempfile::TempDir, PathBuf, ComponentSpec, ComponentAsset, String, String) {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("node.tar.gz");
        let compressor = flate2::write::GzEncoder::new(File::create(&archive).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(compressor);
        for path in ["node-fixture/", "node-fixture/bin/", "node-fixture/lib/", "node-fixture/lib/node_modules/", "node-fixture/lib/node_modules/npm/", "node-fixture/lib/node_modules/npm/bin/"] {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory); header.set_mode(0o755); header.set_size(0); header.set_cksum();
            tar.append_data(&mut header, path, std::io::empty()).unwrap();
        }
        for (path, bytes, mode) in [
            ("bin/node", b"#!/bin/sh\nprintf '%s\\n' fixture-node\n".as_slice(), 0o755),
            ("lib/node_modules/npm/bin/npm-cli.js", b"npm fixture with relative modules\n".as_slice(), 0o755),
            ("lib/node_modules/npm/bin/npx-cli.js", b"npx fixture\n".as_slice(), 0o755),
            ("lib/node_modules/npm/LICENSE", b"fixture license\n".as_slice(), 0o644),
        ] {
            let mut header = tar::Header::new_gnu(); header.set_mode(mode); header.set_size(bytes.len() as u64); header.set_cksum();
            tar.append_data(&mut header, format!("node-fixture/{path}"), bytes).unwrap();
        }
        for (path, target) in [("bin/npm", "../lib/node_modules/npm/bin/npm-cli.js"), ("bin/npx", "../lib/node_modules/npm/bin/npx-cli.js")] {
            let mut header = tar::Header::new_gnu(); header.set_mode(0o777); header.set_size(0); header.set_entry_type(tar::EntryType::Symlink); header.set_link_name(target).unwrap(); header.set_cksum();
            tar.append_data(&mut header, format!("node-fixture/{path}"), std::io::empty()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let mut spec = crate::managed::specification(ToolId::Node);
        let mut asset = spec.assets[0].clone(); asset.target = "aarch64-apple-darwin".into(); asset.format = "tar.gz".into(); asset.sha256 = hex::encode(Sha256::digest(fs::read(&archive).unwrap()));
        spec.assets = vec![asset.clone()];
        let entries = distribution::decode(&archive, &asset.format, None, || false).unwrap();
        let executable = entries.iter().find(|entry| entry.path == "bin/node").unwrap().digest.clone();
        let tree = distribution::identity(&entries).unwrap();
        (root, archive, spec, asset, executable, tree)
    }

    #[test]
    #[cfg(unix)]
    fn full_tree_preserves_launchers_relative_modules_modes_and_rejects_modified_npm() {
        use std::os::unix::fs::PermissionsExt;
        let (_fixture, archive, spec, asset, executable, tree) = fixture();
        let destination = tempfile::tempdir().unwrap(); let cancel = CancellationToken::new();
        let path = publish(destination.path(), &archive, &spec, &asset, &executable, &tree, &cancel).unwrap();
        assert_eq!(path, destination.path().join("distribution-v1/bin/node"));
        assert_eq!(fs::read_link(path.parent().unwrap().join("npm")).unwrap(), Path::new("../lib/node_modules/npm/bin/npm-cli.js"));
        assert_ne!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
        assert_eq!(validated_cached(destination.path(), &spec, &asset, &executable, &tree, &cancel).unwrap(), Some(path));
        let fingerprint = fingerprint(&destination.path().join("distribution-v1/bin/node")).unwrap();
        assert!(fingerprint.validate().unwrap());
        let npm = destination.path().join("distribution-v1/lib/node_modules/npm/bin/npm-cli.js");
        fs::write(npm, b"modified npm").unwrap();
        assert!(!fingerprint.validate().unwrap());
        assert_eq!(validated_cached(destination.path(), &spec, &asset, &executable, &tree, &cancel).unwrap(), None);
        let repaired = publish(destination.path(), &archive, &spec, &asset, &executable, &tree, &cancel).unwrap();
        assert_eq!(validated_cached(destination.path(), &spec, &asset, &executable, &tree, &cancel).unwrap(), Some(repaired));
    }

    #[test]
    fn a_bare_legacy_node_is_not_a_complete_distribution_and_cancellation_does_not_publish() {
        let (_fixture, archive, spec, asset, executable, tree) = fixture();
        let destination = tempfile::tempdir().unwrap(); fs::write(destination.path().join("node"), b"legacy executable").unwrap();
        let cancel = CancellationToken::new();
        assert_eq!(validated_cached(destination.path(), &spec, &asset, &executable, &tree, &cancel).unwrap(), None);
        cancel.cancel();
        assert!(matches!(publish(destination.path(), &archive, &spec, &asset, &executable, &tree, &cancel), Err(ToolError::Cancelled)));
        assert!(!destination.path().join(DIRECTORY).exists());
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_complete_archive_cannot_publish_with_a_different_build_tree_identity() {
        let (_fixture, archive, spec, asset, executable, _) = fixture(); let destination = tempfile::tempdir().unwrap();
        assert!(matches!(publish(destination.path(), &archive, &spec, &asset, &executable, &"0".repeat(64), &CancellationToken::new()), Err(ToolError::InvalidAsset(_))));
        assert!(!destination.path().join(DIRECTORY).exists());
    }

    #[test]
    fn windows_zip_preserves_cmd_launchers_and_modules_beside_node() {
        let root = tempfile::tempdir().unwrap(); let archive = root.path().join("node.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        for (name, bytes) in [("node.exe", b"node windows fixture".as_slice()), ("npm.cmd", b"@node npm-cli.js %*".as_slice()), ("npx.cmd", b"@node npx-cli.js %*".as_slice()), ("node_modules/npm/bin/npm-cli.js", b"npm".as_slice()), ("node_modules/npm/bin/npx-cli.js", b"npx".as_slice())] {
            zip.start_file(format!("node-fixture/{name}"), zip::write::SimpleFileOptions::default().unix_permissions(0o644)).unwrap(); zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let spec = crate::managed::specification(ToolId::Node); let mut asset = spec.assets[0].clone(); asset.format = "zip".into(); asset.target = "x86_64-pc-windows-msvc".into(); asset.executable = "node.exe".into();
        let entries = distribution::decode(&archive, "zip", None, || false).unwrap(); let tree = distribution::identity(&entries).unwrap();
        let executable = entries.iter().find(|entry| entry.path == "node.exe").unwrap().digest.clone(); let destination = tempfile::tempdir().unwrap();
        let path = publish(destination.path(), &archive, &spec, &asset, &executable, &tree, &CancellationToken::new()).unwrap();
        assert!(path.parent().unwrap().join("npm.cmd").is_file()); assert!(path.parent().unwrap().join("node_modules/npm/bin/npx-cli.js").is_file());
    }

    #[test]
    fn zip_traversal_and_escaping_links_are_rejected_before_publication() {
        let root = tempfile::tempdir().unwrap(); let archive = root.path().join("escape.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("node-fixture/../../escape", zip::write::SimpleFileOptions::default()).unwrap(); zip.write_all(b"escape").unwrap(); zip.finish().unwrap();
        assert!(distribution::decode(&archive, "zip", None, || false).is_err());
        let archive = root.path().join("link.zip"); let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.add_symlink("node-fixture/bin/npm", "../../../outside", zip::write::SimpleFileOptions::default()).unwrap(); zip.finish().unwrap();
        assert!(distribution::decode(&archive, "zip", None, || false).is_err());
    }
}
