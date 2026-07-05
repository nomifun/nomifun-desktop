//! `nomifun-creation` — the 生成引擎 (media generation engine): the async task
//! queue behind the 创意工坊 canvas's generation nodes.
//!
//! The engine is **provider-agnostic**: a [`MediaProvider`] adapter declares its
//! capabilities and does submit/poll; the [`CreationService`] owns the state
//! machine (`queued → running → succeeded/failed/canceled`), per-provider
//! concurrency, cancellation, boot reconciliation, and hands produced bytes to
//! an [`AssetSink`] (implemented by the app over `nomifun-workshop`, so neither
//! domain crate depends on the other — no cycle).
//!
//! M0 ships the skeleton only: the task table, the state-machine seams, the
//! provider/sink traits, and an empty `adapters/` module. `POST /api/creation/tasks`
//! enqueues then immediately fails with `adapter_unavailable`; the remaining
//! query/cancel routes are fully live. M2 lands the real adapters + run loop.

mod adapters;
mod dto;
mod types;

pub mod provider;
pub mod routes;
pub mod service;
pub mod state;

pub use dto::CreationTask;
pub use provider::{MediaProvider, PollResult, ProducedAsset, ProducedData, SubmitAck, SubmitRequest};
pub use routes::creation_routes;
pub use service::{AssetSink, CreationService, NewCreationTask};
pub use state::CreationRouterState;
pub use types::{CreationError, CreationInput, MediaCapability, TaskStatus};
