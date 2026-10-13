//! Read-only observation of an authenticated LaunchServices application.
//!
//! LaunchServices applications are not the controller's waitpid children.
//! This observer binds the caller-authenticated launch to a kernel socket
//! peer, then registers an exact process knote before startup is acknowledged.
//! It grants no signal, process-group, tree-cleanup or persisted authority.

use std::{
    ffi::{CStr, OsString}, fmt, io,
    os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::{ffi::OsStringExt, net::UnixStream}},
    path::{Path, PathBuf}, sync::OnceLock, time::{Duration, Instant},
};

use crate::ExactProcessIdentity;

const PROC_PIDUNIQIDENTIFIERINFO: libc::c_int = 17;
const MAX_POLL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DarwinApplicationExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

enum ObservationState {
    Active,
    Exited(DarwinApplicationExit),
    Uncertain(io::ErrorKind, String),
}

/// An exact read-only process observer. Dropping it only closes its knote.
/// The caller separately owns the nonce protocol and verified cleanup receipt.
pub struct DarwinLaunchedApplicationObserver {
    kqueue: OwnedFd,
    identity: ExactProcessIdentity,
    generation: Generation,
    token: AuditToken,
    api: &'static NativeApi,
    state: ObservationState,
}

impl fmt::Debug for DarwinLaunchedApplicationObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("DarwinLaunchedApplicationObserver")
            .field("identity", &self.identity).field("pidversion", &self.generation.version)
            .finish_non_exhaustive()
    }
}

impl DarwinLaunchedApplicationObserver {
    pub fn identity(&self) -> &ExactProcessIdentity { &self.identity }
    pub fn pidversion(&self) -> u32 { self.generation.version }

    /// Reconfirm the same still-running incarnation before controller ready.
    /// This retains the original knote and never reattaches by numeric PID.
    /// A known kernel exit remains cached for polling, but cannot prove ready.
    pub fn confirm_running_generation(&mut self) -> io::Result<()> {
        if self.poll_exit(Duration::ZERO)?.is_some() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "admitted application already exited before ready"));
        }
        if let Err(error) = verify_live(self.api, &self.token, &self.identity, self.generation) {
            // Exit can race the live check. Keep a genuine already queued
            // terminal status rather than replacing it with root-only absence.
            let queued = match wait_event(self.kqueue.as_raw_fd(), Duration::ZERO) {
                Ok(queued) => queued,
                Err(error) => {
                    self.state = ObservationState::Uncertain(error.kind(), error.to_string());
                    return Err(error);
                }
            };
            if let Some(event) = queued {
                match classify_event(event, self.identity.pid) {
                    Ok(exit) => {
                        self.state = ObservationState::Exited(exit);
                        return Err(io::Error::new(io::ErrorKind::NotFound, "admitted application exited before ready"));
                    }
                    Err(error) => {
                        self.state = ObservationState::Uncertain(error.kind(), error.to_string());
                        return Err(error);
                    }
                }
            }
            self.state = ObservationState::Uncertain(error.kind(), error.to_string());
            return Err(error);
        }
        if self.poll_exit(Duration::ZERO)?.is_some() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "admitted application exited before ready"));
        }
        Ok(())
    }

    /// Read the admitted application's kernel exit status. A zero timeout is
    /// nonblocking. A timeout returns None; it never proves cleanup or exit.
    /// Exec/generation changes and observation failures remain sticky errors.
    pub fn poll_exit(&mut self, timeout: Duration) -> io::Result<Option<DarwinApplicationExit>> {
        match &self.state {
            ObservationState::Exited(exit) => return Ok(Some(*exit)),
            ObservationState::Uncertain(kind, message) => return Err(io::Error::new(*kind, message.clone())),
            ObservationState::Active => {}
        }
        if timeout > MAX_POLL {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "application exit poll exceeds 30 seconds"));
        }
        let result = self.poll_active(timeout);
        match result {
            Ok(Some(exit)) => self.state = ObservationState::Exited(exit),
            Err(ref error) => self.state = ObservationState::Uncertain(error.kind(), error.to_string()),
            Ok(None) => {}
        }
        result
    }

    fn poll_active(&self, timeout: Duration) -> io::Result<Option<DarwinApplicationExit>> {
        if let Some(event) = wait_event(self.kqueue.as_raw_fd(), timeout)? {
            return classify_event(event, self.identity.pid).map(Some);
        }
        // Drain before and after inspection: exit can race the first empty
        // poll. Disappearance alone cannot fabricate the missing NOTE_EXIT.
        if let Some(current) = read_generation(self.identity.pid)? {
            if current != self.generation {
                if let Some(event) = wait_event(self.kqueue.as_raw_fd(), Duration::ZERO)? {
                    return classify_event(event, self.identity.pid).map(Some);
                }
                return Err(io::Error::other("admitted application generation changed; exit/cleanup is unknown"));
            }
            if let Some(path) = self.api.token_path(&self.token)? {
                if Some(&canonical_executable(&path)?) != self.identity.executable.as_ref() {
                    return Err(io::Error::other("admitted application executable changed; exit/cleanup is unknown"));
                }
            }
        }
        wait_event(self.kqueue.as_raw_fd(), Duration::ZERO)?
            .map(|event| classify_event(event, self.identity.pid)).transpose()
    }
}

