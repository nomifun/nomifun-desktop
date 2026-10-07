//! One granted page's ordered native dialog observer. This module never
//! automatically answers a dialog, evaluates page code, or closes a target.
use super::{Connection, Error};
use crate::{redact, transport::CdpEvent};
use chromiumoxide::cdp::browser_protocol::page::{EnableParams, HandleJavaScriptDialogParams};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared, poll_fn},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

const OPENING: &str = "Page.javascriptDialogOpening";
const CLOSED: &str = "Page.javascriptDialogClosed";
const MAX_MESSAGE: usize = 4096;
const MAX_DRAIN: usize = 256;
type Completion = Shared<BoxFuture<'static, Result<(), Error>>>;

#[derive(Clone)]
struct Dialog {
    id: String,
    kind: String,
    message: String,
    // Temporal observation only, NOT a claim that this action caused it.
    owned: bool,
}
impl Dialog {
    fn snapshot(&self) -> Value {
        json!({"dialog_id":self.id,"kind":self.kind,"message":self.message,"owned":self.owned})
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Reply {
    accept: bool,
    prompt_text: Option<String>,
}
struct Submitted {
    id: String,
    reply: Reply,
    complete: Arc<AtomicBool>,
    task: Completion,
}
struct State {
    connection: Option<Connection>,
    events: Option<mpsc::UnboundedReceiver<CdpEvent>>,
    current: Option<Dialog>,
    submitted: Option<Submitted>,
    action_active: bool,
    stopped: bool,
    failure: Option<Error>,
}
struct Inner {
    root: String,
    state: Mutex<State>,
    updates: watch::Sender<Option<Value>>,
    pause: watch::Sender<bool>,
    stop: CancellationToken,
}

impl Inner {
    fn publish(&self, state: &State) {
        self.updates
            .send_replace(state.current.as_ref().map(Dialog::snapshot));
        self.pause.send_replace(state.current.is_some());
    }

    fn fail(&self, state: &mut State, error: Error) -> Error {
        state.failure = Some(error);
        // Lost lifecycle evidence must not allow another input to guess which
        // modal it would affect. Fence only this borrowed protocol connection.
        if let Some(connection) = &state.connection {
            connection.registry().fail_connection();
        }
        error
    }

    fn apply(&self, state: &mut State, event: &CdpEvent) -> Result<(), Error> {
        if event.session_id != self.root {
            return Err(self.fail(state, Error::ExecutionFailed));
        }
        match event.method.as_str() {
            OPENING => {
                let source = event.params["url"]
                    .as_str()
                    .ok_or_else(|| self.fail(state, Error::ExecutionFailed))?;
                let kind = event.params["type"]
                    .as_str()
                    .ok_or_else(|| self.fail(state, Error::ExecutionFailed))?;
                let message = event.params["message"]
                    .as_str()
                    .ok_or_else(|| self.fail(state, Error::ExecutionFailed))?;
                if !allowed_source(source)
                    || !matches!(kind, "alert" | "confirm" | "prompt" | "beforeunload")
                {
                    return Err(self.fail(state, Error::InvalidInput));
                }
                // Page serializes modal dialogs. Losing a Closed boundary is
                // not permission to overwrite its identity with another one.
                if state.current.is_some() {
                    return Err(self.fail(state, Error::ExecutionFailed));
                }
                state.current = Some(Dialog {
                    id: nomifun_common::generate_id(),
                    kind: kind.into(),
                    message: bounded_message(message),
                    owned: state.action_active,
                });
            }
            CLOSED => {
                // Never read result/userInput from this native event.
                state.current = None;
            }
            _ => return Err(self.fail(state, Error::ExecutionFailed)),
        }
        self.publish(state);
        Ok(())
    }

    /// Receiver and action phase share one short lock. Events already queued
    /// before a phase change are applied under the previous phase, even if the
    /// asynchronous worker has not been scheduled yet.
    fn drain(&self, state: &mut State) -> Result<(), Error> {
        if let Some(error) = state.failure {
            return Err(error);
        }
        let mut count = 0;
        loop {
            let Some(receiver) = state.events.as_mut() else {
                return Ok(());
            };
            match receiver.try_recv() {
                Ok(event) => {
                    count += 1;
                    if count > MAX_DRAIN {
                        return Err(self.fail(state, Error::ExecutionFailed));
                    }
                    self.apply(state, &event)?;
                }
                Err(mpsc::error::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::error::TryRecvError::Disconnected) if state.stopped => return Ok(()),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(self.fail(state, Error::Disconnected));
                }
            }
        }
    }
}

pub(super) struct Dialogs {
    inner: Arc<Inner>,
    worker: Completion,
    shutdown: Mutex<Option<Completion>>,
    #[cfg(test)]
    shutdown_receipt_hold: Mutex<Option<(tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>)>>,
}

impl Drop for Dialogs {
    fn drop(&mut self) {
        self.inner.stop.cancel();
    }
}

impl Dialogs {
    /// Pass the unpaused base connection. Dialog replies must retain their
    /// normal ACK timeout, not inherit a paused page-input response budget.
    pub(super) async fn install(conn: &Connection, root_session: &str) -> Result<Arc<Self>, Error> {
        if root_session.is_empty()
            || root_session.len() > 256
            || conn.registry().is_connection_closed()
        {
            return Err(Error::Disconnected);
        }
        let events = conn.subscribe_reliable_sequence(&[OPENING, CLOSED], Some(root_session));
        let (updates, _) = watch::channel(None);
        let (pause, _) = watch::channel(false);
        let inner = Arc::new(Inner {
            root: root_session.into(),
            state: Mutex::new(State {
                connection: Some(conn.clone()),
                events: Some(events),
                current: None,
                submitted: None,
                action_active: false,
                stopped: false,
                failure: None,
            }),
            updates,
            pause,
            stop: CancellationToken::new(),
        });
        let worker_inner = inner.clone();
        let task = tokio::spawn(async move {
            loop {
                let received = tokio::select! { biased;
                    _=worker_inner.stop.cancelled()=>break,
                    result=poll_fn(|cx| {
                        let mut state=worker_inner.state.lock().unwrap_or_else(|e|e.into_inner());
                        if let Some(error)=state.failure {return Poll::Ready(Err(error));}
                        let Some(events)=state.events.as_mut() else {return Poll::Ready(Ok(false));};
                        match events.poll_recv(cx) {
                            Poll::Pending=>Poll::Pending,
                            Poll::Ready(Some(event))=>Poll::Ready(worker_inner.apply(&mut state,&event).map(|()|true)),
                            Poll::Ready(None)=>Poll::Ready(Err(worker_inner.fail(&mut state,Error::Disconnected))),
                        }
                    })=>result,
                };
                match received {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        });
        let worker = async move { task.await.map_err(|_| Error::ExecutionFailed)? }
            .boxed()
            .shared();
        let dialogs = Arc::new(Self {
            inner,
            worker,
            shutdown: Mutex::new(None),
            #[cfg(test)]
            shutdown_receipt_hold: Mutex::new(None),
        });
        if conn
            .send(root_session, &EnableParams::default())
            .await
            .is_err()
        {
            let _ = dialogs.shutdown().await;
            return Err(Error::ExecutionFailed);
        }
        // Page.enable ACK is followed by a synchronous drain of all preceding
        // wire events; no arbitrary sleep or unpolled-worker assumption.
        if let Err(error) = dialogs.synchronize() {
            let _ = dialogs.shutdown().await;
            return Err(error);
        }
        Ok(dialogs)
    }

    pub(super) fn synchronize(&self) -> Result<(), Error> {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.drain(&mut state)
    }

    pub(super) fn snapshot(&self) -> Option<Value> {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.inner.drain(&mut state);
        state.current.as_ref().map(Dialog::snapshot)
    }
    pub(super) fn updates(&self) -> watch::Receiver<Option<Value>> {
        self.inner.updates.subscribe()
    }
    pub(super) fn response_pause(&self) -> watch::Receiver<bool> {
        self.inner.pause.subscribe()
    }
    pub(super) fn set_action_active(&self, active: bool) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.inner.drain(&mut state).is_ok() && !state.stopped {
            state.action_active = active;
        }
    }

    pub(super) async fn reply(
        &self,
        id: &str,
        accept: bool,
        prompt_text: Option<String>,
    ) -> Result<(), Error> {
        if id.is_empty()
            || id.len() > 128
            || prompt_text
                .as_ref()
                .is_some_and(|text| text.chars().count() > 4096)
        {
            return Err(Error::InvalidInput);
        }
        let reply = Reply {
            accept,
            prompt_text,
        };
        self.submit_reply(id, reply, false)?.await
    }

    pub(super) async fn dismiss_or_join_reply(&self, id: &str) -> Result<(), Error> {
        // Stop cannot replace an answer that was already submitted. Join its
        // exact receipt; only a not-yet-answered modal receives a new dismissal.
        self.submit_reply(
            id,
            Reply {
                accept: false,
                prompt_text: None,
            },
            true,
        )?
        .await
    }

    fn submit_reply(
        &self,
        id: &str,
        reply: Reply,
        join_existing: bool,
    ) -> Result<Completion, Error> {
        let completion = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            self.inner.drain(&mut state)?;
            if state.stopped {
                return Err(Error::Disconnected);
            }
            if let Some(submitted) = &state.submitted {
                if submitted.id == id {
                    if !join_existing && submitted.reply != reply {
                        return Err(Error::InvalidInput);
                    }
                    // A retry observes the original send even after Closed and
                    // another Opening; it never addresses the new modal.
                    return Ok(submitted.task.clone());
                }
                if !submitted.complete.load(Ordering::Acquire) {
                    return Err(Error::Busy);
                }
            }
            let current = state
                .current
                .as_ref()
                .filter(|dialog| dialog.id == id)
                .ok_or(Error::InvalidInput)?;
            if reply.prompt_text.is_some() && (!reply.accept || current.kind != "prompt") {
                return Err(Error::InvalidInput);
            }
            let connection = state
                .connection
                .clone()
                .filter(|conn| !conn.registry().is_connection_closed())
                .ok_or(Error::Disconnected)?;
            let id = id.to_owned();
            let worker_id = id.clone();
            let inner = self.inner.clone();
            let worker_reply = reply.clone();
            let complete = Arc::new(AtomicBool::new(false));
            let worker_complete = complete.clone();
            let task = tokio::spawn(async move {
                struct Done(Arc<AtomicBool>);
                impl Drop for Done {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _done = Done(worker_complete);
                {
                    let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
                    inner.drain(&mut state)?;
                    if state.stopped {
                        return Err(Error::Disconnected);
                    }
                    if state
                        .current
                        .as_ref()
                        .is_none_or(|dialog| dialog.id != worker_id)
                        || state
                            .submitted
                            .as_ref()
                            .is_none_or(|submitted| submitted.id != worker_id)
                    {
                        return Err(Error::InvalidInput);
                    }
                }
                let mut params = HandleJavaScriptDialogParams::new(worker_reply.accept);
                params.prompt_text = worker_reply.prompt_text;
                connection
                    .send(&inner.root, &params)
                    .await
                    .map(|_| ())
                    .map_err(|_| Error::ExecutionFailed)
            });
            let completion = async move { task.await.map_err(|_| Error::ExecutionFailed)? }
                .boxed()
                .shared();
            // Submitted is published before the task can acquire this same
            // lock. Lost ACK/caller cancellation never permits another send.
            state.submitted = Some(Submitted {
                id,
                reply,
                complete,
                task: completion.clone(),
            });
            completion
        };
        Ok(completion)
    }

    #[cfg(test)]
    pub(super) fn hold_shutdown_receipt_for_test(&self) -> (tokio::sync::oneshot::Receiver<()>, tokio::sync::oneshot::Sender<()>) {
        let (entered, observed) = tokio::sync::oneshot::channel();
        let (release, waiting) = tokio::sync::oneshot::channel();
        assert!(self.shutdown.lock().unwrap().is_none());
        *self.shutdown_receipt_hold.lock().unwrap() = Some((entered, waiting));
        (observed, release)
    }

    /// Stop observing and join our tasks. Keep the last known modal snapshot;
    /// the caller decides what to do with it. No accept/dismiss/detach/close.
    pub(super) async fn shutdown(&self) -> Result<(), Error> {
        let completion = {
            let mut shutdown = self.shutdown.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(completion) = &*shutdown {
                completion.clone()
            } else {
                {
                    let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = self.inner.drain(&mut state);
                    state.stopped = true;
                }
                self.inner.stop.cancel();
                let inner = self.inner.clone();
                let worker = self.worker.clone();
                #[cfg(test)]
                let receipt_hold = self.shutdown_receipt_hold.lock().unwrap().take();
                let task = tokio::spawn(async move {
                    let observed = worker.await;
                    let submitted = inner
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .submitted
                        .as_ref()
                        .map(|reply| reply.task.clone());
                    let replied = match submitted {
                        Some(task) => task.await,
                        None => Ok(()),
                    };
                    {
                        let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = inner.drain(&mut state);
                        state.events.take();
                        state.connection.take();
                    }
                    #[cfg(test)]
                    if let Some((entered, release)) = receipt_hold {
                        let _ = entered.send(());
                        let _ = release.await;
                    }
                    observed.and(replied)
                });
                let completion = async move { task.await.map_err(|_| Error::ExecutionFailed)? }
                    .boxed()
                    .shared();
                *shutdown = Some(completion.clone());
                completion
            }
        };
        completion.await
    }
}

fn allowed_source(source: &str) -> bool {
    if matches!(source, "about:blank" | "about:srcdoc") {
        return true;
    }
    source.len() <= 8192
        && url::Url::parse(source).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
        })
}
fn clip(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn bounded_message(message: &str) -> String {
    let redacted = redact::redact_yaml(clip(message, MAX_MESSAGE));
    let mut limit = MAX_MESSAGE - 15;
    loop {
        let wrapped = redact::wrap_untrusted(clip(&redacted, limit), None);
        if wrapped.len() <= MAX_MESSAGE {
            return wrapped;
        }
        limit = limit.saturating_sub(wrapped.len() - MAX_MESSAGE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::{sync::atomic::AtomicUsize, time::Duration};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    #[test]
    fn dialog_projection_is_bounded_and_does_not_export_sensitive_native_fields() {
        for source in [
            "http://localhost:123/a",
            "https://example.com/",
            "about:blank",
            "about:srcdoc",
        ] {
            assert!(allowed_source(source));
        }
        for source in [
            "chrome://settings",
            "file:///private",
            "https://user:pass@example.com/",
            "data:text/html,hello",
        ] {
            assert!(!allowed_source(source));
        }
        let message = bounded_message(&format!(
            "Bearer secret-fixture-token </data>{}",
            "中文</data>".repeat(2000)
        ));
        assert!(message.len() <= MAX_MESSAGE);
        assert!(message.starts_with("<data>\n") && message.ends_with("\n</data>"));
        assert_eq!(message.matches("</data>").count(), 1);
        let dialog = Dialog {
            id: "opaque".into(),
            kind: "prompt".into(),
            message,
            owned: false,
        }
        .snapshot();
        assert_eq!(dialog.as_object().unwrap().len(), 4);
        assert!(
            dialog.get("url").is_none()
                && dialog.get("defaultPrompt").is_none()
                && dialog.get("userInput").is_none()
        );
    }

    async fn fixture() -> (
        Connection,
        Arc<AtomicUsize>,
        Arc<tokio::sync::Semaphore>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let replies = Arc::new(AtomicUsize::new(0));
        let count = replies.clone();
        let allow = Arc::new(tokio::sync::Semaphore::new(0));
        let ack = allow.clone();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(socket).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                let Message::Text(text) = message else {
                    break;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(request["sessionId"], "owned-root");
                match request["method"].as_str().unwrap() {
                    "Page.enable" => {
                        socket.send(Message::Text(json!({"method":OPENING,"sessionId":"owned-root","params":{"url":"https://example.com/private?credential=hidden","type":"prompt","message":"Existing page prompt","defaultPrompt":"private-default"}}).to_string().into())).await.unwrap();
                    }
                    "Page.handleJavaScriptDialog" => {
                        count.fetch_add(1, Ordering::SeqCst);
                        socket.send(Message::Text(json!({"method":CLOSED,"sessionId":"owned-root","params":{"result":true,"userInput":"private-user-input"}}).to_string().into())).await.unwrap();
                        socket.send(Message::Text(json!({"method":OPENING,"sessionId":"owned-root","params":{"url":"about:blank","type":"alert","message":"Second dialog"}}).to_string().into())).await.unwrap();
                        ack.acquire().await.unwrap().forget();
                    }
                    other => panic!("unexpected native operation: {other}"),
                }
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"sessionId":"owned-root","result":{}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        });
        let connection = Connection::connect(&endpoint).await.unwrap();
        connection.registry().register_session("owned-root", "page");
        (connection, replies, allow, task)
    }

    #[tokio::test]
    async fn install_drains_existing_dialog_and_shutdown_does_not_answer_it() {
        let (connection, replies, _, peer) = fixture().await;
        let dialogs = Dialogs::install(&connection, "owned-root").await.unwrap();
        let before = dialogs
            .snapshot()
            .expect("Opening precedes Page.enable ACK");
        assert_eq!(before["owned"], false);
        assert!(*dialogs.response_pause().borrow());
        dialogs.set_action_active(true);
        assert_eq!(
            dialogs.snapshot().unwrap()["owned"],
            false,
            "existing modal must not become action-owned"
        );
        dialogs.shutdown().await.unwrap();
        assert_eq!(dialogs.snapshot(), Some(before));
        assert_eq!(replies.load(Ordering::SeqCst), 0);
        connection.shutdown().await;
        drop(connection);
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn queued_events_keep_their_preceding_action_phase() {
        let (connection, replies, _, peer) = fixture().await;
        let dialogs = Dialogs::install(&connection, "owned-root").await.unwrap();
        let inject = |method: &str, params: Value| {
            connection
                .registry()
                .dispatch_message(
                    &json!({"method":method,"sessionId":"owned-root","params":params}).to_string(),
                )
                .unwrap();
        };
        inject(CLOSED, json!({"userInput":"not-exported"}));
        inject(
            OPENING,
            json!({"url":"about:srcdoc","type":"confirm","message":"before action"}),
        );
        dialogs.set_action_active(true);
        assert_eq!(dialogs.snapshot().unwrap()["owned"], false);
        inject(CLOSED, json!({}));
        inject(
            OPENING,
            json!({"url":"https://example.com/","type":"confirm","message":"during action"}),
        );
        dialogs.set_action_active(false);
        assert_eq!(dialogs.snapshot().unwrap()["owned"], true);
        assert_eq!(replies.load(Ordering::SeqCst), 0);
        dialogs.shutdown().await.unwrap();
        connection.shutdown().await;
        drop(connection);
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_shutdown_waiter_still_joins_the_original_reply() {
        let (connection, replies, allow, peer) = fixture().await;
        let dialogs = Dialogs::install(&connection, "owned-root").await.unwrap();
        let id = dialogs.snapshot().unwrap()["dialog_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let reply_owner = dialogs.clone();
        let reply = tokio::spawn(async move { reply_owner.reply(&id, false, None).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while replies.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let closing = dialogs.clone();
        let mut shutdown = tokio::spawn(async move { closing.shutdown().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
                .await
                .is_err()
        );
        shutdown.abort();
        assert!(shutdown.await.unwrap_err().is_cancelled());
        let closing = dialogs.clone();
        let mut second = tokio::spawn(async move { closing.shutdown().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut second)
                .await
                .is_err()
        );
        allow.add_permits(1);
        reply.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(replies.load(Ordering::SeqCst), 1);
        assert!(
            dialogs.snapshot().unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("Second dialog")
        );
        connection.shutdown().await;
        drop(connection);
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn dropped_reply_waiter_never_resends_or_clears_the_next_dialog() {
        let (connection, replies, allow, peer) = fixture().await;
        let dialogs = Dialogs::install(&connection, "owned-root").await.unwrap();
        let first = dialogs.snapshot().unwrap();
        let id = first["dialog_id"].as_str().unwrap().to_owned();
        assert_eq!(
            dialogs.reply(&id, false, Some("invalid".into())).await,
            Err(Error::InvalidInput)
        );
        let waiting = dialogs.clone();
        let waiting_id = id.clone();
        let task = tokio::spawn(async move {
            waiting
                .reply(&waiting_id, true, Some("explicit".into()))
                .await
        });
        let mut changes = dialogs.updates();
        tokio::time::timeout(Duration::from_secs(2), async {
            while dialogs.snapshot().as_ref().is_none_or(|value| {
                value["message"]
                    .as_str()
                    .is_none_or(|text| !text.contains("Second dialog"))
            }) {
                changes.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let second = dialogs.snapshot().unwrap();
        assert_ne!(first["dialog_id"], second["dialog_id"]);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let again = dialogs.clone();
        let again_id = id.clone();
        let mut retry =
            tokio::spawn(
                async move { again.reply(&again_id, true, Some("explicit".into())).await },
            );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut retry)
                .await
                .is_err()
        );
        assert_eq!(replies.load(Ordering::SeqCst), 1);
        allow.add_permits(1);
        retry.await.unwrap().unwrap();
        assert_eq!(dialogs.snapshot(), Some(second));
        dialogs
            .reply(&id, true, Some("explicit".into()))
            .await
            .unwrap();
        assert_eq!(replies.load(Ordering::SeqCst), 1);
        dialogs.shutdown().await.unwrap();
        connection.shutdown().await;
        drop(connection);
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }
}
