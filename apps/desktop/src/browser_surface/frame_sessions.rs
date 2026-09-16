//! OOPIF routes discovered exclusively through one embedded WebView's events.
//! No browser-wide discovery, arbitrary target attachment or public debug port.

use super::{ProtocolEvents, protocol_call, protocol_call_session, listen_frames};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::Ordering,
};

const MAX_SESSIONS: usize = 32;
const MAX_DEPTH: usize = 8;
const ROUTE_ERROR: &str = "Browser frame routing is stale or unavailable.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameSession {
    owner: uuid::Uuid,
    session: String,
    generation: u64,
    pub(crate) frame_id: String,
}
impl FrameSession {
    pub(crate) fn matches_protocol_session(&self,session:&str)->bool { self.session==session }
    #[cfg(test)]
    pub(crate) fn fixture(session: &str) -> Self {
        Self { owner:uuid::Uuid::nil(),session:session.into(),generation:1,frame_id:format!("frame-{session}") }
    }
}

struct Route {
    frame: FrameSession,
    parent: String,
    depth: usize,
    configured: bool,
}

struct Routes {
    owner: uuid::Uuid,
    next_generation: u64,
    entries: BTreeMap<String, Route>,
}

impl Routes {
    fn new(owner: uuid::Uuid) -> Self {
        Self {
            owner,
            next_generation: 0,
            entries: BTreeMap::new(),
        }
    }