/// Return the live, same-user kernel peer PID for a public application lookup.
///
/// The caller first authenticates its private launch nonce. This is only a
/// lookup key, not launch confirmation or signal/cleanup authority. The public
/// NSRunningApplication record and executable must be validated before native
/// observer admission; the later NSWorkspace callback must confirm that same
/// PID/bundle/executable before the controller reports ready or success.
pub fn launched_application_peer_pid(peer: &UnixStream) -> io::Result<u32> {
    let api = native_api()?;
    let pid: libc::pid_t = socket_option(peer, libc::LOCAL_PEERPID)?;
    let token: AuditToken = socket_option(peer, libc::LOCAL_PEERTOKEN)?;
    let (token_pid, version, euid, ruid) = unsafe {
        ((api.pid)(token), (api.version)(token) as u32, (api.euid)(token), (api.ruid)(token))
    };
    if pid <= 1 || token_pid != pid || pid as u32 == std::process::id() || version == 0
        || euid != unsafe { libc::geteuid() } || ruid != unsafe { libc::getuid() } {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "application kernel peer PID/user is invalid"));
    }
    let generation = read_generation(pid as u32)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application peer exited before public lookup"))?;
    let bsd = bsd_info(pid as u32)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application peer exited before public lookup"))?;
    if generation.version != version || bsd.pbi_pid != pid as u32 || bsd.pbi_uid != euid
        || bsd.pbi_ruid != ruid || bsd.pbi_status == libc::SZOMB {
        return Err(io::Error::other("application peer generation/user/state changed before public lookup"));
    }
    let path = api.token_path(&token)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application execution exited before public lookup"))?;
    canonical_executable(&path)?;
    if read_generation(pid as u32)? != Some(generation) {
        return Err(io::Error::other("application peer generation changed during public lookup"));
    }
    Ok(pid as u32)
}

