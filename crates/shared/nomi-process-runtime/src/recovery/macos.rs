//! Recovery of a durable root's original Darwin process group.
//!
//! The birth record is admission evidence, not signal authority. Admission
//! binds it to a stable kernel generation, and every signal uses that
//! generation in the kernel. The frozen, live root anchors PGID ownership
//! until all admitted members are terminal; it is terminated last. A vanished
//! root never authorizes another lookup/signalling pass over a cached PGID.
//! Unrecorded descendants that left the original group are outside this proof.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::*;

const PROC_PIDUNIQIDENTIFIERINFO: libc::c_int = 17;
// Current Xcode and CLT SDK usr/include/sys/proc_info.h: PROC_PGRP_ONLY = 2.
// proc_listpids returns bytes; its proc_listpgrppids wrapper returns PID count.
const PROC_PGRP_ONLY: u32 = 2;
const MAX_GROUP_MEMBERS: usize = 16_384;
const POLL_INTERVAL: Duration = Duration::from_millis(2);

// XNU bsd/sys/proc_info_private.h declares this as a 56-byte API structure.
// libc currently omits the flavor and structure. Resolve the signal symbol
// dynamically so unsupported systems fail closed instead of failing to load.
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
struct AuditToken {
    values: [u32; 8],
}

type SignalWithAuditToken = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;
static SIGNAL: OnceLock<Option<SignalWithAuditToken>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Generation {
    pid: u32,
    version: u32,
    unique: u64,
}

#[derive(Clone, Debug)]
struct Snapshot {
    generation: Generation,
    pgid: u32,
    stopped: bool,
    zombie: bool,
    identity: ExactProcessIdentity,
}

pub(super) fn probe(pid: u32) -> io::Result<Option<ExactProcessIdentity>> {
    let Some(snapshot) = process_snapshot(pid, true)? else {
        return Ok(None);
    };
    // Zombies are terminal even when launchd/their parent has not reaped them.
    // They cannot execute or create another process and are absence evidence.
    Ok((!snapshot.zombie).then_some(snapshot.identity))
}

fn native_signal() -> io::Result<SignalWithAuditToken> {
    let function = SIGNAL.get_or_init(|| {
        // SAFETY: dlsym reads the loaded libproc export; the signature matches
        // Apple's libsyscall/wrappers/libproc/libproc.c declaration.
        let pointer = unsafe {
            libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr())
        };
        (!pointer.is_null()).then(|| unsafe {
            std::mem::transmute::<*mut libc::c_void, SignalWithAuditToken>(pointer)
        })
    });
    function.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "macOS orphan recovery requires kernel-checked process-generation signaling",
        )
    })
}

/// False means that this execution generation was not found; it says nothing
/// about a new execution generation or another process occupying the PID.
fn signal_exact(generation: Generation, signal: libc::c_int) -> io::Result<bool> {
    let mut token = AuditToken { values: [0; 8] };
    token.values[5] = generation.pid;
    token.values[7] = generation.version;
    // XNU proc_find_audit_token compares PID version and retains the exact
    // proc reference across signalling. An already queued signal may follow
    // an imminent exec under Darwin's ordinary signal semantics.
    let result = unsafe { native_signal()?(&raw mut token, signal) };
    match result {
        0 => Ok(true),
        libc::ESRCH => Ok(false),
        errno => Err(io::Error::from_raw_os_error(errno)),
    }
}

fn read_info<T>(pid: u32, flavor: libc::c_int) -> io::Result<Option<T>> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "PID exceeds the Darwin PID range")
    })?;
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let size = std::mem::size_of::<T>() as libc::c_int;
    // SAFETY: value has the exact advertised ABI size. Argument 1 requests
    // zombie records for these flavors, rather than hiding them as ESRCH.
    let written = unsafe { libc::proc_pidinfo(pid, flavor, 1, value.as_mut_ptr().cast(), size) };
    if written != size {
        let error = io::Error::last_os_error();
        if written <= 0 && error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        return Err(if written > 0 || error.raw_os_error() == Some(0) {
            io::Error::new(io::ErrorKind::InvalidData, "incomplete Darwin process-info record")
        } else {
            error
        });
    }
    Ok(Some(unsafe { value.assume_init() }))
}

