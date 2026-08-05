//! nomifun-ssh: backend integration for SSH remote sessions.
//!
//! Owns the saved host book (encrypted CRUD over `ssh_hosts`), the live
//! connection pool, the user-scoped status/output events, the HTTP routes, and
//! the `SshBackend` seam implementation that the agent's remote tools call. The
//! transport itself lives in the isolated `nomi-ssh` crate; this crate is the
//! only place that joins transport + credentials + persistence + realtime.

pub mod dto;
pub mod events;
pub mod routes;
pub mod service;
pub mod sink;
pub mod state;

pub use dto::{CreateSshHostRequest, SshHostResponse, SshStatusEvent, UpdateSshHostRequest};
pub use events::SshEventEmitter;
pub use routes::{ssh_host_routes, SshHostRouterState};
pub use service::{DecryptedCredential, SshHostService, SshServiceError};
pub use sink::{SshBackendSink, SshConnectionHandle, SshConnectionProvider};
pub use state::{
    is_retryable, reconnect_delay, SshLinkPhase, SshLinkState, SshTeardown, SSH_CLOSE_BUDGET,
    SSH_LIVENESS_POLL_INTERVAL, SSH_RECONNECT_INITIAL_BACKOFF_MS, SSH_RECONNECT_MAX_ATTEMPTS,
    SSH_RECONNECT_MAX_BACKOFF_MS,
};
