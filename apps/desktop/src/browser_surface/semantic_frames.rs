//! Per-document semantic worlds and frame-local to root-viewport mapping.
//! No input is synthesized here; the Tab driver sends native input at the result.

use super::{cdp, check_cancel, windows};
use nomi_browser_engine::{native_semantic, redact};
use nomifun_browser_platform::runtime::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;
use windows::frames::{FrameSession, FrameSessions, FrameTrees};
use nomi_browser_engine::frame_geometry::{ContentQuad, CHECK_FRAME_OWNER, CHECK_FRAME_HIT};

#[derive(Clone)]
struct FramePlan {
    id: String,
    parent: Option<usize>,
    session: Option<FrameSession>,
}

fn plans(trees: &FrameTrees) -> Result<Vec<FramePlan>, WorkspaceError> {
    let root = trees.root["frame"]["id"]
        .as_str()
        .ok_or(WorkspaceError::NativeCommandFailed)?;
    let sessions: BTreeMap<_, _> = trees
        .children
        .iter()
        .map(|(session, _)| (session.frame_id.as_str(), session))
        .collect();
    let mut parents = BTreeMap::<String, Option<String>>::new();
    let mut pending = vec![(&trees.root, None)];
    pending.extend(trees.children.iter().map(|(_, tree)| (tree, None)));
    let mut visits = 0;
    while let Some((tree, inherited_parent)) = pending.pop() {
        visits += 1;
        if visits > 256 {
            return Err(WorkspaceError::ObservationLimit);
        }
        let id = tree["frame"]["id"]
            .as_str()
            .ok_or(WorkspaceError::NativeCommandFailed)?;
        let parent = tree["frame"]["parentId"]
            .as_str()
            .or(inherited_parent)
            .map(str::to_owned);
        if let Some(existing) = parents.get(id) {
            if existing.is_some() && parent.is_some() && existing != &parent {
                return Err(WorkspaceError::StaleObservation);
            }
        }
        if parent.is_some() || !parents.contains_key(id) {
            parents.insert(id.into(), parent);
        }
        if let Some(children) = tree["childFrames"].as_array() {
            pending.extend(children.iter().map(|child| (child, Some(id))));
        }
    }
    let mut result = vec![FramePlan {
        id: root.into(),
        parent: None,
        session: None,
    }];
    let mut depths = vec![0];
    // Parent-first and bounded. Disconnected, cyclic and over-depth trees remain
    // explicitly unobserved instead of being attached to a guessed parent.
    let mut index = 0;
    while index < result.len() && result.len() < 64 {
        if depths[index] < 8 {
            for (id, parent) in &parents {
                if parent.as_deref() != Some(result[index].id.as_str())
                    || result.iter().any(|plan| &plan.id == id)
                {
                    continue;
                }
                let session = sessions
                    .get(id.as_str())
                    .map(|session| (*session).clone())
                    .or_else(|| result[index].session.clone());
                result.push(FramePlan {
                    id: id.clone(),
                    parent: Some(index),
                    session,
                });
                depths.push(depths[index] + 1);
                if result.len() == 64 {
                    break;
                }
            }
        }
        index += 1;
    }
    Ok(result)
}

struct World {
    plan: FramePlan,
    context: i64,
    object: String,
    group: String,
    // A remote iframe element in the parent's isolated world, not a selector.
    owner: Option<String>,
}

pub(super) struct DragSource {
    session: Option<FrameSession>,
    point: (f64, f64),
}

impl DragSource {
    pub(super) async fn cancel(&self, view: &tauri::Webview, sessions: &FrameSessions) -> Result<(), WorkspaceError> {
        // The root Input drag controller tracks the current drop target, not
        // necessarily the source widget. Explicitly cancel the owned source
        // session too. Empty payload carries no data; this is never a drop or a
        // substitute for successful native dragend with the negotiated effect.
        command(view, sessions, self.session.as_ref(), "Input.dispatchDragEvent",
            json!({"type":"dragCancel","x":self.point.0,"y":self.point.1,"data":{"items":[],"dragOperationsMask":0}})).await?;
        Ok(())
    }
}

fn same_drag_session(source: Option<&FrameSession>, target: Option<&FrameSession>) -> bool {
    source == target
}

#[derive(Default)]
pub(super) struct SemanticFrames {
    worlds: Vec<World>,
    refs: BTreeMap<String, (usize, String)>,
    active: usize,
}

