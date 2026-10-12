//! One reducer owns navigation evidence and document availability. Metadata
//! sampling never creates a document or turns cancellation into success.
use crate::engine::PageSnapshot;
use nomifun_browser_platform::runtime::{
    BrowserCancellationReason as Cancel, BrowserContentState as Content,
    BrowserFailureReason as Reason, BrowserFailureStage as Stage,
    BrowserNativeDomainClass as Domain, BrowserNavigationEvent, BrowserNavigationEventKind as Kind,
    BrowserNavigationPhase as Phase, BrowserNavigationSource as Source, BrowserPageProblem,
    BrowserTabLifecycle,
};

pub(crate) enum Update {
    Begin { url: Option<String>, source: Source, bootstrap: bool },
    Started { url: String },
    Redirect { url: String },
    Committed { url: String },
    Finished { url: String },
    Cancelled(Cancel),
    Failed { domain: String, code: i64 },
    Crashed,
    NoNavigation { url: String },
    Metadata { url: String, title: String, back: bool, forward: bool, progress: u16 },
    Response { url: Option<String>, status: Option<u16> },
    PolicyCancelled { url: Option<String>, reason: Cancel },
    Closed,
}

pub(crate) fn in_flight(phase: Phase) -> bool {
    matches!(phase, Phase::Requested | Phase::Provisional | Phase::Committed)
}

fn document_url(url: &str) -> bool {
    // A page-created about:blank popup may be populated by document.write.
    // Internal bootstrap is excluded by its explicit source flag, not by URL.
    url.starts_with("https://") || url.starts_with("http://") || url == "about:blank"
}

fn same_document(left: &str, right: &str) -> bool {
    let (Ok(mut left), Ok(mut right)) = (url::Url::parse(left), url::Url::parse(right)) else { return false; };
    left.set_fragment(None);
    right.set_fragment(None);
    left == right
}

fn problem(domain: &str, code: i64, phase: Phase) -> BrowserPageProblem {
    let class = match domain {
        "NSURLErrorDomain" => Domain::Url,
        "WKErrorDomain" | "WebKitErrorDomain" => Domain::WebKit,
        _ => Domain::Other,
    };
    let reason = if class == Domain::Url {
        match code {
            -1001 => Reason::Timeout,
            -1003 | -1006 => Reason::Dns,
            -1004 | -1005 | -1009 => Reason::NetworkConnection,
            -1206..=-1200 => Reason::Tls,
            -1000 | -1002 => Reason::UnsupportedUrl,
            -1102 | -1013 => Reason::AccessDenied,
            _ => Reason::Other,
        }
    } else { Reason::Other };
    BrowserPageProblem {
        stage: if phase == Phase::Committed { Stage::Committed } else { Stage::Provisional },
        safe_reason: reason, native_domain_class: class, native_code: code,
    }
}

fn trace(s: &PageSnapshot, kind: Kind, url: Option<String>) -> BrowserNavigationEvent {
    BrowserNavigationEvent::new(kind, Some(s.load.navigation_sequence), Some(s.document_generation), s.navigation_source, url)
}

