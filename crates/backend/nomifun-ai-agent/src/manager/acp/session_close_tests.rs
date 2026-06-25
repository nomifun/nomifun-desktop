//! Close-reason lifecycle tests for `AcpSession`.
//!
//! Pulled out of `session.rs` to keep that file under the 1000-line
//! per-file budget while keeping the assertions co-located with the
//! `record_close_reason` / `last_close_reason` / `take_close_reason`
//! API they exercise. Included via `#[path] mod` from the `tests`
//! module in `session.rs`, so `super::*` resolves to that module's
//! scope (which already has `make_session`, `CloseReason`, etc).

use super::*;

#[test]
fn close_reason_defaults_to_none() {
    let session = make_session();
    assert!(session.last_close_reason().is_none());
}

#[test]
fn record_close_reason_round_trip() {
    let mut session = make_session();
    session.record_close_reason(Some(CloseReason::UserCancel));
    assert_eq!(session.last_close_reason(), Some(&CloseReason::UserCancel));
}

#[test]
fn take_close_reason_is_destructive() {
    let mut session = make_session();
    session.record_close_reason(Some(CloseReason::UserCancel));
    assert!(session.take_close_reason().is_some());
    assert!(session.take_close_reason().is_none());
}

#[test]
fn record_close_reason_overwrites() {
    let mut session = make_session();
    session.record_close_reason(Some(CloseReason::Failed { display: "a".into() }));
    session.record_close_reason(Some(CloseReason::Failed { display: "b".into() }));
    match session.last_close_reason() {
        Some(CloseReason::Failed { display }) => assert_eq!(display, "b"),
        other => panic!("expected Failed{{b}}, got {other:?}"),
    }
}

#[test]
fn clear_session_id_clears_close_reason() {
    let mut session = make_session();
    session.set_session_id(SessionId::new("ses-stale"));
    session.record_close_reason(Some(CloseReason::ProcessExited {
        exit_code: Some(1),
        signal: None,
        redacted_summary: String::new(),
    }));
    session.clear_session_id();
    assert!(
        session.last_close_reason().is_none(),
        "rebuilt session must not inherit the prior turn's close reason"
    );
}
