//! nomifun-ssh: backend integration for SSH remote sessions.
//!
//! Owns the saved host book (encrypted CRUD over `ssh_hosts`), the live
//! connection pool, the user-scoped status/output events, the HTTP routes, and
//! the `SshBackend` seam implementation that the agent's remote tools call. The
//! transport itself lives in the isolated `nomi-ssh` crate; this crate is the
//! only place that joins transport + credentials + persistence + realtime.

pub mod dto;
pub mod service;
pub mod sink;

pub use dto::{CreateSshHostRequest, SshHostResponse, UpdateSshHostRequest};
pub use service::{DecryptedCredential, SshHostService, SshServiceError};
pub use sink::{SshBackendSink, SshConnectionHandle};
