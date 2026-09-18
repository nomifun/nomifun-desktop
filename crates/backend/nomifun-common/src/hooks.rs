//! Cross-crate lifecycle hook traits.
//!
//! Hooks defined here let lower-layer crates react to product-owned lifecycle
//! events without forming dependency cycles.

use async_trait::async_trait;

/// Notified when a terminal session row is deleted via
/// `TerminalService::delete`.
///
/// Lets lower-layer crates react to a terminal going away without
/// `nomifun-terminal` depending on them (e.g. `nomifun-requirement` clears the dual-domain
/// `owner_session_id`/`owner_kind` of requirements owned by a terminal UUIDv7,
/// which has no physical FK to cascade — spec §9.B).
///
/// Implementors are responsible for cleaning up their per-terminal state. Hooks
/// run sequentially in registration order; failures must be logged inside the
/// hook and not propagated. `user_id` is the verified terminal owner captured
/// before deletion, so polymorphic cleanup remains owner-scoped after the row
/// itself is gone.
#[async_trait]
pub trait OnTerminalDelete: Send + Sync {
    async fn on_terminal_deleted(&self, user_id: &str, terminal_id: &str);
}

/// Creates a tracked requirement from an inbound channel message (the opt-in
/// IM → requirement pipeline). Lets `nomifun-channel` file a message as a
/// requirement without depending on `nomifun-requirement`; the concrete
/// implementor (in `nomifun-requirement`) delegates to `RequirementService`.
/// Creating a `Pending` requirement is enough — AutoWork is woken to execute it.
#[async_trait]
pub trait RequirementCreator: Send + Sync {
    /// Create a Pending requirement. `tag` is the board column to file under
    /// (e.g. "inbox"); `created_by` records the origin (e.g. "channel:slack").
    /// Returns the new requirement's stable bare UUIDv7 on success.
    async fn create_from_message(
        &self,
        title: &str,
        content: &str,
        tag: &str,
        created_by: &str,
    ) -> Result<String, String>;
}
