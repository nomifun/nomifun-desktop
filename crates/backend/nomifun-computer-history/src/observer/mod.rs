//! Observer loop: poll the platform backend for an [`ActivitySample`], run it
//! through the observation rules + redaction, and merge it into activity
//! segments with a dedup state machine (identical consecutive samples extend
//! the open segment; any change closes it and opens a new one).
//!
//! The loop never spawns or kills processes — it only polls, so the
//! process-runtime boundary is unaffected.

pub mod stub;

#[cfg(target_os = "macos")]
pub mod macos;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomifun_common::AppError;
use tokio_util::sync::CancellationToken;

use crate::config::MAX_FIELD_CHARS;
use crate::rules::{self, ObservationSettings};
use crate::store::{ActivitySegment, ActivityStore};

/// One raw observation sample from the platform. `None` from a backend means
/// "nothing observable right now" (locked screen, no permission, unsupported
/// platform) — the loop then closes the open segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySample {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub browser_title: Option<String>,
    /// True when macOS Secure Input is active: captured text is withheld.
    pub secure_input: bool,
    /// True when the focused browser window is private browsing.
    pub private_browsing: bool,
}

impl ActivitySample {
    /// Dedup key: the identity of the focused (app, window, URL) context. Two
    /// samples with the same key belong to the same segment.
    fn dedup_key(&self) -> (String, Option<String>, Option<String>, Option<String>) {
        (
            self.app_name.clone(),
            self.bundle_id.clone(),
            self.window_title.clone(),
            self.browser_url.clone(),
        )
    }
}

/// Platform observation backend. The macOS implementation is delivered by a
/// follow-up task; every other platform compiles to [`stub::StubBackend`].
#[async_trait]
pub trait ObserverBackend: Send + Sync {
    /// Sample the current foreground activity, or `None` when nothing is
    /// observable.
    async fn current_sample(&self) -> Option<ActivitySample>;
    /// macOS permission preflight (Accessibility / Apple Events). Non-macOS
    /// backends report `PermissionState::NotRequired`.
    fn permission_state(&self) -> crate::service::PermissionState {
        crate::service::PermissionState::NotRequired
    }
}

/// The platform's production observer backend: real NSWorkspace/AX sampling
/// on macOS, the no-op stub everywhere else. The host assembles exactly one
/// through this so feature gating never leaks platform cfg into app wiring.
pub fn default_backend() -> Arc<dyn ObserverBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacosBackend)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(stub::StubBackend)
    }
}

/// Mutable state of the segment-merge state machine, kept between ticks.
#[derive(Debug, Default)]
pub struct DedupState {
    open_segment: Option<ActivitySegment>,
    open_key: Option<(String, Option<String>, Option<String>, Option<String>)>,
}

impl DedupState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Is a segment currently open?
    pub fn is_open(&self) -> bool {
        self.open_segment.is_some()
    }

    /// In-memory reference to the open segment's event id (for status).
    pub fn open_event_id(&self) -> Option<&str> {
        self.open_segment.as_ref().map(|s| s.event_id.as_str())
    }

    /// Process one tick: either extend the open segment, close + open a new
    /// one, or close it. Returns segments that must be persisted.
    pub fn observe(
        &mut self,
        sample: Option<ActivitySample>,
        user_id: &str,
        settings: &ObservationSettings,
        now_ms: i64,
    ) -> Vec<ActivitySegment> {
        // Rules gate: hardcoded exclusions, private browsing, app + URL
        // policy. Suppressed samples close the open segment (disposition
        // `suppressed` / `blocked` in Codex terms).
        let allowed = sample.as_ref().is_some_and(|sample| {
            if sample.secure_input {
                return false;
            }
            if sample.private_browsing && !settings.observe_private_browsing {
                return false;
            }
            let bundle_ok = sample
                .bundle_id
                .as_deref()
                .map_or(true, |b| rules::observe_application(settings, b));
            let url_ok = sample
                .browser_url
                .as_deref()
                .and_then(url_domain)
                .map_or(true, |d| rules::observe_url_domain(settings, &d));
            bundle_ok && url_ok
        });

        let sample = allowed.then(|| sample.expect("allowed implies sample"));
        let open = self.open_segment.take();
        match (open, sample) {
            (Some(mut open), Some(next)) if self.open_key.as_ref() == Some(&next.dedup_key()) => {
                open.ended_at_ms = now_ms;
                self.open_segment = Some(open);
                Vec::new()
            }
            (previous_open, next) => {
                let mut closed = Vec::new();
                if let Some(mut open) = previous_open {
                    open.ended_at_ms = now_ms;
                    closed.push(open);
                }
                self.open_key = next.as_ref().map(ActivitySample::dedup_key);
                if let Some(next) = next {
                    self.open_segment = Some(ActivitySegment {
                        event_id: nomifun_common::generate_id(),
                        user_id: user_id.to_string(),
                        started_at_ms: now_ms,
                        ended_at_ms: now_ms,
                        app_name: truncate(&next.app_name),
                        bundle_id: next.bundle_id.as_deref().map(truncate),
                        window_title: next.window_title.as_deref().map(rules::redact_captured_text).map(|t| truncate(&t)),
                        // URLs carry secrets too (query-string tokens, basic-auth
                        // userinfo) — redact like titles before persistence.
                        browser_url: next.browser_url.as_deref().map(rules::redact_captured_text).map(|t| truncate(&t)),
                        browser_title: next.browser_title.as_deref().map(rules::redact_captured_text).map(|t| truncate(&t)),
                        source: if next.browser_url.is_some() {
                            "browser"
                        } else {
                            "foreground"
                        }
                        .to_string(),
                        captured_at_ms: now_ms,
                    });
                } else {
                    self.open_key = None;
                }
                closed
            }
        }
    }

    /// Close any open segment without a new sample (shutdown, pause, idle).
    pub fn close(&mut self, now_ms: i64) -> Vec<ActivitySegment> {
        self.observe(None, "", &ObservationSettings::default(), now_ms)
    }
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= MAX_FIELD_CHARS {
        return value.to_string();
    }
    value.chars().take(MAX_FIELD_CHARS).collect()
}

