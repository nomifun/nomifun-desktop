use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_common::{AgentType, AppError, ErrorChain};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::capability::cli_process::CliAgentProcess;

pub(crate) const AGENT_PROCESS_REGISTRY_RELATIVE_PATH: &str =
    nomifun_common::dataset_roots::AGENT_PROCESS_REGISTRY_FILE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RegisteredAgentProcess {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_group_id: Option<u32>,
    pub conversation_id: String,
    pub agent_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_preview: Option<String>,
    pub registered_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProcessRegistry {
    version: u32,
    processes: Vec<RegisteredAgentProcess>,
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            processes: Vec::new(),
        }
    }
}

static REGISTRY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn agent_process_registry_path(data_dir: &Path) -> PathBuf {
    data_dir.join(AGENT_PROCESS_REGISTRY_RELATIVE_PATH)
}

pub(crate) fn register_session_process(
    data_dir: &Path,
    process: Arc<CliAgentProcess>,
    conversation_id: impl Into<String>,
    agent_type: AgentType,
    backend: Option<String>,
    command_preview: Option<String>,
) -> Result<(), AppError> {
    let pid = process.pid();
    let process_group_id = process.process_group_id();
    // Observe exit through the independent watch channel.  Keeping `process`
    // in this task would create a lifecycle cycle: a cancelled construction
    // drops its real owner, but this watcher remains an owner waiting for a
    // process that nobody can now stop.
    let mut exit_rx = process.exit_receiver();
    let entry = RegisteredAgentProcess {
        pid,
        process_group_id,
        conversation_id: conversation_id.into(),
        agent_type: agent_type.serde_name().to_owned(),
        backend,
        command_preview,
        registered_at_ms: now_ms(),
    };

    register_agent_process(data_dir, entry).map_err(|e| {
        AppError::Internal(format!(
            "Failed to register agent process {pid} in runtime registry: {e}"
        ))
    })?;

    let data_dir = data_dir.to_path_buf();
    tokio::spawn(async move {
        if exit_rx.borrow().is_running() {
            let _ = exit_rx.changed().await;
        }
        let terminal = exit_rx.borrow().clone();
        if let Some(error) = terminal.failure() {
            // Retain the durable entry when the exact platform watchdog/Job
            // could not prove tree cleanup. PID/PGID liveness probes are not a
            // substitute: Windows has no Unix group to probe, and on Unix a
            // recycled PID can turn polling into false authority.
            warn!(
                pid,
                process_group_id,
                error,
                "Retaining failed agent process registry entry because process-tree cleanup was not proven"
            );
            return;
        }
        if terminal.exit_status().is_none() {
            warn!(
                pid,
                process_group_id,
                "Retaining agent process registry entry because exit monitor ended without proof"
            );
            return;
        }
        if let Err(e) = unregister_agent_process(&data_dir, pid) {
            warn!(
                pid,
                path = %agent_process_registry_path(&data_dir).display(),
                error = %ErrorChain(&e),
                "Failed to unregister exited agent process from runtime registry"
            );
        }
    });

    Ok(())
}

fn register_agent_process(data_dir: &Path, entry: RegisteredAgentProcess) -> io::Result<()> {
    with_registry_lock(|| {
        let path = agent_process_registry_path(data_dir);
        let mut registry = read_registry_file(&path)?;
        registry.processes.retain(|existing| existing.pid != entry.pid);
        registry.processes.push(entry);
        write_registry_file(&path, &registry)
    })
}

pub(crate) fn unregister_agent_process(data_dir: &Path, pid: u32) -> io::Result<()> {
    with_registry_lock(|| {
        let path = agent_process_registry_path(data_dir);
        let mut registry = read_registry_file(&path)?;
        let original_len = registry.processes.len();
        registry.processes.retain(|existing| existing.pid != pid);
        if registry.processes.len() == original_len {
            return Ok(());
        }
        write_registry_file(&path, &registry)
    })
}

