//! The Computer tool: screenshot, mouse/keyboard synthesis, window control.

use std::sync::Mutex;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use nomi_a11y::{
    A11yEngine, A11yError, ElementAction, ElementEntry, ObserveOpts, SnapshotGen, Target,
};
use nomi_config::config::ComputerConfig;
use nomi_types::tool::ToolResult;

use crate::input::{self, ScrollDirection};
use crate::keys::parse_key_combo;
use crate::scale::{map_llm_coord, map_screen_coord};
use crate::screen::{
    CANONICAL_SCREENSHOT_PNG_BYTES, CaptureGeometry, capture_screen,
    encode_png_with_limit,
};
use crate::fallback_backend;

const MAX_WAIT_SECONDS: f64 = 5.0;
const DEFAULT_SCROLL_AMOUNT: i64 = 3;

type GestureFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>,
>;

/// Injection boundary for ref-addressed pointer gestures. The production port
/// forwards native clicks unchanged; tests substitute a recorder so even a
/// failed safety counterexample cannot send native input.
trait GesturePort: Send + Sync {
    fn click(&self, x: i32, y: i32, button: enigo::Button, count: u32) -> GestureFuture<'_>;
}

struct NativeGesturePort;

impl GesturePort for NativeGesturePort {
    fn click(&self, x: i32, y: i32, button: enigo::Button, count: u32) -> GestureFuture<'_> {
        Box::pin(input::click(x, y, button, count))
    }

}

const UNSUPPORTED_VALUE_INPUT_REJECTION: &str = "COMPUTER_INPUT_UNSUPPORTED_NO_EFFECT: set_element_value was rejected before native mutation. No pointer or keyboard fallback was admitted.";

/// The exact first-party marker for a whole-value setter rejection before any
/// native mutation. A backend hint containing this text is not this marker.
pub fn is_proven_unsupported_input_rejection(result: &ToolResult) -> bool {
    result.is_error && result.images.is_empty() && result.content == UNSUPPORTED_VALUE_INPUT_REJECTION
}

/// True only for the exact semantic-ref stale rejection produced before the
/// Accessibility backend performs an action. The message also proves that the
/// Computer layer did not use its pixel fallback, so a role owner may settle
/// the reserved external effect as rejected instead of outcome-unknown.
pub fn is_proven_stale_input_rejection(result: &ToolResult) -> bool {
    if !result.is_error || !result.images.is_empty() { return false; }
    let Some(body) = result.content.strip_prefix("Accessibility action on [") else { return false; };
    let Some((reference, category)) = body.split_once(']') else { return false; };
    let Ok(reference_number) = reference.parse::<u32>() else { return false; };
    if reference != reference_number.to_string() { return false; }
    let Some(reason) = category.strip_prefix(" failed: stale reference: ") else { return false; };
    let Some(reason) = reason.strip_suffix(
        ". No pixel fallback was performed; re-run observe before using element refs.",
    ) else { return false; };
    !reason.is_empty()
}

/// Example key combo for the platform we are compiled for. The accelerator
/// modifier differs by OS (Command on macOS, Control on Windows/Linux), so we
/// steer the model toward the right one instead of always suggesting `cmd`,
/// which is a macOS idiom. `parse_key_combo` still accepts `cmd` everywhere and
/// remaps it per-platform, but a correct example reduces wrong presses.
#[cfg(target_os = "macos")]
const KEY_COMBO_EXAMPLE: &str = "cmd+shift+t";
#[cfg(not(target_os = "macos"))]
const KEY_COMBO_EXAMPLE: &str = "ctrl+shift+t";

pub struct ComputerTool {
    max_screenshot_edge: u32,
    /// Geometry of the most recent screenshot; pointer coordinates from the
    /// model are interpreted in that image's pixel space.
    last_capture: Mutex<Option<CaptureGeometry>>,
    /// Lazily-initialized accessibility engine (a11y-first targeting). `Some(Err)`
    /// caches an unavailable backend (an OS without an a11y engine, or a startup
    /// failure) so we don't retry per call.
    a11y: Mutex<Option<Result<Arc<dyn A11yEngine>, String>>>,
    /// The most recent `observe` snapshot, for resolving `[ref]` actions. Element
    /// bounds here are in OS accessibility coordinates (screen logical points).
    last_snapshot: Mutex<Option<SnapshotCache>>,
    gesture: Arc<dyn GesturePort>,
}

struct SnapshotCache {
    generation: SnapshotGen,
    entries: Vec<CachedEntry>,
}

#[derive(Clone)]
struct CachedEntry {
    /// What the model sees: display `[ref]`, role/name, accessibility bounds, source.
    display: ElementEntry,
    engine_ref: u32,
}

impl ComputerTool {
    pub fn new(config: &ComputerConfig) -> Self {
        Self {
            max_screenshot_edge: config.max_screenshot_edge,
            last_capture: Mutex::new(None),
            a11y: Mutex::new(None),
            last_snapshot: Mutex::new(None),
            gesture: Arc::new(NativeGesturePort),
        }
    }

    /// Retire cached pixels and element refs without touching any device. Role
    /// owners use this at observation/settlement boundaries; an admitted input
    /// keeps its geometry until it has settled.
    pub fn clear_cached_observation(&self) {
        *self.last_snapshot.lock().unwrap_or_else(|p| p.into_inner()) = None;
        *self.last_capture.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Execute one native operation under an exact canonical `computer`
    /// action grant. The caller supplies the Action ID frozen into the Agent
    /// snapshot; a mismatched native operation is rejected before any OS API
    /// is touched.
    pub async fn execute_authorized(&self, action_id: &str, input: Value) -> ToolResult {
        let Some(granted_action) = crate::capability::ComputerAction::parse(action_id) else {
            return ToolResult::error(format!(
                "COMPUTER_ACTION_NOT_DECLARED: {action_id:?} is not an action declared by the computer module"
            ));
        };
        let Some(native_operation) = input
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return ToolResult::error(
                "Missing required parameter `action`. See the tool description for the list of supported operations.",
            );
        };
        let Some(required_action) =
            crate::capability::ComputerAction::for_native_operation(&native_operation)
        else {
            return ToolResult::error(format!(
                "Unknown Computer operation {native_operation:?}."
            ));
        };
        if granted_action != required_action {
            return ToolResult::error(format!(
                "COMPUTER_ACTION_NOT_GRANTED: {native_operation:?} requires {}, not {}",
                required_action.id(),
                granted_action.id()
            ));
        }
        if granted_action == crate::capability::ComputerAction::A11yObserve
            && native_operation == "observe"
        {
            // Keep the canonical Accessibility action independent from Screen
            // Recording. The separate computer/observe action owns pixels;
            // attaching a full screenshot here couples TCC grants and can force
            // avoidable model-context compaction for an otherwise small tree.
            return self.do_observe().await;
        }
        if granted_action == crate::capability::ComputerAction::Observe
            && native_operation == "screenshot"
        {
            // The canonical Kernel port is JSON-only until the application
            // restores a typed image part. Bound the PNG before that envelope
            // so high-entropy screens cannot be truncated into invalid JSON.
            return self
                .do_screenshot(&input)
                .await;
        }
        self.execute_native(input, &native_operation).await
    }

    async fn execute_native(&self, input: Value, action: &str) -> ToolResult {
        tracing::debug!(action = %action, "ComputerTool executing");

        match action {
            "click_element" => self.do_click_element(&input).await,
            "set_element_value" => self.do_set_element_value(&input).await,
            "right_click_element" => {
                self.do_element_gesture(&input, enigo::Button::Right, 1, "right-click").await
            }
            "double_click_element" => {
                self.do_element_gesture(&input, enigo::Button::Left, 2, "double-click").await
            }
            "launch" => self.do_launch(&input).await,
            "cursor_position" => self.do_cursor_position().await,
            "list_windows" => self.do_list_windows().await,
            "left_click" => self.do_click(&input, enigo::Button::Left, 1).await,
            "right_click" => self.do_click(&input, enigo::Button::Right, 1).await,
            "middle_click" => self.do_click(&input, enigo::Button::Middle, 1).await,
            "double_click" => self.do_click(&input, enigo::Button::Left, 2).await,
            "triple_click" => self.do_click(&input, enigo::Button::Left, 3).await,
            "mouse_move" => self.do_mouse_move(&input).await,
            "left_click_drag" => self.do_drag(&input).await,
            "type" => self.do_type(&input).await,
            "key" => self.do_key(&input).await,
            "scroll" => self.do_scroll(&input).await,
            "focus_window" => self.do_focus_window(&input).await,
            "wait" => self.do_wait(&input).await,
            other => ToolResult::error(format!("Unknown Computer operation {other:?}.")),
        }
    }

