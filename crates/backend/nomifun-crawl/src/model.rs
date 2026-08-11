use nomifun_common::{CrawlJobId, CrawlTaskId, TimestampMs, UserId};
use serde::{Deserialize, Serialize};

/// How a page is retrieved. `Auto` starts on HTTP and escalates to a browser
/// Lane only when the HTTP body looks like an unrendered shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
    #[default]
    Auto,
    Http,
    Browser,
}

impl RenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Http => "http",
            Self::Browser => "browser",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "http" => Some(Self::Http),
            "browser" => Some(Self::Browser),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Draft,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Failed,
    Skipped,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// What a job is allowed to follow. An empty scope is not "everything": the
/// service rejects a job whose scope would admit hosts outside its seeds.
/// `Default` is written out rather than derived: `#[serde(default = ...)]`
/// only governs deserialization, so a derived `Default` would silently flip
/// `same_site` to false and admit the entire web.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlScope {
    /// Only follow links whose registrable domain matches a seed's.
    #[serde(default = "default_true")]
    pub same_site: bool,
    /// Additionally require the URL to start with one of these prefixes.
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    /// Regexes a URL must match at least one of (when non-empty).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Regexes that reject a URL outright; evaluated after `allow`.
    #[serde(default)]
    pub deny: Vec<String>,
}

impl Default for CrawlScope {
    fn default() -> Self {
        Self {
            same_site: true,
            path_prefixes: Vec::new(),
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Where extracted content is written.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrawlSink {
    /// Target knowledge base. `None` keeps results in the task rows only.
    #[serde(default)]
    pub knowledge_base_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CrawlJob {
    pub job_id: CrawlJobId,
    pub user_id: UserId,
    pub name: String,
    pub seeds: Vec<String>,
    pub scope: CrawlScope,
    pub max_depth: u32,
    pub max_urls: u32,
    pub render_mode: RenderMode,
    pub concurrency: u32,
    pub per_host_concurrency: u32,
    pub delay_ms: u64,
    pub respect_robots: bool,
    pub user_agent: Option<String>,
    pub sink: CrawlSink,
    pub status: JobStatus,
    pub error_detail: Option<String>,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone)]
pub struct CrawlTask {
    pub task_id: CrawlTaskId,
    pub job_id: CrawlJobId,
    pub parent_task_id: Option<CrawlTaskId>,
    pub url: String,
    pub url_fingerprint: String,
    pub host: String,
    pub depth: u32,
    pub priority: i64,
    pub status: TaskStatus,
    pub attempt_count: u32,
    pub claim_generation: i64,
    pub owner_node_id: Option<String>,
    pub claimed_at: Option<TimestampMs>,
    pub lease_expires_at: Option<TimestampMs>,
    pub http_status: Option<u16>,
    pub content_hash: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub completed_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// A task plus the fencing capability minted for exactly this claim. The token
/// is never part of [`CrawlTask`] so it cannot leak through a list DTO.
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub task: CrawlTask,
    pub claim_token: String,
}

/// Outcome a worker reports back for one claimed task.
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Fetched {
        http_status: u16,
        content_hash: String,
        etag: Option<String>,
        last_modified: Option<String>,
        /// Newly discovered in-scope URLs, already normalized.
        discovered: Vec<DiscoveredUrl>,
    },
    /// Server confirmed the cached copy is current; nothing to re-ingest.
    Unchanged { http_status: u16 },
    /// In-scope but deliberately not ingested (robots, content type, budget).
    Skipped { reason: String },
    Failed {
        error_code: String,
        error_detail: String,
        /// False for poison URLs that should not consume another attempt.
        retryable: bool,
    },
}

#[derive(Debug, Clone)]
pub struct DiscoveredUrl {
    pub url: String,
    pub fingerprint: String,
    pub host: String,
    pub depth: u32,
}

/// Live counters for one job, derived from the task rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobProgress {
    pub pending: u64,
    pub in_progress: u64,
    pub done: u64,
    pub failed: u64,
    pub skipped: u64,
}

impl JobProgress {
    pub fn total(&self) -> u64 {
        self.pending + self.in_progress + self.done + self.failed + self.skipped
    }

    pub fn is_drained(&self) -> bool {
        self.pending == 0 && self.in_progress == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A derived `Default` silently disagrees with `#[serde(default = ...)]`.
    /// Both of these defaults are safety rails, so they are asserted, not
    /// assumed.
    #[test]
    fn rust_and_json_defaults_agree_on_scope() {
        let from_json: CrawlScope = serde_json::from_str("{}").unwrap();
        assert!(CrawlScope::default().same_site);
        assert_eq!(CrawlScope::default().same_site, from_json.same_site);
    }

    #[test]
    fn rust_and_json_defaults_agree_on_sink() {
        let from_json: CrawlSink = serde_json::from_str("{}").unwrap();
        assert_eq!(CrawlSink::default().knowledge_base_id, from_json.knowledge_base_id);
    }
}
