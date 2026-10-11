//! One immutable, process-wide JavaScript Runtime authority.
//!
//! Unified Plugin Core deliberately has no persisted Runtime selection,
//! candidate Runtime, hot switch, or Plugin-owned Runtime identity. Startup
//! resolves and verifies one Node executable; every Plugin Service lease points
//! at that exact executable for the lifetime of the process.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_process_runtime::ChildProcessBuilder;
use semver::Version;
use serde::Deserialize;
use thiserror::Error;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MINIMUM_NODE_MAJOR: u64 = 22;
const PROBE_SCRIPT: &str = concat!(
    "process.stdout.write(JSON.stringify({",
    "name:process.release&&process.release.name,",
    "version:process.versions&&process.versions.node,",
    "bun:process.versions&&process.versions.bun,",
    "execPath:process.execPath",
    "}))"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedNodeRuntime {
    executable_path: PathBuf,
    node_version: String,
}

impl ResolvedNodeRuntime {
    pub fn executable_path(&self) -> &Path {
        &self.executable_path
    }

    pub fn node_version(&self) -> &str {
        &self.node_version
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeUseLease {
    runtime: Arc<ResolvedNodeRuntime>,
}

impl RuntimeUseLease {
    pub fn runtime(&self) -> &ResolvedNodeRuntime {
        &self.runtime
    }

    pub fn executable_path(&self) -> &Path {
        self.runtime.executable_path()
    }
}

#[derive(Debug, Error)]
pub enum JavaScriptRuntimeError {
    #[error("no verified managed Node Runtime is available")]
    Unavailable,
    #[error("managed Node Runtime supply failed: {0}")]
    SupplyFailed(String),
    #[error("Node Runtime probe timed out: {0}")]
    ProbeTimeout(String),
    #[error("Node Runtime probe failed: {0}")]
    ProbeFailed(String),
    #[error("Node Runtime identity is invalid: {0}")]
    InvalidIdentity(String),
    #[error("Node Runtime {observed} is below the minimum supported major {minimum}")]
    UnsupportedVersion { observed: String, minimum: u64 },
}

#[async_trait]
pub trait CommittedRuntimeProvider: Send + Sync {
    async fn acquire_use(&self) -> Result<RuntimeUseLease, JavaScriptRuntimeError>;
}

/// Immutable authority established exactly once during application startup.
#[derive(Debug)]
pub struct RuntimeAuthority {
    runtime: Arc<ResolvedNodeRuntime>,
}

impl RuntimeAuthority {
    pub async fn discover() -> Result<Arc<Self>, JavaScriptRuntimeError> {
        let verified = nomifun_runtime::resolve_tool(nomifun_runtime::ToolId::Node)
            .await
            .map_err(|error| JavaScriptRuntimeError::SupplyFailed(error.to_string()))?;
        Ok(Arc::new(Self {
            runtime: Arc::new(ResolvedNodeRuntime {
                executable_path: verified.executable_path,
                node_version: verified.version,
            }),
        }))
    }

    pub async fn from_executable(
        executable: impl AsRef<Path>,
    ) -> Result<Arc<Self>, JavaScriptRuntimeError> {
        let executable = dunce::canonicalize(executable.as_ref())
            .map_err(|error| JavaScriptRuntimeError::ProbeFailed(error.to_string()))?;
        let runtime = Arc::new(probe(&executable).await?);
        Ok(Arc::new(Self { runtime }))
    }
}

#[async_trait]
impl CommittedRuntimeProvider for RuntimeAuthority {
    async fn acquire_use(&self) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        Ok(RuntimeUseLease {
            runtime: Arc::clone(&self.runtime),
        })
    }
}

#[derive(Debug, Deserialize)]
struct ProbeIdentity {
    name: Option<String>,
    version: Option<String>,
    bun: Option<String>,
    #[serde(rename = "execPath")]
    exec_path: String,
}

async fn probe(executable: &Path) -> Result<ResolvedNodeRuntime, JavaScriptRuntimeError> {
    let output = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut command = ChildProcessBuilder::clean_cli(executable);
        command
            .arg("-e")
            .arg(PROBE_SCRIPT)
            .env_remove("NODE_OPTIONS")
            .env_remove("BUN_OPTIONS");
        command.output().await
    })
    .await
    .map_err(|_| JavaScriptRuntimeError::ProbeTimeout(executable.display().to_string()))?
    .map_err(|error| JavaScriptRuntimeError::ProbeFailed(error.to_string()))?;
    if !output.status.success() {
        return Err(JavaScriptRuntimeError::ProbeFailed(
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(512)
                .collect(),
        ));
    }
    let identity: ProbeIdentity = serde_json::from_slice(&output.stdout)
        .map_err(|error| JavaScriptRuntimeError::InvalidIdentity(error.to_string()))?;
    if identity.name.as_deref() != Some("node") || identity.bun.is_some() {
        return Err(JavaScriptRuntimeError::InvalidIdentity(
            "executable is not genuine Node; Bun compatibility identity is not accepted".into(),
        ));
    }
    let version = identity.version.ok_or_else(|| {
        JavaScriptRuntimeError::InvalidIdentity("process.versions.node is missing".into())
    })?;
    let parsed = Version::parse(&version)
        .map_err(|error| JavaScriptRuntimeError::InvalidIdentity(error.to_string()))?;
    if parsed.major < MINIMUM_NODE_MAJOR {
        return Err(JavaScriptRuntimeError::UnsupportedVersion {
            observed: version,
            minimum: MINIMUM_NODE_MAJOR,
        });
    }
    let observed = dunce::canonicalize(&identity.exec_path)
        .map_err(|error| JavaScriptRuntimeError::InvalidIdentity(error.to_string()))?;
    if observed != executable {
        return Err(JavaScriptRuntimeError::InvalidIdentity(format!(
            "requested {} but Node reported {}",
            executable.display(),
            observed.display()
        )));
    }
    Ok(ResolvedNodeRuntime {
        executable_path: executable.to_path_buf(),
        node_version: parsed.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn startup_discovers_one_immutable_runtime() {
        let Ok(authority) = RuntimeAuthority::discover().await else {
            // Builders without Node can still compile this crate; Desktop
            // startup intentionally rejects the unavailable environment.
            return;
        };
        let first = authority.acquire_use().await.unwrap();
        let second = authority.acquire_use().await.unwrap();
        assert_eq!(first.runtime(), second.runtime());
        assert!(first.executable_path().is_absolute());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_cannot_claim_node_authority_through_a_node_alias() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let executable = tmp.path().join("node");
        std::fs::write(&executable, "#!/bin/sh\nprintf '%s' '{\"name\":\"node\",\"version\":\"24.3.0\",\"bun\":\"1.3.14\",\"execPath\":\"/fixture/bun\"}'\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = RuntimeAuthority::from_executable(executable)
            .await
            .unwrap_err();
        assert!(matches!(error, JavaScriptRuntimeError::InvalidIdentity(_)));
        assert!(error.to_string().contains("Bun compatibility identity"));
    }
}
