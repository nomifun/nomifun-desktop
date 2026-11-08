use anyhow::{Context, Result, ensure};
use std::{ffi::{c_void, OsString}, fs, mem, os::windows::ffi::{OsStrExt, OsStringExt}, path::{Path, PathBuf}, time::{Duration, Instant}};

type Handle = *mut c_void;
#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    default_heap: usize,
    module: u32,
    threads: u32,
    parent: u32,
    priority: i32,
    flags: u32,
    filename: [u16; 260],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, path: *mut u16, size: *mut u32) -> i32;
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
}

pub struct Parent(Handle);
impl Parent {
    pub fn open(pid: u32) -> Result<Self> {
        let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
        ensure!(!handle.is_null(), "cannot observe installer lifetime: {}", std::io::Error::last_os_error());
        Ok(Self(handle))
    }
    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == 0x102 }
    }
}
impl Drop for Parent {
    fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
}

pub fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    ensure!(unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x1 | 0x8) } != 0, "publish maintenance status: {}", std::io::Error::last_os_error());
    Ok(())
}

/// Retain native process handles before requesting shutdown. Releasing an
/// application lock (or disappearing from a later snapshot) does not prove
/// Windows has finished teardown and unmapped the executable. Handles also
/// avoid following a reused PID. No process is terminated by this observer.
pub struct InstalledProcesses(Vec<(u32, Parent)>);

impl InstalledProcesses {
    pub fn capture(installed: &Path) -> Result<Self> {
        let expected = match installed.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self(Vec::new())),
            Err(error) => return Err(error).context("resolve installed application for native exit wait"),
        };
        let name = expected.file_name().context("installed application has no filename")?.to_string_lossy().to_lowercase();
        let expected = expected.to_string_lossy().to_lowercase();
        let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
        ensure!(snapshot != -1isize as Handle, "cannot inspect installed application processes: {}", std::io::Error::last_os_error());
        let snapshot = Parent(snapshot);
        let mut processes = Vec::new();
        let mut entry: ProcessEntry = unsafe { mem::zeroed() };
        entry.size = mem::size_of::<ProcessEntry>() as u32;
        let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
        while next != 0 {
            let length = entry.filename.iter().position(|value| *value == 0).unwrap_or(entry.filename.len());
            let filename = OsString::from_wide(&entry.filename[..length]);
            if filename.to_string_lossy().to_lowercase() == name {
                // SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION. Do not ask
                // for termination rights or inspect unrelated process images.
                let process = unsafe { OpenProcess(0x00101000, 0, entry.pid) };
                if process.is_null() {
                    let error = std::io::Error::last_os_error();
                    // An already-terminated snapshot entry is normal; access
                    // denial must not silently become evidence of absence.
                    if error.raw_os_error() != Some(87) {
                        return Err(error).with_context(|| format!("observe application process PID {}", entry.pid));
                    }
                } else {
                    let process = Parent(process);
                    let mut image = vec![0u16; 32768];
                    let mut size = image.len() as u32;
                    if unsafe { QueryFullProcessImageNameW(process.0, 0, image.as_mut_ptr(), &mut size) } != 0 {
                        let image = PathBuf::from(OsString::from_wide(&image[..size as usize]));
                        if image.canonicalize()?.to_string_lossy().to_lowercase() == expected
                            && !process_exited(&process)? {
                            processes.push((entry.pid, process));
                        }
                    } else {
                        let error = std::io::Error::last_os_error();
                        if !process_exited(&process)? {
                            return Err(error).with_context(|| format!("inspect application process PID {}", entry.pid));
                        }
                    }
                }
            }
            next = unsafe { Process32NextW(snapshot.0, &mut entry) };
        }
        let error = std::io::Error::last_os_error();
        ensure!(error.raw_os_error() == Some(18), "cannot finish application process snapshot: {error}");
        Ok(Self(processes))
    }

    pub fn desktop_running(&self, runtime: &Path) -> Result<bool> {
        let owner_pid = if nomifun_desktop_maintenance::legacy_service::owner_running_in(runtime)? { fs::read(runtime.join("owner.json")).ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["pid"].as_u64()) } else { None };
        for (pid, process) in &self.0 {
            if Some(*pid as u64) != owner_pid && !process_exited(process)? { return Ok(true); }
        }
        Ok(false)
    }

    /// Called after NSIS has handled old releases without authenticated IPC,
    /// while the maintenance gate and participant lifetime locks remain held.
    pub fn wait_for_exit(&mut self, installed: &Path, wait: Duration) -> Result<()> {
        let deadline = Instant::now() + wait;
        self.0.extend(Self::capture(installed)?.0);
        loop {
            for (pid, process) in self.0.drain(..) {
                let remaining = deadline.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128) as u32;
                let result = unsafe { WaitForSingleObject(process.0, remaining) };
                ensure!(result != 0x102, "application process PID {pid} has not exited; close {} before retrying installation (waited {} ms)", installed.display(), wait.as_millis());
                ensure!(result == 0, "wait for application process PID {pid}: {}", std::io::Error::last_os_error());
            }
            // Catch starts that raced preparation, including already-released
            // versions which do not implement the startup admission gate.
            self.0 = Self::capture(installed)?.0;
            if self.0.is_empty() { return Ok(()); }
            ensure!(Instant::now() < deadline, "application keeps restarting before file replacement: {}", installed.display());
        }
    }
}

