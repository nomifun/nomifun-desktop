//! Per-tab semantic observation and browser input. No page-synthetic input fallback.

use super::native;
use native::View;
use nomi_browser_engine::{
    input::{KeyChord, parse_key_combo},
    native_semantic,
};
use nomifun_browser_platform::{run_guard::RunAdmissionError, runtime::*};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[path = "semantic_frames.rs"]
mod semantic_frames;

#[derive(serde::Deserialize)]
struct SelectPlan {
    multiple: bool,
    #[cfg(target_os = "macos")]
    menu_list: bool,
    #[cfg(target_os = "macos")]
    typeahead_label: String,
    enabled_indices: Vec<usize>,
    selected_indices: Vec<usize>,
    desired_indices: Vec<usize>,
    next_key: String,
    previous_key: String,
    reset_selection: bool,
}

#[derive(Default, PartialEq, Eq)]
enum AgentFocus {
    #[default]
    Inactive,
    Uncertain,
    Active,
}

pub(super) struct TabAutomation {
    frames: Option<native::frames::FrameSessions>,
    semantic: semantic_frames::SemanticFrames,
    observation: u64,
    observed_target: Option<BrowserTabTarget>,
    pressed: Option<(BrowserMouseButton, u8)>,
    dragging: bool,
    drag_source: Option<semantic_frames::DragSource>,
    point: (f64, f64),
    keys: Vec<KeyChord>,
    agent_focus: AgentFocus,
}

impl Default for TabAutomation {
    fn default() -> Self {
        Self {
            frames: None,
            semantic: Default::default(),
            observation: 0,
            observed_target: None,
            pressed: None,
            dragging: false,
            drag_source: None,
            point: (0.0, 0.0),
            keys: vec![],
            agent_focus: AgentFocus::Inactive,
        }
    }
}

async fn cdp(view: &View, method: &str, params: Value) -> Result<Value, WorkspaceError> {
    native::protocol_call(view, method, params)
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), WorkspaceError> {
    if cancel.is_cancelled() {
        Err(RunAdmissionError::Cancelled.into())
    } else {
        Ok(())
    }
}

fn mouse_mask(button: BrowserMouseButton) -> u8 {
    match button {
        BrowserMouseButton::Left => 1,
        BrowserMouseButton::Right => 2,
        BrowserMouseButton::Middle => 4,
    }
}

// Passive observations of native drag lifecycle. These never construct or
// dispatch a DOM event and never read/replace the browser's DataTransfer.
const PREPARE_DRAG: &str = r#"function(ref) {
    this.__nomiDrag?.cleanup?.();
    const el=this._lastAriaSnapshotForQuery?.elements?.get(ref);
    if (!el?.isConnected) return false;
    const plan={};
    const end=event=>{
        if (!event.isTrusted || event.target!==plan.source) return;
        plan.ended=event;plan.resolve?.();
    };
    const start=event=>{
        if (!event.isTrusted || plan.started || !(el===event.target || el.contains(event.target) || event.target?.contains?.(el))) return;
        plan.started=event;plan.source=event.target;
        plan.source.addEventListener('dragend',end,{once:true,passive:true});
    };
    plan.cleanup=()=>{
        document.removeEventListener('dragstart',start,true);
        plan.source?.removeEventListener('dragend',end);
        plan.resolve?.();
    };
    document.addEventListener('dragstart',start,{capture:true,passive:true});
    this.__nomiDrag=plan;
    return true;
}"#;

const WAIT_DRAG_END: &str = r#"async function() {
    const plan=this.__nomiDrag;
    if (!plan) return {error:'stale'};
    if (plan.started && !plan.ended && !plan.started.defaultPrevented) {
        await new Promise(resolve=>{
            const timer=setTimeout(resolve,1000);
            plan.resolve=()=>{clearTimeout(timer);resolve()};
        });
        plan.resolve=undefined;
    }
    return {started:!!plan.started,ended:!!plan.ended,cancelled:!!plan.started?.defaultPrevented};
}"#;

const CLEAR_DRAG: &str = r#"function() {
    this.__nomiDrag?.cleanup?.(); this.__nomiDrag=undefined; return true;
}"#;

