//! Startup-time materialization of the embedded builtin skills corpus to
//! `{data_dir}/builtin-skills/`. Gated on a `.version` file so repeat
//! starts with the same binary skip the rewrite.
//!
//! Algorithm:
//!   staging = data_dir/.builtin-skills.tmp (fresh each call)
//!   write all BUILTIN_SKILLS entries into staging
//!   write staging/.version ← binary version
//!   atomic rename(target → .builtin-skills.old, staging → target)
//!   best-effort remove .builtin-skills.old
//!
//! The atomic rename guarantees that concurrent backend processes, or a
//! crash mid-write, never observe a half-populated target — the old tree
//! stays in place until staging is fully ready.

use std::fs::OpenOptions;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs2::FileExt;
use include_dir::Dir;
use tracing::{info, warn};

use crate::error::ExtensionError;

const VERSION_FILE: &str = ".version";
const LOCK_FILE_NAME: &str = ".builtin-skills.lock";
const STAGING_DIR_NAME: &str = ".builtin-skills.tmp";
const OLD_DIR_NAME: &str = ".builtin-skills.old";
const STARTUP_FILE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
];

/// Decide whether to materialize based on the `.version` file, then do it.
/// Returns `true` if a write happened, `false` if the gate said "skip".
///
/// When `BUILTIN_SKILLS_ENV_VAR` is set and non-empty, the caller has
/// already routed `builtin_skills_dir` at the env-var path — this
/// function still runs but the gate will see whatever version the dev
/// tree has on disk (or missing, and materialize into that dev path,
/// which is wrong). Callers MUST check the env var before calling.
pub async fn materialize_if_needed(
    data_dir: &Path,
    corpus: &Dir<'static>,
    binary_version: &str,
) -> Result<bool, ExtensionError> {
    let target = data_dir.join(crate::constants::BUILTIN_SKILLS_DIR_NAME);

    if version_file_matches(&target, binary_version).await {
        info!(
            target = %target.display(),
            version = binary_version,
            "builtin skills up to date; skipping materialize"
        );
        return Ok(false);
    }

    info!(
        target = %target.display(),
        version = binary_version,
        "materializing embedded builtin skills"
    );
    let _guard = MaterializeLockGuard::acquire(data_dir).await?;
    if version_file_matches(&target, binary_version).await {
        info!(
            target = %target.display(),
            version = binary_version,
            "builtin skills up to date after materialize lock; skipping rewrite"
        );
        return Ok(false);
    }

    match materialize_embedded_builtin_skills_unlocked(data_dir, corpus, binary_version).await {
        Ok(()) => {}
        Err(e) if existing_builtin_skills_looks_usable(&target).await => {
            warn!(
                target = %target.display(),
                version = binary_version,
                error = %e,
                "failed to refresh builtin skills; continuing with existing tree"
            );
            return Ok(false);
        }
        Err(e) => return Err(e),
    }
    Ok(true)
}

/// Read `.version` and compare against the provided `binary_version`.
/// Returns `true` only on exact match. Missing file / IO error /
/// mismatch all return `false`.
async fn version_file_matches(target: &Path, binary_version: &str) -> bool {
    let version_path = target.join(VERSION_FILE);
    match tokio::fs::read_to_string(&version_path).await {
        Ok(s) => s == binary_version,
        Err(_) => false,
    }
}

/// Unconditional materialize: stage, write each file, atomic rename.
/// Exposed separately for tests that want to bypass the gate.
pub async fn materialize_embedded_builtin_skills(
    data_dir: &Path,
    corpus: &Dir<'static>,
    binary_version: &str,
) -> Result<(), ExtensionError> {
    let _guard = MaterializeLockGuard::acquire(data_dir).await?;
    materialize_embedded_builtin_skills_unlocked(data_dir, corpus, binary_version).await
}