/// Bind an authenticated LaunchServices application record to its socket peer.
///
/// Before calling, the controller must authenticate its per-launch nonce and
/// public bundle/executable record. The expected PID may come from a completed
/// NSWorkspace callback or from launched_application_peer_pid plus an exact
/// public NSRunningApplication lookup. In the latter case the late NSWorkspace
/// callback must separately confirm the same identity before ready/success.
/// The app waits for provisional admission ACK before startup: EVFILT_PROC
/// captures future edges only. This does not relax registered-child PPID guards.
pub fn own_launched_application(
    peer: &UnixStream,
    expected_pid: u32,
    expected_executable: &Path,
) -> io::Result<DarwinLaunchedApplicationObserver> {
    let callback_pid = expected_pid;
    let pid = libc::pid_t::try_from(callback_pid).ok().filter(|pid| *pid > 1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "LaunchServices callback PID is invalid"))?;
    if callback_pid == std::process::id() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "application callback cannot name its controller"));
    }
    let expected_executable = canonical_executable(expected_executable)?;
    let api = native_api()?;
    let kernel_pid: libc::pid_t = socket_option(peer, libc::LOCAL_PEERPID)?;
    let token: AuditToken = socket_option(peer, libc::LOCAL_PEERTOKEN)?;
    let (token_pid, version, euid, ruid) = unsafe {
        ((api.pid)(token), (api.version)(token) as u32, (api.euid)(token), (api.ruid)(token))
    };
    if kernel_pid != pid || token_pid != pid || version == 0
        || euid != unsafe { libc::geteuid() } || ruid != unsafe { libc::getuid() } {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "application callback and kernel peer PID/user do not match"));
    }
    let generation = read_generation(callback_pid)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application exited before admission"))?;
    if generation.version != version {
        return Err(io::Error::other("application execution changed before admission"));
    }
    let bsd = bsd_info(callback_pid)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application exited before admission"))?;
    if bsd.pbi_uid != euid || bsd.pbi_ruid != ruid || bsd.pbi_status == libc::SZOMB {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "application peer user/state is invalid"));
    }
    let identity = ExactProcessIdentity {
        pid: callback_pid, start_time_epoch_seconds: bsd.pbi_start_tvsec,
        platform_start_key: birth_key(&bsd)?, executable: Some(expected_executable),
    };
    verify_live(api, &token, &identity, generation)?;
    let descriptor = unsafe { libc::kqueue() };
    if descriptor < 0 { return Err(io::Error::last_os_error()); }
    let kqueue = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if flags == -1 || unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let change = libc::kevent {
        ident: pid as libc::uintptr_t, filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_RECEIPT,
        fflags: libc::NOTE_EXIT | libc::NOTE_EXITSTATUS | libc::NOTE_EXEC,
        data: 0, udata: std::ptr::null_mut(),
    };
    let mut receipt: libc::kevent = unsafe { std::mem::zeroed() };
    let returned = loop {
        let returned = unsafe { libc::kevent(descriptor, &change, 1, &mut receipt, 1, std::ptr::null()) };
        if returned >= 0 { break returned; }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) { return Err(error); }
    };
    validate_receipt(returned, snapshot_event(receipt), callback_pid)?;
    // kevent attaches by numeric PID. The stable token/generation recheck
    // binds its successful receipt to the same still-live peer incarnation.
    verify_live(api, &token, &identity, generation)?;
    Ok(DarwinLaunchedApplicationObserver { kqueue, identity, generation, token, api, state: ObservationState::Active })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Generation { unique: u64, version: u32 }

#[repr(C)]
struct UniqueInfo {
    uuid: [u8; 16], unique: u64, parent_unique: u64,
    version: i32, parent_version: i32, reserved: [u64; 2],
}

fn read_generation(pid: u32) -> io::Result<Option<Generation>> {
    let Some(info) = read_info::<UniqueInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO)? else { return Ok(None); };
    if info.unique == 0 || info.version == 0 { return Err(io::Error::other("invalid Darwin application generation")); }
    Ok(Some(Generation { unique: info.unique, version: info.version as u32 }))
}

fn bsd_info(pid: u32) -> io::Result<Option<libc::proc_bsdinfo>> { read_info(pid, libc::PROC_PIDTBSDINFO) }

fn read_info<T>(pid: u32, flavor: libc::c_int) -> io::Result<Option<T>> {
    let mut info = std::mem::MaybeUninit::<T>::zeroed();
    let size = std::mem::size_of::<T>() as libc::c_int;
    let returned = unsafe { libc::proc_pidinfo(pid as libc::pid_t, flavor, 1, info.as_mut_ptr().cast(), size) };
    if returned != size {
        let error = io::Error::last_os_error();
        if returned <= 0 && error.raw_os_error() == Some(libc::ESRCH) { return Ok(None); }
        return Err(if returned > 0 { io::Error::other("incomplete Darwin application process record") } else { error });
    }
    Ok(Some(unsafe { info.assume_init() }))
}

fn birth_key(info: &libc::proc_bsdinfo) -> io::Result<u64> {
    if info.pbi_start_tvusec >= 1_000_000 { return Err(io::Error::other("invalid application birth microseconds")); }
    info.pbi_start_tvsec.checked_mul(1_000_000).and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|key| *key != 0).ok_or_else(|| io::Error::other("invalid application birth key"))
}

