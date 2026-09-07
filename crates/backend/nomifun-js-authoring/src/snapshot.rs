use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use nomifun_agent_contracts::DigestHex;

use crate::canonical::canonical_digest;
use crate::dependency::DependencyRequestSet;
use crate::error::{AuthoringError, io_error};
use crate::model::{OperationCancellation, SourceStoreLimits, check_canceled};
use crate::path::NormalizedSourcePath;

const SOURCE_SNAPSHOT_FORMAT_VERSION: &str = "1.0.0";
const COPY_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFileDigest {
    normalized_relative_path: NormalizedSourcePath,
    digest: DigestHex,
    size_bytes: u64,
}

impl SourceFileDigest {
    pub fn new(
        normalized_relative_path: NormalizedSourcePath,
        digest: DigestHex,
        size_bytes: u64,
    ) -> Self {
        Self {
            normalized_relative_path,
            digest,
            size_bytes,
        }
    }

    pub fn normalized_relative_path(&self) -> &NormalizedSourcePath {
        &self.normalized_relative_path
    }

    pub fn digest(&self) -> &DigestHex {
        &self.digest
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceSnapshot {
    format_version: String,
    files: Vec<SourceFileDigest>,
    snapshot_digest: DigestHex,
}

impl SourceSnapshot {
    pub fn from_files(mut files: Vec<SourceFileDigest>) -> Result<Self, AuthoringError> {
        files.sort_by(|left, right| {
            left.normalized_relative_path
                .cmp(&right.normalized_relative_path)
        });
        let mut collisions = BTreeMap::new();
        for file in &files {
            validate_digest(&file.digest)?;
            if let Some(reason) = file.normalized_relative_path.fixed_profile_rejection() {
                return Err(AuthoringError::ForbiddenSourceEntry {
                    path: file.normalized_relative_path.to_string(),
                    reason: reason.into(),
                });
            }
            let key = file.normalized_relative_path.collision_key()?;
            if collisions
                .insert(key, file.normalized_relative_path.to_string())
                .is_some()
            {
                return Err(AuthoringError::PathCollision {
                    path: file.normalized_relative_path.to_string(),
                });
            }
        }
        let payload = SourceSnapshotPayload {
            format_version: SOURCE_SNAPSHOT_FORMAT_VERSION,
            files: &files,
        };
        Ok(Self {
            format_version: SOURCE_SNAPSHOT_FORMAT_VERSION.into(),
            snapshot_digest: canonical_digest(&payload)?,
            files,
        })
    }

    pub fn format_version(&self) -> &str {
        &self.format_version
    }

    pub fn files(&self) -> &[SourceFileDigest] {
        &self.files
    }

    pub fn digest(&self) -> &DigestHex {
        &self.snapshot_digest
    }
}

impl<'de> Deserialize<'de> for SourceSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SourceSnapshotWire::deserialize(deserializer)?;
        if wire.format_version != SOURCE_SNAPSHOT_FORMAT_VERSION {
            return Err(D::Error::custom(format!(
                "source snapshot format must be {SOURCE_SNAPSHOT_FORMAT_VERSION}"
            )));
        }
        let rebuilt = Self::from_files(wire.files).map_err(D::Error::custom)?;
        if rebuilt.snapshot_digest != wire.snapshot_digest {
            return Err(D::Error::custom(
                "source snapshot digest does not match canonical files",
            ));
        }
        Ok(rebuilt)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSnapshotWire {
    format_version: String,
    files: Vec<SourceFileDigest>,
    snapshot_digest: DigestHex,
}

#[derive(Serialize)]
struct SourceSnapshotPayload<'a> {
    format_version: &'static str,
    files: &'a [SourceFileDigest],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedSource {
    snapshot: SourceSnapshot,
    dependency_requests: DependencyRequestSet,
}

impl CapturedSource {
    pub fn snapshot(&self) -> &SourceSnapshot {
        &self.snapshot
    }

