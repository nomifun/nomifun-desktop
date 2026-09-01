//! Computer-use capabilities exposed through the Gateway registry.
//!
//! Each handler is a typed request adapter over the shared
//! `nomi_computer::ComputerTool` registry. Read-only operations use
//! `EffectClass::Read`; input operations use `EffectClass::Write`. Host
//! availability remains owned by the registry and is reported without a
//! fallback implementation. Only compiled with the `computer-use` feature.

use std::future::Future;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::computer_registry::{ComputerRegistry, tool_result_to_value};
use crate::deps::{CallerCtx, CompatibilityCapabilityHost};
use crate::registry::{Capability, CapabilityMeta, EffectClass};

const UNAVAILABLE_ERROR: &str = "computer-use is not available on this host";

fn unavailable_error() -> Value {
    json!({ "error": UNAVAILABLE_ERROR })
}

fn registry_from_option(
    registry: Option<&ComputerRegistry>,
) -> Result<&ComputerRegistry, Value> {
    registry.ok_or_else(unavailable_error)
}

#[derive(Clone)]
struct ComputerCapabilityDeps {
    registry: Option<Arc<ComputerRegistry>>,
}

fn scoped(deps: &CompatibilityCapabilityHost) -> ComputerCapabilityDeps {
    ComputerCapabilityDeps {
        registry: deps.computer_registry.clone(),
    }
}

fn adapt<P, F, Fut>(
    handler: F,
) -> impl Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, P) -> Fut + Send + Sync + 'static
where
    P: Send + 'static,
    F: Fn(Arc<ComputerCapabilityDeps>, CallerCtx, P) -> Fut
        + Send
        + Sync
        + Clone
        + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    move |deps, ctx, params| {
        handler(
            Arc::new(scoped(deps.as_ref())),
            ctx,
            params,
        )
    }
}

async fn run(deps: &ComputerCapabilityDeps, input: Value) -> Value {
    match registry_from_option(deps.registry.as_deref()) {
        Ok(reg) => tool_result_to_value(reg.execute(input).await),
        Err(e) => e,
    }
}

