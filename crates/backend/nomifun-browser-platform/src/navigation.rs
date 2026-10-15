//! Bounded, read-only native navigation facts. No page bodies or execution handles.
use crate::{runtime::{BrowserProfilePersistence, BrowserTabTarget}, url_projection::project_metadata_url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, time::{Duration, Instant}};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationPhase {
    #[default]
    Idle,
    Requested,
    Provisional,
    Committed,
    Finished,
    Cancelled,
    Failed,
    Crashed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserContentState {
    #[default]
    None,
    RetainedDocument,
    CurrentDocument,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCancellationReason {
    UserStop,
    Replaced,
    Download,
    NavigationRejected,
    AutomaticNavigationPaused,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFailureStage { Provisional, Committed }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFailureReason {
    NetworkConnection,
    Dns,
    Timeout,
    Tls,
    UnsupportedUrl,
    AccessDenied,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNativeDomainClass { Url, WebKit, WebView2, Other }

/// Native codes only; localizedDescription and userInfo may contain credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserPageProblem {
    pub stage: BrowserFailureStage,
    pub safe_reason: BrowserFailureReason,
    pub native_domain_class: BrowserNativeDomainClass,
    pub native_code: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserLoadSummary {
    pub navigation_sequence: u64,
    pub phase: BrowserNavigationPhase,
    pub content_state: BrowserContentState,
    /// A rounded percentage in 0..=100, not proof of business operation success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_progress: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation_reason: Option<BrowserCancellationReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<BrowserPageProblem>,
    /// Only populated if the adapter proves association to this exact attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<u16>,
    /// Original address for the user's workspace. Agent/diagnostic output must
    /// use agent_projection, never serialize a workspace copy directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_url: Option<String>,
    /// The actually retained/current document address, not the attempted one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_url: Option<String>,
    /// Transient Page navigation controls; absence means the adapter does not
    /// support this control. A loop warning is advisory until the user pauses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_control: Option<BrowserNavigationControl>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserNavigationControl {
    pub paused: bool,
    pub loop_suspected: bool,
    pub repeated_navigation_count: u32,
}

impl BrowserLoadSummary {
    pub fn agent_projection(&self) -> Self {
        let mut projected = self.clone();
        projected.requested_url = self.requested_url.as_deref().map(project_metadata_url);
        projected.content_url = self.content_url.as_deref().map(project_metadata_url);
        projected
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationEventKind {
    Requested, Started, Redirect, Response, Committed, Finished, Cancelled, Failed, Crashed, Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationSource { UserCommand, AgentCommand, PageNavigation, Unknown }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationActionType {
    LinkActivated, FormSubmitted, BackForward, Reload, FormResubmitted, Other,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationHttpMethod {
    Get, Post, Head, Put, Patch, Delete, Options, Connect, Trace, Other,
    #[default]
    Unknown,
}

impl BrowserNavigationHttpMethod {
    /// Classify without retaining an untrusted method string in diagnostics.
    pub fn from_method(method: &str) -> Self {
        if method.len() > 16 { return Self::Other; }
        match method.to_ascii_uppercase().as_str() {
            "GET" => Self::Get, "POST" => Self::Post, "HEAD" => Self::Head,
            "PUT" => Self::Put, "PATCH" => Self::Patch, "DELETE" => Self::Delete,
            "OPTIONS" => Self::Options, "CONNECT" => Self::Connect, "TRACE" => Self::Trace,
            "" => Self::Unknown, _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationPolicy {
    #[default]
    Allow,
    Cancel, Download, Paused,
}

/// Public native action facts. These are not proof of a physical human gesture
/// or permission to associate an action callback with the latest attempt.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserNavigationActionSummary {
    pub navigation_type: BrowserNavigationActionType,
    pub main_frame: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_main_frame: Option<bool>,
    pub http_method: BrowserNavigationHttpMethod,
    pub policy: BrowserNavigationPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_initiated: Option<bool>,
}

impl BrowserNavigationActionSummary {
    fn project_metadata(&mut self) {
        self.source_origin = self.source_origin.as_deref().and_then(|value| {
            let parsed = url::Url::parse(value).ok()?;
            matches!(parsed.scheme(), "http" | "https")
                .then(|| clip(&parsed.origin().ascii_serialization()))
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserNavigationEvent {
    pub sequence: u64,
    pub elapsed_ms: u64,
    /// Page-local host submission ordinal. It proves a command dispatch, not
    /// association with a native navigation or committed document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_generation: Option<u64>,
    pub kind: BrowserNavigationEventKind,
    pub source: BrowserNavigationSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<BrowserPageProblem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation_reason: Option<BrowserCancellationReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<BrowserNavigationActionSummary>,
}

impl BrowserNavigationEvent {
    pub fn new(
        kind: BrowserNavigationEventKind,
        navigation_sequence: Option<u64>,
        document_generation: Option<u64>,
        source: BrowserNavigationSource,
        url: Option<String>,
    ) -> Self {
        Self { sequence: 0, elapsed_ms: 0, dispatch_sequence: None, navigation_sequence, document_generation,
            kind, source, url, response_status: None, problem: None, cancellation_reason: None, action: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationAvailability { Supported, Degraded, Unavailable }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationScope { MainDocumentNavigation }

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserNavigationTraceSnapshot {
    pub scope: BrowserNavigationScope,
    pub availability: BrowserNavigationAvailability,
    pub events: Vec<BrowserNavigationEvent>,
    /// Events omitted for capacity. This is distinct from lost routing proof.
    pub dropped: u64,
    pub routing_lost: bool,
    /// Cumulative Page counters are independent of the retained event ring.
    pub counters: BrowserNavigationCounters,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BrowserNavigationCounters {
    pub user_commands: u64,
    pub agent_commands: u64,
    pub page_navigations: u64,
    pub unknown_navigations: u64,
    pub redirects: u64,
}

pub const MAX_NAVIGATION_ATTEMPTS: usize = 32;
pub const MAX_NAVIGATION_EVENTS: usize = 128;
pub const MAX_NAVIGATION_TRACE_BYTES: usize = 128 * 1024;
const MAX_METADATA_CHARS: usize = 512;
const REPEAT_WINDOW: Duration = Duration::from_secs(10);
const REPEAT_WARNING_THRESHOLD: u32 = 5;

/// Owned by one native Page, retained across its navigations and destroyed
/// with that Page. It is neither a persistent history nor an authorization ID.
pub struct BrowserNavigationTrace {
    started: Instant,
    next_sequence: u64,
    events: VecDeque<(BrowserNavigationEvent, usize)>,
    retained_bytes: usize,
    dropped: u64,
    routing_lost: bool,
    counters: BrowserNavigationCounters,
    requested_high_water: Option<u64>,
    dispatch_high_water: Option<u64>,
    equality_salt: [u8; 16],
    recent_page_attempts: VecDeque<(Instant, [u8; 32])>,
}

impl std::fmt::Debug for BrowserNavigationTrace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("BrowserNavigationTrace")
            .field("events", &self.events)
            .field("retained_bytes", &self.retained_bytes)
            .field("dropped", &self.dropped)
            .field("routing_lost", &self.routing_lost)
            .field("counters", &self.counters)
            .field("recent_page_attempt_count", &self.recent_page_attempts.len())
            .finish_non_exhaustive()
    }
}

impl Default for BrowserNavigationTrace {
    fn default() -> Self {
        Self { started: Instant::now(), next_sequence: 0, events: VecDeque::new(),
            retained_bytes: 0, dropped: 0, routing_lost: false,
            counters: BrowserNavigationCounters::default(), requested_high_water: None,
            dispatch_high_water: None,
            equality_salt: *uuid::Uuid::now_v7().as_bytes(), recent_page_attempts: VecDeque::new() }
    }
}

impl BrowserNavigationTrace {
    pub fn push(&mut self, mut event: BrowserNavigationEvent) {
        self.record_attempt(&event, Instant::now());
        self.next_sequence = self.next_sequence.saturating_add(1);
        event.sequence = self.next_sequence;
        event.elapsed_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        event.url = event.url.as_deref().map(|url| clip(&project_metadata_url(url)));
        if let Some(action) = &mut event.action { action.project_metadata(); }
        if event.navigation_sequence.is_none() {
            // A Page-scoped response without a native attempt identity cannot
            // be attributed to the latest document merely because URLs match.
            event.document_generation = None;
        }
        let bytes = serde_json::to_vec(&event).map_or(MAX_NAVIGATION_TRACE_BYTES + 1, |v| v.len());
        if bytes > MAX_NAVIGATION_TRACE_BYTES {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        self.events.push_back((event, bytes));
        loop {
            let attempts: std::collections::BTreeSet<_> = self.events.iter()
                .filter_map(|(event, _)| event.navigation_sequence).collect();
            if self.events.len() <= MAX_NAVIGATION_EVENTS
                && self.retained_bytes <= MAX_NAVIGATION_TRACE_BYTES
                && attempts.len() <= MAX_NAVIGATION_ATTEMPTS { break; }
            let Some((_, bytes)) = self.events.pop_front() else { break; };
            self.retained_bytes = self.retained_bytes.saturating_sub(bytes);
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    pub fn mark_routing_lost(&mut self) { self.routing_lost = true; }

    fn record_attempt(&mut self, event: &BrowserNavigationEvent, now: Instant) {
        if event.action.as_ref().is_some_and(|action| !action.main_frame) { return; }
        if event.kind == BrowserNavigationEventKind::Redirect {
            self.counters.redirects = self.counters.redirects.saturating_add(1);
            return;
        }
        if event.kind != BrowserNavigationEventKind::Requested { return; }
        if matches!(event.source, BrowserNavigationSource::UserCommand | BrowserNavigationSource::AgentCommand)
            && let Some(dispatch) = event.dispatch_sequence {
            // CDP submission receipts cannot prove NavigationId association.
            // Track the exact host submission independently; never borrow its
            // ordinal as a native attempt or document generation.
            if self.dispatch_high_water.is_some_and(|previous| dispatch <= previous) { return; }
            self.dispatch_high_water = Some(dispatch);
            let counter = if event.source == BrowserNavigationSource::UserCommand {
                &mut self.counters.user_commands
            } else { &mut self.counters.agent_commands };
            *counter = counter.saturating_add(1);
            self.recent_page_attempts.clear();
            return;
        }
        // Requested sequences are Page-local and monotonically allocated by
        // the native owner. Late/duplicate facts cannot inflate total counts.
        let Some(sequence) = event.navigation_sequence else { return; };
        if self.requested_high_water.is_some_and(|previous| sequence <= previous) { return; }
        self.requested_high_water = Some(sequence);
        let counter = match event.source {
            BrowserNavigationSource::UserCommand => &mut self.counters.user_commands,
            BrowserNavigationSource::AgentCommand => &mut self.counters.agent_commands,
            BrowserNavigationSource::PageNavigation => &mut self.counters.page_navigations,
            BrowserNavigationSource::Unknown => &mut self.counters.unknown_navigations,
        };
        *counter = counter.saturating_add(1);
        if matches!(event.source, BrowserNavigationSource::UserCommand | BrowserNavigationSource::AgentCommand)
            || (event.source == BrowserNavigationSource::Unknown
                && !event.action.as_ref().is_some_and(|action| action.main_frame)) {
            // An explicit host navigation starts a new observation window.
            self.recent_page_attempts.clear();
            return;
        }
        if event.action.as_ref().is_some_and(|action| action.user_initiated == Some(true)) {
            self.recent_page_attempts.clear();
            return;
        }
        let Some(raw_url) = event.url.as_deref() else { return; };
        // Keep only salted equality keys. Raw query/credentials never survive
        // this call or appear in exported diagnostics, but differing searches
        // cannot collapse to the same redacted address.
        let mut hash = Sha256::new();
        hash.update(self.equality_salt);
        hash.update(raw_url.as_bytes());
        let key: [u8; 32] = hash.finalize().into();
        self.recent_page_attempts.push_back((now, key));
        while self.recent_page_attempts.len() > MAX_NAVIGATION_ATTEMPTS
            || self.recent_page_attempts.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) > REPEAT_WINDOW) {
            self.recent_page_attempts.pop_front();
        }
    }

    /// Advisory repeat state from distinct associated main-document attempts.
    /// Unknown sources stay unclassified even when their repetitions warn.
    /// Redirects and unassociated actions never count as reloads.
    pub fn control(&self, paused: bool) -> BrowserNavigationControl {
        self.control_at(paused, Instant::now())
    }

    /// Explicit host continuation starts a fresh advisory observation window.
    /// This does not erase cumulative counters or retained diagnostic facts.
    pub fn clear_repeat_window(&mut self) { self.recent_page_attempts.clear(); }

    fn control_at(&self, paused: bool, now: Instant) -> BrowserNavigationControl {
        let keys: Vec<_> = self.recent_page_attempts.iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= REPEAT_WINDOW)
            .map(|(_, key)| key).collect();
        let Some(last) = keys.last() else { return BrowserNavigationControl { paused, ..Default::default() }; };
        let same = keys.iter().rev().take_while(|key| *key == last).count();
        let mut alternating = 1;
        if keys.len() > 1 {
            let previous = keys[keys.len() - 2];
            if previous != *last {
                alternating = keys.iter().rev().enumerate()
                    .take_while(|(index, key)| **key == if index % 2 == 0 { *last } else { previous })
                    .count();
            }
        }
        let repeated_navigation_count = same.max(alternating).min(u32::MAX as usize) as u32;
        BrowserNavigationControl { paused, loop_suspected: repeated_navigation_count >= REPEAT_WARNING_THRESHOLD,
            repeated_navigation_count }
    }

    pub fn snapshot(&self) -> BrowserNavigationTraceSnapshot {
        BrowserNavigationTraceSnapshot {
            scope: BrowserNavigationScope::MainDocumentNavigation,
            availability: if self.routing_lost { BrowserNavigationAvailability::Degraded }
                else { BrowserNavigationAvailability::Supported },
            events: self.events.iter().map(|(event, _)| event.clone()).collect(),
            dropped: self.dropped,
            routing_lost: self.routing_lost,
            counters: self.counters.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserIdentityReport {
    pub profile_id: String,
    pub policy_revision: u32,
    pub engine_family: String,
    pub effective_user_agent: String,
    pub advertised_compatibility_version: String,
    pub version_source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BrowserRuntimeInfo {
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webkit_framework_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_persistence: Option<BrowserProfilePersistence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_identity_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_overrides_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_overrides_present: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserNavigationReport {
    pub target: BrowserTabTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<BrowserLoadSummary>,
    pub trace: BrowserNavigationTraceSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<BrowserIdentityReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<BrowserRuntimeInfo>,
}

impl BrowserNavigationReport {
    /// Final output boundary: projects raw workspace addresses and bounds
    /// implementation metadata. No automatic upload or page-accessible route.
    pub fn project_metadata(mut self) -> Self {
        self.load = self.load.as_ref().map(BrowserLoadSummary::agent_projection);
        if let Some(load) = &mut self.load {
            load.requested_url = load.requested_url.as_deref().map(clip);
            load.content_url = load.content_url.as_deref().map(clip);
        }
        for event in &mut self.trace.events {
            event.url = event.url.as_deref().map(|url| clip(&project_metadata_url(url)));
            if let Some(action) = &mut event.action { action.project_metadata(); }
            if event.navigation_sequence.is_none() {
                event.document_generation = None;
            }
        }
        if let Some(identity) = &mut self.identity {
            for value in [&mut identity.profile_id, &mut identity.engine_family,
                &mut identity.effective_user_agent, &mut identity.advertised_compatibility_version,
                &mut identity.version_source] { *value = clip(value); }
        }
        if let Some(runtime) = &mut self.runtime {
            runtime.os_version = clip(&runtime.os_version);
            runtime.os_build = runtime.os_build.as_deref().map(clip);
            runtime.webkit_framework_version = runtime.webkit_framework_version.as_deref().map(clip);
            runtime.browser_version = runtime.browser_version.as_deref().map(clip);
            runtime.browser_channel = runtime.browser_channel.as_deref().map(clip);
            runtime.profile_identity_token = runtime.profile_identity_token.as_deref().map(clip);
        }
        self
    }
}

fn clip(value: &str) -> String { value.chars().take(MAX_METADATA_CHARS).collect() }

#[cfg(test)]
mod tests {
    use super::*;

    fn event(navigation: Option<u64>) -> BrowserNavigationEvent {
        BrowserNavigationEvent::new(BrowserNavigationEventKind::Started, navigation, Some(9),
            BrowserNavigationSource::Unknown, Some("https://user:secret@example.test/path?q=private#hidden".into()))
    }

    #[test]
    fn trace_projects_addresses_and_never_assigns_unassociated_response_to_latest_document() {
        let mut trace = BrowserNavigationTrace::default();
        let mut response = event(None);
        response.kind = BrowserNavigationEventKind::Response;
        response.response_status = Some(200);
        trace.push(response);
        let snapshot = trace.snapshot();
        assert_eq!(snapshot.events[0].url.as_deref(), Some("https://example.test/path"));
        assert_eq!(snapshot.events[0].document_generation, None);
        assert_eq!(snapshot.events[0].navigation_sequence, None);
        assert_eq!(snapshot.events[0].response_status, Some(200));
        let text = serde_json::to_string(&snapshot).unwrap();
        for secret in ["secret", "private", "hidden", "user:"] { assert!(!text.contains(secret)); }
    }

    #[test]
    fn capacity_loss_is_bounded_and_distinct_from_native_routing_loss() {
        let mut trace = BrowserNavigationTrace::default();
        for _ in 0..150 { trace.push(event(Some(1))); }
        let snapshot = trace.snapshot();
        assert_eq!(snapshot.events.len(), MAX_NAVIGATION_EVENTS);
        assert_eq!(snapshot.dropped, 22);
        assert_eq!(snapshot.availability, BrowserNavigationAvailability::Supported);
        assert!(!snapshot.routing_lost);
        assert_eq!(snapshot.events.first().unwrap().sequence, 23);
        trace.mark_routing_lost();
        assert_eq!(trace.snapshot().availability, BrowserNavigationAvailability::Degraded);
    }

    #[test]
    fn retains_only_recent_attempts_without_erasing_trace_on_new_document() {
        let mut trace = BrowserNavigationTrace::default();
        for attempt in 1..=40 { trace.push(event(Some(attempt))); }
        let snapshot = trace.snapshot();
        assert_eq!(snapshot.events.len(), MAX_NAVIGATION_ATTEMPTS);
        assert_eq!(snapshot.dropped, 8);
        assert_eq!(snapshot.events.first().unwrap().navigation_sequence, Some(9));
        assert_eq!(snapshot.events.last().unwrap().navigation_sequence, Some(40));
    }

    #[test]
    fn utf8_and_byte_budget_are_bounded() {
        let mut trace = BrowserNavigationTrace::default();
        for _ in 0..200 {
            let mut large = event(Some(1));
            large.url = Some(format!("https://example.test/{}", "界".repeat(2000)));
            trace.push(large);
        }
        assert!(trace.retained_bytes <= MAX_NAVIGATION_TRACE_BYTES);
        let snapshot = trace.snapshot();
        assert!(snapshot.events.len() <= MAX_NAVIGATION_EVENTS);
        assert!(snapshot.events.iter().all(|entry| entry.url.as_ref().unwrap().chars().count() <= MAX_METADATA_CHARS));
        assert!(snapshot.dropped > 0);
    }

    #[test]
    fn agent_load_projection_removes_original_address_secrets() {
        let load = BrowserLoadSummary { requested_url: Some("https://name:pass@example.test/find?token=secret#private".into()), ..Default::default() };
        assert_eq!(load.agent_projection().requested_url.as_deref(), Some("https://example.test/find"));
        assert!(load.requested_url.unwrap().contains("secret"));
    }

    #[test]
    fn report_output_projects_both_addresses_and_bounds_runtime_metadata() {
        let raw = "https://name:pass@example.test/find?token=secret#private";
        let load = BrowserLoadSummary {
            requested_url: Some(raw.into()), content_url: Some(raw.into()), ..Default::default()
        };
        let report = BrowserNavigationReport {
            target: BrowserTabTarget { tab_id: "owned".into(), runtime_generation: 1, document_generation: 9 },
            load: Some(load),
            trace: BrowserNavigationTraceSnapshot {
                scope: BrowserNavigationScope::MainDocumentNavigation,
                availability: BrowserNavigationAvailability::Supported,
                events: vec![event(None)], dropped: 0, routing_lost: false, counters: Default::default(),
            },
            identity: None,
            runtime: Some(BrowserRuntimeInfo {
                os_version: "界".repeat(2000), ..Default::default()
            }),
        }.project_metadata();
        assert_eq!(report.runtime.as_ref().unwrap().os_version.chars().count(), MAX_METADATA_CHARS);
        let load = report.load.as_ref().unwrap();
        assert_eq!(load.requested_url.as_deref(), Some("https://example.test/find"));
        assert_eq!(load.content_url.as_deref(), Some("https://example.test/find"));
        assert_eq!(report.trace.events[0].document_generation, None);
        let serialized = serde_json::to_string(&report).unwrap();
        for secret in ["pass", "secret", "private", "user:"] { assert!(!serialized.contains(secret)); }
    }

    fn requested(sequence: u64, source: BrowserNavigationSource, url: &str) -> BrowserNavigationEvent {
        BrowserNavigationEvent::new(BrowserNavigationEventKind::Requested, Some(sequence), Some(sequence), source, Some(url.into()))
    }

    #[test]
    fn cumulative_counts_survive_eviction_and_do_not_count_late_or_duplicate_requests() {
        let mut trace = BrowserNavigationTrace::default();
        for sequence in 1..=150 {
            trace.push(requested(sequence, BrowserNavigationSource::UserCommand, "https://example.test/"));
        }
        trace.push(requested(150, BrowserNavigationSource::UserCommand, "https://example.test/"));
        trace.push(requested(1, BrowserNavigationSource::UserCommand, "https://example.test/"));
        trace.push(requested(151, BrowserNavigationSource::AgentCommand, "https://example.test/"));
        trace.push(requested(152, BrowserNavigationSource::PageNavigation, "https://example.test/"));
        trace.push(requested(153, BrowserNavigationSource::Unknown, "https://example.test/"));
        let mut redirect = event(Some(153));
        redirect.kind = BrowserNavigationEventKind::Redirect;
        trace.push(redirect);
        let snapshot = trace.snapshot();
        assert_eq!(snapshot.counters, BrowserNavigationCounters {
            user_commands: 150, agent_commands: 1, page_navigations: 1, unknown_navigations: 1, redirects: 1,
        });
        assert!(snapshot.dropped > 0);
        assert!(snapshot.events.len() <= MAX_NAVIGATION_EVENTS);
    }

    #[test]
    fn host_dispatch_ordinal_counts_without_inventing_native_attempt_or_document_association() {
        let mut trace = BrowserNavigationTrace::default();
        let mut host = BrowserNavigationEvent::new(BrowserNavigationEventKind::Requested,
            None, Some(99), BrowserNavigationSource::UserCommand, Some("https://example.test/?secret=hidden".into()));
        host.dispatch_sequence = Some(1);
        trace.push(host.clone());
        trace.push(host.clone());
        trace.push(requested(1, BrowserNavigationSource::Unknown, "https://example.test/"));
        trace.push(requested(100, BrowserNavigationSource::Unknown, "https://example.test/other"));
        host.source = BrowserNavigationSource::AgentCommand;
        host.dispatch_sequence = Some(2);
        trace.push(host.clone());
        host.source = BrowserNavigationSource::UserCommand;
        host.dispatch_sequence = Some(3);
        trace.push(host);
        let snapshot = trace.snapshot();
        assert_eq!(snapshot.counters, BrowserNavigationCounters {
            user_commands: 2, agent_commands: 1, unknown_navigations: 2, ..Default::default()
        });
        assert_eq!(snapshot.events[0].dispatch_sequence, Some(1));
        assert_eq!(snapshot.events[0].navigation_sequence, None);
        assert_eq!(snapshot.events[0].document_generation, None);
        assert_eq!(snapshot.events[2].navigation_sequence, Some(1));
        assert_eq!(snapshot.events[2].dispatch_sequence, None);
        assert!(!trace.control(false).loop_suspected);
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("hidden"));
    }

    #[test]
    fn repeated_page_requests_warn_but_redirects_and_unassociated_actions_do_not() {
        let mut trace = BrowserNavigationTrace::default();
        for sequence in 1..=4 {
            trace.push(requested(sequence, BrowserNavigationSource::PageNavigation, "https://example.test/find?q=secret"));
        }
        assert!(!trace.control(false).loop_suspected);
        for _ in 0..15 {
            let mut redirect = event(Some(4));
            redirect.kind = BrowserNavigationEventKind::Redirect;
            trace.push(redirect);
            let mut action = event(None);
            action.kind = BrowserNavigationEventKind::Action;
            trace.push(action);
        }
        assert_eq!(trace.control(false).repeated_navigation_count, 4);
        trace.push(requested(5, BrowserNavigationSource::PageNavigation, "https://example.test/find?q=secret"));
        assert_eq!(trace.control(false), BrowserNavigationControl { paused: false, loop_suspected: true, repeated_navigation_count: 5 });
        assert!(trace.control(true).paused);
        assert_eq!(trace.snapshot().counters.page_navigations, 5);
        assert!(!serde_json::to_string(&trace.snapshot()).unwrap().contains("secret"));
    }

    #[test]
    fn unknown_native_repetitions_warn_without_source_guessing_and_host_dispatch_resets_window() {
        let mut trace = BrowserNavigationTrace::default();
        for sequence in 1..=5 {
            let mut native = requested(sequence, BrowserNavigationSource::Unknown, "https://example.test/refresh");
            native.action = Some(BrowserNavigationActionSummary { main_frame: true, ..Default::default() });
            trace.push(native);
        }
        assert!(trace.control(false).loop_suspected);
        assert_eq!(trace.snapshot().counters.unknown_navigations, 5);
        assert_eq!(trace.snapshot().counters.page_navigations, 0);
        assert!(trace.snapshot().events.iter().all(|event| event.source == BrowserNavigationSource::Unknown));
        let mut host = BrowserNavigationEvent::new(BrowserNavigationEventKind::Requested, None, None,
            BrowserNavigationSource::UserCommand, Some("https://example.test/refresh".into()));
        host.dispatch_sequence = Some(1);
        trace.push(host);
        assert!(!trace.control(false).loop_suspected);
        assert_eq!(trace.snapshot().counters.user_commands, 1);
        for sequence in 6..=10 {
            let mut native = requested(sequence, BrowserNavigationSource::Unknown, "https://example.test/refresh");
            native.action = Some(BrowserNavigationActionSummary { main_frame: true, user_initiated: Some(true), ..Default::default() });
            trace.push(native);
        }
        assert!(!trace.control(false).loop_suspected);
    }

    #[test]
    fn raw_query_equality_and_alternating_pairs_are_distinct_from_safe_url_projection() {
        let mut trace = BrowserNavigationTrace::default();
        for sequence in 1..=8 {
            trace.push(requested(sequence, BrowserNavigationSource::PageNavigation, &format!("https://example.test/find?q={sequence}")));
        }
        assert!(!trace.control(false).loop_suspected);
        assert!(trace.snapshot().events.iter().all(|event| event.url.as_deref() == Some("https://example.test/find")));
        trace.clear_repeat_window();
        for sequence in 9..=13 {
            trace.push(requested(sequence, BrowserNavigationSource::PageNavigation,
                if sequence % 2 == 0 { "https://example.test/a" } else { "https://example.test/b" }));
        }
        assert!(trace.control(false).loop_suspected);
        assert_eq!(trace.control(false).repeated_navigation_count, 5);
        trace.clear_repeat_window();
        assert!(!trace.control(false).loop_suspected);
        assert_eq!(trace.snapshot().counters.page_navigations, 13);
    }

    #[test]
    fn repeat_window_expires_is_bounded_and_excludes_proven_human_navigation() {
        let mut trace = BrowserNavigationTrace::default();
        let now = Instant::now();
        for sequence in 1..=100 {
            trace.record_attempt(&requested(sequence, BrowserNavigationSource::PageNavigation, "https://example.test/"), now);
        }
        assert_eq!(trace.recent_page_attempts.len(), MAX_NAVIGATION_ATTEMPTS);
        assert_eq!(trace.control_at(false, now).repeated_navigation_count, MAX_NAVIGATION_ATTEMPTS as u32);
        assert!(!trace.control_at(false, now + REPEAT_WINDOW + Duration::from_millis(1)).loop_suspected);
        let mut human = requested(101, BrowserNavigationSource::PageNavigation, "https://example.test/");
        human.action = Some(BrowserNavigationActionSummary { main_frame: true, user_initiated: Some(true), ..Default::default() });
        trace.record_attempt(&human, now);
        assert_eq!(trace.control_at(false, now).repeated_navigation_count, 0);
        trace.record_attempt(&requested(102, BrowserNavigationSource::PageNavigation, "https://example.test/"), now);
        trace.record_attempt(&requested(103, BrowserNavigationSource::UserCommand, "https://example.test/"), now);
        assert_eq!(trace.control_at(false, now).repeated_navigation_count, 0);
    }

    #[test]
    fn subframe_request_and_unassociated_action_do_not_create_main_attempts() {
        let mut trace = BrowserNavigationTrace::default();
        let mut subframe = requested(1, BrowserNavigationSource::PageNavigation, "https://example.test/");
        subframe.action = Some(BrowserNavigationActionSummary { main_frame: false, ..Default::default() });
        trace.push(subframe);
        let mut action = event(None);
        action.kind = BrowserNavigationEventKind::Action;
        action.action = Some(BrowserNavigationActionSummary { main_frame: true, source_origin: Some("https://user:password@example.test/private?q=secret#token".into()), ..Default::default() });
        trace.push(action);
        let snapshot = trace.snapshot();
        assert_eq!(snapshot.counters, BrowserNavigationCounters::default());
        assert_eq!(snapshot.events[1].action.as_ref().unwrap().source_origin.as_deref(), Some("https://example.test"));
        assert_eq!(snapshot.events[1].document_generation, None);
        assert!(!trace.control(false).loop_suspected);
        let text = serde_json::to_string(&snapshot).unwrap();
        for secret in ["password", "secret", "private", "token"] { assert!(!text.contains(secret)); }
    }

    #[test]
    fn optional_control_preserves_existing_load_wire_and_methods_never_keep_untrusted_text() {
        let mut value = serde_json::to_value(BrowserLoadSummary::default()).unwrap();
        value.as_object_mut().unwrap().remove("navigation_control");
        assert_eq!(serde_json::from_value::<BrowserLoadSummary>(value).unwrap().navigation_control, None);
        assert_eq!(BrowserNavigationHttpMethod::from_method("post"), BrowserNavigationHttpMethod::Post);
        assert_eq!(BrowserNavigationHttpMethod::from_method("SECRET-METHOD"), BrowserNavigationHttpMethod::Other);
        assert_eq!(BrowserNavigationHttpMethod::from_method(""), BrowserNavigationHttpMethod::Unknown);
    }
}
