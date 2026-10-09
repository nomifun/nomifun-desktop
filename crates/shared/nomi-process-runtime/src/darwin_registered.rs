//! Exact Darwin authority for a child registered through a local socket.
//!
//! Native libraries can launch children without giving the application an
//! owned `Child` handle. A launch nonce belongs to the caller's registration
//! protocol; it is not an OS process identity. After authenticating that
//! protocol, the caller can bind a socket peer to its actual parent, executable
//! and kernel audit token here. No PID/group signal fallback is permitted.

use std::{
    ffi::{CStr, OsString},
    fmt, io,
    os::{fd::AsRawFd, unix::{ffi::OsStringExt, net::UnixStream}},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use crate::ExactProcessIdentity;

const MAX_TERMINATION_WAIT: Duration = Duration::from_secs(30);

/// What was proven about the registered *execution generation*.
///
/// After exec, `AlreadyAbsent` does not authorize signalling the new image or
/// claim that the PID is unoccupied. A signal already sent to a valid old image
/// can be inherited by an imminent exec according to ordinary Darwin semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DarwinGenerationTerminationOutcome {
    AlreadyAbsent,
    SignalledAndAbsent,
}

/// Opaque signal authority obtained from an authenticated kernel socket peer.
///
/// It cannot be created from a caller-supplied PID or audit token, serialized,
/// or widened into process-group authority. Dropping it sends no signal. Its
/// caller retains responsibility for the registration nonce and lifecycle.
pub struct DarwinRegisteredChildAuthority {
    token: AuditToken,
    identity: ExactProcessIdentity,
    pidversion: u32,
    api: &'static NativeApi,
}

impl fmt::Debug for DarwinRegisteredChildAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("DarwinRegisteredChildAuthority")
            .field("identity", &self.identity)
            .field("pidversion", &self.pidversion)
            .finish_non_exhaustive()
    }
}

impl DarwinRegisteredChildAuthority {
    /// Diagnostic process birth identity; signal authority uses the additional
    /// private audit-token execution version, not this start key alone.
    pub fn identity(&self) -> &ExactProcessIdentity { &self.identity }

    pub fn pidversion(&self) -> u32 { self.pidversion }

    /// True only when the kernel reports that this audit-token generation is
    /// absent. Probe errors, including permission errors, prove nothing.
    pub fn generation_is_absent(&self) -> io::Result<bool> {
        Ok(self.api.token_path(&self.token)?.is_none())
    }

    /// Request SIGKILL for this exact registered execution and prove its
    /// generation absent. Blocking and bounded by `timeout` (at most 30s).
    /// Async callers should run this on their dedicated blocking worker.
    ///
    /// Darwin compares pidversion in the kernel and holds the exact proc
    /// reference while sending. A completed exec makes the old token stale.
    /// This is not a promise about signals already queued before an exec.
    pub fn terminate_and_prove_generation_absent(
        &self,
        timeout: Duration,
    ) -> io::Result<DarwinGenerationTerminationOutcome> {
        if timeout > MAX_TERMINATION_WAIT {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "registered-child termination timeout exceeds 30 seconds"));
        }
        let deadline = Instant::now().checked_add(timeout).ok_or_else(||
            io::Error::new(io::ErrorKind::InvalidInput, "termination timeout is too large"))?;
        let mut token = self.token;
        // SAFETY: token came only from LOCAL_PEERTOKEN and remains an opaque
        // SDK-layout value. libproc performs the generation check atomically;
        // this never falls back to kill(pid), killpg, or an enumerated tree.
        let result = unsafe { (self.api.signal)(&raw mut token, libc::SIGKILL) };
        let outcome = match signal_result(result) {
            Ok(()) => DarwinGenerationTerminationOutcome::SignalledAndAbsent,
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) =>
                DarwinGenerationTerminationOutcome::AlreadyAbsent,
            Err(error) => return Err(error),
        };
        let mut backoff = Duration::from_millis(10);
        loop {
            if self.generation_is_absent()? { return Ok(outcome); }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(io::Error::new(io::ErrorKind::TimedOut,
                    format!("registered process {} generation {} is still observable",
                        self.identity.pid, self.pidversion)));
            };
            std::thread::sleep(backoff.min(remaining));
            backoff = (backoff * 2).min(Duration::from_millis(200));
        }
    }
}

