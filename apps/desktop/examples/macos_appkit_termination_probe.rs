//! Request an ordinary AppKit quit for one independently launched Dev App.
//!
//! This probe is not an exit or cleanup verifier. The launch controller must
//! independently observe the exact kernel exit and its authenticated receipt.
//! It never launches an App, requests a permission, sends a signal, or forces
//! termination. Its input is a private record prepared by the launch owner.

#[cfg(target_os = "macos")]
mod native {
    use std::{
        ffi::CStr,
        fs::OpenOptions,
        io::{self, Read, Write},
        os::unix::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}},
        path::{Path, PathBuf},
    };

    use objc2_app_kit::NSRunningApplication;
    use serde::{Deserialize, Serialize};

    const MAX_REQUEST_BYTES: u64 = 16 * 1024;
    const DEV_IDENTIFIER: &str = "com.nomifun.desktop.dev";

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct OwnedRequest {
        owned_process_id: libc::pid_t,
        owned_start_key: u64,
        app_path: PathBuf,
    }

    fn invalid() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "owned Dev application request rejected")
    }

    fn validate_private_parent(path: &Path) -> io::Result<()> {
        if !path.is_absolute() { return Err(invalid()); }
        let directory = path.parent().ok_or_else(invalid)?;
        let parent = std::fs::symlink_metadata(directory)?;
        let uid = unsafe { libc::geteuid() };
        if !parent.is_dir() || parent.uid() != uid || parent.mode() & 0o777 != 0o700 {
            return Err(invalid());
        }
        Ok(())
    }

    fn validate_request(request: &OwnedRequest) -> io::Result<()> {
        if request.owned_process_id <= 0 || request.owned_start_key == 0
            || !request.app_path.is_absolute()
            || request.app_path.file_name() != Some(std::ffi::OsStr::new("NomiFun Dev.app"))
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn read_owned_request(path: &Path) -> io::Result<OwnedRequest> {
        validate_private_parent(path)?;
        let uid = unsafe { libc::geteuid() };
        let mut file = OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o600
            || metadata.len() == 0 || metadata.len() > MAX_REQUEST_BYTES
        {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file).take(MAX_REQUEST_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_REQUEST_BYTES { return Err(invalid()); }
        let request: OwnedRequest = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        validate_request(&request)?;
        Ok(request)
    }

    fn exact_process_program(pid: libc::pid_t) -> io::Result<PathBuf> {
        let mut bytes = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let length = unsafe {
            libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32)
        };
        if length <= 0 { return Err(io::Error::last_os_error()); }
        let program = CStr::from_bytes_until_nul(&bytes).map_err(|_| invalid())?;
        Path::new(std::ffi::OsStr::from_bytes(program.to_bytes())).canonicalize()
    }

    fn owned_process_start_key(pid: libc::pid_t) -> io::Result<u64> {
        if pid <= 0 { return Err(invalid()); }
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let returned = unsafe { libc::proc_pidinfo(pid,
            libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
        if returned <= 0 { return Err(io::Error::last_os_error()); }
        if returned != size { return Err(invalid()); }
        // proc_pidinfo initialized the full SDK structure above.
        let info = unsafe { info.assume_init() };
        let start_key = info.pbi_start_tvsec.checked_mul(1_000_000)
            .and_then(|value| value.checked_add(info.pbi_start_tvusec));
        if info.pbi_pid != pid as u32
            || info.pbi_uid != unsafe { libc::geteuid() }
            || info.pbi_ruid != unsafe { libc::getuid() }
            || info.pbi_start_tvusec >= 1_000_000
            || start_key.is_none_or(|key| key == 0)
        {
            return Err(invalid());
        }
        start_key.ok_or_else(invalid)
    }

    fn validate_process_generation(request: &OwnedRequest) -> io::Result<()> {
        if owned_process_start_key(request.owned_process_id)? != request.owned_start_key {
            return Err(invalid());
        }
        Ok(())
    }

    fn url_path(url: &objc2_foundation::NSURL) -> io::Result<PathBuf> {
        if !url.isFileURL() { return Err(invalid()); }
        let path = url.path().ok_or_else(invalid)?;
        Path::new(&path.to_string()).canonicalize()
    }

    fn exact_owned_application(request: &OwnedRequest)
        -> io::Result<objc2::rc::Retained<NSRunningApplication>>
    {
        validate_request(request)?;
        validate_process_generation(request)?;
        let app_path = request.app_path.canonicalize()?;
        let program = app_path.join("Contents/MacOS/nomifun-desktop").canonicalize()?;
        let identifier = plist::Value::from_file(app_path.join("Contents/Info.plist"))
            .ok().and_then(|value| value.as_dictionary()
                .and_then(|dictionary| dictionary.get("CFBundleIdentifier"))
                .and_then(plist::Value::as_string).map(str::to_owned));
        if identifier.as_deref() != Some(DEV_IDENTIFIER)
            || exact_process_program(request.owned_process_id)? != program
        {
            return Err(invalid());
        }
        let application = NSRunningApplication::runningApplicationWithProcessIdentifier(
            request.owned_process_id,
        ).ok_or_else(invalid)?;
        let bundle_url = application.bundleURL().ok_or_else(invalid)?;
        let executable_url = application.executableURL().ok_or_else(invalid)?;
        if application.processIdentifier() != request.owned_process_id
            || application.isTerminated()
            || application.bundleIdentifier().as_deref().map(|value| value.to_string())
                .as_deref() != Some(DEV_IDENTIFIER)
            || url_path(&*bundle_url)? != app_path
            || url_path(&*executable_url)? != program
            || exact_process_program(request.owned_process_id)? != program
        {
            return Err(invalid());
        }
        validate_process_generation(request)?;
        Ok(application)
    }

    pub enum Report { Prepared, TerminationAccepted(bool) }

    pub fn run() -> io::Result<Report> {
        let mut args = std::env::args_os().skip(1);
        let first = args.next().ok_or_else(invalid)?;
        if first == "--prepare" {
            let pid = args.next().and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse::<libc::pid_t>().ok()).ok_or_else(invalid)?;
            let app_path = args.next().map(PathBuf::from).ok_or_else(invalid)?;
            let request_path = args.next().map(PathBuf::from).ok_or_else(invalid)?;
            if args.next().is_some() { return Err(invalid()); }
            validate_private_parent(&request_path)?;
            let request = OwnedRequest {
                owned_process_id: pid,
                owned_start_key: owned_process_start_key(pid)?,
                app_path,
            };
            // Capturing the private record performs the same exact App and
            // kernel generation checks; it never calls terminate().
            let _application = exact_owned_application(&request)?;
            let bytes = serde_json::to_vec(&request).map_err(|_| invalid())?;
            if bytes.len() as u64 > MAX_REQUEST_BYTES { return Err(invalid()); }
            validate_process_generation(&request)?;
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW).open(&request_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            eprintln!("MACOS_APPKIT_TERMINATION_PROBE owned_request_prepared=true");
            return Ok(Report::Prepared);
        }
        if args.next().is_some() { return Err(invalid()); }
        let request = read_owned_request(&PathBuf::from(first))?;
        let application = exact_owned_application(&request)?;
        eprintln!("MACOS_APPKIT_TERMINATION_PROBE target_validated=true");
        let accepted = application.terminate();
        eprintln!("MACOS_APPKIT_TERMINATION_PROBE normal_request_accepted={accepted}");
        Ok(Report::TerminationAccepted(accepted))
    }
}

#[cfg(target_os = "macos")]
fn main() {
    match native::run() {
        Ok(native::Report::Prepared) => {
            println!("{}", serde_json::json!({
                "scope": "exact_owned_dev_appkit_termination_prepare",
                "owned_request_prepared": true,
                "normal_request_submitted": false,
            }));
        }
        Ok(native::Report::TerminationAccepted(true)) => {
            println!("{}", serde_json::json!({
                "scope": "exact_owned_dev_appkit_termination_request",
                "normal_request_accepted": true,
                "kernel_exit_observed": false,
                "cleanup_verified": false,
            }));
        }
        Ok(native::Report::TerminationAccepted(false)) => {
            eprintln!("MACOS_APPKIT_TERMINATION_PROBE normal_request_accepted=false");
            std::process::exit(1);
        }
        Err(error) => {
            // The record and its path are never diagnostic output.
            eprintln!("MACOS_APPKIT_TERMINATION_PROBE request_rejected error_kind={:?}", error.kind());
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("MACOS_APPKIT_TERMINATION_PROBE unsupported_platform=true");
    std::process::exit(1);
}
