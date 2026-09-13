use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{
    ArtifactId, CandidateTestProvenance, LogicalArtifactRef, PLUGIN_N1_SCHEMA_VERSION,
    PLUGIN_PACKAGE_PROFILE_VERSION, PluginPackageArtifactV1, PluginProjectId,
    PluginShareBundleManifest, PluginShareSourceLineage, canonical_json_bytes, digest_bytes,
};
use nomifun_js_authoring::{
    ExactDependencyLock, NormalizedSourcePath, PluginSourceArchive, PluginSourceFileBytes,
    SourceSnapshot,
};
use crate::StoredPluginArtifact;
use uuid::Uuid;

use crate::application::PluginServiceError;

const BUNDLE_FILE: &str = "bundle.json";
const ARTIFACT_RECORD_FILE: &str = "artifact/artifact.json";
const ARTIFACT_MANIFEST_FILE: &str = "artifact/package/manifest.json";
const ARTIFACT_PACKAGE_PREFIX: &str = "artifact/package/";
const SOURCE_SNAPSHOT_FILE: &str = "source/snapshot.json";
const SOURCE_LOCK_FILE: &str = "source/dependency-lock.json";
const SOURCE_FILES_PREFIX: &str = "source/files/";
const STAGING_PREFIX: &str = ".nomifun-plugin-share-staging-";
const MAX_FILE_COUNT: usize = 8_192;
const MAX_SINGLE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 528 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;

pub struct PluginShareExport<'a> {
    pub originating_project_id: Option<&'a str>,
    pub artifact: &'a StoredPluginArtifact,
    pub source: Option<&'a PluginSourceArchive>,
    pub test_provenance: Option<CandidateTestProvenance>,
}

pub struct ImportedPluginShareBundle {
    pub manifest: PluginShareBundleManifest,
    pub bundle_digest: String,
    pub artifact: PluginPackageArtifactV1,
    pub artifact_package_root: PathBuf,
    pub source: Option<PluginSourceArchive>,
}

#[derive(Clone, Debug, Default)]
pub struct PluginShareBundleFilesystem;

