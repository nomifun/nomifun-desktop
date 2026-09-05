//! `ComputerHistoryService`: the facade over store + observer that gateway
//! capabilities and the agent seam talk to. Start/stop/pause/resume/status,
//! with the pause model from the Codex spec (`thirty_minutes` / `one_hour` /
//! `until_tomorrow`, persisted in the feature-local `feature_config` table).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use nomifun_common::{AppError, now_ms};

use crate::config::ComputerHistoryConfig;
use crate::observer::{ObserverBackend, spawn_observer_loop, url_domain};
use crate::rules::{ObservationSettings, observe_application, observe_url_domain};
use crate::store::{ActivityStore, StorageStatus};

/// Recorder state. Wire values match the Codex `SkysightState` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecorderState {
    Stopped,
    Running,
    Paused,
}

/// Pause duration choices from the Codex `SkysightPauseDuration` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseDuration {
    ThirtyMinutes,
    OneHour,
    UntilTomorrow,
}

impl PauseDuration {
    /// Milliseconds to pause for, relative to `now_ms`.
    pub fn duration_ms(self, now_ms: i64) -> i64 {
        match self {
            Self::ThirtyMinutes => 30 * 60 * 1000,
            Self::OneHour => 60 * 60 * 1000,
            Self::UntilTomorrow => {
                // Next local midnight (until_tomorrow semantics), minimum 1h.
                use chrono::TimeZone;
                let now = chrono::Local.timestamp_millis_opt(now_ms).single();
                match now.map(|now| {
                    let midnight = (now.date_naive().succ_opt())
                        .and_then(|date| chrono::Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).single());
                    midnight.map_or(60 * 60 * 1000, |midnight| {
                        (midnight.timestamp_millis() - now_ms).max(60 * 60 * 1000)
                    })
                }) {
                    Some(ms) => ms,
                    None => 24 * 60 * 60 * 1000,
                }
            }
        }
    }
}

/// macOS permission preflight placeholder. Task #4 fills in the real TCC
/// state; the enum shape is fixed now so UI/settings code can bind to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Unknown,
    NotRequired,
    Granted,
    Denied,
}

/// Status response, mirroring the Codex `computer_history_status` fields plus
/// nomifun additions (storage usage, permission state).
#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatus {
    pub state: RecorderState,
    pub enabled: bool,
    /// Resumes at this epoch-ms when paused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_until_ms: Option<i64>,
    /// Raw event-stream root: `{data_dir}/segments`.
    pub event_stream_root_path: String,
    /// Current (open) raw segment directory, when the loop is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_segment_path: Option<String>,
    pub permission: PermissionState,
    pub storage: StorageStatus,
    pub observation: ObservationSettings,
}

pub struct ComputerHistoryService {
    store: Arc<ActivityStore>,
    /// Read-only analytics over the local Messages database (macOS only).
    /// `None` on non-macOS hosts or when chat.db cannot be opened (missing or
    /// TCC-denied); every chat query then reports itself unavailable instead
    /// of failing the whole service.
    chat_analytics: Option<crate::chat_analytics::ChatAnalytics>,
    backend: Arc<dyn ObserverBackend>,
    config: ComputerHistoryConfig,
    user_id: String,
    shutdown: std::sync::Mutex<Option<CancellationToken>>,
    /// Serialize start/stop/pause mutations.
    mutation_lock: tokio::sync::Mutex<()>,
    /// Runtime mirror of the persisted `CONFIG_KEY_ENABLED` toggle. Atomic so
    /// the hot status path never awaits the store.
    enabled: std::sync::atomic::AtomicBool,
}

impl ComputerHistoryService {
    pub async fn open(
        config: ComputerHistoryConfig,
        backend: Arc<dyn ObserverBackend>,
        user_id: String,
    ) -> Result<Self, AppError> {
        config.validate()?;
        let store = Arc::new(ActivityStore::open(&config.data_dir).await?);
        // Domain-local: a closed or TCC-denied Messages database removes the
        // chat-analytics surface only, never the recorder itself.
        let chat_analytics = crate::chat_analytics::ChatAnalytics::open_default()
            .await
            .inspect_err(|error| {
                tracing::info!(
                    error = %error,
                    "computer history: chat analytics unavailable this boot"
                );
            })
            .ok();
        // The persisted toggle wins over the static config so an explicit
        // enable/disable survives a restart (Codex semantics).
        let enabled = store
            .get_config(crate::store::CONFIG_KEY_ENABLED)
            .await
            .ok()
            .flatten()
            .map(|raw| raw == "true")
            .unwrap_or(config.enabled);
        Ok(Self {
            store,
            chat_analytics,
            backend,
            config,
            user_id,
            shutdown: std::sync::Mutex::new(None),
            mutation_lock: tokio::sync::Mutex::new(()),
            enabled: std::sync::atomic::AtomicBool::new(enabled),
        })
    }

