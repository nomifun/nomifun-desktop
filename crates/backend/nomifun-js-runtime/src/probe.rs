use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalErrorCode, DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION,
    JAVASCRIPT_SDK_CONTRACT_VERSION, MINIMUM_NODE_MAJOR, NodeProbeDisposition,
    NodeRuntimeFingerprint, NodeRuntimeProbeResult, NodeRuntimeSourceKind,
    RECOMMENDED_NODE_LTS_MAJOR, RuntimeInstallationId, RuntimeTarget,
};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::JavaScriptRuntimeError;

pub const NODE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub const NODE_PROBE_FAILED: &str = "NODE_PROBE_FAILED";
pub const NODE_IDENTITY_MISMATCH: &str = "NODE_IDENTITY_MISMATCH";
pub const NODE_VERSION_UNSUPPORTED: &str = "NODE_VERSION_UNSUPPORTED";
pub const NODE_TARGET_UNSUPPORTED: &str = "NODE_TARGET_UNSUPPORTED";

const NODE_PROBE_SCRIPT: &str = concat!(
    "process.stdout.write(JSON.stringify({",
    "name:process.release&&process.release.name,",
    "version:process.versions&&process.versions.node,",
    "platform:process.platform,",
    "arch:process.arch,",
    "execPath:process.execPath",
    "}))"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeProbeCandidate {
    pub source_kind: NodeRuntimeSourceKind,
    pub executable_path: PathBuf,
}

impl NodeProbeCandidate {
    pub fn new(
        source_kind: NodeRuntimeSourceKind,
        executable_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            source_kind,
            executable_path: executable_path.into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NodeDiscoveryRequest {
    pub explicit_path: Option<PathBuf>,
    pub saved_path: Option<PathBuf>,
    pub managed_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeResolution {
    pub selected: Option<NodeRuntimeFingerprint>,
    pub probes: Vec<NodeRuntimeProbeResult>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ObservedNodeIdentity {
    pub name: Option<String>,
    pub version: Option<String>,
    pub platform: String,
    pub arch: String,
    #[serde(rename = "execPath")]
    pub exec_path: String,
}

#[async_trait]
pub trait NodeProbeExecutor: Send + Sync {
    async fn observe(
        &self,
        executable_path: &Path,
    ) -> Result<ObservedNodeIdentity, JavaScriptRuntimeError>;
}

#[derive(Clone, Debug, Default)]
pub struct SystemNodeProbeExecutor;

#[async_trait]
impl NodeProbeExecutor for SystemNodeProbeExecutor {
    async fn observe(
        &self,
        executable_path: &Path,
    ) -> Result<ObservedNodeIdentity, JavaScriptRuntimeError> {
        let mut command = Command::new(executable_path);
        command
            .arg("-e")
            .arg(NODE_PROBE_SCRIPT)
            .kill_on_drop(true);
        let output = tokio::time::timeout(NODE_PROBE_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                JavaScriptRuntimeError::ProbeTimeout(executable_path.to_path_buf())
            })?
            .map_err(|error| JavaScriptRuntimeError::ExecutableUnavailable {
                path: executable_path.to_path_buf(),
                reason: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(JavaScriptRuntimeError::ExecutableUnavailable {
                path: executable_path.to_path_buf(),
                reason: String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .chars()
                    .take(512)
                    .collect(),
            });
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| JavaScriptRuntimeError::InvalidProbeOutput(error.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct NodeRuntimeResolver<E = SystemNodeProbeExecutor> {
    executor: E,
}

impl Default for NodeRuntimeResolver<SystemNodeProbeExecutor> {
    fn default() -> Self {
        Self {
            executor: SystemNodeProbeExecutor,
        }
    }
}

impl<E> NodeRuntimeResolver<E>
where
    E: NodeProbeExecutor,
{
    pub fn new(executor: E) -> Self {
        Self { executor }
    }

    pub fn discover(&self, request: NodeDiscoveryRequest) -> Vec<NodeProbeCandidate> {
        let mut candidates = Vec::new();
        if let Some(path) = request.explicit_path {
            candidates.push(NodeProbeCandidate::new(
                NodeRuntimeSourceKind::ManualPath,
                path,
            ));
        }
        if let Some(path) = request.saved_path {
            candidates.push(NodeProbeCandidate::new(
                NodeRuntimeSourceKind::ManualPath,
                path,
            ));
        }
        if let Ok(path) = which::which("node") {
            candidates.push(NodeProbeCandidate::new(
                NodeRuntimeSourceKind::ProcessPath,
                path,
            ));
        }
        candidates.extend(request.managed_paths.into_iter().map(|path| {
            NodeProbeCandidate::new(NodeRuntimeSourceKind::Managed, path)
        }));
        dedupe_candidates(candidates)
    }

    pub async fn resolve(
        &self,
        request: NodeDiscoveryRequest,
    ) -> Result<NodeResolution, JavaScriptRuntimeError> {
        let candidates = self.discover(request);
        let mut probes = Vec::with_capacity(candidates.len());
        let mut selected = None;
        for candidate in candidates {
            let result = self.probe(&candidate).await;
            if selected.is_none()
                && matches!(
                    result.disposition,
                    NodeProbeDisposition::CompatibleRecommended
                        | NodeProbeDisposition::CompatibleNonRecommended
                )
            {
                selected = result.fingerprint.clone();
            }
            probes.push(result);
        }
        Ok(NodeResolution { selected, probes })
    }

    pub async fn probe(&self, candidate: &NodeProbeCandidate) -> NodeRuntimeProbeResult {
        match self.probe_inner(candidate).await {
            Ok(fingerprint) => {
                let disposition =
                    if fingerprint.node_major == RECOMMENDED_NODE_LTS_MAJOR {
                        NodeProbeDisposition::CompatibleRecommended
                    } else {
                        NodeProbeDisposition::CompatibleNonRecommended
                    };
                NodeRuntimeProbeResult {
                    source_kind: candidate.source_kind,
                    executable_path: candidate.executable_path.display().to_string(),
                    disposition,
                    fingerprint: Some(fingerprint),
                    error_code: None,
                }
            }
            Err(error) => NodeRuntimeProbeResult {
                source_kind: candidate.source_kind,
                executable_path: candidate.executable_path.display().to_string(),
                disposition: NodeProbeDisposition::Incompatible,
                fingerprint: None,
                error_code: Some(CanonicalErrorCode::from(error_code(&error))),
            },
        }
    }

    async fn probe_inner(
        &self,
        candidate: &NodeProbeCandidate,
    ) -> Result<NodeRuntimeFingerprint, JavaScriptRuntimeError> {
        if !candidate.executable_path.is_absolute() {
            return Err(JavaScriptRuntimeError::RelativePath(
                candidate.executable_path.clone(),
            ));
        }
        let requested = canonicalize(&candidate.executable_path)?;
        let observed = self.executor.observe(&requested).await?;
        if observed.name.as_deref() != Some("node") {
            return Err(JavaScriptRuntimeError::InvalidProbeOutput(
                "process.release.name is not node".into(),
            ));
        }
        let version = observed.version.as_deref().ok_or_else(|| {
            JavaScriptRuntimeError::InvalidProbeOutput(
                "process.versions.node is missing".into(),
            )
        })?;
        let version = Version::parse(version).map_err(|error| {
            JavaScriptRuntimeError::InvalidProbeOutput(error.to_string())
        })?;
        if version.major < u64::from(MINIMUM_NODE_MAJOR) {
            return Err(JavaScriptRuntimeError::Contract(format!(
                "Node {} is below the minimum major {}",
                version, MINIMUM_NODE_MAJOR
            )));
        }
        let observed_path = canonicalize(Path::new(&observed.exec_path))?;
        if requested != observed_path {
            return Err(JavaScriptRuntimeError::ExecutableIdentityMismatch {
                requested: requested.display().to_string(),
                observed: observed_path.display().to_string(),
            });
        }
        let target = runtime_target(&observed.platform, &observed.arch)
            .ok_or_else(|| {
                JavaScriptRuntimeError::Contract(format!(
                    "unsupported Node target {}/{}",
                    observed.platform, observed.arch
                ))
            })?;
        let executable_digest = digest_file(&observed_path).await?;
        let runtime_installation_id =
            installation_id(&observed_path, &executable_digest);
        let fingerprint = NodeRuntimeFingerprint {
            runtime_installation_id,
            source_kind: candidate.source_kind,
            node_version: version.to_string().into(),
            node_major: u16::try_from(version.major).map_err(|_| {
                JavaScriptRuntimeError::InvalidProbeOutput(
                    "Node major exceeds u16".into(),
                )
            })?,
            runtime_target: RuntimeTarget::from(target),
            executable_digest,
            javascript_host_protocol_version:
                JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version:
                JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        };
        fingerprint
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        Ok(fingerprint)
    }
}

fn dedupe_candidates(candidates: Vec<NodeProbeCandidate>) -> Vec<NodeProbeCandidate> {
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            let key = candidate
                .executable_path
                .to_string_lossy()
                .replace('\\', "/");
            #[cfg(windows)]
            let key = key.to_ascii_lowercase();
            seen.insert(key)
        })
        .collect()
}

fn canonicalize(path: &Path) -> Result<PathBuf, JavaScriptRuntimeError> {
    dunce::canonicalize(path).map_err(|error| {
        JavaScriptRuntimeError::ExecutableUnavailable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }
    })
}

async fn digest_file(path: &Path) -> Result<DigestHex, JavaScriptRuntimeError> {
    let bytes = tokio::fs::read(path).await.map_err(|error| {
        JavaScriptRuntimeError::ExecutableUnavailable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }
    })?;
    Ok(DigestHex::from(hex::encode(Sha256::digest(bytes))))
}

fn installation_id(path: &Path, executable_digest: &DigestHex) -> RuntimeInstallationId {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().replace('\\', "/").as_bytes());
    hasher.update([0]);
    hasher.update(executable_digest.as_ref().as_bytes());
    let digest = hex::encode(hasher.finalize());
    RuntimeInstallationId::from(format!("node-{}", &digest[..32]))
}

pub fn runtime_target(platform: &str, arch: &str) -> Option<&'static str> {
    match (platform, arch) {
        ("win32", "x64") => Some("x86_64-pc-windows-msvc"),
        ("win32", "arm64") => Some("aarch64-pc-windows-msvc"),
        ("darwin", "arm64") => Some("aarch64-apple-darwin"),
        ("darwin", "x64") => Some("x86_64-apple-darwin"),
        ("linux", "x64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "arm64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

fn error_code(error: &JavaScriptRuntimeError) -> &'static str {
    match error {
        JavaScriptRuntimeError::ExecutableIdentityMismatch { .. } => {
            NODE_IDENTITY_MISMATCH
        }
        JavaScriptRuntimeError::Contract(message)
            if message.contains("below the minimum") =>
        {
            NODE_VERSION_UNSUPPORTED
        }
        JavaScriptRuntimeError::Contract(message)
            if message.contains("unsupported Node target") =>
        {
            NODE_TARGET_UNSUPPORTED
        }
        _ => NODE_PROBE_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;

    #[derive(Clone)]
    struct FakeExecutor {
        observations:
            Arc<Mutex<BTreeMap<PathBuf, Result<ObservedNodeIdentity, String>>>>,
    }

    #[async_trait]
    impl NodeProbeExecutor for FakeExecutor {
        async fn observe(
            &self,
            executable_path: &Path,
        ) -> Result<ObservedNodeIdentity, JavaScriptRuntimeError> {
            self.observations
                .lock()
                .await
                .remove(executable_path)
                .unwrap_or_else(|| Err("unregistered PATH candidate".into()))
                .map_err(JavaScriptRuntimeError::InvalidProbeOutput)
        }
    }

    #[test]
    fn target_mapping_is_explicit_and_closed() {
        assert_eq!(
            runtime_target("win32", "x64"),
            Some("x86_64-pc-windows-msvc")
        );
        assert_eq!(
            runtime_target("darwin", "arm64"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(
            runtime_target("linux", "x64"),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(runtime_target("freebsd", "x64"), None);
    }

    #[tokio::test]
    async fn resolver_prefers_explicit_then_saved_then_process_then_managed() {
        let directory = tempfile::tempdir().unwrap();
        let explicit = directory.path().join("explicit-node.exe");
        let saved = directory.path().join("saved-node.exe");
        tokio::fs::write(&explicit, b"explicit").await.unwrap();
        tokio::fs::write(&saved, b"saved").await.unwrap();
        let explicit = dunce::canonicalize(explicit).unwrap();
        let saved = dunce::canonicalize(saved).unwrap();
        let observations = BTreeMap::from([
            (
                explicit.clone(),
                Ok(ObservedNodeIdentity {
                    name: Some("node".into()),
                    version: Some("24.8.0".into()),
                    platform: "win32".into(),
                    arch: "x64".into(),
                    exec_path: explicit.display().to_string(),
                }),
            ),
            (
                saved.clone(),
                Ok(ObservedNodeIdentity {
                    name: Some("node".into()),
                    version: Some("22.0.0".into()),
                    platform: "win32".into(),
                    arch: "x64".into(),
                    exec_path: saved.display().to_string(),
                }),
            ),
        ]);
        let resolver = NodeRuntimeResolver::new(FakeExecutor {
            observations: Arc::new(Mutex::new(observations)),
        });
        let request = NodeDiscoveryRequest {
            explicit_path: Some(explicit.clone()),
            saved_path: Some(saved.clone()),
            managed_paths: Vec::new(),
        };
        let discovered = resolver.discover(request.clone());
        assert_eq!(discovered[0].executable_path, explicit);
        assert_eq!(discovered[1].executable_path, saved);

        let resolution = resolver
            .resolve(request)
            .await
            .unwrap();
        assert_eq!(
            resolution.selected.unwrap().node_major,
            RECOMMENDED_NODE_LTS_MAJOR
        );
        assert!(resolution.probes.len() >= 2);
        assert_eq!(
            resolution.probes[1].disposition,
            NodeProbeDisposition::CompatibleNonRecommended
        );
    }

    #[tokio::test]
    async fn unsupported_node_is_a_typed_incompatible_probe() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("node.exe");
        tokio::fs::write(&executable, b"node").await.unwrap();
        let executable = dunce::canonicalize(executable).unwrap();
        let resolver = NodeRuntimeResolver::new(FakeExecutor {
            observations: Arc::new(Mutex::new(BTreeMap::from([(
                executable.clone(),
                Ok(ObservedNodeIdentity {
                    name: Some("node".into()),
                    version: Some("20.0.0".into()),
                    platform: "win32".into(),
                    arch: "x64".into(),
                    exec_path: executable.display().to_string(),
                }),
            )]))),
        });
        let result = resolver
            .probe(&NodeProbeCandidate::new(
                NodeRuntimeSourceKind::ManualPath,
                executable,
            ))
            .await;
        assert_eq!(result.disposition, NodeProbeDisposition::Incompatible);
        assert_eq!(
            result.error_code.unwrap().as_ref(),
            NODE_VERSION_UNSUPPORTED
        );
    }

    #[tokio::test]
    async fn system_path_node_produces_a_valid_typed_probe_when_present() {
        let Ok(executable) = which::which("node") else {
            return;
        };
        let result = NodeRuntimeResolver::default()
            .probe(&NodeProbeCandidate::new(
                NodeRuntimeSourceKind::ProcessPath,
                executable,
            ))
            .await;
        result.validate().unwrap();
    }
}
