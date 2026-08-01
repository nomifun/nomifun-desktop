//! One-directional integration seam between AutoWork and IDMM.
//!
//! AutoWork (this crate) drives turn execution; IDMM (the `nomifun-idmm` crate)
//! supervises a session for stalls. To let AutoWork ensure a session is being
//! supervised while a turn runs — WITHOUT this crate depending on `nomifun-idmm`
//! (which would be a cycle, since idmm conceptually sits above requirement) —
//! AutoWork defines this trait and holds an optional handle. `nomifun-idmm`
//! implements it; `nomifun-app` injects the implementation at assembly time.

use nomifun_api_types::AutoWorkTargetKind;

/// Implemented by `nomifun-idmm::IdmmManager`. AutoWork calls
/// `ensure_supervising` at the top of each loop iteration so that, if the user
/// enabled IDMM for this target, supervision is (idempotently) running while the
/// turn executes. The implementation resolves the session owner and config
/// internally; this call is cheap and a no-op when IDMM is disabled or already
/// supervising the target.
pub trait IdmmHandle: Send + Sync {
    fn ensure_supervising(&self, kind: AutoWorkTargetKind, target_id: &str);
}