    pub fn dependency_requests(&self) -> &DependencyRequestSet {
        &self.dependency_requests
    }
}

pub(crate) fn capture_source_tree(
    source_root: &Path,
    limits: SourceStoreLimits,
    cancellation: &dyn OperationCancellation,
) -> Result<CapturedSource, AuthoringError> {
    check_canceled(cancellation)?;
    let metadata =
        fs::symlink_metadata(source_root).map_err(|error| io_error(source_root, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AuthoringError::UnsafeManagedPath {
            path: source_root.to_path_buf(),
        });
    }
    let canonical_root =
        fs::canonicalize(source_root).map_err(|error| io_error(source_root, error))?;
    let mut entries = Vec::new();
    let mut collisions = BTreeMap::new();
    let mut file_count = 0usize;
    let mut total_size = 0u64;

    for entry in WalkDir::new(&canonical_root)
        .follow_links(false)
        .min_depth(1)
    {
        check_canceled(cancellation)?;
        let entry = entry.map_err(|error| AuthoringError::Io {
            path: error
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| canonical_root.clone()),
            source: error
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("cannot walk source tree")),
        })?;
        let relative = entry.path().strip_prefix(&canonical_root).map_err(|_| {
            AuthoringError::UnsafeSourcePath {
                path: entry.path().display().to_string(),
                reason: "entry is outside the source root".into(),
            }
        })?;
        let normalized = NormalizedSourcePath::from_filesystem_path(relative)?;
        if let Some(reason) = normalized.fixed_profile_rejection() {
            return Err(AuthoringError::ForbiddenSourceEntry {
                path: normalized.to_string(),
                reason: reason.into(),
            });
        }
        let collision_key = normalized.collision_key()?;
        if collisions
            .insert(collision_key, normalized.to_string())
            .is_some()
        {
            return Err(AuthoringError::PathCollision {
                path: normalized.to_string(),
            });
        }

        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(AuthoringError::UnsafeSourcePath {
                path: normalized.to_string(),
                reason: "symbolic links are forbidden".into(),
            });
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(AuthoringError::UnsafeSourcePath {
                path: normalized.to_string(),
                reason: "only regular files and directories are allowed".into(),
            });
        }

        file_count += 1;
        if file_count > limits.max_file_count {
            return Err(AuthoringError::TooManyFiles {
                observed: file_count,
                limit: limits.max_file_count,
            });
        }
        if metadata.len() > limits.max_single_file_bytes {
            return Err(AuthoringError::FileTooLarge {
                path: normalized.to_string(),
                observed: metadata.len(),
                limit: limits.max_single_file_bytes,
            });
        }
        total_size = total_size.saturating_add(metadata.len());
        if total_size > limits.max_total_bytes {
            return Err(AuthoringError::TotalSizeExceeded {
                observed: total_size,
                limit: limits.max_total_bytes,
            });
        }

        let canonical =
            fs::canonicalize(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(AuthoringError::UnsafeSourcePath {
                path: normalized.to_string(),
                reason: "resolved file escapes the source root".into(),
            });
        }
        entries.push((normalized, canonical, metadata.len()));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut files = Vec::with_capacity(entries.len());
    let mut package_json = None;
    for (normalized, path, declared_size) in entries {
        let capture_bytes = normalized.as_str() == "package.json";
        let (digest, size, bytes) = hash_file(
            &path,
            &normalized,
            declared_size,
            if capture_bytes {
                Some(limits.max_package_json_bytes)
            } else {
                None
            },
            cancellation,
        )?;
        if capture_bytes {
            package_json = bytes;
        }
        files.push(SourceFileDigest::new(normalized, digest, size));
    }

    let dependency_requests = match package_json {
        Some(bytes) => DependencyRequestSet::from_package_json(&bytes)?,
        None => DependencyRequestSet::empty(),
    };
    Ok(CapturedSource {
        snapshot: SourceSnapshot::from_files(files)?,
        dependency_requests,
    })
}

fn hash_file(
    path: &Path,
    normalized: &NormalizedSourcePath,
    declared_size: u64,
    capture_limit: Option<u64>,
    cancellation: &dyn OperationCancellation,
) -> Result<(DigestHex, u64, Option<Vec<u8>>), AuthoringError> {
    if capture_limit.is_some_and(|limit| declared_size > limit) {
        return Err(AuthoringError::FileTooLarge {
            path: normalized.to_string(),
            observed: declared_size,
            limit: capture_limit.unwrap_or_default(),
        });
    }
    let mut file = File::open(path).map_err(|error| io_error(path, error))?;
    let opened = file.metadata().map_err(|error| io_error(path, error))?;
    if !opened.is_file() || opened.len() != declared_size {
        return Err(AuthoringError::SourceChanged {
            expected: declared_size.to_string(),
            observed: opened.len().to_string(),
        });
    }

    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut captured = capture_limit.map(|_| Vec::with_capacity(declared_size as usize));
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        check_canceled(cancellation)?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_error(path, error))?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
        if let Some(bytes) = &mut captured {
            bytes.extend_from_slice(&buffer[..read]);
        }
    }
    if size != declared_size {
        return Err(AuthoringError::SourceChanged {
            expected: declared_size.to_string(),
            observed: size.to_string(),
        });
    }
    Ok((
        DigestHex::from(hex::encode(hasher.finalize())),
        size,
        captured,
    ))
}

pub(crate) fn verify_source_file(
    source: &Path,
    target: &Path,
    expected: &SourceFileDigest,
    cancellation: &dyn OperationCancellation,
) -> Result<(), AuthoringError> {
    let metadata = fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AuthoringError::UnsafeSourcePath {
            path: expected.normalized_relative_path.to_string(),
            reason: "staged source input must remain a regular file".into(),
        });
    }
    let mut input = File::open(source).map_err(|error| io_error(source, error))?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(|error| io_error(target, error))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        check_canceled(cancellation)?;
        let read = input
            .read(&mut buffer)
            .map_err(|error| io_error(source, error))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| io_error(target, error))?;
        hasher.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    output.sync_all().map_err(|error| io_error(target, error))?;
    let digest = DigestHex::from(hex::encode(hasher.finalize()));
    if size != expected.size_bytes || digest != expected.digest {
        return Err(AuthoringError::SourceChanged {
            expected: expected.digest.as_ref().to_owned(),
            observed: digest.as_ref().to_owned(),
        });
    }
    Ok(())
}

fn validate_digest(digest: &DigestHex) -> Result<(), AuthoringError> {
    let value = digest.as_ref();
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(AuthoringError::InvalidDigest {
            value: value.to_owned(),
        })
    }
}

use std::io::Write;