fn read_registry_file(path: &Path) -> io::Result<ProcessRegistry> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse process registry {}: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ProcessRegistry::default()),
        Err(e) => Err(e),
    }
}

fn write_registry_file(path: &Path, registry: &ProcessRegistry) -> io::Result<()> {
    let payload = serde_json::to_vec_pretty(registry).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize process registry {}: {e}", path.display()),
        )
    })?;

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Process registry path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    // The temporary file must be a unique sibling of the destination:
    // `create_new` prevents following/reusing a pre-planted fixed `.tmp`
    // symlink, and a same-directory rename cannot cross filesystem boundaries.
    let (mut temp_file, temp_path) = create_registry_temp_file(parent)?;
    let mut temp_cleanup = OwnedRegistryTemp::new(temp_path);
    let write_result = temp_file
        .write_all(&payload)
        .and_then(|_| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = write_result {
        return Err(temp_cleanup.cleanup_after(error));
    }

    if let Err(error) = replace_registry_file_atomic(temp_cleanup.path(), path) {
        return Err(temp_cleanup.cleanup_after(error));
    }
    temp_cleanup.disarm();

    // On Unix, fsync the directory after rename so the new directory entry is
    // durable as well as the already-synced file contents. Windows uses
    // MOVEFILE_WRITE_THROUGH in `replace_registry_file_atomic`. A directory
    // sync failure is necessarily post-commit: reporting the whole write as
    // failed would prevent the exit watcher from being installed even though
    // the registry now contains the process. Keep the committed state and
    // surface the durability degradation as an operational warning instead.
    if let Err(error) = sync_registry_parent(parent) {
        warn!(
            path = %path.display(),
            parent = %parent.display(),
            error = %error,
            "Process registry was replaced, but its parent directory could not be synced"
        );
    }
    Ok(())
}

fn create_registry_temp_file(parent: &Path) -> io::Result<(File, PathBuf)> {
    const MAX_NAME_ATTEMPTS: usize = 8;

    for _ in 0..MAX_NAME_ATTEMPTS {
        let path = parent.join(format!(
            ".agent-process-registry.{}.tmp",
            uuid::Uuid::now_v7()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "Could not allocate a unique process registry temporary file in {}",
            parent.display()
        ),
    ))
}

struct OwnedRegistryTemp {
    path: PathBuf,
    armed: bool,
}

impl OwnedRegistryTemp {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn cleanup_after(&mut self, primary: io::Error) -> io::Error {
        self.armed = false;
        match fs::remove_file(&self.path) {
            Ok(()) => primary,
            Err(error) if error.kind() == io::ErrorKind::NotFound => primary,
            Err(cleanup_error) => io::Error::new(
                primary.kind(),
                format!(
                    "{primary}; additionally failed to remove process registry temporary file {}: \
                     {cleanup_error}",
                    self.path.display()
                ),
            ),
        }
    }
}

impl Drop for OwnedRegistryTemp {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn replace_registry_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn replace_registry_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    const RETRY_DELAYS_MS: &[u64] = &[0, 10, 25, 50, 100, 200];

    fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Path contains an embedded NUL: {}", path.display()),
            ));
        }
        wide.push(0);
        Ok(wide)
    }

    let source = wide_path(source)?;
    let target = wide_path(target)?;
    for (attempt, delay_ms) in RETRY_DELAYS_MS.iter().copied().enumerate() {
        if delay_ms != 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
        // SAFETY: both buffers are owned, NUL-terminated UTF-16 strings that
        // remain alive for this synchronous Win32 call.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } != 0
        {
            return Ok(());
        }

        let error = io::Error::last_os_error();
        let retryable = matches!(
            error.raw_os_error(),
            Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
        );
        if !retryable || attempt + 1 == RETRY_DELAYS_MS.len() {
            return Err(error);
        }
    }

    unreachable!("bounded MoveFileExW retry schedule is non-empty")
}

