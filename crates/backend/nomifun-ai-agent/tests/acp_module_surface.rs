//! Compile-only smoke test for `manager::acp` public surface.
//!
//! During the Stage 1 refactor (splitting `manager/acp/agent.rs` into smaller
//! submodules), this file pins the set of type names that must remain
//! reachable through `nomifun_ai_agent::manager::acp`. It proves nothing about
//! behaviour — only that the rename/move did not accidentally drop a public
//! export. Behavioural correctness is guarded by the byte-level diff of the
//! moved function bodies and by the stage's new targeted tests.
#![allow(dead_code, unused_imports)]

use nomifun_ai_agent::manager::acp::{
    AcpSession, AcpSessionEvent, CatalogForwarder, PermissionRouter, ReconcileAction,
};
use nomifun_ai_agent::session::PersistedSessionState;

fn _surface_probe() {
    let _ = std::any::type_name::<AcpSession>();
    let _ = std::any::type_name::<AcpSessionEvent>();
    let _ = std::any::type_name::<CatalogForwarder>();
    let _ = std::any::type_name::<PermissionRouter>();
    let _ = std::any::type_name::<PersistedSessionState>();
    let _ = std::any::type_name::<ReconcileAction>();
}

#[test]
fn public_surface_compiles() {
    // The real assertion is that this file compiled at all.
}
