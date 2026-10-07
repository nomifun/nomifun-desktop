//! Only explicitly granted tabs enter this driver. Semantic JavaScript is
//! fixed application code; all mouse/key/text/wheel effects use CDP input.
use super::{AttachedBrowser, Connection, GrantedTab, ROOT_SESSION};
use crate::{input, native_semantic};
use chromiumoxide::cdp::{
    browser_protocol::{
        input::{
            DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams,
            DispatchMouseEventType, MouseButton,
        },
        page::{BringToFrontParams, CreateIsolatedWorldParams, GetFrameTreeParams, NavigateParams},
        target::{AttachToTargetParams, DetachFromTargetParams},
    },
    js_protocol::runtime::{
        CallArgument, CallFunctionOnParams, EvaluateParams, ExecutionContextId,
        ReleaseObjectGroupParams, RemoteObjectId,
    },
};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_browser_platform::attached_browser::{
    AttachedBrowserCommand as Command, AttachedBrowserRuntimeError as Error,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

mod frame_documents;
mod frame_routes;
mod pending;
mod rendering;
mod script_dialogs;
pub(super) use pending::Pending;

#[derive(Default)]
pub(super) struct TabAutomation {
    session: String,
    frame: String,
    loader: String,
    object: String,
    context: i64,
    observation: String,
    refs: BTreeSet<String>,
    group: String,
    pressed: Option<input::Point>,
    keys: Vec<(input::KeyChord, u32)>,
    frames: frame_documents::Documents,
    dialogs: Option<Arc<script_dialogs::Dialogs>>,
    rendering: Option<rendering::Rendering>,
}

struct ReturnState {
    owner: Arc<Mutex<super::AttachedState>>,
    target: String,
    value: Option<TabAutomation>,
}
impl Drop for ReturnState {
    fn drop(&mut self) {
        let mut state = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .connection
            .as_ref()
            .is_some_and(|conn| !conn.registry().is_connection_closed())
        {
            if let Some(value) = self.value.take() {
                state.automation.insert(self.target.clone(), value);
            }
        }
    }
}

fn check(cancel: &CancellationToken, connection: &Connection) -> Result<(), Error> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if connection.registry().is_connection_closed() {
        return Err(Error::Disconnected);
    }
    Ok(())
}

async fn prepare<T>(
    conn: &Connection,
    cancel: &CancellationToken,
    work: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    if cancel.is_cancelled(){return Err(Error::Cancelled);}
    struct Preparation<'a> {
        conn: &'a Connection,
        finished: bool,
    }
    impl Drop for Preparation<'_> {
        fn drop(&mut self) {
            if !self.finished {
                self.conn.registry().fail_connection();
            }
        }
    }
    let mut guard = Preparation {
        conn,
        finished: false,
    };
    tokio::pin!(work);
    let result = tokio::select! {biased;
        _=cancel.cancelled()=>match tokio::time::timeout(std::time::Duration::from_millis(250),&mut work).await {
            Ok(result)=>result,
            Err(_)=>{conn.registry().fail_connection();Err(Error::Cancelled)},
        },
        result=&mut work=>result,
    };
    guard.finished = true;
    result
}

impl TabAutomation {
    pub(super) fn retire_dialogs(&self) -> Option<BoxFuture<'static, ()>> {
        self.dialogs.clone().map(|dialogs| {
            async move {
                let _ = dialogs.shutdown().await;
            }
            .boxed()
        })
    }
}

async fn call(
    conn: &Connection,
    state: &TabAutomation,
    script: &str,
    args: Vec<Value>,
) -> Result<Value, Error> {
    if state.frames.has_active_child()
        && script != native_semantic::HIGHLIGHT
        && script != native_semantic::CLEAR_HIGHLIGHT
    {
        return state.frames.call_active(conn, script, args).await;
    }
    let guarded = format!(
        "function(...args) {{ if (location.protocol !== 'http:' && location.protocol !== 'https:' && location.href !== 'about:blank') throw new Error('Unsupported document'); return ({script}).apply(this,args); }}"
    );
    let mut params = CallFunctionOnParams::new(guarded);
    params.object_id = Some(RemoteObjectId::new(state.object.clone()));
    params.arguments = Some(
        args.into_iter()
            .map(|value| CallArgument {
                value: Some(value),
                ..Default::default()
            })
            .collect(),
    );
    params.return_by_value = Some(true);
    params.await_promise = Some(true);
    let result = conn
        .send(&state.session, &params)
        .await
        .map_err(|_| Error::InvalidInput)?;
    if result.get("exceptionDetails").is_some() {
        return Err(Error::InvalidInput);
    }
    result
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or(Error::InvalidInput)
}

