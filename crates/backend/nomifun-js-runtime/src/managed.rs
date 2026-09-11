use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use flate2::read::GzDecoder;
use nomifun_agent_contracts::{
    DigestHex, NodeProbeDisposition, NodeRuntimeFingerprint,
    NodeRuntimeSourceKind, RECOMMENDED_NODE_LTS_MAJOR, RuntimeTarget,
    digest_payload,
};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    JavaScriptRuntimeError, NodeProbeCandidate, NodeRuntimeResolver,
};

const NODE_RELEASE_INDEX_URL: &str = "https://nodejs.org/dist/index.json";
const NODE_RELEASE_ROOT_URL: &str = "https://nodejs.org/dist";
const MAX_MANAGED_NODE_ARCHIVE_BYTES: u64 = 192 * 1024 * 1024;
const MAX_MANAGED_NODE_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
// Official Node distributions include npm/corepack documentation and can
// exceed 4,096 entries. Keep extraction bounded while allowing the complete
// release archive on every supported desktop target.
const MAX_MANAGED_NODE_FILES: usize = 16_384;
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedNodeDownloadApproval {
    pub approved_at_ms: i64,
    pub recommended_major: u16,
    pub runtime_target: RuntimeTarget,
}

impl ManagedNodeDownloadApproval {
    pub fn validate(&self) -> Result<(), JavaScriptRuntimeError> {
        if self.approved_at_ms <= 0 {
            return Err(JavaScriptRuntimeError::InvalidDownloadApproval(
                "approval time must be positive".into(),
            ));
        }
        if self.recommended_major != RECOMMENDED_NODE_LTS_MAJOR {
            return Err(JavaScriptRuntimeError::InvalidDownloadApproval(
                "approval must bind the current recommended LTS major".into(),
            ));
        }
        if self.runtime_target.as_ref() != current_runtime_target() {
            return Err(JavaScriptRuntimeError::InvalidDownloadApproval(
                "approval must bind the current host target".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedNodeRelease {
    pub version: Version,
    pub archive_file_name: String,
    pub archive_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ManagedRuntimeOffer {
    pub offer_digest: DigestHex,
    pub node_version: String,
    pub runtime_target: RuntimeTarget,
    pub archive_file_name: String,
    pub archive_sha256: String,
    pub archive_size_bytes: Option<u64>,
}

#[derive(serde::Serialize)]
struct ManagedRuntimeOfferDigestInput<'a> {
    node_version: &'a str,
    runtime_target: &'a RuntimeTarget,
    archive_file_name: &'a str,
    archive_sha256: &'a str,
}

impl ManagedRuntimeOffer {
    fn new(
        runtime_target: RuntimeTarget,
        release: ManagedNodeRelease,
    ) -> Result<Self, JavaScriptRuntimeError> {
        let node_version = release.version.to_string();
        let offer_digest = digest_payload(&ManagedRuntimeOfferDigestInput {
            node_version: &node_version,
            runtime_target: &runtime_target,
            archive_file_name: &release.archive_file_name,
            archive_sha256: &release.archive_sha256,
        })
        .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        Ok(Self {
            offer_digest,
            node_version,
            runtime_target,
            archive_file_name: release.archive_file_name,
            archive_sha256: release.archive_sha256,
            archive_size_bytes: None,
        })
    }

    fn release(&self) -> Result<ManagedNodeRelease, JavaScriptRuntimeError> {
        let version = Version::parse(&self.node_version)
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        let expected = Self::new(
            self.runtime_target.clone(),
            ManagedNodeRelease {
                version: version.clone(),
                archive_file_name: self.archive_file_name.clone(),
                archive_sha256: self.archive_sha256.clone(),
            },
        )?;
        if expected.offer_digest != self.offer_digest {
            return Err(JavaScriptRuntimeError::DownloadOfferStale);
        }
        Ok(ManagedNodeRelease {
            version,
            archive_file_name: self.archive_file_name.clone(),
            archive_sha256: self.archive_sha256.clone(),
        })
    }
}

#[async_trait]
pub trait ManagedRuntimeProvider: Send + Sync {
    async fn resolve_offer(
        &self,
    ) -> Result<ManagedRuntimeOffer, JavaScriptRuntimeError>;

    async fn install_offer(
        &self,
        offer: &ManagedRuntimeOffer,
    ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError>;

    async fn installed_executables(
        &self,
    ) -> Result<Vec<PathBuf>, JavaScriptRuntimeError>;
}

#[derive(Clone)]
pub struct ManagedNodeProvisioner {
    managed_root: PathBuf,
    client: reqwest::Client,
    resolver: NodeRuntimeResolver,
}

impl std::fmt::Debug for ManagedNodeProvisioner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedNodeProvisioner")
            .field("managed_root", &self.managed_root)
            .finish_non_exhaustive()
    }
}

impl ManagedNodeProvisioner {
    pub fn new(managed_root: impl Into<PathBuf>) -> Result<Self, JavaScriptRuntimeError> {
        let managed_root = managed_root.into();
        if !managed_root.is_absolute() {
            return Err(JavaScriptRuntimeError::RelativePath(managed_root));
        }
        Ok(Self {
            managed_root,
            client: nomifun_net::http_client(),
            resolver: NodeRuntimeResolver::default(),
        })
    }

    pub async fn provision(
        &self,
        approval: &ManagedNodeDownloadApproval,
    ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError> {
        approval.validate()?;
        let offer = self.resolve_offer().await?;
        self.install_offer(&offer).await
    }

    async fn provision_exact(
        &self,
        offer: &ManagedRuntimeOffer,
    ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError> {
        if offer.runtime_target.as_ref() != current_runtime_target() {
            return Err(JavaScriptRuntimeError::DownloadOfferStale);
        }
        let release = offer.release()?;
        if release.version.major != u64::from(RECOMMENDED_NODE_LTS_MAJOR) {
            return Err(JavaScriptRuntimeError::DownloadOfferStale);
        }
        let target = offer.runtime_target.as_ref();
        let archive_profile = archive_profile(target)?;
        let final_dir = self.managed_root.join(format!(
            "node-v{}-{}",
            release.version,
            target.replace(['/', '\\', ':'], "_")
        ));
        let final_executable = final_dir.join(archive_profile.executable_relative_path);
        if final_executable.is_file() {
            return self.probe_managed(&final_executable).await;
        }
        if final_dir.exists() {
            remove_managed_staging(&self.managed_root, &final_dir).await?;
        }

        tokio::fs::create_dir_all(&self.managed_root)
            .await
            .map_err(|error| fs_error(&self.managed_root, error))?;
        let staging = self.managed_root.join(format!(
            ".node-v{}-staging-{}-{}",
            release.version,
            std::process::id(),
            STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        tokio::fs::create_dir(&staging)
            .await
            .map_err(|error| fs_error(&staging, error))?;

        let result = self
            .download_extract_publish(
                &release,
                archive_profile,
                &staging,
                &final_dir,
            )
            .await;
        if result.is_err() {
            let _ = remove_managed_staging(&self.managed_root, &staging).await;
        }
        result?;

        match self.probe_managed(&final_executable).await {
            Ok(fingerprint) => Ok(fingerprint),
            Err(error) => {
                let _ = remove_managed_staging(&self.managed_root, &final_dir).await;
                Err(error)
            }
        }
    }

    async fn resolve_release(
        &self,
        recommended_major: u16,
        profile: ArchiveProfile,
    ) -> Result<ManagedNodeRelease, JavaScriptRuntimeError> {
        let index_bytes = download_bounded(&self.client, NODE_RELEASE_INDEX_URL).await?;
        let releases: Vec<NodeIndexRelease> = serde_json::from_slice(&index_bytes)
            .map_err(|error| JavaScriptRuntimeError::ReleaseMetadata(error.to_string()))?;
        let (version, version_label) =
            select_recommended_release(&releases, recommended_major, profile.file_key)?;
        let sums_url = format!(
            "{NODE_RELEASE_ROOT_URL}/{version_label}/SHASUMS256.txt"
        );
        let sums = download_bounded(&self.client, &sums_url).await?;
        let archive_file_name = format!(
            "node-{version_label}-{}",
            profile.archive_suffix
        );
        let archive_sha256 =
            parse_shasums(&sums, &archive_file_name).ok_or_else(|| {
                JavaScriptRuntimeError::ReleaseMetadata(format!(
                    "SHASUMS256.txt does not contain {archive_file_name}"
                ))
            })?;
        Ok(ManagedNodeRelease {
            version,
            archive_file_name,
            archive_sha256,
        })
    }

    async fn download_extract_publish(
        &self,
        release: &ManagedNodeRelease,
        profile: ArchiveProfile,
        staging: &Path,
        final_dir: &Path,
    ) -> Result<(), JavaScriptRuntimeError> {
        let version_label = format!("v{}", release.version);
        let archive_url = format!(
            "{NODE_RELEASE_ROOT_URL}/{version_label}/{}",
            release.archive_file_name
        );
        let archive = download_bounded(&self.client, &archive_url).await?;
        verify_archive_digest(&archive, &release.archive_sha256)?;

        let staging = staging.to_path_buf();
        let extraction_staging = staging.clone();
        let archive_file_name = release.archive_file_name.clone();
        let archive_format = profile.format;
        let extracted_root = tokio::task::spawn_blocking(move || {
            extract_archive(
                &archive,
                &archive_file_name,
                &extraction_staging,
                archive_format,
            )
        })
        .await
        .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))??;

        let executable = extracted_root.join(profile.executable_relative_path);
        if !executable.is_file() {
            return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                "archive is missing {}",
                profile.executable_relative_path
            )));
        }
        if final_dir.exists() {
            return Err(JavaScriptRuntimeError::ManagedFilesystem {
                path: final_dir.to_path_buf(),
                reason: "target appeared while the download was in progress".into(),
            });
        }
        tokio::fs::rename(&extracted_root, final_dir)
            .await
            .map_err(|error| fs_error(final_dir, error))?;
        remove_managed_staging(
            staging
                .parent()
                .ok_or_else(|| JavaScriptRuntimeError::ManagedFilesystem {
                    path: staging.to_path_buf(),
                    reason: "staging directory has no managed parent".into(),
                })?,
            &staging,
        )
        .await?;
        Ok(())
    }

    async fn probe_managed(
        &self,
        executable: &Path,
    ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError> {
        let result = self
            .resolver
            .probe(&NodeProbeCandidate::new(
                NodeRuntimeSourceKind::Managed,
                executable,
            ))
            .await;
        if !matches!(
            result.disposition,
            NodeProbeDisposition::CompatibleRecommended
                | NodeProbeDisposition::CompatibleNonRecommended
        ) {
            return Err(JavaScriptRuntimeError::Contract(format!(
                "managed Node probe failed with {:?}",
                result.error_code
            )));
        }
        result.fingerprint.ok_or_else(|| {
            JavaScriptRuntimeError::Contract(
                "compatible managed probe omitted its fingerprint".into(),
            )
        })
    }
}

#[async_trait]
impl ManagedRuntimeProvider for ManagedNodeProvisioner {
    async fn resolve_offer(
        &self,
    ) -> Result<ManagedRuntimeOffer, JavaScriptRuntimeError> {
        let runtime_target = RuntimeTarget::from(current_runtime_target());
        let profile = archive_profile(runtime_target.as_ref())?;
        let release = self
            .resolve_release(RECOMMENDED_NODE_LTS_MAJOR, profile)
            .await?;
        ManagedRuntimeOffer::new(runtime_target, release)
    }

    async fn install_offer(
        &self,
        offer: &ManagedRuntimeOffer,
    ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError> {
        self.provision_exact(offer).await
    }

    async fn installed_executables(
        &self,
    ) -> Result<Vec<PathBuf>, JavaScriptRuntimeError> {
        let mut executables = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.managed_root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(executables);
            }
            Err(error) => return Err(fs_error(&self.managed_root, error)),
        };
        let canonical_root = dunce::canonicalize(&self.managed_root)
            .map_err(|error| fs_error(&self.managed_root, error))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| fs_error(&self.managed_root, error))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| fs_error(&entry.path(), error))?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let executable = entry.path().join(managed_executable_name());
            if !executable.is_file() {
                continue;
            }
            let canonical = dunce::canonicalize(&executable)
                .map_err(|error| fs_error(&executable, error))?;
            if canonical.starts_with(&canonical_root) {
                executables.push(canonical);
            }
        }
        executables.sort();
        Ok(executables)
    }
}