fn verify_live(api: &NativeApi, token: &AuditToken, identity: &ExactProcessIdentity, generation: Generation) -> io::Result<()> {
    if read_generation(identity.pid)? != Some(generation) {
        return Err(io::Error::other("application generation changed during observer admission"));
    }
    let bsd = bsd_info(identity.pid)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application exited during observer admission"))?;
    if bsd.pbi_pid != identity.pid || birth_key(&bsd)? != identity.platform_start_key
        || bsd.pbi_uid != unsafe { libc::geteuid() } || bsd.pbi_ruid != unsafe { libc::getuid() }
        || bsd.pbi_status == libc::SZOMB {
        return Err(io::Error::other("application birth/user/state changed during admission"));
    }
    let path = api.token_path(token)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application execution exited during admission"))?;
    if Some(&canonical_executable(&path)?) != identity.executable.as_ref()
        || read_generation(identity.pid)? != Some(generation) {
        return Err(io::Error::other("application executable/generation changed during admission"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Event { ident: libc::uintptr_t, filter: i16, flags: u16, fflags: u32, data: libc::intptr_t }

fn snapshot_event(event: libc::kevent) -> Event {
    Event { ident: event.ident, filter: event.filter, flags: event.flags, fflags: event.fflags, data: event.data }
}

fn validate_receipt(returned: i32, receipt: Event, pid: u32) -> io::Result<()> {
    if returned != 1 || receipt.ident != pid as libc::uintptr_t || receipt.filter != libc::EVFILT_PROC
        || receipt.flags & libc::EV_ERROR == 0 {
        return Err(io::Error::other("application exit observer registration receipt is invalid"));
    }
    if receipt.data != 0 {
        return Err(i32::try_from(receipt.data).ok().filter(|errno| *errno > 0)
            .map(io::Error::from_raw_os_error).unwrap_or_else(|| io::Error::other("invalid registration errno")));
    }
    Ok(())
}

fn classify_event(event: Event, pid: u32) -> io::Result<DarwinApplicationExit> {
    if event.ident != pid as libc::uintptr_t || event.filter != libc::EVFILT_PROC || event.flags & libc::EV_ERROR != 0 {
        return Err(io::Error::other("application exit observation has an invalid identity or kernel error"));
    }
    if event.fflags & libc::NOTE_EXEC != 0 {
        return Err(io::Error::other("admitted application exec changed; exit/cleanup is unknown"));
    }
    if event.fflags & (libc::NOTE_EXIT | libc::NOTE_EXITSTATUS) != libc::NOTE_EXIT | libc::NOTE_EXITSTATUS {
        return Err(io::Error::other("application exit observation is missing the kernel exit status"));
    }
    let status = i32::try_from(event.data).ok().filter(|status| *status >= 0 && *status <= 0xffff)
        .ok_or_else(|| io::Error::other("application exit status is outside the native wait-status range"))?;
    if libc::WIFEXITED(status) { return Ok(DarwinApplicationExit { code: Some(libc::WEXITSTATUS(status)), signal: None }); }
    if libc::WIFSIGNALED(status) { return Ok(DarwinApplicationExit { code: None, signal: Some(libc::WTERMSIG(status)) }); }
    Err(io::Error::other("application observation is not a terminal wait status"))
}

fn wait_event(fd: libc::c_int, timeout: Duration) -> io::Result<Option<Event>> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "poll timeout is too large"))?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = libc::timespec { tv_sec: remaining.as_secs() as libc::time_t, tv_nsec: remaining.subsec_nanos() as libc::c_long };
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let returned = unsafe { libc::kevent(fd, std::ptr::null(), 0, &mut event, 1, &wait) };
        if returned == 1 { return Ok(Some(snapshot_event(event))); }
        if returned == 0 { return Ok(None); }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) { return Err(error); }
        if Instant::now() >= deadline { return Ok(None); }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct AuditToken { values: [u32; 8] }
type TokenPath = unsafe extern "C" fn(*mut AuditToken, *mut libc::c_void, u32) -> libc::c_int;
type TokenInt = unsafe extern "C" fn(AuditToken) -> libc::c_int;
type TokenUid = unsafe extern "C" fn(AuditToken) -> libc::uid_t;
struct NativeApi { path: TokenPath, pid: TokenInt, version: TokenInt, euid: TokenUid, ruid: TokenUid, _libraries: [usize; 2] }

impl NativeApi {
    fn token_path(&self, token: &AuditToken) -> io::Result<Option<PathBuf>> {
        let mut token = *token;
        let mut bytes = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let returned = unsafe { (self.path)(&raw mut token, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
        if returned <= 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) { Ok(None) } else { Err(error) };
        }
        let length = returned as usize;
        if length >= bytes.len() || bytes[length] != 0 || bytes[..length].contains(&0) || bytes[0] != b'/' {
            return Err(io::Error::other("invalid kernel application executable path"));
        }
        bytes.truncate(length);
        Ok(Some(PathBuf::from(OsString::from_vec(bytes))))
    }
}

fn native_api() -> io::Result<&'static NativeApi> {
    static API: OnceLock<Result<NativeApi, &'static str>> = OnceLock::new();
    API.get_or_init(load_native_api).as_ref().map_err(|symbol| io::Error::new(io::ErrorKind::Unsupported, format!("Darwin application peer observer API is unavailable: {symbol}")))
}

