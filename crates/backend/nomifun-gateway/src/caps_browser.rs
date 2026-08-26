//! Browser-domain capabilities (feature-gated). Lets a remote Agent drive a
//! lane in the application-owned browser hub. Ownership is the trusted runtime
//! plus logical lane name; companions are attribution only.
//!
//! The GW2 out-of-band approval state machine still gates irreversible browser
//! actions for default callers, while full-auto/yolo callers bypass that hold to
//! keep browser use low-friction. Browser tools are NOT denied on the Channel
//! surface: remote browser driving is the entire point.
//!
//! Only compiled when the `browser-use` feature is on.

use std::sync::Arc;

use nomi_browser::{
    ApprovalTier, OUT_OF_BAND_CONFIRMED_KEY, managed_result_envelope,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};

use crate::browser_registry::{
    BrowserRegistry, browser_result_to_value, platform_error_to_value,
};
use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier};

// ── params ────────────────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NavigateParams {
    /// The URL to load in the caller's browser.
    url: String,
    /// Open in a new tab instead of the current one (default false).
    #[serde(default)]
    new_tab: Option<bool>,
    /// Optional logical lane. Defaults to `default`.
    #[serde(default)]
    lane: Option<String>,
    /// Optional owner-scoped Lane handle. Cannot cross runtime ownership.
    #[serde(default)]
    lane_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObserveParams {
    /// Optional cap on the aria-snapshot depth (for huge pages).
    #[serde(default)]
    max_depth: Option<u64>,
    /// Optional logical lane. Defaults to `default`.
    #[serde(default)]
    lane: Option<String>,
    /// Optional owner-scoped Lane handle. Cannot cross runtime ownership.
    #[serde(default)]
    lane_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ActParams {
    /// The facade action name (click / type / scroll / screenshot /
    /// get_page_text / back / press_key / …). Re-observe after any action that
    /// changes the page (refs go stale).
    action: String,
    /// Optional logical lane. Defaults to `default`.
    #[serde(default)]
    lane: Option<String>,
    /// Optional owner-scoped Lane handle. Cannot cross runtime ownership.
    #[serde(default)]
    lane_id: Option<String>,
    /// Action-specific params (ref / text / url / keys / …), passed through
    /// verbatim to the browser facade.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConfirmParams {
    /// The call_id from an `approval_required` envelope.
    call_id: String,
    /// "proceed_once" to approve the held irreversible action, "cancel" to deny.
    #[serde(default)]
    option: Option<String>,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenParams {
    #[serde(default)]
    lane_name: Option<String>,
    /// Keep this Lane alive across ordinary Agent turn cleanup for user-requested
    /// long-lived media/download work. Explicit close and owner teardown still
    /// reclaim it.
    #[serde(default)]
    keep_alive: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LaneIdParams {
    #[serde(default)]
    lane_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum CrawlConcurrency {
    Auto(String),
    Fixed(u8),
}

impl<'de> Deserialize<'de> for CrawlConcurrency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::String(value) if value == "auto" => Ok(Self::Auto(value)),
            Value::Number(number) => match number.as_u64() {
                Some(value @ 1..=8) => Ok(Self::Fixed(value as u8)),
                _ => Err(de::Error::custom(
                    "`concurrency` must be \"auto\" or an integer from 1 through 8",
                )),
            },
            _ => Err(de::Error::custom(
                "`concurrency` must be \"auto\" or an integer from 1 through 8",
            )),
        }
    }
}

impl JsonSchema for CrawlConcurrency {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "BrowserCrawlConcurrency".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "description": "Bounded browser_crawl_many concurrency: \"auto\" or an integer from 1 through 8.",
            "oneOf": [
                {"type": "string", "enum": ["auto"]},
                {"type": "integer", "minimum": 1, "maximum": 8}
            ]
        })
    }
}

fn deserialize_optional_crawl_concurrency<'de, D>(
    deserializer: D,
) -> Result<Option<CrawlConcurrency>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    serde_json::from_value(value)
        .map(Some)
        .map_err(de::Error::custom)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlManyParams {
    urls: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_optional_crawl_concurrency")]
    #[schemars(with = "CrawlConcurrency")]
    concurrency: Option<CrawlConcurrency>,
    #[serde(default)]
    schema: Option<Value>,
}

