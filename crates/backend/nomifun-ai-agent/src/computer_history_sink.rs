//! Production `ComputerHistorySink` over `ComputerHistoryService`. Mirrors
//! `LiveKnowledgeRetrieval`: the trait lives in `nomi-agent`, this is the
//! backend impl wired in at app startup. Every digest is bounded JSON text —
//! the store already redacts titles before persistence, so nothing secret
//! reaches the model through this seam.

use std::sync::Arc;

use async_trait::async_trait;
use nomi_agent::computer_history_tools::ComputerHistorySink;
use nomifun_common::now_ms;
use nomifun_computer_history::{
    ActivityStore, ComputerHistoryService, PauseDuration, SegmentFilter,
};
use serde_json::{Value, json};

/// Digest row cap shared with the agent tools (`MAX_LIMIT` there); the sink
/// clamps again so a hand-built window string cannot fan out the store.
const MAX_LIMIT: usize = 50;

/// Bridges the agent-facing computer-history trait to the backend service.
pub struct LiveComputerHistorySink {
    pub service: Arc<ComputerHistoryService>,
}

impl LiveComputerHistorySink {
    /// Millisecond bracket for a named window. Accepts the presets the tools
    /// advertise plus an explicit `from/to` ISO-8601 interval; unknown names
    /// fall back to "today" (the tools' parser already validates presets,
    /// this is the sink's own defense).
    fn window_bounds(window: &str) -> (i64, i64) {
        use chrono::TimeZone;
        let now = now_ms();
        let day: i64 = 24 * 60 * 60 * 1000;
        let start_of_day = chrono::Local
            .timestamp_millis_opt(now)
            .single()
            .map(|dt| {
                let local_midnight = dt
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .and_then(|t| chrono::Local.from_local_datetime(&t).single());
                local_midnight.map_or(now - (now % day), |t| t.timestamp_millis())
            })
            .unwrap_or(now - (now % day));
        match window {
            "all" => (0, now),
            "yesterday" => (start_of_day - day, start_of_day),
            "last_7_days" => (start_of_day - 6 * day, now),
            "this_week" => {
                use chrono::Datelike;
                let days_since_monday = chrono::DateTime::from_timestamp_millis(now)
                    .map(|dt| dt.weekday().num_days_from_monday() as i64)
                    .unwrap_or(0);
                (start_of_day - days_since_monday * day, now)
            }
            // Explicit `from/to` ISO range (tool layer validated the shape) or
            // anything unrecognized: today.
            _ => (start_of_day, now),
        }
    }

    fn filter(window: &str, limit: usize) -> SegmentFilter {
        let (from_ms, to_ms) = Self::window_bounds(window);
        SegmentFilter {
            from_ms: Some(from_ms),
            to_ms: Some(to_ms),
            app_name: None,
            url_contains: None,
            limit: limit.clamp(1, MAX_LIMIT) as u32,
        }
    }

    async fn store(&self) -> Result<&Arc<ActivityStore>, String> {
        Ok(self.service.store())
    }

    /// ISO-8601 bounds for the chat-analytics requests (they take ISO strings,
    /// not epoch ms). `None` lower bound for `all` (begin at oldest activity).
    fn iso_bounds(window: &str) -> (Option<String>, Option<String>) {
        use chrono::TimeZone;
        let (from_ms, to_ms) = Self::window_bounds(window);
        let render = |ms: i64| {
            chrono::Local
                .timestamp_millis_opt(ms)
                .single()
                .map(|dt| dt.to_rfc3339())
        };
        (if from_ms == 0 { None } else { render(from_ms) }, render(to_ms))
    }

    /// Shared translation from a ChatAnalyticsError to the agent-facing
    /// string error (keeps the TCC-denied cause readable to the model).
    fn chat_error(error: nomifun_computer_history::ChatAnalyticsError) -> String {
        error.to_string()
    }
}

#[async_trait]
impl ComputerHistorySink for LiveComputerHistorySink {
    async fn recent_activity(&self, window: &str, limit: usize) -> Result<String, String> {
        let segments = self
            .store()
            .await?
            .query_segments(&Self::filter(window, limit))
            .await
            .map_err(|e| e.to_string())?;
        let rows: Vec<Value> = segments
            .into_iter()
            .map(|segment| {
                json!({
                    "event_id": segment.event_id,
                    "app_name": segment.app_name,
                    "bundle_id": segment.bundle_id,
                    "window_title": segment.window_title,
                    "browser_url": segment.browser_url,
                    "browser_title": segment.browser_title,
                    "source": segment.source,
                    "started_at_ms": segment.started_at_ms,
                    "ended_at_ms": segment.ended_at_ms,
                })
            })
            .collect();
        Ok(json!({ "window": window, "segments": rows }).to_string())
    }

    async fn app_usage(&self, window: &str, limit: usize) -> Result<String, String> {
        let rows = self
            .store()
            .await?
            .app_usage(&Self::filter(window, limit))
            .await
            .map_err(|e| e.to_string())?;
        Ok(json!({ "window": window, "rows": rows }).to_string())
    }