fn process_snapshot(pid: u32, include_executable: bool) -> io::Result<Option<Snapshot>> {
    let Some(before) = read_info::<UniqueInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO)? else {
        return Ok(None);
    };
    let Some(bsd) = read_info::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO)? else {
        return Ok(None);
    };
    let executable = include_executable.then(|| executable_path(pid)).flatten();
    let Some(after) = read_info::<UniqueInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO)? else {
        return Ok(None);
    };
    if before.unique_id != after.unique_id || before.pid_version != after.pid_version {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Darwin process generation changed during snapshot",
        ));
    }
    if bsd.pbi_pid != pid || after.unique_id == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "inconsistent Darwin process identity"));
    }
    if bsd.pbi_start_tvusec >= 1_000_000 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid Darwin process birth microseconds"));
    }
    let platform_start_key = u64::from(bsd.pbi_start_tvsec)
        .checked_mul(1_000_000)
        .and_then(|micros| micros.checked_add(u64::from(bsd.pbi_start_tvusec)))
        .filter(|key| *key != 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid Darwin process birth key"))?;
    Ok(Some(Snapshot {
        generation: Generation { pid, version: after.pid_version as u32, unique: after.unique_id },
        pgid: bsd.pbi_pgid,
        stopped: bsd.pbi_status == libc::SSTOP,
        zombie: bsd.pbi_status == libc::SZOMB,
        identity: ExactProcessIdentity {
            pid,
            start_time_epoch_seconds: u64::from(bsd.pbi_start_tvsec),
            platform_start_key,
            executable,
        },
    }))
}

fn executable_path(pid: u32) -> Option<PathBuf> {
    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the PID has already passed read_info's range validation and the
    // buffer is writable for its full advertised length.
    let written = unsafe {
        libc::proc_pidpath(pid as libc::pid_t, buffer.as_mut_ptr().cast(), buffer.len() as u32)
    };
    if written <= 0 { return None; }
    buffer.truncate(written as usize);
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
}

/// Only resumes executions this recovery actually stopped. Every rollback
/// signal retains its original token; stale tokens never resume a new owner.
struct FrozenGroup {
    stopped: BTreeMap<Generation, bool>,
}

impl FrozenGroup {
    fn new() -> Self { Self { stopped: BTreeMap::new() } }

    fn stop(&mut self, member: &Snapshot) -> io::Result<()> {
        if member.zombie { return Ok(()); }
        self.stopped.entry(member.generation)
            .and_modify(|resume| *resume |= !member.stopped)
            .or_insert(!member.stopped);
        signal_exact(member.generation, libc::SIGSTOP)?;
        Ok(())
    }
}

impl Drop for FrozenGroup {
    fn drop(&mut self) {
        for (&generation, &resume) in &self.stopped {
            if resume {
                let _ = signal_exact(generation, libc::SIGCONT);
            }
        }
    }
}

fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(io::ErrorKind::TimedOut,
            "macOS orphan process-group sealing/termination timed out"));
    }
    Ok(())
}

fn pause(deadline: Instant) {
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(POLL_INTERVAL.min(remaining));
    }
}

fn verify_anchor(root: Generation) -> io::Result<()> {
    match process_snapshot(root.pid, false)? {
        Some(snapshot) if snapshot.generation == root && !snapshot.zombie
            && snapshot.pgid == root.pid && snapshot.stopped => Ok(()),
        _ => Err(io::Error::other(
            "frozen recovery root lost its live process-group anchor; refusing further group authority",
        )),
    }
}

fn group_pids(pgid: u32) -> io::Result<Vec<u32>> {
    // A full buffer is ambiguous: never interpret a truncated list as a seal.
    let mut pids = vec![0 as libc::pid_t; MAX_GROUP_MEMBERS];
    let capacity_bytes = std::mem::size_of_val(&pids[..]) as libc::c_int;
    // SAFETY: pids is writable for capacity_bytes. This enumerates the original
    // group only and gives no signal authority without a verified live anchor.
    let (bytes, errno) = unsafe {
        // libproc maps syscall failure to zero, also the valid empty result.
        // Clear thread-local errno so failure is never an empty-group proof.
        *libc::__error() = 0;
        let bytes = libc::proc_listpids(PROC_PGRP_ONLY, pgid, pids.as_mut_ptr().cast(), capacity_bytes);
        (bytes, *libc::__error())
    };
    if bytes < 0 || bytes == 0 && errno != 0 {
        return Err(io::Error::from_raw_os_error(if errno == 0 { libc::EIO } else { errno }));
    }
    if bytes >= capacity_bytes || bytes as usize % std::mem::size_of::<libc::pid_t>() != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "macOS recovery process-group enumeration is incomplete"));
    }
    let count = bytes as usize / std::mem::size_of::<libc::pid_t>();
    let mut result = Vec::with_capacity(count);
    for &pid in &pids[..count] {
        if pid <= 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "invalid PID in macOS recovery process-group enumeration"));
        }
        result.push(pid as u32);
    }
    result.sort_unstable();
    if result.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "duplicate PID in macOS recovery process-group enumeration"));
    }
    Ok(result)
}