/// Extract the host part of a URL for domain rules (no scheme/path).
pub fn url_domain(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Spawn the supervised observation loop. Returns the cancellation token that
/// stops it (the caller owns shutdown; the loop closes the open segment on
/// exit). Panics in the loop body are contained per-tick and restart with
/// capped exponential backoff, mirroring the browser-platform loop pattern in
/// nomifun-app/src/services.rs.
pub fn spawn_observer_loop(
    store: Arc<ActivityStore>,
    backend: Arc<dyn ObserverBackend>,
    user_id: String,
    settings: ObservationSettings,
    poll_interval_ms: u64,
    retention_days: u32,
) -> CancellationToken {
    let token = CancellationToken::new();
    let shutdown = token.clone();
    tokio::spawn(async move {
        let mut state = DedupState::new();
        let mut failure_count: u32 = 0;
        let mut interval = tokio::time::interval(Duration::from_millis(
            poll_interval_ms.max(200),
        ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut prune_interval =
            tokio::time::interval(Duration::from_secs(crate::config::EVENT_PRUNE_INTERVAL_SECONDS));
        prune_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    for segment in state.close(nomifun_common::now_ms()) {
                        if let Err(error) = store.append_segments(&[segment]).await {
                            tracing::warn!(error = %error, "computer history failed to flush open segment on shutdown");
                        }
                    }
                    break;
                }
                _ = prune_interval.tick() => {
                    let cutoff = nomifun_common::now_ms()
                        - i64::from(retention_days) * 24 * 60 * 60 * 1000;
                    match crate::retention::prune(&store, cutoff).await {
                        Ok(removed) => tracing::debug!(removed, "computer history retention prune"),
                        Err(error) => tracing::warn!(error = %error, "computer history retention prune failed"),
                    }
                }
                _ = interval.tick() => {
                    let tick = std::panic::AssertUnwindSafe(run_tick(
                        &store, backend.as_ref(), &user_id, &settings, &mut state,
                    ));
                    // Contain a panicking tick so a platform hiccup restarts
                    // the loop instead of killing it (services.rs pattern).
                    let result = futures::FutureExt::catch_unwind(tick).await;
                    match result {
                        Ok(Ok(())) => failure_count = 0,
                        Ok(Err(error)) => {
                            failure_count = (failure_count + 1).min(6);
                            tracing::warn!(error = %error, failure_count, loop_name = "computer_history", "observer tick failed");
                        }
                        Err(_) => {
                            failure_count = (failure_count + 1).min(6);
                            tracing::error!(failure_count, loop_name = "computer_history", restart_delay_ms = 0, "observer tick panicked");
                        }
                    }
                    if failure_count > 0 {
                        let delay = 2u64.pow(failure_count) * poll_interval_ms.max(200) / 2;
                        tokio::time::sleep(Duration::from_millis(delay.min(60_000))).await;
                    }
                }
            }
        }
    });
    token
}

