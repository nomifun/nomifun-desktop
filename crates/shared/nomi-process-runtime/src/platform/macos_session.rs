//! Cleanup of job-control groups inside an owned controlling-PTY session.
//!
//! A shell may create a process group for each job. The shell's PGID alone is
//! therefore not a cleanup boundary. Callers must retain the unreaped session
//! leader while this module enumerates its session; cached numeric SIDs alone
//! never authorize signals. Every additional process is signaled by Darwin's
//! kernel-checked PID generation, rather than by a racy PID/PGID kill.

use super::unix_protocol::Deadline;

const PID_CAPACITY: usize = 16_384;
const SESSION_CAPACITY: usize = 1_024;
const SEAL_TIMEOUT_MS: u64 = 250;
const PROC_PIDUNIQIDENTIFIERINFO: libc::c_int = 17;
const PROC_PIDT_SHORTBSDINFO: libc::c_int = 13;

// Stable Darwin libproc ABI, declared in XNU bsd/sys/proc_info_private.h.
// libc does not currently expose this process-generation record.
#[repr(C)]
struct UniqueInfo {
    executable_uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    pid_version: i32,
    original_parent_pid_version: i32,
    reserved: [u64; 2],
}

#[repr(C)]
struct ShortBsdInfo {
    pid: u32,
    parent_pid: u32,
    pgid: u32,
    status: u32,
    command: [u8; 16],
    flags: u32,
    credentials_and_reserved: [u32; 7],
}

#[repr(C)]
struct AuditToken {
    values: [u32; 8],
}

type SignalWithAuditToken = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;
static SIGNAL_WITH_AUDIT_TOKEN: std::sync::OnceLock<Option<SignalWithAuditToken>> =
    std::sync::OnceLock::new();

/// Resolve and probe before fork. Older macOS versions must get a truthful
/// unsupported-PTY error, not a missing-symbol failure when loading the app.
/// The watchdog uses only the already initialized pointer (no loader locks).
pub(super) fn prepare_session_cleanup() -> std::io::Result<()> {
    let signal = SIGNAL_WITH_AUDIT_TOKEN.get_or_init(|| {
        let pointer = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
        if pointer.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut libc::c_void, SignalWithAuditToken>(pointer) })
        }
    });
    if signal.is_none() {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported,
            "macOS PTY cleanup requires kernel-checked process-generation signaling"));
    }
    let own = unsafe { process_identity(libc::getpid()) }
        .map_err(std::io::Error::from_raw_os_error)?
        .ok_or_else(|| std::io::Error::other("cannot read current macOS process identity"))?;
    // Darwin rejects signal zero for this API. SIGCONT to our already-running
    // process is harmless and exercises the real kernel permission path.
    unsafe { signal_identity(own, libc::SIGCONT) }.map_err(std::io::Error::from_raw_os_error)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Identity {
    pid: libc::pid_t,
    version: u32,
    unique: u64,
    stopped: bool,
    zombie: bool,
}

type Baselines = [[Identity; PID_CAPACITY]; 2];

/// The lifecycle poller has a deliberately small stack, and the watchdog is
/// forked from a multithreaded host. Anonymous mmap supplies bounded, zeroed
/// scratch memory without either a large stack frame or Rust allocator locks.
struct BaselineStorage(*mut Baselines);

impl BaselineStorage {
    fn new() -> Result<Self, libc::c_int> {
        let memory = unsafe {
            libc::mmap(std::ptr::null_mut(), std::mem::size_of::<Baselines>(),
                libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0)
        };
        if memory == libc::MAP_FAILED {
            Err(unsafe { *libc::__error() })
        } else {
            // All-zero Identity values (including bools) are valid.
            Ok(Self(memory.cast()))
        }
    }
}

impl Drop for BaselineStorage {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.0.cast(), std::mem::size_of::<Baselines>()) };
    }
}

