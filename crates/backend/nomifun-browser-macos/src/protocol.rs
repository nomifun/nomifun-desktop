//! Bounded, in-process DevTools messages for one owned CEF page.
//! No listening port, browser discovery, page-accessible bridge, or global input.

use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicI32, Ordering}}, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

const MAX_MESSAGE: usize = 32 * 1024 * 1024;
const MAX_PENDING: usize = 64;
const MAX_SUBSCRIPTIONS: usize = 64;

#[derive(Clone, Debug)]
pub struct Event {
    pub method: String,
    pub session: Option<String>,
    pub params: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Arc<Protocol>, mpsc::UnboundedReceiver<Value>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let protocol = Protocol::new(Box::new(move |command| {
            tx.send(serde_json::from_slice(&command.bytes).unwrap()).map_err(|_| "test transport closed".into())
        }));
        (protocol, rx)
    }

    #[tokio::test]
    async fn out_of_order_replies_keep_their_owned_request_and_session() {
        let (protocol, mut sent) = fixture();
        let first = { let p = protocol.clone(); tokio::spawn(async move { p.call(Some("frame-a"), "Runtime.evaluate", json!({})).await }) };
        let a = sent.recv().await.unwrap();
        let second = { let p = protocol.clone(); tokio::spawn(async move { p.call(None, "Page.getFrameTree", json!({})).await }) };
        let b = sent.recv().await.unwrap();
        assert_eq!(a["sessionId"], "frame-a");
        assert!(b.get("sessionId").is_none());
        protocol.receive(&serde_json::to_vec(&json!({"id":b["id"],"result":{"value":"second"}})).unwrap());
        protocol.receive(&serde_json::to_vec(&json!({"id":a["id"],"sessionId":"frame-a","result":{"value":"first"}})).unwrap());
        assert_eq!(first.await.unwrap().unwrap()["value"], "first");
        assert_eq!(second.await.unwrap().unwrap()["value"], "second");
    }

    #[tokio::test]
    async fn native_close_rejects_pending_work_and_invalidates_event_owners() {
        let (protocol, mut sent) = fixture();
        let mut events = protocol.subscribe(&["Page.frameNavigated"]).unwrap();
        let call = { let p = protocol.clone(); tokio::spawn(async move { p.call(None, "Input.dispatchMouseEvent", json!({})).await }) };
        sent.recv().await.unwrap();
        protocol.close();
        assert!(call.await.unwrap().is_err());
        assert!(events.failed.load(Ordering::Acquire));
        assert!(events.receiver.recv().await.is_none());
        assert!(protocol.call(None, "Page.navigate", json!({})).await.is_err());
        assert!(protocol.subscribe(&["Page.frameNavigated"]).is_err());
    }

    #[tokio::test]
    async fn event_overflow_is_explicit_instead_of_silently_losing_frame_changes() {
        let (protocol, _) = fixture();
        let events = protocol.subscribe(&["Page.frameNavigated"]).unwrap();
        for generation in 0..129 {
            protocol.receive(&serde_json::to_vec(&json!({"method":"Page.frameNavigated","params":{"generation":generation}})).unwrap());
        }
        assert!(events.failed.load(Ordering::Acquire));
        assert!(!protocol.is_closed(), "an independent subscription is not invalidated by another owner's queue");
    }

    #[tokio::test]
    async fn malformed_native_messages_fail_closed_without_exposing_page_text() {
        let (protocol, mut sent) = fixture();
        let call = { let p = protocol.clone(); tokio::spawn(async move { p.call(None, "Runtime.evaluate", json!({})).await }) };
        sent.recv().await.unwrap();
        protocol.receive(b"not protocol JSON: private page text");
        let error = call.await.unwrap().unwrap_err();
        assert!(!error.contains("private page text"));
        assert!(protocol.is_closed());
    }

    #[tokio::test]
    async fn wrong_frame_reply_invalidates_the_entire_page() {
        let (protocol, mut sent) = fixture();
        let call = { let p = protocol.clone(); tokio::spawn(async move { p.call(Some("frame-a"), "Runtime.evaluate", json!({})).await }) };
        let command = sent.recv().await.unwrap();
        protocol.receive(&serde_json::to_vec(&json!({"id":command["id"],"sessionId":"frame-b","result":{}})).unwrap());
        assert!(call.await.unwrap().is_err());
        assert!(protocol.is_closed());
    }

    #[tokio::test]
    async fn native_dispatch_rejection_does_not_destroy_other_requests() {
        let (protocol, mut sent) = fixture();
        let denied = { let p = protocol.clone(); tokio::spawn(async move { p.call(None, "DOM.setFileInputFiles", json!({})).await }) };
        let command = sent.recv().await.unwrap();
        protocol.reject(command["id"].as_i64().unwrap() as i32);
        assert!(denied.await.unwrap().is_err());
        assert!(!protocol.is_closed());
        let next = { let p = protocol.clone(); tokio::spawn(async move { p.call(None, "Page.getFrameTree", json!({})).await }) };
        let command = sent.recv().await.unwrap();
        protocol.receive(&serde_json::to_vec(&json!({"id":command["id"],"result":{}})).unwrap());
        assert!(next.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn modal_wait_does_not_exhaust_native_command_deadline() {
        let (protocol, _) = fixture();
        protocol.set_modal_pending(true);
        let (tx, rx) = oneshot::channel();
        let wait = protocol.await_settlement(rx, Duration::from_millis(10));
        tokio::pin!(wait);
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut wait).await.is_err());
        tx.send(42).unwrap();
        assert_eq!(wait.await.unwrap().unwrap(), 42);
        protocol.set_modal_pending(false);
        let (_tx, rx) = oneshot::channel::<()>();
        assert!(protocol.await_settlement(rx, Duration::ZERO).await.is_err());
    }

    #[test]
    fn frame_callbacks_are_delivered_before_the_native_reply_barrier() {
        let (protocol, _) = fixture();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let output = seen.clone();
        let _listener = protocol.subscribe_callback(&["Target.attachedToTarget"], Arc::new(move |event| {
            output.lock().unwrap().push(event.params["sessionId"].as_str().unwrap().to_owned()); true
        })).unwrap();
        protocol.receive(br#"{"method":"Target.attachedToTarget","params":{"sessionId":"child"}}"#);
        assert_eq!(*seen.lock().unwrap(), ["child"]);
    }
}

