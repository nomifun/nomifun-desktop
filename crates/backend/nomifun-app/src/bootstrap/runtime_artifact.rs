//! Resolution and validation of the packaged Codex Runtime sidecar.
//!
//! The source-controlled Runtime contract owns protocol identity. Real
//! artifact digests remain in external post-build release locks; this module
//! observes the packaged Sidecar bytes immediately before process admission.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nomifun_agent_contracts::{
    RuntimeHelloPayload, RuntimeTarget, digest_bytes,
};
use nomifun_codex_runtime::{RuntimeHelloExpectation, RuntimeReleaseDescriptor};

#[derive(Clone, Debug)]
pub(crate) struct ResolvedRuntimeArtifact {
    pub executable: PathBuf,
    pub target_id: String,
    pub runtime_target: RuntimeTarget,
    pub executable_digest: nomifun_agent_contracts::DigestHex,
    pub release: RuntimeReleaseDescriptor,
    pub hello_expectation: RuntimeHelloExpectation,
}

pub(crate) fn resolve() -> Result<ResolvedRuntimeArtifact> {
    let release = RuntimeReleaseDescriptor::pinned_contract()
        .context("load the pinned Codex Runtime contract")?;
    let target_id = current_target_id();
    let runtime_target = release_target(&release, &target_id)?;
    let executable = resolve_executable(&target_id)?;
    let bytes = fs::read(&executable)
        .with_context(|| format!("read Codex Runtime sidecar {}", executable.display()))?;
    let executable_digest = digest_bytes(&bytes);

    let hello_path = resolve_hello_path(&executable)?;
    let hello: RuntimeHelloPayload = serde_json::from_slice(
        &fs::read(&hello_path)
            .with_context(|| format!("read Runtime hello metadata {}", hello_path.display()))?,
    )
    .with_context(|| format!("parse Runtime hello metadata {}", hello_path.display()))?;
    validate_hello(&release, &hello, &runtime_target)?;
    let hello_expectation = RuntimeHelloExpectation::from_payload(hello);

    Ok(ResolvedRuntimeArtifact {
        executable,
        target_id,
        runtime_target,
        executable_digest,
        release,
        hello_expectation,
    })
}

fn current_target_id() -> String {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows_desktop_x64".to_owned()
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos_desktop_arm64".to_owned()
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos_desktop_x64".to_owned()
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux_desktop_x64".to_owned()
    } else {
        "unsupported_local_target".to_owned()
    }
}

fn target_relative_path(target_id: &str) -> Option<&'static str> {
    match target_id {
        "windows_desktop_x64" => Some("runtime/windows/x64/nomifun-codex-runtime.exe"),
        "macos_desktop_arm64" => Some("runtime/macos/arm64/nomifun-codex-runtime"),
        "macos_desktop_x64" => Some("runtime/macos/x64/nomifun-codex-runtime"),
        "linux_desktop_x64" => Some("runtime/linux/x64/nomifun-codex-runtime"),
        _ => None,
    }
}

fn release_target(
    release: &RuntimeReleaseDescriptor,
    target_id: &str,
) -> Result<RuntimeTarget> {
    release
        .runtime_target_for_target(target_id)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn resolve_executable(target_id: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("NOMIFUN_CODEX_RUNTIME_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(dir) = std::env::var_os("NOMIFUN_CODEX_RUNTIME_DIR") {
        if let Some(relative) = target_relative_path(target_id) {
            candidates.push(PathBuf::from(dir).join(relative));
        }
    }
    if let Some(relative) = target_relative_path(target_id) {
        if let Ok(executable) = std::env::current_exe() {
            if let Some(parent) = executable.parent() {
                candidates.push(parent.join(relative));
                candidates.push(parent.join("resources").join(relative));
                if let Some(grandparent) = parent.parent() {
                    candidates.push(grandparent.join("Resources").join(relative));
                }
            }
        }
    }

    for candidate in candidates {
        if !candidate.is_absolute() {
            anyhow::bail!(
                "Codex Runtime path must be absolute: {}",
                candidate.display()
            );
        }
        let metadata = match fs::symlink_metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect Codex Runtime path {}", candidate.display()));
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            anyhow::bail!(
                "Codex Runtime path must be a regular non-symlink file: {}",
                candidate.display()
            );
        }
        return Ok(candidate);
    }

    anyhow::bail!(
        "Codex Runtime sidecar for {target_id} was not found; set NOMIFUN_CODEX_RUNTIME_PATH to the packaged executable"
    )
}

fn resolve_hello_path(executable: &Path) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("NOMIFUN_CODEX_RUNTIME_HELLO_PATH") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            anyhow::bail!(
                "NOMIFUN_CODEX_RUNTIME_HELLO_PATH is not a regular file: {}",
                path.display()
            );
        }
        return Ok(path);
    }
    let sidecar_path = PathBuf::from(format!("{}.hello.json", executable.display()));
    if sidecar_path.is_file() {
        return Ok(sidecar_path);
    }
    anyhow::bail!(
        "Runtime hello expectation metadata was not found beside {} or via NOMIFUN_CODEX_RUNTIME_HELLO_PATH",
        executable.display()
    )
}

fn validate_hello(
    release: &RuntimeReleaseDescriptor,
    hello: &RuntimeHelloPayload,
    runtime_target: &RuntimeTarget,
) -> Result<()> {
    if hello.runtime_release_digest != release.contract_digest {
        anyhow::bail!(
            "Runtime hello contract digest {} differs from pinned contract {}",
            hello.runtime_release_digest.as_ref(),
            release.contract_digest.as_ref()
        );
    }
    if hello.runtime_target != *runtime_target {
        anyhow::bail!(
            "Runtime hello target {} differs from release target {}",
            hello.runtime_target.as_ref(),
            runtime_target.as_ref()
        );
    }
    if hello.protocol_version != release.contract.protocol_version
        || hello.protocol_schema_digest != release.contract.protocol_schema_digest
        || hello.supported_profiles != release.contract.supported_profiles
        || hello.full_auto != release.contract.full_auto
        || hello.rpc_allowlist != release.contract.rpc_allowlist
    {
        anyhow::bail!("Runtime hello does not match the pinned release protocol contract");
    }
    RuntimeHelloExpectation::from_payload(hello.clone())
        .validate(hello)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_target_has_a_release_relative_path_on_supported_hosts() {
        let target = current_target_id();
        if target != "unsupported_local_target" {
            assert!(target_relative_path(&target).is_some());
        }
    }

    #[test]
    fn missing_sidecar_error_names_the_explicit_override() {
        let error = resolve_executable("windows_desktop_x64").unwrap_err();
        if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            assert!(error.to_string().contains("NOMIFUN_CODEX_RUNTIME_PATH"));
        }
    }

    #[test]
    fn target_matrix_rejects_unsupported_rows() {
        let release = RuntimeReleaseDescriptor::pinned_contract().unwrap();
        assert!(release_target(&release, "windows_arm64").is_err());
    }
}