// ── per-caller registry + GW2 helpers (ported verbatim) ───────────────────

fn registry(deps: &GatewayDeps) -> Result<&BrowserRegistry, Value> {
    deps
        .browser_registry
        .as_ref()
        .ok_or_else(|| {
            json!({
                "error": "browser tools are not available on this desktop",
                "code": "browser_unavailable",
            })
        })
}

fn managed_envelope(
    result: Result<nomi_types::tool::ToolResult, nomifun_browser_platform::BrowserPlatformError>,
) -> Value {
    match result {
        Ok(result) => {
            let mut envelope = managed_result_envelope(result);
            let images = envelope
                .as_object_mut()
                .and_then(|object| object.remove("_mcp_images"))
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .map(|mut image| {
                    if let Some(object) = image.as_object_mut()
                        && let Some(mime_type) = object.remove("mime_type")
                    {
                        object.insert("media_type".to_owned(), mime_type);
                    }
                    image
                })
                .collect::<Vec<_>>();
            if !images.is_empty()
                && let Some(result) = envelope.get_mut("result").and_then(Value::as_object_mut)
            {
                result.insert("images".to_owned(), Value::Array(images));
            }
            envelope
        }
        Err(error) => platform_error_to_value(error),
    }
}

/// Strip any caller-supplied out-of-band sentinel before classify/forward (trust boundary).
fn sanitize_out_of_band(mut input: Value) -> Value {
    if let Some(obj) = input.as_object_mut() {
        obj.remove(OUT_OF_BAND_CONFIRMED_KEY);
    }
    input
}

fn approval_required_value(call_id: &str, action: &str, args: &Value) -> Value {
    json!({
        "result": {
            "approval_required": {
                "call_id": call_id,
                "title": format!("Approve irreversible browser action: {action}"),
                "description": describe_pending(action, args),
                "how_to": "This action is irreversible (submit / payment / delete / send) and the \
                           caller does not auto-approve Browser approval. Relay this to the user; \
                           once they approve, call nomi_browser_confirm with this call_id and option \
                           \"proceed_once\" (or \"cancel\" to deny).",
                "options": [
                    {"label": "Approve once", "value": "proceed_once"},
                    {"label": "Deny", "value": "cancel"},
                ],
            }
        }
    })
}

fn describe_pending(action: &str, args: &Value) -> String {
    let detail = match action {
        "navigate" => args.get("url").and_then(Value::as_str).map(|u| format!("navigate to {u}")),
        "click" => args.get("ref").and_then(Value::as_str).map(|r| format!("click [ref={r}]")),
        "press_key" => args.get("keys").and_then(Value::as_str).map(|k| format!("press {k}")),
        "reload" => Some("reload the page".to_string()),
        _ => None,
    };
    match detail {
        Some(d) => format!("Will {d} — irreversible (may submit / pay / delete / send)."),
        None => format!("Will run irreversible action `{action}` (may submit / pay / delete / send)."),
    }
}

/// Gate an outbound action through out-of-band approval. `input` MUST already be
/// sanitized. Returns `Some(json)` to short-circuit, `None` to proceed.
fn caller_bypasses_browser_approval(ctx: &CallerCtx) -> bool {
    matches!(
        ctx.session_mode.as_deref().map(str::trim),
        // `agent-full-access` = codex bridge (@agentclientprotocol) native id;
        // `full-access` = its pre-022 predecessor, kept for persisted sessions.
        Some("yolo" | "yoloNoSandbox" | "full-access" | "agent-full-access" | "bypassPermissions")
    )
}

fn gw2_gate(
    ctx: &CallerCtx,
    registry: &BrowserRegistry,
    lane: Option<&str>,
    action: &str,
    input: &Value,
) -> Option<Value> {
    if caller_bypasses_browser_approval(ctx) {
        return None;
    }
    match registry.classify(ctx, lane, action, input) {
        Ok(tier) if tier != ApprovalTier::Irreversible => return None,
        Ok(_) => {}
        Err(error) => return Some(platform_error_to_value(error)),
    }
    match registry.stash_pending(ctx, lane, input) {
        Ok(Some(call_id)) => {
            Some(approval_required_value(&call_id, action, input))
        }
        Ok(None) => Some(json!({
            "error": "too many browser actions are awaiting approval; resolve or cancel some via \
                      nomi_browser_confirm before issuing more irreversible actions"
        })),
        Err(error) => Some(platform_error_to_value(error)),
    }
}

