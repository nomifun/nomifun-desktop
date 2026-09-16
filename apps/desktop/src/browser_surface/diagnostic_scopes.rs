//! Private diagnostic provenance. Observes the existing auto-attach stream;
//! never discovers targets, attaches sessions, or executes page code.
use super::*;
use std::collections::BTreeSet;

const MAX_SESSIONS: usize = 32;
const MAX_CONTEXTS: usize = 256;
const MAX_FRAMES: usize = 256;

struct Route { parent: String, frame: String, depth: usize }
struct Context { unique: String, frame: String }
struct Frame { loader: String, parent: Option<String> }

#[derive(Default)]
pub(super) struct ScopedProjection {
    generation: u64,
    routes: HashMap<String, Route>,
    contexts: HashMap<(String, i64), Context>,
    frames: HashMap<(String, String), Frame>,
    observed_frames: BTreeSet<(String, String)>,
    fenced: bool,
    projection: Projection,
}

fn id(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| !id.is_empty() && id.len() <= 256)
}

impl ScopedProjection {
    fn forget_sessions(&mut self, removed: &BTreeSet<String>) {
        self.routes.retain(|session, _| !removed.contains(session));
        self.contexts.retain(|(session, _), _| !removed.contains(session));
        self.frames.retain(|(session, _), _| !removed.contains(session));
        self.observed_frames.retain(|(session, _)| !removed.contains(session));
        self.projection.requests.retain(|(session, _), _| !removed.contains(session));
    }

    fn forget_frame(&mut self, session: &str, frame: &str) {
        let mut removed = BTreeSet::from([frame.to_owned()]);
        loop {
            let before = removed.len();
            for ((owner, id), value) in &self.frames {
                if owner == session && value.parent.as_ref().is_some_and(|parent| removed.contains(parent)) {
                    removed.insert(id.clone());
                }
            }
            if before == removed.len() { break; }
        }
        for frame in &removed {
            let key=(session.to_owned(),frame.clone());
            if self.observed_frames.len()>=MAX_FRAMES && !self.observed_frames.contains(&key) {
                self.fenced=true;
                break;
            }
            self.observed_frames.insert(key);
        }
        self.contexts.retain(|(owner, _), context| owner != session || !removed.contains(&context.frame));
        self.frames.retain(|(owner, id), _| owner != session || !removed.contains(id));
        // A lifecycle boundary invalidates outstanding requests in that
        // protocol session. Never attribute a late failure to the next loader.
        self.projection.requests.retain(|(owner, _), _| owner != session);
    }

