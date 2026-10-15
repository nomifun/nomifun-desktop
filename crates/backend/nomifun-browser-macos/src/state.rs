//! Native-free Page facts shared by the WK owner and its navigation reducer.
use nomifun_browser_platform::runtime::{
    BrowserDialogKind, BrowserPermissionRequest, BrowserTabLifecycle,
    BrowserLoadSummary, BrowserNavigationSource,
};

#[derive(Clone, Debug, PartialEq)]
pub struct NativeDialog {
    pub request_id: String,
    pub document_generation: u64,
    pub kind: BrowserDialogKind,
    pub message: String,
    pub default_text: String,
    pub origin: String,
    pub text_truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageSnapshot {
    pub document_generation: u64,
    pub url: String,
    pub title: String,
    pub lifecycle: BrowserTabLifecycle,
    pub load: BrowserLoadSummary,
    pub bootstrap_ready: bool,
    pub(crate) bootstrap_loading: bool,
    pub(crate) navigation_source: BrowserNavigationSource,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub dialog: Option<NativeDialog>,
    pub blocked_permissions: Vec<String>,
    pub permission_requests: Vec<BrowserPermissionRequest>,
}

impl Default for PageSnapshot {
    fn default() -> Self {
        Self {
            document_generation: 0,
            url: "about:blank".into(),
            title: String::new(),
            lifecycle: BrowserTabLifecycle::Loading,
            load: BrowserLoadSummary { navigation_control: Some(Default::default()), ..Default::default() },
            bootstrap_ready: false,
            bootstrap_loading: false,
            navigation_source: BrowserNavigationSource::Unknown,
            can_go_back: false,
            can_go_forward: false,
            dialog: None,
            blocked_permissions: vec![],
            permission_requests: vec![],
        }
    }
}