async fn materialize_embedded_builtin_skills_unlocked(
    data_dir: &Path,
    corpus: &Dir<'static>,
    binary_version: &str,
) -> Result<(), ExtensionError> {
    let target = data_dir.join(crate::constants::BUILTIN_SKILLS_DIR_NAME);
    let staging = data_dir.join(STAGING_DIR_NAME);
    let old = data_dir.join(OLD_DIR_NAME);

    // Ensure data_dir itself exists before we try to write into it.
    tokio::fs::create_dir_all(data_dir).await?;

    // Clean any leftover staging from a previous crashed run.
    if staging.exists() {
        retry_startup_file_op("remove builtin skills staging dir", &staging, || {
            tokio::fs::remove_dir_all(&staging)
        })
        .await
        .map_err(|e| {
            ExtensionError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to clean staging dir {}: {e}", staging.display()),
            ))
        })?;
    }
    tokio::fs::create_dir_all(&staging).await?;

    write_dir_recursive(corpus, &staging).await?;

    let version_path = staging.join(VERSION_FILE);
    tokio::fs::write(&version_path, binary_version).await?;

    // Move existing target out of the way, then move staging in.
    commit_staging_dir(&target, &staging, &old).await
}

/// Atomically replace `target` with the fully-populated `staging` directory.
///
/// Moves any existing `target` aside to `old`, renames `staging` → `target`,
/// restores `old` on failure, then best-effort removes `old`. `old` must be a
/// sibling scratch path the caller owns. Reused by the builtin-skills and
/// superpowers baseline materialization paths and the superpowers download
/// installer, so the subtle Windows-safe rename/restore logic lives once here.
pub(crate) async fn commit_staging_dir(target: &Path, staging: &Path, old: &Path) -> Result<(), ExtensionError> {
    if target.exists() {
        if old.exists() {
            // Tolerate leftover .old from a crashed rename sequence.
            if let Err(e) = retry_startup_file_op("remove old materialize dir", old, || tokio::fs::remove_dir_all(old)).await
            {
                warn!(old = %old.display(), error = %e, "failed to remove stale old tree before refresh");
            }
        }
        retry_startup_file_op("rename target to old", target, || tokio::fs::rename(target, old)).await?;
    }

    if let Err(e) = retry_startup_file_op("rename staging to target", staging, || tokio::fs::rename(staging, target)).await
    {
        // Try to restore the original target so we don't leave the user with nothing.
        if old.exists()
            && let Err(restore_error) =
                retry_startup_file_op("restore old target", old, || tokio::fs::rename(old, target)).await
        {
            warn!(
                old = %old.display(),
                target = %target.display(),
                error = %restore_error,
                "failed to restore old tree after refresh failure"
            );
        }
        return Err(ExtensionError::Io(std::io::Error::new(
            e.kind(),
            format!(
                "atomic rename staging→target failed ({} → {}): {e}",
                staging.display(),
                target.display()
            ),
        )));
    }

    // Best-effort cleanup of the superseded tree.
    if old.exists()
        && let Err(e) = retry_startup_file_op("remove superseded dir", old, || tokio::fs::remove_dir_all(old)).await
    {
        warn!(old = %old.display(), error = %e, "failed to remove superseded tree (leaving behind)");
    }

    Ok(())
}

async fn existing_builtin_skills_looks_usable(target: &Path) -> bool {
    if !target.is_dir() {
        return false;
    }
    tokio::fs::metadata(target.join(VERSION_FILE))
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

pub(crate) async fn retry_startup_file_op<T, F, Fut>(operation: &str, path: &Path, mut op: F) -> std::io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    for (attempt, delay) in STARTUP_FILE_RETRY_DELAYS.iter().enumerate() {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) if is_retryable_startup_file_error(&e) => {
                warn!(
                    operation,
                    path = %path.display(),
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    raw_os_error = ?e.raw_os_error(),
                    error = %e,
                    "Startup file operation failed; retrying"
                );
                tokio::time::sleep(*delay).await;
            }
            Err(e) => return Err(e),
        }
    }
    op().await
}

