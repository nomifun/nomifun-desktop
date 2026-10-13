//! Website media decisions remain owned until an exact human reply or denial.
//! WebKit's native Prompt cannot be revoked when the active input owner changes.
use crate::engine::Page;
use block2::{DynBlock, RcBlock};
use nomifun_browser_platform::runtime::BrowserPermissionRequest;
use objc2_web_kit::{WKMediaCaptureType, WKPermissionDecision, WKSecurityOrigin};
use std::{cell::RefCell, collections::{BTreeMap, BTreeSet}, sync::{Weak, atomic::Ordering}, time::Duration};

const MAX_PENDING: usize = 4;
const MAX_CANCELLED: usize = 64;
const REQUEST_LIFETIME: Duration = Duration::from_secs(30);

struct Pending {
    snapshot: BrowserPermissionRequest,
    generation: u64,
    completion: RcBlock<dyn Fn(WKPermissionDecision)>,
}
impl Pending {
    fn finish(self, allow: bool) {
        self.completion.call((if allow { WKPermissionDecision::Grant } else { WKPermissionDecision::Deny },));
    }
}
#[derive(Default)]
struct State {
    pending: BTreeMap<String, Pending>,
    generation: u64,
    cancelled: BTreeSet<(String, String)>,
}
impl State {
    fn document(&mut self, generation: u64) {
        if self.generation != generation {
            self.generation = generation;
            self.cancelled.clear();
        }
    }
    fn cancelled(&mut self, generation: u64, origin: &str, kind: &str) -> bool {
        self.document(generation);
        self.cancelled.len() >= MAX_CANCELLED || self.cancelled.contains(&(origin.into(), kind.into()))
    }
    fn cancel(&mut self, pending: &Pending) {
        self.document(pending.generation);
        if self.cancelled.len() < MAX_CANCELLED {
            self.cancelled.insert((pending.snapshot.origin.clone(), pending.snapshot.kind.clone()));
        }
    }
}
pub(crate) struct Permissions {
    page: Weak<Page>,
    state: RefCell<State>,
}
impl Permissions {
    pub(crate) fn new(page: Weak<Page>) -> Self { Self { page, state: RefCell::new(State::default()) } }
    fn user_page(&self) -> Option<std::sync::Arc<Page>> {
        self.page.upgrade().filter(|page| !page.close_requested.load(Ordering::Acquire)
            && !page.input_locked() && !page.dialog_draining() && page.is_visible())
    }
    fn publish(&self) {
        if let Some(page) = self.page.upgrade() {
            let requests = self.state.borrow().pending.values().map(|pending| pending.snapshot.clone()).collect();
            page.changed(|snapshot| snapshot.permission_requests = requests);
        }
    }
    fn record_denial(&self, kind: &str) {
        if let Some(page) = self.page.upgrade() {
            page.changed(|snapshot| {
                if !snapshot.blocked_permissions.iter().any(|value| value == kind) { snapshot.blocked_permissions.push(kind.into()); }
            });
        }
    }
    fn record_pending_denial(&self, pending: &Pending) {
        // Navigation may already have published the successor document. An
        // old request must not attach its blocked hint to that new document.
        if self.page.upgrade().is_some_and(|page| page.snapshot().document_generation == pending.generation) {
            self.record_denial(&pending.snapshot.kind);
        }
    }
    fn clear_granted_denial(&self, pending: &Pending) {
        if let Some(page) = self.page.upgrade() {
            page.changed(|snapshot| {
                // A retained reply belongs to its original document. It may
                // never erase a successor's independent permission denial.
                if snapshot.document_generation == pending.generation {
                    snapshot.blocked_permissions.retain(|kind| kind != &pending.snapshot.kind);
                }
            });
        }
    }
    pub(crate) fn offer(&self, origin: &WKSecurityOrigin, kind: WKMediaCaptureType, completion: &DynBlock<dyn Fn(WKPermissionDecision)>) {
        let kind = media_kind(kind);
        let Some(page) = self.user_page().filter(|_| kind != "other") else {
            self.record_denial(kind);
            completion.call((WKPermissionDecision::Deny,));
            return;
        };
        let origin = unsafe { media_origin(&origin.protocol().to_string(), &origin.host().to_string(), origin.port()) };
        let Some(origin) = origin else {
            self.record_denial(kind);
            completion.call((WKPermissionDecision::Deny,));
            return;
        };
        let generation = page.snapshot().document_generation;
        {
            let mut state = self.state.borrow_mut();
            if state.pending.len() >= MAX_PENDING || state.cancelled(generation, &origin, kind) {
                drop(state);
                self.record_denial(kind);
                completion.call((WKPermissionDecision::Deny,));
                return;
            }
        }
        let id = uuid::Uuid::now_v7().to_string();
        self.state.borrow_mut().pending.insert(id.clone(), Pending {
            snapshot: BrowserPermissionRequest { request_id: id.clone(), kind: kind.into(), origin },
            generation,
            completion: completion.copy(),
        });
        self.publish();
        let page_id = page.id();
        let when = dispatch2::DispatchTime::try_from(REQUEST_LIFETIME).expect("bounded permission lifetime");
        let _ = dispatch2::DispatchQueue::main().after(when, move || {
            if let Some(native) = crate::engine::native_page(page_id) { native.interactions.permissions.expire(&id); }
        });
    }
    pub(crate) fn reply(&self, id: &str, generation: u64, allow: bool) -> Result<(), String> {
        let page = self.user_page().ok_or("BROWSER_ELEMENT_NOT_ACTIONABLE")?;
        let pending = {
            let mut state = self.state.borrow_mut();
            let pending = state.pending.get(id).ok_or("BROWSER_STALE_OBSERVATION")?;
            if pending.generation != generation || page.snapshot().document_generation != generation { return Err("BROWSER_STALE_OBSERVATION".into()); }
            state.pending.remove(id).expect("checked permission")
        };
        self.publish();
        if allow { self.clear_granted_denial(&pending); }
        else { self.record_pending_denial(&pending); }
        pending.finish(allow);
        Ok(())
    }
    fn expire(&self, id: &str) {
        let pending = self.state.borrow_mut().pending.remove(id);
        if let Some(pending) = pending {
            self.state.borrow_mut().cancel(&pending);
            self.publish();
            self.record_pending_denial(&pending);
            pending.finish(false);
        }
    }
    pub(crate) fn drain(&self) {
        let pending = std::mem::take(&mut self.state.borrow_mut().pending);
        if pending.is_empty() { return; }
        for request in pending.values() { self.state.borrow_mut().cancel(request); }
        self.publish();
        for request in pending.into_values() {
            self.record_pending_denial(&request);
            request.finish(false);
        }
    }
}
impl Drop for Permissions { fn drop(&mut self) { self.drain(); } }