struct GroupSnapshot {
    members: BTreeMap<u32, Snapshot>,
    complete: bool,
}

fn group_snapshot(root: Generation, baseline: &BTreeMap<u32, Snapshot>) -> io::Result<GroupSnapshot> {
    verify_anchor(root)?;
    let pids = group_pids(root.pid)?;
    let snapshot = observe_members(root.pid, pids, baseline, |pid| process_snapshot(pid, false))?;
    verify_anchor(root)?;
    if !snapshot.members.contains_key(&root.pid) {
        return Err(io::Error::other("process-group enumeration omitted its live frozen root"));
    }
    Ok(snapshot)
}

fn observe_members(
    pgid: u32,
    pids: Vec<u32>,
    baseline: &BTreeMap<u32, Snapshot>,
    mut read: impl FnMut(u32) -> io::Result<Option<Snapshot>>,
) -> io::Result<GroupSnapshot> {
    let mut members = BTreeMap::new();
    let mut complete = true;
    for pid in pids {
        let member = match read(pid) {
            Ok(Some(member)) => member,
            Ok(None) => { complete = false; continue; }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                complete = false;
                continue;
            }
            Err(error) => return Err(error),
        };
        // A process observed running before the PID list could fork and exit
        // after that list. Even a zombie seen afterward cannot establish that
        // its child was included. Require this exact generation to have been
        // stopped/terminal in the baseline *before* the current kernel list.
        if !baseline.get(&pid).is_some_and(|prior|
            prior.generation == member.generation && (prior.stopped || prior.zombie)) {
            complete = false;
        }
        if member.pgid != pgid {
            complete = false;
            continue;
        }
        members.insert(pid, member);
    }
    Ok(GroupSnapshot { members, complete })
}

fn generation_terminal(generation: Generation) -> io::Result<bool> {
    Ok(match process_snapshot(generation.pid, false)? {
        None => true,
        // A completed exec invalidates the signal token, but the same process
        // remains live. Token ESRCH/pidversion changes are not termination
        // proof. Unique ID changes, unlike exec, prove the original process
        // instance no longer occupies this PID.
        Some(snapshot) => snapshot.generation.unique != generation.unique || snapshot.zombie,
    })
}

/// An absent root grants no new signal authority. Observing its numeric PGID
/// is still useful: live members/reuse retain an unknown recovery record;
/// complete no-live observation proves only that original contained group.
fn observe_root_gone_group(pgid: u32, deadline: Instant) -> io::Result<OrphanTerminationOutcome> {
    let mut baseline = BTreeMap::new();
    loop {
        check_deadline(deadline)?;
        let snapshot = observe_members(pgid, group_pids(pgid)?, &baseline,
            |pid| process_snapshot(pid, false))?;
        if snapshot.members.values().any(|member| !member.zombie) {
            return Ok(OrphanTerminationOutcome::RootGoneCleanupUnproven);
        }
        if snapshot.complete {
            return Ok(OrphanTerminationOutcome::AlreadyDead);
        }
        baseline = snapshot.members;
        pause(deadline);
    }
}

fn confirm_members_terminal(members: &[Generation], root: Generation, deadline: Instant) -> io::Result<()> {
    loop {
        check_deadline(deadline)?;
        verify_anchor(root)?;
        let mut all_terminal = true;
        for &member in members {
            all_terminal &= generation_terminal(member)?;
        }
        verify_anchor(root)?;
        if all_terminal { return Ok(()); }
        pause(deadline);
    }
}

