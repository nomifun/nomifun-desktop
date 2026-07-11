#![cfg(target_os = "linux")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    process::{Child, Command, ExitStatus},
    time::Duration,
};

fn helper_binary() -> &'static str {
    option_env!("CARGO_BIN_EXE_execution_test_helper")
        .expect("Cargo did not build the execution_test_helper binary")
}

fn harness_binary() -> &'static str {
    option_env!("CARGO_BIN_EXE_parent_death_harness")
        .expect("Cargo did not build the parent_death_harness binary")
}

#[tokio::test]
#[serial_test::serial]
async fn abrupt_harness_exit_kills_and_reaps_the_owned_process_group() {
    let _subreaper = SubreaperGuard::install().expect("test process should become a subreaper");
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let leader_marker = directory.path().join("leader.pid");
    let grandchild_marker = directory.path().join("grandchild.pid");
    let child = Command::new(harness_binary())
        .arg(helper_binary())
        .arg(&leader_marker)
        .arg(&grandchild_marker)
        .spawn()
        .expect("parent-death harness should spawn");
    let mut harness = HarnessCleanup(Some(child));

    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let child = harness.0.as_mut().expect("harness child should be owned");
            if let Some(status) = child.try_wait()? {
                return Ok::<_, io::Error>(status);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("parent-death harness should exit within its bounded setup window")
    .expect("harness should be reaped");
    harness.0.take();
    assert!(status.success(), "harness failed before deliberate _exit: {status:?}");
    let leader = read_pid(&leader_marker).expect("leader PID should be published");
    let grandchild = read_pid(&grandchild_marker).expect("grandchild PID should be published");
    let mut group_cleanup = GroupCleanup {
        pgid: leader,
        members: vec![leader, grandchild],
        armed: true,
    };

    let statuses = reap_owned_group(leader, grandchild)
        .await
        .expect("watchdog-owned descendants should become exactly reapable");

    assert_sigkill(statuses.get(&leader), "leader");
    assert_sigkill(statuses.get(&grandchild), "grandchild");
    assert!(
        statuses
            .iter()
            .any(|(pid, status)| *pid != leader && *pid != grandchild && was_sigkill(*status)),
        "the direct-child watchdog was not discovered and reaped: {statuses:?}"
    );
    assert!(!process_exists(leader));
    assert!(!process_exists(grandchild));
    group_cleanup.armed = false;
}

async fn reap_owned_group(
    leader: libc::pid_t,
    grandchild: libc::pid_t,
) -> io::Result<BTreeMap<libc::pid_t, ExitStatus>> {
    let mut statuses = BTreeMap::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut candidates = BTreeSet::from([leader, grandchild]);
            for pid in direct_children()? {
                if pid == leader || pid == grandchild || process_group(pid) == Some(leader) {
                    candidates.insert(pid);
                }
            }
            for pid in candidates {
                if statuses.contains_key(&pid) {
                    continue;
                }
                if let Some(status) = reap_exact_if_ready(pid)? {
                    statuses.insert(pid, status);
                }
            }
            let remaining_group_child = direct_children()?.into_iter().any(|pid| {
                pid == leader || pid == grandchild || process_group(pid) == Some(leader)
            });
            if !process_exists(leader)
                && !process_exists(grandchild)
                && !remaining_group_child
            {
                return Ok(statuses);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "adopted process-group reap timed out"))?
}

fn direct_children() -> io::Result<BTreeSet<libc::pid_t>> {
    let mut children = BTreeSet::new();
    for task in fs::read_dir("/proc/self/task")? {
        let path = task?.path().join("children");
        let Ok(contents) = fs::read_to_string(path) else {
            continue;
        };
        for field in contents.split_whitespace() {
            if let Ok(pid) = field.parse::<libc::pid_t>() {
                children.insert(pid);
            }
        }
    }
    Ok(children)
}

fn process_group(pid: libc::pid_t) -> Option<libc::pid_t> {
    // SAFETY: getpgid only inspects the identity named by pid.
    let pgid = unsafe { libc::getpgid(pid) };
    (pgid >= 0).then_some(pgid)
}

fn reap_exact_if_ready(pid: libc::pid_t) -> io::Result<Option<ExitStatus>> {
    let mut status = 0;
    // SAFETY: pid is an exact child identity discovered from /proc or a published marker.
    let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if waited == pid {
        use std::os::unix::process::ExitStatusExt;
        return Ok(Some(ExitStatus::from_raw(status)));
    }
    if waited == 0 {
        return Ok(None);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ECHILD) {
        Ok(None)
    } else {
        Err(error)
    }
}

fn assert_sigkill(status: Option<&ExitStatus>, label: &str) {
    assert!(
        status.is_some_and(|status| was_sigkill(*status)),
        "{label} was not reaped from SIGKILL: {status:?}"
    );
}

fn was_sigkill(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.signal() == Some(libc::SIGKILL)
}

fn read_pid(path: &Path) -> io::Result<libc::pid_t> {
    fs::read_to_string(path)?
        .trim()
        .parse::<libc::pid_t>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn process_exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal zero probes liveness without delivering a signal.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

struct SubreaperGuard {
    previous: libc::c_int,
}

impl SubreaperGuard {
    fn install() -> io::Result<Self> {
        let mut previous = 0;
        // SAFETY: prctl writes one c_int to the supplied valid pointer.
        if unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut previous) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: PR_SET_CHILD_SUBREAPER accepts the integral enabled flag.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { previous })
    }
}

impl Drop for SubreaperGuard {
    fn drop(&mut self) {
        // SAFETY: restores the process-wide flag captured by install.
        let _ = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, self.previous) };
    }
}

struct HarnessCleanup(Option<Child>);

impl Drop for HarnessCleanup {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct GroupCleanup {
    pgid: libc::pid_t,
    members: Vec<libc::pid_t>,
    armed: bool,
}

impl Drop for GroupCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: this guard owns the helper-created process group.
        let _ = unsafe { libc::kill(-self.pgid, libc::SIGKILL) };
        for pid in &self.members {
            // SAFETY: these exact PIDs were published by the harness and helper.
            let _ = unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
    }
}