fn media_kind(kind: WKMediaCaptureType) -> &'static str {
    match kind {
        WKMediaCaptureType::Camera => "camera",
        WKMediaCaptureType::Microphone => "microphone",
        WKMediaCaptureType::CameraAndMicrophone => "camera_microphone",
        _ => "other",
    }
}
fn media_origin(scheme: &str, host: &str, port: isize) -> Option<String> {
    if !matches!(scheme, "http" | "https") || host.is_empty() || host.len() > 4096 || !(0..=65535).contains(&port) { return None; }
    let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.into() };
    let port = if port == 0 { String::new() } else { format!(":{port}") };
    let url = url::Url::parse(&format!("{scheme}://{host}{port}/")).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.path() != "/" || url.query().is_some() || url.fragment().is_some() { return None; }
    Some(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc, sync::Arc};
    fn pending(page: &Arc<Page>, owner: &Permissions, id: &str, answer: Rc<Cell<Option<bool>>>) {
        let completion = RcBlock::new(move |value| answer.set(Some(value == WKPermissionDecision::Grant)));
        owner.state.borrow_mut().pending.insert(id.into(), Pending {
            snapshot: BrowserPermissionRequest { request_id: id.into(), kind: "camera".into(), origin: "https://fixture.invalid".into() },
            generation: page.snapshot().document_generation,
            completion,
        });
        owner.publish();
    }
    #[test]
    fn exact_human_reply_is_required_and_consumed_once() {
        let page = Page::test_user_surface(true, true, false, false);
        let owner = Permissions::new(Arc::downgrade(&page));
        let answer = Rc::new(Cell::new(None));
        pending(&page, &owner, "request", answer.clone());
        assert!(owner.reply("request", page.snapshot().document_generation + 1, true).is_err());
        assert_eq!(answer.get(), None);
        assert_eq!(page.snapshot().permission_requests.len(), 1);
        owner.reply("request", page.snapshot().document_generation, true).unwrap();
        assert_eq!(answer.get(), Some(true));
        assert!(page.snapshot().permission_requests.is_empty());
        assert!(owner.reply("request", page.snapshot().document_generation, true).is_err());
    }
    #[test]
    fn hidden_locked_draining_and_closed_pages_cannot_grant() {
        for (visible, locked, draining, closed) in [(false,false,false,false),(true,true,false,false),(true,false,true,false),(true,false,false,true)] {
            let page = Page::test_user_surface(true, visible, locked, draining);
            let owner = Permissions::new(Arc::downgrade(&page));
            let answer = Rc::new(Cell::new(None));
            pending(&page, &owner, "request", answer.clone());
            page.close_requested.store(closed, Ordering::Release);
            assert!(owner.reply("request", page.snapshot().document_generation, true).is_err());
            owner.drain();
            assert_eq!(answer.get(), Some(false));
            assert!(page.snapshot().permission_requests.is_empty());
        }
    }
    #[test]
    fn expiry_and_drain_deny_once_and_prevent_same_document_reprompt() {
        let page = Page::test_user_surface(true, true, false, false);
        let owner = Permissions::new(Arc::downgrade(&page));
        let answer = Rc::new(Cell::new(None));
        pending(&page, &owner, "request", answer.clone());
        owner.expire("request");
        owner.drain();
        assert_eq!(answer.get(), Some(false));
        let generation = page.snapshot().document_generation;
        assert!(owner.state.borrow_mut().cancelled(generation, "https://fixture.invalid", "camera"));
        assert!(!owner.state.borrow_mut().cancelled(generation + 1, "https://fixture.invalid", "camera"));
        assert!(page.snapshot().blocked_permissions.contains(&"camera".into()));
    }
    #[test]
    fn cancelled_scope_is_bounded_and_origin_contains_no_page_data() {
        let mut state = State::default();
        for i in 0..MAX_CANCELLED { state.cancelled.insert((format!("https://{i}.invalid"), "camera".into())); }
        assert!(state.cancelled(0, "https://other.invalid", "microphone"));
        assert!(!state.cancelled(1, "https://other.invalid", "microphone"));
        assert_eq!(media_origin("https", "example.test", 443).as_deref(), Some("https://example.test"));
        assert_eq!(media_origin("http", "::1", 5173).as_deref(), Some("http://[::1]:5173"));
        for (scheme, host, port) in [("file", "example.test", 0),("https", "user:secret@example.test", 0),("https", "example.test/private", 0),("https", "example.test", -1)] { assert!(media_origin(scheme, host, port).is_none()); }
        assert_eq!(media_kind(WKMediaCaptureType(999)), "other");
    }
    #[test]
    fn old_request_denial_does_not_mark_the_successor_document() {
        let page = Page::test_user_surface(true, true, false, false);
        let owner = Permissions::new(Arc::downgrade(&page));
        let answer = Rc::new(Cell::new(None));
        pending(&page, &owner, "request", answer.clone());
        page.changed(|snapshot| { snapshot.document_generation += 1; snapshot.permission_requests.clear(); });
        owner.drain();
        assert_eq!(answer.get(), Some(false));
        assert!(page.snapshot().permission_requests.is_empty());
        assert!(page.snapshot().blocked_permissions.is_empty());
    }
    #[test]
    fn current_grant_clears_only_its_permission_kind() {
        let page = Page::test_user_surface(true, true, false, false);
        let owner = Permissions::new(Arc::downgrade(&page));
        let answer = Rc::new(Cell::new(None));
        page.changed(|snapshot| snapshot.blocked_permissions = vec!["camera".into(), "microphone".into()]);
        pending(&page, &owner, "request", answer.clone());
        owner.reply("request", page.snapshot().document_generation, true).unwrap();
        assert_eq!(answer.get(), Some(true));
        assert_eq!(page.snapshot().blocked_permissions, vec!["microphone"]);
    }
    #[test]
    fn old_grant_and_reply_cannot_clear_a_successor_document_denial() {
        let page = Page::test_user_surface(true, true, false, false);
        let owner = Permissions::new(Arc::downgrade(&page));
        let answer = Rc::new(Cell::new(None));
        pending(&page, &owner, "request", answer.clone());
        let generation = page.snapshot().document_generation;
        page.changed(|snapshot| { snapshot.document_generation += 1; snapshot.permission_requests.clear(); snapshot.blocked_permissions = vec!["camera".into()]; });
        assert!(owner.reply("request", generation, true).is_err());
        assert_eq!(answer.get(), None);
        let old = owner.state.borrow_mut().pending.remove("request").unwrap();
        owner.clear_granted_denial(&old);
        assert_eq!(page.snapshot().blocked_permissions, vec!["camera"]);
        old.finish(false);
        assert_eq!(answer.get(), Some(false));
        assert_eq!(page.snapshot().blocked_permissions, vec!["camera"]);
    }
}
