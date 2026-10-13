//! Linux AT-SPI actor: a dedicated thread owns a current-thread tokio runtime
//! and the `AccessibilityConnection`; the synchronous `A11yEngine` methods
//! marshal here over a command channel and `block_on` the async AT-SPI calls.
//!
//! `invoke` prefers AT-SPI semantic actions (Action.do_action / EditableText /
//! grab_focus) — coordinate-free and reliable on both X11 and Wayland. Element
//! bounds come from Component.get_extents(Screen) (valid pixels on X11; often
//! unavailable on Wayland, where the tool's pixel fallback is degraded anyway).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::action::ActionProxy;
use atspi::proxy::component::ComponentProxy;
use atspi::proxy::editable_text::EditableTextProxy;
use atspi::{CoordType, Interface, InterfaceSet, ObjectRefOwned, State as AtspiState};

use crate::engine::{
    A11yError, Capabilities, Effect, ElementAction, ElementEntry, InputKind, ObserveOpts, Rect,
    Snapshot, SnapshotGen, Source, Target,
};
use crate::tree::format_entries;

enum Cmd {
    Capabilities(Sender<Capabilities>),
    Observe(ObserveOpts, Sender<Result<Snapshot, A11yError>>),
    ResolvePixelTarget(Target, SnapshotGen, Sender<Result<Rect, A11yError>>),
    Invoke(
        Target,
        SnapshotGen,
        ElementAction,
        Sender<Result<Effect, A11yError>>,
    ),
    Focus(i32, Sender<Result<Effect, A11yError>>),
}

pub struct ActorHandle {
    tx: Mutex<Sender<Cmd>>,
}

struct State {
    gen_counter: u64,
    current_gen: SnapshotGen,
    observed_window: Option<ObjectRefOwned>,
    registry: HashMap<u32, RegEntry>,
}

struct RegEntry {
    obj: ObjectRefOwned,
    bounds: Rect,
    role: String,
    name: Option<String>,
}

/// Session probe → honest `Capabilities`. AT-SPI tree-read works on X11 +
/// Wayland; input/coordinates/window-mgmt degrade on Wayland.
fn detect_caps() -> Capabilities {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland =
        session.eq_ignore_ascii_case("wayland") || std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = !wayland
        && (session.eq_ignore_ascii_case("x11") || std::env::var_os("DISPLAY").is_some());
    Capabilities {
        os: "linux".to_string(),
        tree_read: true,
        screenshot: true,
        semantic_action: true,
        synthetic_input: if x11 {
            InputKind::X11
        } else {
            // No reliable persistent unattended Wayland input without a portal grant.
            InputKind::Unsupported
        },
        window_management: x11,
    }
}

// ---- AT-SPI helpers (run on the actor thread's runtime) ------------------

/// Map an AT-SPI role to a stable lowercase name (Debug form, e.g. `pushbutton`,
/// `entry`, `text`). The model just needs readable, stable role names.
fn role_name(acc_role: Option<atspi::Role>) -> String {
    match acc_role {
        Some(r) => format!("{r:?}").to_lowercase(),
        None => "element".to_string(),
    }
}

fn is_click_action(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "click" | "activate" | "press" | "jump" | "open" | "do default" | "default"
    )
}

/// These names must come from GetName, not the localized GetActions result.
/// LeftClick may not silently invoke an unrelated default/open action.
fn select_left_click_action(names: &[String]) -> Option<usize> {
    names.iter().position(|name| {
        matches!(name.trim().to_lowercase().as_str(), "click" | "press" | "activate")
    })
}

fn require_interface(
    interfaces: InterfaceSet,
    required: Interface,
    r: u32,
) -> Result<(), A11yError> {
    if interfaces.contains(required) {
        Ok(())
    } else {
        Err(A11yError::Unsupported {
            capability: format!("AT-SPI {required:?}"),
            hint: format!("element [{r}] does not expose the required interface; no semantic action was invoked"),
        })
    }
}

