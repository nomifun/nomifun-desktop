//! Scoped iframe routing for one granted page. No browser-wide auto-attach,
//! discovery, workers, page creation, debugger pause, or background task.
use super::Error;
use crate::transport::{CdpEvent, Connection};
use chromiumoxide::cdp::browser_protocol::{page::GetFrameTreeParams, target::SetAutoAttachParams};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

const MAX_SESSIONS: usize = 32;
const MAX_DEPTH: usize = 8;
const MAX_EVENTS: usize = 128;
const MAX_RETIRED: usize = 128;
const MAX_PASSES: usize = 12;

struct Route {
    parent: String,
    frame: String,
    depth: usize,
    configured: bool,
}
pub(super) struct FrameRoutes {
    root: String,
    root_frame: Option<String>,
    root_configured: bool,
    root_detached: bool,
    routes: BTreeMap<String, Route>,
    retired: BTreeSet<String>,
    attached: UnboundedReceiver<CdpEvent>,
    detached: UnboundedReceiver<CdpEvent>,
    invalid: bool,
    disabling: bool,
    revision: u64,
}
impl FrameRoutes {
    pub(super) fn new(conn: &Connection, root_session: &str) -> Self {
        Self {
            root: root_session.into(),
            root_frame: None,
            root_configured: false,
            root_detached: false,
            routes: BTreeMap::new(),
            retired: BTreeSet::new(),
            attached: conn.subscribe_reliable("Target.attachedToTarget", None),
            detached: conn.subscribe_reliable("Target.detachedFromTarget", None),
            invalid: root_session.is_empty() || root_session.len() > 256,
            disabling: false,
            revision: 0,
        }
    }
    /// Deepest children first; the caller owns final root detach. Session ids
    /// are internal authority, never user/model input or capability output.
    pub(super) fn sessions(&self) -> Vec<String> {
        let mut children: Vec<_> = self.routes.iter().collect();
        children
            .sort_by_key(|(session, route)| (std::cmp::Reverse(route.depth), (*session).clone()));
        children
            .into_iter()
            .map(|(session, _)| session.clone())
            .chain([self.root.clone()])
            .collect()
    }
    fn remove(&mut self, session: &str) -> Result<(), Error> {
        let mut removed = BTreeSet::from([session.to_owned()]);
        loop {
            let before = removed.len();
            for (child, route) in &self.routes {
                if removed.contains(&route.parent) {
                    removed.insert(child.clone());
                }
            }
            if removed.len() == before {
                break;
            }
        }
        for removed in removed {
            self.routes.remove(&removed);
            self.retired.insert(removed);
        }
        self.revision += 1;
        if self.retired.len() > MAX_RETIRED {
            return Err(Error::ExecutionFailed);
        }
        Ok(())
    }
    fn attached(&mut self, event: CdpEvent) -> Result<(), Error> {
        let parent = &event.session_id;
        let depth = if parent == &self.root {
            0
        } else if let Some(route) = self.routes.get(parent) {
            route.depth
        } else {
            return Ok(());
        };
        if event.params["targetInfo"]["type"] != "iframe" {
            return Ok(());
        }
        let session = identifier(&event.params["sessionId"])?;
        let frame = identifier(&event.params["targetInfo"]["targetId"])?;
        if session == self.root || session == *parent || self.retired.contains(&session) {
            return Err(Error::ExecutionFailed);
        }
        if let Some(route) = self.routes.get(&session) {
            return if route.parent == *parent && route.frame == frame {
                Ok(())
            } else {
                Err(Error::ExecutionFailed)
            };
        }
        if self.routes.len() >= MAX_SESSIONS
            || depth >= MAX_DEPTH
            || self.routes.values().any(|route| route.frame == frame)
        {
            return Err(Error::ExecutionFailed);
        }
        self.routes.insert(
            session,
            Route {
                parent: parent.clone(),
                frame,
                depth: depth + 1,
                configured: false,
            },
        );
        self.revision += 1;
        Ok(())
    }
    fn detached(&mut self, event: CdpEvent) -> Result<(), Error> {
        if event.session_id.is_empty() && event.params["sessionId"] == self.root {
            self.root_detached = true;
            return Err(Error::Disconnected);
        }
        if event.session_id != self.root && !self.routes.contains_key(&event.session_id) {
            return Ok(());
        }
        let session = identifier(&event.params["sessionId"])?;
        if self
            .routes
            .get(&session)
            .is_some_and(|route| route.parent == event.session_id)
        {
            self.remove(&session)?;
        }
        Ok(())
    }
    fn drain(&mut self, conn: &Connection, budget: &mut usize) -> Result<(), Error> {
        // The transport has per-method queues, not a total-order stream.
        // Process attachment FIFO, then retire subtrees. Never resurrect a
        // detached id or guess a generation when the streams overlap.
        loop {
            let event = match self.attached.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err(Error::Disconnected),
            };
            *budget += 1;
            if *budget > MAX_EVENTS {
                return Err(Error::ExecutionFailed);
            }
            self.attached(event)?;
        }
        loop {
            let event = match self.detached.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err(Error::Disconnected),
            };
            *budget += 1;
            if *budget > MAX_EVENTS {
                return Err(Error::ExecutionFailed);
            }
            self.detached(event)?;
        }
        if self.root_detached
            || conn.registry().is_connection_closed()
            || !conn.registry().has_session(&self.root)
        {
            return Err(Error::Disconnected);
        }
        let dead: Vec<_> = self
            .routes
            .keys()
            .filter(|session| !conn.registry().has_session(session))
            .cloned()
            .collect();
        for session in dead {
            self.remove(&session)?;
        }
        Ok(())
    }
    pub(super) async fn refresh(
        &mut self,
        conn: &Connection,
    ) -> Result<Vec<(String, Value)>, Error> {
        if self.invalid || self.disabling {
            return Err(Error::ExecutionFailed);
        }
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(10), self.refresh_inner(conn))
                .await
                .unwrap_or(Err(Error::ExecutionFailed));
        if result.is_err() {
            self.invalid = true;
        }
        result
    }
    async fn refresh_inner(&mut self, conn: &Connection) -> Result<Vec<(String, Value)>, Error> {
        let mut budget = 0;
        for _ in 0..MAX_PASSES {
            self.drain(conn, &mut budget)?;
            if !self.root_configured {
                // Mark before await: a lost ACK still leaves a cleanup duty.
                self.root_configured = true;
                configure(conn, &self.root, true).await?;
                self.drain(conn, &mut budget)?;
            }
            let pending: Vec<_> = self
                .routes
                .iter()
                .filter(|(_, route)| !route.configured)
                .map(|(session, _)| session.clone())
                .collect();
            for session in pending {
                let Some(route) = self.routes.get_mut(&session) else {
                    continue;
                };
                route.configured = true;
                configure(conn, &session, true).await?;
                self.drain(conn, &mut budget)?;
            }
            if self.routes.values().any(|route| !route.configured) {
                continue;
            }
            let revision = self.revision;
            let sessions: Vec<_> = std::iter::once(self.root.clone())
                .chain(self.routes.keys().cloned())
                .collect();
            let mut trees = Vec::new();
            for session in sessions {
                let tree = conn
                    .send(&session, &GetFrameTreeParams::default())
                    .await
                    .map_err(|_| Error::ExecutionFailed)?["frameTree"]
                    .clone();
                validate_tree(&tree)?;
                let frame = identifier(&tree["frame"]["id"])?;
                self.drain(conn, &mut budget)?;
                if self.revision != revision {
                    break;
                }
                if session == self.root {
                    if self
                        .root_frame
                        .as_ref()
                        .is_some_and(|expected| expected != &frame)
                    {
                        return Err(Error::ExecutionFailed);
                    }
                    self.root_frame = Some(frame);
                } else if self
                    .routes
                    .get(&session)
                    .is_none_or(|route| route.frame != frame)
                {
                    return Err(Error::ExecutionFailed);
                }
                trees.push((session, tree));
            }
            if self.revision != revision {
                continue;
            }
            for (session, route) in &self.routes {
                let parent = trees
                    .iter()
                    .find(|(parent, _)| parent == &route.parent)
                    .ok_or(Error::ExecutionFailed)?;
                let child = trees
                    .iter()
                    .find(|(id, _)| id == session)
                    .ok_or(Error::ExecutionFailed)?;
                let parent_id_matches = child.1["frame"]["parentId"]
                    .as_str()
                    .is_some_and(|id| contains_frame(&parent.1, id));
                // Some Chromium frame trees omit an OOPIF child, which then
                // carries the parent frame identity in its own scoped tree.
                // The semantic caller additionally proves its DOM frame owner.
                if !contains_frame(&parent.1, &route.frame) && !parent_id_matches {
                    return Err(Error::ExecutionFailed);
                }
            }
            return Ok(trees);
        }
        Err(Error::ExecutionFailed)
    }
    /// Stop our scoped auto-attach sources before the caller detaches the root.
    /// Clear input/world instrumentation first: disabling can detach children.
    /// Failed ACKs retain configured flags so cleanup can be retried exactly.
    pub(super) async fn disable(&mut self, conn: &Connection) -> Result<(), Error> {
        self.disabling = true;
        if self.root_detached {
            return Err(Error::Disconnected);
        }
        let mut failed = BTreeMap::new();
        let mut budget = 0;
        // Children must stop their own sources before an ancestor disables
        // auto-attach and asynchronously detaches them. Root-first teardown
        // races stale registry liveness with "session not found" replies.
        for session in self.sessions() {
            let configured = if session == self.root {
                self.root_configured
            } else {
                self.routes
                    .get(&session)
                    .is_some_and(|route| route.configured)
            };
            if !configured {
                continue;
            }
            let result = if conn.registry().has_session(&session) {
                configure(conn, &session, false).await
            } else {
                Ok(())
            };
            // Drain already-received events after each acknowledgement. Late
            // children were never configured by this cleanup path; the final
            // root detach remains their owner. A routing error cannot prevent
            // us from attempting to disable previously configured sources.
            if self.drain(conn, &mut budget).is_err() {
                self.invalid = true;
            }
            if result.is_ok()
                || !conn.registry().has_session(&session)
                || self.retired.contains(&session)
            {
                self.disabled(&session);
            } else if let Err(error) = result {
                failed.insert(session, error);
            }
        }
        // A later ancestor disable may prove an earlier failing child was
        // detached. Only actual registry death/scoped detach removes that duty;
        // arbitrary protocol errors never count as cleanup success.
        failed.retain(|session, _| {
            if !conn.registry().has_session(session) || self.retired.contains(session) {
                self.disabled(session);
                false
            } else {
                true
            }
        });
        failed.into_values().next().map_or(Ok(()), Err)
    }
    fn disabled(&mut self, session: &str) {
        if session == self.root {
            self.root_configured = false;
        } else if let Some(route) = self.routes.get_mut(session) {
            route.configured = false;
        }
    }
}
fn identifier(value: &Value) -> Result<String, Error> {
    value
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned)
        .ok_or(Error::ExecutionFailed)
}
async fn configure(conn: &Connection, session: &str, enabled: bool) -> Result<(), Error> {
    if session.is_empty() {
        return Err(Error::ExecutionFailed);
    }
    let mut params = json!({"autoAttach":enabled,"waitForDebuggerOnStart":false,"flatten":true});
    if enabled {
        params["filter"] = json!([{"type":"iframe"},{"exclude":true}]);
    }
    let params: SetAutoAttachParams =
        serde_json::from_value(params).map_err(|_| Error::ExecutionFailed)?;
    conn.send(session, &params)
        .await
        .map(|_| ())
        .map_err(|_| Error::ExecutionFailed)
}
fn validate_tree(tree: &Value) -> Result<(), Error> {
    if tree.to_string().len() > 1024 * 1024 {
        return Err(Error::ExecutionFailed);
    }
    let mut pending = vec![(tree, 0)];
    let mut ids = BTreeSet::new();
    while let Some((tree, depth)) = pending.pop() {
        if depth > MAX_DEPTH || ids.len() >= 64 || !ids.insert(identifier(&tree["frame"]["id"])?) {
            return Err(Error::ExecutionFailed);
        }
        if let Some(children) = tree.get("childFrames") {
            for child in children.as_array().ok_or(Error::ExecutionFailed)? {
                pending.push((child, depth + 1));
            }
        }
    }
    Ok(())
}
fn contains_frame(tree: &Value, id: &str) -> bool {
    tree["frame"]["id"] == id
        || tree["childFrames"]
            .as_array()
            .is_some_and(|children| children.iter().any(|child| contains_frame(child, id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    const ROOT: &str = "granted-page";
    fn attach(parent: &str, session: &str, frame: &str, kind: &str) -> Value {
        json!({"sessionId":parent,"method":"Target.attachedToTarget","params":{"sessionId":session,"waitingForDebugger":false,
            "targetInfo":{"targetId":frame,"type":kind,"title":"Fixture","url":"https://fixture.test/","attached":true,"canAccessOpener":false}}})
    }
    fn detach(parent: &str, session: &str) -> Value {
        json!({"sessionId":parent,"method":"Target.detachedFromTarget","params":{"sessionId":session}})
    }
    struct Wire {
        conn: Connection,
        worker: tokio::task::JoinHandle<()>,
        calls: Arc<Mutex<Vec<Value>>>,
    }
    impl Wire {
        async fn new(nested: bool, detach_on_root: bool, fail_child: bool) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let calls = Arc::new(Mutex::new(vec![]));
            let captured = calls.clone();
            let fail = AtomicBool::new(fail_child);
            let worker = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(socket).await.unwrap();
                let mut root_emitted = false;
                let mut nested_emitted = false;
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    captured.lock().unwrap().push(request.clone());
                    let session = request["sessionId"]
                        .as_str()
                        .expect("no browser-root commands");
                    assert!(
                        matches!(session, ROOT | "iframe-a" | "iframe-b"),
                        "unowned session {session}"
                    );
                    let mut reply = json!({"id":request["id"],"sessionId":session});
                    match request["method"].as_str().unwrap() {
                        "Target.setAutoAttach" => {
                            assert_eq!(request["params"]["waitForDebuggerOnStart"], false);
                            assert_eq!(request["params"]["flatten"], true);
                            let enabled = request["params"]["autoAttach"].as_bool().unwrap();
                            if enabled {
                                assert_eq!(
                                    request["params"]["filter"],
                                    json!([{"type":"iframe"},{"exclude":true}])
                                );
                            } else {
                                assert!(
                                    request["params"].get("filter").is_none(),
                                    "Chrome rejects a target filter when disabling auto-attach"
                                );
                            }
                            if enabled && session == ROOT && !root_emitted {
                                root_emitted = true;
                                for event in [
                                    attach(ROOT, "iframe-a", "frame-a", "iframe"),
                                    attach(ROOT, "foreign-page", "foreign-page-id", "page"),
                                    attach(ROOT, "worker", "worker-id", "worker"),
                                    attach("worker", "worker-child", "worker-frame", "iframe"),
                                    attach(
                                        "other-grant",
                                        "foreign-child",
                                        "foreign-frame",
                                        "iframe",
                                    ),
                                ] {
                                    socket
                                        .send(Message::Text(event.to_string().into()))
                                        .await
                                        .unwrap();
                                }
                            }
                            if enabled && session == "iframe-a" && nested && !nested_emitted {
                                nested_emitted = true;
                                socket
                                    .send(Message::Text(
                                        attach("iframe-a", "iframe-b", "frame-b", "iframe")
                                            .to_string()
                                            .into(),
                                    ))
                                    .await
                                    .unwrap();
                            }
                            if !enabled
                                && session == "iframe-a"
                                && fail.swap(false, Ordering::SeqCst)
                            {
                                reply["error"] =
                                    json!({"code":-32000,"message":"fixture source still live"});
                            } else {
                                reply["result"] = json!({});
                            }
                            if !enabled && session == ROOT && detach_on_root {
                                // The exact detach event, not an arbitrary error
                                // string, proves a previously failed child gone.
                                socket
                                    .send(Message::Text(
                                        detach(ROOT, "iframe-a").to_string().into(),
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }
                        "Page.getFrameTree" => {
                            // Match Chrome's omitted-OOPIF layout: the child
                            // tree supplies parentId; parent has no childFrames.
                            let frame = match session {
                                ROOT => json!({"id":"root-frame","loaderId":"root-loader"}),
                                "iframe-a" => {
                                    json!({"id":"frame-a","parentId":"root-frame","loaderId":"a-loader"})
                                }
                                _ => {
                                    json!({"id":"frame-b","parentId":"frame-a","loaderId":"b-loader"})
                                }
                            };
                            reply["result"] = json!({"frameTree":{"frame":frame}});
                        }
                        other => panic!("unexpected command: {other}"),
                    }
                    socket
                        .send(Message::Text(reply.to_string().into()))
                        .await
                        .unwrap();
                }
            });
            let conn = Connection::connect(&format!("ws://{address}"))
                .await
                .unwrap();
            conn.registry().register_session(ROOT, "page");
            Self {
                conn,
                worker,
                calls,
            }
        }
        fn disable_calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| {
                    call["method"] == "Target.setAutoAttach"
                        && call["params"]["autoAttach"] == false
                })
                .map(|call| call["sessionId"].as_str().unwrap().to_owned())
                .collect()
        }
        async fn close(self) {
            self.conn.shutdown().await;
            self.worker.await.unwrap();
        }
    }

    #[tokio::test]
    async fn subscriptions_are_lazy_and_only_owned_iframe_descendants_are_configured() {
        let wire = Wire::new(true, false, false).await;
        let mut routes = FrameRoutes::new(&wire.conn, ROOT);
        assert!(wire.calls.lock().unwrap().is_empty());
        let trees = routes.refresh(&wire.conn).await.unwrap();
        assert_eq!(trees.len(), 3);
        assert_eq!(routes.sessions(), vec!["iframe-b", "iframe-a", ROOT]);
        routes.disable(&wire.conn).await.unwrap();
        assert_eq!(wire.disable_calls(), vec!["iframe-b", "iframe-a", ROOT]);
        wire.close().await;
    }
    #[tokio::test]
    async fn child_sources_are_disabled_before_root_automatically_detaches_them() {
        let wire = Wire::new(true, true, false).await;
        let mut routes = FrameRoutes::new(&wire.conn, ROOT);
        routes.refresh(&wire.conn).await.unwrap();
        routes.disable(&wire.conn).await.unwrap();
        assert_eq!(wire.disable_calls(), vec!["iframe-b", "iframe-a", ROOT]);
        assert!(!wire.conn.registry().has_session("iframe-a"));
        assert!(!routes.root_configured);
        wire.close().await;
    }
    #[tokio::test]
    async fn failed_live_child_disable_is_not_success_and_is_retried_exactly() {
        let wire = Wire::new(false, false, true).await;
        let mut routes = FrameRoutes::new(&wire.conn, ROOT);
        routes.refresh(&wire.conn).await.unwrap();
        assert_eq!(
            routes.disable(&wire.conn).await,
            Err(Error::ExecutionFailed)
        );
        assert!(routes.routes["iframe-a"].configured);
        assert!(!routes.root_configured);
        routes.disable(&wire.conn).await.unwrap();
        assert_eq!(wire.disable_calls(), vec!["iframe-a", ROOT, "iframe-a"]);
        wire.close().await;
    }
    #[tokio::test]
    async fn exact_ancestor_detach_can_settle_an_earlier_child_disable_failure() {
        let wire = Wire::new(false, true, true).await;
        let mut routes = FrameRoutes::new(&wire.conn, ROOT);
        routes.refresh(&wire.conn).await.unwrap();
        routes.disable(&wire.conn).await.unwrap();
        assert!(routes.retired.contains("iframe-a"));
        assert!(!wire.conn.registry().has_session("iframe-a"));
        assert_eq!(wire.disable_calls(), vec!["iframe-a", ROOT]);
        wire.close().await;
    }
    #[tokio::test]
    async fn parent_detach_retires_descendants_and_session_reuse_fails_closed() {
        let wire = Wire::new(true, false, false).await;
        let mut routes = FrameRoutes::new(&wire.conn, ROOT);
        routes.refresh(&wire.conn).await.unwrap();
        wire.conn
            .registry()
            .dispatch_message(&detach(ROOT, "iframe-a").to_string())
            .unwrap();
        assert_eq!(routes.refresh(&wire.conn).await.unwrap().len(), 1);
        assert_eq!(routes.sessions(), vec![ROOT]);
        let before = wire.calls.lock().unwrap().len();
        let _ = wire
            .conn
            .registry()
            .dispatch_message(&attach(ROOT, "iframe-a", "frame-a", "iframe").to_string());
        assert!(routes.refresh(&wire.conn).await.is_err());
        assert_eq!(
            wire.calls.lock().unwrap().len(),
            before,
            "a reused session must not receive any command"
        );
        wire.close().await;
    }
    #[tokio::test]
    async fn empty_root_and_owned_route_overflow_never_expand_authority() {
        let wire = Wire::new(false, false, false).await;
        let mut invalid = FrameRoutes::new(&wire.conn, "");
        assert_eq!(
            invalid.refresh(&wire.conn).await,
            Err(Error::ExecutionFailed)
        );
        drop(invalid);
        let mut routes = FrameRoutes::new(&wire.conn, ROOT);
        for index in 0..=MAX_SESSIONS {
            wire.conn
                .registry()
                .dispatch_message(
                    &attach(ROOT, &format!("s{index}"), &format!("f{index}"), "iframe").to_string(),
                )
                .unwrap();
        }
        assert_eq!(
            routes.refresh(&wire.conn).await,
            Err(Error::ExecutionFailed)
        );
        assert!(wire.calls.lock().unwrap().is_empty());
        wire.close().await;
    }
}
