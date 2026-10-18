use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use thiserror::Error;
use uuid::Uuid;

/// Largest portable PTY dimension. Windows ConPTY uses signed 16-bit COORD
/// fields, so the shared contract must reject larger values before dispatch.
pub const MAX_PTY_DIMENSION: u16 = i16::MAX as u16;

use crate::{
    outcome::{CleanupReport, ProcessSnapshot, SessionId, SpawnFailure},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRequest {
    pub owner: ProcessOwner,
    pub command: CommandSpec,
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub transport: Transport,
    pub policy: ProcessPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandSpec {
    Program {
        program: OsString,
        args: Vec<OsString>,
    },
    Shell {
        shell: ShellKind,
        script: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Pipe,
    Pty { cols: u16, rows: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    PowerShell,
    /// PowerShell source passed literally after `-Command`, without an added
    /// prologue or exit-status wrapper.
    PowerShellLiteral,
    Posix,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProcessOwner {
    pub invocation_id: Uuid,
    pub call_id: Uuid,
}

impl ProcessOwner {
    pub const fn new(invocation_id: Uuid, call_id: Uuid) -> Self {
        Self {
            invocation_id,
            call_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessPolicy {
    pub output_limit_bytes: usize,
    pub lease: Duration,
    /// Whether the supervisor may retire this session after `lease` elapses
    /// without an API action or output activity.
    ///
    /// Interactive desktop terminals set this to `false` so a long machine
    /// sleep cannot race the idle-session reaper on resume.
    pub expire_on_idle: bool,
    pub deadline: Option<Instant>,
    pub interrupt_grace: Duration,
    pub terminate_grace: Duration,
    pub reap_grace: Duration,
}

impl Default for ProcessPolicy {
    fn default() -> Self {
        Self {
            output_limit_bytes: 4 * 1024 * 1024,
            lease: Duration::from_secs(15 * 60),
            expire_on_idle: true,
            deadline: None,
            interrupt_grace: Duration::from_secs(1),
            terminate_grace: Duration::from_secs(1),
            reap_grace: Duration::from_secs(3),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedProcessRequest {
    pub owner: ProcessOwner,
    pub command: CommandSpec,
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub transport: Transport,
    pub policy: ProcessPolicy,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("invalid working directory {path:?}: {reason}")]
    InvalidWorkingDirectory { path: PathBuf, reason: String },
    #[error("invalid command: {reason}")]
    InvalidCommand { reason: String },
    #[error("invalid transport: {reason}")]
    InvalidTransport { reason: String },
    #[error("process session {session_id} was not found")]
    SessionNotFound { session_id: SessionId },
    #[error("process session {session_id} belongs to a different owner")]
    OwnerMismatch { session_id: SessionId },
    #[error("process supervisor capacity is exhausted (max sessions: {max_sessions})")]
    CapacityExhausted { max_sessions: usize },
    #[error("process supervisor is shutting down")]
    SupervisorShuttingDown,
    #[error("process transport failure: {reason}")]
    Transport { reason: String },
    #[error("process I/O failure during {operation}: {reason}")]
    Io {
        operation: &'static str,
        reason: String,
    },
    #[error("process spawn failed: {failure:?}")]
    SpawnFailed { failure: SpawnFailure },
    #[error(
        "process start ownership was lost: failure={failure:?}, last_known={last_known:?}, cleanup={cleanup:?}"
    )]
    StartLost {
        failure: SpawnFailure,
        last_known: Option<ProcessSnapshot>,
        cleanup: CleanupReport,
    },
}

impl ProcessError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidWorkingDirectory { .. } => "invalid_working_directory",
            Self::InvalidCommand { .. } => "invalid_command",
            Self::InvalidTransport { .. } => "invalid_transport",
            Self::SessionNotFound { .. } => "session_not_found",
            Self::OwnerMismatch { .. } => "owner_mismatch",
            Self::CapacityExhausted { .. } => "capacity_exhausted",
            Self::SupervisorShuttingDown => "supervisor_shutting_down",
            Self::Transport { .. } => "transport",
            Self::Io { .. } => "io",
            Self::SpawnFailed { .. } => "spawn_failed",
            Self::StartLost { .. } => "start_lost",
        }
    }
}

pub fn normalize_request(
    request: ProcessRequest,
    session_cwd: &Path,
) -> Result<NormalizedProcessRequest, ProcessError> {
    let requested_cwd = if request.cwd.is_absolute() {
        request.cwd.clone()
    } else {
        session_cwd.join(&request.cwd)
    };
    let cwd = canonicalize_compatible(&requested_cwd).map_err(|error| {
        ProcessError::InvalidWorkingDirectory {
            path: requested_cwd.clone(),
            reason: error.to_string(),
        }
    })?;

    if !cwd.is_dir() {
        return Err(ProcessError::InvalidWorkingDirectory {
            path: requested_cwd,
            reason: "path is not a directory".to_owned(),
        });
    }

    if matches!(
        request.transport,
        Transport::Pty {
            cols: 0,
            ..
        } | Transport::Pty {
            rows: 0,
            ..
        }
    ) {
        return Err(ProcessError::InvalidTransport {
            reason: "PTY dimensions must be non-zero".to_owned(),
        });
    }

    match &request.command {
        CommandSpec::Program { program, .. } if program.is_empty() => {
            return Err(ProcessError::InvalidCommand {
                reason: "program must not be empty".to_owned(),
            });
        }
        CommandSpec::Shell { script, .. } if script.is_empty() => {
            return Err(ProcessError::InvalidCommand {
                reason: "shell script must not be empty".to_owned(),
            });
        }
        CommandSpec::Program { .. } | CommandSpec::Shell { .. } => {}
    }

    Ok(NormalizedProcessRequest {
        owner: request.owner,
        command: request.command,
        cwd,
        env: request.env,
        transport: request.transport,
        policy: request.policy,
    })
}

pub(crate) fn canonicalize_compatible(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;

    #[cfg(windows)]
    if let Some(simplified) = compatible_win32_candidate(&canonical)
        && same_win32_object(&canonical, &simplified)
    {
        return Ok(simplified);
    }

    Ok(canonical)
}

#[cfg(windows)]
fn compatible_win32_candidate(path: &Path) -> Option<PathBuf> {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Component, Prefix},
    };

    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let (candidate, remainder_offset) = match prefix.kind() {
        Prefix::VerbatimDisk(_) => (encoded.get(4..)?.to_vec(), 3),
        Prefix::VerbatimUNC(..) => {
            let mut candidate = vec![b'\\' as u16, b'\\' as u16];
            candidate.extend_from_slice(encoded.get(8..)?);
            (candidate, 2)
        }
        _ => return None,
    };
    // This is an optional compatibility spelling, never a path restriction.
    // Keep verbatim names when ordinary Win32 normalization or legacy launch
    // budgets could change their meaning. Inspect raw UTF-16 components;
    // Path::components() would already have hidden some dot components.
    if candidate.len() >= 260 || candidate.contains(&0) {
        return None;
    }
    let remainder = candidate.get(remainder_offset..)?;
    if remainder.split(|unit| *unit == b'\\' as u16).any(|component| {
        component == [b'.' as u16] || component == [b'.' as u16, b'.' as u16]
            || matches!(component.last(), Some(last) if *last == b'.' as u16 || *last == b' ' as u16)
            // FileIdInfo identifies the base file, not individual ADS streams.
            || component.contains(&(b':' as u16)) || component.contains(&(b'/' as u16))
    }) {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&candidate)))
}

#[cfg(windows)]
fn same_win32_object(original: &Path, compatible: &Path) -> bool {
    use std::{fs::File, mem, os::windows::{ffi::OsStrExt, io::{AsRawHandle, FromRawHandle}}, ptr};
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FILE_TYPE_DISK,
            FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx, GetFileType, OPEN_EXISTING,
        },
    };
    fn open(path: &Path) -> Option<File> {
        let mut name = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if name.contains(&0) { return None; }
        name.push(0);
        // Call Win32 directly: Rust filesystem helpers can add a verbatim
        // prefix and would not prove what the compatible spelling selects.
        let handle = unsafe { CreateFileW(name.as_ptr(), FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, ptr::null(), OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS, ptr::null_mut()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE { return None; }
        // SAFETY: this fresh valid handle has exactly one RAII owner.
        Some(unsafe { File::from_raw_handle(handle) })
    }
    fn identity(file: &File) -> Option<(u64, [u8; 16], bool)> {
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK { return None; }
        let mut id = FILE_ID_INFO::default();
        let mut standard = FILE_STANDARD_INFO::default();
        if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(), mem::size_of_val(&id) as u32) } == 0
            || unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileStandardInfo,
                (&mut standard as *mut FILE_STANDARD_INFO).cast(), mem::size_of_val(&standard) as u32) } == 0
        { return None; }
        Some((id.VolumeSerialNumber, id.FileId.Identifier, standard.Directory))
    }
    let Some(original) = open(original) else { return false; };
    let Some(compatible) = open(compatible) else { return false; };
    // Both handles stay alive through comparison. This proves equivalence at
    // selection time; it does not add an atomic cwd pin to process creation.
    matches!((identity(&original), identity(&compatible)), (Some(a), Some(b)) if a == b)
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
        mem,
        os::windows::{ffi::{OsStrExt, OsStringExt}, io::{AsRawHandle, FromRawHandle}},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CREATE_NEW, CreateDirectoryW, CreateFileW, DeleteFileW, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FILE_STANDARD_INFO, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx,
            OPEN_EXISTING, RemoveDirectoryW,
        },
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn raw_open(path: &Path, create: bool) -> File {
        let name = wide(path);
        let access = FILE_READ_ATTRIBUTES | GENERIC_READ | if create { GENERIC_WRITE } else { 0 };
        // This intentionally bypasses Rust's path conversion. The regression
        // concerns the different names CreateProcess/Win32 actually receive.
        let handle = unsafe { CreateFileW(name.as_ptr(), access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, ptr::null(),
            if create { CREATE_NEW } else { OPEN_EXISTING }, FILE_FLAG_BACKUP_SEMANTICS, ptr::null_mut()) };
        assert_ne!(handle, INVALID_HANDLE_VALUE, "raw fixture open {:?}: {}", path, std::io::Error::last_os_error());
        unsafe { File::from_raw_handle(handle) }
    }

    fn raw_identity(path: &Path) -> (u64, [u8; 16], bool) {
        let file = raw_open(path, false);
        let mut identity = FILE_ID_INFO::default();
        let mut standard = FILE_STANDARD_INFO::default();
        assert_ne!(unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
            (&mut identity as *mut FILE_ID_INFO).cast(), mem::size_of_val(&identity) as u32) }, 0);
        assert_ne!(unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(), mem::size_of_val(&standard) as u32) }, 0);
        (identity.VolumeSerialNumber, identity.FileId.Identifier, standard.Directory)
    }

    fn raw_bytes(path: &Path) -> Vec<u8> {
        let mut bytes = Vec::new(); raw_open(path, false).read_to_end(&mut bytes).unwrap(); bytes
    }

    fn ordinary_disk(path: &Path) -> PathBuf {
        let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
        assert_eq!(&encoded[..4], &[92, 92, 63, 92]);
        PathBuf::from(OsString::from_wide(&encoded[4..]))
    }

    struct Fixture {
        _root: tempfile::TempDir,
        root: PathBuf,
        files: Vec<PathBuf>,
        directories: Vec<PathBuf>,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            Self { root: fs::canonicalize(root.path()).unwrap(), _root: root, files: Vec::new(), directories: Vec::new() }
        }
        fn directory(&mut self, name: &str) -> PathBuf {
            let path = self.root.join(name); let name = wide(&path);
            assert_ne!(unsafe { CreateDirectoryW(name.as_ptr(), ptr::null()) }, 0);
            self.directories.push(path.clone()); path
        }
        fn file(&mut self, parent: &Path, name: &str, bytes: &[u8]) -> PathBuf {
            let path = parent.join(name); raw_open(&path, true).write_all(bytes).unwrap();
            self.files.push(path.clone()); path
        }
        fn cleanup(&mut self) -> Vec<std::io::Error> {
            let mut errors = Vec::new();
            for path in self.files.drain(..).rev() {
                if unsafe { DeleteFileW(wide(&path).as_ptr()) } == 0 { errors.push(std::io::Error::last_os_error()); }
            }
            for path in self.directories.drain(..).rev() {
                if unsafe { RemoveDirectoryW(wide(&path).as_ptr()) } == 0 { errors.push(std::io::Error::last_os_error()); }
            }
            errors
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { for error in self.cleanup() { eprintln!("raw path fixture cleanup failed: {error}"); } }
    }

    fn assert_literal_alias_preserved(suffix: &str, directory: bool) {
        let mut fixture = Fixture::new();
        let (plain, literal, plain_source, literal_source) = if directory {
            let plain = fixture.directory("target"); let literal = fixture.directory(&format!("target{suffix}"));
            let a = fixture.file(&plain, "marker", b"ordinary target");
            let b = fixture.file(&literal, "marker", b"literal target");
            (plain, literal, a, b)
        } else {
            let root = fixture.root.clone();
            let plain = fixture.file(&root, "target", b"ordinary target");
            let literal = fixture.file(&root, &format!("target{suffix}"), b"literal target");
            (plain.clone(), literal.clone(), plain, literal)
        };
        let plain_identity = raw_identity(&plain); let literal_identity = raw_identity(&literal);
        assert_ne!(plain_identity, literal_identity, "the fixture must contain two actual native objects");
        assert_eq!(raw_identity(&ordinary_disk(&literal)), plain_identity, "ordinary Win32 spelling must demonstrate the alias");
        assert_ne!(raw_bytes(&plain_source), raw_bytes(&literal_source));
        let canonical = fs::canonicalize(&literal).unwrap();
        let selected = canonicalize_compatible(&literal).unwrap();
        let selected_identity = raw_identity(&selected);
        let alias_equivalent = same_win32_object(&literal, &ordinary_disk(&literal));
        let cleanup = fixture.cleanup();
        assert!(cleanup.is_empty(), "all native fixture names must be removed before reporting a regression: {cleanup:?}");
        assert!(!alias_equivalent, "raw identity proof must reject the other actual target");
        assert_eq!(selected, canonical, "a literal native name must retain its verbatim spelling");
        assert_eq!(selected_identity, literal_identity, "the returned spelling must select the original native object");
    }

    #[test]
    fn literal_dot_directory_does_not_select_the_ordinary_alias() { assert_literal_alias_preserved(".", true); }
    #[test]
    fn literal_space_directory_does_not_select_the_ordinary_alias() { assert_literal_alias_preserved(" ", true); }
    #[test]
    fn literal_dot_file_does_not_select_the_ordinary_alias() { assert_literal_alias_preserved(".", false); }
    #[test]
    fn literal_space_file_does_not_select_the_ordinary_alias() { assert_literal_alias_preserved(" ", false); }
    #[test]
    fn ordinary_short_disk_file_and_directory_keep_the_same_native_object() {
        let mut fixture = Fixture::new(); let directory = fixture.directory("ordinary target");
        let file = fixture.file(&directory, "source", b"same native source");
        let results = [&directory, &file].map(|path| {
            let selected = canonicalize_compatible(path).unwrap();
            (selected.clone(), ordinary_disk(path), raw_identity(&selected), raw_identity(path))
        });
        assert!(fixture.cleanup().is_empty());
        for (selected, expected, actual_identity, original_identity) in results {
            assert_eq!(selected, expected); assert_eq!(actual_identity, original_identity);
        }
    }
    #[test]
    fn stream_identity_cannot_authorize_removing_the_verbatim_prefix() {
        let mut fixture = Fixture::new(); let root = fixture.root.clone();
        let base = fixture.file(&root, "source", b"base bytes");
        let stream = fixture.file(&root, "source:stream", b"stream bytes");
        let base_id = raw_identity(&base); let stream_id = raw_identity(&stream);
        let canonical = fs::canonicalize(&stream).unwrap();
        let selected = canonicalize_compatible(&stream).unwrap();
        let bytes = raw_bytes(&selected);
        assert!(fixture.cleanup().is_empty());
        assert_eq!(base_id, stream_id, "native FileIdInfo deliberately does not identify the stream");
        assert_eq!(selected, canonical); assert_eq!(bytes, b"stream bytes");
    }
    #[test]
    fn unc_compatible_candidate_preserves_share_root_and_native_unicode_units() {
        for source in [r"\\?\UNC\LocalHost\Share", r"\\?\UNC\LocalHost\Share\中文🙂 directory"] {
            let mut units = source.encode_utf16().collect::<Vec<_>>();
            // UTF-16 names need not be Unicode scalar strings. Candidate
            // preparation must never replace an unpaired native code unit.
            if source.ends_with("directory") { units.push(0xd800); }
            let path = PathBuf::from(OsString::from_wide(&units));
            let expected = [vec![92, 92], units[8..].to_vec()].concat();
            let selected = compatible_win32_candidate(&path).expect("a safe short UNC spelling must be a candidate");
            assert_eq!(selected.as_os_str().encode_wide().collect::<Vec<_>>(), expected);
        }
    }
    #[test]
    fn compatibility_candidate_preserves_special_and_long_path_semantics() {
        for source in [r"\\?\C:\literal.", r"\\?\C:\literal ", r"\\?\C:\parent.\child",
            r"\\?\C:\parent \child", r"\\?\C:\.\child", r"\\?\C:\..\child",
            r"\\?\C:\source:stream", r"\\?\UNC\server\share\source:stream",
            r"\\?\UNC\server\share\literal.", r"\\?\UNC\server\share\..\child",
            r"\\.\C:\source", r"\\?\Volume{00000000-0000-0000-0000-000000000000}\source"] {
            assert!(compatible_win32_candidate(Path::new(source)).is_none(), "no compatibility spelling for {source:?}");
        }
        let long = format!(r"\\?\C:\{}", "x".repeat(257));
        assert!(compatible_win32_candidate(Path::new(&long)).is_none());
        let boundary = format!(r"\\?\C:\{}\{}", "x".repeat(130), "y".repeat(125));
        assert_eq!(compatible_win32_candidate(Path::new(&boundary)).unwrap().as_os_str().encode_wide().count(), 259);
        assert!(compatible_win32_candidate(Path::new(&(boundary + "y"))).is_none());
    }
}
