use anyhow::{Result, ensure};
use std::{ffi::{c_void, OsString}, fs, mem, os::windows::ffi::{OsStrExt, OsStringExt}, path::{Path, PathBuf}};

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

pub fn installed_desktop_running(installed: &Path, runtime: &Path) -> Result<bool> {
    let expected = if installed.exists() { installed.canonicalize()? } else { return Ok(false); };
    let expected = expected.to_string_lossy().to_lowercase();
    let owner_pid = if nomifun_keyboard::legacy_service::owner_running_in(runtime)? { fs::read(runtime.join("owner.json")).ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["pid"].as_u64()) } else { None };
    let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
    ensure!(snapshot != -1isize as Handle, "cannot inspect installed application processes");
    let snapshot = Parent(snapshot);
    let mut entry: ProcessEntry = unsafe { mem::zeroed() };
    entry.size = mem::size_of::<ProcessEntry>() as u32;
    let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while next != 0 {
        if Some(entry.pid as u64) != owner_pid {
            let process = unsafe { OpenProcess(0x1000, 0, entry.pid) };
            if !process.is_null() {
                let process = Parent(process);
                let mut image = vec![0u16; 32768];
                let mut size = image.len() as u32;
                if unsafe { QueryFullProcessImageNameW(process.0, 0, image.as_mut_ptr(), &mut size) } != 0 {
                    let image = PathBuf::from(OsString::from_wide(&image[..size as usize]));
                    if image.canonicalize().is_ok_and(|image| image.to_string_lossy().to_lowercase() == expected) { return Ok(true); }
                }
            }
        }
        next = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    Ok(false)
}

pub fn validate_runtime_ownership(installed: &Path, runtime: &Path, observed: nomifun_keyboard::maintenance::ResumeIntent) -> Result<()> {
    nomifun_keyboard::legacy_service::validate_runtime_ownership(installed, runtime, observed.desktop)
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
