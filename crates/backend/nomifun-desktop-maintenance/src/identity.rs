//! Operating-system executable identity shared by modern application and released-installer coordination.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs::File, path::{Path, PathBuf}};

pub(crate) fn validate_descriptor(expected: &Path, descriptor: &Path) -> Result<()> {
    ensure!(!descriptor.is_symlink() && std::fs::metadata(descriptor)?.len() <= 32768, "unsafe application participant descriptor");
    let descriptor: Value = serde_json::from_reader(File::open(descriptor)?)?;
    let pid = descriptor["pid"].as_u64().filter(|pid| *pid > 0 && *pid <= u32::MAX as u64)
        .context("application participant PID is unavailable")? as u32;
    let actual = process_image(pid)?.canonicalize().context("resolve application participant executable")?;
    #[cfg(windows)]
    let matches = actual.to_string_lossy().to_lowercase() == expected.to_string_lossy().to_lowercase();
    #[cfg(not(windows))]
    let matches = actual == expected;
    ensure!(matches, "another NomiFun installation owns the application participant; quit that installation before proceeding");
    Ok(())
}

#[cfg(target_os = "linux")]
fn process_image(pid: u32) -> Result<PathBuf> {
    Ok(std::fs::read_link(format!("/proc/{pid}/exe"))?)
}
#[cfg(target_os = "macos")]
fn process_image(pid: u32) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    unsafe extern "C" { fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32; }
    let mut bytes = vec![0u8; 4096];
    let length = unsafe { proc_pidpath(pid as i32, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    ensure!(length > 0, "cannot inspect application participant executable: {}", std::io::Error::last_os_error());
    bytes.truncate(bytes.iter().position(|byte| *byte == 0).unwrap_or(length as usize));
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}
#[cfg(windows)]
fn process_image(pid: u32) -> Result<PathBuf> {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(process: *mut c_void, flags: u32, path: *mut u16, size: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    ensure!(!handle.is_null(), "cannot inspect application participant process: {}", std::io::Error::last_os_error());
    let mut image = vec![0u16; 32768];
    let mut size = image.len() as u32;
    let result = unsafe { QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut size) };
    let error = std::io::Error::last_os_error();
    unsafe { CloseHandle(handle); }
    ensure!(result != 0, "cannot inspect application participant executable: {error}");
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&image[..size as usize])))
}
