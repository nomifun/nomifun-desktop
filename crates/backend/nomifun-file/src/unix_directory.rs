//! Search-only, non-following directory capabilities for Unix mutations.

use std::{ffi::CString, fs::File, io, os::{fd::{AsRawFd, FromRawFd}, unix::ffi::OsStrExt}, path::Path};
use cap_std::fs::Dir;

pub(crate) fn open_for_sync(directory: &Dir) -> io::Result<File> {
    #[cfg(target_os = "macos")]
    return directory.try_clone().map(Dir::into_std_file);
    #[cfg(not(target_os = "macos"))]
    {
        // cap-std open_dir uses O_PATH on Linux even with a read option. This
        // explicit descriptor must support fsync, so acquire O_RDONLY before
        // publication while preserving the already-authorized parent handle.
        // SAFETY: the borrowed directory fd and constant NUL-terminated name
        // stay live for this syscall, and the descriptor is owned below.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        // SAFETY: openat returned a new descriptor and no other owner exists.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

pub(crate) fn verify_named(directory: &Dir, path: &Path) -> io::Result<()> {
    use cap_std::fs::MetadataExt as _;
    use std::os::unix::fs::MetadataExt as _;
    let opened = directory.dir_metadata()?;
    let named = std::fs::metadata(path)?;
    if std::fs::canonicalize(path)? != path
        || opened.dev() != named.dev() || opened.ino() != named.ino()
    {
        return Err(io::Error::other("authorized directory changed identity"));
    }
    Ok(())
}

pub(crate) fn open_search(parent: Option<&Dir>, path: &Path) -> io::Result<Dir> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory name contains NUL"))?;
    #[cfg(target_os = "macos")]
    let access = libc::O_SEARCH;
    #[cfg(target_os = "linux")]
    let access = libc::O_PATH;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let access = libc::O_RDONLY;
    // SAFETY: path is NUL-terminated and the optional parent capability stays
    // borrowed throughout openat. All returned descriptors are owned below.
    let fd = unsafe {
        libc::openat(parent.map_or(libc::AT_FDCWD, AsRawFd::as_raw_fd), path.as_ptr(),
            access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
    };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: successful openat returns a new, exclusively owned descriptor.
    Ok(Dir::from_std_file(unsafe { File::from_raw_fd(fd) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_descriptor_supports_fsync_instead_of_reusing_linux_o_path() {
        let fixture = tempfile::tempdir().unwrap();
        let directory = open_search(None, fixture.path()).unwrap();
        let sync = open_for_sync(&directory).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(unsafe { libc::fcntl(sync.as_raw_fd(), libc::F_GETFL) } & libc::O_PATH, 0);
        sync.sync_all().unwrap();
    }
}
