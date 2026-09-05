//! Native tools that give a nomi agent access to the local computer-history
//! recorder through a `ComputerHistorySink` trait object. The backend
//! (nomifun-computer-history) injects a concrete sink; other hosts pass `None`
//! and these are not registered. Mirrors `companion_tools.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};

/// Recorder lifecycle states mirrored from the recorder's status surface.
pub const COMPUTER_HISTORY_STATES: [&str; 3] = ["stopped", "running", "paused"];

/// Supported pause durations (mirrors the recorder's pause semantics).
pub const COMPUTER_HISTORY_PAUSE_DURATIONS: [&str; 3] = ["thirty_minutes", "one_hour", "until_tomorrow"];

/// Time-window presets accepted by the read tools; implementations may also
/// resolve an explicit ISO-8601 `from/to` range string.
pub const COMPUTER_HISTORY_WINDOWS: [&str; 5] = ["today", "yesterday", "last_7_days", "this_week", "all"];

/// Count dimensions for `computer_history_count_activity`.
pub const COMPUTER_HISTORY_DIMENSIONS: [&str; 3] = ["apps", "urls", "messages"];

/// Read tools share the same bounded digest ceiling as the companion tools.
const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 50;

/// Backend seam for the computer-history store + recorder. Implemented by
/// `nomifun-computer-history`; `nomi-agent` only depends on this trait.
#[async_trait]
pub trait ComputerHistorySink: Send + Sync {
    /// Human-readable digest of recent activity segments (newest-last).
    async fn recent_activity(&self, window: &str, limit: usize) -> Result<String, String>;
    /// Aggregated per-app foreground time inside a time window.
    async fn app_usage(&self, window: &str, limit: usize) -> Result<String, String>;
    /// Browsing-history lookup inside a time window, optionally filtered by
    /// a domain/title query.
    async fn url_history(&self, window: &str, query: Option<&str>, limit: usize) -> Result<String, String>;
    /// Chat-DB analytics: find matching chats inside a window. `cursor` is the
    /// opaque continuation returned by a previous call; the digest must carry
    /// the next cursor when more pages remain.
    async fn find_chats(
        &self,
        window: &str,
        query: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<String, String>;
    /// Chat/message activity counts. `dimension` is one of
    /// `COMPUTER_HISTORY_DIMENSIONS`; `interval` buckets by
    /// `total|day|week|hour`; `chat_guids` narrows to resolved chats.
    async fn count_activity(
        &self,
        window: &str,
        dimension: &str,
        interval: Option<&str>,
        chat_guids: &[String],
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<String, String>;
    /// Capture state (`stopped|running|paused`) plus paths to the current
    /// segment files.
    async fn status(&self) -> Result<String, String>;
    /// Temporarily pause capture without disabling it. Returns the scheduled
    /// resume time.
    async fn pause(&self, duration: &str) -> Result<String, String>;
    /// Resume a paused recorder immediately.
    async fn resume(&self) -> Result<String, String>;
    /// All capture settings as a digest the model can read-modify-write from.
    async fn get_settings(&self) -> Result<String, String>;
    /// Apply a settings change and return the effective settings digest.
    /// `replace_all` switches between patching a single aspect and replacing
    /// the full rule set.
    async fn update_settings(
        &self,
        replace_all: bool,
        default_application_behavior: Option<&str>,
        default_url_behavior: Option<&str>,
        include_applications: &[String],
        exclude_applications: &[String],
        include_urls: &[String],
        exclude_urls: &[String],
    ) -> Result<String, String>;
}

/// Shared input parsing for the window-scoped read tools.
fn parse_window(input: &Value) -> Result<String, String> {
    let window = input
        .get("window")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .unwrap_or("today");
    if COMPUTER_HISTORY_WINDOWS.contains(&window)
        || (window.starts_with('"') && window.ends_with('"') && window.len() > 2)
    {
        return Ok(window.trim_matches('"').to_owned());
    }
    // Allow an explicit ISO range like "2026-01-01T00:00:00Z/2026-01-08T00:00:00Z".
    let looks_like_iso_range = window.split('/').count() == 2 && window.contains('T');
    if looks_like_iso_range {
        Ok(window.to_owned())
    } else {
        Err(format!(
            "window 必须是 {COMPUTER_HISTORY_WINDOWS:?} 之一，或 `from/to` 的 ISO-8601 区间（收到：{window}）"
        ))
    }
}

fn parse_limit(input: &Value) -> usize {
    input
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT) as usize
}

fn parse_query(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_owned)
}