impl PluginShareBundleFilesystem {
    pub fn export(
        &self,
        request: PluginShareExport<'_>,
        destination: impl AsRef<Path>,
    ) -> Result<PluginShareBundleManifest, PluginServiceError> {
        request
            .artifact
            .artifact
            .validate()
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let source_lineage = match request.source {
            Some(source) => {
                let project_id = request.originating_project_id.ok_or_else(|| {
                    PluginServiceError::invalid(
                        "Plugin Share Source requires an originating Project identity",
                    )
                })?;
                let lock_digest = source
                    .dependency_lock()
                    .digest()
                    .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
                if lock_digest != request.artifact.artifact.manifest.payload.dependency_lock_digest {
                    return Err(PluginServiceError::stale(
                        "Plugin Share Source lock differs from the exported Artifact lineage",
                    ));
                }
                require_source_package_identity(source, &request.artifact.artifact)?;
                Some(PluginShareSourceLineage {
                    originating_project_id: Some(PluginProjectId::from(project_id.to_owned())),
                    source_archive: LogicalArtifactRef {
                        artifact_id: ArtifactId::from(Uuid::now_v7().to_string()),
                        normalized_relative_path: SOURCE_SNAPSHOT_FILE.into(),
                        digest: source.snapshot().digest().clone(),
                    },
                    source_snapshot_digest: source.snapshot().digest().clone(),
                    dependency_lock_digest: lock_digest,
                    build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
                })
            }
            None => None,
        };
        let manifest = PluginShareBundleManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            bundle_format_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            package_artifact: LogicalArtifactRef {
                artifact_id: request.artifact.artifact.artifact_id.clone(),
                normalized_relative_path: ARTIFACT_RECORD_FILE.into(),
                digest: request.artifact.artifact.artifact_digest.clone(),
            },
            package_artifact_digest: request.artifact.artifact.artifact_digest.clone(),
            package_manifest_digest: request.artifact.artifact.manifest.payload_digest.clone(),
            source_lineage,
            test_provenance: request.test_provenance,
        };
        manifest
            .validate()
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let destination = destination.as_ref();
        let parent = destination.parent().ok_or_else(|| {
            PluginServiceError::invalid("Plugin Share destination requires an existing parent")
        })?;
        require_regular_directory(parent)?;
        require_normal_destination(destination)?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PluginServiceError::conflict(format!(
                "Plugin Share destination already exists: {}",
                destination.display()
            )));
        }
        let mut staging = StagingGuard::allocate(parent)?;
        write_new_synced(
            &staging.path().join(BUNDLE_FILE),
            &canonical_json_bytes(&manifest)
                .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
        )?;
        write_exported_artifact(staging.path(), request.artifact)?;
        if let Some(source) = request.source {
            write_exported_source(staging.path(), source)?;
        }
        sync_tree(staging.path())?;
        match fs::rename(staging.path(), destination) {
            Ok(()) => {}
            Err(_error) if fs::symlink_metadata(destination).is_ok() => {
                return Err(PluginServiceError::conflict(format!(
                    "Plugin Share destination already exists: {}",
                    destination.display()
                )));
            }
            Err(error) => return Err(io_error(destination, error)),
        }
        sync_directory(parent)?;
        staging.disarm();
        Ok(manifest)
    }

    pub fn import(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<ImportedPluginShareBundle, PluginServiceError> {
        let root = root.as_ref();
        let observed = scan_regular_files(root)?;
        let bundle_bytes = read_regular_bounded(&root.join(BUNDLE_FILE), MAX_METADATA_BYTES)?;
        let manifest: PluginShareBundleManifest = read_canonical(&bundle_bytes, "Share manifest")?;
        manifest
            .validate()
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        if manifest.package_artifact.normalized_relative_path != ARTIFACT_RECORD_FILE {
            return Err(PluginServiceError::invalid(
                "Plugin Share Artifact reference uses an unsupported path",
            ));
        }
        let artifact_bytes =
            read_regular_bounded(&root.join(ARTIFACT_RECORD_FILE), MAX_METADATA_BYTES)?;
        let artifact: PluginPackageArtifactV1 = read_canonical(&artifact_bytes, "Artifact record")?;
        artifact
            .validate()
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        if manifest.package_artifact.artifact_id != artifact.artifact_id
            || manifest.package_artifact.digest != artifact.artifact_digest
            || manifest.package_artifact_digest != artifact.artifact_digest
            || manifest.package_manifest_digest != artifact.manifest.payload_digest
        {
            return Err(PluginServiceError::stale(
                "Plugin Share manifest differs from its exact Artifact",
            ));
        }
        let mut expected = BTreeSet::from([
            BUNDLE_FILE.to_owned(),
            ARTIFACT_RECORD_FILE.to_owned(),
            ARTIFACT_MANIFEST_FILE.to_owned(),
        ]);
        let manifest_bytes = read_regular_bounded(
            &root.join(ARTIFACT_MANIFEST_FILE),
            MAX_METADATA_BYTES,
        )?;
        let expected_manifest_bytes = canonical_json_bytes(&artifact.manifest)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        if manifest_bytes != expected_manifest_bytes {
            return Err(PluginServiceError::stale(
                "Plugin Share package manifest bytes were modified",
            ));
        }
        for file in &artifact.files {
            let relative = format!(
                "{ARTIFACT_PACKAGE_PREFIX}{}",
                file.normalized_relative_path
            );
            expected.insert(relative.clone());
            let bytes = read_regular_bounded(&root.join(&relative), MAX_SINGLE_FILE_BYTES)?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != file.size_bytes
                || digest_bytes(&bytes) != file.digest
            {
                return Err(PluginServiceError::stale(format!(
                    "Plugin Share Artifact file was modified: {}",
                    file.normalized_relative_path
                )));
            }
        }
        let source = match manifest.source_lineage.as_ref() {
            Some(lineage) => {
                if lineage.source_archive.normalized_relative_path != SOURCE_SNAPSHOT_FILE {
                    return Err(PluginServiceError::invalid(
                        "Plugin Share Source reference uses an unsupported path",
                    ));
                }
                expected.insert(SOURCE_SNAPSHOT_FILE.into());
                expected.insert(SOURCE_LOCK_FILE.into());
                let snapshot_bytes =
                    read_regular_bounded(&root.join(SOURCE_SNAPSHOT_FILE), MAX_METADATA_BYTES)?;
                let snapshot: SourceSnapshot = read_canonical(&snapshot_bytes, "Source snapshot")?;
                let lock_bytes =
                    read_regular_bounded(&root.join(SOURCE_LOCK_FILE), MAX_METADATA_BYTES)?;
                let lock: ExactDependencyLock = read_canonical(&lock_bytes, "dependency lock")?;
                let lock_digest = lock
                    .digest()
                    .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
                if lineage.source_archive.digest != *snapshot.digest()
                    || lineage.source_snapshot_digest != *snapshot.digest()
                    || lineage.dependency_lock_digest != lock_digest
                    || lineage.dependency_lock_digest
                        != artifact.manifest.payload.dependency_lock_digest
                {
                    return Err(PluginServiceError::stale(
                        "Plugin Share Source, lock, and Artifact lineage disagree",
                    ));
                }
                let mut files = Vec::with_capacity(snapshot.files().len());
                for file in snapshot.files() {
                    let relative = format!(
                        "{SOURCE_FILES_PREFIX}{}",
                        file.normalized_relative_path()
                    );
                    expected.insert(relative.clone());
                    let bytes =
                        read_regular_bounded(&root.join(&relative), MAX_SINGLE_FILE_BYTES)?;
                    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != file.size_bytes()
                        || digest_bytes(&bytes) != *file.digest()
                    {
                        return Err(PluginServiceError::stale(format!(
                            "Plugin Share Source file was modified: {}",
                            file.normalized_relative_path()
                        )));
                    }
                    files.push(PluginSourceFileBytes::new(
                        file.normalized_relative_path().clone(),
                        bytes,
                    ));
                }
                let source = PluginSourceArchive::new(snapshot, lock, files);
                require_source_package_identity(&source, &artifact)?;
                Some(source)
            }
            None => None,
        };
        if observed.files != expected || observed.directories != expected_directories(&expected) {
            return Err(PluginServiceError::invalid(
                "Plugin Share Bundle contains missing or extra files",
            ));
        }
        Ok(ImportedPluginShareBundle {
            manifest,
            bundle_digest: digest_bytes(&bundle_bytes).as_ref().to_owned(),
            artifact,
            artifact_package_root: root.join("artifact/package"),
            source,
        })
    }
}

