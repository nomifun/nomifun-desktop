//! Human takeover / watch-mode: pause the agent at a sensitive step, surface
//! the request to the user, and resume only on an explicit confirmation.
//!
//! This is ALSO the **security-critical out-of-band approval channel** for
//! irreversible actions under yolo/companion sessions. [`TakeoverResolution::Confirmed`]
//! is the ONLY value that sets `out_of_band_confirmed=true` for [`crate::redline::enforce_redline`].
//! All other outcomes (Cancelled, TimedOut, Unavailable) are **fail-closed** — the
//! irreversible action stays Blocked.
//!
//! # Architecture (Phase D)
//!
//! Out-of-band confirmation is owned by the injected
//! [`crate::approval::BrowserApprovalGate`] (desktop event + `ToolApprovalManager`,
//! or the gateway confirm channel): it notifies the user, awaits their decision,
//! and times out fail-closed. The [`TakeoverController`] kept here is the armed/
//! disarmed switch for that path (`enabled`, flipped by
//! `BrowserTool::with_approval_gate`) plus a test seam (`force_resolution`).
//! Without a gate there is no UI able to resolve a takeover, so
//! [`TakeoverController::resolve_without_ui`] fail-closes immediately.
//!
//! On resume after a confirmed takeover, the facade **re-observes** to rebuild
//! the aria-ref generation (the user may have navigated), so subsequent refs
//! are valid.

/// Why a takeover was requested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TakeoverReason {
    /// An irreversible action needs out-of-band human confirmation (redline gate).
    IrreversibleAction { action: String, description: String },
    /// A login wall / CAPTCHA / 2FA that the agent cannot handle.
    LoginWall { hint: String },
    /// Generic manual intervention request.
    Manual { hint: String },
}

/// The outcome of a takeover request.
///
/// **Security keystone**: ONLY [`TakeoverResolution::Confirmed`] maps to `confirmed=true`.
/// Every other variant is fail-closed (`confirmed=false`). A timeout or cancel MUST
/// never auto-confirm — the irreversible action stays Blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeoverResolution {
    /// User explicitly confirmed ("done" / approved the action).
    Confirmed,
    /// User explicitly cancelled.
    Cancelled,
    /// The takeover timed out without user action.
    TimedOut,
    /// Takeover could not be presented (headless, no display, feature disabled).
    Unavailable,
}

impl TakeoverResolution {
    /// Map to the `out_of_band_confirmed` boolean for [`crate::redline::enforce_redline`].
    ///
    /// **ONLY [`TakeoverResolution::Confirmed`] returns `true`**. All other outcomes
    /// (Cancelled, TimedOut, Unavailable) return `false` — fail-closed. This is the
    /// security keystone: a timeout or user-cancel MUST NOT release an irreversible action.
    pub fn to_confirmed(self) -> bool {
        matches!(self, TakeoverResolution::Confirmed)
    }
}

/// Arms the human-takeover path of the redline gate for a browser session.
///
/// The controller is created per-session, disabled by default (fail-closed).
/// [`crate::BrowserTool::with_approval_gate`] flips `enabled` when the trusted
/// host injects an approval gate; the gate itself owns notify/await/timeout.
pub struct TakeoverController {
    /// Whether takeover is enabled for this session. When `false`, requests
    /// resolve to [`TakeoverResolution::Unavailable`] (fail-closed default OFF).
    pub enabled: bool,
    /// **Test seam**: when `Some`, [`Self::resolve_without_ui`] returns this
    /// value. Production code leaves this `None` (fail-closed without a gate).
    pub force_resolution: Option<TakeoverResolution>,
}

impl TakeoverController {
    /// Create a new controller. `enabled` defaults to `false` (fail-closed: the feature
    /// must be explicitly opted in via client preferences).
    pub fn new() -> Self {
        Self {
            enabled: false,
            force_resolution: None,
        }
    }

    /// Resolve a takeover request when no approval gate is wired.
    ///
    /// Without a gate there is no UI able to answer the request, so this
    /// fail-closes **immediately** ([`TakeoverResolution::Unavailable`])
    /// instead of holding the action open until a timeout. Tests inject a
    /// predetermined outcome via [`Self::force_resolution`].
    pub fn resolve_without_ui(&self) -> TakeoverResolution {
        if !self.enabled {
            return TakeoverResolution::Unavailable;
        }
        self.force_resolution
            .unwrap_or(TakeoverResolution::Unavailable)
    }
}

impl Default for TakeoverController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Task 1: resolution→confirmed mapping (fail-closed keystone) ──────────

    #[test]
    fn resolution_maps_failclosed() {
        // ONLY Confirmed → true; everything else → false (fail-closed).
        assert!(
            TakeoverResolution::Confirmed.to_confirmed(),
            "Confirmed must map to confirmed=true"
        );
        assert!(
            !TakeoverResolution::Cancelled.to_confirmed(),
            "Cancelled must map to confirmed=false (fail-closed)"
        );
        assert!(
            !TakeoverResolution::TimedOut.to_confirmed(),
            "TimedOut must map to confirmed=false (fail-closed)"
        );
        assert!(
            !TakeoverResolution::Unavailable.to_confirmed(),
            "Unavailable must map to confirmed=false (fail-closed)"
        );
    }

    // ── Task 2: gate-less resolution is immediate and fail-closed ────────────

    #[test]
    fn disabled_controller_resolves_unavailable() {
        let controller = TakeoverController::new();
        // enabled defaults to false.
        assert!(!controller.enabled);
        let resolution = controller.resolve_without_ui();
        assert_eq!(resolution, TakeoverResolution::Unavailable);
        assert!(!resolution.to_confirmed(), "Unavailable must be fail-closed");
    }

    #[test]
    fn enabled_controller_without_ui_fails_closed_unless_forced() {
        let mut controller = TakeoverController::new();
        controller.enabled = true;
        // No UI can resolve the request → immediate fail-closed.
        assert_eq!(
            controller.resolve_without_ui(),
            TakeoverResolution::Unavailable
        );
        assert!(!controller.resolve_without_ui().to_confirmed());

        // The test seam injects a predetermined outcome.
        controller.force_resolution = Some(TakeoverResolution::Confirmed);
        assert_eq!(
            controller.resolve_without_ui(),
            TakeoverResolution::Confirmed
        );
        controller.force_resolution = Some(TakeoverResolution::Cancelled);
        assert!(!controller.resolve_without_ui().to_confirmed());
    }
}