    pub(super) fn apply(&mut self, tab: &mut BrowserTabSnapshot, generation: u64, session: &str, method: &str, data: Value) {
        if self.fenced { tab.diagnostics.unavailable=true; return; }
        if generation != tab.target.document_generation { return; }
        if self.generation != generation {
            self.generation = generation;
            self.routes.clear(); self.contexts.clear(); self.frames.clear();
            self.observed_frames.clear();
            self.projection.requests.clear();
        }
        let parent_depth = if session.is_empty() { 0 } else {
            let Some(route) = self.routes.get(session) else { return; };
            route.depth
        };
        match method {
            "Nomi.frameTree" => {
                let mut pending = vec![&data["frameTree"]];
                let mut count = 0;
                while let Some(tree) = pending.pop() {
                    count += 1;
                    if count > MAX_FRAMES { tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1); break; }
                    let frame=&tree["frame"];
                    if let (Some(name),Some(loader))=(id(&frame["id"]),id(&frame["loaderId"])) {
                        let key=(session.to_owned(),name.to_owned());
                        // A tree response is an initial snapshot, never a
                        // later navigation. Do not resurrect a detached frame,
                        // roll back a loader, or clear newly observed contexts.
                        if !self.observed_frames.contains(&key) && !self.frames.contains_key(&key) && self.frames.len()<MAX_FRAMES {
                            self.frames.insert(key,Frame {loader:loader.into(),parent:id(&frame["parentId"]).map(str::to_owned)});
                        }
                    }
                    if let Some(children) = tree["childFrames"].as_array() { pending.extend(children); }
                }
                return;
            }
            "Target.attachedToTarget" => {
                if data["targetInfo"]["type"] != "iframe" { return; }
                let (Some(child), Some(frame)) = (id(&data["sessionId"]), id(&data["targetInfo"]["targetId"])) else { return; };
                if child == session || parent_depth >= 8 { return; }
                if let Some(existing) = self.routes.get(child) {
                    if existing.parent != session || existing.frame != frame {
                        self.fenced=true;
                        tab.diagnostics.unavailable=true;
                        tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
                    }
                    return;
                }
                if self.routes.len() >= MAX_SESSIONS { tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1); return; }
                self.routes.insert(child.into(), Route { parent: session.into(), frame: frame.into(), depth: parent_depth + 1 });
                return;
            }
            "Target.detachedFromTarget" => {
                let Some(child) = id(&data["sessionId"]) else { return; };
                if !self.routes.get(child).is_some_and(|route| route.parent == session) { return; }
                let mut removed = BTreeSet::from([child.to_owned()]);
                loop {
                    let before = removed.len();
                    for (session, route) in &self.routes {
                        if removed.contains(&route.parent) { removed.insert(session.clone()); }
                    }
                    if before == removed.len() { break; }
                }
                self.forget_sessions(&removed);
                return;
            }
            "Runtime.executionContextCreated" => {
                let context = &data["context"];
                if context["auxData"]["isDefault"] != true { return; }
                let (Some(number), Some(unique), Some(frame)) = (context["id"].as_i64(), id(&context["uniqueId"]), id(&context["auxData"]["frameId"])) else { return; };
                let key = (session.to_owned(), number);
                if self.contexts.len() >= MAX_CONTEXTS && !self.contexts.contains_key(&key) { tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1); return; }
                self.contexts.insert(key, Context { unique: unique.into(), frame: frame.into() });
                return;
            }
            "Runtime.executionContextDestroyed" => {
                if let Some(number) = data["executionContextId"].as_i64() {
                    let key = (session.to_owned(), number);
                    if self.contexts.get(&key).is_some_and(|context| data["executionContextUniqueId"].as_str().is_none_or(|unique| unique == context.unique)) {
                        self.contexts.remove(&key);
                    }
                }
                return;
            }
            "Runtime.executionContextsCleared" => {
                self.contexts.retain(|(owner, _), _| owner != session);
                self.projection.requests.retain(|(owner, _), _| owner != session);
                return;
            }
            "Page.frameNavigated" => {
                let frame = &data["frame"];
                let (Some(name), Some(loader)) = (id(&frame["id"]), id(&frame["loaderId"])) else { return; };
                let key=(session.to_owned(),name.to_owned());
                if self.observed_frames.len()>=MAX_FRAMES && !self.observed_frames.contains(&key) {
                    self.fenced=true;tab.diagnostics.unavailable=true;tab.diagnostics.dropped=tab.diagnostics.dropped.saturating_add(1);return;
                }
                self.observed_frames.insert(key);
                if self.frames.get(&(session.into(), name.into())).is_some_and(|frame| frame.loader == loader) { return; }
                self.forget_frame(session, name);
                tab.diagnostics.unavailable |= self.fenced;
                if self.frames.len() >= MAX_FRAMES { tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1); return; }
                self.frames.insert((session.into(), name.into()), Frame { loader: loader.into(), parent: id(&frame["parentId"]).map(str::to_owned) });
                return;
            }
            "Page.frameDetached" => {
                if let Some(frame) = id(&data["frameId"]) {
                    let key=(session.to_owned(),frame.to_owned());
                    if self.observed_frames.len()>=MAX_FRAMES && !self.observed_frames.contains(&key) {
                        self.fenced=true;tab.diagnostics.unavailable=true;tab.diagnostics.dropped=tab.diagnostics.dropped.saturating_add(1);return;
                    }
                    self.observed_frames.insert(key);
                    self.forget_frame(session, frame);
                    tab.diagnostics.unavailable |= self.fenced;
                }
                return;
            }
            "Runtime.consoleAPICalled" | "Runtime.exceptionThrown" => {
                let number = if method == "Runtime.consoleAPICalled" { data["executionContextId"].as_i64() }
                    else { data["exceptionDetails"]["executionContextId"].as_i64() };
                if !number.is_some_and(|number| self.contexts.contains_key(&(session.into(), number))) {
                    tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1); return;
                }
            }
            "Network.requestWillBeSent" => {
                let known = id(&data["frameId"]).and_then(|frame| self.frames.get(&(session.into(), frame.into())))
                    .is_some_and(|frame| data["loaderId"].as_str() == Some(frame.loader.as_str()));
                if !known {
                    tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1); return;
                }
            }
            _ => {}
        }
        self.projection.apply_scoped(tab, generation, session, method, data);
    }
}