// ── handlers ────────────────────────────────────────────────────────────────

async fn navigate(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: NavigateParams) -> Value {
    let registry = match registry(&deps) {
        Ok(registry) => registry,
        Err(e) => return e,
    };
    let lane_name = match registry
        .resolve_lane_selector(&ctx, p.lane.as_deref(), p.lane_id.as_deref())
        .await
    {
        Ok(lane_name) => lane_name,
        Err(error) => return platform_error_to_value(error),
    };
    let input = json!({
        "action": "navigate",
        "url": p.url,
        "new_tab": p.new_tab.unwrap_or(false),
        "lane_id": p.lane_id,
    });
    if let Some(short_circuit) =
        gw2_gate(&ctx, registry, Some(&lane_name), "navigate", &input)
    {
        return short_circuit;
    }
    managed_envelope(
        registry
            .dispatch_managed(&ctx, p.lane.as_deref(), input)
            .await,
    )
}

async fn observe(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ObserveParams) -> Value {
    let registry = match registry(&deps) {
        Ok(registry) => registry,
        Err(e) => return e,
    };
    let lane_name = match registry
        .resolve_lane_selector(&ctx, p.lane.as_deref(), p.lane_id.as_deref())
        .await
    {
        Ok(lane_name) => lane_name,
        Err(error) => return platform_error_to_value(error),
    };
    let mut input = json!({"action": "observe", "lane_id": p.lane_id});
    if let Some(d) = p.max_depth {
        input["max_depth"] = json!(d);
    }
    if let Some(short_circuit) =
        gw2_gate(&ctx, registry, Some(&lane_name), "observe", &input)
    {
        return short_circuit;
    }
    managed_envelope(
        registry
            .dispatch_managed(&ctx, p.lane.as_deref(), input)
            .await,
    )
}

async fn act(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ActParams) -> Value {
    let registry = match registry(&deps) {
        Ok(registry) => registry,
        Err(e) => return e,
    };
    // Reconstruct the facade input from the passthrough params, strip any
    // caller-supplied sentinel (trust boundary), then set the validated action.
    let mut input = sanitize_out_of_band(Value::Object(p.rest));
    input["action"] = json!(p.action);
    input["lane_id"] = p
        .lane_id
        .as_ref()
        .map(|lane_id| Value::String(lane_id.clone()))
        .unwrap_or(Value::Null);
    let lane_name = match registry
        .resolve_lane_selector(&ctx, p.lane.as_deref(), p.lane_id.as_deref())
        .await
    {
        Ok(lane_name) => lane_name,
        Err(error) => return platform_error_to_value(error),
    };
    if let Some(short_circuit) =
        gw2_gate(&ctx, registry, Some(&lane_name), &p.action, &input)
    {
        return short_circuit;
    }
    managed_envelope(
        registry
            .dispatch_managed(&ctx, p.lane.as_deref(), input)
            .await,
    )
}

async fn dispatch_management(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    input: Value,
) -> Value {
    let registry = match registry(&deps) {
        Ok(registry) => registry,
        Err(error) => return error,
    };
    managed_envelope(registry.dispatch_managed(&ctx, None, input).await)
}

async fn browser_open(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: OpenParams) -> Value {
    let mut input = json!({
        "action": "browser_open",
        "lane_name": p.lane_name,
    });
    if let Some(keep_alive) = p.keep_alive {
        input["keep_alive"] = json!(keep_alive);
    }
    dispatch_management(
        deps,
        ctx,
        input,
    )
    .await
}

async fn browser_fork(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: OpenParams) -> Value {
    let mut input = json!({
        "action": "browser_fork",
        "lane_name": p.lane_name,
    });
    if let Some(keep_alive) = p.keep_alive {
        input["keep_alive"] = json!(keep_alive);
    }
    dispatch_management(
        deps,
        ctx,
        input,
    )
    .await
}

async fn browser_list(deps: Arc<GatewayDeps>, ctx: CallerCtx, _p: EmptyParams) -> Value {
    dispatch_management(deps, ctx, json!({"action": "browser_list"})).await
}

async fn browser_status(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    p: LaneIdParams,
) -> Value {
    dispatch_management(
        deps,
        ctx,
        json!({"action": "browser_status", "lane_id": p.lane_id}),
    )
    .await
}

