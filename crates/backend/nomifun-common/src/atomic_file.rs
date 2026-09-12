//! Private durable file publication shared by directory config and reset control.

use std::fs::OpenOptions;
use std::path::Path;

pub(crate) fn write_new_and_publish(
    temp: &Path,
    path: &Path,
    bytes: &[u8],
    publish: fn(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    // Cleanup authority starts only after exclusive creation succeeds.
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temp)?;
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        publish(temp, path)?;
        sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(source, target)
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target path contains a NUL byte",
            )
        },
    )?;
    if unsafe {
        libc::renamex_np(
            source.as_ptr(),
            target.as_ptr(),
            libc::RENAME_EXCL,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target path contains a NUL byte",
            )
        },
    )?;
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos"))
))]
pub(crate) fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::hard_link(source, target)?;
    std::fs::remove_file(source)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::hard_link(source, target)?;
    std::fs::remove_file(source)
}

#[cfg(windows)]
pub(crate) fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(80 | 183)) {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            error,
        ))
    } else {
        Err(error)
    }
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> std::io::Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
