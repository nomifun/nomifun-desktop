//! Computer History: a privacy-filtered, background activity-observation
//! feature for the nomifun desktop app, re-implementing the behavior of the
//! Codex "Computer History" (Skysight) recorder.
//!
//! Layout:
//! - [`store`] — feature-local SQLite (`{data_root}/computer-history/history.db`)
//!   holding activity segments, observation rules and a small `feature_config`
//!   KV table (settings live here, not in the main nomifun-backend.db).
//! - [`rules`] — observation rules (`scope` application|url, default
//!   observe/do_not_observe behaviors, hardcoded safety exclusions).
//! - [`config`] — feature config with defaults (feature DISABLED by default).
//! - [`service`] — the `ComputerHistoryService` facade: start/stop/pause/
//!   resume/status over store + observer.
//! - [`observer`] — sampling loop; macOS backend is a stub (task #4 owns the
//!   NSWorkspace implementation), other platforms compile to a no-op.
//! - [`chat_analytics`] — read-only analytics over the local macOS Messages
//!   database (`chat.db`): find_chats / count_message_activity parity with
//!   the Codex Messages MCP tools. Full Disk Access gated.
//!
//! Privacy discipline (mirrors nomifun-companion/src/collector.rs): every
//! captured title/URL passes `nomi_redact::redact_secrets` before persistence,
//! fields are truncated (`MAX_FIELD_CHARS`), and the store never holds model
//! prose — only observed activity.

pub mod chat_analytics;
pub mod config;
pub mod observer;
pub mod retention;
pub mod rules;
pub mod service;
pub mod store;

pub use chat_analytics::{
    ActivityBreakdown, ActivityBucket, ActivityBucketCounts, ActivityCount, ActivityInterval,
    ActivityRank, ChatAnalytics, ChatAnalyticsError, ChatAnalyticsErrorKind, ChatAnalyticsStatus,
    ChatSummary, ChatType, CountActivityRequest, CountActivityResult, FindChatsRequest,
    FindChatsResult, RankedChatActivity,
};

pub use config::ComputerHistoryConfig;
pub use observer::{ActivitySample, ObserverBackend, spawn_observer_loop};
pub use rules::{
    ActivityRule, DefaultBehavior, RuleAction, RuleScope, ObservationSettings,
    HARDCODED_EXCLUDED_BUNDLE_IDS,
};
pub use service::{
    ComputerHistoryService, PermissionState, PauseDuration, RecorderState, ServiceStatus,
};
pub use store::{ActivitySegment, ActivityStore, SegmentFilter};