#[derive(Clone, Copy)]
struct ArchiveProfile {
    file_key: &'static str,
    archive_suffix: &'static str,
    executable_relative_path: &'static str,
    format: ArchiveFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveFormat {
    Zip,
    TarGz,
}

fn archive_profile(target: &str) -> Result<ArchiveProfile, JavaScriptRuntimeError> {
    match target {
        "x86_64-pc-windows-msvc" => Ok(ArchiveProfile {
            file_key: "win-x64-zip",
            archive_suffix: "win-x64.zip",
            executable_relative_path: "node.exe",
            format: ArchiveFormat::Zip,
        }),
        "aarch64-pc-windows-msvc" => Ok(ArchiveProfile {
            file_key: "win-arm64-zip",
            archive_suffix: "win-arm64.zip",
            executable_relative_path: "node.exe",
            format: ArchiveFormat::Zip,
        }),
        "aarch64-apple-darwin" => Ok(ArchiveProfile {
            file_key: "osx-arm64-tar",
            archive_suffix: "darwin-arm64.tar.gz",
            executable_relative_path: "bin/node",
            format: ArchiveFormat::TarGz,
        }),
        "x86_64-apple-darwin" => Ok(ArchiveProfile {
            file_key: "osx-x64-tar",
            archive_suffix: "darwin-x64.tar.gz",
            executable_relative_path: "bin/node",
            format: ArchiveFormat::TarGz,
        }),
        "x86_64-unknown-linux-gnu" => Ok(ArchiveProfile {
            file_key: "linux-x64",
            archive_suffix: "linux-x64.tar.gz",
            executable_relative_path: "bin/node",
            format: ArchiveFormat::TarGz,
        }),
        "aarch64-unknown-linux-gnu" => Ok(ArchiveProfile {
            file_key: "linux-arm64",
            archive_suffix: "linux-arm64.tar.gz",
            executable_relative_path: "bin/node",
            format: ArchiveFormat::TarGz,
        }),
        _ => Err(JavaScriptRuntimeError::ManagedTargetUnsupported(
            target.to_owned(),
        )),
    }
}

#[derive(Clone, Debug, Deserialize)]
struct NodeIndexRelease {
    version: String,
    lts: serde_json::Value,
    files: Vec<String>,
}

fn select_recommended_release(
    releases: &[NodeIndexRelease],
    recommended_major: u16,
    file_key: &str,
) -> Result<(Version, String), JavaScriptRuntimeError> {
    releases
        .iter()
        .filter_map(|release| {
            if !release.lts.is_string()
                || !release.files.iter().any(|file| file == file_key)
            {
                return None;
            }
            let version = release.version.strip_prefix('v')?;
            let version = Version::parse(version).ok()?;
            (version.major == u64::from(recommended_major)).then_some((
                version,
                release.version.clone(),
            ))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
        .ok_or_else(|| {
            JavaScriptRuntimeError::ReleaseArchiveUnavailable(format!(
                "Node {recommended_major} LTS {file_key}"
            ))
        })
}

fn parse_shasums(bytes: &[u8], file_name: &str) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let observed_file = fields.next()?.trim_start_matches('*');
        (observed_file == file_name
            && digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| digest.to_owned())
    })
}