async fn require_live_interface(
    acc: &AccessibleProxy<'_>,
    required: Interface,
    r: u32,
) -> Result<(), A11yError> {
    let interfaces = acc.get_interfaces().await.map_err(|e| {
        A11yError::Backend(format!("cannot verify interfaces of [{r}] before action: {e}"))
    })?;
    require_interface(interfaces, required, r)
}

fn require_action_count(count: i32, r: u32) -> Result<usize, A11yError> {
    if count < 0 {
        Err(A11yError::Backend(format!("element [{r}] returned an invalid AT-SPI action count: {count}")))
    } else if count == 0 {
        Err(A11yError::Unsupported {
            capability: "AT-SPI action".to_string(),
            hint: format!("element [{r}] exposes no AT-SPI action; no semantic action was invoked"),
        })
    } else {
        Ok(count as usize)
    }
}

async fn read_left_click_action(
    act: &ActionProxy<'_>,
    count: usize,
    r: u32,
) -> Result<usize, A11yError> {
    // Metadata is read-only. A deadline failure cannot be treated as confirmed
    // absence, and must never enter the fallback path. Stop at a known match
    // rather than allocating a vector using a remote application's count.
    tokio::time::timeout(Duration::from_secs(3), async {
        for index in 0..count {
            let name = act.get_name(index as i32).await.map_err(|e| {
                A11yError::Backend(format!("cannot read machine-readable action {index} for [{r}] before action: {e}"))
            })?;
            if select_left_click_action(std::slice::from_ref(&name)).is_some() {
                return Ok(index);
            }
        }
        Err(A11yError::Unsupported {
            capability: "AT-SPI left click".to_string(),
            hint: format!("element [{r}] exposes no click-compatible action; no semantic action was invoked"),
        })
    }).await.map_err(|_| A11yError::Backend(format!(
        "machine-readable action lookup for [{r}] timed out before action; absence was not established"
    )))?
}

/// Use only after a mutating AT-SPI method was called. Neither a negative
/// reply nor a transport/decode error proves that the remote handler had no
/// side effects, so callers must observe again without retrying or falling back.
fn require_mutating_reply<E: std::fmt::Display>(
    reply: Result<bool, E>,
    operation: &str,
    r: u32,
) -> Result<(), A11yError> {
    match reply {
        Ok(true) => Ok(()),
        Ok(false) => Err(A11yError::Uncertain(format!(
            "{operation} on [{r}] returned false after the mutating request; effects are not confirmed. Observe again without retrying or pixel fallback"
        ))),
        Err(e) => Err(A11yError::Uncertain(format!(
            "{operation} on [{r}] failed after the mutating request ({e}); the remote handler may have executed. Observe again without retrying or pixel fallback"
        ))),
    }
}

/// Find the focused (Active) toplevel window by scanning each application's
/// children. Pixel admission requires a unique Active window and a complete
/// scan; ordinary observation retains the headless first-window fallback.
async fn find_active_window(
    conn: &AccessibilityConnection,
    require_active: bool,
) -> Result<ObjectRefOwned, A11yError> {
    let zconn = conn.connection();
    let root = conn
        .root_accessible_on_registry()
        .await
        .map_err(|e| A11yError::Backend(format!("cannot read the AT-SPI registry root: {e}")))?;
    let apps = root.get_children().await.map_err(|e| {
        A11yError::Backend(format!("cannot list accessible applications: {e}"))
    })?;

    let mut first_window: Option<ObjectRefOwned> = None;
    let mut active_window: Option<ObjectRefOwned> = None;
    for app in apps {
        let app_acc = match app.as_accessible_proxy(zconn).await {
            Ok(acc) => acc,
            Err(e) if require_active => {
                return Err(A11yError::Backend(format!("cannot resolve application during active-window validation: {e}")));
            }
            Err(_) => continue,
        };
        let windows = match app_acc.get_children().await {
            Ok(windows) => windows,
            Err(e) if require_active => {
                return Err(A11yError::Backend(format!("cannot list windows during active-window validation: {e}")));
            }
            Err(_) => continue,
        };
        for win in windows {
            let win_acc = match win.as_accessible_proxy(zconn).await {
                Ok(acc) => acc,
                Err(e) if require_active => {
                    return Err(A11yError::Backend(format!("cannot resolve window during active-window validation: {e}")));
                }
                Err(_) => continue,
            };
            let states = match win_acc.get_state().await {
                Ok(states) => states,
                Err(e) if require_active => {
                    return Err(A11yError::Backend(format!("cannot read window state during active-window validation: {e}")));
                }
                Err(_) => Default::default(),
            };
            if states.contains(AtspiState::Active) {
                if !require_active {
                    return Ok(win);
                }
                if states.contains(AtspiState::Defunct) || states.contains(AtspiState::Stale) {
                    return Err(A11yError::Stale("the Active AT-SPI window is no longer live".to_string()));
                }
                if active_window.as_ref().is_some_and(|active| active != &win) {
                    return Err(A11yError::Stale("multiple AT-SPI windows report Active; cannot validate a pixel target".to_string()));
                }
                active_window = Some(win.clone());
            }
            if first_window.is_none() {
                first_window = Some(win);
            }
        }
    }
    if require_active {
        return active_window.ok_or_else(|| A11yError::Stale(
            "no AT-SPI window reports Active; observe again before pixel input".to_string(),
        ));
    }
    // No window reported Active (common headless) — fall back to the first one.
    first_window.ok_or_else(|| {
        A11yError::NotFound(
            "no accessible window found. Ensure the app exposes accessibility \
             (KDE: QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1; Electron: --force-renderer-accessibility)."
                .to_string(),
        )
    })
}