    fn apply(&mut self, method: &str, parent: &str, params: &Value) -> Result<(), String> {
        let parent_depth = if parent.is_empty() {
            0
        } else {
            // A late descendant event after its parent detached cannot acquire
            // ownership. Worker/page sessions are never admitted as parents.
            let Some(route) = self.entries.get(parent) else {
                return Ok(());
            };
            route.depth
        };
        let session = params["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or(ROUTE_ERROR)?;
        match method {
            "Target.attachedToTarget" => {
                if params["targetInfo"]["type"] != "iframe" {
                    return Ok(());
                }
                let frame = params["targetInfo"]["targetId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or(ROUTE_ERROR)?;
                if let Some(existing) = self.entries.get(session) {
                    return if existing.parent == parent && existing.frame.frame_id == frame {
                        Ok(())
                    } else {
                        Err(ROUTE_ERROR.into())
                    };
                }
                if self.entries.len() >= MAX_SESSIONS
                    || parent_depth >= MAX_DEPTH
                    || self
                        .entries
                        .values()
                        .any(|route| route.frame.frame_id == frame)
                {
                    return Err(ROUTE_ERROR.into());
                }
                self.next_generation = self.next_generation.checked_add(1).ok_or(ROUTE_ERROR)?;
                self.entries.insert(
                    session.into(),
                    Route {
                        frame: FrameSession {
                            owner: self.owner,
                            session: session.into(),
                            generation: self.next_generation,
                            frame_id: frame.into(),
                        },
                        parent: parent.into(),
                        depth: parent_depth + 1,
                        configured: false,
                    },
                );
            }
            "Target.detachedFromTarget" => {
                if !self
                    .entries
                    .get(session)
                    .is_some_and(|route| route.parent == parent)
                {
                    return Ok(());
                }
                let mut removed = BTreeSet::from([session.to_owned()]);
                loop {
                    let before = removed.len();
                    for (id, route) in &self.entries {
                        if removed.contains(&route.parent) {
                            removed.insert(id.clone());
                        }
                    }
                    if before == removed.len() {
                        break;
                    }
                }
                self.entries.retain(|id, _| !removed.contains(id));
            }
            _ => return Err(ROUTE_ERROR.into()),
        }
        Ok(())
    }

    fn contains(&self, frame: &FrameSession) -> bool {
        frame.owner == self.owner
            && self
                .entries
                .get(&frame.session)
                .is_some_and(|route| route.frame == *frame)
    }
}

pub(crate) struct FrameTrees {
    pub(crate) root: Value,
    pub(crate) children: Vec<(FrameSession, Value)>,
}

/// A host-only route retained by a user picker while the Agent driver is idle.
/// No serialized session ID can construct this authority.
pub(crate) struct OwnedFrameRoute {
    view: tauri::Webview,
    frame: Option<FrameSession>,
    routes: std::sync::Arc<std::sync::Mutex<Routes>>,
    invalid: std::sync::Arc<std::sync::atomic::AtomicBool>,
    lineage: Vec<String>,
}
impl OwnedFrameRoute {
    pub(crate) fn document_lineage(&self) -> Vec<String> { self.lineage.clone() }
    fn current(&self) -> Result<(), String> {
        if self.invalid.load(Ordering::Acquire) { return Err(ROUTE_ERROR.into()); }
        if let Some(frame) = &self.frame {
            if !self.routes.lock().map_err(|_|ROUTE_ERROR)?.contains(frame) { return Err(ROUTE_ERROR.into()); }
        }
        Ok(())
    }
    pub(crate) async fn command(&self, method: &str, params: Value) -> Result<Value, String> {
        self.current()?;
        let result=protocol_call_session(&self.view,self.frame.as_ref().map(|frame|frame.session.as_str()),method,params).await;
        self.current()?;
        result
    }
    pub(crate) async fn set_user_files(&self,object:&str,paths:&[std::path::PathBuf],guard:super::UserFileCommandGuard) -> Result<(),String> {
        self.current()?;
        let result=super::set_user_files(&self.view,self.frame.as_ref().map(|frame|frame.session.as_str()),json!({"objectId":object,"files":paths}),guard).await;
        self.current()?;
        result.map(|_|())
    }
}

fn tree_contains(tree: &Value, id: &str) -> bool {
    let mut pending=vec![tree];
    let mut visited=0;
    while let Some(tree)=pending.pop() {
        visited+=1;
        if visited>256 { return false; }
        if tree["frame"]["id"].as_str()==Some(id) { return true; }
        if let Some(children)=tree["childFrames"].as_array() { pending.extend(children); }
    }
    false
}

impl FrameTrees {
    fn lineage(&self, id:&str) -> Result<Vec<String>,String> {
        let mut parents=BTreeMap::<String,Option<String>>::new();
        let mut pending=vec![(&self.root,None)];
        pending.extend(self.children.iter().map(|(_,tree)|(tree,None)));
        let mut visited=0;
        while let Some((tree,inherited))=pending.pop() {
            visited+=1;
            if visited>256 {return Err(ROUTE_ERROR.into());}
            let node=tree["frame"]["id"].as_str().ok_or(ROUTE_ERROR)?;
            let parent=tree["frame"]["parentId"].as_str().or(inherited).map(str::to_owned);
            if let Some(Some(existing))=parents.get(node) {
                if parent.as_ref().is_some_and(|parent|parent!=existing) {return Err(ROUTE_ERROR.into());}
            }
            if parent.is_some() || !parents.contains_key(node) {parents.insert(node.into(),parent);}
            if let Some(children)=tree["childFrames"].as_array() {pending.extend(children.iter().map(|child|(child,Some(node))));}
        }
        let mut result=vec![];
        let mut node=id;
        loop {
            if result.len()>=256 || result.iter().any(|previous|previous==node) {return Err(ROUTE_ERROR.into());}
            result.push(node.to_owned());
            match parents.get(node).ok_or(ROUTE_ERROR)? {
                Some(parent)=>node=parent,
                None=>break,
            }
        }
        if result.last().map(String::as_str)!=self.root["frame"]["id"].as_str() {return Err(ROUTE_ERROR.into());}
        Ok(result)
    }
    pub(crate) fn descendant_count(&self) -> Result<usize, String> {
        let mut ids = BTreeSet::new();
        let mut pending = vec![&self.root];
        pending.extend(self.children.iter().map(|(_, tree)| tree));
        let mut visited = 0;
        while let Some(tree) = pending.pop() {
            visited += 1;
            if visited > 256 {
                return Err("Browser frame tree exceeds its limit.".into());
            }
            let id = tree["frame"]["id"].as_str().ok_or(ROUTE_ERROR)?;
            ids.insert(id);
            if let Some(children) = tree["childFrames"].as_array() {
                pending.extend(children);
            }
        }
        Ok(ids.len().saturating_sub(1))
    }
}

async fn consume_events<F, Fut>(
    mut receiver: tokio::sync::mpsc::Receiver<super::ProtocolMessage>,
    routes: std::sync::Arc<std::sync::Mutex<Routes>>,
    invalid: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mut configure: F,
) where F: FnMut(FrameSession) -> Fut, Fut: std::future::Future<Output = Result<(), String>> {
    while let Some(message) = receiver.recv().await {
        match message {
            super::ProtocolMessage::Barrier(sender) => {
                let _ = sender.send(());
            }
            super::ProtocolMessage::Event(event) => {
                if invalid.load(Ordering::Acquire) {
                    continue;
                }
                let result = serde_json::from_str::<Value>(&event.parameters)
                    .map_err(|_| ROUTE_ERROR.to_owned())
                    .and_then(|params| {
                        routes.lock().map_err(|_| ROUTE_ERROR.to_owned())?.apply(
                            event.method,
                            &event.parent_session,
                            &params,
                        )
                    });
                if result.is_err() {
                    invalid.store(true, Ordering::Release);
                    continue;
                }
                // Configure at attachment, not on the next observation. A new
                // OOPIF must acquire policy before its first script can run.
                let pending = routes.lock().ok().and_then(|routes| routes.entries.values()
                    .find(|route| !route.configured).map(|route| route.frame.clone()));
                if let Some(frame) = pending {
                    let result = configure(frame.clone()).await;
                    if let Ok(mut routes) = routes.lock() {
                        if let Some(route) = routes.entries.get_mut(&frame.session) {
                            if route.frame == frame { route.configured = result.is_ok(); }
                        }
                    }
                    if result.is_err() { invalid.store(true, Ordering::Release); }
                }
            }
        }
    }
    invalid.store(true, Ordering::Release);
}

pub(crate) struct FrameSessions {
    chooser_interception: std::sync::Arc<std::sync::atomic::AtomicBool>,
    events: ProtocolEvents,
    routes: std::sync::Arc<std::sync::Mutex<Routes>>,
    // Dropping the handle detaches, never aborts in-flight native callbacks.
    // ProtocolEvents removal closes the queue; the worker drains then exits.
    _worker: tokio::task::JoinHandle<()>,
}

impl FrameSessions {
    pub(crate) async fn connect(view: &tauri::Webview) -> Result<Self, String> {
        let mut events = listen_frames(view).await?;
        let routes = std::sync::Arc::new(std::sync::Mutex::new(Routes::new(events.id)));
        let receiver = events.receiver.take().ok_or(ROUTE_ERROR)?;
        let chooser_interception = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let policy = chooser_interception.clone();
        let owner = view.clone();
        let worker = tokio::spawn(consume_events(
            receiver,
            routes.clone(),
            events.invalid.clone(),
            move |frame| {
                let view = owner.clone();
                let policy = policy.clone();
                async move {
                    let configured = async {
                        protocol_call_session(&view,Some(&frame.session),"Target.setAutoAttach",Self::auto_attach()).await?;
                        protocol_call_session(&view,Some(&frame.session),"Page.enable",json!({})).await?;
                        // Diagnostic subscribers consume the same attached
                        // iframe lineage. Enable before releasing its first JS.
                        if super::diagnostics::enable_session(&view,Some(&frame.session)).await.is_err() {
                            super::diagnostics::mark_unavailable(&view);
                        }
                        protocol_call_session(&view,Some(&frame.session),"Page.setInterceptFileChooserDialog",json!({"enabled":policy.load(Ordering::Acquire)})).await?;
                        Ok::<_,String>(())
                    }.await;
                    // Never run an Agent-owned document without its policy.
                    // Failure invalidates routing; native close/recovery owns
                    // cleanup of the failed view, not an unprotected resume.
                    configured?;
                    let resumed = protocol_call_session(&view,Some(&frame.session),"Runtime.runIfWaitingForDebugger",json!({})).await;
                    resumed.map(|_|())
                }
            },
        ));
        let mut this = Self {
            chooser_interception,
            routes,
            events,
            _worker: worker,
        };
        protocol_call(view, "Target.setAutoAttach", Self::auto_attach()).await?;
        this.refresh().await?;
        Ok(this)
    }

    fn auto_attach() -> Value {
        // The event worker installs policy and promptly resumes each owned
        // iframe. This does not wait for an Agent observation or user action.
        json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,
            "filter":[{"type":"iframe"},{"exclude":true}]})
    }

