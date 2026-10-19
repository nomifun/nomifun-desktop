//! Platform-neutral engine types + the `A11yEngine` trait every OS backend
//! implements.

use serde::{Deserialize, Serialize};

/// Monotonic snapshot generation. A `ref` (index into a snapshot's element
/// list) is only valid against the generation it was produced in; backends use
/// this to reject stale references instead of acting on a moved element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnapshotGen(pub u64);

/// An opaque, generation-tagged handle to an element in a backend's registry.
/// The raw OS handle (AXUIElement / IUIAutomationElement / AT-SPI Accessible)
/// never crosses the engine boundary — only this token and serializable data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementId {
    pub generation: SnapshotGen,
    pub index: u32,
}

/// A rectangle. Backends return element bounds in **OS accessibility
/// coordinates** (e.g. macOS global screen points, top-left origin); mapping to
/// screenshot-pixel space for pixel fallback is the caller's job.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    /// Choose an integer screen point near the OS bounds' center, inside their
    /// half-open area and the native input's i32 range. Reject geometry with no
    /// representable interior point instead of truncating onto a neighbor.
    pub fn pixel_center(&self) -> Option<(i32, i32)> {
        fn axis_point(origin: f64, extent: f64) -> Option<i32> {
            if !origin.is_finite() || !extent.is_finite() || extent <= 0.0 { return None; }
            let end = origin + extent;
            if !end.is_finite() || end <= origin { return None; }
            let first = origin.ceil().max(f64::from(i32::MIN));
            let last = (end.ceil() - 1.0).min(f64::from(i32::MAX));
            if first > last { return None; }
            let point = (origin + extent / 2.0).floor().clamp(first, last);
            if !point.is_finite() || point < origin || point >= end { return None; }
            Some(point as i32)
        }
        Some((axis_point(self.x, self.w)?, axis_point(self.y, self.h)?))
    }
}

/// Where an element entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Native accessibility tree (most reliable).
    A11y,
    /// OCR text recognition (fallback where a11y is thin).
    Ocr,
    /// Vision/icon classification (fallback).
    Vision,
}

/// One interactable element exposed to the model as `[ref]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementEntry {
    /// The number the model targets: "click element [ref]".
    pub r#ref: u32,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    pub bounds: Rect,
    pub source: Source,
}

/// How synthetic input is delivered on this platform/session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// Native event posting (macOS CGEvent / Windows SendInput / AT-SPI action).
    Native,
    /// X11 XTest.
    X11,
    /// Wayland xdg-desktop-portal RemoteDesktop (per-session consent).
    WaylandPortal,
    /// No reliable synthetic-input path in this session.
    Unsupported,
}

/// What the engine can actually do this session — injected into the system
/// prompt so the model knows its real abilities up front.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub os: String,
    /// Can read the accessibility tree (`observe`).
    pub tree_read: bool,
    /// Can capture a screenshot.
    pub screenshot: bool,
    /// Can perform semantic actions (AXPress / Invoke / do_action) on elements.
    pub semantic_action: bool,
    pub synthetic_input: InputKind,
    /// Can move/resize/raise windows.
    pub window_management: bool,
}

/// A completed `observe`: the filtered interactable elements and their
/// indented text rendering for the model.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub generation: SnapshotGen,
    pub entries: Vec<ElementEntry>,
    /// Indented text rendering: `[14] button "Submit" enabled`.
    pub text: String,
    /// True if the tree exceeded the node budget and was truncated.
    pub truncated: bool,
    /// Process id of the observed application (for `focus_window`).
    pub pid: Option<i32>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
}

/// How the model addresses an element. Two mutually-exclusive modes, shared
/// with the browser tool's contract.
#[derive(Debug, Clone)]
pub enum Target {
    /// A `[ref]` from the most recent snapshot.
    Ref(u32),
    /// Last-resort absolute screen coordinates (pixel fallback).
    Pixel { x: i32, y: i32 },
}

/// A semantic action to perform on a resolved element.
#[derive(Debug, Clone)]
pub enum ElementAction {
    /// The element's default action (AXPress / Invoke / do_action).
    Press,
    LeftClick,
    RightClick,
    DoubleClick,
    Focus,
    SetValue(String),
}

/// The observed effect of an action, for closed-loop verification.
#[derive(Debug, Clone)]
pub struct Effect {
    pub changed: bool,
    pub message: String,
}

/// Options controlling an `observe` tree walk.
#[derive(Debug, Clone)]
pub struct ObserveOpts {
    /// Maximum tree depth to traverse.
    pub max_depth: usize,
    /// Stop after this many interactable elements (then set `truncated`).
    pub node_budget: usize,
    /// Restrict to a specific process; `None` = the frontmost app.
    pub pid: Option<i32>,
}

impl Default for ObserveOpts {
    fn default() -> Self {
        Self {
            max_depth: 12,
            node_budget: 120,
            pid: None,
        }
    }
}

