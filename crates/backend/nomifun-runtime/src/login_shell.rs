//! Resolve the same account shell for GUI PATH discovery and terminal launch.

use std::{ffi::{CStr, OsString}, io, os::unix::ffi::OsStringExt, path::{Path, PathBuf}};

/// Prefer an explicitly supplied `SHELL`, otherwise consult the Unix account.
/// Finder/launchd need not export `SHELL`, even when the user's real shell is
/// zsh with a customized profile. Invalid explicit values fail closed rather
/// than silently selecting a different shell or searching inherited PATH.
pub fn resolve_login_shell() -> io::Result<PathBuf> {
    let explicit = std::env::var_os("SHELL");
    if explicit.is_some() {
        return resolve_with(explicit, None, is_executable);
    }
    resolve_with(None, account_login_shell(), is_executable)
}

fn resolve_with<F>(explicit: Option<OsString>, account: Option<PathBuf>, executable: F) -> io::Result<PathBuf>
where F: Fn(&Path) -> bool {
    if let Some(shell) = explicit {
        let shell = PathBuf::from(shell);
        if !shell.is_absolute() || shell.to_str().is_none() || !executable(&shell) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "SHELL must name an existing absolute executable"));
        }
        return Ok(shell);
    }
    if let Some(shell) = account.filter(|shell| shell.is_absolute() && shell.to_str().is_some() && executable(shell)) {
        return Ok(shell);
    }
    #[cfg(target_os = "macos")]
    let candidates = ["/bin/zsh", "/bin/sh"];
    #[cfg(not(target_os = "macos"))]
    let candidates = ["/bin/sh", "/bin/bash"];
    candidates.into_iter().map(PathBuf::from).find(|shell| executable(shell))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no executable Unix login shell is available"))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path_string) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false; };
    path.is_file() && unsafe { libc::access(path_string.as_ptr(), libc::X_OK) } == 0
}

fn account_login_shell() -> Option<PathBuf> {
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut capacity = usize::try_from(suggested).unwrap_or(16 * 1024).clamp(1024, 1024 * 1024);
    loop {
        let mut buffer = vec![0_u8; capacity];
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(libc::getuid(), &mut record, buffer.as_mut_ptr().cast(), buffer.len(), &mut result)
        };
        if status == libc::ERANGE && capacity < 1024 * 1024 {
            capacity = capacity.saturating_mul(2).min(1024 * 1024);
            continue;
        }
        if status != 0 || result.is_null() || record.pw_shell.is_null() {
            return None;
        }
        let bytes = unsafe { CStr::from_ptr(record.pw_shell) }.to_bytes();
        return (!bytes.is_empty()).then(|| PathBuf::from(OsString::from_vec(bytes.to_vec())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_shell_wins_without_replacing_invalid_values() {
        assert_eq!(resolve_with(Some("/fixture/custom-zsh".into()), Some("/bin/zsh".into()), |_| true).unwrap(), Path::new("/fixture/custom-zsh"));
        for value in ["zsh", "", "./zsh", "/fixture/missing-shell"] {
            let error = resolve_with(Some(value.into()), Some("/bin/zsh".into()), |path| path == Path::new("/bin/zsh")).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn absent_shell_uses_actual_account_shell_before_platform_fallback() {
        assert_eq!(resolve_with(None, Some("/fixture/account-shell".into()), |_| true).unwrap(), Path::new("/fixture/account-shell"));
        #[cfg(target_os = "macos")]
        let fallback = "/bin/zsh";
        #[cfg(not(target_os = "macos"))]
        let fallback = "/bin/sh";
        assert_eq!(resolve_with(None, Some("relative-shell".into()), |_| true).unwrap(), Path::new(fallback));
        assert_eq!(resolve_with(None, None, |_| false).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn real_account_lookup_returns_an_absolute_shell_when_available() {
        if let Some(shell) = account_login_shell() {
            assert!(shell.is_absolute());
            assert!(is_executable(&shell));
        }
    }
}
