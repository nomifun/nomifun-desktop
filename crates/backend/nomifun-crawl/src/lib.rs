//! Crawl jobs: a durable URL frontier with AutoWork-style claim semantics.
//!
//! The frontier lives in SQLite (`crawl_jobs` / `crawl_tasks`). Workers claim
//! tasks atomically, hold a renewable lease, and submit results under a fencing
//! token, so a crashed or stalled worker never corrupts the queue and never
//! needs a liveness probe. See
//! `docs/specs/2026-08-05-distributed-crawler-design.zh.md`.

pub mod claim;
pub mod error;
pub mod events;
pub mod executor;
pub mod extract;
pub mod fetcher;
pub mod frontier;
pub mod model;
pub mod politeness;
pub mod routes;
pub mod runner;
pub mod service;
pub mod sink;
pub mod state;
pub mod store;

pub use claim::{LEASE_MS, MAX_ATTEMPTS};
pub use error::CrawlError;
pub use events::{CrawlEvent, CrawlEventSink, NoopEvents};
pub use executor::{CrawlExecutor, LocalExecutor};
pub use fetcher::{CrawlFetcher, HttpCrawlFetcher};
pub use model::{
    ClaimedTask, CrawlJob, CrawlScope, CrawlSink, CrawlTask, DiscoveredUrl, JobProgress, JobStatus,
    RenderMode, TaskOutcome, TaskStatus,
};
pub use politeness::Politeness;
pub use routes::crawl_routes;
pub use service::{CrawlService, NewJob};
pub use sink::{CrawlSinkWriter, KnowledgeSink};
pub use state::CrawlRouterState;
