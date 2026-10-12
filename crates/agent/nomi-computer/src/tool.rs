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

/// True only for the exact semantic-ref stale rejection produced before the
/// Accessibility backend performs an action. The message also proves that the
/// Computer layer did not use its pixel fallback, so a role owner may settle
/// the reserved external effect as rejected instead of outcome-unknown.
pub fn is_proven_stale_input_rejection(result: &ToolResult) -> bool {
    result.is_error
        && result.images.is_empty()
        && result.content.starts_with("Accessibility action on [")
        && result.content.contains(" failed: stale reference:")
        && result.content.contains("No pixel fallback was performed;")
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
    screen_center: (i32, i32),
}

impl ComputerTool {
    pub fn new(config: &ComputerConfig) -> Self {
        Self {
            max_screenshot_edge: config.max_screenshot_edge,
            last_capture: Mutex::new(None),
            a11y: Mutex::new(None),
            last_snapshot: Mutex::new(None),
        }
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
        *self.last_snapshot.lock().unwrap_or_else(|p| p.into_inner()) = None;
        *self.last_capture.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
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
            let (cx, cy) = entry.bounds.center();
            CachedEntry {
                display: entry.clone(),
                engine_ref: entry.r#ref,
                screen_center: (cx as i32, cy as i32),
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
            Ok(Err(e @ (A11yError::Unsupported { .. } | A11yError::Backend(_)))) => {
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
                let (sx, sy) = entry.screen_center;
                match input::click(sx, sy, enigo::Button::Left, 1).await {
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

    /// Set the text value of an element by `[ref]`. Accessibility elements use
    /// AXValue with a gated focus-then-type fallback.
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
            ElementAction::SetValue(text.clone()),
            |_| Self::set_value_by_typing(r, entry.screen_center, text),
        ).await
    }

    async fn set_value_by_typing(r: u32, (sx, sy): (i32, i32), text: String) -> ToolResult {
        if let Err(pe) = input::click(sx, sy, enigo::Button::Left, 1).await {
            return ToolResult::error(format!(
                "Could not focus element [{r}] to type into it: {pe}"
            ));
        }
        match input::type_text(text).await {
            Ok(()) => ToolResult::text(format!(
                "Set element [{r}] by clicking the field and typing the text. Run `observe` to verify."
            )),
            Err(te) => ToolResult::error(format!("Typing into element [{r}] failed: {te}")),
        }
    }

    /// Resolve a `[ref]` from the latest snapshot to its screen-space center,
    /// for a pixel gesture (right/double click) that has no semantic equivalent.
    fn ref_screen_center(&self, r: u32) -> Result<(i32, i32), String> {
        let (_, entry) = self.resolve_ref(r)?;
        Ok(entry.screen_center)
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
        let (sx, sy) = match self.ref_screen_center(r) {
            Ok(v) => v,
            Err(msg) => return ToolResult::error(msg),
        };
        match input::click(sx, sy, button, count).await {
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
                screen_center: (2, 4),
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
        let cases: [(AxOutcome, bool, bool); 7] = [
            (|| Err(A11yError::Stale("stale".into())), false, true),
            (|| Err(A11yError::NotFound("gone".into())), false, true),
            (|| Err(A11yError::Permission("denied".into())), false, true),
            (|| panic!("simulated worker failure"), false, true),
            (|| Err(A11yError::Backend("unsupported pattern".into())), true, false),
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
            ToolResult::text(
                "Accessibility action on [7] failed: stale reference: focus changed. No pixel \
                 fallback was performed; re-run observe before using element refs.",
            ),
        ] {
            assert!(!is_proven_stale_input_rejection(&result));
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