fn require_source_package_identity(
    source: &PluginSourceArchive,
    artifact: &PluginPackageArtifactV1,
) -> Result<(), PluginServiceError> {
    let package_json = source
        .files()
        .iter()
        .find(|file| file.normalized_relative_path().as_str() == "package.json")
        .ok_or_else(|| PluginServiceError::invalid("Plugin Share Source has no package.json"))?;
    let package: serde_json::Value = serde_json::from_slice(package_json.bytes())
        .map_err(|error| PluginServiceError::invalid(format!("invalid package.json: {error}")))?;
    let name = package.get("name").and_then(serde_json::Value::as_str);
    let version = package.get("version").and_then(serde_json::Value::as_str);
    if name != Some(artifact.manifest.payload.package.package_id.as_ref())
        || version != Some(artifact.manifest.payload.package.package_version.as_ref())
    {
        return Err(PluginServiceError::stale(
            "Plugin Share Source package identity differs from its Artifact",
        ));
    }
    Ok(())
}

fn write_exported_artifact(
    root: &Path,
    stored: &StoredPluginArtifact,
) -> Result<(), PluginServiceError> {
    write_new_synced(
        &root.join(ARTIFACT_RECORD_FILE),
        &canonical_json_bytes(&stored.artifact)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
    )?;
    write_new_synced(
        &root.join(ARTIFACT_MANIFEST_FILE),
        &canonical_json_bytes(&stored.artifact.manifest)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
    )?;
    for file in &stored.artifact.files {
        let source = NormalizedSourcePath::parse(&file.normalized_relative_path)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?
            .join(&stored.package_root);
        let bytes = read_regular_bounded(&source, MAX_SINGLE_FILE_BYTES)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != file.size_bytes
            || digest_bytes(&bytes) != file.digest
        {
            return Err(PluginServiceError::stale(format!(
                "stored Plugin Artifact changed during Share export: {}",
                file.normalized_relative_path
            )));
        }
        write_new_synced(
            &root.join(format!(
                "{ARTIFACT_PACKAGE_PREFIX}{}",
                file.normalized_relative_path
            )),
            &bytes,
        )?;
    }
    Ok(())
}

fn write_exported_source(
    root: &Path,
    source: &PluginSourceArchive,
) -> Result<(), PluginServiceError> {
    write_new_synced(
        &root.join(SOURCE_SNAPSHOT_FILE),
        &canonical_json_bytes(source.snapshot())
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
    )?;
    write_new_synced(
        &root.join(SOURCE_LOCK_FILE),
        &canonical_json_bytes(source.dependency_lock())
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
    )?;
    for file in source.files() {
        write_new_synced(
            &root.join(format!(
                "{SOURCE_FILES_PREFIX}{}",
                file.normalized_relative_path()
            )),
            file.bytes(),
        )?;
    }
    Ok(())
}