async fn download_bounded(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, JavaScriptRuntimeError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| JavaScriptRuntimeError::ReleaseMetadata(error.to_string()))?
        .error_for_status()
        .map_err(|error| JavaScriptRuntimeError::ReleaseMetadata(error.to_string()))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANAGED_NODE_ARCHIVE_BYTES)
    {
        return Err(JavaScriptRuntimeError::DownloadTooLarge {
            limit_bytes: MAX_MANAGED_NODE_ARCHIVE_BYTES,
        });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| JavaScriptRuntimeError::ReleaseMetadata(error.to_string()))?;
    if bytes.len() as u64 > MAX_MANAGED_NODE_ARCHIVE_BYTES {
        return Err(JavaScriptRuntimeError::DownloadTooLarge {
            limit_bytes: MAX_MANAGED_NODE_ARCHIVE_BYTES,
        });
    }
    Ok(bytes.to_vec())
}

fn verify_archive_digest(
    archive: &[u8],
    expected: &str,
) -> Result<(), JavaScriptRuntimeError> {
    let observed = hex::encode(Sha256::digest(archive));
    if observed != expected {
        return Err(JavaScriptRuntimeError::ArchiveDigestMismatch {
            expected: expected.to_owned(),
            observed,
        });
    }
    Ok(())
}

