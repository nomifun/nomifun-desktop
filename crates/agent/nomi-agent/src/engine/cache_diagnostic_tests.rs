use super::cache_diagnostic_message;
use crate::cache_diagnostics::{CacheBreakCause, CacheDiagnostic};

#[test]
fn full_miss_is_silent_by_default_and_never_an_error() {
    // A full cache miss — including a benign server-side TTL expiry during the
    // idle gap between AutoWork turns — must NOT surface unless diagnostics are
    // explicitly enabled, and must NEVER be an error. Before the fix this path
    // called emit_error, which the AutoWork runner treated as a FAILED
    // turn (re-pend / tag pause).
    let diag = CacheDiagnostic::FullMiss { cause: CacheBreakCause::TtlExpiry };
    assert_eq!(cache_diagnostic_message(&diag, false), None);
}

#[test]
fn full_miss_surfaces_as_info_text_when_diagnostics_enabled() {
    let diag = CacheDiagnostic::FullMiss { cause: CacheBreakCause::TtlExpiry };
    assert_eq!(
        cache_diagnostic_message(&diag, true).as_deref(),
        Some("Cache full miss: TtlExpiry")
    );
}

#[test]
fn healthy_and_partial_are_gated_by_the_flag() {
    let healthy = CacheDiagnostic::Healthy { hit_rate: 0.9 };
    assert_eq!(cache_diagnostic_message(&healthy, false), None);
    assert!(cache_diagnostic_message(&healthy, true).is_some());

    let partial = CacheDiagnostic::PartialMiss { hit_rate: 0.5, cause: CacheBreakCause::TtlExpiry };
    assert_eq!(cache_diagnostic_message(&partial, false), None);
    assert!(cache_diagnostic_message(&partial, true).is_some());
}
