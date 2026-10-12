//! Bounded, read-only native navigation facts. No page bodies or execution handles.
use crate::{runtime::BrowserTabTarget, url_projection::project_metadata_url};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Instant};

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
pub enum BrowserNativeDomainClass { Url, WebKit, Other }

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
    Requested, Started, Redirect, Response, Committed, Finished, Cancelled, Failed, Crashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavigationSource { UserCommand, AgentCommand, PageNavigation, Unknown }

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserNavigationEvent {
    pub sequence: u64,
    pub elapsed_ms: u64,
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
}

impl BrowserNavigationEvent {
    pub fn new(
        kind: BrowserNavigationEventKind,
        navigation_sequence: Option<u64>,
        document_generation: Option<u64>,
        source: BrowserNavigationSource,
        url: Option<String>,
    ) -> Self {
        Self { sequence: 0, elapsed_ms: 0, navigation_sequence, document_generation,
            kind, source, url, response_status: None, problem: None, cancellation_reason: None }
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
}

pub const MAX_NAVIGATION_ATTEMPTS: usize = 32;
pub const MAX_NAVIGATION_EVENTS: usize = 128;
pub const MAX_NAVIGATION_TRACE_BYTES: usize = 128 * 1024;
const MAX_METADATA_CHARS: usize = 512;

/// Owned by one native Page, retained across its navigations and destroyed
/// with that Page. It is neither a persistent history nor an authorization ID.
#[derive(Debug)]
pub struct BrowserNavigationTrace {
    started: Instant,
    next_sequence: u64,
    events: VecDeque<(BrowserNavigationEvent, usize)>,
    retained_bytes: usize,
    dropped: u64,
    routing_lost: bool,
}

impl Default for BrowserNavigationTrace {
    fn default() -> Self {
        Self { started: Instant::now(), next_sequence: 0, events: VecDeque::new(),
            retained_bytes: 0, dropped: 0, routing_lost: false }
    }
}

impl BrowserNavigationTrace {
    pub fn push(&mut self, mut event: BrowserNavigationEvent) {
        self.next_sequence = self.next_sequence.saturating_add(1);
        event.sequence = self.next_sequence;
        event.elapsed_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        event.url = event.url.as_deref().map(|url| clip(&project_metadata_url(url)));
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

    pub fn snapshot(&self) -> BrowserNavigationTraceSnapshot {
        BrowserNavigationTraceSnapshot {
            scope: BrowserNavigationScope::MainDocumentNavigation,
            availability: if self.routing_lost { BrowserNavigationAvailability::Degraded }
                else { BrowserNavigationAvailability::Supported },
            events: self.events.iter().map(|(event, _)| event.clone()).collect(),
            dropped: self.dropped,
            routing_lost: self.routing_lost,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserRuntimeInfo {
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webkit_framework_version: Option<String>,
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
                events: vec![event(None)], dropped: 0, routing_lost: false,
            },
            identity: None,
            runtime: Some(BrowserRuntimeInfo {
                os_version: "界".repeat(2000), os_build: None, webkit_framework_version: None,
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
}