fn extract_archive(
    bytes: &[u8],
    archive_file_name: &str,
    staging: &Path,
    format: ArchiveFormat,
) -> Result<PathBuf, JavaScriptRuntimeError> {
    match format {
        ArchiveFormat::Zip => extract_zip(bytes, archive_file_name, staging),
        ArchiveFormat::TarGz => extract_tar_gz(bytes, archive_file_name, staging),
    }
}

fn extract_zip(
    bytes: &[u8],
    archive_file_name: &str,
    staging: &Path,
) -> Result<PathBuf, JavaScriptRuntimeError> {
    if !archive_file_name.ends_with(".zip") {
        return Err(JavaScriptRuntimeError::InvalidArchive(
            "managed Node zip archive must use the .zip suffix".into(),
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?;
    if archive.len() > MAX_MANAGED_NODE_FILES {
        return Err(JavaScriptRuntimeError::InvalidArchive(format!(
            "archive has {} entries; maximum is {MAX_MANAGED_NODE_FILES}",
            archive.len()
        )));
    }
    let extraction_root = staging.join("extract");
    std::fs::create_dir(&extraction_root)
        .map_err(|error| fs_error(&extraction_root, error))?;
    let mut top_level: Option<String> = None;
    let mut extracted_bytes = 0u64;
    let mut normalized_paths = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            JavaScriptRuntimeError::InvalidArchive(format!(
                "archive entry {} escapes the extraction root",
                entry.name()
            ))
        })?;
        let normalized = relative
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !normalized_paths.insert(normalized) {
            return Err(JavaScriptRuntimeError::InvalidArchive(
                "archive contains duplicate or case-colliding paths".into(),
            ));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                "archive entry {} is a symbolic link",
                entry.name()
            )));
        }
        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .ok_or_else(|| {
                JavaScriptRuntimeError::InvalidArchive(
                    "archive extracted size overflow".into(),
                )
            })?;
        if extracted_bytes > MAX_MANAGED_NODE_EXTRACTED_BYTES {
            return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                "archive extracts beyond {MAX_MANAGED_NODE_EXTRACTED_BYTES} bytes"
            )));
        }
        let first = relative.components().next().ok_or_else(|| {
            JavaScriptRuntimeError::InvalidArchive("empty archive entry".into())
        })?;
        let first = first.as_os_str().to_string_lossy().to_string();
        if let Some(expected) = &top_level {
            if expected != &first {
                return Err(JavaScriptRuntimeError::InvalidArchive(
                    "archive must contain exactly one top-level directory".into(),
                ));
            }
        } else {
            top_level = Some(first);
        }
        let output = extraction_root.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)
                .map_err(|error| fs_error(&output, error))?;
            continue;
        }
        let parent = output.parent().ok_or_else(|| {
            JavaScriptRuntimeError::InvalidArchive(
                "archive entry has no parent".into(),
            )
        })?;
        std::fs::create_dir_all(parent).map_err(|error| fs_error(parent, error))?;
        let mut file = File::create(&output).map_err(|error| fs_error(&output, error))?;
        std::io::copy(&mut entry, &mut file).map_err(|error| fs_error(&output, error))?;
        file.flush().map_err(|error| fs_error(&output, error))?;
    }
    let top_level = top_level.ok_or_else(|| {
        JavaScriptRuntimeError::InvalidArchive("archive is empty".into())
    })?;
    Ok(extraction_root.join(top_level))
}