async fn command(
    view: &tauri::Webview,
    sessions: &FrameSessions,
    session: Option<&FrameSession>,
    method: &str,
    params: Value,
) -> Result<Value, WorkspaceError> {
    if let Some(session) = session {
        sessions
            .command(session, method, params)
            .await
            .map_err(|_| WorkspaceError::StaleObservation)
    } else {
        cdp(view, method, params).await
    }
}

impl SemanticFrames {
    /// Chromium's virtual input currently completes drag source state on the
    /// final target RenderWidgetHost. Crossing an OOPIF boundary can therefore
    /// deliver a real target drop while leaving the real source without
    /// dragend(move). Reject before mouseDown instead of producing that partial
    /// side effect. Root/same-process frames share None; one OOPIF session is
    /// also safe when both endpoints belong to that exact session.
    pub(super) fn supports_drag(&self, from: &str, to: &str) -> Result<bool, WorkspaceError> {
        let source = self.refs.get(from).map(|(index, _)| *index).ok_or(WorkspaceError::StaleObservation)?;
        let target = self.refs.get(to).map(|(index, _)| *index).ok_or(WorkspaceError::StaleObservation)?;
        Ok(same_drag_session(
            self.worlds[source].plan.session.as_ref(),
            self.worlds[target].plan.session.as_ref(),
        ))
    }
    async fn content_quad(&self,view:&tauri::Webview,sessions:&FrameSessions,index:usize)->Result<ContentQuad,WorkspaceError> {
        let world=&self.worlds[index];
        let parent=world.plan.parent.ok_or(WorkspaceError::NotActionable)?;
        let owner=world.owner.as_ref().ok_or(WorkspaceError::StaleObservation)?;
        let model=command(view,sessions,self.worlds[parent].plan.session.as_ref(),"DOM.getBoxModel",json!({"objectId":owner})).await?;
        ContentQuad::read(&model["model"]["content"])
    }
    async fn viewport(&self,view:&tauri::Webview,sessions:&FrameSessions,index:usize)->Result<(f64,f64),WorkspaceError> {
        let value=self.in_world(view,sessions,index,"function(){return {width:innerWidth,height:innerHeight};}",vec![],true).await?;
        Ok((value["width"].as_f64().ok_or(WorkspaceError::NotActionable)?,value["height"].as_f64().ok_or(WorkspaceError::NotActionable)?))
    }
    fn is_session_root(&self,index:usize)->bool {
        let plan=&self.worlds[index].plan;
        plan.parent.is_none() || plan.session.as_ref().is_some_and(|session|session.frame_id==plan.id)
    }
    pub(super) fn activate(&mut self, reference: &str) -> Result<(), WorkspaceError> {
        self.active = self
            .refs
            .get(reference)
            .ok_or(WorkspaceError::StaleObservation)?
            .0;
        Ok(())
    }

    pub(super) fn local_ref(&self, reference: &str) -> Result<&str, WorkspaceError> {
        self.refs
            .get(reference)
            .map(|(_, local)| local.as_str())
            .ok_or(WorkspaceError::StaleObservation)
    }

    pub(super) fn has_worlds(&self) -> bool {
        !self.worlds.is_empty()
    }

    async fn in_world(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
        index: usize,
        function: &str,
        arguments: Vec<Value>,
        by_value: bool,
    ) -> Result<Value, WorkspaceError> {
        let world = self
            .worlds
            .get(index)
            .ok_or(WorkspaceError::StaleObservation)?;
        let result = command(
            view,
            sessions,
            world.plan.session.as_ref(),
            "Runtime.callFunctionOn",
            json!({
                "objectId":world.object,"functionDeclaration":function,"arguments":arguments,
                "objectGroup":world.group,"returnByValue":by_value,"awaitPromise":true
            }),
        )
        .await
        .map_err(|_| WorkspaceError::StaleObservation)?;
        if result.get("exceptionDetails").is_some() {
            return Err(WorkspaceError::StaleObservation);
        }
        Ok(if by_value {
            result["result"]["value"].clone()
        } else {
            result
        })
    }

    pub(super) async fn call(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
        function: &str,
        args: Vec<Value>,
    ) -> Result<Value, WorkspaceError> {
        self.in_world(
            view,
            sessions,
            self.active,
            function,
            args.into_iter()
                .map(|value| json!({"value":value}))
                .collect(),
            true,
        )
        .await
    }