/// Seal every ordinary job in an owned PTY session, including separate PGIDs.
///
/// # Safety
/// The host caller must own the exact unreaped session leader throughout this
/// call. A forked watchdog instead supplies its original parent PID: its host
/// retains that lease until the watchdog exits. Each complete snapshot is
/// bracketed by direct-parent checks, and signals use only snapshot identities.
/// This does not extend ownership to descendants that deliberately call setsid.
pub(super) unsafe fn seal_owned_session(
    leader: libc::pid_t,
    watchdog_parent: Option<libc::pid_t>,
) -> Result<(), libc::c_int> {
    if leader <= 1 {
        return Err(libc::EINVAL);
    }
    let deadline = Deadline::after(std::time::Duration::from_millis(SEAL_TIMEOUT_MS))
        .map_err(|_| libc::EIO)?;
    let storage = BaselineStorage::new()?;
    let baselines = unsafe { &mut *storage.0 };
    let mut baseline_count = 0;
    let mut first_is_baseline = true;
    loop {
        if deadline.is_expired().map_err(|_| libc::EIO)? {
            return Err(libc::ETIMEDOUT);
        }
        let mut members = [Identity::default(); SESSION_CAPACITY];
        let (first, second) = baselines.split_at_mut(1);
        let (baseline, next_baseline) = if first_is_baseline {
            (&first[0][..baseline_count], &mut second[0][..])
        } else {
            (&second[0][..baseline_count], &mut first[0][..])
        };
        let (count, complete) = unsafe {
            session_snapshot(leader, watchdog_parent, &mut members, baseline, next_baseline, &mut baseline_count)
        }?;
        first_is_baseline = !first_is_baseline;
        if count == 0 && complete {
            return Ok(());
        }
        let members = &members[..count];
        // Freeze before killing: a shell or job can fork while a prior scan is
        // in progress. Rescan until all remaining members are stopped, so a
        // parent exiting during enumeration cannot hide a newly created job.
        let signal = if complete && members.iter().all(|member| member.stopped) {
            libc::SIGKILL
        } else {
            libc::SIGSTOP
        };
        for member in members {
            unsafe { signal_identity(*member, signal) }?;
        }
        unsafe { libc::poll(std::ptr::null_mut(), 0, 1) };
    }
}

unsafe fn session_snapshot(
    leader: libc::pid_t,
    watchdog_parent: Option<libc::pid_t>,
    members: &mut [Identity],
    baseline: &[Identity],
    next_baseline: &mut [Identity],
    next_count: &mut usize,
) -> Result<(usize, bool), libc::c_int> {
    check_parent_lease(watchdog_parent)?;
    let mut pids = [0 as libc::pid_t; PID_CAPACITY];
    let count = unsafe {
        libc::proc_listallpids(pids.as_mut_ptr().cast(), std::mem::size_of_val(&pids) as libc::c_int)
    };
    if count <= 0 || count as usize >= pids.len() {
        return Err(libc::EOVERFLOW);
    }
    let mut used = 0;
    let mut complete = true;
    *next_count = 0;
    for &pid in &pids[..count as usize] {
        if pid <= 1 {
            continue;
        }
        let before = match unsafe { process_identity(pid) } {
            Ok(Some(identity)) => identity,
            Ok(None) | Err(libc::EAGAIN) => { complete = false; continue; }
            Err(errno) => return Err(errno),
        };
        // The baseline predates the kernel PID list. A generation first seen
        // afterward cannot exclude a fork/exit/reuse that hid another job.
        let prior = baseline.binary_search_by_key(&pid, |prior| prior.pid)
            .ok().map(|index| &baseline[index]).filter(|prior| same_process(**prior, before));
        if prior.is_none() {
            complete = false;
        }
        if before.zombie {
            // A newly observed zombie could have forked after the PID list
            // was taken. Only a generation already known dead before this
            // enumeration may be omitted from a complete snapshot.
            if !prior.is_some_and(|prior| prior.zombie) {
                complete = false;
            }
            next_baseline[*next_count] = before;
            *next_count += 1;
            continue;
        }
        let sid = unsafe { libc::getsid(pid) };
        let after = match unsafe { process_identity(pid) } {
            Ok(Some(identity)) => identity,
            Ok(None) | Err(libc::EAGAIN) => { complete = false; continue; }
            Err(errno) => return Err(errno),
        };
        if sid < 0 || !same_process(before, after) || after.zombie {
            complete = false;
            continue;
        }
        next_baseline[*next_count] = after;
        *next_count += 1;
        if sid != leader {
            continue;
        }
        if used == members.len() {
            return Err(libc::EOVERFLOW);
        }
        members[used] = after;
        used += 1;
    }
    next_baseline[..*next_count].sort_unstable_by_key(|identity| identity.pid);
    check_parent_lease(watchdog_parent)?;
    Ok((used, complete))
}

fn same_process(left: Identity, right: Identity) -> bool {
    left.pid == right.pid && left.version == right.version && left.unique == right.unique
}

fn check_parent_lease(parent: Option<libc::pid_t>) -> Result<(), libc::c_int> {
    if parent.is_some_and(|pid| pid <= 1 || unsafe { libc::getppid() } != pid) {
        Err(libc::ECHILD)
    } else {
        Ok(())
    }
}