/// Bind an already authenticated registration socket to its exact live child.
///
/// The caller must first authenticate its per-launch nonce/protocol. This
/// function obtains PID, UID and audit token from the kernel, requires the
/// peer's actual PPID to match `expected_parent`, rechecks that parent's birth
/// generation before and after admission, and requires both executable paths.
/// An unreadable/missing path or unavailable native API fails closed.
pub fn own_registered_child_process(
    peer: &UnixStream,
    expected_parent: &ExactProcessIdentity,
    expected_executable: &Path,
) -> io::Result<DarwinRegisteredChildAuthority> {
    if expected_parent.pid <= 1 || expected_parent.pid > libc::pid_t::MAX as u32
        || expected_parent.platform_start_key == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "registered-child parent requires a valid PID and birth generation"));
    }
    let parent_path = expected_parent.executable.as_ref().ok_or_else(||
        io::Error::new(io::ErrorKind::InvalidInput,
            "registered-child parent executable is required"))?;
    let mut parent = expected_parent.clone();
    parent.executable = Some(canonical_executable(parent_path)?);
    let expected_executable = canonical_executable(expected_executable)?;
    let api = native_api()?;
    let uid = unsafe { libc::geteuid() };
    let ruid = unsafe { libc::getuid() };
    verify_parent(&parent, uid, ruid)?;

    let kernel_pid: libc::pid_t = socket_option(peer, libc::LOCAL_PEERPID)?;
    let token: AuditToken = socket_option(peer, libc::LOCAL_PEERTOKEN)?;
    // SAFETY: these SDK BSM functions are the supported parsers for an opaque
    // audit_token_t; no code relies on the representation's field offsets.
    let (pid, pidversion, peer_uid, peer_ruid) = unsafe {
        ((api.token_pid)(token), (api.token_pidversion)(token) as u32,
            (api.token_euid)(token), (api.token_ruid)(token))
    };
    if pid <= 1 || pid != kernel_pid || pid as u32 == std::process::id()
        || pidversion == 0 || peer_uid != uid || peer_ruid != ruid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child kernel peer identity is invalid or belongs to another user"));
    }
    let info = bsd_info(pid as u32)?.ok_or_else(||
        io::Error::new(io::ErrorKind::NotFound, "registered-child peer already exited"))?;
    if info.pbi_ppid != parent.pid || info.pbi_uid != uid || info.pbi_ruid != ruid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child peer has a different actual parent or user"));
    }
    let actual_path = api.token_path(&token)?.ok_or_else(||
        io::Error::new(io::ErrorKind::NotFound, "registered-child execution already exited"))?;
    if canonical_executable(&actual_path)? != expected_executable {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child executable does not match the expected executable"));
    }
    let start_key = birth_key(&info)?;
    let confirmed = bsd_info(pid as u32)?.ok_or_else(||
        io::Error::new(io::ErrorKind::NotFound, "registered-child peer exited during admission"))?;
    if birth_key(&confirmed)? != start_key || confirmed.pbi_ppid != parent.pid
        || confirmed.pbi_uid != uid || confirmed.pbi_ruid != ruid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child peer changed during admission"));
    }
    let confirmed_path = api.token_path(&token)?.ok_or_else(||
        io::Error::new(io::ErrorKind::NotFound, "registered-child execution changed during admission"))?;
    if canonical_executable(&confirmed_path)? != expected_executable {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child executable changed during admission"));
    }
    verify_parent(&parent, uid, ruid)?;
    Ok(DarwinRegisteredChildAuthority {
        token, pidversion, api,
        identity: ExactProcessIdentity {
            pid: pid as u32,
            start_time_epoch_seconds: info.pbi_start_tvsec,
            platform_start_key: start_key,
            executable: Some(expected_executable),
        },
    })
}

fn canonical_executable(path: &Path) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "registered-child executable must be a regular file"));
    }
    Ok(path)
}