    pub(super) async fn select_node(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
    ) -> Result<(), WorkspaceError> {
        let node = self
            .in_world(
                view,
                sessions,
                self.active,
                native_semantic::SELECT_NODE,
                vec![],
                false,
            )
            .await?;
        let object = node["result"]["objectId"]
            .as_str()
            .ok_or(WorkspaceError::StaleObservation)?;
        command(
            view,
            sessions,
            self.worlds[self.active].plan.session.as_ref(),
            "DOM.focus",
            json!({"objectId":object}),
        )
        .await?;
        Ok(())
    }

    pub(super) async fn is_file_input(&self,view:&tauri::Webview,sessions:&FrameSessions,reference:&str)->Result<bool,WorkspaceError> {
        Ok(self.call(view,sessions,"function(ref){const el=this._lastAriaSnapshotForQuery?.elements?.get(ref);return el instanceof HTMLInputElement && el.type==='file';}",vec![json!(self.local_ref(reference)?)]).await?==true)
    }
    pub(super) async fn upload_choice(&self,view:&tauri::Webview,sessions:&FrameSessions,choice:windows::file_chooser::Choice,files:&nomifun_browser_platform::uploads::PreparedBrowserUpload,cancel:&CancellationToken)->Result<(),WorkspaceError> {
        check_cancel(cancel)?;
        choice.require_current()?;
        // The clicked control may delegate to another already-observed frame
        // in this page. The chooser must still resolve in that exact document's
        // isolated world; navigation or an unobserved target cannot gain files.
        let index=self.worlds.iter().position(|world|world.plan.id==choice.frame &&
            world.plan.session.as_ref().map_or(choice.session.is_empty(),|session|session.matches_protocol_session(&choice.session)))
            .ok_or(WorkspaceError::ActionInterrupted)?;
        let world=&self.worlds[index];
        if !choice.multiple && files.file_count()>1 {return Err(WorkspaceError::ActionInterrupted);}
        let resolved=command(view,sessions,world.plan.session.as_ref(),"DOM.resolveNode",json!({"backendNodeId":choice.backend_node,"executionContextId":world.context,"objectGroup":world.group})).await?;
        let object=resolved["object"]["objectId"].as_str().ok_or(WorkspaceError::ActionInterrupted)?;
        let valid=self.in_world(view,sessions,index,
            "function(el,count){return el instanceof HTMLInputElement && el.ownerDocument===document && el.type==='file' && !el.webkitdirectory && (el.multiple||count<2);}",
            vec![json!({"objectId":object}),json!({"value":files.file_count()})],true).await?;
        if valid!=true {return Err(WorkspaceError::ActionInterrupted);}
        check_cancel(cancel)?;
        choice.require_current()?;
        command(view,sessions,world.plan.session.as_ref(),"DOM.setFileInputFiles",json!({"objectId":object,"files":files.native_paths()?})).await.map_err(|_|WorkspaceError::ActionInterrupted)?;
        choice.require_current()
    }

    pub(super) async fn upload_files(&self,view:&tauri::Webview,sessions:&FrameSessions,reference:&str,files:&nomifun_browser_platform::uploads::PreparedBrowserUpload,cancel:&CancellationToken)->Result<(),WorkspaceError> {
        check_cancel(cancel)?;
        let paths=files.native_paths()?;
        let node=self.in_world(view,sessions,self.active,
            "function(ref,count){const el=this._lastAriaSnapshotForQuery?.elements?.get(ref);if(!(el instanceof HTMLInputElement)||el.type!=='file'||!el.isConnected||el.disabled||el.webkitdirectory||(!el.multiple&&count>1))return null;return el;}",
            vec![json!({"value":self.local_ref(reference)?}),json!({"value":paths.len()})],false).await?;
        let object=node["result"]["objectId"].as_str().ok_or(WorkspaceError::NotActionable)?;
        check_cancel(cancel)?;
        command(view,sessions,self.worlds[self.active].plan.session.as_ref(),"DOM.setFileInputFiles",json!({"objectId":object,"files":paths})).await.map_err(|error|{
            tracing::debug!(%error,"Native file input protocol command failed");
            WorkspaceError::ActionInterrupted
        })?;
        // The page may consume the files and reset/remove this input in its
        // change handler, just as with a real chooser. Do not require unchanged
        // application DOM state after input; the Agent observes the result.
        Ok(())
    }