fn observation_id(command: &Command) -> Option<&str> {
    match command {
        Command::Click { observation_id, .. }
        | Command::Type { observation_id, .. }
        | Command::Press { observation_id, .. }
        | Command::Scroll { observation_id, .. } => Some(observation_id),
        _ => None,
    }
}

fn validate_input(command: &Command) -> Result<(), Error> {
    if observation_id(command).is_some_and(|id| id.is_empty() || id.len() > 128) {
        return Err(Error::InvalidInput);
    }
    match command {
        Command::Dialog {
            dialog_id,
            prompt_text,
            ..
        } if dialog_id.is_empty()
            || dialog_id.chars().count() > 128
            || prompt_text
                .as_ref()
                .is_some_and(|text| text.chars().count() > 4096) =>
        {
            Err(Error::InvalidInput)
        }
        Command::Click { ref_id, .. } | Command::Type { ref_id, .. }
            if ref_id.is_empty() || ref_id.len() > 128 =>
        {
            Err(Error::InvalidInput)
        }
        Command::Type { text, .. } if text.chars().count() > 16384 => Err(Error::InvalidInput),
        Command::Press { keys, .. } => {
            if keys.len() > 128 {
                return Err(Error::InvalidInput);
            }
            let chord = input::parse_key_combo(keys).map_err(|_| Error::InvalidInput)?;
            let key = chord.key.to_ascii_lowercase();
            // Page keyboard/editing only; no browser/window/system accelerators.
            if key.strip_prefix('f').is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
            }) || chord.modifiers & 5 != 0
                || (chord.modifiers & 2 != 0
                    && !matches!(
                        key.as_str(),
                        "a" | "c"
                            | "v"
                            | "x"
                            | "z"
                            | "y"
                            | "home"
                            | "end"
                            | "arrowleft"
                            | "arrowright"
                            | "backspace"
                            | "delete"
                    ))
            {
                return Err(Error::InvalidInput);
            }
            Ok(())
        }
        Command::Scroll {
            delta_x, delta_y, ..
        } if !delta_x.is_finite()
            || !delta_y.is_finite()
            || delta_x.abs() > 10000.0
            || delta_y.abs() > 10000.0 =>
        {
            Err(Error::InvalidInput)
        }
        Command::Navigate { url, .. } => {
            if url.len() > 8192 {
                return Err(Error::InvalidInput);
            }
            let url = url::Url::parse(url).map_err(|_| Error::InvalidInput)?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(Error::InvalidInput);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

impl AttachedBrowser {
    pub fn automation_digest() -> String {
        use sha2::Digest;
        let mut digest = sha2::Sha256::new();
        for source in [
            include_str!("../attached_browser.rs"),
            include_str!("tabs.rs"),
            include_str!("automation.rs"),
            include_str!("automation/frame_documents.rs"),
            include_str!("automation/frame_routes.rs"),
            include_str!("automation/script_dialogs.rs"),
            include_str!("automation/pending.rs"),
            include_str!("../transport.rs"),
            include_str!("../session.rs"),
            include_str!("../frame_geometry.rs"),
            crate::injected::INJECTED_SOURCE,
            include_str!("../native_semantic.rs"),
            include_str!("../native_stability.js"),
            include_str!("../input.rs"),
        ] {
            digest.update(source.as_bytes());
            digest.update([0]);
        }
        format!("{:x}", digest.finalize())
    }

    /// Caller retains the operation future until completion even on Stop.
    /// Cancellation is checked between atomic inputs; no write is replayed.
    pub async fn execute_granted(
        &self,
        grant: &GrantedTab,
        command: Command,
        cancel: &CancellationToken,
    ) -> Result<Value, Error> {
        if grant.incarnation != self.incarnation {
            return Err(Error::TabDenied);
        }
        let requested_tab = match &command {
            Command::Tabs {} => return Err(Error::InvalidInput),
            Command::Observe { tab_id }
            | Command::Navigate { tab_id, .. }
            | Command::Click { tab_id, .. }
            | Command::Type { tab_id, .. }
            | Command::Press { tab_id, .. }
            | Command::Dialog { tab_id, .. }
            | Command::Scroll { tab_id, .. } => tab_id,
        };
        if requested_tab != grant.id() {
            return Err(Error::TabDenied);
        }
        validate_input(&command)?;
        let prior = self.pending(&grant.target_id);
        if let Command::Dialog {
            dialog_id,
            accept,
            prompt_text,
            ..
        } = command
        {
            check(
                cancel,
                &self.current_connection().map_err(|_| Error::Disconnected)?,
            )?;
            let dialogs = prior
                .as_ref()
                .map(|work| work.dialogs.clone())
                .or_else(|| {
                    self.state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .automation
                        .get(&grant.target_id)
                        .and_then(|state| state.dialogs.clone())
                })
                .ok_or(Error::InvalidInput)?;
            let mut updates = dialogs.updates();
            let mut reply = Box::pin(dialogs.reply(&dialog_id, accept, prompt_text));
            loop {
                tokio::select! {
                    biased;
                    result=&mut reply=>{result?;break;},
                    result=updates.changed()=>{
                        result.map_err(|_|Error::Disconnected)?;
                        if let Some(next)=dialogs.snapshot() {
                            if next["dialog_id"]!=dialog_id {return Ok(pending::waiting(grant,&next,prior.is_some()));}
                        }
                    },
                }
            }
            if let Some(work) = prior {
                return self.wait_pending(grant, work).await;
            }
            let _operation = self.operations.lock().await;
            if let Some(state) = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .automation
                .get_mut(&grant.target_id)
            {
                state.observation.clear();
                state.refs.clear();
            }
            return Ok(
                json!({"tab_id":grant.id(),"completed":true,"observe_before_acting":true,"script_dialog":dialogs.snapshot()}),
            );
        }
        if let Some(work) = prior {
            if matches!(command, Command::Observe { .. }) {
                return self.wait_pending(grant, work).await;
            }
            return Err(Error::Busy);
        }
        let operation = self
            .operations
            .clone()
            .try_lock_owned()
            .map_err(|_| Error::Busy)?;
        let conn = self.current_connection().map_err(|_| Error::Disconnected)?;
        check(cancel, &conn)?;
        prepare(&conn, cancel, async {
            super::tabs::target_info(&conn, &grant.target_id)
                .await
                .map_err(|_| Error::TabDenied)
        })
        .await?;
        let mut saved = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.automation.contains_key(&grant.target_id) && state.automation.len() >= 32 {
                return Err(Error::Busy);
            }
            ReturnState {
                owner: self.state.clone(),
                target: grant.target_id.clone(),
                value: Some(
                    state
                        .automation
                        .remove(&grant.target_id)
                        .unwrap_or_default(),
                ),
            }
        };
        let state = saved.value.as_mut().unwrap();
        if state.pressed.is_some() || !state.keys.is_empty() {
            return Err(Error::ExecutionFailed);
        }
        if let Some(expected) = observation_id(&command) {
            if expected != state.observation || state.observation.is_empty() {
                return Err(Error::InvalidInput);
            }
        }
        if let Command::Click { ref_id, .. } | Command::Type { ref_id, .. } = &command {
            if !state.refs.contains(ref_id) {
                return Err(Error::InvalidInput);
            }
            state.frames.activate(ref_id);
        }
        if state.session.is_empty() {
            let mut params = AttachToTargetParams::new(grant.target_id.clone());
            params.flatten = Some(true);
            let response = match prepare(&conn, cancel, async {
                conn.send(ROOT_SESSION, &params)
                    .await
                    .map_err(|_| Error::ExecutionFailed)
            })
            .await
            {
                Ok(value) => value,
                Err(error) => {
                    if error!=Error::Cancelled {self.request_disconnect();}
                    return Err(error);
                }
            };
            state.session = response["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or(Error::ExecutionFailed)?
                .into();
            // attach events are normally registered by the transport before the
            // reply; this idempotent registration covers peers that omit it.
            conn.registry()
                .register_session(state.session.clone(), "page");
        }
        if state.dialogs.is_none() {
            state.dialogs = Some(
                prepare(
                    &conn,
                    cancel,
                    script_dialogs::Dialogs::install(&conn, &state.session),
                )
                .await?,
            );
        }
        let dialogs = state.dialogs.as_ref().unwrap().clone();
        check(cancel,&conn)?;
        if let Some(dialog) = dialogs.snapshot() {
            return Ok(pending::waiting(grant, &dialog, false));
        }
        let active = !matches!(command, Command::Observe { .. });
        let owned_cancel = cancel.clone();
        let owned_grant = grant.clone();
        let rendering_cleanup = conn.clone();
        let paused = conn.with_response_pause(dialogs.response_pause());
        let work = async move {
            let _operation = operation;
            let state = saved.value.as_mut().unwrap();
            let conn = paused;
            let cancel = &owned_cancel;
            let grant = &owned_grant;
            let tree = conn
                .send(&state.session, &GetFrameTreeParams::default())
                .await
                .map_err(|_| Error::ExecutionFailed)?;
            let frame = tree["frameTree"]["frame"]["id"]
                .as_str()
                .ok_or(Error::ExecutionFailed)?;
            let loader = tree["frameTree"]["frame"]["loaderId"]
                .as_str()
                .ok_or(Error::ExecutionFailed)?;
            require_page_url(&tree)?;
            let current_document =
                state.frame == frame && state.loader == loader && !state.object.is_empty();
            if let Some(expected) = observation_id(&command) {
                if expected.is_empty() || expected != state.observation || !current_document {
                    return Err(Error::InvalidInput);
                }
                state.frames.validate(&conn).await?;
            }
            check(cancel, &conn)?;
            if active && state.rendering.is_none() {
                state.rendering = Some(rendering::Rendering::begin(&conn, &state.session, rendering_cleanup).await?);
            }
            check(cancel, &conn)?;
            conn.send(&state.session, &BringToFrontParams::default())
                .await
                .map_err(|_| Error::ExecutionFailed)?;
            if let Command::Navigate { url, .. } = &command {
                let url = url::Url::parse(url).map_err(|_| Error::InvalidInput)?;
                if !matches!(url.scheme(), "http" | "https")
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(Error::InvalidInput);
                }
                state.observation.clear();
                state.refs.clear();
                state.frames.clear(&conn).await?;
                if current_document {
                    call(&conn, state, native_semantic::CLEAR_HIGHLIGHT, vec![])
                        .await
                        .map_err(|_| Error::ExecutionFailed)?;
                }
                check(cancel, &conn)?;
                let result = conn
                    .send(&state.session, &NavigateParams::new(url.to_string()))
                    .await
                    .map_err(|_| Error::ExecutionFailed)?;
                if result.get("errorText").is_some() {
                    return Err(Error::ExecutionFailed);
                }
                return Ok(
                    json!({"tab_id":grant.id,"navigated":true,"observe_before_acting":true}),
                );
            }
            if matches!(command, Command::Observe { .. }) {
                state.observation.clear();
                state.refs.clear();
                state.frames.reset_active();
                if !current_document {
                    if !state.object.is_empty() {
                        clear_instrumentation(&conn, state).await?;
                    }
                    if !state.group.is_empty() {
                        conn.send(
                            &state.session,
                            &ReleaseObjectGroupParams::new(state.group.clone()),
                        )
                        .await
                        .map_err(|_| Error::ExecutionFailed)?;
                        state.group.clear();
                        state.object.clear();
                    }
                    let mut world = CreateIsolatedWorldParams::new(frame.to_owned());
                    world.world_name = Some("nomifun-attached-browser-semantic".into());
                    world.grant_univeral_access = Some(false);
                    let world = conn
                        .send(&state.session, &world)
                        .await
                        .map_err(|_| Error::ExecutionFailed)?;
                    let context = world["executionContextId"]
                        .as_i64()
                        .ok_or(Error::ExecutionFailed)?;
                    state.group = format!("nomi-attached-observe-{}", nomifun_common::generate_id());
                    let mut evaluate = EvaluateParams::new(format!(
                        "if (location.protocol !== 'http:' && location.protocol !== 'https:' && location.href !== 'about:blank') throw new Error('Unsupported document');\n{}",
                        native_semantic::initialization_expression()
                    ));
                    evaluate.context_id = Some(ExecutionContextId::new(context));
                    evaluate.object_group = Some(state.group.clone());
                    let value = conn
                        .send(&state.session, &evaluate)
                        .await
                        .map_err(|_| Error::ExecutionFailed)?;
                    if value.get("exceptionDetails").is_some() {
                        return Err(Error::ExecutionFailed);
                    }
                    state.object = value["result"]["objectId"]
                        .as_str()
                        .ok_or(Error::ExecutionFailed)?
                        .into();
                    state.context = context;
                    state.frame = frame.into();
                    state.loader = loader.into();
                }
                let value = call(&conn, state, native_semantic::OBSERVE, vec![]).await?;
                let content = value["content"]
                    .as_str()
                    .filter(|s| s.len() <= 200_000)
                    .ok_or(Error::ExecutionFailed)?;
                let elements = value["elements"]
                    .as_array()
                    .filter(|s| s.len() <= 2000)
                    .ok_or(Error::ExecutionFailed)?;
                for element in elements {
                    let id = element["ref_id"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 128)
                        .ok_or(Error::ExecutionFailed)?;
                    state.refs.insert(id.into());
                }
                let secret_refs=call(&conn,state,"function(){return Array.from(this._lastAriaSnapshotForQuery?.elements||[]).filter(([,el])=>el.tagName==='INPUT'&&(el.type==='password'||(el.getAttribute('autocomplete')||'').toLowerCase().includes('password'))||el.tagName==='TEXTAREA'&&(el.getAttribute('autocomplete')||'').toLowerCase().includes('password')).map(([ref])=>String(ref));}",vec![]).await?;
                let secret_refs: Vec<String> =
                    serde_json::from_value(secret_refs).map_err(|_| Error::ExecutionFailed)?;
                let mut content = crate::redact::wrap_untrusted(
                    &crate::redact::redact_yaml(&crate::redact::blank_all_editable_values(
                        &crate::redact::blank_secret_values(content, &secret_refs),
                    )),
                    None,
                );
                let mut elements = elements
                    .iter()
                    .map(|element| {
                        let mut element = element.clone();
                        element["name"] = json!(crate::redact::redact_yaml(
                            element["name"].as_str().unwrap_or("")
                        ));
                        element
                    })
                    .collect::<Vec<_>>();
                let root = frame_documents::Document::root(state);
                let unobserved = state
                    .frames
                    .observe(&conn, root, &mut content, &mut elements, cancel)
                    .await?;
                for element in &elements {
                    state.refs.insert(
                        element["ref_id"]
                            .as_str()
                            .ok_or(Error::InvalidInput)?
                            .into(),
                    );
                }
                check(cancel, &conn)?;
                let current = conn
                    .send(&state.session, &GetFrameTreeParams::default())
                    .await
                    .map_err(|_| Error::ExecutionFailed)?;
                if current["frameTree"]["frame"]["id"] != state.frame
                    || current["frameTree"]["frame"]["loaderId"] != state.loader
                {
                    return Err(Error::InvalidInput);
                }
                state.observation = nomifun_common::generate_id();
                return Ok(
                    json!({"tab_id":grant.id,"observation_id":state.observation,"content":content,"elements":elements,
                "coverage":"page_frames","unobserved_child_frames":unobserved,"untrusted_page_content":true}),
                );
            }
            check(cancel, &conn)?;
            match &command {
                Command::Click { ref_id, .. } | Command::Type { ref_id, .. }
                    if !state.refs.contains(ref_id) =>
                {
                    return Err(Error::InvalidInput);
                }
                _ => {}
            }
            match command {
                Command::Click { ref_id, .. } => {
                    click(&conn, state, &ref_id, false, cancel).await?
                }
                Command::Type {
                    ref_id,
                    text,
                    replace,
                    ..
                } => {
                    click(&conn, state, &ref_id, true, cancel).await?;
                    if call(
                        &conn,
                        state,
                        native_semantic::IS_FOCUSED,
                        vec![json!(ref_id)],
                    )
                    .await?
                        != true
                    {
                        return Err(Error::InvalidInput);
                    }
                    check(cancel, &conn)?;
                    if replace {
                        key_combo(&conn, state, "Ctrl+A").await?;
                    }
                    check(cancel, &conn)?;
                    require_document(&conn, state).await?;
                    if call(
                        &conn,
                        state,
                        native_semantic::IS_FOCUSED,
                        vec![json!(ref_id)],
                    )
                    .await?
                        != true
                    {
                        return Err(Error::InvalidInput);
                    }
                    check(cancel, &conn)?;
                    if replace && text.is_empty() {
                        key_combo(&conn, state, "Backspace").await?;
                    } else {
                        input::insert_text(&conn, &state.session, &text)
                            .await
                            .map_err(|_| Error::ExecutionFailed)?;
                    }
                }
                Command::Press { keys, .. } => {
                    if keys.len() > 128 || input::parse_key_combo(&keys).is_err() {
                        return Err(Error::InvalidInput);
                    }
                    if !state.frames.ancestors_focused(&conn).await? {
                        return Err(Error::InvalidInput);
                    }
                    check(cancel, &conn)?;
                    key_combo(&conn, state, &keys).await?;
                }
                Command::Scroll {
                    delta_x, delta_y, ..
                } => {
                    if !delta_x.is_finite()
                        || !delta_y.is_finite()
                        || delta_x.abs() > 10000.0
                        || delta_y.abs() > 10000.0
                    {
                        return Err(Error::InvalidInput);
                    }
                    let viewport = call(
                        &conn,
                        state,
                        "function(){return {width:innerWidth,height:innerHeight}}",
                        vec![],
                    )
                    .await?;
                    let (x, y) = if state.frames.has_active_child() {
                        state.frames.scroll_point(&conn).await?
                    } else {
                        (
                            viewport["width"].as_f64().ok_or(Error::ExecutionFailed)? / 2.0,
                            viewport["height"].as_f64().ok_or(Error::ExecutionFailed)? / 2.0,
                        )
                    };
                    let mut params =
                        DispatchMouseEventParams::new(DispatchMouseEventType::MouseWheel, x, y);
                    params.delta_x = Some(delta_x);
                    params.delta_y = Some(delta_y);
                    check(cancel, &conn)?;
                    input::dispatch_mouse_move(&conn, &state.session, input::Point { x, y })
                        .await
                        .map_err(|_| Error::ExecutionFailed)?;
                    check(cancel, &conn)?;
                    require_document(&conn, state).await?;
                    if state.frames.has_active_child()
                        && state.frames.scroll_point(&conn).await? != (x, y)
                    {
                        return Err(Error::InvalidInput);
                    }
                    check(cancel, &conn)?;
                    conn.send(&state.session, &params)
                        .await
                        .map_err(|_| Error::ExecutionFailed)?;
                }
                _ => return Err(Error::InvalidInput),
            }
            check(cancel, &conn)?;
            Ok(
                json!({"tab_id":grant.id,"completed":true,"input_fidelity":"browser_protocol","untrusted_page_content":true}),
            )
        };
        let pending = pending::spawn(work, dialogs, conn, cancel.clone(), active);
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .insert(grant.target_id.clone(), pending.clone());
        self.wait_pending(grant, pending).await
    }

    /// Release only this granted tab's instrumentation/session, never the page.
    pub async fn release_granted(&self, grant: &GrantedTab) -> Result<(), Error> {
        if grant.incarnation != self.incarnation {
            return Err(Error::TabDenied);
        }
        if let Some(pending) = self.pending(&grant.target_id) {
            pending.cancel.cancel();
            let _ = pending.job.clone().await;
            self.forget_pending(&grant.target_id, &pending.id);
        }
        let dialogs = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .automation
            .get(&grant.target_id)
            .and_then(|state| state.dialogs.clone());
        if let Some(dialogs) = dialogs {
            if let Some(dialog) = dialogs.snapshot() {
                if dialog["owned"] != true
                    || dialogs
                        .dismiss_or_join_reply(
                            dialog["dialog_id"].as_str().ok_or(Error::InvalidInput)?,
                        )
                        .await
                        .is_err()
                {
                    self.request_disconnect();
                }
            }
        }
        let _operation = self.operations.lock().await;
        let conn = match self.current_connection() {
            Ok(connection) => connection,
            Err(_) => {
                drop(_operation);
                self.disconnect()
                    .await
                    .map_err(|_| Error::ExecutionFailed)?;
                return Ok(());
            }
        };
        let state = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .automation
            .remove(&grant.target_id);
        let mut saved = ReturnState {
            owner: self.state.clone(),
            target: grant.target_id.clone(),
            value: state,
        };
        if let Some(state) = saved.value.as_mut() {
            release_inputs(&conn, state).await?;
            state.frames.clear(&conn).await?;
            if !state.object.is_empty() {
                clear_instrumentation(&conn, state).await?;
            }
            if !state.group.is_empty() {
                conn.send(
                    &state.session,
                    &ReleaseObjectGroupParams::new(state.group.clone()),
                )
                .await
                .map_err(|_| Error::ExecutionFailed)?;
                state.group.clear();
                state.object.clear();
            }
            if let Some(routes) = state.frames.routes.as_mut() {
                routes.disable(&conn).await?;
            }
            if let Some(dialogs) = state.dialogs.as_ref() {
                dialogs.shutdown().await?;
            }
            if let Some(rendering) = state.rendering.take() {
                rendering.finish().await?;
            }
            if !state.session.is_empty() {
                let params = DetachFromTargetParams::builder()
                    .session_id(state.session.clone())
                    .build();
                if conn.send(ROOT_SESSION, &params).await.is_err() {
                    self.request_disconnect();
                    return Err(Error::ExecutionFailed);
                }
            }
        }
        saved.value = None;
        Ok(())
    }
}