pub(super) fn terminate(identity: &ExactProcessIdentity, timeout: Duration) -> io::Result<OrphanTerminationOutcome> {
    let deadline = Instant::now().checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
    if identity.pid <= 1 || identity.platform_start_key == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid macOS recovery root"));
    }
    let Some(root_snapshot) = process_snapshot(identity.pid, true)? else {
        return observe_root_gone_group(identity.pid, deadline);
    };
    if root_snapshot.zombie || !same_recorded_process(identity, &root_snapshot.identity) {
        return observe_root_gone_group(identity.pid, deadline);
    }
    if root_snapshot.pgid != identity.pid {
        return Err(io::Error::other(
            "recorded process is not its own process-group leader; refusing recovery"));
    }
    if unsafe { libc::getpgrp() } as u32 == identity.pid {
        return Err(io::Error::other("refusing to terminate the current application process group"));
    }
    native_signal()?;

    let root = root_snapshot.generation;
    let mut frozen = FrozenGroup::new();
    check_deadline(deadline)?;
    frozen.stop(&root_snapshot)?;
    // No additional member has been admitted yet. A root that disappeared
    // here is already dead; a replacement/exec cannot authorize group recovery.
    loop {
        check_deadline(deadline)?;
        match process_snapshot(root.pid, false)? {
            None => return observe_root_gone_group(identity.pid, deadline),
            Some(snapshot) if snapshot.zombie => return observe_root_gone_group(identity.pid, deadline),
            Some(snapshot) if snapshot.generation != root => {
                return observe_root_gone_group(identity.pid, deadline);
            }
            Some(snapshot) if snapshot.pgid != root.pid => {
                return Err(io::Error::other("recovery root left its process group while freezing"));
            }
            Some(snapshot) if snapshot.stopped => break,
            Some(_) => pause(deadline),
        }
    }

    #[cfg(test)]
    run_after_root_stop_hook();

    let mut baseline = BTreeMap::new();
    let sealed = loop {
        check_deadline(deadline)?;
        let snapshot = group_snapshot(root, &baseline)?;
        if snapshot.complete && snapshot.members.values().all(|member| member.stopped || member.zombie) {
            break snapshot.members;
        }
        // All identities were admitted in a snapshot bracketed by root-anchor
        // checks. Each token stays exact even if the root vanishes afterward.
        verify_anchor(root)?;
        for member in snapshot.members.values() {
            frozen.stop(member)?;
        }
        verify_anchor(root)?;
        baseline = snapshot.members;
        pause(deadline);
    };

    let members: Vec<_> = sealed.values().filter(|member| member.generation != root)
        .map(|member| member.generation).collect();
    verify_anchor(root)?;
    for &member in &members {
        signal_exact(member, libc::SIGKILL)?;
    }
    confirm_members_terminal(&members, root, deadline)?;
    let final_group = group_snapshot(root, &sealed)?;
    if !final_group.complete || final_group.members.values().any(|member|
        member.generation != root && !member.zombie) {
        return Err(io::Error::other("macOS recovery could not prove the original group terminal"));
    }
    // Root is last: once it is signalled, never enumerate the numeric PGID to
    // authorize another process. Only the admitted root generation is probed.
    verify_anchor(root)?;
    signal_exact(root, libc::SIGKILL)?;
    loop {
        check_deadline(deadline)?;
        if generation_terminal(root)? {
            return Ok(OrphanTerminationOutcome::KilledAndProven);
        }
        pause(deadline);
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_ROOT_STOP: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_after_root_stop_hook() {
    AFTER_ROOT_STOP.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() { hook(); }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::os::unix::process::CommandExt;

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct ExactCleanup(Generation);
    impl Drop for ExactCleanup {
        fn drop(&mut self) { let _ = signal_exact(self.0, libc::SIGKILL); }
    }

    fn sleeper() -> OwnedChild {
        OwnedChild(Command::new("/bin/sleep").arg("60").process_group(0)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap())
    }

    fn read_marker(path: &std::path::Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(value) = std::fs::read_to_string(path) {
                if let Ok(pid) = value.trim().parse() { return pid; }
            }
            assert!(Instant::now() < deadline, "child failed to publish its PID");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn sleep_generation(pid: u32) -> Generation {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(Some(snapshot)) = process_snapshot(pid, true) {
                if snapshot.identity.executable.as_ref().is_some_and(|path|
                    path.file_name() == Some(std::ffi::OsStr::new("sleep"))) {
                    return snapshot.generation;
                }
            }
            assert!(Instant::now() < deadline, "descendant failed to exec sleep");
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    #[test]
    fn native_signal_rejects_replaced_execution_generation_at_signal_boundary() {
        let child = sleeper();
        let actual = process_snapshot(child.0.id(), false).unwrap().unwrap().generation;
        let stale = Generation { version: actual.version.wrapping_add(1), ..actual };
        assert!(!signal_exact(stale, libc::SIGKILL).unwrap(),
            "a stale generation must be rejected inside the signal syscall");
        assert_eq!(process_snapshot(actual.pid, false).unwrap().unwrap().generation, actual);
        assert!(!generation_terminal(actual).unwrap(), "the current PID owner must still run");
        assert!(!generation_terminal(stale).unwrap(),
            "token ESRCH cannot prove the same process instance terminal");
    }

    #[test]
    fn original_group_recovery_proves_all_descendants_terminal() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("child.pid");
        let root = OwnedChild(Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "recovery-root"])
            .arg(&marker).process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap());
        let descendant = read_marker(&marker);
        let root_identity = probe(root.0.id()).unwrap().unwrap();
        let descendant_generation = sleep_generation(descendant);
        let _descendant_cleanup = ExactCleanup(descendant_generation);
        assert_eq!(process_snapshot(descendant, false).unwrap().unwrap().pgid, root.0.id());
        assert_eq!(terminate(&root_identity, Duration::from_secs(5)).unwrap(), OrphanTerminationOutcome::KilledAndProven);
        assert!(generation_terminal(descendant_generation).unwrap(), "success must include the descendant's terminal proof");
        assert!(probe(root.0.id()).unwrap().is_none());
    }

    #[test]
    fn public_recovery_retains_unproven_group_when_root_already_gone() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("child.pid");
        let mut root = OwnedChild(Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "recovery-root"])
            .arg(&marker).process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap());
        let descendant = read_marker(&marker);
        let descendant_generation = sleep_generation(descendant);
        let _descendant_cleanup = ExactCleanup(descendant_generation);
        let identity = probe(root.0.id()).unwrap().unwrap();
        root.0.kill().unwrap();
        root.0.wait().unwrap();
        assert!(probe(identity.pid).unwrap().is_none());
        assert_eq!(crate::terminate_verified_orphan(&identity, Duration::from_secs(5)).unwrap(),
            OrphanTerminationOutcome::RootGoneCleanupUnproven);
        assert!(!generation_terminal(descendant_generation).unwrap(),
            "an already dead root grants no authority to signal its remaining members");
    }

    #[test]
    fn mismatched_root_never_signals_numeric_group_and_retains_unknown_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("child.pid");
        let root = OwnedChild(Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "recovery-root"])
            .arg(&marker).process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap());
        let descendant = read_marker(&marker);
        let descendant_generation = sleep_generation(descendant);
        let _descendant_cleanup = ExactCleanup(descendant_generation);
        let mut identity = probe(root.0.id()).unwrap().unwrap();
        identity.platform_start_key += 1;
        assert_eq!(crate::terminate_verified_orphan(&identity, Duration::from_secs(5)).unwrap(),
            OrphanTerminationOutcome::RootGoneCleanupUnproven);
        assert!(!process_snapshot(root.0.id(), false).unwrap().unwrap().stopped,
            "mismatched root cannot authorize even group freezing");
        assert!(!generation_terminal(descendant_generation).unwrap());
    }

    #[test]
    fn vanished_root_cannot_authorize_cached_group_or_claim_tree_success() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("child.pid");
        let root = OwnedChild(Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "recovery-root"])
            .arg(&marker).process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap());
        let descendant = read_marker(&marker);
        let root_identity = probe(root.0.id()).unwrap().unwrap();
        let root_generation = process_snapshot(root.0.id(), false).unwrap().unwrap().generation;
        let descendant_generation = sleep_generation(descendant);
        let _descendant_cleanup = ExactCleanup(descendant_generation);
        AFTER_ROOT_STOP.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
            assert!(signal_exact(root_generation, libc::SIGKILL).unwrap());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !generation_terminal(root_generation).unwrap() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(POLL_INTERVAL);
            }
        })));
        let result = terminate(&root_identity, Duration::from_secs(5));
        // Clear a hook even if admission failed before it could be consumed.
        AFTER_ROOT_STOP.with(|hook| hook.borrow_mut().take());
        let descendant_survived = !generation_terminal(descendant_generation).unwrap();
        signal_exact(descendant_generation, libc::SIGKILL).unwrap();
        assert!(result.is_err(), "leader disappearance must not become a group/tree success");
        assert!(descendant_survived, "lost root must not authorize cached-PGID cleanup");
    }

    #[test]
    fn snapshot_requires_pre_enumeration_terminal_or_stopped_generation() {
        let child = sleeper();
        let live = process_snapshot(child.0.id(), false).unwrap().unwrap();
        let mut terminal = live.clone();
        terminal.zombie = true;
        let mut baseline = BTreeMap::from([(live.generation.pid, live.clone())]);
        let pids = vec![live.generation.pid];
        let snapshot = observe_members(live.pgid, pids.clone(), &baseline, |_| Ok(Some(terminal.clone()))).unwrap();
        assert!(!snapshot.complete, "running parent could fork after the PID list and then exit");
        baseline.insert(live.generation.pid, terminal.clone());
        assert!(observe_members(live.pgid, pids.clone(), &baseline, |_| Ok(Some(terminal.clone()))).unwrap().complete);
        terminal.generation.version = terminal.generation.version.wrapping_add(1);
        assert!(!observe_members(live.pgid, pids, &baseline, |_| Ok(Some(terminal.clone()))).unwrap().complete,
            "a reused PID or completed exec invalidates the prior baseline");
    }
}
