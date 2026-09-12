use super::*;
use russh_sftp::{protocol::*, server};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct FileState {
    bytes: Vec<u8>,
    permissions: u32,
}

struct ServerState {
    files: HashMap<String, FileState>,
    reject_rename: bool,
    reject_setstat: bool,
    collide_on_open: bool,
    removed: Vec<String>,
    no_posix_rename: bool,
    fsync: bool,
    reject_fsync: bool,
    fsync_calls: usize,
    small_limits: bool,
    reject_stat: bool,
    reject_close: bool,
    reject_write: bool,
    sessions: usize,
    dropped_sessions: usize,
    created_modes: Vec<u32>,
    write_chunks: Vec<usize>,
    closed_handles: usize,
    hide_size: bool,
    directory_batches: usize,
    stat_delay: std::time::Duration,
    write_delay: std::time::Duration,
    write_gate: Option<Arc<tokio::sync::Semaphore>>,
    write_started: Arc<tokio::sync::Notify>,
}

impl Default for ServerState {
    fn default() -> Self {
        Self {
            files: HashMap::from([(
                "/target".into(),
                FileState {
                    bytes: b"original".to_vec(),
                    permissions: 0o600,
                },
            )]),
            reject_rename: false,
            reject_setstat: false,
            collide_on_open: false,
            removed: Vec::new(),
            no_posix_rename: false,
            fsync: false,
            reject_fsync: false,
            fsync_calls: 0,
            small_limits: false,
            reject_stat: false,
            reject_close: false,
            reject_write: false,
            sessions: 0,
            dropped_sessions: 0,
            created_modes: Vec::new(),
            write_chunks: Vec::new(),
            closed_handles: 0,
            hide_size: false,
            directory_batches: 0,
            stat_delay: std::time::Duration::ZERO,
            write_delay: std::time::Duration::ZERO,
            write_gate: None,
            write_started: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

struct MemoryServer(Arc<Mutex<ServerState>>);

impl Drop for MemoryServer {
    fn drop(&mut self) {
        self.0.lock().unwrap().dropped_sessions += 1;
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: "en".into(),
    }
}

impl server::Handler for MemoryServer {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, StatusCode> {
        let mut version = Version::new();
        let mut state = self.0.lock().unwrap();
        state.sessions += 1;
        if state.fsync {
            version
                .extensions
                .insert("fsync@openssh.com".into(), "1".into());
        }
        if state.small_limits {
            version
                .extensions
                .insert("limits@openssh.com".into(), "1".into());
        }
        if !state.no_posix_rename {
            version
                .extensions
                .insert("posix-rename@openssh.com".into(), "1".into());
        }
        Ok(version)
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let delay = self.0.lock().unwrap().stat_delay;
        tokio::time::sleep(delay).await;
        let state = self.0.lock().unwrap();
        if state.reject_stat {
            return Err(StatusCode::PermissionDenied);
        }
        let file = state.files.get(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes {
                size: (!state.hide_size).then_some(file.bytes.len() as u64),
                permissions: Some(file.permissions),
                ..Default::default()
            },
        })
    }

    async fn open(
        &mut self,
        id: u32,
        path: String,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        let mut state = self.0.lock().unwrap();
        if flags.contains(OpenFlags::READ) && !flags.contains(OpenFlags::WRITE) {
            if !state.files.contains_key(&path) {
                return Err(StatusCode::NoSuchFile);
            }
            return Ok(Handle { id, handle: path });
        }
        if state.collide_on_open {
            state.files.insert(
                path.clone(),
                FileState {
                    bytes: b"unrelated".to_vec(),
                    permissions: 0o600,
                },
            );
        }
        if flags.contains(OpenFlags::EXCLUDE) && state.files.contains_key(&path) {
            return Err(StatusCode::Failure);
        }
        state.created_modes.push(attrs.permissions.unwrap_or(0o644));
        state.files.insert(
            path.clone(),
            FileState {
                bytes: Vec::new(),
                permissions: attrs.permissions.unwrap_or(0o644),
            },
        );
        Ok(Handle { id, handle: path })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        let (gate, delay, started) = {
            let state = self.0.lock().unwrap();
            (
                state.write_gate.clone(),
                state.write_delay,
                state.write_started.clone(),
            )
        };
        started.notify_one();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        tokio::time::sleep(delay).await;
        let mut state = self.0.lock().unwrap();
        if state.reject_write {
            return Err(StatusCode::Failure);
        }
        state.write_chunks.push(data.len());
        let file = state.files.get_mut(&handle).ok_or(StatusCode::NoSuchFile)?;
        let offset = offset as usize;
        file.bytes
            .resize(file.bytes.len().max(offset + data.len()), 0);
        file.bytes[offset..offset + data.len()].copy_from_slice(&data);
        Ok(ok(id))
    }

    async fn close(&mut self, id: u32, _: String) -> Result<Status, StatusCode> {
        let mut state = self.0.lock().unwrap();
        if state.reject_close {
            return Err(StatusCode::Failure);
        }
        state.closed_handles += 1;
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, StatusCode> {
        let state = self.0.lock().unwrap();
        let file = state.files.get(&handle).ok_or(StatusCode::NoSuchFile)?;
        let offset = offset as usize;
        if offset >= file.bytes.len() {
            return Err(StatusCode::Eof);
        }
        Ok(Data {
            id,
            data: file.bytes[offset..file.bytes.len().min(offset + len as usize)].to_vec(),
        })
    }

    async fn opendir(&mut self, id: u32, _: String) -> Result<Handle, StatusCode> {
        Ok(Handle {
            id,
            handle: "dir".into(),
        })
    }

    async fn readdir(&mut self, id: u32, _: String) -> Result<Name, StatusCode> {
        let mut state = self.0.lock().unwrap();
        state.directory_batches += 1;
        // Each batch is below the dependency's packet limit. The total is not.
        if state.directory_batches > 2000 {
            return Err(StatusCode::Eof);
        }
        Ok(Name {
            id,
            files: vec![File {
                filename: "x".repeat(16 * 1024),
                longname: String::new(),
                attrs: FileAttributes::empty(),
            }],
        })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        Ok(Name {
            id,
            files: vec![File {
                filename: path,
                longname: String::new(),
                attrs: FileAttributes::empty(),
            }],
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        let mut state = self.0.lock().unwrap();
        if state.reject_setstat {
            return Err(StatusCode::PermissionDenied);
        }
        if let Some(permissions) = attrs.permissions {
            state
                .files
                .get_mut(&path)
                .ok_or(StatusCode::NoSuchFile)?
                .permissions = permissions;
        }
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        self.setstat(id, handle, attrs).await
    }

    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut state = self.0.lock().unwrap();
        state.removed.push(path.clone());
        state.files.remove(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        let mut state = self.0.lock().unwrap();
        if state.reject_rename || state.files.contains_key(&to) {
            return Err(StatusCode::Failure);
        }
        let file = state.files.remove(&from).ok_or(StatusCode::NoSuchFile)?;
        state.files.insert(to, file);
        Ok(ok(id))
    }

    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, StatusCode> {
        if request == "limits@openssh.com" {
            let data = [256_u64, 64, 64, 1]
                .into_iter()
                .flat_map(u64::to_be_bytes)
                .collect();
            return Ok(Packet::ExtendedReply(ExtendedReply { id, data }));
        }
        if request == "fsync@openssh.com" {
            let mut state = self.0.lock().unwrap();
            state.fsync_calls += 1;
            return if state.reject_fsync {
                Err(StatusCode::Failure)
            } else {
                Ok(Packet::Status(ok(id)))
            };
        }
        if request != "posix-rename@openssh.com" {
            return Err(StatusCode::OpUnsupported);
        }
        fn string(data: &mut &[u8]) -> String {
            let length = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
            let value = String::from_utf8(data[4..4 + length].to_vec()).unwrap();
            *data = &data[4 + length..];
            value
        }
        let mut data = data.as_slice();
        let from = string(&mut data);
        let to = string(&mut data);
        let mut state = self.0.lock().unwrap();
        if state.reject_rename {
            return Err(StatusCode::PermissionDenied);
        }
        let file = state.files.remove(&from).ok_or(StatusCode::NoSuchFile)?;
        state.files.insert(to, file);
        Ok(Packet::Status(ok(id)))
    }
}

async fn remote(state: Arc<Mutex<ServerState>>) -> RemoteFs {
    let connect: Connector = Box::new(move || {
        let state = state.clone();
        Box::pin(async move {
            let (client, server_stream) = tokio::io::duplex(64 * 1024);
            server::run(server_stream, MemoryServer(state)).await;
            Session::new(client).await
        })
    });
    let session = connect().await.unwrap();
    RemoteFs {
        session: tokio::sync::Mutex::new(Some(session)),
        connect,
    }
}

#[tokio::test]
async fn rename_failure_never_deletes_the_original_file() {
    let state = Arc::new(Mutex::new(ServerState {
        reject_rename: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    assert!(
        fs.write_file_atomic("/target", b"replacement")
            .await
            .is_err()
    );
    let state = state.lock().unwrap();
    assert_eq!(
        state.files.get("/target").map(|file| file.bytes.as_slice()),
        Some(b"original".as_slice())
    );
    assert!(!state.removed.iter().any(|path| path == "/target"));
}

#[tokio::test]
async fn temporary_file_creation_does_not_truncate_a_collision() {
    let state = Arc::new(Mutex::new(ServerState {
        collide_on_open: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    assert!(
        fs.write_file_atomic("/target", b"replacement")
            .await
            .is_err()
    );
    let state = state.lock().unwrap();
    assert!(
        state
            .files
            .iter()
            .any(|(path, file)| path != "/target" && file.bytes == b"unrelated")
    );
    assert!(state.removed.is_empty());
}

#[tokio::test]
async fn permission_preservation_failure_prevents_publication() {
    let state = Arc::new(Mutex::new(ServerState {
        reject_setstat: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    assert!(
        fs.write_file_atomic("/target", b"replacement")
            .await
            .is_err()
    );
    assert_eq!(state.lock().unwrap().files["/target"].bytes, b"original");
}

#[tokio::test]
async fn overwrite_uses_an_atomic_server_operation_without_remove() {
    let state = Arc::new(Mutex::new(ServerState::default()));
    let fs = remote(state.clone()).await;
    fs.write_file_atomic("/target", b"replacement")
        .await
        .unwrap();
    let state = state.lock().unwrap();
    assert_eq!(state.files["/target"].bytes, b"replacement");
    assert_eq!(state.files["/target"].permissions, 0o600);
    assert!(!state.removed.iter().any(|path| path == "/target"));
}

#[tokio::test]
async fn plain_v3_can_create_but_never_removes_an_existing_destination() {
    let state = Arc::new(Mutex::new(ServerState {
        no_posix_rename: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    fs.write_file_atomic("/new", b"new").await.unwrap();
    assert!(
        fs.write_file_atomic("/target", b"replacement")
            .await
            .is_err()
    );
    let state = state.lock().unwrap();
    assert_eq!(state.files["/target"].bytes, b"original");
    assert_eq!(state.files["/new"].permissions, 0o600);
    assert_eq!(
        state.files.len(),
        2,
        "failed publication cleans its owned temp"
    );
    assert!(!state.removed.iter().any(|path| path == "/target"));
}

#[tokio::test]
async fn write_or_close_failure_prevents_publication_and_cleans_the_temp() {
    for fail_close in [false, true] {
        let state = Arc::new(Mutex::new(ServerState {
            reject_close: fail_close,
            reject_write: !fail_close,
            ..Default::default()
        }));
        let fs = remote(state.clone()).await;
        assert!(
            fs.write_file_atomic("/target", b"replacement")
                .await
                .is_err()
        );
        let state = state.lock().unwrap();
        assert_eq!(state.files["/target"].bytes, b"original");
        assert_eq!(state.files.len(), 1);
    }
}

#[tokio::test]
async fn denied_metadata_is_not_treated_as_a_new_file() {
    let state = Arc::new(Mutex::new(ServerState {
        reject_stat: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    assert!(
        fs.write_file_atomic("/target", b"replacement")
            .await
            .is_err()
    );
    let state = state.lock().unwrap();
    assert_eq!(state.files["/target"].bytes, b"original");
    assert!(state.created_modes.is_empty());
}

#[tokio::test]
async fn bounded_chunks_roundtrip_and_success_reuses_one_session() {
    let state = Arc::new(Mutex::new(ServerState::default()));
    let fs = remote(state.clone()).await;
    let bytes = vec![42; 100_000];
    fs.write_file_atomic("/target", &bytes).await.unwrap();
    assert_eq!(fs.read_file("/target").await.unwrap(), bytes);
    assert_eq!(fs.stat("/target").await.unwrap().size, 100_000);
    assert_eq!(fs.canonicalize("/target").await.unwrap(), "/target");
    let state = state.lock().unwrap();
    assert_eq!(state.sessions, 1);
    assert_eq!(state.closed_handles, 2);
    assert!(state.write_chunks.len() > 1);
    assert!(state.write_chunks.iter().all(|&n| n <= 32 * 1024));
    assert_eq!(state.created_modes, [0o600]);
}

#[tokio::test]
async fn empty_files_publish_and_read_without_a_write_request() {
    let state = Arc::new(Mutex::new(ServerState::default()));
    let fs = remote(state.clone()).await;
    fs.write_file_atomic("/new", b"").await.unwrap();
    assert!(fs.read_file("/new").await.unwrap().is_empty());
    assert!(state.lock().unwrap().write_chunks.is_empty());
}

async fn wait_for_closed_session(state: &Arc<Mutex<ServerState>>) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state.lock().unwrap().dropped_sessions == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the server must observe EOF and release its session");
}

#[tokio::test]
async fn failed_operation_retires_channel_and_next_operation_recovers() {
    let state = Arc::new(Mutex::new(ServerState {
        reject_rename: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    assert!(
        fs.write_file_atomic("/target", b"replacement")
            .await
            .is_err()
    );
    wait_for_closed_session(&state).await;
    state.lock().unwrap().reject_rename = false;
    fs.write_file_atomic("/target", b"recovered").await.unwrap();
    assert_eq!(state.lock().unwrap().sessions, 2);
}

#[tokio::test]
async fn cancelled_operation_closes_channel_without_publishing_and_recovers() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let state = Arc::new(Mutex::new(ServerState {
        write_gate: Some(gate.clone()),
        ..Default::default()
    }));
    let started = state.lock().unwrap().write_started.clone();
    let fs = Arc::new(remote(state.clone()).await);
    let writer = fs.clone();
    let task = tokio::spawn(async move { writer.write_file_atomic("/target", b"cancelled").await });
    started.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Let the server finish its current request so it can observe client EOF.
    gate.add_permits(1);
    wait_for_closed_session(&state).await;
    assert_eq!(state.lock().unwrap().files["/target"].bytes, b"original");
    state.lock().unwrap().write_gate = None;
    fs.write_file_atomic("/target", b"recovered").await.unwrap();
    assert_eq!(state.lock().unwrap().sessions, 2);
}

#[tokio::test(start_paused = true)]
async fn one_deadline_covers_metadata_and_write_requests_together() {
    let state = Arc::new(Mutex::new(ServerState {
        stat_delay: std::time::Duration::from_secs(20),
        write_delay: std::time::Duration::from_secs(20),
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    fs.session
        .lock()
        .await
        .as_ref()
        .unwrap()
        .raw
        .set_timeout(60);
    let start = tokio::time::Instant::now();
    assert!(matches!(
        fs.write_file_atomic("/target", b"replacement").await,
        Err(SshError::TimedOut(_))
    ));
    assert_eq!(start.elapsed(), SSH_OPERATION_TIMEOUT);
    assert_eq!(state.lock().unwrap().files["/target"].bytes, b"original");
}

#[tokio::test]
async fn directory_limit_stops_requests_before_the_server_finishes_listing() {
    let state = Arc::new(Mutex::new(ServerState::default()));
    let fs = remote(state.clone()).await;
    assert!(matches!(
        fs.list_dir("/").await,
        Err(SshError::InvalidInput(_))
    ));
    let batches = state.lock().unwrap().directory_batches;
    assert!(
        batches <= MAX_SSH_OUTPUT_BYTES / (16 * 1024) + 1,
        "read {batches} batches"
    );
    wait_for_closed_session(&state).await;
}

#[tokio::test]
async fn read_limit_is_enforced_even_when_metadata_omits_size() {
    let state = Arc::new(Mutex::new(ServerState {
        hide_size: true,
        ..Default::default()
    }));
    state
        .lock()
        .unwrap()
        .files
        .get_mut("/target")
        .unwrap()
        .bytes = vec![42; MAX_SSH_OUTPUT_BYTES + 1];
    let fs = remote(state.clone()).await;
    assert!(matches!(
        fs.read_file("/target").await,
        Err(SshError::InvalidInput(_))
    ));
    wait_for_closed_session(&state).await;
}

#[tokio::test]
async fn advertised_server_limits_bound_each_read_and_write_request() {
    let state = Arc::new(Mutex::new(ServerState {
        small_limits: true,
        ..Default::default()
    }));
    let fs = remote(state.clone()).await;
    let bytes = vec![42; 129];
    fs.write_file_atomic("/target", &bytes).await.unwrap();
    assert_eq!(fs.read_file("/target").await.unwrap(), bytes);
    assert_eq!(state.lock().unwrap().write_chunks, [64, 64, 1]);
    assert_eq!(state.lock().unwrap().sessions, 1);
}

#[tokio::test]
async fn advertised_fsync_must_succeed_before_publication() {
    for reject_fsync in [false, true] {
        let state = Arc::new(Mutex::new(ServerState {
            fsync: true,
            reject_fsync,
            ..Default::default()
        }));
        let fs = remote(state.clone()).await;
        assert_eq!(
            fs.write_file_atomic("/target", b"replacement")
                .await
                .is_err(),
            reject_fsync
        );
        let state = state.lock().unwrap();
        assert_eq!(state.fsync_calls, 1);
        assert_eq!(
            state.files["/target"].bytes,
            if reject_fsync {
                b"original".as_slice()
            } else {
                b"replacement".as_slice()
            }
        );
        assert_eq!(state.files.len(), 1);
    }
}