async fn click(
    conn: &Connection,
    state: &mut TabAutomation,
    reference: &str,
    editable: bool,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let point = call(
        conn,
        state,
        native_semantic::LOCATE,
        vec![json!(reference), json!(editable)],
    )
    .await?;
    let (x, y) = (
        point["x"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or(Error::InvalidInput)?,
        point["y"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or(Error::InvalidInput)?,
    );
    let _ = call(
        conn,
        state,
        native_semantic::HIGHLIGHT,
        vec![json!(x), json!(y)],
    )
    .await;
    check(cancel, conn)?;
    input::dispatch_mouse_move(conn, &state.session, input::Point { x, y })
        .await
        .map_err(|_| Error::ExecutionFailed)?;
    check(cancel, conn)?;
    require_document(conn, state).await?;
    let after_move = call(
        conn,
        state,
        native_semantic::LOCATE,
        vec![json!(reference), json!(editable)],
    )
    .await?;
    if after_move["x"].as_f64() != Some(x) || after_move["y"].as_f64() != Some(y) {
        return Err(Error::InvalidInput);
    }
    check(cancel, conn)?;
    // Once pressed, always submit release before returning or observing Stop.
    let mut down = DispatchMouseEventParams::new(DispatchMouseEventType::MousePressed, x, y);
    down.button = Some(MouseButton::Left);
    down.buttons = Some(1);
    down.click_count = Some(1);
    state.pressed = Some(input::Point { x, y });
    let pressed = conn.send(&state.session, &down).await;
    let mut up = DispatchMouseEventParams::new(DispatchMouseEventType::MouseReleased, x, y);
    up.button = Some(MouseButton::Left);
    up.buttons = Some(0);
    up.click_count = Some(1);
    let released = conn.send(&state.session, &up).await;
    if released.is_ok() {
        state.pressed = None;
    }
    if pressed.is_err() || released.is_err() {
        return Err(Error::ExecutionFailed);
    }
    check(cancel, conn)
}

fn require_page_url(tree: &Value) -> Result<(), Error> {
    let address = tree["frameTree"]["frame"]["url"]
        .as_str()
        .ok_or(Error::TabDenied)?;
    if address.is_empty() {
        return Err(Error::ExecutionFailed);
    }
    if address == "about:blank" {
        return Ok(());
    }
    let url = url::Url::parse(address).map_err(|_| Error::TabDenied)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::TabDenied);
    }
    Ok(())
}
async fn clear_instrumentation(conn: &Connection, state: &TabAutomation) -> Result<(), Error> {
    if call(conn, state, native_semantic::CLEAR_HIGHLIGHT, vec![]).await == Ok(json!(true)) {
        return Ok(());
    }
    let current = conn
        .send(&state.session, &GetFrameTreeParams::default())
        .await
        .map_err(|_| Error::ExecutionFailed)?;
    let frame = current["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or(Error::ExecutionFailed)?;
    let loader = current["frameTree"]["frame"]["loaderId"]
        .as_str()
        .ok_or(Error::ExecutionFailed)?;
    if frame == state.frame && loader == state.loader {
        return Err(Error::ExecutionFailed);
    }
    Ok(())
}
async fn require_document(conn: &Connection, state: &mut TabAutomation) -> Result<(), Error> {
    let tree = conn
        .send(&state.session, &GetFrameTreeParams::default())
        .await
        .map_err(|_| Error::InvalidInput)?;
    require_page_url(&tree)?;
    if tree["frameTree"]["frame"]["id"] != state.frame
        || tree["frameTree"]["frame"]["loaderId"] != state.loader
    {
        return Err(Error::InvalidInput);
    }
    state.frames.validate(conn).await?;
    Ok(())
}
async fn dispatch_key(
    conn: &Connection,
    session: &str,
    chord: &input::KeyChord,
    kind: DispatchKeyEventType,
    text: Option<String>,
) -> Result<(), Error> {
    let mut params = DispatchKeyEventParams::new(kind);
    params.modifiers = Some(chord.modifiers.into());
    params.key = Some(chord.key.clone());
    params.code = Some(chord.code.clone());
    params.windows_virtual_key_code = Some(chord.vk);
    params.native_virtual_key_code = Some(chord.vk);
    params.text = text;
    conn.send(session, &params)
        .await
        .map_err(|_| Error::ExecutionFailed)?;
    Ok(())
}
async fn release_inputs(conn: &Connection, state: &mut TabAutomation) -> Result<(), Error> {
    if let Some(point) = state.pressed {
        let mut up =
            DispatchMouseEventParams::new(DispatchMouseEventType::MouseReleased, point.x, point.y);
        up.button = Some(MouseButton::Left);
        up.buttons = Some(0);
        up.click_count = Some(1);
        conn.send(&state.session, &up)
            .await
            .map_err(|_| Error::ExecutionFailed)?;
        state.pressed = None;
    }
    while let Some((held, bit)) = state.keys.last() {
        let mut chord = held.clone();
        chord.modifiers = state.keys.iter().fold(0, |mask, (_, bit)| mask | bit) & !bit;
        dispatch_key(
            conn,
            &state.session,
            &chord,
            DispatchKeyEventType::KeyUp,
            None,
        )
        .await?;
        state.keys.pop();
    }
    Ok(())
}
async fn key_combo(conn: &Connection, state: &mut TabAutomation, keys: &str) -> Result<(), Error> {
    let chord = input::parse_key_combo(keys).map_err(|_| Error::InvalidInput)?;
    let result = async {
        let mut mask = 0;
        for (bit, key, code, vk) in [
            (2, "Control", "ControlLeft", 17),
            (1, "Alt", "AltLeft", 18),
            (8, "Shift", "ShiftLeft", 16),
            (4, "Meta", "MetaLeft", 91),
        ] {
            if chord.modifiers & bit == 0 {
                continue;
            }
            mask |= bit;
            let modifier = input::KeyChord {
                modifiers: mask,
                key: key.into(),
                code: code.into(),
                vk,
            };
            state.keys.push((modifier.clone(), bit));
            dispatch_key(
                conn,
                &state.session,
                &modifier,
                DispatchKeyEventType::KeyDown,
                None,
            )
            .await?;
        }
        state.keys.push((chord.clone(), 0));
        dispatch_key(
            conn,
            &state.session,
            &chord,
            DispatchKeyEventType::KeyDown,
            input::chord_text(&chord),
        )
        .await
    }
    .await;
    let released = release_inputs(conn, state).await;
    result.and(released)
}

#[cfg(test)]
mod tests;
