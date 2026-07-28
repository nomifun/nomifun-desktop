use std::{
    io,
    os::fd::{FromRawFd, OwnedFd},
};

/// Exact Linux process identity used to anchor a verified process-group
/// recovery.
///
/// Opening the pidfd alone is not authorization to terminate anything. The
/// caller must still bind it back to its durable PID/start-time/executable
/// marker after [`Self::open`] and again after [`Self::stop`]. The final group
/// signal remains here, at the shared process-ownership boundary.
pub struct LinuxProcessGroupAnchor {
    pid: libc::pid_t,
    pidfd: OwnedFd,
    resume_on_drop: bool,
    stopped: bool,
}

impl LinuxProcessGroupAnchor {
    /// Open an exact pidfd for a candidate process-group leader.
    pub fn open(pid: u32) -> io::Result<Self> {
        let pid = libc::pid_t::try_from(pid).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "process PID exceeds pid_t")
        })?;
        if pid <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "process-group recovery requires a PID greater than 1",
            ));
        }

        // SAFETY: pidfd_open consumes only integer arguments and returns a new
        // descriptor on success.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_pidfd_open,
                pid,
                0 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: pidfd_open returned a fresh descriptor now owned here.
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
        Ok(Self {
            pid,
            pidfd,
            resume_on_drop: false,
            stopped: false,
        })
    }

    /// Stop the exact pidfd-anchored leader before its marker is revalidated.
    ///
    /// Any later failure resumes the leader from `Drop`; a successful
    /// [`Self::terminate_group`] disarms that fallback.
    pub fn stop(&mut self) -> io::Result<()> {
        self.send_signal(libc::SIGSTOP)?;
        self.stopped = true;
        self.resume_on_drop = true;
        Ok(())
    }

    /// Query the current process group of the exact anchored PID.
    pub fn process_group_id(&self) -> io::Result<u32> {
        // SAFETY: getpgid is a read-only query for the pidfd-anchored PID.
        let pgid = unsafe { libc::getpgid(self.pid) };
        if pgid <= 0 {
            return Err(io::Error::last_os_error());
        }
        u32::try_from(pgid)
            .map_err(|_| io::Error::other("process group id does not fit in u32"))
    }

    /// Terminate the process group whose stopped exact leader is this pidfd.
    ///
    /// The cached numeric PGID is used only after proving that it still equals
    /// the anchored PID and does not overlap the current application's group.
    pub fn terminate_group(&mut self) -> io::Result<()> {
        if !self.stopped {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "process-group leader must be stopped before termination",
            ));
        }
        let pgid = self.process_group_id()?;
        if pgid != self.pid as u32 {
            return Err(io::Error::other(
                "pidfd-anchored process is no longer its process-group leader",
            ));
        }
        // SAFETY: getpgrp is a read-only query for the current process.
        let current_group = unsafe { libc::getpgrp() };
        if current_group <= 0 || current_group == self.pid {
            return Err(io::Error::other(
                "refusing to terminate the current application process group",
            ));
        }

        // SAFETY: the exact stopped leader remains anchored by `pidfd`, PID 0
        // and non-leaders were rejected, and the group was re-read immediately
        // above. The negative PID therefore addresses only that verified group.
        if unsafe { libc::kill(-self.pid, libc::SIGKILL) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.resume_on_drop = false;
        Ok(())
    }

    fn send_signal(&self, signal: libc::c_int) -> io::Result<()> {
        use std::os::fd::AsRawFd;

        // SAFETY: the descriptor is owned by this value, siginfo is null by
        // contract, and flags must be zero for pidfd_send_signal.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0 as libc::c_uint,
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for LinuxProcessGroupAnchor {
    fn drop(&mut self) {
        if self.resume_on_drop {
            let _ = self.send_signal(libc::SIGCONT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LinuxProcessGroupAnchor;

    #[test]
    fn invalid_pid_is_rejected_before_pidfd_open() {
        assert!(LinuxProcessGroupAnchor::open(0).is_err());
        assert!(LinuxProcessGroupAnchor::open(1).is_err());
    }
}