    async fn url_history(
        &self,
        window: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<String, String> {
        let mut filter = Self::filter(window, limit);
        filter.url_contains = query.map(str::to_string);
        let rows = self
            .store()
            .await?
            .url_history(&filter)
            .await
            .map_err(|e| e.to_string())?;
        Ok(json!({ "window": window, "rows": rows }).to_string())
    }

    async fn find_chats(
        &self,
        window: &str,
        query: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<String, String> {
        use nomifun_computer_history::FindChatsRequest;
        let Some(analytics) = self.service.chat_analytics() else {
            return Err(
                "chat history analytics is unavailable (macOS Messages database not accessible)"
                    .to_string(),
            );
        };
        let (from, to) = Self::iso_bounds(window);
        let request = FindChatsRequest {
            participants: query.map(|q| vec![q.to_string()]),
            exact_participants: None,
            // `query` matches chat name OR participants (tool contract); the
            // name field is also set so a chat-name-only query still hits.
            chat_name: query.map(str::to_string),
            from,
            to,
            unread_only: None,
            limit: Some(limit.clamp(1, MAX_LIMIT) as u32),
            cursor: cursor.map(str::to_string),
        };
        let result = analytics
            .find_chats(&request)
            .await
            .map_err(Self::chat_error)?;
        Ok(serde_json::to_string(&result).map_err(|e| e.to_string())?)
    }

    async fn count_activity(
        &self,
        window: &str,
        dimension: &str,
        interval: Option<&str>,
        chat_guids: &[String],
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<String, String> {
        match dimension {
            "apps" => self.app_usage(window, limit).await,
            "urls" => self.url_history(window, None, limit).await,
            "messages" => {
                use nomifun_computer_history::{
                    ActivityBreakdown, ActivityInterval as Interval, CountActivityRequest,
                };
                let Some(analytics) = self.service.chat_analytics() else {
                    return Err(
                        "chat history analytics is unavailable (macOS Messages database not accessible)"
                            .to_string(),
                    );
                };
                let (from, to) = Self::iso_bounds(window);
                let bucket = interval.and_then(|value| match value {
                    "day" => Some(Interval::Day),
                    "week" => Some(Interval::Week),
                    "hour" => Some(Interval::Hour),
                    "total" => Some(Interval::Total),
                    _ => None,
                });
                let request = CountActivityRequest {
                    from,
                    to,
                    interval: bucket,
                    chat_type: None,
                    chat_guids: (!chat_guids.is_empty()).then(|| chat_guids.to_vec()),
                    // Named chats need a per-chat ranking; a bare count stays
                    // aggregate so the digest stays small.
                    breakdown: (!chat_guids.is_empty()).then_some(ActivityBreakdown::Chat),
                    rank_by: None,
                    limit: Some(limit.clamp(1, MAX_LIMIT) as u32),
                    cursor: cursor.map(str::to_string),
                };
                let result = analytics
                    .count_message_activity(&request)
                    .await
                    .map_err(Self::chat_error)?;
                Ok(serde_json::to_string(&result).map_err(|e| e.to_string())?)
            }
            other => Err(format!("unsupported count dimension: {other}")),
        }
    }

    async fn status(&self) -> Result<String, String> {
        let status = self.service.status().await.map_err(|e| e.to_string())?;
        Ok(serde_json::to_string(&status).map_err(|e| e.to_string())?)
    }

    async fn pause(&self, duration: &str) -> Result<String, String> {
        let duration = match duration {
            "thirty_minutes" => PauseDuration::ThirtyMinutes,
            "one_hour" => PauseDuration::OneHour,
            "until_tomorrow" => PauseDuration::UntilTomorrow,
            other => return Err(format!("unsupported pause duration: {other}")),
        };
        let until = self.service.pause(duration).await.map_err(|e| e.to_string())?;
        Ok(json!({ "paused_until_ms": until }).to_string())
    }

    async fn resume(&self) -> Result<String, String> {
        self.service.resume().await.map_err(|e| e.to_string())?;
        self.status().await
    }

    async fn get_settings(&self) -> Result<String, String> {
        let settings = self
            .service
            .observation_settings()
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_string(&settings).map_err(|e| e.to_string())?)
    }

    async fn update_settings(
        &self,
        replace_all: bool,
        default_application_behavior: Option<&str>,
        default_url_behavior: Option<&str>,
        include_applications: &[String],
        exclude_applications: &[String],
        include_urls: &[String],
        exclude_urls: &[String],
    ) -> Result<String, String> {
        use nomifun_computer_history::{
            ActivityRule, DefaultBehavior, RuleAction, RuleScope,
        };
        // Patch mode (replace_all = false) starts from the CURRENT settings so
        // an unset field keeps its value; replace_all rebuilds from defaults
        // (get → modify → send stays the documented workflow).
        let mut settings = if replace_all {
            nomifun_computer_history::ObservationSettings::default()
        } else {
            self.service
                .observation_settings()
                .await
                .map_err(|e| e.to_string())?
        };
        let parse_behavior = |value: Option<&str>,
                              current: DefaultBehavior,
                              label: &str|
         -> Result<DefaultBehavior, String> {
            match value {
                // Patch mode + no value = keep the current default.
                None => Ok(current),
                Some("observe") => Ok(DefaultBehavior::Observe),
                Some("do_not_observe") => Ok(DefaultBehavior::DoNotObserve),
                Some(other) => Err(format!("unsupported {label}: {other}")),
            }
        };
        settings.default_application_behavior = parse_behavior(
            default_application_behavior,
            settings.default_application_behavior,
            "defaultApplicationBehavior",
        )?;
        settings.default_url_behavior = parse_behavior(
            default_url_behavior,
            settings.default_url_behavior,
            "defaultURLBehavior",
        )?;
        let rules = |scope: RuleScope, domains_or_bundles: &[String]| -> Vec<ActivityRule> {
            domains_or_bundles
                .iter()
                .filter_map(|entry| {
                    ActivityRule {
                        id: nomifun_common::generate_id(),
                        scope,
                        bundle_id: (scope == RuleScope::Application).then(|| entry.clone()),
                        url_domain: (scope == RuleScope::Url).then(|| entry.clone()),
                        action: RuleAction::Capture,
                    }
                    .validated()
                    .ok()
                })
                .collect()
        };
        // Patch mode with empty lists keeps the existing rules untouched;
        // replace_all always rebuilds both lists from the arguments.
        if replace_all || !include_applications.is_empty() || !include_urls.is_empty() {
            settings.allowlist = [
                rules(RuleScope::Application, include_applications),
                rules(RuleScope::Url, include_urls),
            ]
            .concat();
        }
        if replace_all || !exclude_applications.is_empty() || !exclude_urls.is_empty() {
            settings.blocklist = [
                rules(RuleScope::Application, exclude_applications),
                rules(RuleScope::Url, exclude_urls),
            ]
            .concat();
        }
        self.service
            .update_observation_settings(&settings)
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_string(&settings).map_err(|e| e.to_string())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_computer_history::{
        ComputerHistoryConfig, ComputerHistoryService, DefaultBehavior,
        ObservationSettings, RuleScope,
    };

    async fn service_with(settings: &ObservationSettings) -> Arc<ComputerHistoryService> {
        struct NeverBackend;
        #[async_trait::async_trait]
        impl nomifun_computer_history::ObserverBackend for NeverBackend {
            async fn current_sample(
                &self,
            ) -> Option<nomifun_computer_history::ActivitySample> {
                None
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let service = ComputerHistoryService::open(
            ComputerHistoryConfig {
                data_dir: dir.path().to_path_buf(),
                ..Default::default()
            },
            Arc::new(NeverBackend),
            "01900000-0000-7000-8000-000000000001".into(),
        )
        .await
        .unwrap();
        service.update_observation_settings(settings).await.unwrap();
        // dir is leaked deliberately: the service owns the sqlite file and
        // the temp guard must outlive it.
        std::mem::forget(dir);
        Arc::new(service)
    }

    /// MAJOR-1 regression: patch mode must MERGE — an unset default keeps its
    /// value, existing rules survive, and only the named aspect changes.
    #[tokio::test]
    async fn patch_mode_preserves_unset_defaults_and_rules() {
        let mut base = ObservationSettings::default();
        base.default_application_behavior = DefaultBehavior::DoNotObserve;
        let service = service_with(&base).await;
        let sink = LiveComputerHistorySink { service: service.clone() };

        // Patch that only sets defaultURLBehavior: the application default
        // must stay DoNotObserve (the old code reset it to Observe).
        sink.update_settings(
            false,
            None,
            Some("observe"),
            &[],
            &[],
            &[],
            &[],
        )
        .await
        .unwrap();
        let after = service.observation_settings().await.unwrap();
        assert_eq!(
            after.default_application_behavior,
            DefaultBehavior::DoNotObserve,
            "patch mode wiped an unset default"
        );
        assert_eq!(after.default_url_behavior, DefaultBehavior::Observe);

        // Patch that only names exclude_applications: allowlist stays empty,
        // blocklist gets exactly the new entry.
        sink.update_settings(
            false,
            None,
            None,
            &[],
            &["com.example.Blocked".to_string()],
            &[],
            &[],
        )
        .await
        .unwrap();
        let after = service.observation_settings().await.unwrap();
        assert!(after.allowlist.is_empty());
        assert_eq!(after.blocklist.len(), 1);
        assert_eq!(after.blocklist[0].bundle_id.as_deref(), Some("com.example.Blocked"));
        assert_eq!(after.blocklist[0].scope, RuleScope::Application);

        // replace_all rebuilds from defaults: the DoNotObserve default and
        // the blocklist entry are both gone.
        sink.update_settings(true, Some("observe"), None, &[], &[], &[], &[]).await.unwrap();
        let after = service.observation_settings().await.unwrap();
        assert_eq!(after.default_application_behavior, DefaultBehavior::Observe);
        assert!(after.blocklist.is_empty());
    }
}
