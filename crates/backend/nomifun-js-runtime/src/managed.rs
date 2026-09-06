use std::fs::File;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nomifun_agent_contracts::{
    NodeProbeDisposition, NodeRuntimeFingerprint, NodeRuntimeSourceKind,
    RECOMMENDED_NODE_LTS_MAJOR, RuntimeTarget,
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
        let target = approval.runtime_target.as_ref();
        let archive_profile = archive_profile(target)?;
        let release = self
            .resolve_release(approval.recommended_major, archive_profile)
            .await?;
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
        let extracted_root = tokio::task::spawn_blocking(move || {
            extract_windows_zip(&archive, &archive_file_name, &extraction_staging)
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

#[derive(Clone, Copy)]
struct ArchiveProfile {
    file_key: &'static str,
    archive_suffix: &'static str,
    executable_relative_path: &'static str,
}

fn archive_profile(target: &str) -> Result<ArchiveProfile, JavaScriptRuntimeError> {
    match target {
        "x86_64-pc-windows-msvc" => Ok(ArchiveProfile {
            file_key: "win-x64-zip",
            archive_suffix: "win-x64.zip",
            executable_relative_path: "node.exe",
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

fn extract_windows_zip(
    bytes: &[u8],
    archive_file_name: &str,
    staging: &Path,
) -> Result<PathBuf, JavaScriptRuntimeError> {
    if !archive_file_name.ends_with(".zip") {
        return Err(JavaScriptRuntimeError::InvalidArchive(
            "Windows managed Node archive must be zip".into(),
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| JavaScriptRuntimeError::InvalidArchive(error.to_string()))?;
    let extraction_root = staging.join("extract");
    std::fs::create_dir(&extraction_root)
        .map_err(|error| fs_error(&extraction_root, error))?;
    let mut top_level: Option<String> = None;
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

fn current_runtime_target() -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

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
        let error = extract_windows_zip(
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
        let root = extract_windows_zip(
            output.get_ref(),
            "node-v24.2.0-win-x64.zip",
            directory.path(),
        )
        .unwrap();
        assert!(root.join("node.exe").is_file());
    }
}
