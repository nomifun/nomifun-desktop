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
use semver::Version;
use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MINIMUM_NODE_MAJOR: u64 = 22;
const PROBE_SCRIPT: &str = concat!(
    "process.stdout.write(JSON.stringify({",
    "name:process.release&&process.release.name,",
    "version:process.versions&&process.versions.node,",
    "execPath:process.execPath",
    "}))"
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaScriptWorkKind {
    PluginServiceHost,
}

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
    #[error("no compatible Node Runtime is available on PATH")]
    Unavailable,
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
    async fn acquire_use(
        &self,
        kind: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError>;

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError>;
}

/// Immutable authority established exactly once during application startup.
#[derive(Debug)]
pub struct RuntimeAuthority {
    runtime: Arc<ResolvedNodeRuntime>,
}

impl RuntimeAuthority {
    pub async fn discover() -> Result<Arc<Self>, JavaScriptRuntimeError> {
        let executable = which::which("node").map_err(|_| JavaScriptRuntimeError::Unavailable)?;
        Self::from_executable(executable).await
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
    async fn acquire_use(
        &self,
        _kind: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        Ok(RuntimeUseLease {
            runtime: Arc::clone(&self.runtime),
        })
    }

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        Ok(Some((*self.runtime).clone()))
    }
}

#[derive(Debug, Deserialize)]
struct ProbeIdentity {
    name: Option<String>,
    version: Option<String>,
    #[serde(rename = "execPath")]
    exec_path: String,
}

async fn probe(executable: &Path) -> Result<ResolvedNodeRuntime, JavaScriptRuntimeError> {
    let output = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut command = Command::new(executable);
        command.arg("-e").arg(PROBE_SCRIPT).kill_on_drop(true);
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
    if identity.name.as_deref() != Some("node") {
        return Err(JavaScriptRuntimeError::InvalidIdentity(
            "process.release.name is not node".into(),
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
        let first = authority
            .acquire_use(JavaScriptWorkKind::PluginServiceHost)
            .await
            .unwrap();
        let second = authority
            .acquire_use(JavaScriptWorkKind::PluginServiceHost)
            .await
            .unwrap();
        assert_eq!(first.runtime(), second.runtime());
        assert!(first.executable_path().is_absolute());
    }
}