/// Errors are data the model reads and routes around — never a panic, never a
/// silent no-op.
#[derive(Debug, thiserror::Error)]
pub enum A11yError {
    #[error("accessibility operation cancelled before native dispatch")]
    CancelledBeforeDispatch,
    /// The requested capability/action is unavailable before any mutation was
    /// admitted. Only this explicit rejection can authorize a guarded input
    /// fallback; an arbitrary backend error does not prove zero effect.
    #[error("not supported ({capability}): {hint}")]
    Unsupported { capability: String, hint: String },
    #[error("element not found: {0}")]
    NotFound(String),
    #[error("stale reference: {0}")]
    Stale(String),
    #[error("permission required: {0}")]
    Permission(String),
    /// A mutating request reached the native API, but its acknowledgement was
    /// lost or timed out. The effect may have occurred; callers must record the
    /// uncertainty and observe again without automatically retrying the action.
    #[error("accessibility action effect is uncertain: {0}")]
    Uncertain(String),
    #[error("accessibility backend error: {0}")]
    Backend(String),
}

/// The contract every OS backend implements. Methods are synchronous; callers
/// invoke them from `spawn_blocking`. macOS marshals each call to a single
/// CFRunLoop actor thread internally, so the engine is `Send + Sync`.
pub trait A11yEngine: Send + Sync {
    /// Honest report of what this session can do.
    fn capabilities(&self) -> Capabilities;

    /// Read the frontmost (or `opts.pid`) window's accessibility tree, filter to
    /// interactable elements, and return them as a numbered element
    /// snapshot. Element `bounds` are in OS accessibility coordinates.
    fn observe(&self, opts: &ObserveOpts) -> Result<Snapshot, A11yError>;

    /// Resolve a retained element for a pixel gesture without performing any
    /// semantic action. Implementations must validate the snapshot generation,
    /// owning window and live element before returning OS screen coordinates.
    /// An unavailable validator does not authorize use of cached coordinates.
    fn resolve_pixel_target(
        &self,
        _target: &Target,
        _generation: SnapshotGen,
    ) -> Result<Rect, A11yError> {
        Err(A11yError::Unsupported {
            capability: "pixel gesture target validation".to_string(),
            hint: "This accessibility backend cannot validate a retained ref for pixel input.".to_string(),
        })
    }

    /// Perform `action` on the element addressed by `target`. `Ref` targets are
    /// validated against `generation` and rejected if stale.
    fn invoke(
        &self,
        target: &Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError>;

    /// Retain the same request/result owner. A queued mutator may be withdrawn
    /// before its actor begins native work; an admitted request is still joined.
    fn invoke_controlled(&self, target: &Target, generation: SnapshotGen, action: ElementAction,
        cancellation: &tokio_util::sync::CancellationToken) -> Result<Effect, A11yError> {
        if cancellation.is_cancelled() { return Err(A11yError::CancelledBeforeDispatch); }
        self.invoke(target, generation, action)
    }

    /// Raise/activate a window by its owning process id.
    fn focus_window(&self, pid: i32) -> Result<Effect, A11yError>;
}

#[cfg(test)]
mod rect_tests {
    use super::Rect;

    #[test]
    fn pixel_center_keeps_negative_one_pixel_point_inside_half_open_bounds() {
        let bounds = Rect { x: -1.0, y: 10.0, w: 1.0, h: 1.0 };
        assert_eq!(bounds.pixel_center(), Some((-1, 10)),
            "MAC38 integer pointer target must remain inside [-1, 0), not truncate -0.5 to 0");
    }

    #[test]
    fn pixel_center_rejects_subpixel_bounds_without_any_integer_point() {
        for bounds in [
            Rect { x: 0.2, y: 0.2, w: 0.2, h: 0.2 },
            Rect { x: -0.4, y: -0.4, w: 0.2, h: 0.2 },
        ] {
            assert_eq!(bounds.pixel_center(), None,
                "MAC38 an integer input point must actually exist inside the half-open bounds: {bounds:?}");
        }
    }

    #[test]
    fn pixel_center_preserves_negative_origins_and_rejects_clamped_geometry() {
        let bounds = Rect { x: -1200.0, y: 42.0, w: 80.0, h: 20.0 };
        assert_eq!(bounds.pixel_center(), Some((-1160, 52)));
        for invalid in [
            Rect { x: f64::NAN, ..bounds },
            Rect { w: f64::INFINITY, ..bounds },
            Rect { x: f64::MAX, w: f64::MAX, ..bounds },
            Rect { x: f64::from(i32::MAX) + 1.0, ..bounds },
            Rect { x: f64::from(i32::MIN) - 100.0, ..bounds },
            Rect { h: 0.0, ..bounds },
        ] {
            assert_eq!(invalid.pixel_center(), None, "{invalid:?}");
        }
        assert_eq!(Rect { x: f64::from(i32::MAX), ..bounds }.pixel_center(), Some((i32::MAX, 52)));
        let subpixel_with_integer = Rect { x: -0.2, y: 0.2, w: 0.8, h: 1.2 };
        assert_eq!(subpixel_with_integer.pixel_center(), Some((0, 1)));
    }
}