    fn with_routes<T>(&self, action: impl FnOnce(&mut Routes) -> T) -> Result<T, String> {
        if self.events.invalid.load(Ordering::Acquire) {
            return Err(ROUTE_ERROR.into());
        }
        let mut routes = self.routes.lock().map_err(|_| ROUTE_ERROR.to_owned())?;
        Ok(action(&mut routes))
    }

    async fn synchronize(&self) -> Result<(), String> {
        if self.events.invalid.load(Ordering::Acquire) {
            return Err(ROUTE_ERROR.into());
        }
        // FIFO barrier: all already-received lifecycle events must be applied
        // before checking a route. The worker also drains while users browse.
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.events
            .sender
            .send(super::ProtocolMessage::Barrier(sender))
            .await
            .map_err(|_| ROUTE_ERROR.to_owned())?;
        receiver.await.map_err(|_| ROUTE_ERROR.to_owned())?;
        self.with_routes(|_| ())
    }

    async fn refresh(&mut self) -> Result<(), String> {
        // The FIFO barrier waits for earlier attachment policy callbacks too.
        self.synchronize().await
    }

    async fn configure_file_chooser(&self,frame:&FrameSession)->Result<Value,String> {
        self.command(frame,"Page.enable",json!({})).await?;
        self.command(frame,"Page.setInterceptFileChooserDialog",json!({"enabled":self.chooser_interception.load(Ordering::Acquire)})).await
    }
    pub(crate) async fn set_file_chooser_interception(&mut self,enabled:bool)->Result<(),String> {
        self.chooser_interception.store(enabled, Ordering::Release);
        self.refresh().await?;
        protocol_call(&self.events.view,"Page.enable",json!({})).await?;
        let frames=self.with_routes(|routes|routes.entries.values().map(|route|route.frame.clone()).collect::<Vec<_>>())?;
        let mut failure=None;
        for frame in frames {
            if let Err(error)=self.configure_file_chooser(&frame).await {failure=Some(error);}
        }
        failure.map_or(Ok(()),Err)
    }