unsafe fn process_identity(pid: libc::pid_t) -> Result<Option<Identity>, libc::c_int> {
    // Both flavors permit inspecting other UIDs. Reading the generation on
    // either side binds the short BSD status to the same process incarnation.
    let Some(before) = (unsafe { read_info::<UniqueInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO) })? else { return Ok(None); };
    let Some(bsd) = (unsafe { read_info::<ShortBsdInfo>(pid, PROC_PIDT_SHORTBSDINFO) })? else { return Ok(None); };
    let Some(after) = (unsafe { read_info::<UniqueInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO) })? else { return Ok(None); };
    if before.unique_id != after.unique_id || before.pid_version != after.pid_version {
        return Err(libc::EAGAIN);
    }
    if bsd.pid != pid as u32 || after.unique_id == 0 {
        return Err(libc::EPROTO);
    }
    Ok(Some(Identity {
        pid,
        version: after.pid_version as u32,
        unique: after.unique_id,
        stopped: bsd.status == libc::SSTOP,
        zombie: bsd.status == libc::SZOMB,
    }))
}

unsafe fn read_info<T>(pid: libc::pid_t, flavor: libc::c_int) -> Result<Option<T>, libc::c_int> {
    let mut info = std::mem::MaybeUninit::<T>::zeroed();
    let expected = std::mem::size_of::<T>() as libc::c_int;
    // Nonzero argument includes zombies instead of treating them as absent.
    let returned = unsafe { libc::proc_pidinfo(pid, flavor, 1, info.as_mut_ptr().cast(), expected) };
    if returned != expected {
        let errno = unsafe { *libc::__error() };
        return if errno == libc::ESRCH { Ok(None) } else { Err(if errno == 0 { libc::EPROTO } else { errno }) };
    }
    Ok(Some(unsafe { info.assume_init() }))
}

unsafe fn signal_identity(identity: Identity, signal: libc::c_int) -> Result<(), libc::c_int> {
    let mut token = AuditToken { values: [0; 8] };
    token.values[5] = identity.pid as u32;
    token.values[7] = identity.version;
    let function = SIGNAL_WITH_AUDIT_TOKEN.get().and_then(|function| *function).ok_or(libc::ENOTSUP)?;
    // libproc returns an errno value directly, unlike kill(2).
    match unsafe { function(&mut token, signal) } {
        0 | libc::ESRCH => Ok(()),
        errno => Err(errno),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darwin_signal_rejects_wrong_pid_generation() {
        prepare_session_cleanup().unwrap();
        let own = unsafe { process_identity(libc::getpid()) }.unwrap().unwrap();
        assert_eq!(unsafe { signal_identity(own, libc::SIGCONT) }, Ok(()));
        let wrong = Identity { version: own.version.wrapping_add(1), ..own };
        let mut token = AuditToken { values: [0; 8] };
        token.values[5] = wrong.pid as u32;
        token.values[7] = wrong.version;
        let function = SIGNAL_WITH_AUDIT_TOKEN.get().unwrap().unwrap();
        assert_eq!(unsafe { function(&mut token, libc::SIGCONT) }, libc::ESRCH);
        assert!(unsafe { process_identity(own.pid) }.unwrap().is_some());
    }

    #[test]
    fn empty_session_snapshot_rejects_new_or_reused_process_generations() {
        // Observation only: choose an impossible SID, so a missing member must
        // never disguise an incomplete process enumeration as a sealed session.
        let mut members = [Identity::default(); SESSION_CAPACITY];
        let mut baseline = [Identity::default(); PID_CAPACITY];
        let mut count = 0;
        let first = unsafe {
            session_snapshot(i32::MAX, None, &mut members, &[], &mut baseline, &mut count)
        }.unwrap();
        assert_eq!(first, (0, false), "no identity baseline cannot prove emptiness");

        let own_pid = unsafe { libc::getpid() };
        let index = baseline[..count].binary_search_by_key(&own_pid, |identity| identity.pid).unwrap();
        baseline[index].version = baseline[index].version.wrapping_add(1);
        let mut next = [Identity::default(); PID_CAPACITY];
        let mut next_count = 0;
        let reused = unsafe {
            session_snapshot(i32::MAX, None, &mut members, &baseline[..count], &mut next, &mut next_count)
        }.unwrap();
        assert_eq!(reused, (0, false), "numeric PID equality cannot accept a new generation");

        // A fork after the baseline creates a new PID in the next kernel list.
        // Keep its exact StdChild lease and reap it even if the assertion fails.
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let created = unsafe {
            session_snapshot(i32::MAX, None, &mut members, &next[..next_count], &mut baseline, &mut count)
        };
        let _ = child.kill();
        child.wait().unwrap();
        assert_eq!(created.unwrap(), (0, false), "a newly forked process requires another scan");
    }
}