struct Collected {
    obj: ObjectRefOwned,
    role: String,
    name: Option<String>,
    value: Option<String>,
    states: Vec<String>,
    bounds: Rect,
}

/// Walk the window subtree (iteratively, to avoid async recursion), collecting
/// interactable elements with screen-pixel bounds.
async fn walk_window(
    conn: &AccessibilityConnection,
    window: ObjectRefOwned,
    opts: &ObserveOpts,
) -> (Vec<Collected>, bool) {
    let zconn = conn.connection();
    let mut out: Vec<Collected> = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<(ObjectRefOwned, usize)> = vec![(window, 0)];

    while let Some((obj, depth)) = stack.pop() {
        if out.len() >= opts.node_budget {
            truncated = true;
            break;
        }
        let Ok(acc) = obj.as_accessible_proxy(zconn).await else {
            continue;
        };
        let dest = acc.inner().destination().to_owned();
        let path = acc.inner().path().to_owned();

        let role = acc.get_role().await.ok();
        let name = acc.name().await.ok().filter(|s| !s.trim().is_empty());
        let states = acc.get_state().await.unwrap_or_default();

        let bounds = match ComponentProxy::builder(zconn)
            .destination(dest.clone())
            .and_then(|b| b.path(path.clone()))
        {
            Ok(builder) => match builder.build().await {
                Ok(comp) => comp.get_extents(CoordType::Screen).await.ok().map(|(x, y, w, h)| {
                    Rect {
                        x: x as f64,
                        y: y as f64,
                        w: w as f64,
                        h: h as f64,
                    }
                }),
                Err(_) => None,
            },
            Err(_) => None,
        };

        let has_action = match ActionProxy::builder(zconn)
            .destination(dest.clone())
            .and_then(|b| b.path(path.clone()))
        {
            Ok(builder) => match builder.build().await {
                Ok(act) => act.n_actions().await.map(|n| n > 0).unwrap_or(false),
                Err(_) => false,
            },
            Err(_) => false,
        };

        let focusable = states.contains(AtspiState::Focusable);
        let editable = states.contains(AtspiState::Editable);
        let enabled = states.contains(AtspiState::Enabled) || states.contains(AtspiState::Sensitive);

        if let Some(b) = bounds {
            if b.w > 0.0 && b.h > 0.0 && (has_action || focusable || editable) {
                let mut st = Vec::new();
                if !enabled {
                    st.push("disabled".to_string());
                }
                if states.contains(AtspiState::Focused) {
                    st.push("focused".to_string());
                }
                out.push(Collected {
                    obj: obj.clone(),
                    role: role_name(role),
                    name,
                    value: None,
                    states: st,
                    bounds: b,
                });
            }
        }

        if depth < opts.max_depth {
            if let Ok(children) = acc.get_children().await {
                for c in children {
                    stack.push((c, depth + 1));
                }
            }
        }
    }
    (out, truncated)
}