    pub(crate) async fn command(
        &self,
        frame: &FrameSession,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        self.synchronize().await?;
        if !self.with_routes(|routes| routes.contains(frame))? {
            return Err(ROUTE_ERROR.into());
        }
        let result =
            protocol_call_session(&self.events.view, Some(&frame.session), method, params).await;
        self.synchronize().await?;
        if !self.with_routes(|routes| routes.contains(frame))? {
            return Err(ROUTE_ERROR.into());
        }
        result
    }

    pub(crate) async fn trees(&mut self) -> Result<FrameTrees, String> {
        self.refresh().await?;
        let root =
            protocol_call(&self.events.view, "Page.getFrameTree", json!({})).await?["frameTree"].clone();
        self.refresh().await?;
        let frames: Vec<_> = self.with_routes(|routes| {
            routes
                .entries
                .values()
                .map(|route| route.frame.clone())
                .collect()
        })?;
        let mut children = vec![];
        for frame in frames {
            let result = self.command(&frame, "Page.getFrameTree", json!({})).await;
            if !self.with_routes(|routes| routes.contains(&frame))? {
                continue;
            }
            let tree = result?["frameTree"].clone();
            // Prove the CDP session reaches the very iframe we admitted.
            if tree["frame"]["id"].as_str() != Some(frame.frame_id.as_str()) {
                self.events.invalid.store(true, Ordering::Release);
                return Err(ROUTE_ERROR.into());
            }
            children.push((frame, tree));
        }
        let trees = FrameTrees { root, children };
        trees.descendant_count()?;
        Ok(trees)
    }