    #[cfg(test)]
    pub async fn open_memory(
        config: ComputerHistoryConfig,
        backend: Arc<dyn ObserverBackend>,
        user_id: String,
    ) -> Result<Self, AppError> {
        let enabled_default = config.enabled;
        config.validate()?;
        let store = Arc::new(ActivityStore::open_memory().await?);
        Ok(Self {
            store,
            chat_analytics: None,
            backend,
            config,
            user_id,
            shutdown: std::sync::Mutex::new(None),
            mutation_lock: tokio::sync::Mutex::new(()),
            enabled: std::sync::atomic::AtomicBool::new(enabled_default),
        })
    }

    pub fn store(&self) -> &Arc<ActivityStore> {
        &self.store
    }

    /// Read-only chat.db analytics, when the Messages database is reachable.
    pub fn chat_analytics(
        &self,
    ) -> Option<&crate::chat_analytics::ChatAnalytics> {
        self.chat_analytics.as_ref()
    }

    /// Availability snapshot for status surfaces (`available` + `db_path`);
    /// `None` when chat analytics is absent (non-macOS, closed, TCC-denied).
    pub async fn chat_analytics_status(&self) -> Option<crate::chat_analytics::ChatAnalyticsStatus> {
        match &self.chat_analytics {
            Some(analytics) => Some(analytics.status().await),
            None => None,
        }
    }

    fn event_stream_root(&self) -> PathBuf {
        self.config.data_dir.join("segments")
    }

    fn enabled_in_store(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Master switch (Codex settings toggle). Enabling starts the observer
    /// immediately when the macOS permission is granted; a denied grant keeps
    /// the recorder stopped so `status` reports `permission: "denied"` +
    /// `state: "stopped"` and the UI can prompt. Disabling always stops.
    pub async fn set_enabled(&self, enabled: bool) -> Result<(), AppError> {
        self.enabled
            .store(enabled, std::sync::atomic::Ordering::Release);
        self.store
            .set_config(
                crate::store::CONFIG_KEY_ENABLED,
                if enabled { "true" } else { "false" },
            )
            .await?;
        if enabled {
            if self.backend.permission_state() == PermissionState::Denied {
                tracing::warn!("computer history enabled but Accessibility permission denied; recorder stays stopped");
                return Ok(());
            }
            self.start().await
        } else {
            self.stop().await
        }
    }

    async fn paused_until(&self) -> Option<i64> {        self.store
            .get_config(crate::store::CONFIG_KEY_PAUSE_UNTIL)
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.parse::<i64>().ok())
            .filter(|until| *until > now_ms())
    }

    /// Start the observation loop (idempotent). Fails with the Codex-equivalent
    /// message when the feature toggle is off.
    pub async fn start(&self) -> Result<(), AppError> {
        {
            // Scoped so the std-MutexGuard-backed guard never crosses an
            // await: `mutation_lock` is a tokio Mutex but the guard would
            // still be held across IO, which makes the future non-Send for
            // sink callers.
            let _guard = self.mutation_lock.lock().await;
            if !self.enabled_in_store() {
                return Err(AppError::BadRequest(
                    "Computer History is stopped. Enable Computer History in Codex Settings first."
                        .into(),
                ));
            }
            // The std `shutdown` guard must never cross an await (it is not
            // Send, and holding it across IO would make this future non-Send
            // for sink callers). Take the slot out, await, then put it back.
            let slot = self.shutdown.lock().expect("shutdown mutex poisoned").take();
            if slot.is_some() {
                *self.shutdown.lock().expect("shutdown mutex poisoned") = slot;
                return Ok(());
            }
            drop(slot);
            let token = spawn_observer_loop(
                self.store.clone(),
                self.backend.clone(),
                self.user_id.clone(),
                self.store.observation_settings().await?,
                self.config.poll_interval_ms,
                self.config.retention_days,
            );
            self.store
                .set_config(crate::store::CONFIG_KEY_PAUSE_UNTIL, "")
                .await?;
            *self.shutdown.lock().expect("shutdown mutex poisoned") = Some(token);
        }
        tracing::info!("computer history observation started");
        Ok(())
    }