fn extract_tar_gz(
    bytes: &[u8],
    archive_file_name: &str,
    staging: &Path,
) -> Result<PathBuf, JavaScriptRuntimeError> {
    if !archive_file_name.ends_with(".tar.gz") {
        return Err(JavaScriptRuntimeError::InvalidArchive(
            "managed Node tar archive must use the .tar.gz suffix".into(),
        ));
    }
    let extraction_root = staging.join("extract");
    std::fs::create_dir(&extraction_root)
        .map_err(|error| fs_error(&extraction_root, error))?;

    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(bytes)));
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    let entries = archive
        .entries()
        .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?;
    let mut top_level: Option<String> = None;
    let mut extracted_bytes = 0u64;
    let mut entry_count = 0usize;
    let mut normalized_paths = BTreeSet::new();

    for entry in entries {
        entry_count = entry_count.checked_add(1).ok_or_else(|| {
            JavaScriptRuntimeError::InvalidArchive("archive entry count overflow".into())
        })?;
        if entry_count > MAX_MANAGED_NODE_FILES {
            return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                "archive has more than {MAX_MANAGED_NODE_FILES} entries"
            )));
        }
        let mut entry = entry
            .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?;
        let relative = entry
            .path()
            .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?
            .into_owned();
        let path_components = archive_path_components(&relative)?;
        let normalized = path_components.join("/").to_ascii_lowercase();
        if !normalized_paths.insert(normalized) {
            return Err(JavaScriptRuntimeError::InvalidArchive(
                "archive contains duplicate or case-colliding paths".into(),
            ));
        }
        let first = path_components.first().cloned().ok_or_else(|| {
            JavaScriptRuntimeError::InvalidArchive("empty archive entry".into())
        })?;
        if let Some(expected) = &top_level {
            if expected != &first {
                return Err(JavaScriptRuntimeError::InvalidArchive(
                    "archive must contain exactly one top-level directory".into(),
                ));
            }
        } else {
            top_level = Some(first.clone());
        }

        let entry_type = entry.header().entry_type();
        if !(entry_type.is_file() || entry_type.is_dir() || entry_type.is_symlink()) {
            return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                "archive entry {} has an unsupported type",
                relative.display()
            )));
        }
        if entry_type.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?
                .ok_or_else(|| {
                    JavaScriptRuntimeError::InvalidArchive(format!(
                        "archive symlink {} has no target",
                        relative.display()
                    ))
                })?;
            validate_archive_symlink(&path_components, &target, &first)?;
        } else if entry_type.is_file() {
            extracted_bytes = extracted_bytes
                .checked_add(entry.size())
                .ok_or_else(|| {
                    JavaScriptRuntimeError::InvalidArchive(
                        "archive extracted size overflow".into(),
                    )
                })?;
            if extracted_bytes > MAX_MANAGED_NODE_EXTRACTED_BYTES {
                return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                    "archive extracts beyond {MAX_MANAGED_NODE_EXTRACTED_BYTES} bytes"
                )));
            }
        }
        let unpacked = entry
            .unpack_in(&extraction_root)
            .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?;
        if !unpacked {
            return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                "archive entry {} escapes the extraction root",
                relative.display()
            )));
        }
    }

    let top_level = top_level.ok_or_else(|| {
        JavaScriptRuntimeError::InvalidArchive("archive is empty".into())
    })?;
    Ok(extraction_root.join(top_level))
}

