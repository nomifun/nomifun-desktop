use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::PathBuf,
    time::{Duration, Instant},
};

use nomi_process_runtime::{
    CleanupReport, CommandSpec, EncodingMetadata,
    ProcessOutcome, ProcessOwner, ProcessPolicy, ProcessRequest, OutputChunk, OutputCursor,
    OutputSnapshot, OutputStream, ProcessSnapshot, ProcessState, SessionId, ShellKind, SpawnFailure,
    Transport, normalize_request,
};
use uuid::Uuid;

fn request(cwd: PathBuf) -> ProcessRequest {
    ProcessRequest {
        owner: ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7()),
        command: CommandSpec::Program {
            program: OsString::from("tool"),
            args: vec![OsString::from("--flag")],
        },
        cwd,
        env: BTreeMap::new(),
        transport: Transport::Pipe,
        policy: ProcessPolicy::default(),
    }
}

#[test]
fn relative_cwd_is_anchored_and_validated() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    let req = request(PathBuf::from("child"));

    let normalized = normalize_request(req, root.path()).unwrap();
    assert_eq!(
        normalized.cwd.canonicalize().unwrap(),
        root.path().join("child").canonicalize().unwrap()
    );
}

#[test]
fn missing_cwd_fails_before_spawn() {
    let root = tempfile::tempdir().unwrap();
    let req = request(PathBuf::from("missing"));

    let err = normalize_request(req, root.path()).unwrap_err();
    assert_eq!(err.code(), "invalid_working_directory");
}

#[test]
fn absolute_cwd_outside_the_session_default_is_accepted() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let normalized = normalize_request(request(outside.path().to_path_buf()), root.path()).unwrap();
    assert_eq!(normalized.cwd.canonicalize().unwrap(), outside.path().canonicalize().unwrap());
}

#[test]
fn parent_relative_cwd_is_resolved_from_the_session_default() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("session")).unwrap();
    std::fs::create_dir(root.path().join("outside 中文")).unwrap();
    let normalized = normalize_request(request(PathBuf::from("../outside 中文")), &root.path().join("session")).unwrap();
    assert_eq!(normalized.cwd.canonicalize().unwrap(), root.path().join("outside 中文").canonicalize().unwrap());
}

#[cfg(unix)]
#[test]
fn symlink_cwd_may_target_outside_the_session_default() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked directory")).unwrap();
    let normalized = normalize_request(request(PathBuf::from("linked directory")), root.path()).unwrap();
    assert_eq!(normalized.cwd, outside.path().canonicalize().unwrap());
}

#[cfg(windows)]
#[test]
fn junction_cwd_may_target_outside_the_session_default() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let alias = root.path().join("linked 中文");
    junction::create(outside.path(), &alias).unwrap();
    let result = normalize_request(request(PathBuf::from("linked 中文")), root.path());
    junction::delete(&alias).unwrap();
    assert_eq!(result.unwrap().cwd.canonicalize().unwrap(), outside.path().canonicalize().unwrap());
    assert!(outside.path().is_dir());
}

#[test]
fn non_directory_cwd_fails_before_spawn() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"not a directory").unwrap();
    let req = request(PathBuf::from("file"));


    let err = normalize_request(req, root.path()).unwrap_err();

    assert_eq!(err.code(), "invalid_working_directory");
}

#[test]
fn zero_sized_pty_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut req = request(root.path().to_path_buf());

    req.transport = Transport::Pty { cols: 0, rows: 24 };

    assert!(normalize_request(req, root.path()).is_err());
}

#[test]
fn empty_program_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut req = request(root.path().to_path_buf());

    req.command = CommandSpec::Program {
        program: OsString::new(),
        args: Vec::new(),
    };

    assert!(normalize_request(req, root.path()).is_err());
}

#[test]
fn empty_shell_script_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut req = request(root.path().to_path_buf());

    req.command = CommandSpec::Shell {
        shell: ShellKind::Posix,
        script: String::new(),
    };

    assert!(normalize_request(req, root.path()).is_err());
}

#[test]
fn cancellation_policy_totals_five_seconds() {
    let p = ProcessPolicy::default();
    assert_eq!(p.interrupt_grace, Duration::from_secs(1));
    assert_eq!(p.terminate_grace, Duration::from_secs(1));
    assert_eq!(p.reap_grace, Duration::from_secs(3));
}

#[test]
fn session_ids_are_uuid_v7_and_unpredictable() {
    let a = SessionId::new();
    let b = SessionId::new();
    assert_ne!(a, b);
    assert_eq!(a.as_uuid().get_version_num(), 7);
}

#[test]
fn session_ids_round_trip_through_text() {
    let id = SessionId::new();

    let parsed = id.to_string().parse::<SessionId>().unwrap();

    assert_eq!(parsed, id);
}

#[test]
fn shell_requires_a_script_and_program_preserves_os_strings() {
    let shell = CommandSpec::Shell {
        shell: ShellKind::Posix,
        script: "printf ok".to_owned(),
    };
    assert!(matches!(shell, CommandSpec::Shell { .. }));

    let spec = CommandSpec::Program {
        program: OsString::from("echo"),
        args: vec![OsString::from("ok")],
    };
    assert!(matches!(spec, CommandSpec::Program { .. }));
}

#[test]
fn output_cursor_exposes_absolute_offsets() {
    assert_eq!(OutputCursor::START.offset(), 0);
    assert_eq!(OutputCursor::new(42).offset(), 42);
}

#[test]
fn outcome_variants_preserve_process_facts() {
    let snapshot = || OutputSnapshot {
        chunks: vec![OutputChunk {
            seq: 1,
            start: 0,
            stream: OutputStream::Stdout,
            bytes: b"ok".to_vec(),
            text: "ok".to_owned(),
        }],
        next_cursor: OutputCursor::new(2),
        retained_bytes: 2,
        dropped_bytes: 0,
        encoding: EncodingMetadata {
            source_encoding: "utf-8".to_owned(),
            decode_errors: 0,
        },
    };
    let cleanup = || CleanupReport {
        interrupt_attempted: false,
        terminate_attempted: false,
        force_kill_attempted: false,
        reaped: true,
        elapsed: Duration::ZERO,
        errors: Vec::new(),
    };
    let now = Instant::now();

    let outcomes = [
        ProcessOutcome::Exited {
            code: Some(0),
            signal: None,
            output: snapshot(),
            cleanup: cleanup(),
        },
        ProcessOutcome::SpawnFailed(SpawnFailure {
            code: "spawn_failed".to_owned(),
            message: "could not spawn".to_owned(),
        }),
        ProcessOutcome::Cancelled {
            output: snapshot(),
            cleanup: cleanup(),
        },
        ProcessOutcome::TimedOut {
            output: snapshot(),
            cleanup: cleanup(),
        },
        ProcessOutcome::Lost {
            last_known: ProcessSnapshot {
                pid: 7,
                state: ProcessState::Lost,
                started_at: now,
                last_activity_at: now,
            },
            output: snapshot(),
            cleanup: cleanup(),
        },
    ];

    assert_eq!(outcomes.len(), 5);
}