    /// Stop the loop and flush the open segment (idempotent).
    pub async fn stop(&self) -> Result<(), AppError> {
        let _guard = self.mutation_lock.lock().await;
        let token = {
            let mut shutdown = self.shutdown.lock().expect("shutdown mutex poisoned");
            shutdown.take()
        };
        if let Some(token) = token {
            token.cancel();
            // Give the loop one tick to flush the open segment.
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        tracing::info!("computer history observation stopped");
        Ok(())
    }

    /// Temporarily pause without disabling; the pause survives restarts
    /// (persisted `pause_until` timestamp, Codex `skysight.pauseResumeDate`).
    pub async fn pause(&self, duration: PauseDuration) -> Result<i64, AppError> {
        let until = {
            let _guard = self.mutation_lock.lock().await;
            if !self.enabled_in_store() {
                return Err(AppError::BadRequest(
                    "Computer History is stopped. Enable Computer History in Codex Settings first."
                        .into(),
                ));
            }
            let now = now_ms();
            let until = now + duration.duration_ms(now);
            self.store
                .set_config(crate::store::CONFIG_KEY_PAUSE_UNTIL, &until.to_string())
                .await?;
            until
        };
        // Stop sampling while paused; the open segment is closed by shutdown.
        let token = {
            let mut shutdown = self.shutdown.lock().expect("shutdown mutex poisoned");
            shutdown.take()
        };
        if let Some(token) = token {
            token.cancel();
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        tracing::info!(paused_until_ms = until, "computer history paused");
        Ok(until)
    }

    /// Resume a paused recorder.
    pub async fn resume(&self) -> Result<(), AppError> {
        {
            // Scoped so the std MutexGuard never crosses an await (it is not
            // Send and would poison the sink's future).
            let _guard = self.mutation_lock.lock().await;
            self.store
                .set_config(crate::store::CONFIG_KEY_PAUSE_UNTIL, "")
                .await?;
        }
        self.start().await
    }

    /// Current status, including paths, permission and storage usage.
    pub async fn status(&self) -> Result<ServiceStatus, AppError> {
        let paused_until = self.paused_until().await;
        let running = {
            let shutdown = self.shutdown.lock().expect("shutdown mutex poisoned");
            shutdown.is_some()
        };
        let state = if running {
            RecorderState::Running
        } else if paused_until.is_some() {
            RecorderState::Paused
        } else {
            RecorderState::Stopped
        };
        let current_segment = running.then(|| {
            // Segment dirs are UTC 10-minute boundaries (inferred from Codex).
            let now = now_ms();
            let boundary = now - (now % 600_000);
            let name = chrono::DateTime::from_timestamp_millis(boundary)
                .unwrap_or_default()
                .format("%Y-%m-%dT%H-%M-00Z");
            format!("{}/{}", self.event_stream_root().display(), name)
        });
        let storage = self.store.storage_status(self.config.retention_days).await?;
        let observation = self.store.observation_settings().await?;
        Ok(ServiceStatus {
            state,
            enabled: self.enabled_in_store(),
            paused_until_ms: paused_until,
            event_stream_root_path: self.event_stream_root().display().to_string(),
            current_segment_path: current_segment,
            permission: self.backend.permission_state(),
            storage,
            observation,
        })
    }

    /// Replace the whole observation settings object (Codex semantics).
    pub async fn update_observation_settings(
        &self,
        settings: &ObservationSettings,
    ) -> Result<(), AppError> {
        self.store.set_observation_settings(settings).await
    }

    pub async fn observation_settings(&self) -> Result<ObservationSettings, AppError> {
        self.store.observation_settings().await
    }

    /// Would the given app/url pair be observed under the current settings?
    /// Exposed for settings UI previews and the gateway status surface.
    pub async fn would_observe(&self, bundle_id: &str, url: Option<&str>) -> Result<bool, AppError> {
        let settings = self.store.observation_settings().await?;
        Ok(observe_application(&settings, bundle_id)
            && url.map_or(true, |url| {
                url_domain(url).map_or(true, |d| observe_url_domain(&settings, &d))
            }))
    }

    /// Clear history. `scope` mirrors the Codex clear scopes: `all`,
    /// `last_ten_minutes`, `last_hour`, `last_day`, `interval`.
    pub async fn clear_history(&self, scope: &str, start_ms: i64, end_ms: i64) -> Result<u64, AppError> {
        let now = now_ms();
        let cutoff = match scope {
            "all" => return self.store.purge_all().await,
            "last_ten_minutes" => now - 10 * 60 * 1000,
            "last_hour" => now - 60 * 60 * 1000,
            "last_day" => now - 24 * 60 * 60 * 1000,
            "interval" => {
                if start_ms >= end_ms {
                    return Err(AppError::BadRequest(
                        "interval start must be earlier than interval end".into(),
                    ));
                }
                start_ms
            }
            other => {
                return Err(AppError::BadRequest(format!(
                    "unsupported clear history scope: {other}"
                )));
            }
        };
        let end = end_ms.min(now);
        // Segments fully inside [cutoff, end] are removed; purge_before is the
        // retention helper (deletes everything ending before a timestamp) and
        // `purge_ended_between` handles the scoped windows.
        let removed = self
            .store
            .purge_ended_between(cutoff, end)
            .await?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::ActivitySample;

    struct NeverBackendStub;

    #[async_trait::async_trait]
    impl ObserverBackend for NeverBackendStub {
        async fn current_sample(&self) -> Option<ActivitySample> {
            None
        }
    }

    fn config() -> ComputerHistoryConfig {
        ComputerHistoryConfig::default()
    }

    fn service_with(enable: bool) -> ComputerHistoryConfig {
        ComputerHistoryConfig {
            enabled: enable,
            ..config()
        }
    }

    #[tokio::test]
    async fn start_requires_enabled_feature() {
        let service = ComputerHistoryService::open_memory(
            service_with(false),
            Arc::new(NeverBackendStub),
            "01900000-0000-7000-8000-000000000001".into(),
        )
        .await
        .unwrap();
        let error = service.start().await.unwrap_err().to_string();
        assert_eq!(
            error,
            "Bad request: Computer History is stopped. Enable Computer History in Codex Settings first."
        );
        let status = service.status().await.unwrap();
        assert_eq!(status.state, RecorderState::Stopped);
        assert!(!status.enabled);
    }

    #[tokio::test]
    async fn start_stop_and_status_paths() {
        let service = ComputerHistoryService::open_memory(
            service_with(true),
            Arc::new(NeverBackendStub),
            "01900000-0000-7000-8000-000000000001".into(),
        )
        .await
        .unwrap();
        service.start().await.unwrap();
        service.start().await.unwrap(); // idempotent
        let status = service.status().await.unwrap();
        assert_eq!(status.state, RecorderState::Running);
        assert!(status.event_stream_root_path.ends_with("segments"));
        assert!(status.current_segment_path.is_some());
        service.stop().await.unwrap();
        service.stop().await.unwrap(); // idempotent
        assert_eq!(service.status().await.unwrap().state, RecorderState::Stopped);
    }

    #[tokio::test]
    async fn pause_and_resume_cycle() {
        let service = ComputerHistoryService::open_memory(
            service_with(true),
            Arc::new(NeverBackendStub),
            "01900000-0000-7000-8000-000000000001".into(),
        )
        .await
        .unwrap();
        service.start().await.unwrap();
        let until = service.pause(PauseDuration::ThirtyMinutes).await.unwrap();
        assert!(until > now_ms());
        assert_eq!(service.status().await.unwrap().state, RecorderState::Paused);
        assert!(service.status().await.unwrap().paused_until_ms.is_some());
        service.resume().await.unwrap();
        assert_eq!(service.status().await.unwrap().state, RecorderState::Running);
        assert!(service.status().await.unwrap().paused_until_ms.is_none());
        service.stop().await.unwrap();
    }

    #[tokio::test]
    async fn pause_durations_are_sane() {
        let now = now_ms();
        assert_eq!(
            PauseDuration::ThirtyMinutes.duration_ms(now),
            30 * 60 * 1000
        );
        assert_eq!(PauseDuration::OneHour.duration_ms(now), 60 * 60 * 1000);
        assert!(PauseDuration::UntilTomorrow.duration_ms(now) >= 60 * 60 * 1000);
        assert!(PauseDuration::UntilTomorrow.duration_ms(now) <= 24 * 60 * 60 * 1000 + 60 * 60 * 1000);
    }

    #[tokio::test]
    async fn clear_history_scopes() {
        let service = ComputerHistoryService::open_memory(
            service_with(true),
            Arc::new(NeverBackendStub),
            "01900000-0000-7000-8000-000000000001".into(),
        )
        .await
        .unwrap();
        assert_eq!(service.clear_history("all", 0, 0).await.unwrap(), 0);
        assert!(service.clear_history("bogus", 0, 0).await.is_err());
        assert!(service.clear_history("interval", 200, 100).await.is_err());
    }

    #[tokio::test]
    async fn would_observe_reflects_settings() {
        let service = ComputerHistoryService::open_memory(
            service_with(true),
            Arc::new(NeverBackendStub),
            "01900000-0000-7000-8000-000000000001".into(),
        )
        .await
        .unwrap();
        assert!(service.would_observe("com.example.app", None).await.unwrap());
        assert!(!service.would_observe("com.apple.controlcenter", None).await.unwrap());
    }
}