impl TabAutomation {
    pub(super) fn invalidate_observation(&mut self) {
        self.observed_target = None;
    }
    /// Keep the owned page active while the Agent works, even when its native
    /// surface is hidden. This is browser lifecycle emulation, not OS focus or
    /// synthetic DOM input. Restore it at the run boundary, not after each key.
    pub async fn activate_for_agent(&mut self, view: &View, cancel: &CancellationToken) -> Result<(), WorkspaceError> {
        check_cancel(cancel)?;
        if self.agent_focus != AgentFocus::Active {
            // Retain cleanup responsibility even if the protocol result is lost.
            self.agent_focus = AgentFocus::Uncertain;
            if let Some(frames)=self.frames.as_mut() {
                frames.set_file_chooser_interception(true).await.map_err(|_|WorkspaceError::NativeCommandFailed)?;
            }
            cdp(view, "Emulation.setFocusEmulationEnabled", json!({"enabled":true})).await?;
            self.agent_focus = AgentFocus::Active;
        }
        check_cancel(cancel)
    }

    pub async fn settle_agent(&mut self, view: &View) -> Result<(), WorkspaceError> {
        let input = self.release(view).await;
        let chooser=if let Some(frames)=self.frames.as_mut() {
            frames.set_file_chooser_interception(false).await.map_err(|_|WorkspaceError::NativeCommandFailed)
        } else {Ok(())};
        if let Some(sessions) = &self.frames {
            // Presentation is not an input lock. Navigation can destroy its
            // world before settlement; that must not prevent native cleanup.
            let _ = self.semantic.clear_highlight(view, sessions).await;
        }
        let focus = if self.agent_focus != AgentFocus::Inactive {
            let result = cdp(view, "Emulation.setFocusEmulationEnabled", json!({"enabled":false})).await;
            if result.is_ok() { self.agent_focus = AgentFocus::Inactive; }
            result.map(|_| ())
        } else { Ok(()) };
        input.and(chooser).and(focus)
    }

    pub async fn initialize_frames(&mut self, view: &View) -> Result<(), WorkspaceError> {
        if self.frames.is_none() {
            self.frames = Some(
                native::frames::FrameSessions::connect(view)
                    .await
                    .map_err(|_| WorkspaceError::NativeCommandFailed)?,
            );
        }
        Ok(())
    }

    pub async fn configure_file_choosers(&mut self, view: &View, locked: bool) -> Result<(), WorkspaceError> {
        self.initialize_frames(view).await?;
        self.frames.as_mut().ok_or(WorkspaceError::NativeCommandFailed)?
            .set_file_chooser_interception(locked).await.map_err(|_|WorkspaceError::NativeCommandFailed)
    }

    pub(crate) async fn user_file_route(&mut self, view: &View, choice: &native::file_chooser::Choice) -> Result<native::frames::OwnedFrameRoute, String> {
        self.initialize_frames(view).await.map_err(|error|error.to_string())?;
        self.frames.as_mut().ok_or("Browser frame owner is unavailable")?.chooser_route(&choice.frame,&choice.session).await
    }

    async fn selection_state(
        &self,
        view: &View,
        require_focus: bool,
    ) -> Result<Vec<usize>, WorkspaceError> {
        let state = self
            .call(view, native_semantic::SELECT_STATE, vec![])
            .await
            .map_err(|_| WorkspaceError::ActionInterrupted)?;
        if state.get("error").is_some()
            || (require_focus
                && (state["focused"] != true
                    || !self
                        .semantic
                        .ancestors_focused(view, self.frame_sessions()?)
                        .await?))
        {
            return Err(WorkspaceError::ActionInterrupted);
        }
        serde_json::from_value(state["selected_indices"].clone())
            .map_err(|_| WorkspaceError::ActionInterrupted)
    }