fn parse_cursor(input: &Value) -> Option<String> {
    input
        .get("cursor")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
}

fn parse_string_list(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|term| !term.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn tool_result(out: Result<String, String>) -> ToolResult {
    match out {
        Ok(content) => ToolResult {
            content,
            is_error: false,
            images: Vec::new(),
        },
        Err(content) => ToolResult {
            content,
            is_error: true,
            images: Vec::new(),
        },
    }
}

const WINDOW_SCHEMA: &str = "Time window: today | yesterday | last_7_days | this_week | all, or an ISO-8601 `from/to` range. Defaults to today.";
const LIMIT_SCHEMA: &str = "Max entries to return. Defaults to 20, capped at 50.";

/// `computer_history_recent` — recent activity segments (app, title, url, time).
pub struct RecentActivityTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl RecentActivityTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for RecentActivityTool {
    fn name(&self) -> &str {
        "computer_history_recent"
    }

    fn description(&self) -> &str {
        "Get the user's recent computer activity segments (foreground app, window title, \
         visited URL, time range) for a time window. Use when the user asks what they were \
         doing recently or you need recent on-computer context."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "window": {"type": "string", "enum": COMPUTER_HISTORY_WINDOWS, "description": WINDOW_SCHEMA},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": LIMIT_SCHEMA}
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let window = match parse_window(&input) {
            Ok(w) => w,
            Err(e) => return tool_result(Err(e)),
        };
        tool_result(self.sink.recent_activity(&window, parse_limit(&input)).await)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_apps` — per-app usage rollup.
pub struct AppUsageTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl AppUsageTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for AppUsageTool {
    fn name(&self) -> &str {
        "computer_history_apps"
    }

    fn description(&self) -> &str {
        "Aggregate foreground time per application inside a time window. Use to see which \
         apps dominated the user's time, ranked by usage."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "window": {"type": "string", "enum": COMPUTER_HISTORY_WINDOWS, "description": WINDOW_SCHEMA},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": LIMIT_SCHEMA}
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let window = match parse_window(&input) {
            Ok(w) => w,
            Err(e) => return tool_result(Err(e)),
        };
        tool_result(self.sink.app_usage(&window, parse_limit(&input)).await)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_urls` — browsing-history lookup.
pub struct UrlHistoryTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl UrlHistoryTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for UrlHistoryTool {
    fn name(&self) -> &str {
        "computer_history_urls"
    }

    fn description(&self) -> &str {
        "Search the user's browsing history (URL + page title) inside a time window. \
         Use an optional query to filter by domain or title keywords; omit it to list \
         the most recent visits."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "window": {"type": "string", "enum": COMPUTER_HISTORY_WINDOWS, "description": WINDOW_SCHEMA},
                "query": {"type": "string", "description": "Optional filter on domain or page title keywords."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": LIMIT_SCHEMA}
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let window = match parse_window(&input) {
            Ok(w) => w,
            Err(e) => return tool_result(Err(e)),
        };
        let query = parse_query(&input, "query");
        tool_result(self.sink.url_history(&window, query.as_deref(), parse_limit(&input)).await)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_find_chats` — locate chat conversations by participant or name.
pub struct FindChatsTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl FindChatsTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for FindChatsTool {
    fn name(&self) -> &str {
        "computer_history_find_chats"
    }

    fn description(&self) -> &str {
        "Find chat conversations by participant name, chat name, or time window. Returns each \
         chat's unread count and a stable chat id for reuse with computer_history_count_activity. \
         When more chats are available the response includes next_cursor; pass that value as \
         cursor in the next call with the original arguments. When counts for specific chats are \
         needed, resolve them here first, then pass the returned chat ids."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "window": {"type": "string", "enum": COMPUTER_HISTORY_WINDOWS, "description": WINDOW_SCHEMA},
                "query": {"type": "string", "description": "Optional participant or chat-name filter."},
                "cursor": {"type": "string", "description": "Opaque continuation from a previous call's next_cursor."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": LIMIT_SCHEMA}
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let window = match parse_window(&input) {
            Ok(w) => w,
            Err(e) => return tool_result(Err(e)),
        };
        let query = parse_query(&input, "query");
        let cursor = parse_cursor(&input);
        tool_result(
            self.sink
                .find_chats(&window, query.as_deref(), cursor.as_deref(), parse_limit(&input))
                .await,
        )
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_count_activity` — message/activity counts over time.
pub struct CountActivityTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl CountActivityTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for CountActivityTool {
    fn name(&self) -> &str {
        "computer_history_count_activity"
    }

    fn description(&self) -> &str {
        "Count computer or chat activity over time, either overall or per chat. Counts split \
         into sent and received and may be grouped by calendar interval (day buckets start \
         Monday). When the request names specific chats, resolve them with \
         computer_history_find_chats first, then pass the returned chat ids as chat_guids. \
         When more chats are available the response includes next_cursor; pass that value as \
         cursor in the next call with the original filter arguments."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "window": {"type": "string", "enum": COMPUTER_HISTORY_WINDOWS, "description": WINDOW_SCHEMA},
                "dimension": {
                    "type": "string",
                    "enum": COMPUTER_HISTORY_DIMENSIONS,
                    "description": "What to count: apps, urls, or messages. Defaults to messages."
                },
                "interval": {
                    "type": "string",
                    "enum": ["total", "day", "week", "hour"],
                    "description": "Calendar bucket interval. total returns one bucket for the whole window."
                },
                "chat_guids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Restrict the count to these chat ids (from computer_history_find_chats)."
                },
                "cursor": {"type": "string", "description": "Opaque continuation from a previous call's next_cursor."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": LIMIT_SCHEMA}
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let window = match parse_window(&input) {
            Ok(w) => w,
            Err(e) => return tool_result(Err(e)),
        };
        let dimension = input
            .get("dimension")
            .and_then(|v| v.as_str())
            .filter(|d| COMPUTER_HISTORY_DIMENSIONS.contains(d))
            .unwrap_or("messages");
        let interval = parse_query(&input, "interval");
        let chat_guids = parse_string_list(&input, "chat_guids");
        let cursor = parse_cursor(&input);
        tool_result(
            self.sink
                .count_activity(
                    &window,
                    dimension,
                    interval.as_deref(),
                    &chat_guids,
                    cursor.as_deref(),
                    parse_limit(&input),
                )
                .await,
        )
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_status` — recorder state + current segment paths.
pub struct StatusTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl StatusTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for StatusTool {
    fn name(&self) -> &str {
        "computer_history_status"
    }

    fn description(&self) -> &str {
        "Get Computer History capture status and paths to recent activity files. Returns the \
         recorder state (stopped | running | paused), the activity stream root path, and the \
         current segment's events/metadata paths."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        tool_result(self.sink.status().await)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_pause` — temporarily pause capture without disabling it.
pub struct PauseTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl PauseTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for PauseTool {
    fn name(&self) -> &str {
        "computer_history_pause"
    }

    fn description(&self) -> &str {
        "Temporarily pause Computer History capture without disabling it. The recorder resumes \
         automatically after the chosen duration (thirty_minutes | one_hour) or at the start of \
         the next day (until_tomorrow). Ask the user before pausing."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "duration": {
                    "type": "string",
                    "enum": COMPUTER_HISTORY_PAUSE_DURATIONS,
                    "description": "How long to stay paused. Defaults to thirty_minutes."
                }
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let duration = input
            .get("duration")
            .and_then(|v| v.as_str())
            .filter(|d| COMPUTER_HISTORY_PAUSE_DURATIONS.contains(d))
            .unwrap_or("thirty_minutes");
        tool_result(self.sink.pause(duration).await)
    }

    fn category(&self) -> ToolCategory {
        // Mutates recorder state, so it rides the normal approval lane instead
        // of the read-only Info auto-approval.
        ToolCategory::Exec
    }
}

/// `computer_history_resume` — resume a paused recorder.
pub struct ResumeTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl ResumeTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for ResumeTool {
    fn name(&self) -> &str {
        "computer_history_resume"
    }

    fn description(&self) -> &str {
        "Resume a paused Computer History recorder immediately, cancelling any scheduled \
         automatic resume."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        tool_result(self.sink.resume().await)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

/// `computer_history_get_settings` — read the full settings surface.
pub struct GetSettingsTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl GetSettingsTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for GetSettingsTool {
    fn name(&self) -> &str {
        "computer_history_get_settings"
    }

    fn description(&self) -> &str {
        "Get all Computer History settings. Call this immediately before \
         computer_history_update_settings so unchanged fields can be preserved — settings \
         updates are read-modify-write, never blind overwrites."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        tool_result(self.sink.get_settings().await)
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `computer_history_update_settings` — read-modify-write the capture rules.
pub struct UpdateSettingsTool {
    sink: Arc<dyn ComputerHistorySink>,
}

impl UpdateSettingsTool {
    pub fn new(sink: Arc<dyn ComputerHistorySink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for UpdateSettingsTool {
    fn name(&self) -> &str {
        "computer_history_update_settings"
    }

    fn description(&self) -> &str {
        "Replace or adjust Computer History settings. Preserve every setting the user did not \
         ask to change by first calling computer_history_get_settings. Application rules use \
         bundle IDs; URL rules use a bare domain without scheme or path. Pass replace_all=true \
         only when the user asked to replace the entire rule set, otherwise individual changes \
         are merged. Ask the user before changing what gets observed."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "replace_all": {
                    "type": "boolean",
                    "description": "When true, replaces the entire settings/rule set with the given values; when false, merges individual changes into the current settings. Defaults to false."
                },
                "default_application_behavior": {
                    "type": "string",
                    "enum": ["observe", "do_not_observe"],
                    "description": "Default observation behavior for applications not matched by any rule."
                },
                "default_url_behavior": {
                    "type": "string",
                    "enum": ["observe", "do_not_observe"],
                    "description": "Default observation behavior for URLs not matched by any rule."
                },
                "include_applications": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Bundle IDs to always observe."
                },
                "exclude_applications": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Bundle IDs to never observe."
                },
                "include_urls": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "URL domains (no scheme or path) to always observe."
                },
                "exclude_urls": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "URL domains (no scheme or path) to never observe."
                }
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let default_application_behavior = input
            .get("default_application_behavior")
            .and_then(|v| v.as_str())
            .filter(|b| ["observe", "do_not_observe"].contains(b));
        let default_url_behavior = input
            .get("default_url_behavior")
            .and_then(|v| v.as_str())
            .filter(|b| ["observe", "do_not_observe"].contains(b));
        tool_result(
            self.sink
                .update_settings(
                    input.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false),
                    default_application_behavior,
                    default_url_behavior,
                    &parse_string_list(&input, "include_applications"),
                    &parse_string_list(&input, "exclude_applications"),
                    &parse_string_list(&input, "include_urls"),
                    &parse_string_list(&input, "exclude_urls"),
                )
                .await,
        )
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

/// Register every computer-history tool when the host provides a sink. Hosts
/// without the capability pass `None` and nothing is registered.
pub fn register_computer_history_tools(
    registry: &mut nomi_tools::registry::ToolRegistry,
    sink: Option<Arc<dyn ComputerHistorySink>>,
) {
    let Some(sink) = sink else {
        return;
    };
    let tools: Vec<Box<dyn Tool>> = vec![        Box::new(RecentActivityTool::new(sink.clone())),
        Box::new(AppUsageTool::new(sink.clone())),
        Box::new(UrlHistoryTool::new(sink.clone())),
        Box::new(FindChatsTool::new(sink.clone())),
        Box::new(CountActivityTool::new(sink.clone())),
        Box::new(StatusTool::new(sink.clone())),
        Box::new(PauseTool::new(sink.clone())),
        Box::new(ResumeTool::new(sink.clone())),
        Box::new(GetSettingsTool::new(sink.clone())),
        Box::new(UpdateSettingsTool::new(sink)),
    ];
    for tool in tools {
        registry.register(tool);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingSink {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingSink {
        fn record(&self, line: String) {
            self.calls.lock().unwrap().push(line);
        }

        fn last(&self) -> String {
            self.calls.lock().unwrap().last().cloned().unwrap_or_default()
        }
    }

    #[async_trait]
    impl ComputerHistorySink for RecordingSink {
        async fn recent_activity(&self, window: &str, limit: usize) -> Result<String, String> {
            self.record(format!("recent {window} {limit}"));
            Ok("segments".into())
        }
        async fn app_usage(&self, window: &str, limit: usize) -> Result<String, String> {
            self.record(format!("apps {window} {limit}"));
            Ok("usage".into())
        }
        async fn url_history(&self, window: &str, query: Option<&str>, limit: usize) -> Result<String, String> {
            self.record(format!("urls {window} {query:?} {limit}"));
            Ok("urls".into())
        }
        async fn find_chats(
            &self,
            window: &str,
            query: Option<&str>,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<String, String> {
            self.record(format!("find_chats {window} {query:?} {cursor:?} {limit}"));
            Ok("chats".into())
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
            self.record(format!(
                "count {window} {dimension} {interval:?} {chat_guids:?} {cursor:?} {limit}"
            ));
            Ok("counts".into())
        }
        async fn status(&self) -> Result<String, String> {
            self.record("status".into());
            Ok("running".into())
        }
        async fn pause(&self, duration: &str) -> Result<String, String> {
            self.record(format!("pause {duration}"));
            Ok("paused".into())
        }
        async fn resume(&self) -> Result<String, String> {
            self.record("resume".into());
            Ok("resumed".into())
        }
        async fn get_settings(&self) -> Result<String, String> {
            self.record("get_settings".into());
            Ok("settings".into())
        }
        async fn update_settings(
            &self,
            replace_all: bool,
            default_application_behavior: Option<&str>,
            _default_url_behavior: Option<&str>,
            include_applications: &[String],
            _exclude_applications: &[String],
            _include_urls: &[String],
            _exclude_urls: &[String],
        ) -> Result<String, String> {
            self.record(format!(
                "update replace_all={replace_all} app={default_application_behavior:?} include_apps={include_applications:?}"
            ));
            Ok("updated".into())
        }
    }

    fn sink() -> Arc<RecordingSink> {
        Arc::new(RecordingSink {
            calls: Mutex::new(vec![]),
        })
    }

    #[tokio::test]
    async fn recent_validates_window_and_clamps_limit() {
        let s = sink();
        let tool = RecentActivityTool::new(s.clone());
        assert!(tool.execute(json!({"window": "tomorrow"})).await.is_error);
        let out = tool.execute(json!({"window": "last_7_days", "limit": 9999})).await;
        assert!(!out.is_error);
        assert_eq!(s.last(), "recent last_7_days 50");
        let _ = tool.execute(json!({})).await;
        assert_eq!(s.last(), "recent today 20");
        // Explicit ISO range is accepted.
        let iso = tool
            .execute(json!({"window": "2026-07-01T00:00:00Z/2026-07-08T00:00:00Z"}))
            .await;
        assert!(!iso.is_error);
    }

    #[tokio::test]
    async fn url_tools_pass_optional_query() {
        let s = sink();
        let tool = UrlHistoryTool::new(s.clone());
        let _ = tool.execute(json!({"window": "today", "query": " rust " })).await;
        assert_eq!(s.last(), "urls today Some(\"rust\") 20");
        let _ = tool.execute(json!({"window": "today"})).await;
        assert_eq!(s.last(), "urls today None 20");
        let _ = tool.execute(json!({"window": "today", "limit": 0})).await;
        assert_eq!(s.last(), "urls today None 1");
    }

    #[tokio::test]
    async fn chat_tools_thread_cursor_and_guids() {
        let s = sink();
        let find = FindChatsTool::new(s.clone());
        let _ = find
            .execute(json!({"window": "all", "query": "mom", "cursor": "abc", "limit": 5}))
            .await;
        assert_eq!(s.last(), "find_chats all Some(\"mom\") Some(\"abc\") 5");

        let count = CountActivityTool::new(s.clone());
        let _ = count
            .execute(json!({
                "window": "last_7_days",
                "dimension": "bogus",
                "interval": "week",
                "chat_guids": ["guid-1", " "],
                "cursor": "next",
                "limit": 99
            }))
            .await;
        // Unknown dimension falls back to messages; blank guids are dropped.
        assert_eq!(s.last(), "count last_7_days messages Some(\"week\") [\"guid-1\"] Some(\"next\") 50");
    }

    #[tokio::test]
    async fn lifecycle_tools_reach_sink() {
        let s = sink();
        let _ = StatusTool::new(s.clone()).execute(json!({})).await;
        assert_eq!(s.last(), "status");
        let _ = PauseTool::new(s.clone()).execute(json!({"duration": "one_hour"})).await;
        assert_eq!(s.last(), "pause one_hour");
        // Unknown duration falls back to thirty_minutes.
        let _ = PauseTool::new(s.clone()).execute(json!({"duration": "forever"})).await;
        assert_eq!(s.last(), "pause thirty_minutes");
        let _ = ResumeTool::new(s.clone()).execute(json!({})).await;
        assert_eq!(s.last(), "resume");
        let _ = GetSettingsTool::new(s.clone()).execute(json!({})).await;
        assert_eq!(s.last(), "get_settings");
    }

    #[tokio::test]
    async fn update_settings_passes_read_modify_write_shape() {
        let s = sink();
        let tool = UpdateSettingsTool::new(s.clone());
        let _ = tool
            .execute(json!({
                "replace_all": true,
                "default_application_behavior": "do_not_observe",
                "include_applications": ["com.apple.Safari"]
            }))
            .await;
        assert_eq!(
            s.last(),
            "update replace_all=true app=Some(\"do_not_observe\") include_apps=[\"com.apple.Safari\"]"
        );
        // Invalid behavior enum values are dropped, not errors.
        let _ = tool.execute(json!({"default_application_behavior": "spy"})).await;
        assert_eq!(s.last(), "update replace_all=false app=None include_apps=[]");
    }

    #[tokio::test]
    async fn register_with_none_registers_nothing() {
        let mut registry = nomi_tools::registry::ToolRegistry::new();
        register_computer_history_tools(&mut registry, None);
        assert!(registry.tool_names().is_empty());

        let mut registry = nomi_tools::registry::ToolRegistry::new();
        register_computer_history_tools(&mut registry, Some(sink()));
        let names = registry.tool_names();
        assert_eq!(names.len(), 10, "{names:?}");
        for expected in [
            "computer_history_recent",
            "computer_history_apps",
            "computer_history_urls",
            "computer_history_find_chats",
            "computer_history_count_activity",
            "computer_history_status",
            "computer_history_pause",
            "computer_history_resume",
            "computer_history_get_settings",
            "computer_history_update_settings",
        ] {
            assert!(names.contains(&expected.to_owned()), "missing {expected}: {names:?}");
        }
    }
}