pub(crate) fn reduce(s: &mut PageSnapshot, update: Update) -> Vec<BrowserNavigationEvent> {
    let mut events = Vec::new();
    let event = match update {
        Update::Begin { url, source, bootstrap } => {
            if in_flight(s.load.phase) {
                let mut replaced = trace(s, Kind::Cancelled, s.load.requested_url.clone());
                replaced.cancellation_reason = Some(Cancel::Replaced);
                events.push(replaced);
            }
            s.document_generation = s.document_generation.saturating_add(1);
            s.bootstrap_loading = bootstrap;
            s.navigation_source = source;
            if !bootstrap { s.load.navigation_sequence = s.load.navigation_sequence.saturating_add(1); }
            s.load.phase = if bootstrap { Phase::Idle } else { Phase::Requested };
            s.load.content_state = if s.load.content_state == Content::None { Content::None } else { Content::RetainedDocument };
            s.load.requested_url = url.clone();
            s.load.estimated_progress = Some(0);
            s.load.problem = None;
            s.load.response_status = None;
            s.load.cancellation_reason = None;
            s.dialog = None;
            s.permission_requests.clear();
            if bootstrap { None } else { Some(trace(s, Kind::Requested, url)) }
        }
        Update::Started { url } => {
            if s.bootstrap_loading { None } else {
                s.load.phase = Phase::Provisional;
                s.url = url.clone();
                Some(trace(s, Kind::Started, Some(url)))
            }
        }
        Update::Redirect { url } => {
            s.url = url.clone();
            Some(trace(s, Kind::Redirect, Some(url)))
        }
        Update::Committed { url } => {
            if s.bootstrap_loading { None } else {
                s.load.phase = Phase::Committed;
                s.url = url.clone();
                s.load.content_state = if document_url(&url) { Content::CurrentDocument } else { Content::None };
                s.load.content_url = (s.load.content_state != Content::None).then_some(url.clone());
                Some(trace(s, Kind::Committed, Some(url)))
            }
        }
        Update::Finished { url } => {
            if s.bootstrap_loading {
                s.bootstrap_ready = true;
                s.bootstrap_loading = false;
                s.load.phase = Phase::Idle;
                None
            } else {
                s.load.phase = Phase::Finished;
                s.load.estimated_progress = Some(100);
                s.url = url.clone();
                s.load.content_state = if document_url(&url) { Content::CurrentDocument } else { Content::None };
                s.load.content_url = (s.load.content_state != Content::None).then_some(url.clone());
                Some(trace(s, Kind::Finished, Some(url)))
            }
        }
        Update::Cancelled(reason) => {
            s.bootstrap_loading = false;
            s.load.phase = Phase::Cancelled;
            s.load.cancellation_reason = Some(reason);
            s.load.problem = None;
            let mut event = trace(s, Kind::Cancelled, s.load.requested_url.clone());
            event.cancellation_reason = Some(reason);
            Some(event)
        }
        Update::Failed { domain, code } => {
            if domain == "NSURLErrorDomain" && code == -999 {
                return reduce(s, Update::Cancelled(Cancel::Unknown));
            }
            let failure = problem(&domain, code, s.load.phase);
            s.bootstrap_loading = false;
            s.load.phase = Phase::Failed;
            s.load.problem = Some(failure.clone());
            let mut event = trace(s, Kind::Failed, s.load.requested_url.clone());
            event.problem = Some(failure);
            Some(event)
        }
        Update::Crashed | Update::Closed => {
            s.bootstrap_loading = false;
            s.document_generation = s.document_generation.saturating_add(1);
            s.load.phase = Phase::Crashed;
            s.load.content_state = Content::None;
            s.load.content_url = None;
            s.load.problem = None;
            s.load.response_status = None;
            s.load.cancellation_reason = None;
            s.load.estimated_progress = None;
            s.dialog = None;
            s.permission_requests.clear();
            Some(trace(s, Kind::Crashed, None))
        }
        Update::NoNavigation { url } => {
            if s.bootstrap_loading { return reduce(s, Update::Finished { url }); }
            if s.load.content_url.as_ref().is_some_and(|old| same_document(old, &url))
                && s.load.requested_url.as_ref().is_some_and(|requested| same_document(requested, &url)) {
                return reduce(s, Update::Finished { url });
            }
            return reduce(s, Update::Cancelled(Cancel::NavigationRejected));
        }
        Update::Metadata { url, title, back, forward, progress } => {
            if !in_flight(s.load.phase) && s.load.content_state != Content::None && s.load.content_url.as_ref() != Some(&url) {
                s.document_generation = s.document_generation.saturating_add(1);
                s.load.content_url = document_url(&url).then_some(url.clone());
            }
            s.url = url;
            s.title = title;
            s.can_go_back = back;
            s.can_go_forward = forward;
            if in_flight(s.load.phase) { s.load.estimated_progress = Some(progress.min(100)); }
            None
        }
        Update::Response { url, status } => {
            // WKNavigationResponse has no navigation token. A main frame URL
            // is not sufficient proof, especially across same-URL retries.
            let mut event = BrowserNavigationEvent::new(Kind::Response, None, None, Source::Unknown, url);
            event.response_status = status;
            Some(event)
        }
        Update::PolicyCancelled { url, reason } => {
            // Action/response policy callbacks have no WKNavigation identity.
            let mut event = BrowserNavigationEvent::new(Kind::Cancelled, None, None, Source::Unknown, url);
            event.cancellation_reason = Some(reason);
            Some(event)
        }
    };
    s.lifecycle = match s.load.phase {
        Phase::Idle | Phase::Requested | Phase::Provisional | Phase::Committed => BrowserTabLifecycle::Loading,
        Phase::Finished => BrowserTabLifecycle::Ready,
        Phase::Cancelled => BrowserTabLifecycle::Stopped,
        Phase::Failed => BrowserTabLifecycle::Failed,
        Phase::Crashed => BrowserTabLifecycle::Crashed,
    };
    if let Some(event) = event { events.push(event); }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    fn begin(s: &mut PageSnapshot, url: &str) {
        reduce(s, Update::Begin { url: Some(url.into()), source: Source::UserCommand, bootstrap: false });
        reduce(s, Update::Started { url: url.into() });
    }
    #[test]
    fn first_stop_is_not_ready_and_bootstrap_is_not_content() {
        let mut s = PageSnapshot::default();
        reduce(&mut s, Update::Begin { url: None, source: Source::Unknown, bootstrap: true });
        reduce(&mut s, Update::Finished { url: "about:blank".into() });
        assert!(s.bootstrap_ready);
        assert_eq!(s.load.content_state, Content::None);
        begin(&mut s, "https://example.com/");
        reduce(&mut s, Update::Cancelled(Cancel::UserStop));
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Stopped);
        assert_eq!(s.load.content_state, Content::None);
    }
    #[test]
    fn real_blank_popup_can_have_content_without_counting_bootstrap() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "about:blank");
        reduce(&mut s, Update::Committed { url: "about:blank".into() });
        reduce(&mut s, Update::Finished { url: "about:blank".into() });
        assert_eq!(s.load.content_state, Content::CurrentDocument);
        assert_eq!(s.load.content_url.as_deref(), Some("about:blank"));
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Ready);
        assert!(!s.bootstrap_ready);
    }
    #[test]
    fn failed_navigation_retains_old_or_partial_document() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/old");
        reduce(&mut s, Update::Committed { url: "https://example.com/old".into() });
        reduce(&mut s, Update::Finished { url: "https://example.com/old".into() });
        begin(&mut s, "https://example.com/new");
        reduce(&mut s, Update::Failed { domain: "NSURLErrorDomain".into(), code: -1009 });
        assert_eq!(s.load.content_state, Content::RetainedDocument);
        assert_eq!(s.load.content_url.as_deref(), Some("https://example.com/old"));
        begin(&mut s, "https://example.com/new");
        assert!(s.load.problem.is_none());
        reduce(&mut s, Update::Committed { url: "https://example.com/new".into() });
        reduce(&mut s, Update::Failed { domain: "NSURLErrorDomain".into(), code: -1005 });
        assert_eq!(s.load.content_state, Content::CurrentDocument);
        assert_eq!(s.load.problem.unwrap().stage, Stage::Committed);
    }
    #[test]
    fn cancellation_requires_domain_and_crash_cannot_be_repaired_by_metadata() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/");
        reduce(&mut s, Update::Failed { domain: "OtherDomain".into(), code: -999 });
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Failed);
        begin(&mut s, "https://example.com/");
        reduce(&mut s, Update::Failed { domain: "NSURLErrorDomain".into(), code: -999 });
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Stopped);
        reduce(&mut s, Update::Crashed);
        reduce(&mut s, Update::Metadata { url: "https://example.com/".into(), title: "old".into(), back: false, forward: false, progress: 100 });
        assert_eq!(s.load.content_state, Content::None);
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Crashed);
    }
    #[test]
    fn redirects_share_attempt_and_responses_are_uncorrelated() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/");
        let sequence = s.load.navigation_sequence;
        let generation = s.document_generation;
        reduce(&mut s, Update::Redirect { url: "https://example.com/check".into() });
        assert_eq!(s.load.navigation_sequence, sequence);
        assert_eq!(s.document_generation, generation);
        let event = reduce(&mut s, Update::Response { url: Some("https://example.com/check?secret=1".into()), status: Some(403) }).into_iter().next().unwrap();
        assert!(event.navigation_sequence.is_none());
        assert!(event.document_generation.is_none());
        assert!(s.load.response_status.is_none());
    }
    #[test]
    fn replacement_records_old_cancellation_without_publishing_stopped_for_new_attempt() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/first");
        let old_sequence = s.load.navigation_sequence;
        let events = reduce(&mut s, Update::Begin { url: Some("https://example.com/second".into()), source: Source::UserCommand, bootstrap: false });
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].navigation_sequence, Some(old_sequence));
        assert_eq!(events[0].cancellation_reason, Some(Cancel::Replaced));
        assert_eq!(events[1].navigation_sequence, Some(old_sequence + 1));
        assert_eq!(s.load.phase, Phase::Requested);
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Loading);
        assert!(s.load.cancellation_reason.is_none());
    }
}
