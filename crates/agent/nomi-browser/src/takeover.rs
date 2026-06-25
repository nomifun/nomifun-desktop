//! Human takeover / watch-mode: pause the agent at a sensitive step, surface a
//! headful live window for the user, await their resolution, then resume.
//!
//! This is ALSO the **security-critical out-of-band approval channel** for
//! irreversible actions under yolo/companion sessions. [`TakeoverResolution::Confirmed`]
//! is the ONLY value that sets `out_of_band_confirmed=true` for [`crate::redline::enforce_redline`].
//! All other outcomes (Cancelled, TimedOut, Unavailable) are **fail-closed** — the
//! irreversible action stays Blocked.
//!
//! # Architecture
//!
//! A [`TakeoverController`] (facade level) exposes [`TakeoverController::request`] that:
//! 1. Ensures Chrome is headful & the window is visible/foregrounded (engine seam).
//! 2. Emits a UI event ("human takeover requested: <reason>") to the desktop.
//! 3. Awaits a resolution (user clicks "done" / "cancel" / timeout).
//!
//! On resume, the facade **re-observes** to rebuild the aria-ref generation (the user
//! may have navigated), so subsequent refs are valid.

use std::time::Duration;
use tokio::sync::oneshot;

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

/// A handle to an in-flight takeover request. The holder can resolve it from the UI side.
pub struct TakeoverHandle {
    tx: oneshot::Sender<TakeoverResolution>,
}

impl TakeoverHandle {
    /// Resolve the takeover from the UI side (user clicked "done" or "cancel").
    /// Returns `Err` if the receiver was already dropped (timeout fired first).
    pub fn resolve(self, resolution: TakeoverResolution) -> Result<(), TakeoverResolution> {
        self.tx.send(resolution)
    }
}

/// Controls human takeover requests for a browser session.
///
/// The controller is created per-session. When a takeover is needed, [`Self::request`]
/// returns a future that resolves to [`TakeoverResolution`] (either from the UI via
/// [`TakeoverHandle::resolve`] or from a timeout).
pub struct TakeoverController {
    /// Default timeout for a takeover request. If the user does not act within this
    /// duration, the takeover resolves to [`TakeoverResolution::TimedOut`] (fail-closed).
    pub timeout: Duration,
    /// Whether takeover is enabled for this session. When `false`, all requests
    /// immediately resolve to [`TakeoverResolution::Unavailable`] (fail-closed default OFF).
    pub enabled: bool,
    /// **Test seam**: when `Some`, all requests immediately resolve to this value
    /// (bypassing the oneshot/timeout mechanism). Production code leaves this `None`.
    /// Tests set it to inject a predetermined resolution.
    pub force_resolution: Option<TakeoverResolution>,
}

impl TakeoverController {
    /// Create a new controller. `enabled` defaults to `false` (fail-closed: the feature
    /// must be explicitly opted in via client preferences).
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            enabled: false,
            force_resolution: None,
        }
    }

    /// Request a human takeover. Returns `(TakeoverHandle, impl Future<Output=TakeoverResolution>)`.
    ///
    /// The caller awaits the future; the UI side resolves via the handle.
    /// If `self.enabled == false`, returns `Unavailable` immediately (no handle needed).
    /// If `force_resolution` is set (test seam), returns that immediately.
    /// If the timeout fires before the handle resolves, returns `TimedOut`.
    pub fn request(
        &self,
        _reason: TakeoverReason,
    ) -> TakeoverRequest {
        if !self.enabled {
            return TakeoverRequest::Immediate(TakeoverResolution::Unavailable);
        }
        if let Some(forced) = self.force_resolution {
            return TakeoverRequest::Immediate(forced);
        }
        let (tx, rx) = oneshot::channel();
        let handle = TakeoverHandle { tx };
        let timeout = self.timeout;
        TakeoverRequest::Pending { handle, rx, timeout }
    }
}

/// The result of [`TakeoverController::request`]. Either immediately resolved
/// (feature disabled / headless) or pending user action.
pub enum TakeoverRequest {
    /// Resolved immediately without needing user action.
    Immediate(TakeoverResolution),
    /// Awaiting user action via the handle, with a timeout.
    Pending {
        handle: TakeoverHandle,
        rx: oneshot::Receiver<TakeoverResolution>,
        timeout: Duration,
    },
}