async fn browser_close(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    p: LaneIdParams,
) -> Value {
    dispatch_management(
        deps,
        ctx,
        json!({"action": "browser_close", "lane_id": p.lane_id}),
    )
    .await
}

async fn browser_close_all(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    _p: EmptyParams,
) -> Value {
    dispatch_management(deps, ctx, json!({"action": "browser_close_all"})).await
}

async fn browser_crawl_many(
    deps: Arc<GatewayDeps>,
    ctx: CallerCtx,
    p: CrawlManyParams,
) -> Value {
    let mut input = json!({
        "action": "browser_crawl_many",
        "urls": p.urls,
    });
    if let Some(concurrency) = p.concurrency {
        input["concurrency"] = json!(concurrency);
    }
    if let Some(schema) = p.schema {
        input["schema"] = schema;
    }
    dispatch_management(deps, ctx, input).await
}

async fn confirm(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ConfirmParams) -> Value {
    let registry = match registry(&deps) {
        Ok(registry) => registry,
        Err(e) => return e,
    };
    let option = p.option.as_deref().map(str::trim).unwrap_or("cancel");
    let approve = matches!(option, "proceed_once" | "proceed_always" | "approve" | "yes");

    let pending = match registry.take_pending_for(&ctx, &p.call_id) {
        Ok(Some(pending)) => pending,
        Ok(None) => {
            return json!({"error": format!("no pending browser approval with call_id {} (already resolved, expired, or never existed)", p.call_id)});
        }
        Err(error) => return platform_error_to_value(error),
    };
    if !approve {
        return json!({
            "result": {
                "resolved": p.call_id,
                "approved": false,
                "text": "Denied. The irreversible browser action was not run."
            }
        });
    }
    let mut envelope = browser_result_to_value(
        registry.execute_confirmed(&ctx, pending).await,
    );
    if let Some(result) = envelope.get_mut("result").and_then(Value::as_object_mut) {
        result.insert("resolved".to_string(), json!(p.call_id));
        result.insert("approved".to_string(), json!(true));
    }
    envelope
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<OpenParams, _, _>(
        CapabilityMeta::new("nomi_browser_open", "browser", "Idempotently open the caller's default or named managed Browser Lane using the trusted host-selected interactive identity.", DangerTier::Write),
        browser_open,
    ));
    out.push(Capability::new::<OpenParams, _, _>(
        CapabilityMeta::new("nomi_browser_fork", "browser", "Create or open an additional managed Browser Lane using the trusted host-selected interactive identity and return its owner-scoped handle.", DangerTier::Write),
        browser_fork,
    ));
    out.push(Capability::new::<EmptyParams, _, _>(
        CapabilityMeta::new("nomi_browser_list", "browser", "List the managed Browser Lanes owned by this runtime, including queue, capacity, identity, epoch, and recovery state.", DangerTier::Read),
        browser_list,
    ));
    out.push(Capability::new::<LaneIdParams, _, _>(
        CapabilityMeta::new("nomi_browser_status", "browser", "Read one owner-scoped managed Browser Lane status, defaulting to the default Lane.", DangerTier::Read),
        browser_status,
    ));
    out.push(Capability::new::<LaneIdParams, _, _>(
        CapabilityMeta::new("nomi_browser_close", "browser", "Close one owner-scoped managed Browser Lane.", DangerTier::Write),
        browser_close,
    ));
    out.push(Capability::new::<EmptyParams, _, _>(
        CapabilityMeta::new("nomi_browser_close_all", "browser", "Close every managed Browser Lane owned by this runtime and no other runtime.", DangerTier::Write),
        browser_close_all,
    ));
    out.push(Capability::new::<CrawlManyParams, _, _>(
        CapabilityMeta::new("nomi_browser_crawl_many", "browser", "Read or extract an ordered bounded URL batch using Hub-managed Lanes with cleanup; the trusted host selects the crawl identity policy.", DangerTier::Read),
        browser_crawl_many,
    ));
    out.push(Capability::new::<NavigateParams, _, _>(
        CapabilityMeta::new("nomi_browser_navigate", "browser", "Load a URL in the caller's browser (optionally a new tab).", DangerTier::Write),
        navigate,
    ));
    out.push(Capability::new::<ObserveParams, _, _>(
        CapabilityMeta::new("nomi_browser_observe", "browser", "Read the page's accessibility tree (aria snapshot + ref table) to target later. Read-only.", DangerTier::Read),
        observe,
    ));
    out.push(Capability::new::<ActParams, _, _>(
        CapabilityMeta::new("nomi_browser_act", "browser", "Run any browser action (click/type/scroll/screenshot/...); irreversible actions are held for out-of-band approval.", DangerTier::Write),
        act,
    ));
    out.push(Capability::new::<ConfirmParams, _, _>(
        CapabilityMeta::new("nomi_browser_confirm", "browser", "Resolve a pending out-of-band browser approval (proceed_once / cancel).", DangerTier::Write),
        confirm,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_caller_supplied_out_of_band_sentinel() {
        let dirty = json!({"action": "click", "ref": "f0e1", OUT_OF_BAND_CONFIRMED_KEY: true});
        let clean = sanitize_out_of_band(dirty);
        assert!(clean.get(OUT_OF_BAND_CONFIRMED_KEY).is_none());
        assert_eq!(clean.get("action").and_then(Value::as_str), Some("click"));
    }

    #[test]
    fn describe_pending_surfaces_action_detail_without_secrets() {
        assert!(describe_pending("navigate", &json!({"url": "https://shop.test/pay"})).contains("shop.test/pay"));
        assert!(describe_pending("click", &json!({"ref": "f0e9"})).contains("f0e9"));
        let d = describe_pending("type", &json!({"text": "secret:CARD"}));
        assert!(!d.contains("secret:CARD"), "preview must not echo a secret reference: {d}");
    }

    #[test]
    fn approval_required_value_mirrors_confirmation_shape() {
        let v = approval_required_value(
            "019f6672-ed10-7193-8a86-7981f6c6feae",
            "click",
            &json!({"ref": "f0e3"}),
        );
        let ar = v
            .pointer("/result/approval_required")
            .expect("approval_required result");
        assert_eq!(
            ar.get("call_id").and_then(Value::as_str),
            Some("019f6672-ed10-7193-8a86-7981f6c6feae")
        );
        let opts = ar.get("options").and_then(Value::as_array).expect("options");
        let values: Vec<&str> = opts.iter().filter_map(|o| o.get("value").and_then(Value::as_str)).collect();
        assert!(values.contains(&"proceed_once") && values.contains(&"cancel"));
    }

    #[test]
    fn gw2_gate_fails_closed_without_an_injected_hub_and_identity() {
        let registry = BrowserRegistry::default_for_browser_use();
        let input = json!({"action": "press_key", "keys": "Enter"});

        let ctx = CallerCtx::default();
        let result = gw2_gate(&ctx, &registry, None, "press_key", &input)
            .expect("missing trusted browser authority must short-circuit");

        assert_eq!(
            result.get("code").and_then(Value::as_str),
            Some("browser_unavailable")
        );
    }

    #[test]
    fn gw2_gate_skips_irreversible_prompt_for_yolo_context() {
        let ctx = CallerCtx {
            session_mode: Some("yolo".to_owned()),
            ..Default::default()
        };
        let registry = BrowserRegistry::default_for_browser_use();
        let input = json!({"action": "press_key", "keys": "Enter"});

        let result = gw2_gate(&ctx, &registry, None, "press_key", &input);

        assert!(
            result.is_none(),
            "yolo gateway browser context should not return approval_required"
        );
    }

    #[test]
    fn act_flatten_captures_passthrough_params() {
        let p: ActParams = serde_json::from_value(json!({
            "action": "click",
            "lane_id": "lane-owned",
            "ref": "f0e1",
            "text": "hi"
        }))
        .unwrap();
        assert_eq!(p.action, "click");
        assert_eq!(p.rest.get("ref").and_then(Value::as_str), Some("f0e1"));
        assert_eq!(p.rest.get("text").and_then(Value::as_str), Some("hi"));
        assert!(p.lane.is_none());
        assert_eq!(p.lane_id.as_deref(), Some("lane-owned"));
        assert!(!p.rest.contains_key("lane_id"));
        assert!(!p.rest.contains_key("action"), "flatten must exclude the named action field");
    }

    #[test]
    fn registers_lane_management_and_lane_aware_legacy_contracts() {
        let mut capabilities = Vec::new();
        register(&mut capabilities);
        let names = capabilities
            .iter()
            .map(|capability| capability.meta.name)
            .collect::<Vec<_>>();
        for expected in [
            "nomi_browser_open",
            "nomi_browser_fork",
            "nomi_browser_list",
            "nomi_browser_status",
            "nomi_browser_close",
            "nomi_browser_close_all",
            "nomi_browser_crawl_many",
            "nomi_browser_navigate",
            "nomi_browser_observe",
            "nomi_browser_act",
            "nomi_browser_confirm",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        for capability in capabilities {
            if matches!(
                capability.meta.name,
                "nomi_browser_navigate" | "nomi_browser_observe" | "nomi_browser_act"
            ) {
                assert!(
                    capability
                        .input_schema
                        .get("properties")
                        .and_then(Value::as_object)
                        .is_some_and(|properties| properties.contains_key("lane_id")),
                    "{} must preserve legacy fields and accept lane_id: {:?}",
                    capability.meta.name,
                    capability.input_schema,
                );
            }
        }
    }

    #[test]
    fn management_schemas_do_not_expose_identity_policy_inputs() {
        let mut capabilities = Vec::new();
        register(&mut capabilities);

        for capability in capabilities {
            if matches!(
                capability.meta.name,
                "nomi_browser_open" | "nomi_browser_fork" | "nomi_browser_crawl_many"
            ) {
                let properties = capability
                    .input_schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("management capability schema properties");
                assert!(
                    !properties.contains_key("identity_mode"),
                    "{} must not expose identity_mode: {:?}",
                    capability.meta.name,
                    capability.input_schema,
                );
                assert!(
                    !properties.contains_key("authenticated"),
                    "{} must not expose authenticated: {:?}",
                    capability.meta.name,
                    capability.input_schema,
                );
            }
        }
    }

    #[test]
    fn legacy_identity_policy_fields_fail_closed() {
        assert!(
            serde_json::from_value::<OpenParams>(json!({
                "lane_name": "default",
                "identity_mode": "isolated"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CrawlManyParams>(json!({
                "urls": ["https://example.test"],
                "identity_mode": "authenticated_replica"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CrawlManyParams>(json!({
                "urls": ["https://example.test"],
                "authenticated": true
            }))
            .is_err()
        );
    }

    #[test]
    fn crawl_many_concurrency_accepts_only_auto_or_integer_one_through_eight() {
        for valid in [json!("auto"), json!(1), json!(8)] {
            assert!(
                serde_json::from_value::<CrawlManyParams>(json!({
                    "urls": ["https://example.test"],
                    "concurrency": valid,
                }))
                .is_ok()
            );
        }
        for invalid in [
            json!("AUTO"),
            json!("4"),
            json!(0),
            json!(-1),
            json!(9),
            json!(1.5),
            json!(4.0),
            json!(false),
            json!(null),
            json!({}),
            json!([]),
        ] {
            assert!(
                serde_json::from_value::<CrawlManyParams>(json!({
                    "urls": ["https://example.test"],
                    "concurrency": invalid,
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn crawl_many_concurrency_schema_is_bounded() {
        let mut capabilities = Vec::new();
        register(&mut capabilities);
        let capability = capabilities
            .iter()
            .find(|capability| capability.meta.name == "nomi_browser_crawl_many")
            .expect("crawl capability");
        let schema = &capability.input_schema["properties"]["concurrency"];
        let rendered = schema.to_string();
        assert!(rendered.contains("\"auto\""), "{schema}");
        assert!(rendered.contains("\"minimum\":1"), "{schema}");
        assert!(rendered.contains("\"maximum\":8"), "{schema}");
        assert!(!rendered.contains("\"null\""), "{schema}");
    }

    #[test]
    fn managed_screenshot_keeps_gateway_image_envelope() {
        let image = nomi_types::tool::ToolImage {
            media_type: "image/png".to_owned(),
            data: "QUJD".to_owned(),
        };
        let envelope = managed_envelope(Ok(
            nomi_types::tool::ToolResult::text(r#"{"ok":true,"action":"screenshot"}"#)
                .with_images(vec![image]),
        ));
        assert!(envelope.get("_mcp_images").is_none());
        assert_eq!(
            envelope
                .pointer("/result/images/0/media_type")
                .and_then(Value::as_str),
            Some("image/png")
        );
    }
}