fn archive_path_components(path: &Path) -> Result<Vec<String>, JavaScriptRuntimeError> {
    let mut normalized = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    JavaScriptRuntimeError::InvalidArchive(
                        "archive path is not valid UTF-8".into(),
                    )
                })?;
                if value.is_empty() {
                    return Err(JavaScriptRuntimeError::InvalidArchive(
                        "archive path contains an empty component".into(),
                    ));
                }
                normalized.push(value.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(JavaScriptRuntimeError::InvalidArchive(format!(
                    "archive path {} escapes the extraction root",
                    path.display()
                )));
            }
        }
    }
    if normalized.is_empty() {
        return Err(JavaScriptRuntimeError::InvalidArchive(
            "archive path is empty".into(),
        ));
    }
    Ok(normalized)
}

fn validate_archive_symlink(
    entry_components: &[String],
    target: &Path,
    top_level: &str,
) -> Result<(), JavaScriptRuntimeError> {
    if target.is_absolute() {
        return Err(JavaScriptRuntimeError::InvalidArchive(
            "archive symlink target must be relative".into(),
        ));
    }
    let mut resolved = entry_components
        .get(..entry_components.len().saturating_sub(1))
        .unwrap_or_default()
        .to_vec();
    for component in target.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    JavaScriptRuntimeError::InvalidArchive(
                        "archive symlink target is not valid UTF-8".into(),
                    )
                })?;
                resolved.push(value.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if resolved.pop().is_none() {
                    return Err(JavaScriptRuntimeError::InvalidArchive(
                        "archive symlink target escapes the extraction root".into(),
                    ));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(JavaScriptRuntimeError::InvalidArchive(
                    "archive symlink target must be relative".into(),
                ));
            }
        }
    }
    if resolved.first().is_none_or(|value| value != top_level) {
        return Err(JavaScriptRuntimeError::InvalidArchive(
            "archive symlink target escapes its top-level directory".into(),
        ));
    }
    Ok(())
}

