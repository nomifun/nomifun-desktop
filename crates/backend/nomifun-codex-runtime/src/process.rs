use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_contracts::DigestHex;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

use crate::credential::{CredentialHandleDescriptor, PreparedCredentialChannel};
use crate::error::RuntimeError;
use crate::release::RuntimeReleaseDescriptor;

const PINNED_APP_SERVER_ARGS: [&str; 3] = ["app-server", "--listen", "stdio://"];

#[derive(Clone)]
pub struct RuntimeProcessConfig {
    executable: PathBuf,
    working_directory: PathBuf,
    arguments: Vec<OsString>,
    environment: BTreeMap<OsString, OsString>,
    target_id: String,
    expected_executable_digest: Option<DigestHex>,
}

impl RuntimeProcessConfig {
    pub fn pinned_app_server(
        executable: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
        target_id: impl Into<String>,
        expected_executable_digest: DigestHex,
        release: &RuntimeReleaseDescriptor,
    ) -> Result<Self, RuntimeError> {
        let target_id = target_id.into();
        release.runtime_target_for_target(&target_id)?;
        let config = Self {
            executable: executable.into(),
            working_directory: working_directory.into(),
            arguments: PINNED_APP_SERVER_ARGS
                .into_iter()
                .map(OsString::from)
                .collect(),
            environment: BTreeMap::new(),
            target_id,
            expected_executable_digest: Some(expected_executable_digest),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn with_non_secret_environment(
        mut self,
        key: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Result<Self, RuntimeError> {
        let key = key.into();
        reject_secret_environment_key(&key)?;
        self.environment.insert(key, value.into());
        Ok(self)
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn expected_executable_digest(&self) -> Option<&DigestHex> {
        self.expected_executable_digest.as_ref()
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn validate_release(
        &self,
        release: &RuntimeReleaseDescriptor,
    ) -> Result<(), RuntimeError> {
        release.runtime_target_for_target(&self.target_id)?;
        Ok(())
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if !self.executable.is_absolute() {
            return Err(RuntimeError::Process(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "runtime executable path must be absolute",
            )));
        }
        let metadata = std::fs::symlink_metadata(&self.executable).map_err(|error| {
            RuntimeError::Process(std::io::Error::new(
                error.kind(),
                format!(
                    "runtime executable path could not be inspected: {} ({error})",
                    self.executable.display()
                ),
            ))
        })?;
        if metadata.file_type().is_symlink()
            || metadata_is_windows_reparse_point(&metadata)
            || !metadata.is_file()
        {
            return Err(RuntimeError::Process(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "runtime executable must be a regular non-symlink, non-reparse file: {}",
                    self.executable.display()
                ),
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = metadata.permissions().mode();
            if mode & 0o111 == 0 {
                return Err(RuntimeError::Process(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "runtime executable is not executable: {}",
                        self.executable.display()
                    ),
                )));
            }
            if mode & 0o022 != 0 {
                return Err(RuntimeError::Process(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "runtime executable must not be group/world writable: {}",
                        self.executable.display()
                    ),
                )));
            }
        }
        if !self.working_directory.is_absolute() {
            return Err(RuntimeError::Process(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "runtime working directory must be absolute",
            )));
        }
        if self.arguments
            != PINNED_APP_SERVER_ARGS
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        {
            return Err(RuntimeError::Protocol(
                "runtime process must use the pinned app-server stdio invocation".to_owned(),
            ));
        }
        for key in self.environment.keys() {
            reject_secret_environment_key(key)?;
        }
        if self.target_id.is_empty() {
            return Err(RuntimeError::Protocol(
                "runtime release target id is required".to_owned(),
            ));
        }
        if self.expected_executable_digest.is_none() {
            return Err(RuntimeError::Protocol(
                "runtime executable digest pin is required".to_owned(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn fixture(
        executable: PathBuf,
        working_directory: PathBuf,
        arguments: Vec<OsString>,
        environment: BTreeMap<OsString, OsString>,
    ) -> Self {
        Self {
            executable,
            working_directory,
            arguments,
            environment,
            target_id: "test_fixture".to_owned(),
            expected_executable_digest: None,
        }
    }
}

impl fmt::Debug for RuntimeProcessConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeProcessConfig")
            .field("executable", &self.executable)
            .field("working_directory", &self.working_directory)
            .field("arguments", &self.arguments)
            .field("target_id", &self.target_id)
            .field(
                "expected_executable_digest",
                &self.expected_executable_digest,
            )
            .field(
                "environment",
                &self
                    .environment
                    .keys()
                    .map(|key| (key, "<redacted>"))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(windows)]
fn metadata_is_windows_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_windows_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessTreeDisposeReport {
    pub root_pid: Option<u32>,
    pub stderr_bytes: u64,
    pub stderr_lines: u64,
}

pub struct SpawnedRuntimeProcess {
    pub process: ManagedRuntimeProcess,
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub credential_channel: PreparedCredentialChannel,
    pub credential_handle: CredentialHandleDescriptor,
}

pub struct ManagedRuntimeProcess {
    process: ManagedChildProcess,
    stderr_task: Option<JoinHandle<StderrSummary>>,
    root_pid: Option<u32>,
    disposed: Option<ProcessTreeDisposeReport>,
}

impl fmt::Debug for ManagedRuntimeProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedRuntimeProcess")
            .field("root_pid", &self.root_pid)
            .field("disposed", &self.disposed)
            .finish_non_exhaustive()
    }
}

impl ManagedRuntimeProcess {
    pub fn spawn(config: &RuntimeProcessConfig) -> Result<SpawnedRuntimeProcess, RuntimeError> {
        #[cfg(not(test))]
        {
            config.validate()?;
            verify_executable_digest(config)?;
        }
        #[cfg(test)]
        {
            if config.arguments
                == PINNED_APP_SERVER_ARGS
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            {
                config.validate()?;
                verify_executable_digest(config)?;
            } else {
                if !config.executable.is_absolute() || !config.working_directory.is_absolute() {
                    return Err(RuntimeError::Protocol(
                        "fixture process paths must be absolute".to_owned(),
                    ));
                }
                for key in config.environment.keys() {
                    reject_secret_environment_key(key)?;
                }
            }
        }

        let mut builder = ChildProcessBuilder::new(&config.executable);
        builder
            .args(&config.arguments)
            .current_dir(&config.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        sanitize_builder_environment(&mut builder);
        for (key, value) in &config.environment {
            builder.env(key, value);
        }
        let mut credential_channel = PreparedCredentialChannel::prepare(&mut builder)?;
        let mut process = builder.spawn_managed()?;
        let credential_handle = credential_channel.bind_child(&process)?;
        let root_pid = process.id();
        let stdin = process.stdin.take().ok_or_else(|| {
            RuntimeError::Process(std::io::Error::other(
                "runtime process stdin was not captured",
            ))
        })?;
        let stdout = process.stdout.take().ok_or_else(|| {
            RuntimeError::Process(std::io::Error::other(
                "runtime process stdout was not captured",
            ))
        })?;
        let stderr = process.stderr.take().ok_or_else(|| {
            RuntimeError::Process(std::io::Error::other(
                "runtime process stderr was not captured",
            ))
        })?;
        let stderr_task = tokio::spawn(drain_stderr(stderr));

        Ok(SpawnedRuntimeProcess {
            process: Self {
                process,
                stderr_task: Some(stderr_task),
                root_pid,
                disposed: None,
            },
            stdin,
            stdout,
            credential_channel,
            credential_handle,
        })
    }

    pub fn root_pid(&self) -> Option<u32> {
        self.root_pid
    }

    pub async fn dispose_tree(&mut self) -> Result<ProcessTreeDisposeReport, RuntimeError> {
        if let Some(report) = &self.disposed {
            return Ok(report.clone());
        }
        self.process.shutdown().await?;
        let stderr = match self.stderr_task.take() {
            Some(task) => task.await.unwrap_or_default(),
            None => StderrSummary::default(),
        };
        let report = ProcessTreeDisposeReport {
            root_pid: self.root_pid,
            stderr_bytes: stderr.bytes,
            stderr_lines: stderr.lines,
        };
        self.disposed = Some(report.clone());
        Ok(report)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct StderrSummary {
    bytes: u64,
    lines: u64,
}

async fn drain_stderr(mut stderr: ChildStderr) -> StderrSummary {
    let mut summary = StderrSummary::default();
    let mut buffer = [0_u8; 8192];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return summary,
            Ok(read) => {
                summary.bytes = summary
                    .bytes
                    .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                summary.lines = summary.lines.saturating_add(
                    u64::try_from(buffer[..read].iter().filter(|byte| **byte == b'\n').count())
                        .unwrap_or(u64::MAX),
                );
            }
        }
    }
}

fn reject_secret_environment_key(key: &OsStr) -> Result<(), RuntimeError> {
    let normalized = key.to_string_lossy().to_ascii_uppercase();
    if [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "CREDENTIAL",
        "API_KEY",
        "AUTH",
    ]
    .into_iter()
    .any(|needle| normalized.contains(needle))
    {
        return Err(RuntimeError::Credential(format!(
            "runtime environment key {key:?} may carry credential material"
        )));
    }
    Ok(())
}

fn verify_executable_digest(config: &RuntimeProcessConfig) -> Result<(), RuntimeError> {
    let expected = config.expected_executable_digest.as_ref().ok_or_else(|| {
        RuntimeError::Protocol("runtime executable digest pin is required".to_owned())
    })?;
    let mut file = std::fs::File::open(&config.executable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = DigestHex::from(hex_lower(&digest.finalize()));
    if &actual != expected {
        return Err(RuntimeError::ReleaseManifest(format!(
            "runtime executable digest mismatch: expected {}, got {}",
            expected.as_ref(),
            actual.as_ref()
        )));
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn sanitize_builder_environment(builder: &mut ChildProcessBuilder) {
    for (key, _) in std::env::vars_os() {
        builder.env_remove(key);
    }
    for key in [
        "APPDATA",
        "COMSPEC",
        "HOME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "LOCALAPPDATA",
        "LOGNAME",
        "NO_COLOR",
        "PATH",
        "PATHEXT",
        "SHELL",
        "SYSTEMROOT",
        "TEMP",
        "TERM",
        "TMP",
        "TMPDIR",
        "TZ",
        "USER",
        "USERNAME",
        "USERPROFILE",
        "WINDIR",
    ] {
        if let Some(value) = std::env::var_os(key) {
            builder.env(key, value);
        }
    }
    builder.env("NO_COLOR", "1").env("TERM", "dumb");
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::time::Duration;

    use super::*;

    #[test]
    fn pinned_config_rejects_relative_paths_and_secret_environment() {
        let cwd = std::env::current_dir().unwrap();
        let release = RuntimeReleaseDescriptor::pinned_contract().unwrap();
        assert!(
            RuntimeProcessConfig::pinned_app_server(
                "relative.exe",
                &cwd,
                "windows_desktop_x64",
                DigestHex::from("0".repeat(64)),
                &release,
            )
            .is_err()
        );
        let executable = std::env::current_exe().unwrap();
        let executable_digest = sha256_path(&executable);
        let release_pinned = RuntimeProcessConfig::pinned_app_server(
            &executable,
            &cwd,
            "windows_desktop_x64",
            executable_digest.clone(),
            &release,
        )
        .unwrap();
        release_pinned.validate_release(&release).unwrap();
        assert_eq!(
            release_pinned.expected_executable_digest(),
            Some(&executable_digest)
        );
        let digest = sha256_path(&executable);
        let config = RuntimeProcessConfig {
            executable,
            working_directory: cwd,
            arguments: PINNED_APP_SERVER_ARGS
                .into_iter()
                .map(OsString::from)
                .collect(),
            environment: BTreeMap::new(),
            target_id: "test_fixture".to_owned(),
            expected_executable_digest: Some(digest),
        };
        verify_executable_digest(&config).unwrap();
        let mut wrong_pin = config.clone();
        wrong_pin.expected_executable_digest = Some(DigestHex::from("wrong"));
        assert!(verify_executable_digest(&wrong_pin).is_err());
        assert!(
            config
                .clone()
                .with_non_secret_environment("OPENAI_API_KEY", "not-allowed")
                .is_err()
        );
        assert!(
            config
                .with_non_secret_environment("NOMIFUN_RUNTIME_LOG", "json")
                .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn pinned_config_rejects_a_sidecar_symlink() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("runtime");
        std::fs::write(&executable, b"not a real executable").unwrap();
        let link = root.path().join("runtime-link");
        std::os::unix::fs::symlink(&executable, &link).unwrap();
        let release = RuntimeReleaseDescriptor::pinned_contract().unwrap();

        let error = RuntimeProcessConfig::pinned_app_server(
            &link,
            root.path(),
            "macos_desktop_arm64",
            DigestHex::from("0".repeat(64)),
            &release,
        )
        .unwrap_err();
        assert!(error.to_string().contains("non-symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn pinned_config_rejects_a_non_executable_sidecar() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("runtime");
        std::fs::write(&executable, b"not executable").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o644)).unwrap();
        let release = RuntimeReleaseDescriptor::pinned_contract().unwrap();

        let error = RuntimeProcessConfig::pinned_app_server(
            &executable,
            root.path(),
            "macos_desktop_arm64",
            DigestHex::from("0".repeat(64)),
            &release,
        )
        .unwrap_err();
        assert!(error.to_string().contains("not executable"));
    }

    #[cfg(unix)]
    #[test]
    fn pinned_config_rejects_a_group_or_world_writable_sidecar() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("runtime");
        std::fs::write(&executable, b"runtime").unwrap();
        std::fs::set_permissions(
            &executable,
            std::fs::Permissions::from_mode(0o755 | 0o020),
        )
        .unwrap();
        let release = RuntimeReleaseDescriptor::pinned_contract().unwrap();

        let error = RuntimeProcessConfig::pinned_app_server(
            &executable,
            root.path(),
            "macos_desktop_arm64",
            DigestHex::from("0".repeat(64)),
            &release,
        )
        .unwrap_err();
        assert!(error.to_string().contains("group/world writable"));
    }

    #[cfg(windows)]
    #[test]
    fn pinned_config_rejects_a_windows_reparse_point() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("runtime-target");
        std::fs::create_dir(&target).unwrap();
        let alias = root.path().join("runtime-alias");
        junction::create(&target, &alias).unwrap();

        let metadata = std::fs::symlink_metadata(&alias).unwrap();
        assert!(metadata_is_windows_reparse_point(&metadata));
        let release = RuntimeReleaseDescriptor::pinned_contract().unwrap();
        let error = RuntimeProcessConfig::pinned_app_server(
            &alias,
            root.path(),
            "windows_desktop_x64",
            DigestHex::from("0".repeat(64)),
            &release,
        )
        .unwrap_err();
        assert!(error.to_string().contains("non-reparse"));

        junction::delete(&alias).unwrap();
    }

    fn sha256_path(path: &Path) -> DigestHex {
        let mut file = std::fs::File::open(path).unwrap();
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        DigestHex::from(hex_lower(&digest.finalize()))
    }

    #[test]
    fn inherited_environment_allowlist_contains_no_credential_names() {
        for key in [
            "APPDATA",
            "COMSPEC",
            "HOME",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "LOCALAPPDATA",
            "LOGNAME",
            "NO_COLOR",
            "PATH",
            "PATHEXT",
            "SHELL",
            "SYSTEMROOT",
            "TEMP",
            "TERM",
            "TMP",
            "TMPDIR",
            "TZ",
            "USER",
            "USERNAME",
            "USERPROFILE",
            "WINDIR",
        ] {
            reject_secret_environment_key(OsStr::new(key)).unwrap();
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_job_dispose_is_idempotent_and_cleans_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("runtime-process-tree.pids");
        let executable = std::env::current_exe().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let config = RuntimeProcessConfig::fixture(
            executable,
            cwd,
            vec![
                OsString::from("--exact"),
                OsString::from("process::tests::windows_process_tree_parent_fixture"),
                OsString::from("--nocapture"),
            ],
            BTreeMap::from([(
                OsString::from("NOMIFUN_CODEX_TREE_MARKER"),
                marker.as_os_str().to_owned(),
            )]),
        );
        let SpawnedRuntimeProcess {
            mut process,
            stdin: _stdin,
            stdout: _stdout,
            credential_channel: _credential_channel,
            credential_handle: _credential_handle,
        } = ManagedRuntimeProcess::spawn(&config).unwrap();
        let (root_pid, descendant_pid) = wait_for_pid_pair(&marker).await;
        assert_eq!(process.root_pid(), Some(root_pid));
        assert_ne!(root_pid, descendant_pid);

        let first = process.dispose_tree().await.unwrap();
        let second = process.dispose_tree().await.unwrap();
        assert_eq!(first, second);
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_tree_parent_fixture() {
        let Some(marker) = std::env::var_os("NOMIFUN_CODEX_TREE_MARKER") else {
            return;
        };
        let executable = std::env::current_exe().unwrap();
        let child = std::process::Command::new(executable)
            .args([
                "--exact",
                "process::tests::windows_process_tree_descendant_fixture",
                "--nocapture",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(
            PathBuf::from(marker),
            format!("{} {}", std::process::id(), child.id()),
        )
        .unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_tree_descendant_fixture() {
        if std::env::var_os("NOMIFUN_CODEX_TREE_MARKER").is_none() {
            return;
        }
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    #[cfg(windows)]
    async fn wait_for_pid_pair(path: &Path) -> (u32, u32) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(contents) = std::fs::read_to_string(path) {
                    let values = contents
                        .split_whitespace()
                        .map(str::parse::<u32>)
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    if values.len() == 2 {
                        return (values[0], values[1]);
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

}