async fn run_tick(
    store: &ActivityStore,
    backend: &dyn ObserverBackend,
    user_id: &str,
    fallback_settings: &ObservationSettings,
    state: &mut DedupState,
) -> Result<(), AppError> {
    // Re-read the observation settings every tick so allow/blocklist edits
    // and default-behavior flips take effect while the recorder is running
    // (a settings write was invisible until restart — MAJOR-2). A read
    // failure keeps the last-known settings rather than sampling unfiltered.
    let settings = store.observation_settings().await.unwrap_or_else(|error| {
        tracing::warn!(error = %error, "computer history settings re-read failed; using last known");
        fallback_settings.clone()
    });
    let sample = backend.current_sample().await;
    let now = nomifun_common::now_ms();
    let segments = state.observe(sample, user_id, &settings, now);
    store.append_segments(&segments).await?;
    // The open segment's end timestamp moves on every tick; flush it so a
    // crash never loses more than one tick of duration.
    if let Some(event_id) = state.open_event_id() {
        store.close_open_segment(event_id, now).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{ActivityRule, DefaultBehavior, RuleAction, RuleScope};
    use crate::service::PermissionState;

    fn sample(app: &str, title: &str) -> Option<ActivitySample> {
        Some(ActivitySample {
            app_name: app.into(),
            bundle_id: Some(format!("com.example.{app}")),
            window_title: Some(title.into()),
            browser_url: None,
            browser_title: None,
            secure_input: false,
            private_browsing: false,
        })
    }

    struct FixedBackend(Option<ActivitySample>);

    #[async_trait]
    impl ObserverBackend for FixedBackend {
        async fn current_sample(&self) -> Option<ActivitySample> {
            self.0.clone()
        }
        fn permission_state(&self) -> PermissionState {
            PermissionState::Granted
        }
    }

    fn settings() -> ObservationSettings {
        ObservationSettings::default()
    }

    #[test]
    fn identical_samples_merge_into_one_segment() {
        let mut state = DedupState::new();
        assert!(state.observe(sample("Safari", "Docs"), "u", &settings(), 100).is_empty());
        assert!(state.observe(sample("Safari", "Docs"), "u", &settings(), 200).is_empty());
        let closed = state.close(300);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].started_at_ms, 100);
        assert_eq!(closed[0].ended_at_ms, 300);
        assert_eq!(closed[0].source, "foreground");
    }

    #[test]
    fn changed_context_closes_and_opens() {
        let mut state = DedupState::new();
        state.observe(sample("Safari", "Docs"), "u", &settings(), 100);
        let closed = state.observe(sample("Terminal", "zsh"), "u", &settings(), 200);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].app_name, "Safari");
        assert_eq!(closed[0].ended_at_ms, 200);
        assert_eq!(state.open_event_id().map(str::len), Some(36));
    }

    #[test]
    fn none_sample_closes_open_segment() {
        let mut state = DedupState::new();
        state.observe(sample("Safari", "Docs"), "u", &settings(), 100);
        let closed = state.observe(None, "u", &settings(), 200);
        assert_eq!(closed.len(), 1);
        assert!(!state.is_open());
    }

    #[test]
    fn blocked_samples_are_suppressed() {
        let mut settings = ObservationSettings {
            default_application_behavior: DefaultBehavior::DoNotObserve,
            ..ObservationSettings::default()
        };
        settings.allowlist.push(
            ActivityRule {
                id: nomifun_common::generate_id(),
                scope: RuleScope::Application,
                bundle_id: Some("com.example.Safari".into()),
                url_domain: None,
                action: RuleAction::Capture,
            }
            .validated()
            .unwrap(),
        );
        let mut state = DedupState::new();
        // Not on the allowlist → suppressed.
        assert!(state.observe(sample("Terminal", "zsh"), "u", &settings, 100).is_empty());
        assert!(!state.is_open());
        // Allowlisted → segment opens.
        assert!(state.observe(sample("Safari", "Docs"), "u", &settings, 200).is_empty());
        assert!(state.is_open());
    }

    #[test]
    fn secure_input_and_private_browsing_suppress() {
        let mut s = sample("Safari", "password").unwrap();
        s.secure_input = true;
        let mut state = DedupState::new();
        assert!(state.observe(Some(s), "u", &settings(), 100).is_empty());
        assert!(!state.is_open());

        let mut p = sample("Safari", "private").unwrap();
        p.private_browsing = true;
        assert!(state.observe(Some(p), "u", &settings(), 200).is_empty());
        assert!(!state.is_open());
    }

    #[test]
    fn hardcoded_exclusions_are_never_observed() {
        let mut s = sample("WindowManager", "spacer").unwrap();
        s.bundle_id = Some("com.apple.WindowManager".into());
        let mut state = DedupState::new();
        assert!(state.observe(Some(s), "u", &settings(), 100).is_empty());
        assert!(!state.is_open());
    }

    #[test]
    fn titles_are_redacted_before_persistence_state() {
        let mut s = sample("Notes", "token: supersecretvalue123").unwrap();
        s.secure_input = false;
        let mut state = DedupState::new();
        state.observe(Some(s), "u", &settings(), 100);
        let open = state.close(200);
        assert!(!open[0].window_title.as_deref().unwrap().contains("supersecretvalue123"));
    }

    #[test]
    fn browser_urls_are_redacted_before_persistence() {
        // MINOR-2 regression: a URL with a query-string secret must not land
        // in the store with the secret intact.
        let mut s = sample("Safari", "docs").unwrap();
        s.browser_url = Some("https://docs.example.com/?api_key=sk-supersecretvalue123".into());
        let mut state = DedupState::new();
        state.observe(Some(s), "u", &settings(), 100);
        let open = state.close(200);
        let url = open[0].browser_url.as_deref().unwrap();
        assert!(!url.contains("supersecretvalue123"), "secret leaked in url: {url}");
        // Structure survives (host still readable) so URL domain rules keep
        // working on the persisted value.
        assert!(url.contains("docs.example.com"));
    }

    #[tokio::test]
    async fn loop_picks_up_settings_changes_while_running() {
        // MAJOR-2 regression: a blocklist write must affect the RUNNING loop
        // without a restart (run_tick re-reads settings per tick).
        let store = Arc::new(ActivityStore::open_memory().await.unwrap());
        // Start with everything observed.
        let mut current = ObservationSettings::default();
        // A backend that always samples the same app; the loop decides.
        struct AlwaysBackend;
        #[async_trait]
        impl ObserverBackend for AlwaysBackend {
            async fn current_sample(&self) -> Option<ActivitySample> {
                Some(ActivitySample {
                    app_name: "Terminal".into(),
                    bundle_id: Some("com.example.Terminal".into()),
                    window_title: None,
                    browser_url: None,
                    browser_title: None,
                    secure_input: false,
                    private_browsing: false,
                })
            }
        }
        // Write initial settings (observe default), spawn the loop.
        store.set_observation_settings(&current).await.unwrap();
        let token = spawn_observer_loop(
            store.clone(),
            Arc::new(AlwaysBackend),
            "01900000-0000-7000-8000-000000000001".into(),
            current.clone(),
            200,
            30,
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        // Now flip the default to do_not_observe while the loop runs.
        current.default_application_behavior = DefaultBehavior::DoNotObserve;
        store.set_observation_settings(&current).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        token.cancel();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let segments = store
            .query_segments(&Default::default())
            .await
            .unwrap();
        // Segments created BEFORE the flip are expected; after the flip the
        // app must be suppressed, so no segment may have started after the
        // write. (The pre-flitch open segment legitimately exists.)
        let flip_at = nomifun_common::now_ms() - 600;
        assert!(
            segments.iter().all(|s| s.started_at_ms < flip_at),
            "settings change while running was ignored: {segments:?}"
        );
    }

    #[test]
    fn browser_samples_use_browser_source_and_domain_rules() {
        let mut settings = ObservationSettings {
            default_url_behavior: DefaultBehavior::DoNotObserve,
            ..ObservationSettings::default()
        };
        settings.allowlist.push(
            ActivityRule {
                id: nomifun_common::generate_id(),
                scope: RuleScope::Url,
                bundle_id: None,
                url_domain: Some("docs.example.com".into()),
                action: RuleAction::Capture,
            }
            .validated()
            .unwrap(),
        );
        let mut blocked = sample("Safari", "fun").unwrap();
        blocked.browser_url = Some("https://games.example.com/play".into());
        let mut allowed = sample("Safari", "docs").unwrap();
        allowed.browser_url = Some("https://docs.example.com/guide".into());
        let mut state = DedupState::new();
        assert!(state.observe(Some(blocked), "u", &settings, 100).is_empty());
        assert!(!state.is_open());
        assert!(state.observe(Some(allowed), "u", &settings, 200).is_empty());
        let closed = state.close(300);
        assert_eq!(closed[0].source, "browser");
        assert_eq!(closed[0].browser_url.as_deref(), Some("https://docs.example.com/guide"));
    }

    #[test]
    fn url_domain_extracts_host() {
        assert_eq!(
            url_domain("https://user:pw@Docs.Example.com:8443/a/b?q=1#frag"),
            Some("docs.example.com".into())
        );
        assert_eq!(url_domain("example.com/x"), Some("example.com".into()));
        assert_eq!(url_domain(""), None);
    }

    #[tokio::test]
    async fn backend_trait_default_permission() {
        let backend = FixedBackend(None);
        assert_eq!(backend.current_sample().await, None);
        let stub = stub::StubBackend;
        assert_eq!(stub.permission_state(), PermissionState::NotRequired);
        assert_eq!(stub.current_sample().await, None);
    }

    #[tokio::test]
    async fn loop_flushes_and_shuts_down() {
        let store = Arc::new(ActivityStore::open_memory().await.unwrap());
        let token = spawn_observer_loop(
            store.clone(),
            Arc::new(FixedBackend(None)),
            "01900000-0000-7000-8000-000000000001".into(),
            settings(),
            200,
            30,
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        token.cancel();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(store.query_segments(&Default::default()).await.unwrap().is_empty());
    }
}