    pub(crate) async fn chooser_route(&mut self, frame_id: &str, session: &str) -> Result<OwnedFrameRoute, String> {
        let trees=self.trees().await?;
        let frame=if session.is_empty() {
            if !tree_contains(&trees.root,frame_id) || trees.children.iter().any(|(_,tree)|tree_contains(tree,frame_id)) {
                return Err(ROUTE_ERROR.into());
            }
            None
        } else {
            Some(trees.children.iter().find(|(frame,tree)|frame.matches_protocol_session(session) && tree_contains(tree,frame_id))
                .ok_or(ROUTE_ERROR)?.0.clone())
        };
        Ok(OwnedFrameRoute { view:self.events.view.clone(),frame,routes:self.routes.clone(),invalid:self.events.invalid.clone(),lineage:trees.lineage(frame_id)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attach(
        routes: &mut Routes,
        parent: &str,
        session: &str,
        frame: &str,
        kind: &str,
    ) -> Result<(), String> {
        routes.apply(
            "Target.attachedToTarget",
            parent,
            &json!({"sessionId":session,
            "targetInfo":{"type":kind,"targetId":frame}}),
        )
    }

    #[test]
    fn only_owned_iframe_descendants_are_admitted() {
        let mut routes = Routes::new(uuid::Uuid::now_v7());
        attach(&mut routes, "", "worker", "worker-id", "worker").unwrap();
        attach(&mut routes, "", "popup", "popup-id", "page").unwrap();
        attach(&mut routes, "worker", "escape", "escape-id", "iframe").unwrap();
        attach(&mut routes, "foreign", "other", "other-id", "iframe").unwrap();
        assert!(routes.entries.is_empty());
        attach(&mut routes, "", "first", "frame1", "iframe").unwrap();
        attach(&mut routes, "first", "nested", "frame2", "iframe").unwrap();
        assert_eq!(routes.entries.len(), 2);
        let frame = routes.entries["nested"].frame.clone();
        assert!(routes.contains(&frame));
        let mut other = Routes::new(uuid::Uuid::now_v7());
        attach(&mut other, "", "first", "frame1", "iframe").unwrap();
        attach(&mut other, "first", "nested", "frame2", "iframe").unwrap();
        assert!(!other.contains(&frame));
    }

    #[test]
    fn parent_detach_invalidates_descendants_and_session_reuse() {
        let mut routes = Routes::new(uuid::Uuid::now_v7());
        attach(&mut routes, "", "first", "frame1", "iframe").unwrap();
        attach(&mut routes, "first", "nested", "frame2", "iframe").unwrap();
        let frame = routes.entries["first"].frame.clone();
        routes
            .apply(
                "Target.detachedFromTarget",
                "nested",
                &json!({"sessionId":"first"}),
            )
            .unwrap();
        assert!(routes.contains(&frame));
        routes
            .apply(
                "Target.detachedFromTarget",
                "",
                &json!({"sessionId":"first"}),
            )
            .unwrap();
        assert!(routes.entries.is_empty());
        attach(&mut routes, "first", "late", "late-frame", "iframe").unwrap();
        assert!(routes.entries.is_empty());
        attach(&mut routes, "", "first", "frame1", "iframe").unwrap();
        assert!(!routes.contains(&frame));
    }

    #[test]
    fn route_aliases_and_limits_fail_closed() {
        let mut routes = Routes::new(uuid::Uuid::now_v7());
        attach(&mut routes, "", "first", "frame1", "iframe").unwrap();
        attach(&mut routes, "", "first", "frame1", "iframe").unwrap();
        assert!(attach(&mut routes, "", "first", "different", "iframe").is_err());
        assert!(attach(&mut routes, "", "alias", "frame1", "iframe").is_err());
        for i in 1..MAX_SESSIONS {
            attach(
                &mut routes,
                "",
                &format!("s{i}"),
                &format!("f{i}"),
                "iframe",
            )
            .unwrap();
        }
        assert!(attach(&mut routes, "", "overflow", "overflow", "iframe").is_err());
        let mut routes = Routes::new(uuid::Uuid::now_v7());
        let mut parent = String::new();
        for i in 0..MAX_DEPTH {
            let session = format!("s{i}");
            attach(&mut routes, &parent, &session, &format!("f{i}"), "iframe").unwrap();
            parent = session;
        }
        assert!(attach(&mut routes, &parent, "too-deep", "too-deep", "iframe").is_err());
    }

    #[test]
    fn frame_tree_count_deduplicates_oopif_roots() {
        let frame = FrameSession {
            owner: uuid::Uuid::now_v7(),
            session: "s".into(),
            generation: 1,
            frame_id: "child".into(),
        };
        let trees = FrameTrees {
            root: json!({"frame":{"id":"root"},"childFrames":[{"frame":{"id":"child"}}]}),
            children: vec![(
                frame,
                json!({"frame":{"id":"child"},"childFrames":[{"frame":{"id":"nested"}}]}),
            )],
        };
        assert_eq!(trees.descendant_count().unwrap(), 2);
        assert_eq!(trees.lineage("nested").unwrap(),vec!["nested","child","root"]);
        assert_eq!(trees.lineage("root").unwrap(),vec!["root"]);
        assert!(trees.lineage("unknown").is_err());
    }

    #[test]
    fn file_document_lineage_rejects_cycles_and_conflicting_parents() {
        let trees=FrameTrees {root:json!({"frame":{"id":"root"},"childFrames":[{"frame":{"id":"child","parentId":"child"}}]}),children:vec![]};
        assert!(trees.lineage("child").is_err());
        let trees=FrameTrees {root:json!({"frame":{"id":"root"},"childFrames":[{"frame":{"id":"child","parentId":"missing"}}]}),children:vec![]};
        assert!(trees.lineage("child").is_err());
    }

    #[tokio::test]
    async fn lifecycle_worker_drains_without_agent_observations() {
        use super::super::{ProtocolEvent, ProtocolMessage};
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        let routes = Arc::new(Mutex::new(Routes::new(uuid::Uuid::now_v7())));
        let invalid = Arc::new(AtomicBool::new(false));
        let worker = tokio::spawn(consume_events(receiver, routes.clone(), invalid.clone(), |_| async { Ok(()) }));
        // More events than the entire buffer, without a trees/command call.
        for _ in 0..100 {
            for (method, params) in [
                (
                    "Target.attachedToTarget",
                    json!({"sessionId":"session","targetInfo":{"type":"iframe","targetId":"frame"}}),
                ),
                ("Target.detachedFromTarget", json!({"sessionId":"session"})),
            ] {
                sender
                    .send(ProtocolMessage::Event(ProtocolEvent {
                        method,
                        parent_session: String::new(),
                        parameters: params.to_string(),
                    }))
                    .await
                    .unwrap();
            }
        }
        let (barrier, completed) = tokio::sync::oneshot::channel();
        sender
            .send(ProtocolMessage::Barrier(barrier))
            .await
            .unwrap();
        completed.await.unwrap();
        assert!(!invalid.load(Ordering::Acquire));
        assert!(routes.lock().unwrap().entries.is_empty());
        assert_eq!(routes.lock().unwrap().next_generation, 100);
        drop(sender);
        worker.await.unwrap();
        assert!(invalid.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn attachment_policy_settles_before_barrier_and_is_not_repeated() {
        use super::super::{ProtocolEvent, ProtocolMessage};
        use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize}};
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let routes = Arc::new(Mutex::new(Routes::new(uuid::Uuid::now_v7())));
        let invalid = Arc::new(AtomicBool::new(false));
        let (finish, finished) = tokio::sync::oneshot::channel();
        let mut finished = Some(finished);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let worker = tokio::spawn(consume_events(receiver, routes.clone(), invalid.clone(), move |_| {
            counted.fetch_add(1, Ordering::AcqRel);
            let done = finished.take().expect("duplicate native configuration");
            async move { done.await.map_err(|_| "policy callback lost".into()) }
        }));
        for _ in 0..2 {
            sender.send(ProtocolMessage::Event(ProtocolEvent {
                method:"Target.attachedToTarget", parent_session:String::new(),
                parameters:json!({"sessionId":"s","targetInfo":{"type":"iframe","targetId":"f"}}).to_string(),
            })).await.unwrap();
        }
        let (barrier, mut completed) = tokio::sync::oneshot::channel();
        sender.send(ProtocolMessage::Barrier(barrier)).await.unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut completed).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 1);
        finish.send(()).unwrap();
        completed.await.unwrap();
        assert!(routes.lock().unwrap().entries["s"].configured);
        assert!(!invalid.load(Ordering::Acquire));
        assert_eq!(calls.load(Ordering::Acquire), 1);
        drop(sender);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn failed_attachment_policy_invalidates_before_barrier() {
        use super::super::{ProtocolEvent, ProtocolMessage};
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let routes = Arc::new(Mutex::new(Routes::new(uuid::Uuid::now_v7())));
        let invalid = Arc::new(AtomicBool::new(false));
        let worker = tokio::spawn(consume_events(receiver, routes.clone(), invalid.clone(), |_| async { Err("native policy failure".into()) }));
        sender.send(ProtocolMessage::Event(ProtocolEvent {
            method:"Target.attachedToTarget", parent_session:String::new(),
            parameters:json!({"sessionId":"s","targetInfo":{"type":"iframe","targetId":"f"}}).to_string(),
        })).await.unwrap();
        let (barrier, completed) = tokio::sync::oneshot::channel();
        sender.send(ProtocolMessage::Barrier(barrier)).await.unwrap();
        completed.await.unwrap();
        assert!(invalid.load(Ordering::Acquire));
        assert!(!routes.lock().unwrap().entries["s"].configured);
        drop(sender);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_lifecycle_event_permanently_invalidates_routing() {
        use super::super::{ProtocolEvent, ProtocolMessage};
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        let routes = Arc::new(Mutex::new(Routes::new(uuid::Uuid::now_v7())));
        let invalid = Arc::new(AtomicBool::new(false));
        let worker = tokio::spawn(consume_events(receiver, routes.clone(), invalid.clone(), |_| async { Ok(()) }));
        for parameters in [
            "broken JSON".into(),
            json!({"sessionId":"s","targetInfo":{"type":"iframe","targetId":"f"}}).to_string(),
        ] {
            sender
                .send(ProtocolMessage::Event(ProtocolEvent {
                    method: "Target.attachedToTarget",
                    parent_session: String::new(),
                    parameters,
                }))
                .await
                .unwrap();
        }
        let (barrier, completed) = tokio::sync::oneshot::channel();
        sender
            .send(ProtocolMessage::Barrier(barrier))
            .await
            .unwrap();
        completed.await.unwrap();
        assert!(invalid.load(Ordering::Acquire));
        assert!(routes.lock().unwrap().entries.is_empty());
        drop(sender);
        worker.await.unwrap();
    }
}
