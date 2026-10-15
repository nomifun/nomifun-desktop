//! One UI-thread owner for WebView2 main-document attempts. Native IDs, not
//! matching URLs or the most recent completion, decide which load may change.
use nomifun_browser_platform::{
    navigation::*,
    revision::BrowserRevision,
    runtime::{BrowserTabLifecycle, BrowserTabSnapshot, BrowserTabTarget, WorkspaceError},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::{Arc, Mutex},
};
use tauri::Manager;
use webview2_com::{
    ContentLoadingEventHandler, Microsoft::Web::WebView2::Win32::*,
    NavigationCompletedEventHandler, NavigationStartingEventHandler, SourceChangedEventHandler,
};
use windows::core::{BOOL, Interface, PWSTR};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub sequence: u64,
    native_id: Option<u64>,
    submission: u64,
    generation: u64,
}

#[derive(Clone)]
struct Attempt {
    sequence: u64,
    generation: u64,
    source: BrowserNavigationSource,
    committed: bool,
    url: String,
    cancellation: Option<BrowserCancellationReason>,
}

struct State {
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<BrowserRevision>,
    trace: BrowserNavigationTrace,
    attempts: BTreeMap<u64, Attempt>,
    current: Option<u64>,
    terminal: Option<u64>,
    sequence: u64,
    submission: u64,
    active_dispatches: u32,
    // WebView2 does not return NavigationId from Navigate/CDP. Keep the
    // dispatch and native attempt separate until the dispatch interval ends.
    host_interval: bool,
    // The current dispatch kind and origin uncertainty have different lifetimes:
    // a subsequent same-document command can replace the kind without proving
    // that an older history callback can no longer arrive.
    history_dispatch_unresolved: bool,
    history_origin_unresolved: bool,
    bootstrap: bool,
    paused: bool,
    pause_pending: bool,
    gate_supported: bool,
}

