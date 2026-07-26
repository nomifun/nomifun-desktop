#![cfg(unix)]

use std::fs;
use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

fn harness_binary() -> &'static str {
    env!("CARGO_BIN_EXE_terminal_parent_death_harness")
}

#[tokio::test]
async fn abrupt_backend_exit_kills_terminal_leader_and_grandchild() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let leader_marker = directory.path().join("leader.pid");
    let grandchild_marker = directory.path().join("grandchild.pid");
    let child = Command::new(harness_binary())
        .arg(&leader_marker)
        .arg(&grandchild_marker)
        .spawn()
        .expect("terminal parent-death harness should spawn");
    let mut harness = HarnessCleanup(Some(child));

    let status = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(status) = harness.0.as_mut().unwrap().try_wait()? {
                return Ok::<_, io::Error>(status);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("harness should deliberately _exit")
    .expect("harness wait should succeed");
    harness.0.take();
    assert!(status.success(), "harness failed before deliberate _exit");

    let leader = read_pid(&leader_marker);
    let grandchild = read_pid(&grandchild_marker);
    let mut cleanup = GroupCleanup {
        pgid: leader,
        pids: [leader, grandchild],
        armed: true,
    };
    tokio::time::timeout(Duration::from_secs(8), async {
        while process_exists(leader) || process_exists(grandchild) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("runtime watchdog should remove the terminal process group");
    cleanup.armed = false;
}

fn read_pid(path: &Path) -> libc::pid_t {
    fs::read_to_string(path)
        .expect("PID marker should be readable")
        .trim()
        .parse()
        .expect("PID marker should contain an integer")
}

fn process_exists(pid: libc::pid_t) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
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
    pids: [libc::pid_t; 2],
    armed: bool,
}

impl Drop for GroupCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = unsafe { libc::kill(-self.pgid, libc::SIGKILL) };
        for pid in self.pids {
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}
