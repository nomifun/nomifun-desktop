//! Platform-neutral engine types + the `A11yEngine` trait every OS backend
//! implements.

use serde::{Deserialize, Serialize};

use nomi_types::tool::ToolImage;

use crate::selector::Selector;

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
/// screenshot-pixel space for overlays/pixel-fallback is the caller's job (see
/// the design's AX-points→pixel conversion).
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
}

/// Where an element entry came from. Set-of-Marks fuses these.
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

/// A line of text recognized by OCR, with bounds in screenshot-pixel space
/// (top-left origin). Fused into the Set-of-Marks list where the accessibility
/// tree is thin (Electron/canvas/games).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrLine {
    pub text: String,
    pub bounds: Rect,
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
    /// Can capture a screenshot for the Set-of-Marks overlay.
    pub screenshot: bool,
    /// Can perform semantic actions (AXPress / Invoke / do_action) on elements.
    pub semantic_action: bool,
    pub synthetic_input: InputKind,
    /// Can move/resize/raise windows.
    pub window_management: bool,
}

/// A completed `observe`: the filtered interactable elements + an optional
/// Set-of-Marks overlay image, plus the indented text rendering for the model.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub generation: SnapshotGen,
    pub entries: Vec<ElementEntry>,
    /// Set-of-Marks overlay (numbered boxes on the screenshot), when produced.
    pub overlay: Option<ToolImage>,
    /// Indented text rendering: `[14] button "Submit" enabled`.
    pub text: String,
    /// True if the tree exceeded the node budget and was truncated.
    pub truncated: bool,
    /// Process id of the observed application (for `focus_window`).
    pub pid: Option<i32>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
}

/// How the model addresses an element. Three mutually-exclusive modes, shared
/// with the browser tool's contract.
#[derive(Debug, Clone)]
pub enum Target {
    /// A `[ref]` from the most recent snapshot.
    Ref(u32),
    /// A deterministic selector (`role:Button && name:Save`).
    Selector(Selector),
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
    #[error("not supported ({capability}): {hint}")]
    Unsupported { capability: String, hint: String },
    #[error("element not found: {0}")]
    NotFound(String),
    #[error("stale reference: {0}")]
    Stale(String),
    #[error("permission required: {0}")]
    Permission(String),
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
    /// interactable elements, and return them numbered as a Set-of-Marks
    /// snapshot. Element `bounds` are in OS accessibility coordinates.
    fn observe(&self, opts: &ObserveOpts) -> Result<Snapshot, A11yError>;

    /// Perform `action` on the element addressed by `target`. `Ref` targets are
    /// validated against `generation` and rejected if stale.
    fn invoke(
        &self,
        target: &Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError>;

    /// Raise/activate a window by its owning process id.
    fn focus_window(&self, pid: i32) -> Result<Effect, A11yError>;
}