struct Subscriber {
    methods: Vec<String>,
    delivery: Delivery,
    failed: Arc<AtomicBool>,
}

#[derive(Clone)]
enum Delivery {
    Queue(mpsc::Sender<Event>),
    Callback(Arc<dyn Fn(Event) -> bool + Send + Sync>),
}

struct Pending {
    session: Option<String>,
    sender: oneshot::Sender<Result<Value, String>>,
}

#[derive(Default)]
struct State {
    pending: BTreeMap<i32, Pending>,
    subscribers: BTreeMap<uuid::Uuid, Subscriber>,
}

/// A full/closed event queue invalidates its owner instead of silently losing
/// document, frame, dialog or destruction events.
pub struct Subscription {
    id: uuid::Uuid,
    owner: Arc<Protocol>,
    pub receiver: mpsc::Receiver<Event>,
    pub failed: Arc<AtomicBool>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.failed.store(true, Ordering::Release);
        self.owner.state.lock().unwrap().subscribers.remove(&self.id);
    }
}

pub struct CallbackSubscription {
    id: uuid::Uuid,
    owner: Arc<Protocol>,
    pub failed: Arc<AtomicBool>,
}
impl Drop for CallbackSubscription {
    fn drop(&mut self) {
        self.failed.store(true, Ordering::Release);
        self.owner.state.lock().unwrap().subscribers.remove(&self.id);
    }
}

/// The sender only queues work on the browser UI thread. It must reject a
/// closed page; it cannot select another page or execute page JavaScript.
pub type DispatchGuard = dyn Fn() -> bool + Send + Sync;
pub struct Command {
    pub id: i32,
    pub bytes: Vec<u8>,
    /// Host-owned authority, checked on the native UI thread before sending.
    pub guard: Option<Arc<DispatchGuard>>,
}
pub type SendMessage = dyn Fn(Command) -> Result<(), String> + Send + Sync;

pub struct Protocol {
    state: Mutex<State>,
    next: AtomicI32,
    closed: AtomicBool,
    send: Box<SendMessage>,
    modal: watch::Sender<bool>,
}

