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

use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::action::ActionProxy;
use atspi::proxy::component::ComponentProxy;
use atspi::proxy::editable_text::EditableTextProxy;
use atspi::{CoordType, ObjectRefOwned, State as AtspiState};

use crate::engine::{
    A11yError, Capabilities, Effect, ElementAction, ElementEntry, InputKind, ObserveOpts, Rect,
    Snapshot, SnapshotGen, Source, Target,
};
use crate::tree::format_entries;

enum Cmd {
    Capabilities(Sender<Capabilities>),
    Observe(ObserveOpts, Sender<Result<Snapshot, A11yError>>),
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
    registry: HashMap<u32, ObjectRefOwned>,
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

/// Find the focused (Active) toplevel window by scanning each application's
/// children. Returns its object reference.
async fn find_active_window(conn: &AccessibilityConnection) -> Result<ObjectRefOwned, A11yError> {
    let zconn = conn.connection();
    let root = conn
        .root_accessible_on_registry()
        .await
        .map_err(|e| A11yError::Backend(format!("cannot read the AT-SPI registry root: {e}")))?;
    let apps = root.get_children().await.map_err(|e| {
        A11yError::Backend(format!("cannot list accessible applications: {e}"))
    })?;

    let mut first_window: Option<ObjectRefOwned> = None;
    for app in apps {
        let Ok(app_acc) = app.as_accessible_proxy(zconn).await else {
            continue;
        };
        let Ok(windows) = app_acc.get_children().await else {
            continue;
        };
        for win in windows {
            let Ok(win_acc) = win.as_accessible_proxy(zconn).await else {
                continue;
            };
            let states = win_acc.get_state().await.unwrap_or_default();
            if states.contains(AtspiState::Active) {
                return Ok(win);
            }
            if first_window.is_none() {
                first_window = Some(win);
            }
        }
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
    let window = find_active_window(conn).await?;
    let window_title = match window.as_accessible_proxy(conn.connection()).await {
        Ok(w) => w.name().await.ok().filter(|s| !s.trim().is_empty()),
        Err(_) => None,
    };

    let (mut collected, truncated) = walk_window(conn, window, opts).await;

    // Reading order: top-to-bottom, left-to-right.
    collected.sort_by(|a, b| {
        (a.bounds.y.round() as i64, a.bounds.x.round() as i64)
            .cmp(&(b.bounds.y.round() as i64, b.bounds.x.round() as i64))
    });

    state.gen_counter += 1;
    let generation = SnapshotGen(state.gen_counter);
    state.current_gen = generation;
    state.registry.clear();

    let mut entries = Vec::with_capacity(collected.len());
    for (i, c) in collected.into_iter().enumerate() {
        let r = i as u32 + 1;
        state.registry.insert(r, c.obj);
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
        overlay: None,
        text,
        truncated,
        pid: None,
        app_name: None,
        window_title,
    })
}

async fn do_invoke(
    conn: &AccessibilityConnection,
    target: &Target,
    generation: SnapshotGen,
    action: &ElementAction,
    state: &State,
) -> Result<Effect, A11yError> {
    let r = match target {
        Target::Ref(r) => *r,
        Target::Selector(_) => {
            return Err(A11yError::Unsupported {
                capability: "selector targeting".to_string(),
                hint: "Resolve a selector against the latest observe() result and act by [ref]."
                    .to_string(),
            });
        }
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
    let obj = state
        .registry
        .get(&r)
        .cloned()
        .ok_or_else(|| A11yError::NotFound(format!("no element [{r}] in the latest snapshot")))?;

    let zconn = conn.connection();
    let acc = obj
        .as_accessible_proxy(zconn)
        .await
        .map_err(|e| A11yError::Backend(format!("cannot resolve [{r}]: {e}")))?;
    let dest = acc.inner().destination().to_owned();
    let path = acc.inner().path().to_owned();

    match action {
        ElementAction::Press | ElementAction::LeftClick | ElementAction::DoubleClick => {
            let act = ActionProxy::builder(zconn)
                .destination(dest)
                .and_then(|b| b.path(path))
                .map_err(|e| A11yError::Backend(format!("action proxy: {e}")))?
                .build()
                .await
                .map_err(|e| A11yError::Backend(format!("action proxy: {e}")))?;
            let actions = act.get_actions().await.unwrap_or_default();
            let idx = actions
                .iter()
                .position(|a| is_click_action(&a.name))
                .unwrap_or(0);
            if act.n_actions().await.unwrap_or(0) <= 0 {
                return Err(A11yError::Backend(format!(
                    "element [{r}] exposes no AT-SPI action; fall back to a pixel click"
                )));
            }
            match act.do_action(idx as i32).await {
                Ok(true) => Ok(Effect {
                    changed: true,
                    message: format!("performed action {idx} on element [{r}]"),
                }),
                Ok(false) => Err(A11yError::Backend(format!(
                    "do_action on [{r}] returned false; try a pixel click"
                ))),
                Err(e) => Err(A11yError::Backend(format!("do_action on [{r}] failed: {e}"))),
            }
        }
        ElementAction::RightClick => Err(A11yError::Unsupported {
            capability: "right click".to_string(),
            hint: "AT-SPI has no standard right-click action; use a pixel right-click.".to_string(),
        }),
        ElementAction::Focus => {
            let comp = ComponentProxy::builder(zconn)
                .destination(dest)
                .and_then(|b| b.path(path))
                .map_err(|e| A11yError::Backend(format!("component proxy: {e}")))?
                .build()
                .await
                .map_err(|e| A11yError::Backend(format!("component proxy: {e}")))?;
            match comp.grab_focus().await {
                Ok(_) => Ok(Effect {
                    changed: true,
                    message: format!("focused element [{r}]"),
                }),
                Err(e) => Err(A11yError::Backend(format!("grab_focus on [{r}] failed: {e}"))),
            }
        }
        ElementAction::SetValue(v) => {
            let et = EditableTextProxy::builder(zconn)
                .destination(dest)
                .and_then(|b| b.path(path))
                .map_err(|e| A11yError::Backend(format!("editable-text proxy: {e}")))?
                .build()
                .await
                .map_err(|e| A11yError::Backend(format!("editable-text proxy: {e}")))?;
            match et.set_text_contents(v).await {
                Ok(_) => Ok(Effect {
                    changed: true,
                    message: format!("set value of element [{r}]"),
                }),
                Err(e) => Err(A11yError::Backend(format!(
                    "set_text_contents on [{r}] failed ({e}); fall back to focus + type"
                ))),
            }
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
            .map_err(|_| A11yError::Backend("AT-SPI actor dropped the reply".to_string()))?
    }

    pub fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Focus(pid, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AT-SPI actor dropped the reply".to_string()))?
    }
}