async fn do_observe(
    conn: &AccessibilityConnection,
    opts: &ObserveOpts,
    state: &mut State,
) -> Result<Snapshot, A11yError> {
    let window = find_active_window(conn, false).await?;
    let window_title = match window.as_accessible_proxy(conn.connection()).await {
        Ok(w) => w.name().await.ok().filter(|s| !s.trim().is_empty()),
        Err(_) => None,
    };

    let (mut collected, truncated) = walk_window(conn, window.clone(), opts).await;

    // Reading order: top-to-bottom, left-to-right.
    collected.sort_by(|a, b| {
        (a.bounds.y.round() as i64, a.bounds.x.round() as i64)
            .cmp(&(b.bounds.y.round() as i64, b.bounds.x.round() as i64))
    });

    state.gen_counter += 1;
    let generation = SnapshotGen(state.gen_counter);
    state.current_gen = generation;
    state.observed_window = Some(window);
    state.registry.clear();

    let mut entries = Vec::with_capacity(collected.len());
    for (i, c) in collected.into_iter().enumerate() {
        let r = i as u32 + 1;
        state.registry.insert(r, RegEntry {
            obj: c.obj,
            bounds: c.bounds,
            role: c.role.clone(),
            name: c.name.clone(),
        });
        entries.push(ElementEntry {
            r#ref: r,
            role: c.role,
            name: c.name,
            value: c.value,
            states: c.states,
            bounds: c.bounds,
            source: Source::A11y,
        });
    }

    let text = format_entries(&entries);
    Ok(Snapshot {
        generation,
        entries,
        text,
        truncated,
        pid: None,
        app_name: None,
        window_title,
    })
}

fn registered_target<'a>(
    target: &Target,
    generation: SnapshotGen,
    state: &'a State,
) -> Result<(u32, &'a RegEntry), A11yError> {
    let r = match target {
        Target::Ref(r) => *r,
        Target::Pixel { .. } => {
            return Err(A11yError::Unsupported {
                capability: "pixel targeting".to_string(),
                hint: "Pixel fallback is handled by the computer tool's input layer.".to_string(),
            });
        }
    };
    if generation != state.current_gen {
        return Err(A11yError::Stale(format!(
            "ref [{r}] is from an older snapshot; re-run observe and use a fresh [ref]"
        )));
    }
    let entry = state
        .registry
        .get(&r)
        .ok_or_else(|| A11yError::NotFound(format!("no element [{r}] in the latest snapshot")))?;
    Ok((r, entry))
}

fn valid_pixel_bounds(bounds: Rect) -> bool {
    // GTK uses G_MININT for offscreen coordinates. Shared pixel_center owns
    // finite/representable geometry and the integer point's containment checks.
    bounds.x > i32::MIN as f64
        && bounds.y > i32::MIN as f64
        && bounds.pixel_center().is_some()
}

async fn require_observed_active_window(
    conn: &AccessibilityConnection,
    observed_window: &ObjectRefOwned,
) -> Result<(), A11yError> {
    let active = find_active_window(conn, true).await.map_err(|e| {
        A11yError::Stale(format!("cannot verify the observed Active window: {e}; observe again"))
    })?;
    if &active != observed_window {
        return Err(A11yError::Stale("the Active window changed after observe; observe again".to_string()));
    }
    Ok(())
}

async fn require_window_ancestry(
    conn: &AccessibilityConnection,
    obj: &ObjectRefOwned,
    observed_window: &ObjectRefOwned,
    r: u32,
) -> Result<(), A11yError> {
    let mut current = obj.clone();
    let mut visited = Vec::new();
    for _ in 0..128 {
        if &current == observed_window {
            return Ok(());
        }
        if current.is_null() || visited.contains(&current) {
            break;
        }
        visited.push(current.clone());
        let acc = current.as_accessible_proxy(conn.connection()).await.map_err(|e| {
            A11yError::Stale(format!("cannot resolve the current parent of [{r}]: {e}; observe again"))
        })?;
        current = acc.parent().await.map_err(|e| {
            A11yError::Stale(format!("cannot read the current parent of [{r}]: {e}; observe again"))
        })?;
    }
    Err(A11yError::Stale(format!("element [{r}] no longer has a verifiable parent chain to the observed window; observe again")))
}