impl State {
    fn new(
        metadata: Arc<Mutex<BrowserTabSnapshot>>,
        revision: Arc<BrowserRevision>,
        bootstrap: bool,
    ) -> Self {
        metadata.lock().unwrap_or_else(|e| e.into_inner()).load =
            Some(BrowserLoadSummary::default());
        Self {
            metadata,
            revision,
            trace: Default::default(),
            attempts: Default::default(),
            current: None,
            terminal: None,
            sequence: 0,
            submission: 0,
            active_dispatches: 0,
            host_interval: false,
            history_dispatch_unresolved: false,
            history_origin_unresolved: false,
            bootstrap,
            paused: false,
            pause_pending: false,
            gate_supported: super::navigation_request_gate::supported(),
        }
    }
    fn receipt(&self) -> Receipt {
        Receipt {
            sequence: self.sequence,
            native_id: self.current,
            submission: self.submission,
            generation: self
                .metadata
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .target
                .document_generation,
        }
    }
    fn matches(&self, receipt: Receipt) -> bool {
        self.receipt() == receipt
    }
    fn requested_matches(&self, receipt: Receipt) -> bool {
        !self.history_dispatch_unresolved
            && self.matches(receipt)
            && self
                .metadata
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .load
                .as_ref()
                .is_some_and(|load| load.phase == BrowserNavigationPhase::Requested)
    }
    fn before_unload_scope(&self, receipt: Receipt) -> bool {
        self.matches(receipt)
            && ((self.history_dispatch_unresolved && self.active_dispatches > 0)
                || (self.current.is_none() && self.requested_matches(receipt)))
    }
    fn before_unload_stopped_matches(&self, receipt: Receipt) -> bool {
        self.matches(receipt)
            && ((self.history_dispatch_unresolved && self.active_dispatches > 0)
                || (self.current.is_none()
                    && self.metadata.lock().unwrap_or_else(|e| e.into_inner()).load.as_ref()
                        .is_some_and(|load| load.phase == BrowserNavigationPhase::Cancelled
                            && load.cancellation_reason == Some(BrowserCancellationReason::NavigationRejected))))
    }
    fn reject_before_unload(&mut self, receipt: Receipt) -> bool {
        if !self.before_unload_scope(receipt) {
            return false;
        }
        // The existing command registry separately owns the Stop and explicit
        // false deferral acknowledgements. Rejecting history does not cancel
        // the retained document's already established load facts.
        if self.history_dispatch_unresolved { return true; }
        let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        let load = tab.load.as_mut().unwrap();
        load.phase = BrowserNavigationPhase::Cancelled;
        load.cancellation_reason = Some(BrowserCancellationReason::NavigationRejected);
        load.problem = None;
        load.estimated_progress = None;
        tab.lifecycle = BrowserTabLifecycle::Stopped;
        true
    }
    fn settled_rejection(&mut self, receipt: Receipt, source: BrowserNavigationSource) {
        self.active_dispatches = self.active_dispatches.saturating_sub(1);
        if self.active_dispatches == 0 && receipt.submission == self.submission
            && !self.history_origin_unresolved {
            self.host_interval = false;
        }
        let mut event = BrowserNavigationEvent::new(
            BrowserNavigationEventKind::Cancelled,
            None,
            None,
            source,
            None,
        );
        event.dispatch_sequence = Some(receipt.submission);
        event.cancellation_reason = Some(BrowserCancellationReason::NavigationRejected);
        self.push(event);
    }
    fn receipt_for_target(
        &self,
        target: &BrowserTabTarget,
        expected_sequence: Option<u64>,
    ) -> Result<Receipt, WorkspaceError> {
        if self
            .metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .target
            != *target
            || expected_sequence.is_some_and(|sequence| sequence != self.sequence)
        {
            return Err(WorkspaceError::StaleTarget);
        }
        Ok(self.receipt())
    }
    fn committed_receipt(&self) -> Option<Receipt> {
        let tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        let committed = self
            .current
            .and_then(|id| self.attempts.get(&id))
            .is_some_and(|attempt| attempt.committed)
            && tab.load.as_ref().is_some_and(|load| {
                load.navigation_sequence == self.sequence
                    && load.content_state == BrowserContentState::CurrentDocument
                    && matches!(
                        load.phase,
                        BrowserNavigationPhase::Committed | BrowserNavigationPhase::Finished
                    )
            });
        drop(tab);
        committed.then(|| self.receipt())
    }
    fn control(&mut self) {
        let control = self.trace.control(self.paused);
        if let Some(load) = &mut self.metadata.lock().unwrap_or_else(|e| e.into_inner()).load {
            load.navigation_control = self.gate_supported.then_some(control);
        }
    }
    fn popup_permitted(&self, target: &BrowserTabTarget) -> bool {
        !self.paused
            && !self.pause_pending
            && self.metadata.lock().unwrap_or_else(|e| e.into_inner()).target == *target
    }
    fn popup_requested(&mut self, url: Option<String>, user_initiated: bool) -> bool {
        if !self.paused && !self.pause_pending {
            return true;
        }
        let mut event = BrowserNavigationEvent::new(
            BrowserNavigationEventKind::Action,
            None,
            None,
            BrowserNavigationSource::PageNavigation,
            url,
        );
        // The callback identifies this native view, but supplies no NavigationId
        // or document identity. Keep it unassociated instead of attaching the
        // owner's latest generation. A gesture cannot unlock the page hold.
        event.action = Some(BrowserNavigationActionSummary {
            main_frame: false,
            policy: BrowserNavigationPolicy::Paused,
            user_initiated: Some(user_initiated),
            ..Default::default()
        });
        event.cancellation_reason = Some(BrowserCancellationReason::AutomaticNavigationPaused);
        self.push(event);
        false
    }
    fn push(&mut self, event: BrowserNavigationEvent) {
        self.trace.push(event);
        self.control();
        self.revision.bump();
    }
    fn requested(
        &mut self,
        source: BrowserNavigationSource,
        url: Option<String>,
        action: BrowserNavigationActionType,
    ) -> Receipt {
        self.bootstrap = false;
        self.paused = false;
        self.pause_pending = false;
        self.trace.clear_repeat_window();
        self.submission = self.submission.saturating_add(1);
        // This observable dispatch fence also revokes controls issued for A
        // while B is queued before its first native NavigationStarting event.
        // It is not a guessed association with a later native NavigationId.
        self.sequence = self.sequence.saturating_add(1);
        self.history_dispatch_unresolved = action == BrowserNavigationActionType::BackForward;
        self.history_origin_unresolved |= self.history_dispatch_unresolved;
        if !self.history_dispatch_unresolved {
            self.current = None;
            self.terminal = None;
        }
        self.active_dispatches = self.active_dispatches.saturating_add(1);
        self.host_interval = true;
        {
            let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            let load = tab.load.get_or_insert_with(Default::default);
            // This is also a control fence. Advancing it for history rejects
            // old wire controls; it does not create another native attempt.
            load.navigation_sequence = self.sequence;
            if !self.history_dispatch_unresolved {
                load.phase = BrowserNavigationPhase::Requested;
                load.requested_url = url.clone();
                load.problem = None;
                load.cancellation_reason = None;
                load.response_status = None;
                load.estimated_progress = Some(0);
                if load.content_state != BrowserContentState::None {
                    load.content_state = BrowserContentState::RetainedDocument;
                }
                tab.lifecycle = BrowserTabLifecycle::Loading;
            }
        }
        // No native NavigationId is available at dispatch. Do not invent an
        // association based on a later same-URL NavigationStarting callback.
        let mut event = BrowserNavigationEvent::new(
            BrowserNavigationEventKind::Requested,
            None,
            None,
            source,
            url,
        );
        event.dispatch_sequence = Some(self.submission);
        event.action = Some(action_summary(action, BrowserNavigationPolicy::Allow, None));
        self.push(event);
        self.receipt()
    }
    fn settled(
        &mut self,
        receipt: Receipt,
        source: BrowserNavigationSource,
        failed: bool,
        download: bool,
        same_document: bool,
    ) {
        self.settled_problem(receipt, source, failed, download, same_document, None);
    }
    fn settled_problem(
        &mut self,
        receipt: Receipt,
        source: BrowserNavigationSource,
        failed: bool,
        download: bool,
        same_document: bool,
        failure: Option<BrowserPageProblem>,
    ) {
        self.active_dispatches = self.active_dispatches.saturating_sub(1);
        let fenced_terminal = self.current.is_none()
            && self
                .metadata
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .load
                .as_ref()
                .is_some_and(|load| {
                    matches!(
                        load.phase,
                        BrowserNavigationPhase::Cancelled
                            | BrowserNavigationPhase::Failed
                            | BrowserNavigationPhase::Crashed
                            | BrowserNavigationPhase::Finished
                    )
                });
        if self.active_dispatches == 0 && !self.history_origin_unresolved
            && ((self.current.is_some() && self.terminal == self.current) || fenced_terminal)
        {
            self.host_interval = false;
        }
        if same_document && self.requested_matches(receipt) {
            let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(load) = &mut tab.load {
                load.phase = BrowserNavigationPhase::Finished;
                load.content_state = BrowserContentState::CurrentDocument;
                load.estimated_progress = Some(100);
                load.cancellation_reason = None;
                load.problem = None;
            }
            tab.lifecycle = BrowserTabLifecycle::Ready;
            drop(tab);
            if !self.history_origin_unresolved { self.host_interval = false; }
            let mut event = BrowserNavigationEvent::new(
                BrowserNavigationEventKind::Finished,
                None,
                None,
                source,
                None,
            );
            event.dispatch_sequence = Some(receipt.submission);
            self.push(event);
        }
        if !failed && !download {
            return;
        }
        let mut event = BrowserNavigationEvent::new(
            if download {
                BrowserNavigationEventKind::Cancelled
            } else {
                BrowserNavigationEventKind::Failed
            },
            None,
            None,
            source,
            None,
        );
        event.dispatch_sequence = Some(receipt.submission);
        if failed {
            event.problem = Some(failure.unwrap_or_else(|| protocol_problem(None)));
        }
        if download {
            event.cancellation_reason = Some(BrowserCancellationReason::Download);
        }
        // A CDP result has no native ID. It can change a pending request only
        // if no native attempt or newer host submission has intervened.
        if self.requested_matches(receipt) {
            let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            let load = tab.load.as_mut().unwrap();
            load.phase = if download {
                BrowserNavigationPhase::Cancelled
            } else {
                BrowserNavigationPhase::Failed
            };
            load.cancellation_reason = download.then_some(BrowserCancellationReason::Download);
            if failed {
                load.problem = event.problem.clone();
            }
            tab.lifecycle = if download {
                BrowserTabLifecycle::Stopped
            } else {
                BrowserTabLifecycle::Failed
            };
            if !self.history_origin_unresolved { self.host_interval = false; }
        }
        self.push(event);
    }
    fn started(
        &mut self,
        id: u64,
        url: String,
        redirect: bool,
        action: BrowserNavigationActionSummary,
    ) -> bool {
        if self.bootstrap && url == "about:blank" {
            return false;
        }
        if self.paused || self.pause_pending {
            let mut event = BrowserNavigationEvent::new(
                BrowserNavigationEventKind::Action,
                None,
                None,
                if self.host_interval {
                    BrowserNavigationSource::Unknown
                } else {
                    BrowserNavigationSource::PageNavigation
                },
                Some(url),
            );
            let mut action = action;
            action.policy = BrowserNavigationPolicy::Paused;
            event.action = Some(action);
            event.cancellation_reason = Some(BrowserCancellationReason::AutomaticNavigationPaused);
            self.push(event);
            return false;
        }
        if redirect {
            if let Some(attempt) = self.attempts.get_mut(&id) {
                attempt.url = url.clone();
                let attempt = attempt.clone();
                let mut event =
                    attempt.event(BrowserNavigationEventKind::Redirect, Some(url.clone()));
                event.action = Some(action);
                if self.current == Some(id) {
                    self.metadata.lock().unwrap_or_else(|e| e.into_inner()).url = url;
                }
                self.push(event);
            } else {
                self.trace.mark_routing_lost();
                self.control();
                self.revision.bump();
            }
            return true;
        }
        if self.attempts.contains_key(&id) {
            return true;
        }
        self.bootstrap = false;
        self.terminal = None;
        self.sequence = self.sequence.saturating_add(1);
        // A native event during a host dispatch interval cannot be assigned to
        // that command. Outside that interval, this owner has issued no host
        // navigation and the request is demonstrably page-originated.
        let source = if self.host_interval {
            BrowserNavigationSource::Unknown
        } else {
            BrowserNavigationSource::PageNavigation
        };
        if self.host_interval {
            self.trace.mark_routing_lost();
        }
        self.history_dispatch_unresolved = false;
        self.history_origin_unresolved = false;
        let generation = {
            let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            tab.url = url.clone();
            tab.lifecycle = BrowserTabLifecycle::Loading;
            let load = tab.load.as_mut().unwrap();
            load.navigation_sequence = self.sequence;
            load.phase = BrowserNavigationPhase::Provisional;
            load.requested_url = Some(url.clone());
            load.problem = None;
            load.cancellation_reason = None;
            load.response_status = None;
            load.estimated_progress = Some(0);
            if load.content_state != BrowserContentState::None {
                load.content_state = BrowserContentState::RetainedDocument;
            }
            tab.target.document_generation
        };
        let attempt = Attempt {
            sequence: self.sequence,
            generation,
            source,
            committed: false,
            url: url.clone(),
            cancellation: None,
        };
        self.current = Some(id);
        self.attempts.insert(id, attempt.clone());
        while self.attempts.len() > MAX_NAVIGATION_ATTEMPTS {
            if let Some(old) = self
                .attempts
                .keys()
                .find(|id| Some(**id) != self.current)
                .copied()
            {
                self.attempts.remove(&old);
            } else {
                break;
            }
        }
        let mut request = attempt.event(BrowserNavigationEventKind::Requested, Some(url.clone()));
        request.action = Some(action.clone());
        self.push(request);
        let mut event = attempt.event(BrowserNavigationEventKind::Started, Some(url));
        event.action = Some(action);
        self.push(event);
        true
    }
    fn rejected(&mut self, id: u64, url: String, mut action: BrowserNavigationActionSummary) {
        action.policy = BrowserNavigationPolicy::Cancel;
        let mut event = BrowserNavigationEvent::new(
            BrowserNavigationEventKind::Action,
            None,
            None,
            BrowserNavigationSource::Unknown,
            Some(url),
        );
        event.action = Some(action);
        event.cancellation_reason = Some(BrowserCancellationReason::NavigationRejected);
        if let Some(attempt) = self.attempts.get_mut(&id) {
            attempt.cancellation = Some(BrowserCancellationReason::NavigationRejected);
            event.navigation_sequence = Some(attempt.sequence);
            event.document_generation = Some(attempt.generation);
            event.source = attempt.source;
            if self.current == Some(id) {
                let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
                let load = tab.load.as_mut().unwrap();
                load.phase = BrowserNavigationPhase::Cancelled;
                load.cancellation_reason = Some(BrowserCancellationReason::NavigationRejected);
                load.problem = None;
                tab.lifecycle = BrowserTabLifecycle::Stopped;
            }
        }
        self.push(event);
    }
    fn committed(&mut self, id: u64, url: String, error_page: bool) -> bool {
        let Some(attempt) = self.attempts.get_mut(&id) else {
            return false;
        };
        if attempt.committed {
            return false;
        }
        attempt.committed = true;
        attempt.url = url.clone();
        if self.current == Some(id) {
            let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            tab.target.document_generation = tab.target.document_generation.saturating_add(1);
            attempt.generation = tab.target.document_generation;
            tab.blocked_permissions.clear();
            tab.diagnostics.clear_page();
            let load = tab.load.as_mut().unwrap();
            // A real ContentLoading still advances the document identity when
            // it arrives after Stop/Pause acknowledgement. It does not revoke
            // that exact attempt's acknowledged cancellation.
            let cancelled_by_host = attempt.cancellation.is_some()
                && load.phase == BrowserNavigationPhase::Cancelled
                && load.cancellation_reason == attempt.cancellation;
            if !cancelled_by_host {
                load.phase = BrowserNavigationPhase::Committed;
            }
            if !error_page {
                load.content_state = BrowserContentState::CurrentDocument;
                load.content_url = Some(url.clone());
            } else {
                load.content_state = BrowserContentState::None;
                load.content_url = None;
            }
            tab.url = url.clone();
            tab.lifecycle = if cancelled_by_host {
                BrowserTabLifecycle::Stopped
            } else {
                BrowserTabLifecycle::Loading
            };
        }
        let attempt = attempt.clone();
        self.push(attempt.event(BrowserNavigationEventKind::Committed, Some(url)));
        self.current == Some(id)
    }
    fn completed(
        &mut self,
        id: u64,
        url: String,
        success: bool,
        status: COREWEBVIEW2_WEB_ERROR_STATUS,
        http: Option<u16>,
    ) -> bool {
        let Some(attempt) = self.attempts.get(&id).cloned() else {
            return false;
        };
        let url = if self.current == Some(id) {
            url
        } else {
            attempt.url.clone()
        };
        let cancelled = !success && status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED;
        let reason = attempt.cancellation.unwrap_or(if self.current != Some(id) {
            BrowserCancellationReason::Replaced
        } else {
            BrowserCancellationReason::Unknown
        });
        let kind = if success {
            BrowserNavigationEventKind::Finished
        } else if cancelled {
            BrowserNavigationEventKind::Cancelled
        } else {
            BrowserNavigationEventKind::Failed
        };
        if let Some(http) = http {
            // replaceState may already have changed Core.Source without a
            // request. HTTP evidence belongs to the native attempt address.
            let mut event = attempt.event(
                BrowserNavigationEventKind::Response,
                Some(attempt.url.clone()),
            );
            event.response_status = Some(http);
            self.push(event);
        }
        let mut event = attempt.event(kind, Some(url.clone()));
        if cancelled {
            event.cancellation_reason = Some(reason);
        } else if !success {
            event.problem = Some(problem(status, attempt.committed));
        }
        let (current, cancelled_by_host) = {
            let tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            let current = self.current == Some(id)
                && tab
                    .load
                    .as_ref()
                    .is_some_and(|load| load.phase != BrowserNavigationPhase::Requested);
            // Stop/Pause can abort Fetch before WebView2 reports completion.
            // The SDK may then report CONNECTION_ABORTED rather than
            // OPERATION_CANCELED. Keep this exact attempt's confirmed cancel
            // from Stop/Pause acknowledgement or a native cancellation event;
            // retain the SDK's independent outcome in trace. An intention set
            // before Core.Stop alone is not a positive cancellation fact.
            let cancelled_by_host = current
                && attempt.cancellation.is_some()
                && tab.load.as_ref().is_some_and(|load| {
                    load.phase == BrowserNavigationPhase::Cancelled
                        && load.cancellation_reason == attempt.cancellation
                });
            (current, cancelled_by_host)
        };
        if current {
            self.terminal = Some(id);
            if self.active_dispatches == 0 && !self.history_origin_unresolved {
                self.host_interval = false;
            }
        }
        if current && !cancelled_by_host {
            let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
            let load = tab.load.as_mut().unwrap();
            load.response_status = http;
            load.phase = if success {
                BrowserNavigationPhase::Finished
            } else if cancelled {
                BrowserNavigationPhase::Cancelled
            } else {
                BrowserNavigationPhase::Failed
            };
            load.problem = event.problem.clone();
            load.cancellation_reason = event.cancellation_reason;
            if success {
                load.content_state = BrowserContentState::CurrentDocument;
                load.content_url = Some(url.clone());
                load.estimated_progress = Some(100);
                tab.url = url;
            }
            tab.lifecycle = if success {
                BrowserTabLifecycle::Ready
            } else if cancelled {
                BrowserTabLifecycle::Stopped
            } else {
                BrowserTabLifecycle::Failed
            };
        }
        self.push(event);
        current && !cancelled_by_host
    }
    fn same_document(&mut self, url: String) {
        let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        tab.url = url.clone();
        if let Some(load) = &mut tab.load {
            if load.content_state != BrowserContentState::None {
                load.content_url = Some(url);
            }
        }
        drop(tab);
        self.revision.bump();
    }
    fn stop(&mut self, receipt: Receipt) -> Result<(), String> {
        self.stop_for(receipt, BrowserCancellationReason::UserStop)
    }
    fn cancellation_intent(
        &mut self,
        receipt: Receipt,
        reason: BrowserCancellationReason,
    ) -> Result<(), String> {
        if !self.matches(receipt) {
            return Err("Browser navigation is no longer current.".into());
        }
        // A second control on a cancelled attempt must not replace the reason
        // that already proves its terminal state (for example Pause -> Stop).
        let active = self
            .metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .load
            .as_ref()
            .is_some_and(|load| {
                matches!(
                    load.phase,
                    BrowserNavigationPhase::Requested
                        | BrowserNavigationPhase::Provisional
                        | BrowserNavigationPhase::Committed
                )
            });
        if active {
            if let Some(attempt) = self.current.and_then(|id| self.attempts.get_mut(&id)) {
                attempt.cancellation = Some(reason);
            }
        }
        Ok(())
    }
    fn stop_for(
        &mut self,
        receipt: Receipt,
        reason: BrowserCancellationReason,
    ) -> Result<(), String> {
        if !self.matches(receipt) {
            return Err("Browser navigation was replaced before Stop.".into());
        }
        if self
            .metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .load
            .as_ref()
            .is_some_and(|load| {
                !matches!(
                    load.phase,
                    BrowserNavigationPhase::Requested
                        | BrowserNavigationPhase::Provisional
                        | BrowserNavigationPhase::Committed
                )
            })
        {
            return Ok(());
        }
        if let Some(attempt) = self.current.and_then(|id| self.attempts.get_mut(&id)) {
            attempt.cancellation = Some(reason);
        }
        let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        let load = tab.load.as_mut().unwrap();
        load.phase = BrowserNavigationPhase::Cancelled;
        load.cancellation_reason = Some(reason);
        load.problem = None;
        tab.lifecycle = BrowserTabLifecycle::Stopped;
        drop(tab);
        // A history control fence may be newer than this retained native
        // attempt. Only its actual NavigationId gives event association.
        let native_attempt = receipt.native_id.and_then(|id| self.attempts.get(&id));
        let mut event = BrowserNavigationEvent::new(
            BrowserNavigationEventKind::Cancelled,
            native_attempt.map(|attempt| attempt.sequence),
            native_attempt.map(|attempt| attempt.generation),
            BrowserNavigationSource::UserCommand,
            None,
        );
        event.cancellation_reason = Some(reason);
        self.push(event);
        Ok(())
    }
    fn crashed(&mut self) {
        let native_sequence = self.current.and_then(|id| self.attempts.get(&id)).map(|attempt| attempt.sequence);
        self.current = None;
        self.terminal = None;
        self.host_interval = false;
        self.history_dispatch_unresolved = false;
        self.history_origin_unresolved = false;
        self.active_dispatches = 0;
        self.bootstrap = false;
        let mut tab = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        tab.target.document_generation = tab.target.document_generation.saturating_add(1);
        let generation = tab.target.document_generation;
        tab.lifecycle = BrowserTabLifecycle::Crashed;
        tab.blocked_permissions.clear();
        tab.script_dialog = None;
        tab.diagnostics.clear_page();
        let load = tab.load.as_mut().unwrap();
        load.phase = BrowserNavigationPhase::Crashed;
        load.content_state = BrowserContentState::None;
        load.content_url = None;
        load.problem = None;
        load.response_status = None;
        load.cancellation_reason = None;
        drop(tab);
        self.push(BrowserNavigationEvent::new(
            BrowserNavigationEventKind::Crashed,
            native_sequence,
            Some(generation),
            BrowserNavigationSource::Unknown,
            None,
        ));
    }
}
impl Attempt {
    fn event(
        &self,
        kind: BrowserNavigationEventKind,
        url: Option<String>,
    ) -> BrowserNavigationEvent {
        BrowserNavigationEvent::new(
            kind,
            Some(self.sequence),
            Some(self.generation),
            self.source,
            url,
        )
    }
}