fn is_retryable_startup_file_error(error: &std::io::Error) -> bool {
    match error.kind() {
        std::io::ErrorKind::Interrupted
        | std::io::ErrorKind::PermissionDenied
        | std::io::ErrorKind::TimedOut
        | std::io::ErrorKind::WouldBlock => true,
        _ => matches!(error.raw_os_error(), Some(5 | 32 | 33)),
    }
}

pub(crate) struct MaterializeLockGuard {
    file: std::fs::File,
}

impl MaterializeLockGuard {
    async fn acquire(data_dir: &Path) -> std::io::Result<Self> {
        Self::acquire_named(data_dir, LOCK_FILE_NAME).await
    }

    /// Acquire an exclusive file lock named `lock_name` under `data_dir`.
    /// Lets separate corpora (builtin skills vs superpowers) use distinct lock
    /// files so they don't needlessly serialize against each other.
    pub(crate) async fn acquire_named(data_dir: &Path, lock_name: &str) -> std::io::Result<Self> {
        let data_dir = data_dir.to_path_buf();
        let lock_name = lock_name.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&data_dir)?;
            let lock_path = data_dir.join(&lock_name);
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(lock_path)?;
            FileExt::lock_exclusive(&file)?;
            Ok(Self { file })
        })
        .await
        .map_err(|e| std::io::Error::other(format!("materialize lock task failed: {e}")))?
    }
}

impl Drop for MaterializeLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Recursively copy every file in an `include_dir::Dir` tree into `dest`.
/// Directories are created as needed. Files overwrite silently.
pub(crate) async fn write_dir_recursive(dir: &Dir<'static>, dest: &Path) -> Result<(), ExtensionError> {
    // The include_dir API is synchronous; we flatten into a Vec then
    // feed the writes through tokio::fs to stay off the reactor's thread
    // for big IO bursts.
    let mut stack: Vec<(&Dir<'static>, PathBuf)> = vec![(dir, dest.to_path_buf())];
    while let Some((d, prefix)) = stack.pop() {
        for file in d.files() {
            let rel = file.path();
            let out_path = prefix.join(rel.strip_prefix(d.path()).unwrap_or(rel));
            if let Some(parent) = out_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&out_path, file.contents()).await?;
        }
        for sub in d.dirs() {
            let sub_rel = sub.path();
            let sub_dest = prefix.join(sub_rel.strip_prefix(d.path()).unwrap_or(sub_rel));
            tokio::fs::create_dir_all(&sub_dest).await?;
            stack.push((sub, sub_dest));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn commit_staging_dir_replaces_existing_target() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        let staging = tmp.path().join("staging");
        let old = tmp.path().join("old");

        // Existing target with stale content that must be replaced.
        tokio::fs::create_dir_all(target.join("sub")).await.unwrap();
        tokio::fs::write(target.join("stale.txt"), b"old").await.unwrap();
        // Fully-populated staging.
        tokio::fs::create_dir_all(&staging).await.unwrap();
        tokio::fs::write(staging.join("fresh.txt"), b"new").await.unwrap();

        commit_staging_dir(&target, &staging, &old).await.unwrap();

        assert!(target.join("fresh.txt").is_file(), "fresh content moved in");
        assert!(!target.join("stale.txt").exists(), "stale content replaced");
        assert!(!staging.exists(), "staging consumed by rename");
        assert!(!old.exists(), "old scratch cleaned up");
    }

    #[tokio::test]
    async fn commit_staging_dir_creates_target_when_absent() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        let staging = tmp.path().join("staging");
        let old = tmp.path().join("old");
        tokio::fs::create_dir_all(&staging).await.unwrap();
        tokio::fs::write(staging.join("fresh.txt"), b"new").await.unwrap();

        commit_staging_dir(&target, &staging, &old).await.unwrap();

        assert!(target.join("fresh.txt").is_file());
        assert!(!staging.exists());
    }
}