fn verify_parent(expected: &ExactProcessIdentity, uid: libc::uid_t, ruid: libc::uid_t) -> io::Result<()> {
    let info = bsd_info(expected.pid)?.ok_or_else(||
        io::Error::new(io::ErrorKind::PermissionDenied, "registered-child parent is absent"))?;
    if birth_key(&info)? != expected.platform_start_key || info.pbi_uid != uid || info.pbi_ruid != ruid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child parent birth generation or user changed"));
    }
    let path = process_path(expected.pid)?;
    if expected.executable.as_ref() != Some(&canonical_executable(&path)?) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "registered-child parent executable changed"));
    }
    Ok(())
}

fn bsd_info(pid: u32) -> io::Result<Option<libc::proc_bsdinfo>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer is writable for the exact advertised SDK size.
    let returned = unsafe { libc::proc_pidinfo(pid as libc::pid_t,
        libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if returned <= 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) { Ok(None) } else { Err(error) };
    }
    if returned != size {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Darwin bsdinfo size mismatch"));
    }
    // SAFETY: proc_pidinfo initialized the full SDK structure.
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Darwin bsdinfo PID mismatch"));
    }
    Ok(Some(info))
}

fn birth_key(info: &libc::proc_bsdinfo) -> io::Result<u64> {
    info.pbi_start_tvsec.checked_mul(1_000_000)
        .and_then(|value| value.checked_add(info.pbi_start_tvusec))
        .filter(|value| *value != 0 && info.pbi_start_tvusec < 1_000_000)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
            "Darwin process birth generation is invalid"))
}

fn socket_option<T>(peer: &UnixStream, option: libc::c_int) -> io::Result<T> {
    let mut result = std::mem::MaybeUninit::<T>::zeroed();
    let mut size = std::mem::size_of::<T>() as libc::socklen_t;
    // SAFETY: both callers use the exact SDK type of their LOCAL socket option.
    let returned = unsafe { libc::getsockopt(peer.as_raw_fd(), libc::SOL_LOCAL,
        option, result.as_mut_ptr().cast(), &raw mut size) };
    if returned != 0 { return Err(io::Error::last_os_error()); }
    if size as usize != std::mem::size_of::<T>() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Darwin peer socket option size mismatch"));
    }
    // SAFETY: getsockopt returned and initialized the exact type's full size.
    Ok(unsafe { result.assume_init() })
}