impl TakeoverRequest {
    /// Consume this request: if `Immediate`, return the resolution; if `Pending`,
    /// split into the handle (for the UI) and a future that resolves to the outcome.
    /// The caller must give the handle to the UI layer and await the future.
    pub fn split(self) -> (Option<TakeoverHandle>, TakeoverRequestFuture) {
        match self {
            TakeoverRequest::Immediate(res) => {
                (None, TakeoverRequestFuture::Ready(res))
            }
            TakeoverRequest::Pending { handle, rx, timeout } => {
                (Some(handle), TakeoverRequestFuture::Awaiting { rx, timeout })
            }
        }
    }
}

/// A future that resolves to [`TakeoverResolution`].
pub enum TakeoverRequestFuture {
    Ready(TakeoverResolution),
    Awaiting {
        rx: oneshot::Receiver<TakeoverResolution>,
        timeout: Duration,
    },
}

impl TakeoverRequestFuture {
    /// Await the resolution (with timeout).
    pub async fn resolve(self) -> TakeoverResolution {
        match self {
            TakeoverRequestFuture::Ready(res) => res,
            TakeoverRequestFuture::Awaiting { rx, timeout } => {
                match tokio::time::timeout(timeout, rx).await {
                    Ok(Ok(resolution)) => resolution,
                    Ok(Err(_)) => {
                        // Sender dropped without sending — treat as cancelled.
                        TakeoverResolution::Cancelled
                    }
                    Err(_) => {
                        // Timeout elapsed — fail-closed.
                        TakeoverResolution::TimedOut
                    }
                }
            }
        }
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

    // ── Task 2: TakeoverController request/await with timeout ────────────────

    #[tokio::test]
    async fn request_times_out_to_failclosed() {
        tokio::time::pause();
        let controller = TakeoverController {
            timeout: Duration::from_millis(50),
            enabled: true,
            force_resolution: None,
        };
        let req = controller.request(TakeoverReason::IrreversibleAction {
            action: "click".into(),
            description: "Pay $100".into(),
        });
        let (handle, future) = req.split();
        assert!(handle.is_some(), "enabled controller should yield a handle");
        // Do NOT resolve — keep the handle alive but idle so the timeout fires.
        let _keep_alive = handle;
        // Advance time past the timeout.
        let resolution = future.resolve().await;
        assert_eq!(resolution, TakeoverResolution::TimedOut);
        assert!(!resolution.to_confirmed(), "TimedOut must be fail-closed");
    }

    #[tokio::test]
    async fn request_confirmed_resolves_true() {
        let controller = TakeoverController {
            timeout: Duration::from_secs(60),
            enabled: true,
            force_resolution: None,
        };
        let req = controller.request(TakeoverReason::Manual {
            hint: "test".into(),
        });
        let (handle, future) = req.split();
        let handle = handle.unwrap();
        handle.resolve(TakeoverResolution::Confirmed).unwrap();
        let resolution = future.resolve().await;
        assert_eq!(resolution, TakeoverResolution::Confirmed);
        assert!(resolution.to_confirmed());
    }

    #[tokio::test]
    async fn request_cancelled_resolves_false() {
        let controller = TakeoverController {
            timeout: Duration::from_secs(60),
            enabled: true,
            force_resolution: None,
        };
        let req = controller.request(TakeoverReason::Manual {
            hint: "test".into(),
        });
        let (handle, future) = req.split();
        let handle = handle.unwrap();
        handle.resolve(TakeoverResolution::Cancelled).unwrap();
        let resolution = future.resolve().await;
        assert_eq!(resolution, TakeoverResolution::Cancelled);
        assert!(!resolution.to_confirmed());
    }

    #[tokio::test]
    async fn disabled_controller_returns_unavailable_immediately() {
        let controller = TakeoverController::new(Duration::from_secs(60));
        // enabled defaults to false.
        assert!(!controller.enabled);
        let req = controller.request(TakeoverReason::IrreversibleAction {
            action: "click".into(),
            description: "Delete account".into(),
        });
        let (handle, future) = req.split();
        assert!(handle.is_none(), "disabled controller yields no handle");
        let resolution = future.resolve().await;
        assert_eq!(resolution, TakeoverResolution::Unavailable);
        assert!(!resolution.to_confirmed(), "Unavailable must be fail-closed");
    }

    #[tokio::test]
    async fn handle_dropped_without_resolving_yields_cancelled() {
        let controller = TakeoverController {
            timeout: Duration::from_secs(60),
            enabled: true,
            force_resolution: None,
        };
        let req = controller.request(TakeoverReason::Manual {
            hint: "test".into(),
        });
        let (handle, future) = req.split();
        // Drop the handle without resolving — sender gone.
        drop(handle);
        let resolution = future.resolve().await;
        assert_eq!(resolution, TakeoverResolution::Cancelled);
        assert!(!resolution.to_confirmed());
    }
}