fn action_summary(
    kind: BrowserNavigationActionType,
    policy: BrowserNavigationPolicy,
    user: Option<bool>,
) -> BrowserNavigationActionSummary {
    BrowserNavigationActionSummary {
        navigation_type: kind,
        main_frame: true,
        source_main_frame: None,
        http_method: BrowserNavigationHttpMethod::Unknown,
        policy,
        source_origin: None,
        user_initiated: user,
    }
}
fn problem(status: COREWEBVIEW2_WEB_ERROR_STATUS, committed: bool) -> BrowserPageProblem {
    let reason = match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => BrowserFailureReason::Dns,
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => BrowserFailureReason::Timeout,
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => {
            BrowserFailureReason::Tls
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET
        | COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED
        | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => {
            BrowserFailureReason::NetworkConnection
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => {
            BrowserFailureReason::AccessDenied
        }
        _ => BrowserFailureReason::Other,
    };
    BrowserPageProblem {
        stage: if committed {
            BrowserFailureStage::Committed
        } else {
            BrowserFailureStage::Provisional
        },
        safe_reason: reason,
        native_domain_class: BrowserNativeDomainClass::WebView2,
        native_code: status.0 as i64,
    }
}
fn protocol_problem(error: Option<&str>) -> BrowserPageProblem {
    let safe_reason = match error {
        Some("net::ERR_NAME_NOT_RESOLVED") => BrowserFailureReason::Dns,
        Some("net::ERR_TIMED_OUT" | "net::ERR_CONNECTION_TIMED_OUT") => {
            BrowserFailureReason::Timeout
        }
        Some(
            "net::ERR_CONNECTION_REFUSED"
            | "net::ERR_CONNECTION_RESET"
            | "net::ERR_CONNECTION_CLOSED"
            | "net::ERR_CONNECTION_ABORTED"
            | "net::ERR_INTERNET_DISCONNECTED",
        ) => BrowserFailureReason::NetworkConnection,
        Some(error)
            if error.starts_with("net::ERR_CERT_") || error == "net::ERR_SSL_PROTOCOL_ERROR" =>
        {
            BrowserFailureReason::Tls
        }
        _ => BrowserFailureReason::Other,
    };
    BrowserPageProblem {
        stage: BrowserFailureStage::Provisional,
        safe_reason,
        native_domain_class: BrowserNativeDomainClass::Other,
        native_code: 0,
    }
}

struct Registration {
    core: ICoreWebView2,
    state: Rc<RefCell<State>>,
    starting: i64,
    content: i64,
    completed: i64,
    source: i64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            let _ = self.core.remove_NavigationStarting(self.starting);
            let _ = self.core.remove_ContentLoading(self.content);
            let _ = self.core.remove_NavigationCompleted(self.completed);
            let _ = self.core.remove_SourceChanged(self.source);
        }
    }
}
thread_local! {static OWNERS:RefCell<HashMap<String,Registration>>=RefCell::default();}
pub(super) fn close_view(label: &str) {
    drop(OWNERS.with(|owners| owners.borrow_mut().remove(label)));
}
pub(crate) async fn gate_unavailable(view: &tauri::Webview) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = OWNERS.with(|owners| {
            let owners = owners.borrow();
            let owner = owners
                .get(&label)
                .ok_or_else(|| "Native navigation owner is unavailable.".to_owned())?;
            let mut state = owner.state.borrow_mut();
            state.gate_supported = false;
            state.control();
            state.revision.bump();
            Ok(())
        });
        let _ = tx.send(result);
    })
    .map_err(|_| "Native navigation capability update is unavailable.".to_owned())?;
    rx.await
        .map_err(|_| "Native navigation capability update acknowledgement was lost.".to_owned())?
}
pub(super) fn crashed(label: &str) -> bool {
    let changed = OWNERS.with(|owners| {
        let owners = owners.borrow();
        if let Some(owner) = owners.get(label) {
            owner.state.borrow_mut().crashed();
            true
        } else {
            false
        }
    });
    if changed {
        let _ = super::presentation::refresh(label);
    }
    changed
}
pub(super) fn receipt(label: &str) -> Result<Receipt, String> {
    OWNERS.with(|owners| {
        owners
            .borrow()
            .get(label)
            .map(|owner| owner.state.borrow().receipt())
            .ok_or_else(|| "Native navigation owner is unavailable.".into())
    })
}
pub(super) fn requested_receipt(label: &str) -> Option<Receipt> {
    OWNERS.with(|owners| {
        let owners = owners.borrow();
        let state = owners.get(label)?.state.borrow();
        let receipt = state.receipt();
        state.before_unload_scope(receipt).then_some(receipt)
    })
}
/// Called only on the native UI thread, before a popup can bind its WindowProxy.
pub(crate) fn popup_permitted(label: &str, target: &BrowserTabTarget) -> bool {
    OWNERS.with(|owners| owners.borrow().get(label)
        .is_some_and(|owner| owner.state.borrow().popup_permitted(target)))
}
/// None denotes a transport-only view with no navigation owner. The production
/// host's separate exact admission rejects a missing owner rather than guessing.
pub(super) fn popup_requested(label: &str, url: Option<String>, user: bool) -> Option<bool> {
    OWNERS.with(|owners| owners.borrow().get(label)
        .map(|owner| owner.state.borrow_mut().popup_requested(url, user)))
}
pub(super) fn stop_requested_before_unload(label: &str, receipt: Receipt) -> Result<bool, String> {
    let owner = OWNERS
        .with(|owners| {
            let owners = owners.borrow();
            owners
                .get(label)
                .map(|owner| (owner.core.clone(), owner.state.clone()))
        })
        .ok_or_else(|| "Native navigation owner is unavailable.".to_owned())?;
    if requested_receipt(label) != Some(receipt) {
        return Ok(false);
    }
    // No RefCell borrow crosses COM. Reentrant callbacks may revoke the scope.
    unsafe { owner.0.Stop() }.map_err(|_| "Native browser Stop failed.".to_owned())?;
    if requested_receipt(label) != Some(receipt) {
        return Ok(false);
    }
    let mut state = owner.1.borrow_mut();
    let rejected = state.reject_before_unload(receipt);
    if rejected {
        state.control();
        state.revision.bump();
    }
    drop(state);
    if rejected {
        let _ = super::presentation::refresh(label);
    }
    Ok(rejected)
}
pub(super) fn before_unload_stopped_matches(label: &str, receipt: Receipt) -> bool {
    OWNERS.with(|owners| {
        let owners = owners.borrow();
        let Some(owner) = owners.get(label) else {
            return false;
        };
        let state = owner.state.borrow();
        state.before_unload_stopped_matches(receipt)
    })
}
pub(super) fn settled_rejection(label: &str, receipt: Receipt, source: BrowserNavigationSource) {
    OWNERS.with(|owners| {
        if let Some(owner) = owners.borrow().get(label) {
            owner.state.borrow_mut().settled_rejection(receipt, source);
        }
    });
    let _ = super::presentation::refresh(label);
}
pub(super) fn settled_protocol_failure(
    label: &str,
    receipt: Receipt,
    source: BrowserNavigationSource,
    error: Option<&str>,
) {
    OWNERS.with(|owners| {
        if let Some(owner) = owners.borrow().get(label) {
            owner.state.borrow_mut().settled_problem(
                receipt,
                source,
                true,
                false,
                false,
                Some(protocol_problem(error)),
            );
        }
    });
    let _ = super::presentation::refresh(label);
}
pub(super) fn prepare(
    label: &str,
    source: BrowserNavigationSource,
    url: Option<String>,
    action: BrowserNavigationActionType,
) -> Result<Receipt, String> {
    let result = OWNERS.with(|owners| {
        owners
            .borrow()
            .get(label)
            .map(|owner| owner.state.borrow_mut().requested(source, url, action))
            .ok_or_else(|| "Native navigation owner is unavailable.".into())
    });
    let _ = super::presentation::refresh(label);
    result
}
pub(super) fn settled(
    label: &str,
    receipt: Receipt,
    source: BrowserNavigationSource,
    failed: bool,
    download: bool,
    same_document: bool,
) {
    OWNERS.with(|owners| {
        if let Some(owner) = owners.borrow().get(label) {
            owner
                .state
                .borrow_mut()
                .settled(receipt, source, failed, download, same_document);
        }
    });
    let _ = super::presentation::refresh(label);
}
pub(super) fn check(label: &str, receipt: Receipt) -> Result<(), String> {
    if self::receipt(label)? == receipt {
        Ok(())
    } else {
        Err("Browser navigation is no longer current.".into())
    }
}

