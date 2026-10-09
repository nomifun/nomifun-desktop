//! Desktop WebKit compatibility identity, applied before any page request.
//!
//! `Version/17.0` is an advertised compatibility baseline for macOS 14+,
//! not a measurement of the Safari application or the loaded WebKit runtime.
use objc2_foundation::{NSProcessInfo, NSString};
use objc2_web_kit::WKWebView;
use nomifun_browser_platform::runtime::{BrowserIdentityReport, BrowserRuntimeInfo};

pub(crate) const PROFILE_ID: &str = "macos-desktop-webkit-safari17-v1";
pub(crate) const POLICY_REVISION: u32 = 1;
pub(crate) const COMPATIBILITY_VERSION: &str = "17.0";
pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

pub fn report() -> BrowserIdentityReport {
    BrowserIdentityReport {
        profile_id: PROFILE_ID.into(),
        policy_revision: POLICY_REVISION,
        engine_family: "webkit".into(),
        effective_user_agent: USER_AGENT.into(),
        advertised_compatibility_version: COMPATIBILITY_VERSION.into(),
        version_source: "compatibility_profile".into(),
    }
}

/// Report OS facts separately from the advertised compatibility version.
/// The public OS version does not identify the loaded Safari/WebKit version.
pub fn runtime_info() -> BrowserRuntimeInfo {
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    BrowserRuntimeInfo {
        os_version: format!("{}.{}.{}", version.majorVersion, version.minorVersion, version.patchVersion),
        os_build: None,
        webkit_framework_version: None,
    }
}

/// One native property configures both networking and the page's UA. Never
/// override a JavaScript navigator getter or an individual request header.
pub(crate) fn apply(view: &WKWebView) -> Result<(), String> {
    unsafe {
        view.setCustomUserAgent(Some(&NSString::from_str(USER_AGENT)));
        if view.customUserAgent().as_ref().map(|value| value.to_string()).as_deref()
            != Some(USER_AGENT)
        {
            return Err("WK desktop compatibility identity was not applied".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_identity_has_one_consistent_safari_compatibility_baseline() {
        assert_eq!(USER_AGENT.matches("Version/").count(), 1);
        assert_eq!(USER_AGENT.matches("Safari/").count(), 1);
        assert!(USER_AGENT.ends_with(&format!("Version/{COMPATIBILITY_VERSION} Safari/605.1.15")));
        assert!(USER_AGENT.contains("Macintosh; Intel Mac OS X 10_15_7"));
        assert!(!USER_AGENT.contains("Chrome/"));
        assert!(!USER_AGENT.contains("Mobile/"));
        assert_eq!(POLICY_REVISION, 1);
        assert_eq!(PROFILE_ID, "macos-desktop-webkit-safari17-v1");
    }
}