fn load_native_api() -> Result<NativeApi, &'static str> {
    let proc_library = unsafe { libc::dlopen(c"/usr/lib/libproc.dylib".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if proc_library.is_null() { return Err("libproc"); }
    let bsm_library = unsafe { libc::dlopen(c"/usr/lib/libbsm.dylib".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if bsm_library.is_null() { unsafe { libc::dlclose(proc_library); } return Err("libbsm"); }
    let result = (|| Ok(unsafe { NativeApi {
        path: std::mem::transmute::<*mut libc::c_void, TokenPath>(symbol(proc_library, c"proc_pidpath_audittoken")?),
        pid: std::mem::transmute::<*mut libc::c_void, TokenInt>(symbol(bsm_library, c"audit_token_to_pid")?),
        version: std::mem::transmute::<*mut libc::c_void, TokenInt>(symbol(bsm_library, c"audit_token_to_pidversion")?),
        euid: std::mem::transmute::<*mut libc::c_void, TokenUid>(symbol(bsm_library, c"audit_token_to_euid")?),
        ruid: std::mem::transmute::<*mut libc::c_void, TokenUid>(symbol(bsm_library, c"audit_token_to_ruid")?),
        _libraries: [proc_library as usize, bsm_library as usize],
    } }))();
    if result.is_err() { unsafe { libc::dlclose(bsm_library); libc::dlclose(proc_library); } }
    result
}

unsafe fn symbol(library: *mut libc::c_void, name: &'static CStr) -> Result<*mut libc::c_void, &'static str> {
    let pointer = unsafe { libc::dlsym(library, name.as_ptr()) };
    if pointer.is_null() { Err(name.to_str().unwrap_or("native symbol")) } else { Ok(pointer) }
}

fn canonical_executable(path: &Path) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.is_file() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "application executable must be a regular file")); }
    Ok(path)
}

fn socket_option<T>(peer: &UnixStream, option: libc::c_int) -> io::Result<T> {
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let mut size = std::mem::size_of::<T>() as libc::socklen_t;
    if unsafe { libc::getsockopt(peer.as_raw_fd(), libc::SOL_LOCAL, option, value.as_mut_ptr().cast(), &raw mut size) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if size as usize != std::mem::size_of::<T>() { return Err(io::Error::other("application peer socket option size mismatch")); }
    Ok(unsafe { value.assume_init() })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(fflags: u32, data: libc::intptr_t) -> Event {
        Event { ident: 123, filter: libc::EVFILT_PROC, flags: libc::EV_EOF, fflags, data }
    }
    #[test]
    fn application_exit_uses_raw_wait_status_and_requires_status_receipt() {
        let flags = libc::NOTE_EXIT | libc::NOTE_EXITSTATUS;
        assert_eq!(classify_event(event(flags, 7 << 8), 123).unwrap(), DarwinApplicationExit { code: Some(7), signal: None });
        assert_eq!(classify_event(event(flags, libc::SIGTERM as libc::intptr_t), 123).unwrap(), DarwinApplicationExit { code: None, signal: Some(libc::SIGTERM) });
        assert!(classify_event(event(libc::NOTE_EXIT, 0), 123).is_err());
        assert!(classify_event(event(flags | libc::NOTE_EXEC, 0), 123).is_err());
        assert!(classify_event(event(flags, 0xffff), 123).is_err());
    }
}