impl Protocol {
    pub fn new(send: Box<SendMessage>) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State::default()), next: AtomicI32::new(1), closed: AtomicBool::new(false), modal: watch::channel(false).0, send })
    }

    pub fn subscribe(self: &Arc<Self>, methods: &[&str]) -> Result<Subscription, String> {
        let mut state = self.state.lock().unwrap();
        if self.closed.load(Ordering::Acquire) || state.subscribers.len() >= MAX_SUBSCRIPTIONS {
            return Err("CEF page event subscription is unavailable".into());
        }
        let id = uuid::Uuid::now_v7();
        let (sender, receiver) = mpsc::channel(128);
        let failed = Arc::new(AtomicBool::new(false));
        state.subscribers.insert(id, Subscriber { methods: methods.iter().map(|s| (*s).into()).collect(), delivery: Delivery::Queue(sender), failed: failed.clone() });
        Ok(Subscription { id, owner: self.clone(), receiver, failed })
    }

    /// Direct non-blocking forwarding preserves ordering with native replies.
    /// Used by frame routing's existing event/barrier queue; no extra async hop.
    pub fn subscribe_callback(self: &Arc<Self>, methods: &[&str], callback: Arc<dyn Fn(Event) -> bool + Send + Sync>) -> Result<CallbackSubscription, String> {
        let mut state = self.state.lock().unwrap();
        if self.closed.load(Ordering::Acquire) || state.subscribers.len() >= MAX_SUBSCRIPTIONS { return Err("CEF page event subscription is unavailable".into()); }
        let id = uuid::Uuid::now_v7();
        let failed = Arc::new(AtomicBool::new(false));
        state.subscribers.insert(id, Subscriber { methods: methods.iter().map(|s| (*s).into()).collect(), delivery: Delivery::Callback(callback), failed: failed.clone() });
        Ok(CallbackSubscription { id, owner: self.clone(), failed })
    }

    pub async fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        self.call_guarded(session, method, params, None).await
    }

    pub async fn call_guarded(&self, session: Option<&str>, method: &str, params: Value, guard: Option<Arc<DispatchGuard>>) -> Result<Value, String> {
        let id = self.next.fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| "CEF protocol request sequence exhausted")?;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session { message["sessionId"] = session.into(); }
        let bytes = serde_json::to_vec(&message).map_err(|_| "CEF protocol request encoding failed")?;
        if bytes.len() > MAX_MESSAGE { return Err("CEF protocol request exceeds its limit".into()); }
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.lock().unwrap();
            if self.closed.load(Ordering::Acquire) || state.pending.len() >= MAX_PENDING {
                return Err("CEF page is closed or has too many pending commands".into());
            }
            state.pending.insert(id, Pending { session: session.map(str::to_owned), sender: tx });
        }
        if let Err(error) = (self.send)(Command { id, bytes, guard }) {
            self.state.lock().unwrap().pending.remove(&id);
            return Err(error);
        }
        match self.await_settlement(rx, Duration::from_secs(30)).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("CEF page closed before the command settled".into()),
            Err(_) => {
                // Settlement is unknown. Invalidate the entire channel so the
                // runtime cannot issue more actions or claim cleanup succeeded.
                self.close();
                Err(format!("CEF protocol {method} settlement timed out; rebuild the browser"))
            }
        }
    }

    pub(crate) fn set_modal_pending(&self, pending: bool) { self.modal.send_replace(pending); }

    // Website dialogs suspend the renderer's original command. Time spent
    // awaiting its explicit response must not expire a healthy page protocol.
    async fn await_settlement<T>(&self, mut rx: oneshot::Receiver<T>, mut remaining: Duration) -> Result<Result<T, oneshot::error::RecvError>, ()> {
        let mut modal = self.modal.subscribe();
        loop {
            let paused = *modal.borrow_and_update();
            if paused {
                tokio::select! { biased;
                    result = &mut rx => return Ok(result),
                    result = modal.changed() => { if result.is_err() { return Err(()); } }
                }
            } else {
                let start = tokio::time::Instant::now();
                tokio::select! { biased;
                    result = &mut rx => return Ok(result),
                    result = modal.changed() => { if result.is_err() { return Err(()); } }
                    _ = tokio::time::sleep(remaining) => return Err(()),
                }
                remaining = remaining.saturating_sub(start.elapsed());
            }
        }
    }

    /// Called exclusively by the owned page's CEF observer. Oversized or
    /// malformed messages fail closed; errors do not expose page-controlled text.
    pub fn receive(&self, bytes: &[u8]) {
        if self.closed.load(Ordering::Acquire) { return; }
        if bytes.len() > MAX_MESSAGE { self.close(); return; }
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else { self.close(); return; };
        let mut state = self.state.lock().unwrap();
        if let Some(id) = value.get("id").and_then(Value::as_i64).and_then(|id| i32::try_from(id).ok()) {
            if let Some(pending) = state.pending.remove(&id) {
                if value.get("sessionId").and_then(Value::as_str) != pending.session.as_deref() {
                    let _ = pending.sender.send(Err("CEF protocol reply belongs to another frame session".into()));
                    drop(state);
                    self.close();
                    return;
                }
                let result = if value.get("error").is_some() { Err("CEF rejected the page protocol command".into()) }
                    else { Ok(value.get("result").cloned().unwrap_or(Value::Null)) };
                let _ = pending.sender.send(result);
            }
        } else if let Some(method) = value.get("method").and_then(Value::as_str) {
            let event = Event { method: method.into(), session: value.get("sessionId").and_then(Value::as_str).map(str::to_owned), params: value.get("params").cloned().unwrap_or(Value::Null) };
            let deliveries: Vec<_> = state.subscribers.iter().filter(|(_, subscriber)| subscriber.methods.iter().any(|candidate| candidate == method))
                .map(|(id, subscriber)| (*id, subscriber.delivery.clone(), subscriber.failed.clone())).collect();
            drop(state);
            for (id, delivery, failed) in deliveries {
                let accepted = match delivery { Delivery::Queue(sender) => sender.try_send(event.clone()).is_ok(), Delivery::Callback(callback) => callback(event.clone()) };
                if !accepted { failed.store(true, Ordering::Release); self.state.lock().unwrap().subscribers.remove(&id); }
            }
        }
    }

    pub fn is_closed(&self) -> bool { self.closed.load(Ordering::Acquire) }

    pub fn reject(&self, id: i32) {
        if let Some(pending) = self.state.lock().unwrap().pending.remove(&id) {
            let _ = pending.sender.send(Err("CEF command authority changed before native dispatch".into()));
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let mut state = self.state.lock().unwrap();
        for (_, subscriber) in std::mem::take(&mut state.subscribers) { subscriber.failed.store(true, Ordering::Release); }
        for (_, pending) in std::mem::take(&mut state.pending) { let _ = pending.sender.send(Err("CEF page protocol is closed".into())); }
    }
}
