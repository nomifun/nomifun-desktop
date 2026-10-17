//! Pure shared classification of inherited application controls.
use std::ffi::OsStr;

/// Retain inherited custom module directories while selecting the system
/// Windows PowerShell modules ahead of incompatible ambient PS7 modules.
/// Callers apply this only to inherited values for an already selected WinPS
/// execution path. Explicit task values, including empty, remain unchanged.
pub fn windows_powershell_module_path(ambient: &OsStr) -> std::io::Result<std::ffi::OsString> {
    #[cfg(windows)]
    {
        let mut value = crate::platform::windows::windows_directory()
            .map_err(std::io::Error::other)?
            .join("System32").join("WindowsPowerShell").join("v1.0").join("Modules")
            .into_os_string();
        value.push(";");
        value.push(ambient);
        Ok(value)
    }
    #[cfg(not(windows))]
    { Ok(ambient.to_owned()) }
}

/// Compare native environment names using the target OS's own semantics.
/// Windows uses CompareStringOrdinal with case ignored; Unix names remain
/// exact bytes. No Unicode conversion or lossy case folding is involved.
pub fn environment_key_eq(left: &OsStr, right: &OsStr) -> bool {
    #[cfg(windows)]
    {
        crate::platform::windows::compare_os_case_insensitive(left, right)
            == std::cmp::Ordering::Equal
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Application controls are not inherited user tool configuration. Consumers
/// apply this only to ambient inheritance, then overlay explicit task values.
pub fn is_inherited_application_environment_key(key: &OsStr) -> bool {
    #[cfg(windows)]
    {
        let key = key.to_string_lossy().to_ascii_uppercase();
        key == "CLAUDECODE" || key.starts_with("NOMIFUN_")
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        key == OsStr::new("CLAUDECODE") || key.as_bytes().starts_with(b"NOMIFUN_")
    }
    #[cfg(not(any(unix, windows)))]
    {
        key == OsStr::new("CLAUDECODE") || key.to_string_lossy().starts_with("NOMIFUN_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_environment_names_follow_platform_case_semantics() {
        assert!(environment_key_eq(OsStr::new(""), OsStr::new("")));
        assert!(!environment_key_eq(OsStr::new(""), OsStr::new("PATH")));
        assert!(environment_key_eq(
            OsStr::new("USERPROFILE"),
            OsStr::new("USERPROFILE")
        ));
        assert_eq!(
            environment_key_eq(OsStr::new("Path"), OsStr::new("PATH")),
            cfg!(windows)
        );
        assert_eq!(
            environment_key_eq(OsStr::new("tool_ä"), OsStr::new("TOOL_Ä")),
            cfg!(windows)
        );
        assert!(!environment_key_eq(
            OsStr::new("Path"),
            OsStr::new("PATHEXT")
        ));
    }

    #[test]
    fn inherited_application_controls_are_distinct_from_user_tool_configuration() {
        for key in ["NOMIFUN_LOCAL_TRUST", "NOMIFUN_DEBUG_FIXTURE", "CLAUDECODE"] {
            assert!(is_inherited_application_environment_key(OsStr::new(key)));
        }
        for key in [
            "HOME",
            "USERPROFILE",
            "NODE_OPTIONS",
            "NPM_CONFIG_PREFIX",
            "LD_LIBRARY_PATH",
            "HTTP_PROXY",
        ] {
            assert!(!is_inherited_application_environment_key(OsStr::new(key)));
        }
        assert_eq!(
            is_inherited_application_environment_key(OsStr::new("nomifun_control")),
            cfg!(windows)
        );
    }
}