async fn remove_managed_staging(
    managed_root: &Path,
    candidate: &Path,
) -> Result<(), JavaScriptRuntimeError> {
    if candidate.parent() != Some(managed_root) {
        return Err(JavaScriptRuntimeError::ManagedFilesystem {
            path: candidate.to_path_buf(),
            reason: "cleanup target is outside the managed Runtime root".into(),
        });
    }
    match tokio::fs::remove_dir_all(candidate).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(fs_error(candidate, error)),
    }
}

fn fs_error(path: &Path, error: std::io::Error) -> JavaScriptRuntimeError {
    JavaScriptRuntimeError::ManagedFilesystem {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

pub fn current_runtime_target() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        "unsupported"
    }
}

fn managed_executable_name() -> &'static str {
    #[cfg(windows)]
    {
        "node.exe"
    }
    #[cfg(not(windows))]
    {
        "bin/node"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use zip::write::SimpleFileOptions;

    fn tar_gz_with_node_and_link(link_target: Option<&str>) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let node = b"node";
        let mut node_header = tar::Header::new_gnu();
        node_header.set_entry_type(tar::EntryType::Regular);
        node_header.set_mode(0o755);
        node_header.set_size(node.len() as u64);
        node_header.set_cksum();
        builder
            .append_data(
                &mut node_header,
                "node-v24.2.0-darwin-arm64/bin/node",
                Cursor::new(node),
            )
            .unwrap();
        if let Some(target) = link_target {
            let mut link_header = tar::Header::new_gnu();
            link_header.set_entry_type(tar::EntryType::Symlink);
            link_header.set_mode(0o777);
            link_header.set_size(0);
            link_header.set_link_name(target).unwrap();
            link_header.set_cksum();
            builder
                .append_data(
                    &mut link_header,
                    "node-v24.2.0-darwin-arm64/bin/npm",
                    Cursor::new([]),
                )
                .unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn archive_profiles_cover_every_supported_runtime_target() {
        let expected = [
            ("x86_64-pc-windows-msvc", "win-x64-zip", "win-x64.zip", ArchiveFormat::Zip),
            ("aarch64-pc-windows-msvc", "win-arm64-zip", "win-arm64.zip", ArchiveFormat::Zip),
            ("aarch64-apple-darwin", "osx-arm64-tar", "darwin-arm64.tar.gz", ArchiveFormat::TarGz),
            ("x86_64-apple-darwin", "osx-x64-tar", "darwin-x64.tar.gz", ArchiveFormat::TarGz),
            ("x86_64-unknown-linux-gnu", "linux-x64", "linux-x64.tar.gz", ArchiveFormat::TarGz),
            ("aarch64-unknown-linux-gnu", "linux-arm64", "linux-arm64.tar.gz", ArchiveFormat::TarGz),
        ];
        for (target, file_key, suffix, format) in expected {
            let profile = archive_profile(target).unwrap();
            assert_eq!(profile.file_key, file_key);
            assert_eq!(profile.archive_suffix, suffix);
            assert_eq!(profile.format, format);
        }
    }

    #[test]
    fn release_selection_requires_lts_major_and_archive_profile() {
        let releases = vec![
            NodeIndexRelease {
                version: "v24.2.0".into(),
                lts: serde_json::json!("Krypton"),
                files: vec!["win-x64-zip".into()],
            },
            NodeIndexRelease {
                version: "v24.3.0".into(),
                lts: serde_json::json!(false),
                files: vec!["win-x64-zip".into()],
            },
            NodeIndexRelease {
                version: "v22.9.0".into(),
                lts: serde_json::json!("Jod"),
                files: vec!["win-x64-zip".into()],
            },
        ];
        let (version, label) =
            select_recommended_release(&releases, 24, "win-x64-zip").unwrap();
        assert_eq!(version, Version::new(24, 2, 0));
        assert_eq!(label, "v24.2.0");
    }

    #[test]
    fn shasum_parser_matches_exact_file_only() {
        let digest = "a".repeat(64);
        let input = format!(
            "{digest}  node-v24.2.0-win-x64.zip\n{}  other.zip\n",
            "b".repeat(64)
        );
        assert_eq!(
            parse_shasums(input.as_bytes(), "node-v24.2.0-win-x64.zip"),
            Some(digest)
        );
        assert_eq!(parse_shasums(input.as_bytes(), "missing.zip"), None);
    }

    #[test]
    fn zip_extraction_rejects_path_traversal() {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            writer
                .start_file("../escape.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"escape").unwrap();
            writer.finish().unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let error = extract_zip(
            output.get_ref(),
            "node-v24.2.0-win-x64.zip",
            directory.path(),
        )
        .unwrap_err();
        assert!(matches!(error, JavaScriptRuntimeError::InvalidArchive(_)));
        assert!(!directory.path().parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn zip_extraction_keeps_one_top_level_directory() {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            writer
                .start_file(
                    "node-v24.2.0-win-x64/node.exe",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(b"node").unwrap();
            writer.finish().unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let root = extract_zip(
            output.get_ref(),
            "node-v24.2.0-win-x64.zip",
            directory.path(),
        )
        .unwrap();
        assert!(root.join("node.exe").is_file());
    }

    #[test]
    fn zip_extraction_rejects_windows_case_collisions() {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            for path in [
                "node-v24.2.0-win-x64/node.exe",
                "node-v24.2.0-win-x64/NODE.EXE",
            ] {
                writer
                    .start_file(path, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"node").unwrap();
            }
            writer.finish().unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        assert!(extract_zip(
            output.get_ref(),
            "node-v24.2.0-win-x64.zip",
            directory.path(),
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tar_gz_extraction_preserves_node_executable_and_safe_internal_link() {
        use std::os::unix::fs::PermissionsExt;

        let bytes = tar_gz_with_node_and_link(Some("node"));
        let directory = tempfile::tempdir().unwrap();
        let root = extract_tar_gz(
            &bytes,
            "node-v24.2.0-darwin-arm64.tar.gz",
            directory.path(),
        )
        .unwrap();
        let node = root.join("bin/node");
        assert!(node.is_file());
        assert_ne!(node.metadata().unwrap().permissions().mode() & 0o111, 0);
        assert_eq!(std::fs::read_link(root.join("bin/npm")).unwrap(), Path::new("node"));
    }

    #[test]
    fn tar_gz_extraction_rejects_symlink_escape() {
        let bytes = tar_gz_with_node_and_link(Some("../../../outside"));
        let directory = tempfile::tempdir().unwrap();
        let error = extract_tar_gz(
            &bytes,
            "node-v24.2.0-darwin-arm64.tar.gz",
            directory.path(),
        )
        .unwrap_err();
        assert!(matches!(error, JavaScriptRuntimeError::InvalidArchive(_)));
        assert!(!directory.path().parent().unwrap().join("outside").exists());
    }

    #[tokio::test]
    #[ignore = "requires access to the official Node.js release service"]
    async fn official_lts_download_installs_and_probes_the_current_host() {
        let directory = tempfile::tempdir().unwrap();
        let provisioner = ManagedNodeProvisioner::new(directory.path().join("managed")).unwrap();
        let fingerprint = provisioner
            .provision(&ManagedNodeDownloadApproval {
                approved_at_ms: 1,
                recommended_major: RECOMMENDED_NODE_LTS_MAJOR,
                runtime_target: RuntimeTarget::from(current_runtime_target()),
            })
            .await
            .unwrap();
        assert_eq!(fingerprint.node_major, RECOMMENDED_NODE_LTS_MAJOR);
        assert_eq!(fingerprint.runtime_target.as_ref(), current_runtime_target());
        assert_eq!(provisioner.installed_executables().await.unwrap().len(), 1);
    }
}