    pub(super) async fn highlight(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
        point: (f64, f64),
    ) -> Result<(), WorkspaceError> {
        self.in_world(
            view,
            sessions,
            0,
            native_semantic::HIGHLIGHT,
            vec![json!({"value":point.0}), json!({"value":point.1})],
            true,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn ancestors_focused(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
    ) -> Result<bool, WorkspaceError> {
        let mut index = self.active;
        while let Some(parent) = self.worlds[index].plan.parent {
            let owner = self.worlds[index]
                .owner
                .as_ref()
                .ok_or(WorkspaceError::StaleObservation)?;
            if self.in_world(view, sessions, parent, "function(owner) { return owner.isConnected && document.activeElement === owner; }",
                vec![json!({"objectId":owner})], true).await? != true { return Ok(false) }
            index = parent;
        }
        Ok(true)
    }

    pub(super) async fn clear_highlight(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
    ) -> Result<(), WorkspaceError> {
        if self.worlds.is_empty() { return Ok(()); }
        self.in_world(view, sessions, 0, native_semantic::CLEAR_HIGHLIGHT, vec![], true).await?;
        Ok(())
    }

    pub(super) async fn observe(
        &mut self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
        trees: &FrameTrees,
        target: BrowserTabTarget,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<BrowserObservation, WorkspaceError> {
        self.refs.clear();
        self.active = 0;
        // Retire presentation before releasing the only remote reference to it.
        // Navigation may already have destroyed this document and its overlay.
        let _ = self.clear_highlight(view, sessions).await;
        for world in self.worlds.drain(..) {
            // Old documents may already be gone. Release every retained group,
            // including parent-side iframe handles, before making fresh refs.
            let _ = command(
                view,
                sessions,
                world.plan.session.as_ref(),
                "Runtime.releaseObjectGroup",
                json!({"objectGroup":world.group}),
            )
            .await;
        }
        let plans = plans(trees)?;
        let mut worlds_by_plan = BTreeMap::new();
        let mut elements = vec![];
        let mut contents = vec![];
        let mut content_size = 0;
        for (plan_index, mut plan) in plans.into_iter().enumerate() {
            check_cancel(cancel)?;
            if let Some(parent) = plan.parent {
                let Some(actual_parent) = worlds_by_plan.get(&parent).copied() else {
                    continue;
                };
                plan.parent = Some(actual_parent);
            }
            let group = format!("nomi-frame-{}", uuid::Uuid::now_v7());
            // A stable world name avoids accumulating isolated contexts on each
            // observe; remote object groups, refs and injected objects are fresh.
            let created = command(
                view,
                sessions,
                plan.session.as_ref(),
                "Page.createIsolatedWorld",
                json!({"frameId":plan.id,"worldName":"nomifun-browser-semantic"}),
            )
            .await;
            let context = match created
                .ok()
                .and_then(|value| value["executionContextId"].as_i64())
            {
                Some(context) => context,
                None if plan_index > 0 => continue,
                None => return Err(WorkspaceError::NativeCommandFailed),
            };
            // Retain the group before evaluation: even a failed/uncertain
            // callback must leave its remote handles eligible for cleanup.
            let index = self.worlds.len();
            self.worlds.push(World {
                plan,
                context,
                object: String::new(),
                group,
                owner: None,
            });
            let initialized = command(
                view,
                sessions,
                self.worlds[index].plan.session.as_ref(),
                "Runtime.evaluate",
                json!({
                    "expression":native_semantic::initialization_expression(),"contextId":context,
                    "objectGroup":self.worlds[index].group,"returnByValue":false
                }),
            )
            .await?;
            self.worlds[index].object = initialized["result"]["objectId"]
                .as_str()
                .ok_or(WorkspaceError::StaleObservation)?
                .to_owned();
            if let Some(parent) = self.worlds[index].plan.parent {
                let owner = command(
                    view,
                    sessions,
                    self.worlds[parent].plan.session.as_ref(),
                    "DOM.getFrameOwner",
                    json!({"frameId":self.worlds[index].plan.id}),
                )
                .await?;
                let owner = command(view, sessions, self.worlds[parent].plan.session.as_ref(), "DOM.resolveNode", json!({
                    "backendNodeId":owner["backendNodeId"],"executionContextId":self.worlds[parent].context,
                    "objectGroup":self.worlds[parent].group
                })).await?;
                self.worlds[index].owner = Some(
                    owner["object"]["objectId"]
                        .as_str()
                        .ok_or(WorkspaceError::StaleObservation)?
                        .to_owned(),
                );
            }
            let snapshot = self
                .in_world(
                    view,
                    sessions,
                    index,
                    native_semantic::OBSERVE,
                    vec![],
                    true,
                )
                .await?;
            if snapshot["error"] == "limit" {
                return Err(WorkspaceError::ObservationLimit);
            }
            let content = snapshot["content"]
                .as_str()
                .ok_or(WorkspaceError::StaleObservation)?;
            content_size += content.len();
            let raw = snapshot["elements"]
                .as_array()
                .ok_or(WorkspaceError::StaleObservation)?;
            if content_size > 200_000 || elements.len() + raw.len() > 2000 {
                return Err(WorkspaceError::ObservationLimit);
            }
            // Prefix refs in both the structured result and the aria text. Raw
            // engine IDs and protocol frame IDs never become model route inputs.
            let mut content = content.to_owned();
            for element in raw {
                let local = element["ref_id"]
                    .as_str()
                    .ok_or(WorkspaceError::StaleObservation)?;
                let reference = format!("f{index}:{local}");
                content = content.replace(&format!("[ref={local}]"), &format!("[ref={reference}]"));
                self.refs.insert(reference.clone(), (index, local.into()));
                elements.push(BrowserElement {
                    reference: BrowserElementRef {
                        target: target.clone(),
                        observation_generation: generation,
                        ref_id: reference,
                    },
                    role: element["role"].as_str().unwrap_or("").into(),
                    name: redact::redact_yaml(element["name"].as_str().unwrap_or("")),
                    focused: element["focused"].as_bool().unwrap_or(false),
                });
            }
            contents.push(format!("Frame f{index}:\n{content}"));
            worlds_by_plan.insert(plan_index, index);
        }
        check_cancel(cancel)?;
        let unobserved_frames = trees
            .descendant_count()
            .map_err(|_| WorkspaceError::ObservationLimit)?
            .saturating_sub(self.worlds.len().saturating_sub(1));
        let content = redact::wrap_untrusted(
            &redact::redact_yaml(&redact::blank_all_editable_values(&contents.join("\n"))),
            None,
        );
        Ok(BrowserObservation {
            target,
            observation_generation: generation,
            content,
            elements,
            script_dialog: None,
            unobserved_frames,
        })
    }

    pub(super) async fn locate(
        &self,
        view: &tauri::Webview,
        sessions: &FrameSessions,
        reference: &str,
        editable: bool,
    ) -> Result<(f64, f64), WorkspaceError> {
        Ok(self.locate_native(view, sessions, reference, editable, false).await?.point)
    }

    pub(super) async fn drag_source(&self, view: &tauri::Webview, sessions: &FrameSessions, reference: &str) -> Result<DragSource, WorkspaceError> {
        self.locate_native(view, sessions, reference, false, true).await
    }

    async fn locate_native(&self, view: &tauri::Webview, sessions: &FrameSessions, reference: &str, editable: bool, source_scope: bool) -> Result<DragSource, WorkspaceError> {
        let (mut index, local) = self
            .refs
            .get(reference)
            .cloned()
            .ok_or(WorkspaceError::StaleObservation)?;
        let located = self
            .in_world(
                view,
                sessions,
                index,
                native_semantic::LOCATE,
                vec![json!({"value":local}), json!({"value":editable})],
                true,
            )
            .await?;
        let mut point = read_point(&located)?;
        let session = self.worlds[index].plan.session.clone();
        while let Some(parent) = self.worlds[index].plan.parent {
            // Source cleanup addresses its own native widget: transform through
            // same-process ancestors only, stopping at the OOPIF session root.
            if source_scope && self.worlds[parent].plan.session != session { break; }
            let owner = self.worlds[index]
                .owner
                .as_ref()
                .ok_or(WorkspaceError::StaleObservation)?;
            let ready=self.in_world(view,sessions,parent,CHECK_FRAME_OWNER,vec![json!({"objectId":owner})],true).await?;
            if ready["error"].is_string() {read_point(&ready)?;}
            if ready["ready"]!=true {return Err(WorkspaceError::NotActionable);}
            let viewport=self.viewport(view,sessions,index).await?;
            let quad=self.content_quad(view,sessions,index).await?;
            let mut parent_point=quad.project(point,viewport)?;
            // Box-model coordinates belong to the protocol session's root,
            // which may be above a same-process parent document.
            let parent_quad=if self.is_session_root(parent) {None} else {Some(self.content_quad(view,sessions,parent).await?)};
            let parent_viewport=self.viewport(view,sessions,parent).await?;
            if let Some(parent_quad)=&parent_quad {parent_point=parent_quad.unproject(parent_point,parent_viewport)?;}
            let mapped = self
                .in_world(
                    view,
                    sessions,
                    parent,
                    CHECK_FRAME_HIT,
                    vec![
                        json!({"objectId":owner}),
                        json!({"value":parent_point.0}),
                        json!({"value":parent_point.1}),
                    ],
                    true,
                )
                .await?;
            point = read_point(&mapped)?;
            if !quad.unchanged(&self.content_quad(view,sessions,index).await?)
                || self.viewport(view,sessions,index).await?!=viewport
                || self.viewport(view,sessions,parent).await?!=parent_viewport {return Err(WorkspaceError::NotActionable);}
            if let Some(parent_quad)=parent_quad {
                if !parent_quad.unchanged(&self.content_quad(view,sessions,parent).await?) {return Err(WorkspaceError::NotActionable);}
            }
            index = parent;
        }
        Ok(DragSource { session: if source_scope { session } else { None }, point })
    }
}

fn read_point(value: &Value) -> Result<(f64, f64), WorkspaceError> {
    match value["error"].as_str() {
        Some("stale") => return Err(WorkspaceError::StaleObservation),
        Some("unsupported") => return Err(WorkspaceError::UnsupportedAction),
        Some(_) => return Err(WorkspaceError::NotActionable),
        None => {}
    }
    let point = (
        value["x"].as_f64().ok_or(WorkspaceError::NotActionable)?,
        value["y"].as_f64().ok_or(WorkspaceError::NotActionable)?,
    );
    if !point.0.is_finite() || !point.1.is_finite() {
        return Err(WorkspaceError::NotActionable);
    }
    Ok(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_orders_same_process_frames_and_keeps_disconnected_unobserved() {
        let trees = FrameTrees {
            root: json!({"frame":{"id":"root"},"childFrames":[
                {"frame":{"id":"one"},"childFrames":[{"frame":{"id":"two"}}]},
                {"frame":{"id":"unrelated","parentId":"foreign"}}
            ]}),
            children: vec![],
        };
        let plans = plans(&trees).unwrap();
        assert_eq!(
            plans
                .iter()
                .map(|plan| plan.id.as_str())
                .collect::<Vec<_>>(),
            ["root", "one", "two"]
        );
        assert_eq!(plans[2].parent, Some(1));
        assert!(plans.iter().all(|plan| plan.session.is_none()));
    }

    #[test]
    fn plan_bounds_depth_and_rejects_contradicting_parents() {
        let mut tree = json!({"frame":{"id":"deep"}});
        for i in (0..20).rev() {
            tree = json!({"frame":{"id":format!("f{i}")},"childFrames":[tree]});
        }
        assert_eq!(
            plans(&FrameTrees {
                root: tree,
                children: vec![]
            })
            .unwrap()
            .len(),
            9
        );
        let tree = json!({"frame":{"id":"root"},"childFrames":[
            {"frame":{"id":"child","parentId":"first"}}, {"frame":{"id":"child","parentId":"second"}}
        ]});
        assert!(
            plans(&FrameTrees {
                root: tree,
                children: vec![]
            })
            .is_err()
        );
    }

    #[test]
    fn drag_support_never_crosses_a_native_frame_session() {
        let first=FrameSession::fixture("first");
        let same=first.clone();
        let second=FrameSession::fixture("second");
        assert!(same_drag_session(None,None));
        assert!(same_drag_session(Some(&first),Some(&same)));
        assert!(!same_drag_session(None,Some(&first)));
        assert!(!same_drag_session(Some(&first),None));
        assert!(!same_drag_session(Some(&first),Some(&second)));
    }
}
