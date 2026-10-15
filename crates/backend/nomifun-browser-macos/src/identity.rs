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

/// Immutable for one engine, including its popups. The alternate policy is
/// exposed for controlled native acceptance; production keeps Safari17.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BrowserIdentityPolicy {
    #[default]
    Safari17,
    NativeWebKit,
}

pub fn report() -> BrowserIdentityReport {
    report_for(BrowserIdentityPolicy::Safari17, None)
}

pub(crate) fn report_for(policy: BrowserIdentityPolicy, measured_user_agent: Option<&str>) -> BrowserIdentityReport {
    if policy == BrowserIdentityPolicy::NativeWebKit {
        let measured = measured_user_agent.unwrap_or_default();
        return BrowserIdentityReport {
            profile_id: "macos-desktop-webkit-native-v1".into(),
            policy_revision: POLICY_REVISION,
            engine_family: "webkit".into(),
            effective_user_agent: measured.into(),
            advertised_compatibility_version: measured.split_whitespace()
                .find_map(|token| token.strip_prefix("Version/")).unwrap_or_default().into(),
            version_source: if measured.is_empty() { "native_user_agent_unavailable" } else { "native_user_agent" }.into(),
        };
    }
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
        ..Default::default()
    }
}

/// One native property configures both networking and the page's UA. Never
/// override a JavaScript navigator getter or an individual request header.
pub(crate) fn apply(view: &WKWebView, policy: BrowserIdentityPolicy) -> Result<(), String> {
    unsafe {
        if policy == BrowserIdentityPolicy::NativeWebKit {
            view.setCustomUserAgent(None);
            if view.customUserAgent().is_some() { return Err("WK native identity was not applied".into()); }
            return Ok(());
        }
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
    #[test]
    fn native_policy_does_not_invent_a_user_agent_or_safari_version() {
        let report = report_for(BrowserIdentityPolicy::NativeWebKit, None);
        assert!(report.effective_user_agent.is_empty());
        assert!(report.advertised_compatibility_version.is_empty());
        assert_eq!(report.version_source, "native_user_agent_unavailable");
        assert_eq!(BrowserIdentityPolicy::default(), BrowserIdentityPolicy::Safari17);
    }
    #[test]
    fn native_identity_report_uses_only_the_measured_string() {
        let measured = "Mozilla/5.0 AppleWebKit/605.1.15";
        let report = report_for(BrowserIdentityPolicy::NativeWebKit, Some(measured));
        assert_eq!(report.effective_user_agent, measured);
        assert!(report.advertised_compatibility_version.is_empty());
        assert_eq!(report.version_source, "native_user_agent");
        let measured_safari = "Mozilla/5.0 Version/18.4 Safari/605.1.15";
        assert_eq!(report_for(BrowserIdentityPolicy::NativeWebKit, Some(measured_safari))
            .advertised_compatibility_version, "18.4");
    }
}