fn process_exited(process: &Parent) -> Result<bool> {
    match unsafe { WaitForSingleObject(process.0, 0) } {
        0 => Ok(true),
        0x102 => Ok(false),
        _ => Err(std::io::Error::last_os_error()).context("inspect native application lifetime"),
    }
}

pub fn installed_desktop_running(installed: &Path, runtime: &Path) -> Result<bool> {
    InstalledProcesses::capture(installed)?.desktop_running(runtime)
}

pub fn validate_runtime_ownership(installed: &Path, runtime: &Path, observed: nomifun_desktop_maintenance::maintenance::ResumeIntent) -> Result<()> {
    nomifun_desktop_maintenance::legacy_service::validate_runtime_ownership(installed, runtime, observed.desktop)
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegDeleteKeyValueW(key: Handle, subkey: *const u16, name: *const u16) -> i32;
    fn RegGetValueW(key: Handle, subkey: *const u16, name: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
}
fn login_value() -> Result<Option<String>> {
    let key = 0x80000001u32 as i32 as isize as Handle;
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".encode_utf16().collect();
    let name: Vec<u16> = "NomiFunKeyboard\0".encode_utf16().collect();
    let mut size = 0;
    let result = unsafe { RegGetValueW(key, subkey.as_ptr(), name.as_ptr(), 2, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
    // A value of a different registry type is not an owned registration. Never
    // rewrite or delete it, and do not make it a prerequisite for app updates.
    if [2, 3, 1630].contains(&result) { return Ok(None); }
    ensure!(result == 0 && size <= 65536 && size % 2 == 0, "cannot inspect keyboard login registration: {result}");
    let mut value = vec![0u16; size as usize / 2];
    let result = unsafe { RegGetValueW(key, subkey.as_ptr(), name.as_ptr(), 2, std::ptr::null_mut(), value.as_mut_ptr().cast(), &mut size) };
    ensure!(result == 0, "cannot read keyboard login registration: {result}");
    value.truncate(size as usize / 2);
    while value.last() == Some(&0) { value.pop(); }
    Ok(Some(String::from_utf16(&value)?))
}

pub fn owns_login_registration(installed: &Path) -> Result<bool> {
    let Some(command) = login_value()? else { return Ok(false); };
    let Some(executable) = command.strip_prefix('"').and_then(|value| value.strip_suffix("\" --keyboard-service")) else { return Ok(false); };
    if executable.is_empty() || executable.contains(['"', '\n', '\r', '\0']) { return Ok(false); }
    let registered = Path::new(executable);
    if !registered.is_absolute() { return Ok(false); }
    let resolve = |path: &Path| -> Result<Option<PathBuf>> {
        match path.canonicalize() {
            Ok(path) => Ok(Some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    };
    let (Some(registered), Some(installed)) = (resolve(registered)?, resolve(installed)?) else { return Ok(false); };
    Ok(registered.to_string_lossy().to_lowercase() == installed.to_string_lossy().to_lowercase())
}

pub fn unregister_login(installed: &Path) -> Result<()> {
    // Another installation (including a developer fixture) must never remove
    // the current user's real keyboard login registration.
    let Some(command) = login_value()? else { return Ok(()); };
    if !owns_login_registration(installed)? { return Ok(()); }
    ensure!(login_value()?.as_deref() == Some(command.as_str()), "keyboard login registration changed; it was not removed");
    let key = 0x80000001u32 as i32 as isize as Handle;
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".encode_utf16().collect();
    let name: Vec<u16> = "NomiFunKeyboard\0".encode_utf16().collect();
    let result = unsafe { RegDeleteKeyValueW(key, subkey.as_ptr(), name.as_ptr()) };
    ensure!([0, 2, 3].contains(&result), "cannot remove keyboard login registration: {result}");
    Ok(())
}