fn process_path(pid: u32) -> io::Result<PathBuf> {
    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: buffer is writable for the advertised size.
    let returned = unsafe { libc::proc_pidpath(pid as libc::pid_t,
        buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if returned <= 0 { return Err(io::Error::last_os_error()); }
    path_from_buffer(buffer, returned as usize)
}

fn path_from_buffer(mut buffer: Vec<u8>, length: usize) -> io::Result<PathBuf> {
    if length == 0 || length >= buffer.len() || buffer[length] != 0
        || buffer[..length].contains(&0) || buffer[0] != b'/' {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Darwin process path is invalid"));
    }
    buffer.truncate(length);
    Ok(PathBuf::from(OsString::from_vec(buffer)))
}

// This is only the SDK's storage/ABI layout. BSM functions parse every field.
#[derive(Clone, Copy)]
#[repr(C)]
struct AuditToken { values: [u32; 8] }

type Signal = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;
type TokenPath = unsafe extern "C" fn(*mut AuditToken, *mut libc::c_void, u32) -> libc::c_int;
type TokenPid = unsafe extern "C" fn(AuditToken) -> libc::pid_t;
type TokenVersion = unsafe extern "C" fn(AuditToken) -> libc::c_int;
type TokenUid = unsafe extern "C" fn(AuditToken) -> libc::uid_t;

struct NativeApi {
    signal: Signal,
    path: TokenPath,
    token_pid: TokenPid,
    token_pidversion: TokenVersion,
    token_euid: TokenUid,
    token_ruid: TokenUid,
    // Successful loads remain open for process lifetime so cached function
    // pointers cannot outlive their libraries. These are never exposed.
    _library_handles: [usize; 2],
}

impl NativeApi {
    fn token_path(&self, token: &AuditToken) -> io::Result<Option<PathBuf>> {
        let mut token = *token;
        let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: SDK-layout token and writable advertised buffer. libproc
        // compares pidversion in the kernel; a stale execution returns ESRCH.
        let returned = unsafe { (self.path)(&raw mut token,
            buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if returned <= 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) { Ok(None) } else { Err(error) };
        }
        path_from_buffer(buffer, returned as usize).map(Some)
    }
}

fn native_api() -> io::Result<&'static NativeApi> {
    static API: OnceLock<Result<NativeApi, &'static str>> = OnceLock::new();
    API.get_or_init(load_native_api).as_ref().map_err(|missing|
        io::Error::new(io::ErrorKind::Unsupported,
            format!("Darwin exact registered-child API is unavailable: {missing}")))
}

fn load_native_api() -> Result<NativeApi, &'static str> {
    // SAFETY: system-owned absolute library names and RTLD_LOCAL prevent
    // importing caller-selected code or exposing these libraries globally.
    let proc_library = unsafe { libc::dlopen(c"/usr/lib/libproc.dylib".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if proc_library.is_null() { return Err("libproc"); }
    let bsm_library = unsafe { libc::dlopen(c"/usr/lib/libbsm.dylib".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if bsm_library.is_null() {
        unsafe { libc::dlclose(proc_library); }
        return Err("libbsm");
    }
    let result = (|| {
        // SAFETY: each resolved function is cast to its exact public SDK
        // declaration. No symbol is called until all required symbols exist.
        Ok(unsafe { NativeApi {
            signal: std::mem::transmute::<*mut libc::c_void, Signal>(symbol(proc_library, c"proc_signal_with_audittoken")?),
            path: std::mem::transmute::<*mut libc::c_void, TokenPath>(symbol(proc_library, c"proc_pidpath_audittoken")?),
            token_pid: std::mem::transmute::<*mut libc::c_void, TokenPid>(symbol(bsm_library, c"audit_token_to_pid")?),
            token_pidversion: std::mem::transmute::<*mut libc::c_void, TokenVersion>(symbol(bsm_library, c"audit_token_to_pidversion")?),
            token_euid: std::mem::transmute::<*mut libc::c_void, TokenUid>(symbol(bsm_library, c"audit_token_to_euid")?),
            token_ruid: std::mem::transmute::<*mut libc::c_void, TokenUid>(symbol(bsm_library, c"audit_token_to_ruid")?),
            _library_handles: [proc_library as usize, bsm_library as usize],
        } })
    })();
    if result.is_err() {
        unsafe { libc::dlclose(bsm_library); libc::dlclose(proc_library); }
    }
    result
}

unsafe fn symbol(library: *mut libc::c_void, name: &'static CStr) -> Result<*mut libc::c_void, &'static str> {
    // SAFETY: library remains open and name is a static NUL-terminated string.
    let address = unsafe { libc::dlsym(library, name.as_ptr()) };
    require_symbol(address, name)
}

fn require_symbol(address: *mut libc::c_void, name: &'static CStr) -> Result<*mut libc::c_void, &'static str> {
    if address.is_null() { Err(name.to_str().unwrap_or("unknown native symbol")) } else { Ok(address) }
}

fn signal_result(result: libc::c_int) -> io::Result<()> {
    match result {
        0 => Ok(()),
        error if error > 0 => Err(io::Error::from_raw_os_error(error)),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData,
            "Darwin audit-token signal returned a negative result")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_errors_use_positive_returned_errno_without_ambient_errno() {
        assert!(signal_result(0).is_ok());
        assert_eq!(signal_result(libc::ESRCH).unwrap_err().raw_os_error(), Some(libc::ESRCH));
        assert_eq!(signal_result(libc::EPERM).unwrap_err().raw_os_error(), Some(libc::EPERM));
        assert_eq!(signal_result(-1).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn missing_exact_signal_symbol_is_rejected_without_a_fallback() {
        assert_eq!(require_symbol(std::ptr::null_mut(), c"proc_signal_with_audittoken"),
            Err("proc_signal_with_audittoken"));
    }
}