async fn resolve_pixel_target(
    conn: &AccessibilityConnection,
    target: &Target,
    generation: SnapshotGen,
    state: &State,
) -> Result<Rect, A11yError> {
    let (r, entry) = registered_target(target, generation, state)?;
    if detect_caps().synthetic_input != InputKind::X11 {
        return Err(A11yError::Unsupported {
            capability: "pixel gesture target validation".to_string(),
            hint: "Reliable AT-SPI Screen coordinates for pixel input require an X11 session.".to_string(),
        });
    }
    let window = state.observed_window.as_ref().ok_or_else(|| {
        A11yError::Stale("no observed window is retained; observe again".to_string())
    })?;
    require_observed_active_window(conn, window).await?;

    let zconn = conn.connection();
    let acc = entry.obj.as_accessible_proxy(zconn).await.map_err(|e| {
        A11yError::Stale(format!("cannot resolve live element [{r}]: {e}; observe again"))
    })?;
    let states = acc.get_state().await.map_err(|e| {
        A11yError::Stale(format!("cannot read live state of [{r}]: {e}; observe again"))
    })?;
    if states.contains(AtspiState::Defunct)
        || states.contains(AtspiState::Stale)
        || !states.contains(AtspiState::Visible)
        || !states.contains(AtspiState::Showing)
    {
        return Err(A11yError::Stale(format!("element [{r}] is no longer a live visible screen target; observe again")));
    }
    let role = acc.get_role().await.map_err(|e| {
        A11yError::Stale(format!("cannot read live role of [{r}]: {e}; observe again"))
    })?;
    let name = acc.name().await.map_err(|e| {
        A11yError::Stale(format!("cannot read live name of [{r}]: {e}; observe again"))
    })?;
    let name = (!name.trim().is_empty()).then_some(name);
    if role_name(Some(role)) != entry.role || name != entry.name {
        return Err(A11yError::Stale(format!("element [{r}] changed after observe; observe again")));
    }
    require_window_ancestry(conn, &entry.obj, window, r).await?;

    let comp = ComponentProxy::builder(zconn)
        .destination(acc.inner().destination().to_owned())
        .and_then(|b| b.path(acc.inner().path().to_owned()))
        .map_err(|e| A11yError::Stale(format!("cannot build live component for [{r}]: {e}")))?
        .build()
        .await
        .map_err(|e| A11yError::Stale(format!("cannot resolve live component for [{r}]: {e}")))?;
    let (x, y, w, h) = comp.get_extents(CoordType::Screen).await.map_err(|e| {
        A11yError::Stale(format!("cannot read live Screen bounds of [{r}]: {e}; observe again"))
    })?;
    let bounds = Rect { x: x as f64, y: y as f64, w: w as f64, h: h as f64 };
    if !valid_pixel_bounds(bounds) || !valid_pixel_bounds(entry.bounds) || bounds != entry.bounds {
        return Err(A11yError::Stale(format!("element [{r}] has invalid or changed Screen bounds; observe again")));
    }
    // Recheck focus after the remote element reads so a scan earlier in this
    // request cannot authorize coordinates for a subsequently switched window.
    require_observed_active_window(conn, window).await?;
    // The focus scan also awaits remote replies. Reject target movement during
    // that scan and return coordinates from the final live geometry read.
    let (x, y, w, h) = comp.get_extents(CoordType::Screen).await.map_err(|e| {
        A11yError::Stale(format!("cannot recheck live Screen bounds of [{r}]: {e}; observe again"))
    })?;
    let current_bounds = Rect { x: x as f64, y: y as f64, w: w as f64, h: h as f64 };
    if current_bounds != bounds {
        return Err(A11yError::Stale(format!("element [{r}] moved during target validation; observe again")));
    }
    Ok(current_bounds)
}