    async fn select_options(
        &mut self,
        view: &View,
        plan: SelectPlan,
        cancel: &CancellationToken,
    ) -> Result<(), WorkspaceError> {
        let mut selected = self.selection_state(view, false).await?;
        if selected != plan.selected_indices {
            return Err(WorkspaceError::ActionInterrupted);
        }
        if selected == plan.desired_indices {
            return Ok(());
        }
        // Browser focus avoids opening an OS select popup outside the input gate.
        // Only native key input below changes selection; no JS focus/value mutation.
        check_cancel(cancel)?;
        self.semantic
            .select_node(view, self.frame_sessions()?)
            .await?;
        if self.selection_state(view, true).await? != selected {
            return Err(WorkspaceError::ActionInterrupted);
        }
        #[cfg(target_os = "macos")]
        if plan.menu_list {
            // Cocoa menu-list selects open an OS popup on arrow keys. Use the
            // browser's native type-ahead selection instead, without opening a
            // separate menu or assigning selected/value in page JavaScript.
            return self.select_typeahead(view, &plan, cancel).await;
        }
        if plan.multiple {
            if plan.reset_selection {
                selected = vec![
                    *plan
                        .enabled_indices
                        .first()
                        .ok_or(WorkspaceError::NotActionable)?,
                ];
                self.select_key(view, "Home", &selected, cancel).await?;
            }
            self.select_key(view, "ControlOrMeta+Home", &selected, cancel)
                .await?;
            for (position, index) in plan.enabled_indices.iter().enumerate() {
                if selected.contains(index) != plan.desired_indices.contains(index) {
                    if selected.contains(index) {
                        selected.retain(|value| value != index);
                    } else {
                        selected.push(*index);
                        selected.sort_unstable();
                    }
                    self.select_key(view, "ControlOrMeta+Space", &selected, cancel)
                        .await?;
                }
                if position + 1 < plan.enabled_indices.len() {
                    self.select_key(
                        view,
                        &format!("ControlOrMeta+{}", plan.next_key),
                        &selected,
                        cancel,
                    )
                    .await?;
                }
            }
        } else {
            let wanted = *plan
                .desired_indices
                .first()
                .ok_or(WorkspaceError::NotActionable)?;
            let destination = plan
                .enabled_indices
                .iter()
                .position(|index| *index == wanted)
                .ok_or(WorkspaceError::NotActionable)?;
            let current = selected.first().and_then(|index| {
                plan.enabled_indices
                    .iter()
                    .position(|candidate| candidate == index)
            });
            let mut position = if let Some(position) = current {
                position
            } else {
                let first = *plan
                    .enabled_indices
                    .first()
                    .ok_or(WorkspaceError::NotActionable)?;
                selected = vec![first];
                self.select_key(view, "Home", &selected, cancel).await?;
                0
            };
            while position != destination {
                let key = if position < destination {
                    position += 1;
                    plan.next_key.as_str()
                } else {
                    position -= 1;
                    plan.previous_key.as_str()
                };
                selected = vec![plan.enabled_indices[position]];
                self.select_key(view, key, &selected, cancel).await?;
            }
        }
        if self.selection_state(view, true).await? != plan.desired_indices {
            return Err(WorkspaceError::ActionInterrupted);
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    async fn select_typeahead(&mut self, view: &View, plan: &SelectPlan, cancel: &CancellationToken) -> Result<(), WorkspaceError> {
        if plan.typeahead_label.is_empty() || plan.typeahead_label.chars().count() > 512 { return Err(WorkspaceError::UnsupportedAction); }
        // Blink's type-ahead buffer expires after 1 s. Do not alter focus or
        // call private renderer test hooks to reset an earlier user's buffer.
        tokio::select! { biased;
            _ = cancel.cancelled() => return Err(RunAdmissionError::Cancelled.into()),
            _ = tokio::time::sleep(std::time::Duration::from_millis(1100)) => {}
        }
        if self.selection_state(view, true).await? != plan.selected_indices { return Err(WorkspaceError::ActionInterrupted); }
        for character in plan.typeahead_label.chars() {
            check_cancel(cancel)?;
            self.selection_state(view, true).await?;
            if self.call(view, native_semantic::ARM_SELECT_KEY, vec![json!("keypress")]).await? != true { return Err(WorkspaceError::ActionInterrupted); }
            let text = character.to_string();
            let chord = parse_key_combo(&text).unwrap_or(KeyChord { key: text.clone(), code: String::new(), vk: 0, modifiers: 0 });
            self.keys.push(chord.clone());
            cdp(view, "Input.dispatchKeyEvent", json!({"type":"keyDown","key":text,"code":chord.code,"windowsVirtualKeyCode":chord.vk,"modifiers":0,"text":text,"unmodifiedText":text})).await?;
            cdp(view, "Input.dispatchKeyEvent", json!({"type":"keyUp","key":text,"code":chord.code,"windowsVirtualKeyCode":chord.vk,"modifiers":0})).await?;
            self.keys.pop();
            if self.call(view, native_semantic::SELECT_KEY_ACCEPTED, vec![]).await? != true { return Err(WorkspaceError::ActionInterrupted); }
            if self.selection_state(view, true).await? == plan.desired_indices { return Ok(()); }
        }
        Err(WorkspaceError::ActionInterrupted)
    }

    async fn select_key(
        &mut self,
        view: &View,
        key: &str,
        expected: &[usize],
        cancel: &CancellationToken,
    ) -> Result<(), WorkspaceError> {
        check_cancel(cancel)?;
        if self
            .call(view, native_semantic::ARM_SELECT_KEY, vec![])
            .await?
            != true
        {
            return Err(WorkspaceError::ActionInterrupted);
        }
        check_cancel(cancel)?;
        self.key(view, key)
            .await
            .map_err(|_| WorkspaceError::ActionInterrupted)?;
        if self
            .call(view, native_semantic::SELECT_KEY_ACCEPTED, vec![])
            .await?
            != true
        {
            return Err(WorkspaceError::ActionInterrupted);
        }
        if self.selection_state(view, true).await? != expected {
            return Err(WorkspaceError::ActionInterrupted);
        }
        Ok(())
    }

    fn frame_sessions(&self) -> Result<&native::frames::FrameSessions, WorkspaceError> {
        self.frames
            .as_ref()
            .ok_or(WorkspaceError::NativeCommandFailed)
    }

    async fn call(
        &self,
        view: &View,
        function: &str,
        args: Vec<Value>,
    ) -> Result<Value, WorkspaceError> {
        self.semantic
            .call(view, self.frame_sessions()?, function, args)
            .await
    }

    async fn focused(
        &self,
        view: &View,
        reference: &BrowserElementRef,
    ) -> Result<bool, WorkspaceError> {
        Ok(self
            .call(
                view,
                native_semantic::IS_FOCUSED,
                vec![json!(self.semantic.local_ref(&reference.ref_id)?)],
            )
            .await?
            == true
            && self
                .semantic
                .ancestors_focused(view, self.frame_sessions()?)
                .await?)
    }

    pub async fn observe(
        &mut self,
        view: &View,
        target: BrowserTabTarget,
        cancel: &CancellationToken,
    ) -> Result<BrowserObservation, WorkspaceError> {
        check_cancel(cancel)?;
        self.observed_target = None;
        self.observation += 1;
        self.initialize_frames(view).await?;
        let sessions = self
            .frames
            .as_mut()
            .ok_or(WorkspaceError::NativeCommandFailed)?;
        let trees = sessions
            .trees()
            .await
            .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        let result = self
            .semantic
            .observe(
                view,
                sessions,
                &trees,
                target.clone(),
                self.observation,
                cancel,
            )
            .await?;
        self.observed_target = Some(target);
        Ok(result)
    }

    fn validate(&self, reference: &BrowserElementRef) -> Result<(), WorkspaceError> {
        if self.observed_target.as_ref() != Some(&reference.target)
            || self.observation != reference.observation_generation
        {
            Err(WorkspaceError::StaleObservation)
        } else {
            Ok(())
        }
    }

    async fn locate(
        &self,
        view: &View,
        reference: &BrowserElementRef,
        editable: bool,
    ) -> Result<(f64, f64), WorkspaceError> {
        self.validate(reference)?;
        self.semantic
            .locate(view, self.frame_sessions()?, &reference.ref_id, editable)
            .await
    }

    async fn move_to(
        &mut self,
        view: &View,
        point: (f64, f64),
    ) -> Result<(), WorkspaceError> {
        self.point = point;
        self.semantic
            .highlight(view, self.frame_sessions()?, point)
            .await?;
        cdp(
            view,
            "Input.dispatchMouseEvent",
            json!({"type":"mouseMoved","x":point.0,"y":point.1,
            "button":self.pressed.map(|(button,_)|json!(button)).unwrap_or(json!("none")),
            "buttons":self.pressed.map(|(button,_)|mouse_mask(button)).unwrap_or(0)}),
        )
        .await?;
        Ok(())
    }

    async fn down(
        &mut self,
        view: &View,
        button: BrowserMouseButton,
        click_count: u8,
    ) -> Result<(), WorkspaceError> {
        // Keep the pressed marker even if command completion is uncertain.
        self.pressed = Some((button, click_count));
        cdp(view,"Input.dispatchMouseEvent",json!({"type":"mousePressed","x":self.point.0,"y":self.point.1,"button":button,"buttons":mouse_mask(button),"clickCount":click_count})).await?;
        Ok(())
    }

    async fn up(&mut self, view: &View) -> Result<(), WorkspaceError> {
        let Some((button, click_count)) = self.pressed else {
            return Ok(());
        };
        cdp(view,"Input.dispatchMouseEvent",json!({"type":"mouseReleased","x":self.point.0,"y":self.point.1,"button":button,"buttons":0,"clickCount":click_count})).await?;
        self.pressed = None;
        Ok(())
    }

    async fn key(&mut self, view: &View, keys: &str) -> Result<(), WorkspaceError> {
        let chord = parse_key_combo(keys).map_err(|_| WorkspaceError::UnsupportedAction)?;
        let text = nomi_browser_engine::input::chord_text(&chord).unwrap_or_default();
        self.keys.push(chord.clone());
        let params = json!({"type":"keyDown","key":chord.key,"code":chord.code,"windowsVirtualKeyCode":chord.vk,"modifiers":chord.modifiers,"text":text});
        #[cfg(target_os = "macos")]
        let params = {
            let mut params = params;
            let commands = nomi_browser_engine::input::mac_editing_commands(&chord.code, chord.modifiers);
            if !commands.is_empty() { params["commands"] = json!(commands); }
            params
        };
        cdp(view,"Input.dispatchKeyEvent",params).await?;
        cdp(view,"Input.dispatchKeyEvent",json!({"type":"keyUp","key":chord.key,"code":chord.code,"windowsVirtualKeyCode":chord.vk,"modifiers":chord.modifiers})).await?;
        self.keys.pop();
        Ok(())
    }

    pub async fn release(&mut self, view: &View) -> Result<(), WorkspaceError> {
        // Run admission and terminal cleanup both call release. Refs from a
        // prior turn cannot become valid merely because the document survived.
        self.observed_target = None;
        if self.dragging {
            // Releasing a button over a valid drop target can commit a native
            // HTML drag. Cancel that drag first on failure/Stop, then release
            // the tracked button. Keep the marker if cancellation is uncertain.
            cdp(view, "Input.cancelDragging", json!({})).await?;
            if let Some(source) = &self.drag_source {
                source.cancel(view, self.frame_sessions()?).await?;
            }
            self.dragging = false;
            self.drag_source = None;
        }
        if self.pressed.is_some() {
            self.up(view).await?;
        }
        while let Some(key) = self.keys.last() {
            cdp(view,"Input.dispatchKeyEvent",json!({"type":"keyUp","key":key.key,"code":key.code,"windowsVirtualKeyCode":key.vk,"modifiers":key.modifiers})).await?;
            self.keys.pop();
        }
        if self.semantic.has_worlds() {
            // Navigation may already have destroyed the isolated world.
            let _ = self.call(view, native_semantic::CLEAR_SELECT, vec![]).await;
            let _ = self.call(view, CLEAR_DRAG, vec![]).await;
        }
        Ok(())
    }

    pub async fn upload(&mut self,view:&View,element:BrowserElementRef,files:&nomifun_browser_platform::uploads::PreparedBrowserUpload,cancel:&CancellationToken)->Result<(),WorkspaceError> {
        check_cancel(cancel)?;
        self.semantic.activate(&element.ref_id)?;
        let point=self.locate(view,&element,false).await?;
        self.move_to(view,point).await?;
        check_cancel(cancel)?;
        let result=async { if self.semantic.is_file_input(view,self.frame_sessions()?,&element.ref_id).await? {
            self.semantic.upload_files(view,self.frame_sessions()?,&element.ref_id,files,cancel).await
        } else {
            let mut chooser=native::file_chooser::FileChooser::listen(view).await?;
            self.frames.as_mut().ok_or(WorkspaceError::StaleObservation)?.set_file_chooser_interception(true).await.map_err(|_|WorkspaceError::NativeCommandFailed)?;
            chooser.arm();
            async {
                check_cancel(cancel)?;
                self.down(view,BrowserMouseButton::Left,1).await?;
                self.up(view).await?;
                check_cancel(cancel)?;
                let choice=chooser.next(cancel).await?;
                check_cancel(cancel)?;
                self.semantic.upload_choice(view,self.frame_sessions()?,choice,files,cancel).await
            }.await
        }}.await;
        self.observed_target=None;
        let cleanup=self.release(view).await;
        result?;cleanup?;check_cancel(cancel)
    }

    pub async fn act(
        &mut self,
        view: &View,
        action: BrowserAction,
        cancel: &CancellationToken,
    ) -> Result<(), WorkspaceError> {
        check_cancel(cancel)?;
        match &action {
            BrowserAction::Click { click_count, .. } if !(1..=2).contains(click_count) => {
                return Err(WorkspaceError::UnsupportedAction);
            }
            BrowserAction::Type { text, .. } if text.len() > 65536 => {
                return Err(WorkspaceError::UnsupportedAction);
            }
            BrowserAction::Select { labels, .. }
                if labels.len() > 512 || labels.iter().any(|label| label.len() > 512) =>
            {
                return Err(WorkspaceError::UnsupportedAction);
            }
            BrowserAction::Press { keys, .. } => {
                parse_key_combo(keys).map_err(|_| WorkspaceError::UnsupportedAction)?;
            }
            BrowserAction::Scroll {
                delta_x, delta_y, ..
            } if !delta_x.is_finite()
                || !delta_y.is_finite()
                || delta_x.abs() > 10000.0
                || delta_y.abs() > 10000.0 =>
            {
                return Err(WorkspaceError::UnsupportedAction);
            }
            _ => {}
        }
        self.semantic.activate(&action.element().ref_id)?;
        let point = self
            .locate(
                view,
                action.element(),
                matches!(action, BrowserAction::Type { .. }),
            )
            .await?;
        if let BrowserAction::Press { element, keys } = &action {
            // Pressing Enter on a button must not first click it and submit twice.
            if self
                .call(
                    view,
                    native_semantic::IS_FOCUSED,
                    vec![json!(self.semantic.local_ref(&element.ref_id)?)],
                )
                .await?
                != true
                || !self
                    .semantic
                    .ancestors_focused(view, self.frame_sessions()?)
                    .await?
            {
                return Err(WorkspaceError::NotActionable);
            }
            check_cancel(cancel)?;
            let result = self.key(view, keys).await;
            let cleanup = self.release(view).await;
            return result.and(cleanup);
        }
        let destination = if let BrowserAction::Drag { to, .. } = &action {
            if !self.semantic.supports_drag(&action.element().ref_id, &to.ref_id)? {
                return Err(WorkspaceError::UnsupportedAction);
            }
            Some(self.locate(view, to, false).await?)
        } else {
            None
        };
        let selection = if let BrowserAction::Select { element, labels } = &action {
            let plan = self
                .call(
                    view,
                    native_semantic::PREPARE_SELECT,
                    vec![
                        json!(self.semantic.local_ref(&element.ref_id)?),
                        json!(labels),
                    ],
                )
                .await?;
            match plan["error"].as_str() {
                Some("unsupported") => return Err(WorkspaceError::UnsupportedAction),
                Some("stale") => return Err(WorkspaceError::StaleObservation),
                Some(_) => return Err(WorkspaceError::NotActionable),
                None => Some(
                    serde_json::from_value::<SelectPlan>(plan)
                        .map_err(|_| WorkspaceError::NativeCommandFailed)?,
                ),
            }
        } else {
            None
        };
        // The result consumes this observation even on input failure. No automatic
        // retry can replay a click whose browser callback may already have run.
        let result = async {
            check_cancel(cancel)?;
            self.move_to(view,point).await?;
            check_cancel(cancel)?;
            // Hover handlers can move or replace controls before the first
            // pressed event. Revalidate the exact element and every iframe.
            let current=self.locate(view,action.element(),matches!(action,BrowserAction::Type {..})).await?;
            if (current.0-point.0).abs()>2.0 || (current.1-point.1).abs()>2.0 {
                return Err(WorkspaceError::ActionInterrupted);
            }
            check_cancel(cancel)?;
            match action {
                BrowserAction::Select { element, .. } => {
                    self.locate(view,&element,false).await?;
                    self.select_options(view,selection.ok_or(WorkspaceError::NotActionable)?,cancel).await?;
                }
                BrowserAction::Click { element, button, click_count } => {
                    for count in 1..=click_count {
                        check_cancel(cancel)?;
                        if count > 1 {
                            // First-click handlers may navigate, replace the element, or
                            // put another control at these coordinates. Do not send the
                            // second click into a different page/target.
                            let current=self.locate(view,&element,false).await
                                .map_err(|_|WorkspaceError::ActionInterrupted)?;
                            if (current.0-point.0).abs()>2.0 || (current.1-point.1).abs()>2.0 {
                                return Err(WorkspaceError::ActionInterrupted);
                            }
                            check_cancel(cancel)?;
                        }
                        self.down(view,button,count).await?;
                        self.up(view).await?;
                    }
                }
                BrowserAction::Hover { .. } => {},
                BrowserAction::Scroll { delta_x, delta_y, .. } => {
                    if !delta_x.is_finite() || !delta_y.is_finite() || delta_x.abs()>10000.0 || delta_y.abs()>10000.0 { return Err(WorkspaceError::UnsupportedAction); }
                    cdp(view,"Input.dispatchMouseEvent",json!({"type":"mouseWheel","x":point.0,"y":point.1,"deltaX":delta_x,"deltaY":delta_y})).await?;
                }
                BrowserAction::Drag { from, to: reference } => {
                    let to = destination.ok_or(WorkspaceError::NotActionable)?;
                    if self.call(view,PREPARE_DRAG,vec![json!(self.semantic.local_ref(&from.ref_id)?)]).await? != true {
                        return Err(WorkspaceError::StaleObservation);
                    }
                    check_cancel(cancel)?;
                    let source = self.semantic.drag_source(view,self.frame_sessions()?,&from.ref_id).await?;
                    let current = self.locate(view,&from,false).await?;
                    if (current.0-point.0).abs()>2.0 || (current.1-point.1).abs()>2.0 {
                        return Err(WorkspaceError::NotActionable);
                    }
                    check_cancel(cancel)?;
                    self.drag_source = Some(source);
                    self.dragging = true;
                    self.down(view,BrowserMouseButton::Left,1).await?;
                    for step in 1..=8 {
                        tokio::select! {
                            _=cancel.cancelled()=>return Err(RunAdmissionError::Cancelled.into()),
                            _=tokio::time::sleep(std::time::Duration::from_millis(16))=>{},
                        }
                        check_cancel(cancel)?;
                        let t=step as f64/8.0;
                        self.move_to(view,(point.0+(to.0-point.0)*t,point.1+(to.1-point.1)*t)).await?;
                    }
                    // dragover handlers may replace or move the destination.
                    // Do not release over a different control at the old point.
                    let current=self.locate(view,&reference,false).await
                        .map_err(|_|WorkspaceError::ActionInterrupted)?;
                    if (current.0-to.0).abs()>2.0 || (current.1-to.1).abs()>2.0 {
                        return Err(WorkspaceError::ActionInterrupted);
                    }
                    check_cancel(cancel)?;
                    self.up(view).await?;
                    let lifecycle=self.call(view,WAIT_DRAG_END,vec![]).await
                        .map_err(|_|WorkspaceError::ActionInterrupted)?;
                    if lifecycle.get("error").is_some() || lifecycle["cancelled"]==true
                        || (lifecycle["started"]==true && lifecycle["ended"]!=true) {
                        return Err(WorkspaceError::ActionInterrupted);
                    }
                    self.dragging = false;
                    self.drag_source = None;
                }
                action => {
                    self.down(view,BrowserMouseButton::Left,1).await?; self.up(view).await?;
                    check_cancel(cancel)?;
                    match action {
                        BrowserAction::Type { element, text } => {
                            if !self.focused(view,&element).await.map_err(|_|WorkspaceError::ActionInterrupted)? {
                                return Err(WorkspaceError::ActionInterrupted);
                            }
                            self.key(view,"ControlOrMeta+A").await?;
                            // A key handler can redirect focus or navigate before
                            // the following text insertion reaches the browser.
                            if !self.focused(view,&element).await.map_err(|_|WorkspaceError::ActionInterrupted)? {
                                return Err(WorkspaceError::ActionInterrupted);
                            }
                            check_cancel(cancel)?;
                            if text.is_empty() { self.key(view,"Backspace").await?; }
                            else { cdp(view,"Input.insertText",json!({"text":text})).await?; }
                        }
                        _ => {},
                    }
                }
            }
            Ok(())
        }.await;
        self.observed_target = None;
        let cleanup = self.release(view).await;
        result.and(cleanup)
    }
}
