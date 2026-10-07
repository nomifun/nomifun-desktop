//! Stable UTF-16 backing for CEF struct fields.
//!
//! cef-rs 152.3.0 converts owned (Clear) CefString fields to empty native
//! strings when a settings struct is copied to its C representation. Supplying
//! borrowed, destructor-free fields preserves the values and avoids shallow
//! ownership copies. CEF copies settings during the native call.
pub(crate) struct Text(Vec<u16>);

impl Text {
    pub(crate) fn new(value: &str) -> Self { Self(value.encode_utf16().collect()) }

    /// The result must not outlive `self`. Keep this buffer alive through the
    /// synchronous CEF call that copies the settings; do not store it in CEF callbacks.
    pub(crate) unsafe fn field(&self) -> cef::CefString {
        cef::CefString::from(cef::sys::cef_string_utf16_t {
            str_: self.0.as_ptr().cast_mut(),
            length: self.0.len(),
            dtor: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_settings_preserve_helper_root_and_unicode_paths() {
        let helper = Text::new("/owned/伙伴 Helper.app/Contents/MacOS/helper");
        let root = Text::new("/owned/browser-v2");
        let settings = cef::Settings {
            // SAFETY: both buffers outlive the settings and its raw copy.
            browser_subprocess_path: unsafe { helper.field() },
            root_cache_path: unsafe { root.field() },
            ..Default::default()
        };
        let raw: cef::sys::cef_settings_t = settings.clone().into();
        assert!(!raw.browser_subprocess_path.str_.is_null());
        assert!(!raw.root_cache_path.str_.is_null());
        assert_eq!(unsafe { std::slice::from_raw_parts(raw.browser_subprocess_path.str_, raw.browser_subprocess_path.length) }, helper.0);
        assert_eq!(unsafe { std::slice::from_raw_parts(raw.root_cache_path.str_, raw.root_cache_path.length) }, root.0);
        assert!(raw.root_cache_path.dtor.is_none());
    }

    #[test]
    fn native_request_context_keeps_its_distinct_profile_path() {
        let path = Text::new("/owned/browser-v2/conversations/one");
        let settings = cef::RequestContextSettings { cache_path: unsafe { path.field() }, ..Default::default() };
        let raw: cef::sys::cef_request_context_settings_t = settings.clone().into();
        assert_eq!(unsafe { std::slice::from_raw_parts(raw.cache_path.str_, raw.cache_path.length) }, path.0);
    }
}