    /// Lazily construct (and cache) the accessibility engine. The error string
    /// is cached too, so an unavailable backend is reported without retrying.
    fn engine(&self) -> Result<Arc<dyn A11yEngine>, String> {
        let mut guard = self.a11y.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some(nomi_a11y::create_engine().map_err(|e| e.to_string()));
        }
        guard.as_ref().unwrap().clone()
    }

    /// Read the accessibility tree without capturing pixels.
    async fn do_observe(&self) -> ToolResult {
        // A failed refresh must not leave old targets actionable.
        self.clear_cached_observation();
        let engine = match self.engine() {
            Ok(e) => e,
            Err(msg) => {
                return ToolResult::error(format!(
                    "Accessibility engine unavailable: {msg} Use the pixel actions \
                     (screenshot + click with x,y) instead."
                ));
            }
        };
        let eng = engine.clone();
        let snap = tokio::task::spawn_blocking(move || eng.observe(&ObserveOpts::default()))
            .await
            .unwrap_or_else(|e| Err(A11yError::Backend(format!("observe task failed: {e}"))));
        let snap = match snap {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("Accessibility observe failed: {e}")),
        };
        let app_note = snap.app_name.as_deref()
            .map(|a| format!(" in {a}")).unwrap_or_default();
        let ax_note = if snap.truncated {
            " (a11y tree truncated to the node budget)"
        } else {
            ""
        };
        let cached: Vec<_> = snap.entries.iter().map(|entry| {
            CachedEntry {
                display: entry.clone(),
                engine_ref: entry.r#ref,
            }
        }).collect();
        let count = cached.len();
        *self.last_snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(SnapshotCache {
            generation: snap.generation,
            entries: cached,
        });
        ToolResult::text(format!(
            "Accessibility snapshot (gen {}): {count} element(s){app_note}{ax_note}. Pixel overlay intentionally omitted by computer/a11y.observe; use the separate computer/observe screenshot action when pixels are required.\n\n{}",
            snap.generation.0, snap.text
        ))
    }

    /// Look up a `[ref]` in the latest snapshot, returning its generation and a
    /// clone of the cached entry (with its action target).
    fn resolve_ref(&self, r: u32) -> Result<(SnapshotGen, CachedEntry), String> {
        let guard = self.last_snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_ref() {
            Some(cache) => match cache.entries.iter().find(|c| c.display.r#ref == r) {
                Some(c) => Ok((cache.generation, c.clone())),
                None => Err(format!(
                    "No element [{r}] in the latest snapshot. Run `observe` and use a ref it lists."
                )),
            },
            None => Err("No accessibility snapshot yet. Run the `observe` action first.".to_string()),
        }
    }

    /// Share semantic invocation and its fallback gate. Lost targets, denied
    /// permission, or a failed task are not permission to click cached pixels.
    async fn act_on_ax<F, Fut>(
        &self,
        r: u32,
        generation: SnapshotGen,
        action: ElementAction,
        fallback: F,
    ) -> ToolResult
    where
        F: FnOnce(A11yError) -> Fut,
        Fut: std::future::Future<Output = ToolResult>,
    {
        let engine = match self.engine() {
            Ok(engine) => engine,
            Err(msg) => return ToolResult::error(format!("Accessibility engine unavailable: {msg}")),
        };
        let result = tokio::task::spawn_blocking(move || {
            engine.invoke(&Target::Ref(r), generation, action)
        })
        .await;
        let error = match result {
            Ok(Ok(effect)) => return ToolResult::text(format!(
                "{}. Run `observe` to verify the result.", effect.message
            )),
            Ok(Err(e @ A11yError::Unsupported { .. })) => {
                return fallback(e).await;
            }
            Ok(Err(e)) => e.to_string(),
            Err(e) => format!("invoke task failed: {e}"),
        };
        *self.last_snapshot.lock().unwrap_or_else(|p| p.into_inner()) = None;
        ToolResult::error(format!(
            "Accessibility action on [{r}] failed: {error}. No pixel fallback was \
             performed; re-run observe before using element refs."
        ))
    }

    /// Act on an element by its `[ref]` from the latest `observe` snapshot.
    /// Accessibility elements use AXPress with a gated pixel-click fallback.
    async fn do_click_element(&self, input: &Value) -> ToolResult {
        let r = match require_u32(input, "ref") {
            Ok(r) => r,
            Err(e) => return ToolResult::error(e),
        };
        let (generation, entry) = match self.resolve_ref(r) {
            Ok(v) => v,
            Err(msg) => return ToolResult::error(msg),
        };

        self.act_on_ax(
            entry.engine_ref,
            generation,
            ElementAction::LeftClick,
            |e| async move {
                let (sx, sy) = match self.validated_ref_screen_center(
                    r, entry.engine_ref, generation, entry.display.bounds,
                ).await {
                    Ok(center) => center,
                    Err(error) => return ToolResult::error(error),
                };
                match self.gesture.click(sx, sy, enigo::Button::Left, 1).await {
                    Ok(()) => ToolResult::text(format!(
                        "The accessibility action on [{r}] did not succeed ({e}); fell \
                         back to a pixel click at the element center. Run `observe` to verify."
                    )),
                    Err(pe) => ToolResult::error(format!(
                        "Accessibility action on [{r}] failed ({e}) and the pixel fallback \
                         also failed: {pe}"
                    )),
                }
            },
        ).await
    }

    /// Set the complete value of an element by `[ref]`. Only native whole-value
    /// setters implement this operation; typing at a caret is not equivalent.
    async fn do_set_element_value(&self, input: &Value) -> ToolResult {
        let r = match require_u32(input, "ref") {
            Ok(r) => r,
            Err(e) => return ToolResult::error(e),
        };
        let Some(text) = input.get("text").and_then(|v| v.as_str()) else {
            return ToolResult::error("Missing required parameter `text` for set_element_value.");
        };
        let text = text.to_string();
        let (generation, entry) = match self.resolve_ref(r) {
            Ok(v) => v,
            Err(msg) => return ToolResult::error(msg),
        };

        self.act_on_ax(
            entry.engine_ref,
            generation,
            ElementAction::SetValue(text),
            |_| async {
                self.clear_cached_observation();
                ToolResult::error(UNSUPPORTED_VALUE_INPUT_REJECTION)
            },
        ).await
    }

    /// Revalidate a retained `[ref]` before a pointer gesture. This never
    /// invokes a semantic action and never falls back to stale coordinates if
    /// the backend cannot verify the target. The OS scene can still change
    /// after validation; this is not an atomic scene/input transaction.
    async fn ref_screen_center(&self, r: u32) -> Result<(i32, i32), String> {
        let (generation, entry) = self.resolve_ref(r)?;
        self.validated_ref_screen_center(r, entry.engine_ref, generation, entry.display.bounds).await
    }

    async fn validated_ref_screen_center(
        &self,
        r: u32,
        engine_ref: u32,
        generation: SnapshotGen,
        expected_bounds: nomi_a11y::Rect,
    ) -> Result<(i32, i32), String> {
        let engine = match self.engine() {
            Ok(engine) => engine,
            Err(error) => return self.reject_gesture_target(r, A11yError::Backend(error)),
        };
        let resolved = tokio::task::spawn_blocking(move || {
            engine.resolve_pixel_target(&Target::Ref(engine_ref), generation)
        }).await;
        let bounds = match resolved {
            Ok(Ok(bounds)) if bounds == expected_bounds => bounds,
            Ok(Ok(_)) => return self.reject_gesture_target(r, A11yError::Stale(
                "the live gesture bounds no longer match the observed target; re-run observe".into(),
            )),
            Ok(Err(error)) => return self.reject_gesture_target(r, error),
            Err(error) => return self.reject_gesture_target(r, A11yError::Backend(
                format!("target validation task failed: {error}"),
            )),
        };
        match bounds.pixel_center() {
            Some(center) => Ok(center),
            None => self.reject_gesture_target(r, A11yError::Stale(
                "the gesture target has invalid or unrepresentable screen bounds; re-run observe".into(),
            )),
        }
    }

    fn reject_gesture_target<T>(&self, r: u32, error: A11yError) -> Result<T, String> {
        *self.last_snapshot.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Err(format!(
            "Accessibility action on [{r}] failed: {error}. No pixel fallback was \
             performed; re-run observe before using element refs."
        ))
    }

    /// Perform a pixel mouse gesture (right-click / double-click) on the element
    /// addressed by `[ref]`. These are pointer gestures with no UIA semantic
    /// equivalent, so they click the element's center directly.
    async fn do_element_gesture(
        &self,
        input: &Value,
        button: enigo::Button,
        count: u32,
        verb: &str,
    ) -> ToolResult {
        let r = match require_u32(input, "ref") {
            Ok(r) => r,
            Err(e) => return ToolResult::error(e),
        };
        let (sx, sy) = match self.ref_screen_center(r).await {
            Ok(v) => v,
            Err(msg) => return ToolResult::error(msg),
        };
        match self.gesture.click(sx, sy, button, count).await {
            Ok(()) => ToolResult::text(format!(
                "Performed {verb} on element [{r}] at its center. Run `observe` (or take a \
                 screenshot) to verify the result."
            )),
            Err(e) => ToolResult::error(format!("{verb} on element [{r}] failed: {e}")),
        }
    }

    /// Reliably open an application, file, or folder via the OS shell
    /// (ShellExecute on Windows). The dependable way to launch things — never
    /// shell out to `cmd /c start` / `Start-Process`, which fail and pop a
    /// "Windows cannot find" dialog on this host. Web URLs fail closed toward
    /// the managed Browser tool (see `launch::validate_agent_web_target`).
    async fn do_launch(&self, input: &Value) -> ToolResult {
        let Some(target) = input.get("target").and_then(|v| v.as_str()) else {
            return ToolResult::error(
                "Missing required parameter `target` for launch (a file/folder path, or an \
                 application name like \"notepad\"). Web URLs are opened with the managed \
                 Browser tool, not launch.",
            );
        };
        let app = match input.get("app") {
            None | Some(Value::Null) => None,
            Some(Value::String(app)) => Some(app.as_str()),
            Some(_) => return ToolResult::error("Parameter `app` must be a string."),
        };
        match crate::launch::launch(target, app).await {
            Ok(msg) => ToolResult::text(format!(
                "{msg} Take a screenshot or run `observe` to see the result."
            )),
            Err(e) => ToolResult::error(e),
        }
    }

    /// Map model-provided screenshot coordinates to absolute screen
    /// coordinates. Identity when no screenshot has been taken yet.
    fn to_screen(&self, x: i32, y: i32) -> (i32, i32) {
        match *self.last_capture.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) {
            Some(g) => {
                let (lx, ly) = map_llm_coord(x, y, g.img_w, g.img_h, g.logical_w, g.logical_h);
                (g.origin_x + lx, g.origin_y + ly)
            }
            None => (x, y),
        }
    }

    /// Map an absolute screen coordinate into the most recent screenshot's
    /// pixel space (for reporting the cursor to the model).
    fn to_image(&self, x: i32, y: i32) -> (i32, i32) {
        match *self.last_capture.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) {
            Some(g) => map_screen_coord(
                x - g.origin_x,
                y - g.origin_y,
                g.img_w,
                g.img_h,
                g.logical_w,
                g.logical_h,
            ),
            None => (x, y),
        }
    }

    async fn do_screenshot(&self, input: &Value) -> ToolResult {
        let display = match input.get("display") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_u64() {
                Some(d) => match usize::try_from(d) {
                    Ok(d) => Some(d),
                    Err(_) => return ToolResult::error("Parameter `display` is out of range."),
                },
                None => {
                    return ToolResult::error(
                        "Parameter `display` must be a non-negative integer display index.",
                    );
                }
            },
        };

        let max_edge = self.max_screenshot_edge;
        let captured = tokio::task::spawn_blocking(move || capture_screen(display, max_edge))
            .await
            .unwrap_or_else(|e| Err(format!("Screenshot task failed: {e}")));

        match captured {
            Ok(shot) => {
                let encoded = encode_png_with_limit(&shot.image, CANONICAL_SCREENSHOT_PNG_BYTES);
                match encoded {
                    Ok(encoded) => {
                        let mut geometry = shot.geometry;
                        geometry.img_w = encoded.width;
                        geometry.img_h = encoded.height;
                        *self.last_capture.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            Some(geometry);
                        let text = format!(
                            "Screenshot captured: {}x{} (display {}, scaled from {}x{} physical). \
                             Coordinates you provide will be mapped back to the screen automatically.",
                            encoded.width,
                            encoded.height,
                            shot.display_index,
                            shot.physical_w,
                            shot.physical_h
                        );
                        ToolResult::text(text).with_images(vec![encoded.image])
                    }
                    Err(e) => ToolResult::error(e),
                }
            }
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_cursor_position(&self) -> ToolResult {
        match input::cursor_position().await {
            Ok((sx, sy)) => {
                let has_capture = self
                    .last_capture
                    .lock()
                    .expect("last_capture poisoned")
                    .is_some();
                if has_capture {
                    let (ix, iy) = self.to_image(sx, sy);
                    ToolResult::text(format!(
                        "Cursor position: ({ix}, {iy}) in screenshot coordinates \
                         (screen: ({sx}, {sy}))."
                    ))
                } else {
                    ToolResult::text(format!(
                        "Cursor position: ({sx}, {sy}) in screen coordinates \
                         (no screenshot taken yet)."
                    ))
                }
            }
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_list_windows(&self) -> ToolResult {
        let listed = tokio::task::spawn_blocking(fallback_backend::list_windows)
            .await
            .unwrap_or_else(|e| Err(format!("Window listing task failed: {e}")));
        match listed {
            Ok(list) => ToolResult::text(fallback_backend::format_window_list(&list)),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_click(&self, input: &Value, button: enigo::Button, count: u32) -> ToolResult {
        let (x, y) = match require_xy(input, "x", "y") {
            Ok(xy) => xy,
            Err(e) => return ToolResult::error(e),
        };
        let (sx, sy) = self.to_screen(x, y);
        match input::click(sx, sy, button, count).await {
            Ok(()) => ToolResult::text(format!(
                "Clicked at ({x}, {y}) (screen ({sx}, {sy})). Take a screenshot to verify the \
                 result."
            )),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_mouse_move(&self, input: &Value) -> ToolResult {
        let (x, y) = match require_xy(input, "x", "y") {
            Ok(xy) => xy,
            Err(e) => return ToolResult::error(e),
        };
        let (sx, sy) = self.to_screen(x, y);
        match input::mouse_move(sx, sy).await {
            Ok(()) => ToolResult::text(format!("Moved cursor to ({x}, {y}) (screen ({sx}, {sy}))." )),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_drag(&self, input: &Value) -> ToolResult {
        let (start_x, start_y) = match require_xy(input, "start_x", "start_y") {
            Ok(xy) => xy,
            Err(e) => return ToolResult::error(e),
        };
        let (end_x, end_y) = match require_xy(input, "end_x", "end_y") {
            Ok(xy) => xy,
            Err(e) => return ToolResult::error(e),
        };
        let (sx, sy) = self.to_screen(start_x, start_y);
        let (ex, ey) = self.to_screen(end_x, end_y);
        match input::drag(sx, sy, ex, ey).await {
            Ok(()) => ToolResult::text(format!(
                "Dragged from ({start_x}, {start_y}) to ({end_x}, {end_y}). Take a screenshot \
                 to verify the result."
            )),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_type(&self, input: &Value) -> ToolResult {
        let Some(text) = input.get("text").and_then(|v| v.as_str()) else {
            return ToolResult::error(
                "Missing required parameter `text` for the type action.",
            );
        };
        let char_count = text.chars().count();
        match input::type_text(text.to_string()).await {
            Ok(()) => ToolResult::text(format!("Typed {char_count} character(s).")),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_key(&self, input: &Value) -> ToolResult {
        let Some(combo) = input.get("key").and_then(|v| v.as_str()) else {
            return ToolResult::error(format!(
                "Missing required parameter `key` for the key action, e.g. \"enter\" or \
                 \"{KEY_COMBO_EXAMPLE}\"."
            ));
        };
        let keys = match parse_key_combo(combo) {
            Ok(keys) => keys,
            Err(e) => return ToolResult::error(e),
        };
        match input::key_combo(keys).await {
            Ok(()) => ToolResult::text(format!("Pressed {combo:?}.")),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_scroll(&self, input: &Value) -> ToolResult {
        let Some(direction_str) = input.get("direction").and_then(|v| v.as_str()) else {
            return ToolResult::error(
                "Missing required parameter `direction` for the scroll action \
                 (up, down, left or right).",
            );
        };
        let direction = match ScrollDirection::parse(direction_str) {
            Ok(d) => d,
            Err(e) => return ToolResult::error(e),
        };
        let amount = match input.get("amount") {
            None | Some(Value::Null) => DEFAULT_SCROLL_AMOUNT,
            Some(v) => match v.as_i64() {
                Some(amount) => amount,
                None => return ToolResult::error("Parameter `amount` must be an integer."),
            },
        }.clamp(1, 100) as i32;
        let at = match optional_xy(input) {
            Ok(at) => at.map(|(x, y)| self.to_screen(x, y)),
            Err(e) => return ToolResult::error(e),
        };
        match input::scroll(at, direction, amount).await {
            Ok(()) => ToolResult::text(format!(
                "Scrolled {direction_str} by {amount}. Take a screenshot to see the result."
            )),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_focus_window(&self, input: &Value) -> ToolResult {
        let window_id = match require_u32(input, "window_id") {
            Ok(id) => id,
            Err(e) => return ToolResult::error(e),
        };
        match fallback_backend::focus_window(window_id).await {
            Ok(msg) => ToolResult::text(msg),
            Err(e) => ToolResult::error(e),
        }
    }

    async fn do_wait(&self, input: &Value) -> ToolResult {
        let requested = input
            .get("seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        let seconds = requested.clamp(0.0, MAX_WAIT_SECONDS);
        tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
        ToolResult::text(format!("Waited {seconds} second(s)."))
    }
}

fn require_u32(input: &Value, name: &str) -> Result<u32, String> {
    let value = input.get(name).and_then(Value::as_u64)
        .ok_or_else(|| format!("Missing or invalid required parameter `{name}`: expected an unsigned integer."))?;
    u32::try_from(value).map_err(|_| format!("Parameter `{name}` is out of range for a 32-bit id."))
}

fn optional_xy(input: &Value) -> Result<Option<(i32, i32)>, String> {
    match (input.get("x"), input.get("y")) {
        (None | Some(Value::Null), None | Some(Value::Null)) => Ok(None),
        _ => require_xy(input, "x", "y").map(Some),
    }
}

/// Extract a required (x, y)-style coordinate pair, naming the missing
/// parameters in the error.
fn require_xy(input: &Value, x_name: &str, y_name: &str) -> Result<(i32, i32), String> {
    let x = input.get(x_name).and_then(|v| v.as_i64());
    let y = input.get(y_name).and_then(|v| v.as_i64());
    match (x, y) {
        (Some(x), Some(y)) => Ok((
            i32::try_from(x).map_err(|_| format!("Parameter `{x_name}` is out of range for a 32-bit coordinate."))?,
            i32::try_from(y).map_err(|_| format!("Parameter `{y_name}` is out of range for a 32-bit coordinate."))?,
        )),
        (None, Some(_)) => Err(format!("Missing required parameter `{x_name}`.")),
        (Some(_), None) => Err(format!("Missing required parameter `{y_name}`.")),
        (None, None) => Err(format!(
            "Missing required parameters `{x_name}` and `{y_name}`."
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::*;
    use nomi_a11y::Source;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct GestureLiveState {
        frontmost_pid: i32,
        window_id: u32,
        bounds: nomi_a11y::Rect,
        dirty: bool,
        covered: bool,
    }

    const OBSERVED_GESTURE_STATE: GestureLiveState = GestureLiveState {
        frontmost_pid: 41,
        window_id: 11,
        bounds: nomi_a11y::Rect { x: -1200.0, y: 42.0, w: 80.0, h: 20.0 },
        dirty: false,
        covered: false,
    };

    #[derive(Clone, Copy)]
    enum SemanticReply {
        UnsupportedAfterDrift(GestureLiveState),
        BackendAfterEffect,
        UncertainAfterEffect,
        UncertainWithStaleHintAfterEffect,
        NativeWholeValueSetter,
    }

    struct FakeTextField {
        value: String,
        caret: usize,
        readonly: bool,
    }

    struct GestureEngine {
        live: Mutex<GestureLiveState>,
        resolver_calls: std::sync::atomic::AtomicUsize,
        resolver_failure: Mutex<Option<fn() -> A11yError>>,
        semantic_reply: Mutex<Option<SemanticReply>>,
        semantic_calls: std::sync::atomic::AtomicUsize,
        semantic_effects: std::sync::atomic::AtomicUsize,
        text_field: Mutex<Option<Arc<Mutex<FakeTextField>>>>,
    }

    impl Default for GestureEngine {
        fn default() -> Self {
            Self {
                live: Mutex::new(OBSERVED_GESTURE_STATE),
                resolver_calls: std::sync::atomic::AtomicUsize::new(0),
                resolver_failure: Mutex::new(None),
                semantic_reply: Mutex::new(None),
                semantic_calls: std::sync::atomic::AtomicUsize::new(0),
                semantic_effects: std::sync::atomic::AtomicUsize::new(0),
                text_field: Mutex::new(None),
            }
        }
    }

    impl A11yEngine for GestureEngine {
        fn capabilities(&self) -> nomi_a11y::Capabilities {
            panic!("the gesture test must not probe a native backend")
        }

        fn observe(&self, _: &ObserveOpts) -> Result<nomi_a11y::Snapshot, A11yError> {
            assert_eq!(*self.live.lock().unwrap(), OBSERVED_GESTURE_STATE);
            let value = self.text_field.lock().unwrap().as_ref()
                .map(|field| field.lock().unwrap().value.clone());
            Ok(nomi_a11y::Snapshot {
                generation: SnapshotGen(9),
                entries: vec![ElementEntry {
                    r#ref: 7,
                    role: "textfield".into(),
                    name: Some("Gesture audit target".into()),
                    value,
                    states: vec![],
                    bounds: OBSERVED_GESTURE_STATE.bounds,
                    source: Source::A11y,
                }],
                text: "[7] textfield \"Gesture audit target\"".into(),
                truncated: false,
                pid: Some(OBSERVED_GESTURE_STATE.frontmost_pid),
                app_name: Some("Fake gesture fixture".into()),
                window_title: Some("Fake gesture window".into()),
            })
        }

        fn resolve_pixel_target(
            &self,
            target: &Target,
            generation: SnapshotGen,
        ) -> Result<nomi_a11y::Rect, A11yError> {
            use std::sync::atomic::Ordering;
            self.resolver_calls.fetch_add(1, Ordering::SeqCst);
            assert!(matches!(target, Target::Ref(7)));
            assert_eq!(generation, SnapshotGen(9));
            if let Some(failure) = *self.resolver_failure.lock().unwrap() {
                return Err(failure());
            }
            let live = *self.live.lock().unwrap();
            if live != OBSERVED_GESTURE_STATE {
                return Err(A11yError::Stale(format!(
                    "the retained gesture target changed after observe: {live:?}"
                )));
            }
            Ok(live.bounds)
        }

        fn invoke(
            &self,
            target: &Target,
            generation: SnapshotGen,
            action: ElementAction,
        ) -> Result<nomi_a11y::Effect, A11yError> {
            use std::sync::atomic::Ordering;
            let reply = *self.semantic_reply.lock().unwrap();
            let Some(reply) = reply else {
                panic!("pixel target validation must not perform a semantic action")
            };
            assert!(matches!(target, Target::Ref(7)));
            assert_eq!(generation, SnapshotGen(9));
            assert!(matches!(action, ElementAction::LeftClick | ElementAction::SetValue(_)));
            self.semantic_calls.fetch_add(1, Ordering::SeqCst);
            match reply {
                SemanticReply::UnsupportedAfterDrift(live) => {
                    *self.live.lock().unwrap() = live;
                    Err(A11yError::Unsupported {
                        capability: "semantic action".into(),
                        hint: "no semantic request was admitted, but the live scene may have changed".into(),
                    })
                }
                SemanticReply::BackendAfterEffect => {
                    self.semantic_effects.fetch_add(1, Ordering::SeqCst);
                    Err(A11yError::Backend("the semantic effect occurred before the provider reply failed".into()))
                }
                SemanticReply::UncertainAfterEffect => {
                    self.semantic_effects.fetch_add(1, Ordering::SeqCst);
                    Err(A11yError::Uncertain("the admitted semantic request has an unknown effect".into()))
                }
                SemanticReply::UncertainWithStaleHintAfterEffect => {
                    self.semantic_effects.fetch_add(1, Ordering::SeqCst);
                    Err(A11yError::Uncertain(
                        "provider reply failed: stale reference: this is only a nested hint. No pixel fallback was performed; the original semantic request already had an effect".into(),
                    ))
                }
                SemanticReply::NativeWholeValueSetter => {
                    let ElementAction::SetValue(value) = action else { panic!("the fake whole-value setter only accepts SetValue"); };
                    let field = self.text_field.lock().unwrap().clone().expect("a field must exist");
                    let mut field = field.lock().unwrap();
                    if field.readonly {
                        return Err(A11yError::Unsupported {
                            capability: "native whole-value setter".into(), hint: "the field is read-only before dispatch".into(),
                        });
                    }
                    field.value = value;
                    field.caret = field.value.len();
                    self.semantic_effects.fetch_add(1, Ordering::SeqCst);
                    Ok(nomi_a11y::Effect { changed: true, message: "native whole-value setter acknowledged".into() })
                }
            }
        }

        fn focus_window(&self, _: i32) -> Result<nomi_a11y::Effect, A11yError> {
            panic!("the gesture test must not activate a window")
        }
    }

    #[derive(Default)]
    struct FakeGesturePort {
        calls: Mutex<Vec<(i32, i32, enigo::Button, u32)>>,
    }

    impl GesturePort for FakeGesturePort {
        fn click(&self, x: i32, y: i32, button: enigo::Button, count: u32) -> GestureFuture<'_> {
            self.calls.lock().unwrap().push((x, y, button, count));
            Box::pin(std::future::ready(Ok(())))
        }
    }

    async fn gesture_fixture() -> (ComputerTool, Arc<GestureEngine>, Arc<FakeGesturePort>) {
        gesture_fixture_with_text_field(None).await
    }

    async fn gesture_fixture_with_text_field(
        text_field: Option<Arc<Mutex<FakeTextField>>>,
    ) -> (ComputerTool, Arc<GestureEngine>, Arc<FakeGesturePort>) {
        let engine = Arc::new(GestureEngine::default());
        let gesture = Arc::new(FakeGesturePort::default());
        *engine.text_field.lock().unwrap() = text_field.clone();
        let mut tool = tool();
        *tool.a11y.lock().unwrap() = Some(Ok(engine.clone()));
        tool.gesture = gesture.clone();
        let observed = tool.execute_authorized(
            crate::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID,
            json!({ "action": "observe" }),
        ).await;
        assert!(!observed.is_error, "{}", observed.content);
        assert!(gesture.calls.lock().unwrap().is_empty());
        (tool, engine, gesture)
    }

    async fn assert_gesture_rejects_drift(live: GestureLiveState) {
        use std::sync::atomic::Ordering;
        let mut failures = Vec::new();
        for operation in ["right_click_element", "double_click_element"] {
            let (tool, engine, gesture) = gesture_fixture().await;
            *engine.live.lock().unwrap() = live;
            let result = tool.execute_authorized(
                crate::capability::COMPUTER_INPUT_ACTION_ID,
                json!({ "action": operation, "ref": 7 }),
            ).await;
            let calls = gesture.calls.lock().unwrap().clone();
            let resolver_calls = engine.resolver_calls.load(Ordering::SeqCst);
            if !result.is_error || !calls.is_empty() || resolver_calls != 1
                || !result.content.contains("stale reference")
            {
                failures.push(format!(
                    "{operation}: resolver_calls={resolver_calls}, native_gesture_calls={calls:?}, \
                     live={live:?}, result={}", result.content,
                ));
            }
        }
        assert!(failures.is_empty(),
            "MAC38 gestures must validate the exact observed ref/generation and reject live \
             drift before native admission:\n{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn element_gesture_rejects_frontmost_drift_without_native_admission() {
        assert_gesture_rejects_drift(GestureLiveState {
            frontmost_pid: 42,
            ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_gesture_rejects_window_drift_without_native_admission() {
        assert_gesture_rejects_drift(GestureLiveState {
            window_id: 12,
            ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_gesture_rejects_bounds_drift_without_native_admission() {
        assert_gesture_rejects_drift(GestureLiveState {
            bounds: nomi_a11y::Rect { x: -800.0, ..OBSERVED_GESTURE_STATE.bounds },
            ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_gesture_rejects_dirty_snapshot_without_native_admission() {
        assert_gesture_rejects_drift(GestureLiveState {
            dirty: true,
            ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_gesture_preserves_buttons_counts_and_ax_coordinates() {
        let (tool, _engine, gesture) = gesture_fixture().await;
        // Ref geometry stays in OS logical coordinates even if a stale image
        // mapping exists. No screenshot or native device is touched here.
        *tool.last_capture.lock().unwrap() = Some(CaptureGeometry {
            img_w: 200,
            img_h: 100,
            logical_w: 2000,
            logical_h: 1000,
            origin_x: 500,
            origin_y: 700,
        });
        for operation in ["right_click_element", "double_click_element"] {
            let result = tool.execute_authorized(
                crate::capability::COMPUTER_INPUT_ACTION_ID,
                json!({ "action": operation, "ref": 7 }),
            ).await;
            assert!(!result.is_error, "{}", result.content);
        }
        assert_eq!(*gesture.calls.lock().unwrap(), vec![
            (-1160, 52, enigo::Button::Right, 1),
            (-1160, 52, enigo::Button::Left, 2),
        ]);
    }

    #[tokio::test]
    async fn element_gesture_never_uses_cached_pixels_when_validation_fails() {
        use std::sync::atomic::Ordering;
        let failures: [fn() -> A11yError; 5] = [
            || A11yError::NotFound("the live element was destroyed".into()),
            || A11yError::Permission("permission was revoked".into()),
            || A11yError::Backend("live geometry could not be read".into()),
            || A11yError::Unsupported {
                capability: "pixel gesture target validation".into(), hint: "unavailable".into(),
            },
            || panic!("the validation worker failed"),
        ];
        for failure in failures {
            for operation in ["right_click_element", "double_click_element"] {
                let (tool, engine, gesture) = gesture_fixture().await;
                *engine.resolver_failure.lock().unwrap() = Some(failure);
                let result = tool.execute_authorized(
                    crate::capability::COMPUTER_INPUT_ACTION_ID,
                    json!({ "action": operation, "ref": 7 }),
                ).await;
                assert!(result.is_error, "{}", result.content);
                assert!(gesture.calls.lock().unwrap().is_empty(),
                    "a validation error never authorizes cached-coordinate fallback");
                assert_eq!(engine.resolver_calls.load(Ordering::SeqCst), 1);
                assert!(tool.last_snapshot.lock().unwrap().is_none());
            }
        }
    }

    async fn assert_element_fallback_rejects_drift(live: GestureLiveState) {
        use std::sync::atomic::Ordering;
        let mut failures = Vec::new();
        for operation in ["click_element", "set_element_value"] {
            let (tool, engine, gesture) = gesture_fixture().await;
            *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::UnsupportedAfterDrift(live));
            let result = tool.execute_authorized(
                crate::capability::COMPUTER_INPUT_ACTION_ID,
                json!({ "action": operation, "ref": 7, "text": "AX fallback audit" }),
            ).await;
            let pointers = gesture.calls.lock().unwrap().clone();
            let resolver_calls = engine.resolver_calls.load(Ordering::SeqCst);
            let semantic_calls = engine.semantic_calls.load(Ordering::SeqCst);
            let expected_resolution = if operation == "click_element" { 1 } else { 0 };
            if !result.is_error || !pointers.is_empty()
                || resolver_calls != expected_resolution || semantic_calls != 1
            {
                failures.push(format!(
                    "{operation}: semantic_calls={semantic_calls}, resolver_calls={resolver_calls}, \
                     pointer_calls={pointers:?}, live={live:?}, result={}",
                    result.content,
                ));
            }
        }
        assert!(failures.is_empty(),
            "MAC38 ordinary AX fallback must validate the target again after a semantic \
             Unsupported reply and reject scene drift before pointer/type admission:\n{}",
            failures.join("\n"));
    }

    #[tokio::test]
    async fn element_fallback_rejects_frontmost_drift_before_native_input() {
        assert_element_fallback_rejects_drift(GestureLiveState {
            frontmost_pid: 42, ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_fallback_rejects_window_drift_before_native_input() {
        assert_element_fallback_rejects_drift(GestureLiveState {
            window_id: 12, ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_fallback_rejects_bounds_drift_before_native_input() {
        assert_element_fallback_rejects_drift(GestureLiveState {
            bounds: nomi_a11y::Rect { x: -800.0, ..OBSERVED_GESTURE_STATE.bounds },
            ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_fallback_rejects_overlay_drift_before_native_input() {
        assert_element_fallback_rejects_drift(GestureLiveState {
            covered: true, ..OBSERVED_GESTURE_STATE
        }).await;
    }

    #[tokio::test]
    async fn element_fallback_never_retries_a_backend_error_after_semantic_effect() {
        use std::sync::atomic::Ordering;
        let mut failures = Vec::new();
        for operation in ["click_element", "set_element_value"] {
            let (tool, engine, gesture) = gesture_fixture().await;
            *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::BackendAfterEffect);
            let result = tool.execute_authorized(
                crate::capability::COMPUTER_INPUT_ACTION_ID,
                json!({ "action": operation, "ref": 7, "text": "must not be repeated" }),
            ).await;
            let pointers = gesture.calls.lock().unwrap().clone();
            let effects = engine.semantic_effects.load(Ordering::SeqCst);
            if !result.is_error || !pointers.is_empty() || effects != 1 {
                failures.push(format!(
                    "{operation}: semantic_effects={effects}, pointer_calls={pointers:?}, result={}", result.content,
                ));
            }
        }
        assert!(failures.is_empty(),
            "MAC38 Backend does not prove zero semantic effect and must not authorize \
             automatic pixel/type retry:\n{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn element_fallback_preserves_uncertain_semantic_result_without_retry() {
        use std::sync::atomic::Ordering;
        for operation in ["click_element", "set_element_value"] {
            let (tool, engine, gesture) = gesture_fixture().await;
            *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::UncertainAfterEffect);
            let result = tool.execute_authorized(
                crate::capability::COMPUTER_INPUT_ACTION_ID,
                json!({ "action": operation, "ref": 7, "text": "must not be repeated" }),
            ).await;
            assert!(result.is_error && result.content.contains("effect is uncertain"));
            assert!(gesture.calls.lock().unwrap().is_empty());
            assert_eq!(engine.semantic_effects.load(Ordering::SeqCst), 1);
            assert_eq!(engine.resolver_calls.load(Ordering::SeqCst), 0);
            assert!(!is_proven_stale_input_rejection(&result));
        }
    }

    #[tokio::test]
    async fn element_fallback_preserves_stable_click_input() {
        use std::sync::atomic::Ordering;
        let (tool, engine, gesture) = gesture_fixture().await;
        *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::UnsupportedAfterDrift(OBSERVED_GESTURE_STATE));
        let result = tool.execute_authorized(
            crate::capability::COMPUTER_INPUT_ACTION_ID,
            json!({ "action": "click_element", "ref": 7 }),
        ).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(engine.semantic_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*gesture.calls.lock().unwrap(), vec![
            (-1160, 52, enigo::Button::Left, 1),
        ]);
    }

    #[tokio::test]
    async fn clear_cached_observation_retires_refs_and_pixels_without_native_calls() {
        use std::sync::atomic::Ordering;
        let (tool, engine, gesture) = gesture_fixture().await;
        *tool.last_capture.lock().unwrap() = Some(CaptureGeometry {
            img_w: 100, img_h: 100, logical_w: 200, logical_h: 200,
            origin_x: -1000, origin_y: 200,
        });
        assert!(tool.resolve_ref(7).is_ok());
        tool.clear_cached_observation();
        assert!(tool.last_capture.lock().unwrap().is_none());
        assert!(tool.last_snapshot.lock().unwrap().is_none());
        assert!(tool.resolve_ref(7).is_err());
        assert!(gesture.calls.lock().unwrap().is_empty());
        assert_eq!(engine.semantic_calls.load(Ordering::SeqCst), 0);
        assert_eq!(engine.resolver_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn element_value_fallback_replaces_nonempty_value_or_rejects_before_pointer() {
        let original = "existing-field-value";
        let replacement = "replacement-value";
        let field = Arc::new(Mutex::new(FakeTextField {
            value: original.to_string(), caret: original.len(), readonly: false,
        }));
        let (tool, engine, gesture) = gesture_fixture_with_text_field(Some(field.clone())).await;
        assert_eq!(tool.resolve_ref(7).unwrap().1.display.value.as_deref(), Some(original));
        *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::UnsupportedAfterDrift(OBSERVED_GESTURE_STATE));
        // Retain the original nonempty value and end caret. Neither can prove
        // that a whole-value replacement is supported by keyboard insertion.
        let result = tool.execute_authorized(
            crate::capability::COMPUTER_INPUT_ACTION_ID,
            json!({ "action": "set_element_value", "ref": 7, "text": replacement }),
        ).await;
        let actual = field.lock().unwrap().value.clone();
        let pointers = gesture.calls.lock().unwrap().clone();
        if result.is_error {
            assert!(pointers.is_empty() && actual == original,
                "a value replacement that cannot be proven must reject before effect: \
                 pointer_calls={pointers:?}, actual={actual:?}, result={}", result.content);
            assert_eq!(field.lock().unwrap().caret, original.len());
            assert!(is_proven_unsupported_input_rejection(&result));
        } else {
            assert_eq!(actual, replacement,
                "MAC38 set_element_value must replace the whole value, not report successful \
                 caret insertion: original={original:?}, pointer_calls={pointers:?}, result={}", result.content);
        }
    }

    #[tokio::test]
    async fn uncertain_semantic_effect_hint_cannot_forge_stale_zero_effect_rejection() {
        use std::sync::atomic::Ordering;
        let (tool, engine, gesture) = gesture_fixture().await;
        *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::UncertainWithStaleHintAfterEffect);
        let result = tool.execute_authorized(
            crate::capability::COMPUTER_INPUT_ACTION_ID,
            json!({ "action": "click_element", "ref": 7 }),
        ).await;
        assert!(result.is_error && result.content.contains("effect is uncertain"));
        assert_eq!(engine.semantic_effects.load(Ordering::SeqCst), 1);
        assert!(gesture.calls.lock().unwrap().is_empty());
        assert!(!is_proven_stale_input_rejection(&result),
            "MAC38 an actual admitted semantic effect must not become zero-effect merely \
             because its Uncertain hint contains stale/no-fallback phrases: {}", result.content);
    }

    #[tokio::test]
    async fn native_whole_value_setter_preserves_exact_replacement_without_pointer_fallback() {
        use std::sync::atomic::Ordering;
        let field = Arc::new(Mutex::new(FakeTextField {
            value: "old value with caret".into(), caret: 2, readonly: false,
        }));
        let (tool, engine, gesture) = gesture_fixture_with_text_field(Some(field.clone())).await;
        *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::NativeWholeValueSetter);
        let result = tool.execute_authorized(
            crate::capability::COMPUTER_INPUT_ACTION_ID,
            json!({ "action": "set_element_value", "ref": 7, "text": "  exact replacement\n" }),
        ).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(field.lock().unwrap().value, "  exact replacement\n");
        assert_eq!(engine.semantic_effects.load(Ordering::SeqCst), 1);
        assert_eq!(engine.resolver_calls.load(Ordering::SeqCst), 0);
        assert!(gesture.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unsupported_value_setter_never_infers_replacement_from_display_value() {
        use std::sync::atomic::Ordering;
        for original in [None, Some(""), Some("  \t\n"), Some("nonempty value")] {
            let field = original.map(|value| Arc::new(Mutex::new(FakeTextField {
                value: value.to_string(), caret: value.len(), readonly: false,
            })));
            let (tool, engine, gesture) = gesture_fixture_with_text_field(field.clone()).await;
            *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::UnsupportedAfterDrift(OBSERVED_GESTURE_STATE));
            let result = tool.execute_authorized(
                crate::capability::COMPUTER_INPUT_ACTION_ID,
                json!({ "action": "set_element_value", "ref": 7, "text": "replacement" }),
            ).await;
            assert!(is_proven_unsupported_input_rejection(&result), "{}", result.content);
            assert!(gesture.calls.lock().unwrap().is_empty());
            assert_eq!(engine.semantic_effects.load(Ordering::SeqCst), 0);
            assert_eq!(engine.resolver_calls.load(Ordering::SeqCst), 0);
            if let Some(field) = field { assert_eq!(field.lock().unwrap().value.as_str(), original.unwrap()); }
        }
        let field = Arc::new(Mutex::new(FakeTextField { value: "read-only".into(), caret: 3, readonly: true }));
        let (tool, engine, gesture) = gesture_fixture_with_text_field(Some(field.clone())).await;
        *engine.semantic_reply.lock().unwrap() = Some(SemanticReply::NativeWholeValueSetter);
        let result = tool.execute_authorized(
            crate::capability::COMPUTER_INPUT_ACTION_ID,
            json!({ "action": "set_element_value", "ref": 7, "text": "replacement" }),
        ).await;
        assert!(is_proven_unsupported_input_rejection(&result));
        assert_eq!(field.lock().unwrap().value, "read-only");
        assert_eq!(engine.semantic_effects.load(Ordering::SeqCst), 0);
        assert!(gesture.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn unsupported_zero_effect_marker_requires_the_exact_setter_rejection() {
        assert!(is_proven_unsupported_input_rejection(&ToolResult::error(UNSUPPORTED_VALUE_INPUT_REJECTION)));
        for result in [
            ToolResult::text(UNSUPPORTED_VALUE_INPUT_REJECTION),
            ToolResult::error(format!("accessibility backend error: {UNSUPPORTED_VALUE_INPUT_REJECTION}")),
            ToolResult::error(format!("accessibility action effect is uncertain: {UNSUPPORTED_VALUE_INPUT_REJECTION}")),
            ToolResult::error(format!("{UNSUPPORTED_VALUE_INPUT_REJECTION} extra hint")),
            ToolResult::error("not supported (generic action): no effect"),
        ] {
            assert!(!is_proven_unsupported_input_rejection(&result), "{}", result.content);
        }
    }

    type AxOutcome = fn() -> Result<nomi_a11y::Effect, A11yError>;

    struct FakeEngine(AxOutcome);

    impl A11yEngine for FakeEngine {
        fn capabilities(&self) -> nomi_a11y::Capabilities {
            panic!("unexpected capability probe")
        }
        fn observe(&self, _: &ObserveOpts) -> Result<nomi_a11y::Snapshot, A11yError> {
            Err(A11yError::Backend("simulated observe failure".into()))
        }
        fn invoke(&self, target: &Target, generation: SnapshotGen, _: ElementAction)
            -> Result<nomi_a11y::Effect, A11yError>
        {
            assert!(matches!(target, Target::Ref(1)));
            assert_eq!(generation, SnapshotGen(7));
            (self.0)()
        }
        fn focus_window(&self, _: i32) -> Result<nomi_a11y::Effect, A11yError> {
            panic!("unexpected window activation")
        }
    }

    struct SnapshotEngine;

    impl A11yEngine for SnapshotEngine {
        fn capabilities(&self) -> nomi_a11y::Capabilities {
            panic!("unexpected capability probe")
        }
        fn observe(&self, _: &ObserveOpts) -> Result<nomi_a11y::Snapshot, A11yError> {
            Ok(nomi_a11y::Snapshot {
                generation: SnapshotGen(9),
                entries: vec![ElementEntry {
                    r#ref: 1,
                    role: "button".into(),
                    name: Some("Continue".into()),
                    value: None,
                    states: vec![],
                    bounds: nomi_a11y::Rect {
                        x: 10.0,
                        y: 20.0,
                        w: 30.0,
                        h: 40.0,
                    },
                    source: Source::A11y,
                }],
                text: "[1] button \"Continue\"".into(),
                truncated: false,
                pid: Some(1),
                app_name: Some("Fixture".into()),
                window_title: Some("Fixture Window".into()),
            })
        }
        fn invoke(
            &self,
            _: &Target,
            _: SnapshotGen,
            _: ElementAction,
        ) -> Result<nomi_a11y::Effect, A11yError> {
            panic!("unexpected element action")
        }
        fn focus_window(&self, _: i32) -> Result<nomi_a11y::Effect, A11yError> {
            panic!("unexpected window activation")
        }
    }

    struct TruncatedSnapshotEngine;

    impl A11yEngine for TruncatedSnapshotEngine {
        fn capabilities(&self) -> nomi_a11y::Capabilities {
            panic!("unexpected capability probe")
        }
        fn observe(&self, _: &ObserveOpts) -> Result<nomi_a11y::Snapshot, A11yError> {
            let entries: Vec<_> = (0..120)
                .map(|index| ElementEntry {
                    r#ref: index + 1,
                    role: "button".into(),
                    name: Some(format!("AX_ITEM_{index:03}")),
                    value: None,
                    states: vec![],
                    bounds: nomi_a11y::Rect {
                        x: 10.0,
                        y: 20.0 + f64::from(index),
                        w: 30.0,
                        h: 20.0,
                    },
                    source: Source::A11y,
                })
                .collect();
            let text = entries
                .iter()
                .map(|entry| {
                    format!(
                        "[{}] button \"{}\"",
                        entry.r#ref,
                        entry.name.as_deref().unwrap()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(nomi_a11y::Snapshot {
                generation: SnapshotGen(10),
                entries,
                text,
                truncated: true,
                pid: Some(2),
                app_name: Some("Large Fixture".into()),
                window_title: Some("Large Fixture Window".into()),
            })
        }
        fn invoke(
            &self,
            _: &Target,
            _: SnapshotGen,
            _: ElementAction,
        ) -> Result<nomi_a11y::Effect, A11yError> {
            panic!("unexpected element action")
        }
        fn focus_window(&self, _: i32) -> Result<nomi_a11y::Effect, A11yError> {
            panic!("unexpected window activation")
        }
    }

    fn tool_with_snapshot(outcome: AxOutcome) -> ComputerTool {
        let t = tool();
        *t.a11y.lock().unwrap() = Some(Ok(Arc::new(FakeEngine(outcome))));
        *t.last_snapshot.lock().unwrap() = Some(SnapshotCache {
            generation: SnapshotGen(7),
            entries: vec![CachedEntry {
                display: ElementEntry {
                    r#ref: 1,
                    role: "text".into(),
                    name: None,
                    value: None,
                    states: vec![],
                    bounds: nomi_a11y::Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 },
                    source: Source::A11y,
                },
                engine_ref: 1,
            }],
        });
        t
    }

    fn tool() -> ComputerTool {
        ComputerTool::new(&ComputerConfig::default())
    }

    #[tokio::test]
    async fn semantic_fallback_gate_preserves_target_and_task_errors() {
        // The fallback is injected: even a regression cannot synthesize input.
        let cases: [(AxOutcome, bool, bool); 8] = [
            (|| Err(A11yError::Stale("stale".into())), false, true),
            (|| Err(A11yError::NotFound("gone".into())), false, true),
            (|| Err(A11yError::Permission("denied".into())), false, true),
            (|| Err(A11yError::Uncertain("native AX request timed out after admission".into())), false, true),
            (|| panic!("simulated worker failure"), false, true),
            (|| Err(A11yError::Backend("provider reply failed".into())), false, true),
            (|| Err(A11yError::Unsupported { capability: "action".into(), hint: "".into() }), true, false),
            (|| Ok(nomi_a11y::Effect { changed: true, message: "done".into() }), false, false),
        ];
        for (outcome, expect_fallback, expect_error) in cases {
            for action in [ElementAction::LeftClick, ElementAction::SetValue("value".into())] {
                let t = tool_with_snapshot(outcome);
                let mut called = false;
                let result = t.act_on_ax(1, SnapshotGen(7), action, |_| {
                    called = true;
                    std::future::ready(ToolResult::text("injected fallback"))
                }).await;
                assert_eq!(called, expect_fallback);
                assert_eq!(result.is_error, expect_error, "{}", result.content);
                assert_eq!(t.last_snapshot.lock().unwrap().is_none(), expect_error);
            }
        }
    }

    #[tokio::test]
    async fn uncertain_ax_action_neither_retries_nor_proves_zero_effect() {
        let t = tool_with_snapshot(|| Err(A11yError::Uncertain(
            "native AX API admitted the request but its effect is unknown".into(),
        )));
        let mut retried = false;
        let result = t.act_on_ax(1, SnapshotGen(7), ElementAction::LeftClick, |_| {
            retried = true;
            std::future::ready(ToolResult::text("unexpected pixel retry"))
        }).await;
        assert!(result.is_error);
        assert!(!retried, "an admitted AX request must not be repeated with pixel input");
        assert!(result.content.contains("effect is uncertain"), "{}", result.content);
        assert!(!is_proven_stale_input_rejection(&result), "an unknown effect must not settle as a pre-action rejection");
        assert!(t.last_snapshot.lock().unwrap().is_none());
    }

    #[test]
    fn only_exact_no_fallback_stale_errors_prove_zero_input_effect() {
        let stale = ToolResult::error(
            "Accessibility action on [7] failed: stale reference: focus changed. No pixel \
             fallback was performed; re-run observe before using element refs.",
        );
        assert!(is_proven_stale_input_rejection(&stale));
        for result in [
            ToolResult::error(
                "Accessibility action on [7] failed: accessibility backend error: worker lost. \
                 No pixel fallback was performed; re-run observe before using element refs.",
            ),
            ToolResult::error("stale reference: focus changed"),
            ToolResult::error(
                "Accessibility action on [7] failed: accessibility action effect is uncertain: \
                 nested failed: stale reference: focus changed. No pixel fallback was performed; \
                 re-run observe before using element refs.",
            ),
            ToolResult::error(
                "Accessibility action on [7] failed: accessibility backend error: nested failed: \
                 stale reference: focus changed. No pixel fallback was performed; \
                 re-run observe before using element refs.",
            ),
            ToolResult::text(
                "Accessibility action on [7] failed: stale reference: focus changed. No pixel \
                 fallback was performed; re-run observe before using element refs.",
            ),
        ] {
            assert!(!is_proven_stale_input_rejection(&result));
        }
        for reference in ["", "-1", "+7", "07", "4294967296", "wrong"] {
            let result = ToolResult::error(format!(
                "Accessibility action on [{reference}] failed: stale reference: focus changed. \
                 No pixel fallback was performed; re-run observe before using element refs.",
            ));
            assert!(!is_proven_stale_input_rejection(&result), "{}", result.content);
        }
    }

    #[tokio::test]
    async fn failed_observe_invalidates_old_refs() {
        let t = tool_with_snapshot(|| panic!("observe must not invoke an element"));
        assert!(t.resolve_ref(1).is_ok());
        let result = t.execute_authorized(crate::capability::ComputerAction::A11yObserve.id(), json!({"action": "observe"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("simulated observe failure"));
        assert!(t.resolve_ref(1).is_err());
    }

    #[test]
    fn target_numbers_are_checked_before_narrowing() {
        for name in ["ref", "window_id"] {
            for value in [json!(u32::MAX as u64 + 1), json!(u64::MAX), json!(-1), json!("1")] {
                let input = json!({(name): value});
                assert!(require_u32(&input, name).is_err());
            }
            assert_eq!(require_u32(&json!({(name): u32::MAX}), name).unwrap(), u32::MAX);
        }
        for (x, y) in [(i64::MAX, 0), (0, i64::MIN), (i32::MAX as i64 + 1, 0)] {
            assert!(require_xy(&json!({"x": x, "y": y}), "x", "y").is_err());
        }
        assert_eq!(require_xy(&json!({"x": i32::MIN, "y": i32::MAX}), "x", "y").unwrap(),
            (i32::MIN, i32::MAX));
    }

    #[test]
    fn optional_scroll_target_is_absent_or_a_complete_valid_pair() {
        assert_eq!(optional_xy(&json!({})).unwrap(), None);
        assert_eq!(optional_xy(&json!({"x": null, "y": null})).unwrap(), None);
        assert_eq!(optional_xy(&json!({"x": -2, "y": 4})).unwrap(), Some((-2, 4)));
        for input in [json!({"x": 1}), json!({"y": 1}), json!({"x": "1", "y": 2}),
            json!({"x": 1, "y": null}), json!({"x": i64::MAX, "y": 1})] {
            assert!(optional_xy(&input).is_err(), "{input}");
        }
    }

    #[tokio::test]
    async fn click_element_without_ref_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "click_element"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("ref"), "{}", result.content);
    }

    #[tokio::test]
    async fn click_element_without_snapshot_is_error() {
        // No observe has run, so there is no snapshot to resolve the ref against.
        // (On macOS the engine initializes; the error is about the missing
        // snapshot, surfaced as a non-panicking ToolResult error.)
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "click_element", "ref": 1}))
            .await;
        assert!(result.is_error);
    }

    // --- execute error paths (no real screen/input needed) ---

    #[tokio::test]
    async fn unknown_action_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "fly"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("fly"), "{}", result.content);
    }

    #[tokio::test]
    async fn missing_action_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({})).await;
        assert!(result.is_error);
        assert!(result.content.contains("action"), "{}", result.content);
    }

    #[tokio::test]
    async fn canonical_action_grant_cannot_authorize_another_operation_group() {
        let result = tool()
            .execute_authorized(
                crate::capability::COMPUTER_OBSERVE_ACTION_ID,
                json!({"action":"launch","target":"notepad"}),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("COMPUTER_ACTION_NOT_GRANTED"));
        assert!(
            result
                .content
                .contains(crate::capability::COMPUTER_LAUNCH_ACTION_ID)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn canonical_action_grant_admits_only_its_native_operation_group() {
        let result = tool()
            .execute_authorized(
                crate::capability::COMPUTER_OBSERVE_ACTION_ID,
                json!({"action":"wait","seconds":0}),
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
    }

    #[tokio::test]
    async fn canonical_a11y_observe_never_captures_or_emits_screen_pixels() {
        let t = tool();
        *t.a11y.lock().unwrap() = Some(Ok(Arc::new(SnapshotEngine)));
        *t.last_capture.lock().unwrap() = Some(CaptureGeometry {
            img_w: 100,
            img_h: 100,
            logical_w: 100,
            logical_h: 100,
            origin_x: 0,
            origin_y: 0,
        });

        let result = t
            .execute_authorized(
                crate::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID,
                json!({"action":"observe"}),
            )
            .await;

        assert!(!result.is_error, "{}", result.content);
        assert!(result.images.is_empty());
        assert!(result.content.contains("Accessibility snapshot (gen 9)"));
        assert!(result.content.contains("Pixel overlay intentionally omitted"));
        assert!(t.last_capture.lock().unwrap().is_none());
        assert!(t.resolve_ref(1).is_ok());
    }

    #[tokio::test]
    async fn canonical_a11y_observe_marks_an_incomplete_node_budget_without_tail_refs() {
        let t = tool();
        *t.a11y.lock().unwrap() = Some(Ok(Arc::new(TruncatedSnapshotEngine)));

        let result = t
            .execute_authorized(
                crate::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID,
                json!({"action":"observe"}),
            )
            .await;

        assert!(!result.is_error, "{}", result.content);
        assert!(result.images.is_empty());
        assert!(result.content.contains("120 element(s)"));
        assert!(result.content.contains("a11y tree truncated to the node budget"));
        assert!(result.content.contains("AX_ITEM_000"));
        assert!(result.content.contains("AX_ITEM_119"));
        assert!(!result.content.contains("AX_OMITTED_SENTINEL_199"));
        assert!(result.content.len() <= 64 * 1024);
        assert!(t.resolve_ref(120).is_ok());
        assert!(t.resolve_ref(121).is_err());
    }

    #[tokio::test]
    async fn click_without_coordinates_is_error_naming_params() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "left_click"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("x"), "{}", result.content);
        assert!(result.content.contains("y"), "{}", result.content);
    }

    #[tokio::test]
    async fn click_with_only_x_is_error_naming_y() {
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "left_click", "x": 10}))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("`y`"), "{}", result.content);
    }

    #[tokio::test]
    async fn drag_without_end_is_error_naming_params() {
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "left_click_drag", "start_x": 1, "start_y": 2}))
            .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("end_x") && result.content.contains("end_y"),
            "{}",
            result.content
        );
    }

    #[tokio::test]
    async fn type_without_text_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "type"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("text"), "{}", result.content);
    }

    #[tokio::test]
    async fn key_without_key_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "key"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("key"), "{}", result.content);
    }

    #[tokio::test]
    async fn key_with_unknown_combo_is_error() {
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "key", "key": "cmd+notakey"}))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("notakey"), "{}", result.content);
    }

    #[tokio::test]
    async fn scroll_without_direction_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "scroll"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("direction"), "{}", result.content);
    }

    #[tokio::test]
    async fn scroll_with_bad_direction_is_error() {
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "scroll", "direction": "sideways"}))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("sideways"), "{}", result.content);
    }

    #[tokio::test]
    async fn focus_window_without_id_is_error() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Input.id(), json!({"action": "focus_window"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("window_id"), "{}", result.content);
    }

    #[tokio::test]
    async fn screenshot_with_bad_display_type_is_error() {
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "screenshot", "display": "main"}))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("display"), "{}", result.content);
    }

    // --- wait ---

    #[tokio::test(start_paused = true)]
    async fn wait_clamps_to_five_seconds() {
        let start = tokio::time::Instant::now();
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "wait", "seconds": 60}))
            .await;
        assert!(!result.is_error, "{}", result.content);
        // Paused-clock runtime: the virtual elapsed time is the slept time.
        assert_eq!(start.elapsed(), Duration::from_secs(5));
        assert!(result.content.contains('5'), "{}", result.content);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_default_is_one_second() {
        let start = tokio::time::Instant::now();
        let result = tool().execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "wait"})).await;
        assert!(!result.is_error);
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn wait_negative_clamps_to_zero() {
        let start = tokio::time::Instant::now();
        let result = tool()
            .execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "wait", "seconds": -3}))
            .await;
        assert!(!result.is_error);
        assert_eq!(start.elapsed(), Duration::from_secs(0));
    }

    #[tokio::test]
    async fn wait_with_zero_seconds_succeeds() {
        let t = tool();
        let input = json!({"action": "wait", "seconds": 0});
        let result = t.execute_authorized(crate::capability::ComputerAction::Observe.id(), input).await;
        assert!(!result.is_error);
    }

    // --- coordinate mapping through stored geometry ---

    #[test]
    fn to_screen_identity_without_capture() {
        assert_eq!(tool().to_screen(123, 456), (123, 456));
    }

    #[test]
    fn to_screen_maps_with_capture_geometry() {
        let t = tool();
        *t.last_capture.lock().unwrap() = Some(crate::screen::CaptureGeometry {
            img_w: 1568,
            img_h: 980,
            logical_w: 1440,
            logical_h: 900,
            origin_x: 0,
            origin_y: 0,
        });
        assert_eq!(t.to_screen(0, 0), (0, 0));
        assert_eq!(t.to_screen(1567, 979), (1439, 899));
    }

    #[test]
    fn to_screen_applies_monitor_origin() {
        let t = tool();
        *t.last_capture.lock().unwrap() = Some(crate::screen::CaptureGeometry {
            img_w: 1000,
            img_h: 800,
            logical_w: 1000,
            logical_h: 800,
            origin_x: 1440,
            origin_y: -100,
        });
        assert_eq!(t.to_screen(10, 20), (1450, -80));
    }

    // --- real-device tests ---

    // Requires a display and Screen Recording permission.
    #[tokio::test]
    #[ignore]
    async fn screenshot_real() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "screenshot"})).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].media_type, "image/png");
        assert!(result.content.contains("Screenshot captured"));
    }

    // Requires Accessibility permission.
    #[tokio::test]
    #[ignore]
    async fn cursor_position_real() {
        let result = tool().execute_authorized(crate::capability::ComputerAction::Observe.id(), json!({"action": "cursor_position"})).await;
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("Cursor position"));
    }
}