async fn do_resolve_pixel_target(
    conn: &AccessibilityConnection,
    target: &Target,
    generation: SnapshotGen,
    state: &State,
) -> Result<Rect, A11yError> {
    tokio::time::timeout(
        Duration::from_secs(3),
        resolve_pixel_target(conn, target, generation, state),
    ).await.map_err(|_| A11yError::Stale(
        "AT-SPI pixel target validation timed out; observe again without admitting input".to_string(),
    ))?
}

async fn do_invoke(
    conn: &AccessibilityConnection,
    target: &Target,
    generation: SnapshotGen,
    action: &ElementAction,
    state: &State,
) -> Result<Effect, A11yError> {
    let (r, entry) = registered_target(target, generation, state)?;
    let obj = entry.obj.clone();

    let zconn = conn.connection();
    let acc = obj
        .as_accessible_proxy(zconn)
        .await
        .map_err(|e| A11yError::Backend(format!("cannot resolve [{r}]: {e}")))?;
    let dest = acc.inner().destination().to_owned();
    let path = acc.inner().path().to_owned();

    match action {
        ElementAction::Press | ElementAction::LeftClick | ElementAction::DoubleClick => {
            require_live_interface(&acc, Interface::Action, r).await?;
            let act = ActionProxy::builder(zconn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .destination(dest)
                .and_then(|b| b.path(path))
                .map_err(|e| A11yError::Backend(format!("action proxy: {e}")))?
                .build()
                .await
                .map_err(|e| A11yError::Backend(format!("action proxy: {e}")))?;
            let count = act.n_actions().await.map_err(|e| {
                A11yError::Backend(format!("cannot read AT-SPI action count for [{r}] before action: {e}"))
            })?;
            let count = require_action_count(count, r)?;
            let idx = if matches!(action, ElementAction::Press) {
                // Press explicitly requests the advertised default action.
                0
            } else if matches!(action, ElementAction::LeftClick) {
                read_left_click_action(&act, count, r).await?
            } else {
                // Preserve the existing semantic DoubleClick policy. The
                // computer tool's actual double gesture uses pixel validation.
                let actions = act.get_actions().await.map_err(|e| {
                    A11yError::Backend(format!("cannot read AT-SPI actions for [{r}] before action: {e}"))
                })?;
                actions.iter().position(|a| is_click_action(&a.name)).unwrap_or(0)
            };
            if idx >= count {
                return Err(A11yError::Backend(format!("element [{r}] returned inconsistent AT-SPI action metadata")));
            }
            require_mutating_reply(act.do_action(idx as i32).await, "do_action", r)?;
            Ok(Effect {
                changed: true,
                message: format!("performed action {idx} on element [{r}]"),
            })
        }
        ElementAction::RightClick => Err(A11yError::Unsupported {
            capability: "right click".to_string(),
            hint: "AT-SPI has no standard right-click action; use a pixel right-click.".to_string(),
        }),
        ElementAction::Focus => {
            require_live_interface(&acc, Interface::Component, r).await?;
            let comp = ComponentProxy::builder(zconn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .destination(dest)
                .and_then(|b| b.path(path))
                .map_err(|e| A11yError::Backend(format!("component proxy: {e}")))?
                .build()
                .await
                .map_err(|e| A11yError::Backend(format!("component proxy: {e}")))?;
            require_mutating_reply(comp.grab_focus().await, "grab_focus", r)?;
            Ok(Effect {
                changed: true,
                message: format!("focused element [{r}]"),
            })
        }
        ElementAction::SetValue(v) => {
            require_live_interface(&acc, Interface::EditableText, r).await?;
            let et = EditableTextProxy::builder(zconn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .destination(dest)
                .and_then(|b| b.path(path))
                .map_err(|e| A11yError::Backend(format!("editable-text proxy: {e}")))?
                .build()
                .await
                .map_err(|e| A11yError::Backend(format!("editable-text proxy: {e}")))?;
            require_mutating_reply(et.set_text_contents(v).await, "set_text_contents", r)?;
            Ok(Effect {
                changed: true,
                message: format!("set value of element [{r}]"),
            })
        }
    }
}

fn do_focus(_pid: i32) -> Result<Effect, A11yError> {
    Err(A11yError::Unsupported {
        capability: "window activation".to_string(),
        hint: "Cross-application window activation is not wired on Linux yet (X11 EWMH / Wayland \
               has no portable protocol); the focused window is used by observe."
            .to_string(),
    })
}

// ---- thread + channel plumbing ------------------------------------------

impl ActorHandle {
    pub fn spawn() -> Result<Self, A11yError> {
        let (tx, rx) = channel::<Cmd>();
        let (ready_tx, ready_rx) = channel::<Result<(), A11yError>>();

        std::thread::Builder::new()
            .name("nomi-a11y-linux".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ =
                            ready_tx.send(Err(A11yError::Backend(format!("tokio runtime: {e}"))));
                        return;
                    }
                };
                let conn = match rt.block_on(AccessibilityConnection::new()) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(A11yError::Permission(format!(
                            "cannot connect to the AT-SPI accessibility bus: {e}. Ensure \
                             at-spi2-core is running; KDE needs QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1, \
                             Electron apps need --force-renderer-accessibility."
                        ))));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));

                let mut state = State {
                    gen_counter: 0,
                    current_gen: SnapshotGen(0),
                    observed_window: None,
                    registry: HashMap::new(),
                };

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Capabilities(reply) => {
                            let _ = reply.send(detect_caps());
                        }
                        Cmd::Observe(opts, reply) => {
                            let _ = reply.send(rt.block_on(do_observe(&conn, &opts, &mut state)));
                        }
                        Cmd::ResolvePixelTarget(target, generation, reply) => {
                            let _ = reply.send(rt.block_on(do_resolve_pixel_target(
                                &conn,
                                &target,
                                generation,
                                &state,
                            )));
                        }
                        Cmd::Invoke(target, generation, action, reply) => {
                            let _ = reply.send(rt.block_on(do_invoke(
                                &conn,
                                &target,
                                generation,
                                &action,
                                &state,
                            )));
                        }
                        Cmd::Focus(pid, reply) => {
                            let _ = reply.send(do_focus(pid));
                        }
                    }
                }
            })
            .map_err(|e| A11yError::Backend(format!("failed to start AT-SPI actor thread: {e}")))?;

        ready_rx
            .recv()
            .map_err(|_| A11yError::Backend("AT-SPI actor died at startup".to_string()))??;
        Ok(Self { tx: Mutex::new(tx) })
    }

    fn send(&self, cmd: Cmd) -> Result<(), A11yError> {
        self.tx
            .lock()
            .map_err(|_| A11yError::Backend("AT-SPI actor lock poisoned".to_string()))?
            .send(cmd)
            .map_err(|_| A11yError::Backend("AT-SPI actor thread is gone".to_string()))
    }

    pub fn capabilities(&self) -> Capabilities {
        let (tx, rx) = channel();
        if self.send(Cmd::Capabilities(tx)).is_err() {
            return detect_caps();
        }
        rx.recv().unwrap_or_else(|_| detect_caps())
    }

    pub fn observe(&self, opts: ObserveOpts) -> Result<Snapshot, A11yError> {
        // Never silently substitute an unrelated active window for an explicit PID.
        if opts.pid.is_some() {
            return Err(A11yError::Unsupported {
                capability: "observe by pid".to_string(),
                hint: "The AT-SPI backend currently supports only active-window observation."
                    .to_string(),
            });
        }
        let (tx, rx) = channel();
        self.send(Cmd::Observe(opts, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AT-SPI actor dropped the reply".to_string()))?
    }

    pub fn invoke(
        &self,
        target: Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Invoke(target, generation, action, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Uncertain(
                "AT-SPI actor dropped the reply after accepting the mutating command; the action may have executed. Observe again without retrying or pixel fallback".to_string(),
            ))?
    }

    pub fn resolve_pixel_target(
        &self,
        target: Target,
        generation: SnapshotGen,
    ) -> Result<Rect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::ResolvePixelTarget(target, generation, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AT-SPI actor dropped the reply".to_string()))?
    }

    pub fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Focus(pid, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AT-SPI actor dropped the reply".to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_false_and_transport_errors_are_uncertain() {
        for operation in ["do_action", "grab_focus", "set_text_contents"] {
            assert!(require_mutating_reply::<&str>(Ok(true), operation, 7).is_ok());
            for reply in [Ok(false), Err("synthetic lost reply after dispatch")] {
                let err = require_mutating_reply(reply, operation, 7).unwrap_err();
                assert!(matches!(&err, A11yError::Uncertain(_)));
                assert!(err.to_string().contains(&format!("{operation} on [7]")));
                assert!(err.to_string().contains("without retrying or pixel fallback"));
            }
        }
    }

    #[test]
    fn left_click_selects_only_advertised_click_compatible_machine_names() {
        for accepted in ["click", "press", "activate", " CLICK "] {
            assert_eq!(select_left_click_action(&["delete".into(), accepted.into()]), Some(1));
        }
        for rejected in ["open", "jump", "default", "do default", "confirm", "delete", "点击"] {
            assert_eq!(select_left_click_action(&[rejected.into()]), None);
        }
        assert_eq!(select_left_click_action(&[]), None);
    }

    #[test]
    fn only_confirmed_interface_and_action_absence_are_unsupported() {
        for interface in [Interface::Action, Interface::Component, Interface::EditableText] {
            assert!(require_interface(InterfaceSet::new(interface), interface, 7).is_ok());
            assert!(matches!(require_interface(InterfaceSet::empty(), interface, 7), Err(A11yError::Unsupported { .. })));
        }
        assert_eq!(require_action_count(2, 7).unwrap(), 2);
        assert!(matches!(require_action_count(0, 7), Err(A11yError::Unsupported { .. })));
        assert!(matches!(require_action_count(-1, 7), Err(A11yError::Backend(_))));
    }

    #[test]
    fn accepted_mutating_command_without_actor_reply_is_uncertain() {
        let (tx, rx) = channel();
        let actor = ActorHandle { tx: Mutex::new(tx) };
        // Consume and drop only the in-memory command reply. No actor runtime,
        // D-Bus connection or native accessibility operation is constructed.
        let responder = std::thread::spawn(move || match rx.recv() {
            Ok(Cmd::Invoke(_, _, _, reply)) => drop(reply),
            Ok(_) => panic!("unexpected actor command"),
            Err(_) => panic!("mutating command was not accepted"),
        });
        let result = actor.invoke(Target::Ref(7), SnapshotGen(1), ElementAction::LeftClick);
        assert!(matches!(result, Err(A11yError::Uncertain(_))));
        drop(actor);
        responder.join().unwrap();
    }

    #[test]
    fn mutating_command_rejected_before_actor_admission_is_backend_failure() {
        let (tx, rx) = channel();
        drop(rx);
        let actor = ActorHandle { tx: Mutex::new(tx) };
        assert!(matches!(
            actor.invoke(Target::Ref(7), SnapshotGen(1), ElementAction::LeftClick),
            Err(A11yError::Backend(_))
        ));
    }

    #[test]
    fn explicit_pid_is_rejected_before_sending_to_actor() {
        let (tx, rx) = channel();
        let actor = ActorHandle { tx: Mutex::new(tx) };
        // An in-memory responder keeps the old (incorrectly enqueued) path
        // from hanging, without starting AT-SPI or reading a desktop.
        let responder = std::thread::spawn(move || match rx.recv() {
            Ok(Cmd::Observe(_, reply)) => {
                let _ = reply.send(Err(A11yError::Backend("unexpected observe command".into())));
                true
            }
            Ok(_) => panic!("unexpected actor command"),
            Err(_) => false,
        });
        let result = actor.observe(ObserveOpts { pid: Some(7), ..Default::default() });
        drop(actor);
        let sent = responder.join().unwrap();
        assert!(matches!(result, Err(A11yError::Unsupported { .. })));
        assert!(!sent);
    }
}