struct ObservedTree {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

fn scan_regular_files(root: &Path) -> Result<ObservedTree, PluginServiceError> {
    require_regular_directory(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut total = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| io_error(&directory, error))? {
            let entry = entry.map_err(|error| io_error(&directory, error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
            if metadata_is_link_or_reparse(&metadata) {
                return Err(PluginServiceError::invalid(
                    "Plugin Share Bundle cannot contain links or reparse entries",
                ));
            }
            if metadata.is_dir() {
                directories.insert(normalized_relative(root, &path)?);
                pending.push(path);
                continue;
            }
            if !metadata.is_file() || metadata.len() > MAX_SINGLE_FILE_BYTES {
                return Err(PluginServiceError::invalid(
                    "Plugin Share Bundle contains an unsupported or oversized entry",
                ));
            }
            total = total.saturating_add(metadata.len());
            if total > MAX_TOTAL_BYTES || files.len() >= MAX_FILE_COUNT {
                return Err(PluginServiceError::invalid(
                    "Plugin Share Bundle exceeds its fixed inventory budget",
                ));
            }
            let normalized = normalized_relative(root, &path)?;
            if normalized.is_empty()
                || normalized.len() > 1_024
                || normalized.contains('\\')
                || normalized.chars().any(char::is_control)
                || !files.insert(normalized)
            {
                return Err(PluginServiceError::invalid(
                    "Plugin Share Bundle contains an invalid or colliding path",
                ));
            }
        }
    }
    Ok(ObservedTree { files, directories })
}

fn normalized_relative(root: &Path, path: &Path) -> Result<String, PluginServiceError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| PluginServiceError::invalid("Share entry escaped its root"))?;
    let normalized = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| PluginServiceError::invalid("Share path must be UTF-8"))?
        .join("/");
    if normalized.is_empty()
        || normalized.len() > 1_024
        || normalized.contains('\\')
        || normalized.chars().any(char::is_control)
    {
        return Err(PluginServiceError::invalid(
            "Plugin Share Bundle contains an invalid path",
        ));
    }
    Ok(normalized)
}

fn expected_directories(files: &BTreeSet<String>) -> BTreeSet<String> {
    let mut directories = BTreeSet::new();
    for file in files {
        let mut current = file.as_str();
        while let Some(index) = current.rfind('/') {
            current = &current[..index];
            directories.insert(current.to_owned());
        }
    }
    directories
}

fn read_canonical<T: serde::de::DeserializeOwned + serde::Serialize>(
    bytes: &[u8],
    label: &str,
) -> Result<T, PluginServiceError> {
    let value: T = serde_json::from_slice(bytes)
        .map_err(|error| PluginServiceError::invalid(format!("invalid {label}: {error}")))?;
    let canonical = canonical_json_bytes(&value)
        .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
    if canonical != bytes {
        return Err(PluginServiceError::invalid(format!(
            "{label} must use canonical JSON"
        )));
    }
    Ok(value)
}

fn read_regular_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, PluginServiceError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() || metadata.len() > limit {
        return Err(PluginServiceError::invalid(format!(
            "Plugin Share file is missing, unsafe, or oversized: {}",
            path.display()
        )));
    }
    let file = File::open(path).map_err(|error| io_error(path, error))?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != metadata.len() {
        return Err(PluginServiceError::stale(format!(
            "Plugin Share file changed while reading: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), PluginServiceError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SINGLE_FILE_BYTES {
        return Err(PluginServiceError::invalid("Plugin Share file is oversized"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| PluginServiceError::invalid("Plugin Share path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes).map_err(|error| io_error(path, error))?;
    file.sync_all().map_err(|error| io_error(path, error))
}

fn require_regular_directory(path: &Path) -> Result<(), PluginServiceError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        Err(PluginServiceError::invalid(format!(
            "Plugin Share directory is unsafe: {}",
            path.display()
        )))
    } else {
        Ok(())
    }
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn require_normal_destination(path: &Path) -> Result<(), PluginServiceError> {
    let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        PluginServiceError::invalid("Plugin Share destination must have a UTF-8 name")
    })?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
        || is_windows_reserved_name(name)
    {
        Err(PluginServiceError::invalid(
            "Plugin Share destination name is unsafe",
        ))
    } else {
        Ok(())
    }
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name
        .split_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|value| matches!(value, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}

fn sync_tree(root: &Path) -> Result<(), PluginServiceError> {
    let mut directories = vec![root.to_path_buf()];
    let mut index = 0;
    while index < directories.len() {
        let directory = directories[index].clone();
        index += 1;
        for entry in fs::read_dir(&directory).map_err(|error| io_error(&directory, error))? {
            let path = entry.map_err(|error| io_error(&directory, error))?.path();
            if fs::symlink_metadata(&path)
                .map_err(|error| io_error(&path, error))?
                .is_dir()
            {
                directories.push(path);
            }
        }
    }
    for directory in directories.into_iter().rev() {
        sync_directory(&directory)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), PluginServiceError> {
    match File::open(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(_error) if cfg!(windows) => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}

fn io_error(path: &Path, error: std::io::Error) -> PluginServiceError {
    PluginServiceError::integration(format!("{}: {error}", path.display()))
}

struct StagingGuard {
    parent: PathBuf,
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn allocate(parent: &Path) -> Result<Self, PluginServiceError> {
        for _ in 0..8 {
            let path = parent.join(format!("{STAGING_PREFIX}{}", Uuid::now_v7()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        parent: parent.to_path_buf(),
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(&path, error)),
            }
        }
        Err(PluginServiceError::integration(
            "could not allocate Plugin Share staging directory",
        ))
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("staging path is armed")
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if path.parent() == Some(self.parent.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(STAGING_PREFIX))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}
