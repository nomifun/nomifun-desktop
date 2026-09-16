//! Retain the original browser operation while the model answers a native
//! dialog. A dialog reply is not queued behind the input ACK it must unblock.
use super::*;
use futures_util::{StreamExt, stream::FuturesUnordered};

#[derive(Clone)]
pub(in crate::attached_browser) struct Pending {
    pub(super) id: String,
    pub(super) job: Shared<BoxFuture<'static, Result<Value, Error>>>,
    pub(super) cancel: CancellationToken,
    pub(super) dialogs: Arc<script_dialogs::Dialogs>,
    interrupted: Arc<std::sync::atomic::AtomicBool>,
}

impl Pending {
    pub(in crate::attached_browser) fn retirement(&self) -> BoxFuture<'static, ()> {
        let job = self.job.clone();
        let dialogs = self.dialogs.clone();
        async move {
            let _ = job.await;
            let _ = dialogs.shutdown().await;
        }
        .boxed()
    }
}

pub(super) fn waiting(grant: &GrantedTab, dialog: &Value, active: bool) -> Value {
    json!({"tab_id":grant.id(),"completed":false,"action_pending":active,"script_dialog":dialog,
        "observe_before_acting":true,"untrusted_page_content":true})
}

pub(super) fn spawn(
    work: impl std::future::Future<Output = Result<Value, Error>> + Send + 'static,
    dialogs: Arc<script_dialogs::Dialogs>,
    conn: Connection,
    cancel: CancellationToken,
    active: bool,
) -> Pending {
    let dialog_owner = dialogs.clone();
    let stopped = cancel.clone();
    let handle = tokio::spawn(async move {
        if stopped.is_cancelled() {
            return Err(Error::Cancelled);
        }
        dialog_owner.set_action_active(active);
        let mut updates = dialog_owner.updates();
        let mut work = Box::pin(work);
        let mut draining = false;
        let mut read_deadline = None;
        let mut answered = BTreeSet::new();
        let mut replies = FuturesUnordered::<BoxFuture<'static, Result<(), Error>>>::new();
        let result = loop {
            if draining {
                if let Some(dialog) = dialog_owner.snapshot() {
                    let id = dialog["dialog_id"].as_str().unwrap_or("").to_owned();
                    if dialog["owned"] != true || new_dialog_exceeds_budget(&answered, &id) {
                        // No blind dismissal of a pre-existing/unknown dialog.
                        // Retiring only our transport terminates its waiters;
                        // the user's page and original modal remain in Chrome.
                        conn.registry().fail_connection();
                    } else if answered.insert(id.clone()) {
                        let owner = dialog_owner.clone();
                        replies.push(async move { owner.dismiss_or_join_reply(&id).await }.boxed());
                    }
                }
            }
            tokio::select! {
                biased;
                result=&mut work=>break result,
                _=stopped.cancelled(),if !draining=>{
                    draining=true;
                    // A normal read may finish cooperatively without losing
                    // authorization. One fixed grace bounds unknown old modal
                    // waits; lifecycle traffic must never restart this budget.
                    if !active {read_deadline=Some(tokio::time::Instant::now()+std::time::Duration::from_millis(250));}
                },
                _=async {tokio::time::sleep_until(read_deadline.expect("enabled read deadline")).await},if read_deadline.is_some()=>{
                    conn.registry().fail_connection();read_deadline=None;
                },
                result=replies.next(),if !replies.is_empty()=>{
                    if result.is_some_and(|result|result.is_err()) {
                        // A submitted reply is never replayed. Closed events
                        // and the original input ACK remain the proof of exit.
                        if dialog_owner.snapshot().is_some() {conn.registry().fail_connection();}
                    }
                },
                result=updates.changed()=>{
                    if result.is_err(){conn.registry().fail_connection();}
                },
            }
        };
        dialog_owner.set_action_active(false);
        while replies.next().await.is_some() {}
        result
    });
    Pending {
        id: nomifun_common::generate_id(),
        job: async move { handle.await.unwrap_or(Err(Error::ExecutionFailed)) }
            .boxed()
            .shared(),
        cancel,
        dialogs,
        interrupted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

fn new_dialog_exceeds_budget(answered: &BTreeSet<String>, id: &str) -> bool {
    !answered.contains(id) && answered.len() >= 8
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;

impl AttachedBrowser {
    pub(super) fn pending(&self, target: &str) -> Option<Pending> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .get(target)
            .cloned()
    }
    pub(super) fn forget_pending(&self, target: &str, id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .pending
            .get(target)
            .is_some_and(|pending| pending.id == id)
        {
            state.pending.remove(target);
        }
    }
    pub(super) async fn wait_pending(
        &self,
        grant: &GrantedTab,
        pending: Pending,
    ) -> Result<Value, Error> {
        let mut updates = pending.dialogs.updates();
        let mut job = Box::pin(pending.job.clone());
        loop {
            tokio::select! {
                biased;
                result=&mut job=>{
                    self.forget_pending(&grant.target_id,&pending.id);
                    if pending.interrupted.load(std::sync::atomic::Ordering::Acquire) {
                        if let Some(state)=self.state.lock().unwrap_or_else(|e|e.into_inner()).automation.get_mut(&grant.target_id) {
                            state.observation.clear();state.refs.clear();
                        }
                    }
                    return result;
                },
                dialog=async {
                    loop {
                        if let Some(dialog)=pending.dialogs.snapshot(){return Ok(dialog);}
                        updates.changed().await.map_err(|_|Error::Disconnected)?;
                    }
                }=>{pending.interrupted.store(true,std::sync::atomic::Ordering::Release);return Ok(waiting(grant,&dialog?,true));},
            }
        }
    }
}