// ---- typed request parameters -----------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoParams {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RefParams {
    /// Element number `[ref]` from the most recent `nomi_computer_snapshot`.
    r#ref: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetValueParams {
    /// Element number `[ref]` from the most recent snapshot.
    r#ref: u32,
    /// The text to set into the element.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct XyParams {
    /// X coordinate in pixels of the most recent screenshot.
    x: i64,
    /// Y coordinate in pixels of the most recent screenshot.
    y: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TypeParams {
    /// The text to type into the focused control.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KeyParams {
    /// Key or combo to press, e.g. "enter" or "ctrl+a".
    key: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScrollParams {
    /// Scroll direction: up, down, left, or right.
    direction: String,
    /// Wheel clicks (default 3).
    #[serde(default)]
    amount: Option<i64>,
    /// Optional X to scroll at (screenshot pixels).
    #[serde(default)]
    x: Option<i64>,
    /// Optional Y to scroll at (screenshot pixels).
    #[serde(default)]
    y: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LaunchParams {
    /// What to open: a file/folder path or an app name. Web URLs (http/https)
    /// are rejected; use the browser capabilities for web pages.
    target: String,
    /// Optional application to open the target WITH (e.g. a specific editor).
    #[serde(default)]
    app: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScreenshotParams {
    /// Optional display index to capture (default: primary).
    #[serde(default)]
    display: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitParams {
    /// Seconds to wait (max 5).
    #[serde(default)]
    seconds: Option<f64>,
}

// ---- handlers (forward to the shared tool's action dispatcher) --------------

async fn snapshot(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, _p: NoParams) -> Value {
    run(&deps, json!({ "action": "observe" })).await
}
async fn screenshot(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: ScreenshotParams) -> Value {
    run(&deps, json!({ "action": "screenshot", "display": p.display })).await
}
async fn click(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: RefParams) -> Value {
    run(&deps, json!({ "action": "click_element", "ref": p.r#ref })).await
}
async fn right_click(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: RefParams) -> Value {
    run(&deps, json!({ "action": "right_click_element", "ref": p.r#ref })).await
}
async fn double_click(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: RefParams) -> Value {
    run(&deps, json!({ "action": "double_click_element", "ref": p.r#ref })).await
}
async fn set_value(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: SetValueParams) -> Value {
    run(&deps, json!({ "action": "set_element_value", "ref": p.r#ref, "text": p.text })).await
}
async fn click_xy(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: XyParams) -> Value {
    run(&deps, json!({ "action": "left_click", "x": p.x, "y": p.y })).await
}
async fn type_text(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: TypeParams) -> Value {
    run(&deps, json!({ "action": "type", "text": p.text })).await
}
async fn key(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: KeyParams) -> Value {
    run(&deps, json!({ "action": "key", "key": p.key })).await
}
async fn scroll(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: ScrollParams) -> Value {
    run(
        &deps,
        json!({ "action": "scroll", "direction": p.direction, "amount": p.amount, "x": p.x, "y": p.y }),
    )
    .await
}
async fn launch(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: LaunchParams) -> Value {
    run(&deps, json!({ "action": "launch", "target": p.target, "app": p.app })).await
}
async fn list_windows(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, _p: NoParams) -> Value {
    run(&deps, json!({ "action": "list_windows" })).await
}
async fn cursor_position(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, _p: NoParams) -> Value {
    run(&deps, json!({ "action": "cursor_position" })).await
}
async fn wait(deps: Arc<ComputerCapabilityDeps>, _ctx: CallerCtx, p: WaitParams) -> Value {
    run(&deps, json!({ "action": "wait", "seconds": p.seconds })).await
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<NoParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_snapshot",
            "computer",
            "Read the desktop accessibility tree (windows → controls, numbered [ref] + Set-of-Marks overlay). Do this first, then act on a [ref]. Re-run after any UI change. Read-only.",
            EffectClass::Read,
        ),
        adapt(snapshot),
    ));
    out.push(Capability::new::<ScreenshotParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_screenshot",
            "computer",
            "Capture the screen as a PNG (optional `display` index).",
            EffectClass::Read,
        ),
        adapt(screenshot),
    ));
    out.push(Capability::new::<RefParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_click",
            "computer",
            "Activate the element with the given `ref` from the latest snapshot.",
            EffectClass::Write,
        ),
        adapt(click),
    ));
    out.push(Capability::new::<RefParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_right_click",
            "computer",
            "Right-click the element with the given `ref` (opens its context menu).",
            EffectClass::Write,
        ),
        adapt(right_click),
    ));
    out.push(Capability::new::<RefParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_double_click",
            "computer",
            "Double-click the element with the given `ref`.",
            EffectClass::Write,
        ),
        adapt(double_click),
    ));
    out.push(Capability::new::<SetValueParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_set_value",
            "computer",
            "Set the `text` value of the element with the given `ref` (good for text fields).",
            EffectClass::Write,
        ),
        adapt(set_value),
    ));
    out.push(Capability::new::<XyParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_click_xy",
            "computer",
            "Left-click at pixel coordinates (`x`, `y`) of the most recent screenshot. Prefer click-by-ref when possible.",
            EffectClass::Write,
        ),
        adapt(click_xy),
    ));
    out.push(Capability::new::<TypeParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_type",
            "computer",
            "Type the `text` string into the focused control.",
            EffectClass::Write,
        ),
        adapt(type_text),
    ));
    out.push(Capability::new::<KeyParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_key",
            "computer",
            "Press a key or combo, e.g. \"enter\" or \"ctrl+a\".",
            EffectClass::Write,
        ),
        adapt(key),
    ));
    out.push(Capability::new::<ScrollParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_scroll",
            "computer",
            "Scroll in `direction` (up/down/left/right) by `amount` wheel clicks, optionally at (`x`, `y`).",
            EffectClass::Write,
        ),
        adapt(scroll),
    ));
    out.push(Capability::new::<LaunchParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_launch",
            "computer",
            "Open an application, file, or folder through the host desktop integration. Web URLs (http/https) are rejected; use browser capabilities for web pages.",
            EffectClass::Write,
        ),
        adapt(launch),
    ));
    out.push(Capability::new::<NoParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_list_windows",
            "computer",
            "List open windows with ids, titles, positions and sizes.",
            EffectClass::Read,
        ),
        adapt(list_windows),
    ));
    out.push(Capability::new::<NoParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_cursor_position",
            "computer",
            "Report the mouse cursor position in screenshot coordinates.",
            EffectClass::Read,
        ),
        adapt(cursor_position),
    ));
    out.push(Capability::new::<WaitParams, _, _>(
        CapabilityMeta::new(
            "nomi_computer_wait",
            "computer",
            "Pause for `seconds` (max 5) to let the UI settle.",
            EffectClass::Read,
        ),
        adapt(wait),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOL_NAMES: [&str; 14] = [
        "nomi_computer_snapshot",
        "nomi_computer_screenshot",
        "nomi_computer_click",
        "nomi_computer_right_click",
        "nomi_computer_double_click",
        "nomi_computer_set_value",
        "nomi_computer_click_xy",
        "nomi_computer_type",
        "nomi_computer_key",
        "nomi_computer_scroll",
        "nomi_computer_launch",
        "nomi_computer_list_windows",
        "nomi_computer_cursor_position",
        "nomi_computer_wait",
    ];

    #[test]
    fn registers_the_complete_typed_computer_surface() {
        let mut capabilities = Vec::new();
        register(&mut capabilities);

        assert_eq!(capabilities.len(), TOOL_NAMES.len());
        let names = capabilities
            .iter()
            .map(|capability| capability.meta.name)
            .collect::<Vec<_>>();
        assert_eq!(names.as_slice(), TOOL_NAMES.as_slice());

        for capability in capabilities {
            assert_eq!(capability.meta.domain, "computer");
            assert!(
                capability.input_schema.contains_key("properties"),
                "{} must expose a typed object schema",
                capability.meta.name
            );
            assert_eq!(
                capability.input_schema.get("additionalProperties"),
                Some(&json!(false)),
                "{} must reject unknown request fields",
                capability.meta.name
            );
        }
    }

    #[test]
    fn typed_requests_reject_unknown_fields() {
        let mut capabilities = Vec::new();
        register(&mut capabilities);

        for (name, args) in [
            ("nomi_computer_snapshot", json!({"unexpected": true})),
            ("nomi_computer_click", json!({"ref": 1, "unexpected": true})),
            (
                "nomi_computer_scroll",
                json!({"direction": "down", "unexpected": true}),
            ),
            (
                "nomi_computer_launch",
                json!({"target": "notepad", "unexpected": true}),
            ),
        ] {
            let capability = capabilities
                .iter()
                .find(|capability| capability.meta.name == name)
                .expect("computer capability must be registered");
            assert!(
                capability.validate_arguments(&args).is_err(),
                "{name} must reject unknown request fields"
            );
        }
    }

    #[test]
    fn unavailable_host_returns_the_exact_error_envelope() {
        let error = match registry_from_option(None) {
            Ok(_) => panic!("missing registry must fail closed"),
            Err(error) => error,
        };
        assert_eq!(error, json!({ "error": UNAVAILABLE_ERROR }));
    }
}