/// A committed loader sample may prove a document only when the native current
/// attempt has itself committed. During provisional B, CDP still exposes A.
pub(crate) async fn committed_receipt(
    view: &tauri::Webview,
) -> Result<Option<Receipt>, WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.app_handle()
        .run_on_main_thread(move || {
            let result = OWNERS.with(|owners| {
                let owners = owners.borrow();
                let owner = owners
                    .get(&label)
                    .ok_or(WorkspaceError::NativeCommandFailed)?;
                Ok(owner.state.borrow().committed_receipt())
            });
            let _ = tx.send(result);
        })
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
/// Stop is a browser-process native operation, independent of renderer CDP
/// queues or dialogs. The exact attempt is checked in the same UI dispatch.
pub(crate) async fn stop_current(view: &tauri::Webview, expected: Receipt) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = OWNERS.with(|owners| {
            let owners = owners.borrow();
            let owner = owners
                .get(&label)
                .ok_or_else(|| "Native navigation owner is unavailable.".to_owned())?;
            let mut state = owner.state.borrow_mut();
            state.cancellation_intent(expected, BrowserCancellationReason::UserStop)?;
            drop(state);
            unsafe { owner.core.Stop() }.map_err(|_| "Native browser Stop failed.".to_owned())?;
            owner.state.borrow_mut().stop(expected)
        });
        let _ = super::presentation::refresh(&label);
        let _ = tx.send(result);
    })
    .map_err(|_| "Native browser Stop could not be dispatched.".to_owned())?;
    rx.await
        .map_err(|_| "Native browser Stop acknowledgement was lost.".to_owned())?
}
pub(crate) async fn receipt_for_target(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    expected_sequence: Option<u64>,
) -> Result<Receipt, WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.app_handle()
        .run_on_main_thread(move || {
            let result = OWNERS.with(|owners| {
                let owners = owners.borrow();
                let owner = owners
                    .get(&label)
                    .ok_or(WorkspaceError::NativeCommandFailed)?;
                let state = owner.state.borrow();
                state.receipt_for_target(&target, expected_sequence)
            });
            let _ = tx.send(result);
        })
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
pub(crate) async fn set_paused(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    paused: bool,
    expected_sequence: u64,
) -> Result<(), WorkspaceError> {
    if paused && !super::navigation_request_gate::supported() {
        return Err(WorkspaceError::UnsupportedAction);
    }
    // The native hold prevents replacement while request-stage activation is
    // pending. The public paused flag is set only after Fetch acknowledgement.
    let receipt = receipt_for_target(view, target.clone(), Some(expected_sequence)).await?;
    if paused {
        set_paused_native(view, target.clone(), true, expected_sequence, true).await?;
        let lease = match super::navigation_request_gate::enable(view).await {
            Ok(lease) => lease,
            Err(_) => {
                // A failed gate must not be advertised as a completed pause. Its
                // private owner retains uncertain jobs until safe cleanup/close.
                if super::navigation_request_gate::safeinactive(view)
                    .await
                    .unwrap_or(false)
                {
                    rollback_pending(view, expected_sequence).await?;
                }
                return Err(WorkspaceError::UnsupportedAction);
            }
        };
        if let Err(error) =
            set_paused_native(view, target.clone(), true, expected_sequence, false).await
        {
            super::navigation_request_gate::disable_owned(view, lease)
                .await
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            rollback_pending(view, expected_sequence).await?;
            return Err(error);
        }
    } else {
        super::navigation_request_gate::disable_guarded(view, Some(receipt))
            .await
            .map_err(|error| {
                if error == "Browser navigation is no longer current." {
                    WorkspaceError::StaleTarget
                } else {
                    WorkspaceError::UnsupportedAction
                }
            })?;
        set_paused_native(view, target, false, expected_sequence, false).await?;
    }
    Ok(())
}
async fn rollback_pending(
    view: &tauri::Webview,
    expected_sequence: u64,
) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = OWNERS.with(|owners| {
            let owners = owners.borrow();
            let owner = owners
                .get(&label)
                .ok_or(WorkspaceError::NativeCommandFailed)?;
            let mut state = owner.state.borrow_mut();
            if state.sequence == expected_sequence {
                state.pause_pending = false;
            }
            state.control();
            state.revision.bump();
            Ok(())
        });
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
async fn set_paused_native(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    paused: bool,
    expected_sequence: u64,
    pending: bool,
) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = OWNERS.with(|owners| {
            let (core, owned_state) = {
                let owners = owners.borrow();
                let owner = owners.get(&label).ok_or(WorkspaceError::NativeCommandFailed)?;
                (owner.core.clone(), owner.state.clone())
            };
            let mut state = owned_state.borrow_mut();
            state.receipt_for_target(&target, Some(expected_sequence))?;
            if pending {
                state.pause_pending = paused;
            } else {
                state.paused = paused;
                state.pause_pending = false;
            }
            if !paused {
                state.trace.clear_repeat_window();
            }
            state.control();
            state.revision.bump();
            let receipt = state.receipt();
            let in_flight = state
                .metadata
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .load
                .as_ref()
                .is_some_and(|load| {
                    matches!(
                        load.phase,
                        BrowserNavigationPhase::Requested
                            | BrowserNavigationPhase::Provisional
                            | BrowserNavigationPhase::Committed
                    )
                });
            if paused && in_flight {
                if let Some(attempt) = state.current.and_then(|id| state.attempts.get_mut(&id)) {
                    attempt.cancellation =
                        Some(BrowserCancellationReason::AutomaticNavigationPaused);
                }
            }
            // Completing a denied popup can produce native callbacks. Publish
            // the hold first and drop the navigation borrow before cleanup.
            drop(state);
            if paused {
                super::popup::pause_view(&label);
            }
            if paused && in_flight {
                unsafe { core.Stop() }.map_err(|_| WorkspaceError::NativeCommandFailed)?;
                owned_state
                    .borrow_mut()
                    .stop_for(
                        receipt,
                        BrowserCancellationReason::AutomaticNavigationPaused,
                    )
                    .map_err(|_| WorkspaceError::StaleTarget)?;
            }
            Ok(())
        });
        let _ = super::presentation::refresh(&label);
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
pub(crate) async fn report(
    view: &tauri::Webview,
    target: BrowserTabTarget,
) -> Result<BrowserNavigationReport, WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = OWNERS.with(|owners| {
            let owners = owners.borrow();
            let owner = owners
                .get(&label)
                .ok_or(WorkspaceError::NativeCommandFailed)?;
            let state = owner.state.borrow();
            let tab = state.metadata.lock().unwrap_or_else(|e| e.into_inner());
            if tab.target != target {
                return Err(WorkspaceError::StaleTarget);
            }
            Ok(BrowserNavigationReport {
                target,
                load: tab.load.clone(),
                trace: state.trace.snapshot(),
                identity: None,
                runtime: None,
            })
        });
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<BrowserRevision>,
    input_locked: Arc<std::sync::atomic::AtomicBool>,
    bootstrap: bool,
) -> Result<(), String> {
    super::presentation::install(view, metadata.clone()).await?;
    let label = view.label().to_owned();
    let owned_view = view.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if OWNERS.with(|owners| owners.borrow().contains_key(&label)) {
                return Ok(());
            }
            let core = unsafe { platform.controller().CoreWebView2()? };
            let state = Rc::new(RefCell::new(State::new(metadata, revision, bootstrap)));
            let starting_state = state.clone();
            let starting_label = label.clone();
            let handler = NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let paused = {
                    let state = starting_state.borrow();
                    state.paused || state.pause_pending
                };
                // Enforce the hold before reading optional diagnostic facts.
                let request_stage_hold = super::navigation_request_gate::blocking(&starting_label);
                // A proved HTTP request-stage hold aborts the exact Fetch job.
                // Avoid SDK cancellation racing that acknowledgement; it can
                // retire the interception before the event reaches the host.
                if paused && !request_stage_hold {
                    unsafe { args.SetCancel(true)? };
                }
                let (mut id, mut redirect, mut user, mut raw) =
                    (0, BOOL::default(), BOOL::default(), PWSTR::null());
                unsafe {
                    args.NavigationId(&mut id)?;
                    args.IsRedirected(&mut redirect)?;
                    args.IsUserInitiated(&mut user)?;
                    args.Uri(&mut raw)?;
                }
                let Some(url) = super::event_string(raw, 8192) else {
                    starting_state.borrow_mut().trace.mark_routing_lost();
                    return Ok(());
                };
                if paused
                    && request_stage_hold
                    && !url::Url::parse(&url)
                        .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
                {
                    unsafe { args.SetCancel(true)? };
                }
                let mut native_kind = COREWEBVIEW2_NAVIGATION_KIND::default();
                let kind =
                    if let Ok(args3) = args.cast::<ICoreWebView2NavigationStartingEventArgs3>() {
                        if unsafe { args3.NavigationKind(&mut native_kind) }.is_ok() {
                            match native_kind {
                                COREWEBVIEW2_NAVIGATION_KIND_RELOAD => {
                                    BrowserNavigationActionType::Reload
                                }
                                COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD => {
                                    BrowserNavigationActionType::BackForward
                                }
                                _ => BrowserNavigationActionType::Other,
                            }
                        } else {
                            BrowserNavigationActionType::Unknown
                        }
                    } else {
                        BrowserNavigationActionType::Unknown
                    };
                let action =
                    action_summary(kind, BrowserNavigationPolicy::Allow, Some(user.as_bool()));
                let mut rejected = BOOL::default();
                unsafe {
                    args.Cancel(&mut rejected)?;
                }
                if rejected.as_bool() && !paused {
                    starting_state.borrow_mut().rejected(id, url, action);
                    let _ = super::presentation::refresh(&starting_label);
                    return Ok(());
                }
                let _started =
                    starting_state
                        .borrow_mut()
                        .started(id, url, redirect.as_bool(), action);
                let _ = super::presentation::refresh(&starting_label);
                Ok(())
            }));
            let mut starting = 0;
            unsafe {
                core.add_NavigationStarting(&handler, &mut starting)?;
            }
            let content_state = state.clone();
            let content_label = label.clone();
            let handler = ContentLoadingEventHandler::create(Box::new(move |core, args| {
                let (Some(core), Some(args)) = (core, args) else {
                    return Ok(());
                };
                let (mut id, mut error_page, mut raw) = (0, BOOL::default(), PWSTR::null());
                unsafe {
                    args.NavigationId(&mut id)?;
                    args.IsErrorPage(&mut error_page)?;
                    core.Source(&mut raw)?;
                }
                if let Some(url) = super::event_string(raw, 8192) {
                    if content_state
                        .borrow_mut()
                        .committed(id, url, error_page.as_bool())
                    {
                        let _ = super::permissions::navigation_started(&content_label);
                        super::user_file_chooser::cancel(&content_label);
                        super::user_downloads::cancel_pending(&content_label);
                    }
                    let _ = super::presentation::refresh(&content_label);
                }
                Ok(())
            }));
            let mut content = 0;
            if let Err(error) = unsafe { core.add_ContentLoading(&handler, &mut content) } {
                let _ = unsafe { core.remove_NavigationStarting(starting) };
                return Err(error);
            }
            let completed_state = state.clone();
            let completed_label = label.clone();
            let handler = NavigationCompletedEventHandler::create(Box::new(move |core, args| {
                let (Some(core), Some(args)) = (core, args) else {
                    return Ok(());
                };
                let (mut id, mut success, mut status, mut raw) = (
                    0,
                    BOOL::default(),
                    COREWEBVIEW2_WEB_ERROR_STATUS::default(),
                    PWSTR::null(),
                );
                unsafe {
                    args.NavigationId(&mut id)?;
                    args.IsSuccess(&mut success)?;
                    args.WebErrorStatus(&mut status)?;
                    core.Source(&mut raw)?;
                }
                let mut http = 0;
                let http = args
                    .cast::<ICoreWebView2NavigationCompletedEventArgs2>()
                    .ok()
                    .and_then(|args| unsafe { args.HttpStatusCode(&mut http) }.ok())
                    .and_then(|_| u16::try_from(http).ok())
                    .filter(|status| *status > 0);
                if let Some(url) = super::event_string(raw, 8192) {
                    if completed_state.borrow_mut().completed(
                        id,
                        url,
                        success.as_bool(),
                        status,
                        http,
                    ) {
                        let _ = super::presentation::refresh(&completed_label);
                        super::diagnostics::document_loaded(&owned_view);
                        super::super::host::reconcile_chooser_after_load(
                            &owned_view,
                            input_locked.clone(),
                        );
                    }
                }
                Ok(())
            }));
            let mut completed = 0;
            if let Err(error) = unsafe { core.add_NavigationCompleted(&handler, &mut completed) } {
                unsafe {
                    let _ = core.remove_NavigationStarting(starting);
                    let _ = core.remove_ContentLoading(content);
                }
                return Err(error);
            }
            let source_state = state.clone();
            let handler = SourceChangedEventHandler::create(Box::new(move |core, args| {
                let (Some(core), Some(args)) = (core, args) else {
                    return Ok(());
                };
                let mut new_document = BOOL::default();
                unsafe {
                    args.IsNewDocument(&mut new_document)?;
                }
                if !new_document.as_bool() {
                    let mut raw = PWSTR::null();
                    unsafe {
                        core.Source(&mut raw)?;
                    }
                    if let Some(url) = super::event_string(raw, 8192) {
                        source_state.borrow_mut().same_document(url);
                    }
                }
                Ok(())
            }));
            let mut source = 0;
            if let Err(error) = unsafe { core.add_SourceChanged(&handler, &mut source) } {
                unsafe {
                    let _ = core.remove_NavigationStarting(starting);
                    let _ = core.remove_ContentLoading(content);
                    let _ = core.remove_NavigationCompleted(completed);
                }
                return Err(error);
            }
            OWNERS.with(|owners| {
                owners.borrow_mut().insert(
                    label,
                    Registration {
                        core,
                        state,
                        starting,
                        content,
                        completed,
                        source,
                    },
                )
            });
            Ok(())
        })();
        let _ = tx.send(
            result.map_err(|_| "Native navigation tracking could not be installed.".to_owned()),
        );
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        let tab = BrowserTabSnapshot {
            target: BrowserTabTarget {
                tab_id: "navigation".into(),
                runtime_generation: 1,
                document_generation: 0,
            },
            title: String::new(),
            url: "about:blank".into(),
            lifecycle: BrowserTabLifecycle::Loading,
            load: None,
            can_go_back: false,
            can_go_forward: false,
            zoom_percent: 100,
            blocked_permissions: vec![],
            permission_requests: vec![],
            script_dialog: None,
            diagnostics: Default::default(),
        };
        State::new(
            Arc::new(Mutex::new(tab)),
            Arc::new(BrowserRevision::default()),
            true,
        )
    }
    fn start(s: &mut State, id: u64, url: &str, redirect: bool) -> bool {
        s.started(
            id,
            url.into(),
            redirect,
            action_summary(
                BrowserNavigationActionType::Other,
                BrowserNavigationPolicy::Allow,
                Some(false),
            ),
        )
    }
    #[test]
    fn bootstrap_redirects_and_late_completions_do_not_forge_documents() {
        let mut s = state();
        assert!(!start(&mut s, 1, "about:blank", false));
        assert_eq!(s.sequence, 0);
        let receipt = s.requested(
            BrowserNavigationSource::UserCommand,
            Some("https://example.test/a".into()),
            BrowserNavigationActionType::Other,
        );
        start(&mut s, 2, "https://example.test/a", false);
        start(&mut s, 2, "https://example.test/b", true);
        assert_eq!(s.sequence, 2);
        s.settled(
            receipt,
            BrowserNavigationSource::UserCommand,
            false,
            false,
            false,
        );
        start(&mut s, 3, "https://example.test/c", false);
        s.committed(3, "https://example.test/c".into(), false);
        assert!(!s.completed(
            2,
            "https://example.test/b".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200)
        ));
        let tab = s.metadata.lock().unwrap();
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Loading);
        assert_eq!(tab.url, "https://example.test/c");
    }
    #[test]
    fn failed_and_stopped_attempts_retain_content_and_exact_guards() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/ok", false);
        s.committed(1, "https://example.test/ok".into(), false);
        s.completed(
            1,
            "https://example.test/ok".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        start(&mut s, 2, "https://missing.test/", false);
        let old = s.receipt();
        s.completed(
            2,
            "https://missing.test/".into(),
            false,
            COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED,
            None,
        );
        let tab = s.metadata.lock().unwrap();
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Failed);
        assert_eq!(
            tab.load.as_ref().unwrap().content_state,
            BrowserContentState::RetainedDocument
        );
        assert_eq!(
            tab.load
                .as_ref()
                .unwrap()
                .problem
                .as_ref()
                .unwrap()
                .safe_reason,
            BrowserFailureReason::Dns
        );
        drop(tab);
        start(&mut s, 3, "https://example.test/next", false);
        assert!(s.stop(old).is_err());
        assert!(s.stop(s.receipt()).is_ok());
        s.completed(
            3,
            "https://example.test/next".into(),
            false,
            COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
            None,
        );
        assert_eq!(
            s.metadata.lock().unwrap().lifecycle,
            BrowserTabLifecycle::Stopped
        );
    }
    #[test]
    fn paused_policy_preserves_document_and_trace_counts_only_real_attempts() {
        let mut s = state();
        s.bootstrap = false;
        for id in 1..=6 {
            start(&mut s, id, "https://example.test/reload?private=1", false);
            s.completed(
                id,
                "https://example.test/reload?private=1".into(),
                true,
                COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                Some(200),
            );
        }
        assert!(s.trace.control(false).loop_suspected);
        s.paused = true;
        assert!(!start(
            &mut s,
            7,
            "https://example.test/reload?private=1",
            false
        ));
        assert_eq!(s.sequence, 6);
        assert_eq!(
            s.metadata.lock().unwrap().lifecycle,
            BrowserTabLifecycle::Ready
        );
        let trace = s.trace.snapshot();
        let json = serde_json::to_string(&trace).unwrap();
        assert!(!json.contains("private"));
    }
    #[test]
    fn pause_blocks_native_popup_even_with_a_gesture_without_forging_main_navigation() {
        for pending in [false, true] {
            for gesture in [false, true] {
                let mut s = state();
                s.bootstrap = false;
                start(&mut s, 1, "https://example.test/parent", false);
                s.committed(1, "https://example.test/parent".into(), false);
                s.completed(
                    1,
                    "https://example.test/parent".into(),
                    true,
                    COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                    Some(200),
                );
                s.pause_pending = pending;
                s.paused = !pending;
                let receipt = s.receipt();
                let before = s.trace.snapshot();
                assert!(!s.popup_requested(
                    Some("https://example.test/child?credential=hidden".into()),
                    gesture,
                ));
                assert_eq!(s.receipt(), receipt);
                let after = s.trace.snapshot();
                assert_eq!(after.counters, before.counters);
                let event = after.events.last().unwrap();
                assert_eq!(event.kind, BrowserNavigationEventKind::Action);
                assert_eq!(event.navigation_sequence, None);
                assert_eq!(event.document_generation, None);
                assert_eq!(event.source, BrowserNavigationSource::PageNavigation);
                assert_eq!(event.url.as_deref(), Some("https://example.test/child"));
                let action = event.action.as_ref().unwrap();
                assert!(!action.main_frame);
                assert_eq!(action.policy, BrowserNavigationPolicy::Paused);
                assert_eq!(action.user_initiated, Some(gesture));
                assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
                assert!(!serde_json::to_string(&after).unwrap().contains("hidden"));
            }
        }
    }
    #[test]
    fn popup_admission_requires_exact_live_opener_and_hold_release() {
        let mut s = state();
        let target = s.metadata.lock().unwrap().target.clone();
        assert!(s.popup_permitted(&target));
        s.pause_pending = true;
        assert!(!s.popup_permitted(&target));
        s.pause_pending = false;
        s.paused = true;
        assert!(!s.popup_permitted(&target));
        s.paused = false;
        assert!(s.popup_permitted(&target));
        for foreign in [
            BrowserTabTarget { tab_id: "foreign".into(), ..target.clone() },
            BrowserTabTarget { runtime_generation: 2, ..target.clone() },
            BrowserTabTarget { document_generation: 1, ..target.clone() },
        ] {
            assert!(!s.popup_permitted(&foreign));
        }
    }
    #[test]
    fn dispatch_and_native_completion_orders_end_uncertainty_without_forging_origin() {
        for native_first in [true, false] {
            let mut s = state();
            let dispatch = s.requested(
                BrowserNavigationSource::UserCommand,
                Some("https://example.test/".into()),
                BrowserNavigationActionType::Other,
            );
            start(&mut s, 1, "https://example.test/", false);
            if native_first {
                s.completed(
                    1,
                    "https://example.test/".into(),
                    true,
                    COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                    Some(200),
                );
                s.settled(
                    dispatch,
                    BrowserNavigationSource::UserCommand,
                    false,
                    false,
                    false,
                );
            } else {
                s.settled(
                    dispatch,
                    BrowserNavigationSource::UserCommand,
                    false,
                    false,
                    false,
                );
                s.completed(
                    1,
                    "https://example.test/".into(),
                    true,
                    COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                    Some(200),
                );
            }
            assert!(!s.host_interval);
            start(&mut s, 2, "https://example.test/reload", false);
            let trace = s.trace.snapshot();
            assert_eq!(trace.counters.user_commands, 1);
            assert_eq!(trace.counters.unknown_navigations, 1);
            assert_eq!(trace.counters.page_navigations, 1);
            assert!(
                trace
                    .events
                    .iter()
                    .filter(|event| event.kind == BrowserNavigationEventKind::Started)
                    .all(|event| event.source != BrowserNavigationSource::UserCommand)
            );
        }
    }
    #[test]
    fn provisional_successor_retains_document_but_revokes_old_control_and_loader_proof() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/a", false);
        s.committed(1, "https://example.test/a".into(), false);
        s.completed(
            1,
            "https://example.test/a".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        let target = s.metadata.lock().unwrap().target.clone();
        let old = s.receipt();
        assert!(s.committed_receipt().is_some());
        start(&mut s, 2, "https://example.test/b", false);
        assert_eq!(s.metadata.lock().unwrap().target, target);
        assert_eq!(
            s.receipt_for_target(&target, Some(old.sequence)),
            Err(WorkspaceError::StaleTarget)
        );
        assert!(s.committed_receipt().is_none());
        assert!(s.stop(old).is_err());
        let current = s.receipt_for_target(&target, Some(s.sequence)).unwrap();
        assert!(s.stop(current).is_ok());
        s.committed(2, "https://example.test/b".into(), false);
        assert_ne!(
            s.metadata.lock().unwrap().target.document_generation,
            target.document_generation
        );
    }
    #[test]
    fn unknown_provisional_refresh_loop_warns_without_inventing_page_origin() {
        let mut s = state();
        let dispatch = s.requested(
            BrowserNavigationSource::UserCommand,
            Some("https://example.test/loop".into()),
            BrowserNavigationActionType::Other,
        );
        start(&mut s, 1, "https://example.test/loop", false);
        s.settled(
            dispatch,
            BrowserNavigationSource::UserCommand,
            false,
            false,
            false,
        );
        for id in 2..=7 {
            start(&mut s, id, "https://example.test/loop", false);
            s.completed(
                id - 1,
                "https://example.test/loop".into(),
                false,
                COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
                None,
            );
        }
        let trace = s.trace.snapshot();
        assert_eq!(trace.counters.user_commands, 1);
        assert_eq!(trace.counters.page_navigations, 0);
        assert_eq!(trace.counters.unknown_navigations, 7);
        assert!(s.trace.control(false).loop_suspected);
    }
    #[test]
    fn late_cancel_retains_its_native_reason_and_download_retains_dispatch_identity() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/a", false);
        let first = s.receipt();
        s.stop(first).unwrap();
        start(&mut s, 2, "https://example.test/b", false);
        s.completed(
            1,
            "https://example.test/a".into(),
            false,
            COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
            None,
        );
        assert_eq!(
            s.trace
                .snapshot()
                .events
                .last()
                .unwrap()
                .cancellation_reason,
            Some(BrowserCancellationReason::UserStop)
        );
        let dispatch = s.requested(
            BrowserNavigationSource::AgentCommand,
            Some("https://example.test/file".into()),
            BrowserNavigationActionType::Other,
        );
        start(&mut s, 3, "https://example.test/file", false);
        s.settled(
            dispatch,
            BrowserNavigationSource::AgentCommand,
            false,
            true,
            false,
        );
        let trace = s.trace.snapshot();
        let result = trace.events.last().unwrap();
        assert_eq!(result.dispatch_sequence, Some(dispatch.submission));
        assert_eq!(result.navigation_sequence, None);
        assert_eq!(result.source, BrowserNavigationSource::AgentCommand);
        assert_eq!(
            result.cancellation_reason,
            Some(BrowserCancellationReason::Download)
        );
    }
    #[test]
    fn same_document_command_finishes_without_a_new_native_attempt() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/a", false);
        s.committed(1, "https://example.test/a".into(), false);
        s.completed(
            1,
            "https://example.test/a".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        let generation = s.metadata.lock().unwrap().target.document_generation;
        let dispatch = s.requested(
            BrowserNavigationSource::UserCommand,
            Some("https://example.test/a#section".into()),
            BrowserNavigationActionType::Other,
        );
        s.same_document("https://example.test/a#section".into());
        s.settled(
            dispatch,
            BrowserNavigationSource::UserCommand,
            false,
            false,
            true,
        );
        let tab = s.metadata.lock().unwrap();
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Ready);
        assert_eq!(tab.target.document_generation, generation);
        assert_eq!(tab.load.as_ref().unwrap().navigation_sequence, 2);
        assert_eq!(
            tab.load.as_ref().unwrap().content_url.as_deref(),
            Some("https://example.test/a#section")
        );
    }
    fn ready_history_state() -> State {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/document", false);
        s.committed(1, "https://example.test/document".into(), false);
        s.completed(1, "https://example.test/document".into(), true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, Some(200));
        s
    }
    #[test]
    fn history_keeps_ready_document_before_and_after_same_document_source_or_reply() {
        for source_first in [false, true] {
            for source in [BrowserNavigationSource::UserCommand, BrowserNavigationSource::AgentCommand] {
                let mut s = ready_history_state();
                let original = s.metadata.lock().unwrap().clone();
                let before = s.trace.snapshot();
                let history = s.requested(source, None, BrowserNavigationActionType::BackForward);
                assert_eq!(s.current, Some(1));
                assert!(!s.requested_matches(history));
                if source_first { s.same_document("https://example.test/document#previous".into()); }
                s.settled(history, source, false, false, false);
                if !source_first { s.same_document("https://example.test/document#previous".into()); }
                let tab = s.metadata.lock().unwrap();
                let load = tab.load.as_ref().unwrap();
                assert_eq!(tab.lifecycle, BrowserTabLifecycle::Ready);
                assert_eq!(tab.target, original.target);
                assert_eq!(load.phase, BrowserNavigationPhase::Finished);
                assert_eq!(load.content_state, BrowserContentState::CurrentDocument);
                assert_eq!(load.estimated_progress, Some(100));
                assert_eq!(load.navigation_sequence, history.sequence);
                assert!(history.sequence > original.load.as_ref().unwrap().navigation_sequence);
                assert_eq!(load.content_url.as_deref(), Some("https://example.test/document#previous"));
                drop(tab);
                let after = s.trace.snapshot();
                assert_eq!(after.events.len(), before.events.len() + 1);
                assert_eq!(after.events.last().unwrap().dispatch_sequence, Some(history.submission));
                assert_eq!(after.events.last().unwrap().navigation_sequence, None);
                assert_eq!(after.counters.page_navigations, before.counters.page_navigations);
                assert_eq!(after.counters.unknown_navigations, before.counters.unknown_navigations);
                assert!(s.history_dispatch_unresolved && s.host_interval);
            }
        }
    }
    #[test]
    fn history_preserves_failed_cancelled_crashed_content_and_safe_problem_facts() {
        for case in 0..5 {
            let mut s = ready_history_state();
            start(&mut s, 2, "https://example.test/attempt", false);
            match case {
                0 => { s.completed(2, "https://example.test/attempt".into(), false,
                    COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED, None); }
                1 => { s.committed(2, "https://example.test/attempt".into(), true);
                    s.completed(2, "https://example.test/attempt".into(), false,
                        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID, None); }
                2 => { s.stop(s.receipt()).unwrap(); }
                3 => { s.committed(2, "https://example.test/attempt".into(), true);
                    s.stop(s.receipt()).unwrap(); }
                _ => s.crashed(),
            }
            s.paused = true;
            s.control();
            let original = s.metadata.lock().unwrap().clone();
            let native_id = s.current;
            let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
            s.settled(history, BrowserNavigationSource::UserCommand, false, false, false);
            let tab = s.metadata.lock().unwrap();
            let mut expected = original.load.unwrap();
            expected.navigation_sequence = history.sequence;
            expected.navigation_control = tab.load.as_ref().unwrap().navigation_control.clone();
            assert_eq!(tab.load.as_ref().unwrap(), &expected);
            assert_eq!(tab.lifecycle, original.lifecycle);
            assert_eq!(tab.target, original.target);
            assert_eq!(s.current, native_id);
            assert!(!s.paused);
        }
    }
    #[test]
    fn full_history_enters_loading_only_on_a_real_native_attempt() {
        let mut s = ready_history_state();
        let generation = s.receipt().generation;
        let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
        s.settled(history, BrowserNavigationSource::UserCommand, false, false, false);
        assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
        assert!(s.host_interval);
        start(&mut s, 2, "https://example.test/full-history", false);
        {
            let tab = s.metadata.lock().unwrap();
            assert_eq!(tab.lifecycle, BrowserTabLifecycle::Loading);
            assert_eq!(tab.load.as_ref().unwrap().phase, BrowserNavigationPhase::Provisional);
            assert_eq!(tab.load.as_ref().unwrap().content_state, BrowserContentState::RetainedDocument);
            assert_eq!(tab.target.document_generation, generation);
        }
        assert!(!s.history_dispatch_unresolved);
        assert_eq!(s.attempts.get(&2).unwrap().source, BrowserNavigationSource::Unknown);
        s.committed(2, "https://example.test/full-history".into(), false);
        s.completed(2, "https://example.test/full-history".into(), true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, Some(200));
        assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
        assert_eq!(s.receipt().generation, generation + 1);
        assert!(!s.host_interval);
    }
    #[test]
    fn history_control_fence_preserves_native_stop_and_sticky_cancel_association() {
        let mut s = ready_history_state();
        start(&mut s, 2, "https://example.test/loading", false);
        let old = s.receipt();
        let target = s.metadata.lock().unwrap().target.clone();
        let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
        assert_eq!(s.receipt_for_target(&target, Some(old.sequence)), Err(WorkspaceError::StaleTarget));
        assert!(s.stop(old).is_err());
        assert_eq!(s.receipt_for_target(&target, Some(history.sequence)).unwrap(), history);
        s.settled(history, BrowserNavigationSource::UserCommand, false, false, false);
        s.stop_for(history, BrowserCancellationReason::AutomaticNavigationPaused).unwrap();
        let trace = s.trace.snapshot();
        let stop = trace.events.last().unwrap();
        assert_eq!(stop.navigation_sequence, Some(s.attempts.get(&2).unwrap().sequence));
        assert_ne!(stop.navigation_sequence, Some(history.sequence));
        assert!(s.committed(2, "https://example.test/loading".into(), false));
        assert!(!s.completed(2, "https://example.test/loading".into(), false,
            COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED, None));
        let tab = s.metadata.lock().unwrap();
        assert_eq!(tab.load.as_ref().unwrap().phase, BrowserNavigationPhase::Cancelled);
        assert_eq!(tab.load.as_ref().unwrap().cancellation_reason, Some(BrowserCancellationReason::AutomaticNavigationPaused));
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
        drop(tab);
        assert!(s.history_origin_unresolved && s.host_interval);
    }
    #[test]
    fn same_document_successor_reply_does_not_guess_a_late_history_native_start_origin() {
        let mut s = ready_history_state();
        let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
        s.settled(history, BrowserNavigationSource::UserCommand, false, false, false);
        let successor = s.requested(BrowserNavigationSource::UserCommand,
            Some("https://example.test/document#explicit".into()), BrowserNavigationActionType::Other);
        assert!(!s.history_dispatch_unresolved);
        s.same_document("https://example.test/document#explicit".into());
        s.settled(successor, BrowserNavigationSource::UserCommand, false, false, true);
        assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
        assert!(s.history_origin_unresolved && s.host_interval);
        start(&mut s, 2, "https://example.test/late-history", false);
        assert_eq!(s.attempts.get(&2).unwrap().source, BrowserNavigationSource::Unknown);
        assert!(!s.history_origin_unresolved);
        s.committed(2, "https://example.test/late-history".into(), false);
        s.completed(2, "https://example.test/late-history".into(), true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, Some(200));
        assert!(!s.host_interval);
    }
    #[test]
    fn late_history_replies_and_old_native_callbacks_cannot_settle_a_successor() {
        for rejection in [false, true] {
            let mut s = ready_history_state();
            let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
            let successor = s.requested(BrowserNavigationSource::UserCommand,
                Some("https://example.test/successor".into()), BrowserNavigationActionType::Other);
            assert!(!s.history_dispatch_unresolved);
            if rejection { s.settled_rejection(history, BrowserNavigationSource::UserCommand); }
            else { s.settled(history, BrowserNavigationSource::UserCommand, false, false, true); }
            s.same_document("https://example.test/document#old".into());
            assert!(!s.completed(1, "https://example.test/document".into(), false,
                COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED, None));
            assert!(s.requested_matches(successor));
            assert!(s.host_interval);
            start(&mut s, 2, "https://example.test/successor", false);
            s.settled(successor, BrowserNavigationSource::UserCommand, false, false, false);
            s.committed(2, "https://example.test/successor".into(), false);
            s.completed(2, "https://example.test/successor".into(), true,
                COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, Some(200));
            assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
        }
    }
    #[test]
    fn history_before_unload_scope_preserves_load_and_is_revoked_by_a_new_attempt() {
        let mut s = ready_history_state();
        let old = s.receipt();
        let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
        let original = s.metadata.lock().unwrap().load.clone();
        assert!(s.before_unload_scope(history));
        assert!(!s.before_unload_scope(old));
        assert!(s.reject_before_unload(history));
        assert!(s.before_unload_stopped_matches(history));
        assert_eq!(s.metadata.lock().unwrap().load, original);
        start(&mut s, 2, "https://example.test/new", false);
        assert!(!s.before_unload_scope(history));
        assert!(!s.reject_before_unload(history));
        assert!(!s.before_unload_stopped_matches(history));
        assert_eq!(s.metadata.lock().unwrap().load.as_ref().unwrap().phase, BrowserNavigationPhase::Provisional);
    }
    #[test]
    fn history_crash_event_uses_native_attempt_instead_of_control_fence() {
        let mut s = ready_history_state();
        let native_sequence = s.attempts.get(&1).unwrap().sequence;
        let history = s.requested(BrowserNavigationSource::UserCommand, None, BrowserNavigationActionType::BackForward);
        s.crashed();
        let trace = s.trace.snapshot();
        let event = trace.events.last().unwrap();
        assert_eq!(event.kind, BrowserNavigationEventKind::Crashed);
        assert_eq!(event.navigation_sequence, Some(native_sequence));
        assert_ne!(event.navigation_sequence, Some(history.sequence));
        assert!(!s.history_dispatch_unresolved);
        assert_eq!(s.metadata.lock().unwrap().load.as_ref().unwrap().content_state, BrowserContentState::None);
    }
    #[test]
    fn host_dispatch_fence_revokes_old_wire_controls_before_native_start() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/a", false);
        s.committed(1, "https://example.test/a".into(), false);
        s.completed(
            1,
            "https://example.test/a".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        let target = s.metadata.lock().unwrap().target.clone();
        let old_sequence = s.sequence;
        let dispatch = s.requested(
            BrowserNavigationSource::UserCommand,
            Some("https://example.test/b".into()),
            BrowserNavigationActionType::Other,
        );
        assert_eq!(s.metadata.lock().unwrap().target, target);
        assert!(s.sequence > old_sequence);
        assert_eq!(s.current, None);
        assert_eq!(
            s.receipt_for_target(&target, Some(old_sequence)),
            Err(WorkspaceError::StaleTarget)
        );
        s.stop(dispatch).unwrap();
        s.completed(
            1,
            "https://example.test/a".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        let tab = s.metadata.lock().unwrap();
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
        assert_eq!(
            tab.load.as_ref().unwrap().phase,
            BrowserNavigationPhase::Cancelled
        );
        assert_eq!(
            tab.load.as_ref().unwrap().requested_url.as_deref(),
            Some("https://example.test/b")
        );
    }
    #[test]
    fn stopped_request_is_not_rewritten_by_late_failed_or_same_document_reply() {
        for reason in [
            BrowserCancellationReason::UserStop,
            BrowserCancellationReason::AutomaticNavigationPaused,
        ] {
            for same_document in [true, false] {
                let mut s = state();
                let dispatch = s.requested(
                    BrowserNavigationSource::UserCommand,
                    Some("https://example.test/".into()),
                    BrowserNavigationActionType::Other,
                );
                s.stop_for(dispatch, reason).unwrap();
                s.settled(
                    dispatch,
                    BrowserNavigationSource::UserCommand,
                    !same_document,
                    false,
                    same_document,
                );
                let tab = s.metadata.lock().unwrap();
                assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
                assert_eq!(
                    tab.load.as_ref().unwrap().phase,
                    BrowserNavigationPhase::Cancelled
                );
                assert_eq!(tab.load.as_ref().unwrap().cancellation_reason, Some(reason));
            }
        }
    }
    #[test]
    fn acknowledged_stop_keeps_state_and_retains_independent_native_completion() {
        for reason in [
            BrowserCancellationReason::UserStop,
            BrowserCancellationReason::AutomaticNavigationPaused,
        ] {
            for retained_document in [false, true] {
                for native_first in [false, true] {
                    for (success, status, expected_kind, failure_reason) in [
                        (
                            false,
                            COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
                            BrowserNavigationEventKind::Failed,
                            Some(BrowserFailureReason::NetworkConnection),
                        ),
                        (
                            false,
                            COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED,
                            BrowserNavigationEventKind::Failed,
                            Some(BrowserFailureReason::Dns),
                        ),
                        (
                            false,
                            COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID,
                            BrowserNavigationEventKind::Failed,
                            Some(BrowserFailureReason::Tls),
                        ),
                        (
                            false,
                            COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
                            BrowserNavigationEventKind::Cancelled,
                            None,
                        ),
                        (
                            true,
                            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                            BrowserNavigationEventKind::Finished,
                            None,
                        ),
                    ] {
                        let mut s = state();
                        s.bootstrap = false;
                        if retained_document {
                            start(&mut s, 1, "https://example.test/retained", false);
                            s.committed(1, "https://example.test/retained".into(), false);
                            s.completed(
                                1,
                                "https://example.test/retained".into(),
                                true,
                                COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                                Some(200),
                            );
                        }
                        let dispatch = s.requested(
                            BrowserNavigationSource::UserCommand,
                            Some("https://example.test/slow".into()),
                            BrowserNavigationActionType::Other,
                        );
                        start(&mut s, 2, "https://example.test/slow", false);
                        s.paused = reason == BrowserCancellationReason::AutomaticNavigationPaused;
                        let native_sequence = s.sequence;
                        s.stop_for(s.receipt(), reason).unwrap();
                        let stopped = s.metadata.lock().unwrap().clone();
                        if !native_first {
                            s.settled(
                                dispatch,
                                BrowserNavigationSource::UserCommand,
                                false,
                                false,
                                false,
                            );
                        }
                        assert!(!s.completed(
                            2,
                            "https://example.test/slow".into(),
                            success,
                            status,
                            success.then_some(200),
                        ));
                        if native_first {
                            s.settled(
                                dispatch,
                                BrowserNavigationSource::UserCommand,
                                false,
                                false,
                                false,
                            );
                        }
                        let tab = s.metadata.lock().unwrap();
                        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
                        assert_eq!(tab.target, stopped.target);
                        assert_eq!(tab.url, stopped.url);
                        assert_eq!(tab.load, stopped.load);
                        drop(tab);
                        assert!(!s.host_interval);
                        let trace = s.trace.snapshot();
                        let outcome = trace.events.last().unwrap();
                        assert_eq!(outcome.navigation_sequence, Some(native_sequence));
                        assert_eq!(outcome.kind, expected_kind);
                        assert_eq!(
                            outcome.problem.as_ref().map(|problem| problem.safe_reason),
                            failure_reason
                        );
                        if let Some(problem) = &outcome.problem {
                            assert_eq!(problem.native_code, i64::from(status.0));
                            assert_eq!(problem.native_domain_class, BrowserNativeDomainClass::WebView2);
                        }
                        if expected_kind == BrowserNavigationEventKind::Cancelled {
                            assert_eq!(outcome.cancellation_reason, Some(reason));
                        } else {
                            assert!(outcome.cancellation_reason.is_none());
                        }
                        // A new explicit navigation ends the hold and owns its
                        // own completion; the old native result grants no control.
                        s.requested(
                            BrowserNavigationSource::UserCommand,
                            Some("https://example.test/successor".into()),
                            BrowserNavigationActionType::Other,
                        );
                        start(&mut s, 3, "https://example.test/successor", false);
                        assert!(s.completed(
                            3,
                            "https://example.test/successor".into(),
                            true,
                            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                            Some(200),
                        ));
                        assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
                    }
                }
            }
        }
    }
    #[test]
    fn stop_intention_without_acknowledged_cancel_does_not_hide_native_failure() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/failed", false);
        // Native control sets this intention before Core.Stop. Without its
        // acknowledgement, the native outcome remains the current load truth.
        s.cancellation_intent(s.receipt(), BrowserCancellationReason::AutomaticNavigationPaused)
            .unwrap();
        assert!(s.completed(
            1,
            "https://example.test/failed".into(),
            false,
            COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
            None,
        ));
        let tab = s.metadata.lock().unwrap();
        let load = tab.load.as_ref().unwrap();
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Failed);
        assert_eq!(load.phase, BrowserNavigationPhase::Failed);
        assert!(load.cancellation_reason.is_none());
        assert_eq!(
            load.problem.as_ref().unwrap().safe_reason,
            BrowserFailureReason::NetworkConnection
        );
    }
    #[test]
    fn actual_commit_after_acknowledged_stop_advances_document_without_revoking_cancel() {
        for reason in [
            BrowserCancellationReason::UserStop,
            BrowserCancellationReason::AutomaticNavigationPaused,
        ] {
            for error_page in [false, true] {
                let mut s = state();
                s.bootstrap = false;
                start(&mut s, 1, "https://example.test/slow", false);
                let stopped_receipt = s.receipt();
                s.stop_for(stopped_receipt, reason).unwrap();
                assert!(s.committed(1, "https://example.test/slow".into(), error_page));
                {
                    let tab = s.metadata.lock().unwrap();
                    let load = tab.load.as_ref().unwrap();
                    assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
                    assert_eq!(load.phase, BrowserNavigationPhase::Cancelled);
                    assert_eq!(load.cancellation_reason, Some(reason));
                    assert_eq!(tab.target.document_generation, 1);
                    assert_eq!(
                        load.content_state,
                        if error_page {
                            BrowserContentState::None
                        } else {
                            BrowserContentState::CurrentDocument
                        }
                    );
                    assert_eq!(
                        load.content_url.as_deref(),
                        (!error_page).then_some("https://example.test/slow")
                    );
                }
                assert!(!s.matches(stopped_receipt));
                assert!(!s.completed(
                    1,
                    "https://example.test/slow".into(),
                    false,
                    COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
                    None,
                ));
                let tab = s.metadata.lock().unwrap();
                let load = tab.load.as_ref().unwrap();
                assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
                assert_eq!(load.phase, BrowserNavigationPhase::Cancelled);
                assert!(load.problem.is_none());
                drop(tab);
                let trace = s.trace.snapshot();
                let commit = trace
                    .events
                    .iter()
                    .find(|event| event.kind == BrowserNavigationEventKind::Committed)
                    .unwrap();
                assert_eq!(commit.document_generation, Some(1));
                let outcome = trace.events.last().unwrap();
                assert_eq!(outcome.kind, BrowserNavigationEventKind::Failed);
                assert_eq!(outcome.document_generation, Some(1));
                assert_eq!(
                    outcome.problem.as_ref().unwrap().stage,
                    BrowserFailureStage::Committed
                );
            }
        }
    }
    #[test]
    fn cancelled_attempts_late_commit_and_completion_do_not_change_successor() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/old", false);
        s.stop(s.receipt()).unwrap();
        start(&mut s, 2, "https://example.test/current", false);
        s.committed(2, "https://example.test/current".into(), false);
        let successor = s.metadata.lock().unwrap().clone();
        assert!(!s.committed(1, "https://example.test/old".into(), false));
        assert!(!s.completed(
            1,
            "https://example.test/old".into(),
            false,
            COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
            None,
        ));
        let tab = s.metadata.lock().unwrap();
        assert_eq!(tab.target, successor.target);
        assert_eq!(tab.lifecycle, successor.lifecycle);
        assert_eq!(tab.url, successor.url);
        assert_eq!(tab.load, successor.load);
        drop(tab);
        assert!(s.completed(
            2,
            "https://example.test/current".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        ));
        assert_eq!(s.metadata.lock().unwrap().lifecycle, BrowserTabLifecycle::Ready);
    }
    #[test]
    fn stop_after_pause_preserves_confirmed_cancel_and_independent_late_failure() {
        for reason in [
            BrowserCancellationReason::AutomaticNavigationPaused,
            BrowserCancellationReason::NavigationRejected,
        ] {
            let mut s = state();
            s.bootstrap = false;
            start(&mut s, 1, "https://example.test/cancelled", false);
            let receipt = s.receipt();
            s.stop_for(receipt, reason).unwrap();
            // This is the actual Stop entry's pre-COM mutation, followed by
            // its acknowledgement. Neither may alter an existing cancel.
            s.cancellation_intent(receipt, BrowserCancellationReason::UserStop)
                .unwrap();
            s.stop(receipt).unwrap();
            assert!(!s.completed(
                1,
                "https://example.test/cancelled".into(),
                false,
                COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
                None,
            ));
            let tab = s.metadata.lock().unwrap();
            let load = tab.load.as_ref().unwrap();
            assert_eq!(tab.lifecycle, BrowserTabLifecycle::Stopped);
            assert_eq!(load.phase, BrowserNavigationPhase::Cancelled);
            assert_eq!(load.cancellation_reason, Some(reason));
            assert!(load.problem.is_none());
            drop(tab);
            let trace = s.trace.snapshot();
            let failure = trace.events.last().unwrap();
            assert_eq!(failure.kind, BrowserNavigationEventKind::Failed);
            assert_eq!(failure.problem.as_ref().unwrap().native_code, 9);
        }
    }
    #[test]
    fn replace_state_before_completion_does_not_relabel_http_response() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/request", false);
        s.committed(1, "https://example.test/request".into(), false);
        s.same_document("https://example.test/spa".into());
        s.completed(
            1,
            "https://example.test/spa".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        let trace = s.trace.snapshot();
        let response = trace
            .events
            .iter()
            .find(|event| event.kind == BrowserNavigationEventKind::Response)
            .unwrap();
        assert_eq!(
            response.url.as_deref(),
            Some("https://example.test/request")
        );
        assert_eq!(
            s.metadata
                .lock()
                .unwrap()
                .load
                .as_ref()
                .unwrap()
                .content_url
                .as_deref(),
            Some("https://example.test/spa")
        );
    }
    #[test]
    fn exact_before_unload_refusal_retains_document_and_cannot_cancel_successor() {
        let mut s = state();
        s.bootstrap = false;
        start(&mut s, 1, "https://example.test/retained", false);
        s.committed(1, "https://example.test/retained".into(), false);
        s.completed(
            1,
            "https://example.test/retained".into(),
            true,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
            Some(200),
        );
        let original_generation = s.metadata.lock().unwrap().target.document_generation;
        let refused = s.requested(
            BrowserNavigationSource::AgentCommand,
            Some("https://example.test/rejected".into()),
            BrowserNavigationActionType::Other,
        );
        assert!(s.reject_before_unload(refused));
        s.settled_rejection(refused, BrowserNavigationSource::AgentCommand);
        {
            let tab = s.metadata.lock().unwrap();
            let load = tab.load.as_ref().unwrap();
            assert_eq!(tab.target.document_generation, original_generation);
            assert_eq!(load.phase, BrowserNavigationPhase::Cancelled);
            assert_eq!(
                load.cancellation_reason,
                Some(BrowserCancellationReason::NavigationRejected)
            );
            assert_eq!(load.content_state, BrowserContentState::RetainedDocument);
            assert_eq!(
                load.content_url.as_deref(),
                Some("https://example.test/retained")
            );
            assert!(load.problem.is_none());
        }
        assert!(!s.host_interval);
        let successor = s.requested(
            BrowserNavigationSource::UserCommand,
            Some("https://example.test/successor".into()),
            BrowserNavigationActionType::Other,
        );
        assert!(!s.reject_before_unload(refused));
        assert!(s.requested_matches(successor));
        start(&mut s, 2, "https://example.test/successor", false);
        assert!(!s.reject_before_unload(successor));
        let trace = s.trace.snapshot();
        assert!(trace.events.iter().any(|event| event.kind
            == BrowserNavigationEventKind::Cancelled
            && event.dispatch_sequence == Some(refused.submission)
            && event.cancellation_reason == Some(BrowserCancellationReason::NavigationRejected)
            && event.navigation_sequence.is_none()));
        assert!(
            !trace
                .events
                .iter()
                .any(|event| event.kind == BrowserNavigationEventKind::Failed)
        );
        let ordinary = s.requested(
            BrowserNavigationSource::UserCommand,
            Some("https://example.test/network-error".into()),
            BrowserNavigationActionType::Other,
        );
        s.settled(
            ordinary,
            BrowserNavigationSource::UserCommand,
            true,
            false,
            false,
        );
        assert_eq!(
            s.metadata.lock().unwrap().load.as_ref().unwrap().phase,
            BrowserNavigationPhase::Failed
        );
    }
    #[test]
    fn explicit_stay_cancellation_keeps_independent_dns_and_tls_failure_evidence() {
        for (error, reason) in [
            ("net::ERR_NAME_NOT_RESOLVED", BrowserFailureReason::Dns),
            ("net::ERR_CERT_AUTHORITY_INVALID", BrowserFailureReason::Tls),
        ] {
            let mut s = state();
            s.bootstrap = false;
            start(&mut s, 1, "https://example.test/old-document", false);
            s.committed(1, "https://example.test/old-document".into(), false);
            s.completed(
                1,
                "https://example.test/old-document".into(),
                true,
                COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                Some(200),
            );
            let requested = s.requested(
                BrowserNavigationSource::UserCommand,
                Some("https://example.test/failed".into()),
                BrowserNavigationActionType::Other,
            );
            assert!(s.reject_before_unload(requested));
            s.settled_problem(
                requested,
                BrowserNavigationSource::UserCommand,
                true,
                false,
                false,
                Some(protocol_problem(Some(error))),
            );
            assert!(!s.host_interval);
            assert_eq!(
                s.metadata.lock().unwrap().load.as_ref().unwrap().phase,
                BrowserNavigationPhase::Cancelled
            );
            let trace = s.trace.snapshot();
            let failed = trace
                .events
                .iter()
                .find(|event| {
                    event.kind == BrowserNavigationEventKind::Failed
                        && event.dispatch_sequence == Some(requested.submission)
                })
                .unwrap();
            assert_eq!(failed.problem.as_ref().unwrap().safe_reason, reason);
            assert_eq!(
                failed.problem.as_ref().unwrap().native_domain_class,
                BrowserNativeDomainClass::Other
            );
            assert!(failed.navigation_sequence.is_none());
            assert!(
                !trace
                    .events
                    .iter()
                    .any(|event| event.kind == BrowserNavigationEventKind::Cancelled
                        && event.dispatch_sequence == Some(requested.submission))
            );
        }
    }
}
