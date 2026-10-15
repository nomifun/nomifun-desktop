//! One reducer owns navigation evidence and document availability. Metadata
//! sampling never creates a document or turns cancellation into success.
use crate::state::PageSnapshot;
use nomifun_browser_platform::runtime::{
    BrowserCancellationReason as Cancel, BrowserContentState as Content,
    BrowserFailureReason as Reason, BrowserFailureStage as Stage,
    BrowserNativeDomainClass as Domain, BrowserNavigationEvent, BrowserNavigationEventKind as Kind,
    BrowserNavigationPhase as Phase, BrowserNavigationSource as Source, BrowserPageProblem,
    BrowserTabLifecycle,
};

pub(crate) enum Update {
    Begin { url: Option<String>, source: Source, bootstrap: bool },
    HistoryRequested { source: Source },
    HistoryStarted { url: String, source: Source, dispatch_sequence: u64 },
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
    AutomaticNavigationPaused(bool),
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
    BrowserNavigationEvent::new(kind, Some(s.navigation_attempt_sequence), Some(s.document_generation), s.navigation_source, url)
}

fn begin_navigation(s: &mut PageSnapshot, url: Option<String>, source: Source, bootstrap: bool,
    existing_dispatch: Option<u64>) -> Vec<BrowserNavigationEvent> {
    let mut events = Vec::new();
    if in_flight(s.load.phase) {
        let mut replaced = trace(s, Kind::Cancelled, s.load.requested_url.clone());
        replaced.cancellation_reason = Some(Cancel::Replaced);
        events.push(replaced);
    }
    s.document_generation = s.document_generation.saturating_add(1);
    s.bootstrap_loading = bootstrap;
    s.navigation_source = source;
    if !bootstrap { s.load.navigation_sequence = s.load.navigation_sequence.saturating_add(1); }
    s.navigation_attempt_sequence = s.load.navigation_sequence;
    s.load.phase = if bootstrap { Phase::Idle } else { Phase::Requested };
    s.load.content_state = if s.load.content_state == Content::None { Content::None } else { Content::RetainedDocument };
    s.load.requested_url = url.clone();
    s.load.estimated_progress = Some(0);
    s.load.problem = None;
    s.load.response_status = None;
    s.load.cancellation_reason = None;
    s.dialog = None;
    s.blocked_permissions.clear();
    s.permission_requests.clear();
    s.lifecycle = BrowserTabLifecycle::Loading;
    if !bootstrap {
        let mut requested = trace(s, Kind::Requested, url);
        if matches!(source, Source::UserCommand | Source::AgentCommand) {
            requested.dispatch_sequence = Some(existing_dispatch.unwrap_or_else(|| {
                s.navigation_dispatch_sequence = s.navigation_dispatch_sequence.saturating_add(1);
                s.navigation_dispatch_sequence
            }));
        }
        events.push(requested);
    }
    events
}