#[cfg(all(not(unix), not(windows)))]
fn replace_registry_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(unix)]
fn sync_registry_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_registry_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn with_registry_lock<T>(f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let _guard = REGISTRY_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    f()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_path_is_scoped_under_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = agent_process_registry_path(dir.path());
        assert_eq!(path, dir.path().join("agent-process-registry.json"));
    }

    #[test]
    fn unregister_is_idempotent_for_missing_pid() {
        let dir = tempfile::tempdir().unwrap();
        unregister_agent_process(dir.path(), 42).unwrap();
        let registry = read_registry_file(&agent_process_registry_path(dir.path())).unwrap();
        assert!(registry.processes.is_empty());
    }

    #[test]
    fn register_then_unregister_updates_registry_file() {
        let dir = tempfile::tempdir().unwrap();
        let entry = RegisteredAgentProcess {
            pid: 42,
            process_group_id: Some(42),
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000211".into(),
            agent_type: AgentType::Acp.serde_name().into(),
            backend: Some("codex".into()),
            command_preview: Some("codex-acp".into()),
            registered_at_ms: 123,
        };

        register_agent_process(dir.path(), entry.clone()).unwrap();
        let path = agent_process_registry_path(dir.path());
        let registry = read_registry_file(&path).unwrap();
        assert_eq!(registry.processes, vec![entry]);

        unregister_agent_process(dir.path(), 42).unwrap();
        let registry = read_registry_file(&path).unwrap();
        assert!(registry.processes.is_empty());
    }

    #[test]
    fn atomic_write_replaces_an_existing_registry() {
        let dir = tempfile::tempdir().unwrap();
        let path = agent_process_registry_path(dir.path());
        let old = ProcessRegistry {
            version: 1,
            processes: vec![test_process(11)],
        };
        let new = ProcessRegistry {
            version: 1,
            processes: vec![test_process(22)],
        };

        write_registry_file(&path, &old).unwrap();
        write_registry_file(&path, &new).unwrap();

        assert_eq!(read_registry_file(&path).unwrap(), new);
        assert_no_registry_temps(dir.path());
    }

    #[test]
    fn failed_atomic_replace_does_not_remove_existing_registry() {
        let dir = tempfile::tempdir().unwrap();
        let target = agent_process_registry_path(dir.path());
        let missing_source = dir.path().join("missing-registry-source.tmp");
        let original = b"original registry bytes";
        fs::write(&target, original).unwrap();

        replace_registry_file_atomic(&missing_source, &target).unwrap_err();

        assert_eq!(fs::read(&target).unwrap(), original);
    }

    #[test]
    fn failed_write_cleans_its_owned_temp_without_touching_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = agent_process_registry_path(dir.path());
        fs::create_dir(&target).unwrap();
        let registry = ProcessRegistry {
            version: 1,
            processes: vec![test_process(33)],
        };

        write_registry_file(&target, &registry).unwrap_err();

        assert!(target.is_dir());
        assert_no_registry_temps(dir.path());
    }

    fn test_process(pid: u32) -> RegisteredAgentProcess {
        RegisteredAgentProcess {
            pid,
            process_group_id: Some(pid),
            conversation_id: format!("conversation-{pid}"),
            agent_type: AgentType::Acp.serde_name().into(),
            backend: Some("codex".into()),
            command_preview: Some(format!("codex-acp-{pid}")),
            registered_at_ms: u64::from(pid),
        }
    }

    fn assert_no_registry_temps(parent: &Path) {
        let temp_names = fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| {
                let name = name.to_string_lossy();
                name.starts_with(".agent-process-registry.") && name.ends_with(".tmp")
            })
            .collect::<Vec<_>>();
        assert!(
            temp_names.is_empty(),
            "owned temporary files were not cleaned: {temp_names:?}"
        );
    }
}
