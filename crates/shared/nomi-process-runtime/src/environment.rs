//! Pure shared classification of inherited application controls.
use std::ffi::OsStr;

/// Application controls are not inherited user tool configuration. Consumers
/// apply this only to ambient inheritance, then overlay explicit task values.
pub fn is_inherited_application_environment_key(key: &OsStr) -> bool {
    #[cfg(windows)] {
        let key = key.to_string_lossy().to_ascii_uppercase();
        key == "CLAUDECODE" || key.starts_with("NOMIFUN_")
    }
    #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;
        key == OsStr::new("CLAUDECODE") || key.as_bytes().starts_with(b"NOMIFUN_")
    }
    #[cfg(not(any(unix, windows)))] {
        key == OsStr::new("CLAUDECODE") || key.to_string_lossy().starts_with("NOMIFUN_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_application_controls_are_distinct_from_user_tool_configuration() {
        for key in ["NOMIFUN_LOCAL_TRUST", "NOMIFUN_DEBUG_FIXTURE", "CLAUDECODE"] {
            assert!(is_inherited_application_environment_key(OsStr::new(key)));
        }
        for key in ["HOME", "USERPROFILE", "NODE_OPTIONS", "NPM_CONFIG_PREFIX", "LD_LIBRARY_PATH", "HTTP_PROXY"] {
            assert!(!is_inherited_application_environment_key(OsStr::new(key)));
        }
        assert_eq!(is_inherited_application_environment_key(OsStr::new("nomifun_control")), cfg!(windows));
    }
}