pub(crate) fn reduce(s: &mut PageSnapshot, update: Update) -> Vec<BrowserNavigationEvent> {
    let mut events = Vec::new();
    let event = match update {
        Update::Begin { url, source, bootstrap } => {
            return begin_navigation(s, url, source, bootstrap, None);
        }
        Update::HistoryRequested { source } => {
            // A native history request may only traverse pushState/fragment
            // entries. Its ACK and URL changes do not prove a document load.
            s.load.navigation_sequence = s.load.navigation_sequence.saturating_add(1);
            s.navigation_dispatch_sequence = s.navigation_dispatch_sequence.saturating_add(1);
            let mut event = BrowserNavigationEvent::new(Kind::Requested, None, None, source, None);
            event.dispatch_sequence = Some(s.navigation_dispatch_sequence);
            event.action = Some(nomifun_browser_platform::runtime::BrowserNavigationActionSummary {
                navigation_type: nomifun_browser_platform::runtime::BrowserNavigationActionType::BackForward,
                main_frame: true,
                ..Default::default()
            });
            Some(event)
        }
        Update::HistoryStarted { url, source, dispatch_sequence } => {
            return begin_navigation(s, Some(url), source, false, Some(dispatch_sequence));
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
                // Public WK metadata cannot prove same-document identity. Keep
                // the existing conservative invalidation of DOM observations;
                // this is not evidence that another document was requested.
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
        Update::AutomaticNavigationPaused(paused) => {
            s.load.navigation_control.get_or_insert_with(Default::default).paused = paused;
            None
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
    fn reload_starts_a_new_document_without_old_permission_denials() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/");
        reduce(&mut s, Update::Finished { url: "https://example.com/".into() });
        let generation = s.document_generation;
        s.blocked_permissions = vec!["camera".into(), "microphone".into()];
        s.permission_requests.push(nomifun_browser_platform::runtime::BrowserPermissionRequest {
            request_id: "old-camera".into(), kind: "camera".into(), origin: "https://example.com".into(),
        });
        reduce(&mut s, Update::Begin { url: Some("https://example.com/".into()), source: Source::UserCommand, bootstrap: false });
        assert!(s.document_generation > generation);
        assert!(s.blocked_permissions.is_empty());
        assert!(s.permission_requests.is_empty());
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
    #[test]
    fn pause_keeps_the_current_document_and_metadata_cannot_resume_it() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/current");
        reduce(&mut s, Update::Committed { url: "https://example.com/current".into() });
        reduce(&mut s, Update::Finished { url: "https://example.com/current".into() });
        let generation = s.document_generation;
        let sequence = s.load.navigation_sequence;
        assert!(reduce(&mut s, Update::AutomaticNavigationPaused(true)).is_empty());
        reduce(&mut s, Update::Metadata { url: "https://example.com/current".into(), title: "retained".into(), back: false, forward: false, progress: 100 });
        assert!(s.load.navigation_control.as_ref().unwrap().paused);
        assert_eq!(s.load.content_state, Content::CurrentDocument);
        assert_eq!(s.document_generation, generation);
        assert_eq!(s.load.navigation_sequence, sequence);
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Ready);
    }
    #[test]
    fn pausing_in_flight_has_a_distinct_reason_and_resume_replays_nothing() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/repeat");
        let generation = s.document_generation;
        let sequence = s.load.navigation_sequence;
        reduce(&mut s, Update::AutomaticNavigationPaused(true));
        let events = reduce(&mut s, Update::Cancelled(Cancel::AutomaticNavigationPaused));
        assert_eq!(events[0].cancellation_reason, Some(Cancel::AutomaticNavigationPaused));
        assert_eq!(s.load.phase, Phase::Cancelled);
        assert_eq!(s.load.content_state, Content::None);
        assert!(s.load.navigation_control.as_ref().unwrap().paused);
        assert!(reduce(&mut s, Update::AutomaticNavigationPaused(false)).is_empty());
        assert_eq!(s.load.phase, Phase::Cancelled);
        assert!(!s.load.navigation_control.as_ref().unwrap().paused);
        assert_eq!(s.document_generation, generation);
        assert_eq!(s.load.navigation_sequence, sequence);
    }
    #[test]
    fn a_paused_policy_sample_does_not_invent_a_navigation_attempt() {
        let mut s = PageSnapshot::default();
        let events = reduce(&mut s, Update::PolicyCancelled {
            url: Some("https://example.com/repeat".into()), reason: Cancel::AutomaticNavigationPaused,
        });
        assert_eq!(events[0].navigation_sequence, None);
        assert_eq!(events[0].document_generation, None);
        assert_eq!(s.load.navigation_sequence, 0);
    }

    #[test]
    fn history_submission_preserves_the_loaded_document_without_native_start() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/spa?step=2");
        reduce(&mut s, Update::Finished { url: "https://example.com/spa?step=2".into() });
        let old_generation = s.document_generation;
        let old_sequence = s.load.navigation_sequence;
        let old_attempt = s.navigation_attempt_sequence;
        let events = reduce(&mut s, Update::HistoryRequested { source: Source::UserCommand });
        assert_eq!(s.load.phase, Phase::Finished);
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Ready);
        assert_eq!(s.load.estimated_progress, Some(100));
        assert_eq!(s.load.content_state, Content::CurrentDocument);
        assert_eq!(s.document_generation, old_generation);
        assert!(s.load.navigation_sequence > old_sequence);
        assert_eq!(s.navigation_attempt_sequence, old_attempt);
        assert_eq!(events.len(), 1);
        assert!(events[0].navigation_sequence.is_none());
        assert!(events[0].document_generation.is_none());
        assert!(events[0].dispatch_sequence.is_some());

        let before_metadata = s.load.clone();
        let events = reduce(&mut s, Update::Metadata { url: "https://example.com/spa?step=1".into(),
            title: "same document".into(), back: true, forward: true, progress: 100 });
        assert!(events.is_empty(), "URL changes cannot synthesize a native Finished event");
        assert_eq!(s.load.phase, before_metadata.phase);
        assert_eq!(s.load.estimated_progress, before_metadata.estimated_progress);
        assert_eq!(s.load.navigation_sequence, before_metadata.navigation_sequence);
        assert!(s.document_generation > old_generation, "unproven WK URL identity still revokes prior DOM observations conservatively");
    }

    #[test]
    fn history_fence_does_not_rebind_an_in_flight_native_attempt() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/current");
        let attempt = s.load.navigation_sequence;
        reduce(&mut s, Update::HistoryRequested { source: Source::UserCommand });
        let fence = s.load.navigation_sequence;
        assert!(fence > attempt);
        assert_eq!(s.load.phase, Phase::Provisional);
        let events = reduce(&mut s, Update::Finished { url: "https://example.com/current".into() });
        assert_eq!(events[0].navigation_sequence, Some(attempt));
        assert_eq!(s.load.navigation_sequence, fence);
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Ready);
    }

    #[test]
    fn only_a_native_history_start_creates_a_new_load_and_dispatch_counts_once() {
        let mut s = PageSnapshot::default();
        let mut trace = nomifun_browser_platform::runtime::BrowserNavigationTrace::default();
        for event in reduce(&mut s, Update::Begin { url: Some("https://example.com/one".into()), source: Source::UserCommand, bootstrap: false }) { trace.push(event); }
        reduce(&mut s, Update::Finished { url: "https://example.com/one".into() });
        let generation = s.document_generation;
        for event in reduce(&mut s, Update::HistoryRequested { source: Source::UserCommand }) { trace.push(event); }
        let dispatch_sequence = s.navigation_dispatch_sequence;
        assert_eq!(s.load.phase, Phase::Finished);
        let events = reduce(&mut s, Update::HistoryStarted { url: "https://example.com/two".into(), source: Source::UserCommand, dispatch_sequence });
        assert_eq!(s.load.phase, Phase::Requested);
        assert_eq!(s.load.content_state, Content::RetainedDocument);
        assert!(s.document_generation > generation);
        for event in events { trace.push(event); }
        assert_eq!(trace.snapshot().counters.user_commands, 2);
        assert_eq!(s.navigation_dispatch_sequence, dispatch_sequence);
    }

    #[test]
    fn history_dispatch_keeps_failure_and_retained_document_evidence() {
        let mut s = PageSnapshot::default();
        begin(&mut s, "https://example.com/loaded");
        reduce(&mut s, Update::Finished { url: "https://example.com/loaded".into() });
        begin(&mut s, "https://example.com/disconnected");
        reduce(&mut s, Update::Failed { domain: "NSURLErrorDomain".into(), code: -1005 });
        let before = s.load.clone();
        reduce(&mut s, Update::HistoryRequested { source: Source::UserCommand });
        assert_eq!(s.load.problem, before.problem);
        assert_eq!(s.load.phase, before.phase);
        assert_eq!(s.load.content_url, before.content_url);
        assert_eq!(s.load.content_state, Content::RetainedDocument);
        assert_eq!(s.lifecycle, BrowserTabLifecycle::Failed);
    }
}
