//! Browser work remains owned across modal dialogs, including navigation and
//! observations. Yielding a dialog never drops or replays a native operation.
use super::*;
use tokio::task::JoinHandle;

enum Work {
    Evaluate(BrowserEvaluation),
    Input(NativeOperation),
    Command(BrowserTabCommand),
    Observe(Option<String>),
    Screenshot(Option<String>),
}
#[derive(Clone)]
enum Output {
    Evaluate(BrowserEvaluationResult),
    Input(BrowserActionResult),
    Command(BrowserRuntimeSnapshot),
    Observe(BrowserObservation),
    Screenshot(BrowserScreenshot),
}
type WorkResult = Result<Output, WorkspaceError>;
enum Yielded {
    Finished(Output),
    Dialog(BrowserDialog),
}
struct TaskState {
    handle: Option<JoinHandle<WorkResult>>,
    result: Option<WorkResult>,
}
pub(super) struct PendingWork {
    id: String,
    waiting_for: StdMutex<Option<String>>,
    scope: Arc<StdMutex<Option<String>>>,
    closing_tab: bool,
    fidelity: InteractionFidelity,
    cancel: CancellationToken,
    finished: Arc<AtomicBool>,
    task: Mutex<TaskState>,
}
struct Finished(Arc<AtomicBool>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl PendingWork {
    async fn wait(&self) -> WorkResult {
        let mut task = self.task.lock().await;
        if let Some(result) = &task.result {
            return result.clone();
        }
        let result = task
            .handle
            .as_mut()
            .expect("owned browser work")
            .await
            .unwrap_or(Err(RunAdmissionError::WorkerFailed.into()));
        task.handle.take();
        task.result = Some(result.clone());
        result
    }
}
fn uncertain(result: &WorkResult) -> bool {
    matches!(
        result,
        Err(WorkspaceError::Admission(RunAdmissionError::WorkerFailed))
    )
}
fn waiting(dialog: BrowserDialog, fidelity: InteractionFidelity) -> BrowserActionResult {
    BrowserActionResult {
        download: None,
        target: dialog.target.clone(),
        interaction_fidelity: fidelity,
        outcome: BrowserActionOutcome::AwaitingDialog { dialog },
    }
}

impl DesktopBrowserRuntime {
    async fn owned_tasks(&self) -> Vec<Arc<PendingWork>> {
        self.pending_work.lock().await.values().cloned().collect()
    }
    async fn clear_pending(&self, pending: &Arc<PendingWork>) {
        let mut slot = self.pending_work.lock().await;
        slot.remove(&pending.id);
    }
    pub(super) async fn ensure_no_pending_work(&self) -> Result<(), WorkspaceError> {
        for pending in self.owned_tasks().await {
            if !pending.finished.load(Ordering::Acquire) {
                if pending
                    .waiting_for
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some()
                {
                    return Err(WorkspaceError::DialogPending);
                }
                continue;
            }
            let result = pending.wait().await;
            if uncertain(&result) {
                return result.map(|_| ());
            }
            self.clear_pending(&pending).await;
            result?;
        }
        Ok(())
    }
    async fn dialog_policy(&self, drain: bool) -> Result<(), WorkspaceError> {
        let tabs: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        let mut failure = None;
        for tab in tabs {
            if tab.popup_stop.is_cancelled() { continue; }
            let result = if drain {
                native::script_dialogs::drain(&tab.view).await
            } else {
                native::script_dialogs::resume(&tab.view).await
            };
            if let Err(error) = result {
                if !tab.popup_stop.is_cancelled() { failure = Some(error); }
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub(super) async fn settle_pending_work(&self) -> Result<(), RunAdmissionError> {
        let tasks = self.owned_tasks().await;
        for pending in &tasks {
            pending.cancel.cancel();
        }
        // Async page dialogs also block protocol cleanup when no tool is active.
        self.dialog_policy(true)
            .await
            .map_err(|_| RunAdmissionError::InputGateFailed)?;
        for pending in tasks {
            let result = pending.wait().await;
            if uncertain(&result) {
                return Err(RunAdmissionError::InputGateFailed);
            }
            self.clear_pending(&pending).await;
        }
        Ok(())
    }
    pub(super) async fn cancel_pending_for_close(&self) {
        for pending in self.owned_tasks().await {
            pending.cancel.cancel();
        }
        let _ = self.dialog_policy(true).await;
    }
    pub(super) async fn retire_pending_after_close(&self) {
        for pending in self.owned_tasks().await {
            let _ = pending.wait().await;
            self.clear_pending(&pending).await;
        }
    }
    async fn current_dialog(&self) -> Option<BrowserDialog> {
        let state = self.state.lock().await;
        state
            .active
            .as_ref()
            .and_then(|id| state.tabs.get(id))
            .and_then(|tab| {
                tab.metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .script_dialog
                    .clone()
            })
            .or_else(|| {
                state.tabs.values().find_map(|tab| {
                    tab.metadata
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .script_dialog
                        .clone()
                })
            })
    }
    async fn wait_owned(&self, pending: Arc<PendingWork>) -> Result<Yielded, WorkspaceError> {
        let mut changes = self.revision.subscribe();
        loop {
            if !pending.closing_tab && !pending.cancel.is_cancelled() {
                if let Some(dialog) = self.current_dialog().await {
                    *pending
                        .waiting_for
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(dialog.request_id.clone());
                    let mut state = self.state.lock().await;
                    if state.closed {
                        return Err(WorkspaceError::WorkspaceClosed);
                    }
                    if !state.input_enabled {
                        state.active = Some(dialog.target.tab_id.clone());
                        self.request_presentation(&mut state);
                    }
                    return Ok(Yielded::Dialog(dialog));
                }
            }
            tokio::select! {
                result=pending.wait()=>{
                    if !uncertain(&result) {self.clear_pending(&pending).await;}
                    return result.map(Yielded::Finished);
                }
                changed=changes.changed()=>{changed.map_err(|_|WorkspaceError::NativeCommandFailed)?;}
            }
        }
    }
    async fn start_work(
        &self,
        work: Work,
        cancel: CancellationToken,
    ) -> Result<Yielded, WorkspaceError> {
        let closing_tab = matches!(&work, Work::Command(BrowserTabCommand::Close { .. } | BrowserTabCommand::CloseAll { .. } | BrowserTabCommand::ClearSiteData { .. }));
        if !closing_tab {
            self.ensure_no_pending_work().await?;
        }
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        if self.closing.is_cancelled() {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let runtime = self.weak.upgrade().ok_or(WorkspaceError::WorkspaceClosed)?;
        let scope = match &work {
            Work::Input(operation) => Some(operation.element().target.tab_id.clone()),
            Work::Evaluate(request) => Some(request.target.tab_id.clone()),
            Work::Command(command) => command.target().map(|target| target.tab_id.clone()),
            Work::Observe(tab) | Work::Screenshot(tab) => {
                tab.clone().or(self.state.lock().await.active.clone())
            }
        };
        let scope = Arc::new(StdMutex::new(scope));
        let worker_scope = scope.clone();
        let id = uuid::Uuid::now_v7().to_string();
        let worker_id = id.clone();
        let fidelity = match &work {
            Work::Input(NativeOperation::Input(_) | NativeOperation::Download { .. }) => InteractionFidelity::BrowserInput,
            _ => InteractionFidelity::BrowserProtocol,
        };
        let finished = Arc::new(AtomicBool::new(false));
        let marker = Finished(finished.clone());
        let run_cancel = cancel;
        let cancel = run_cancel.child_token();
        let operation_cancel = cancel.clone();
        let mut slot = self.pending_work.lock().await;
        if !closing_tab
            && slot.values().any(|pending| {
                pending
                    .waiting_for
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some()
            })
        {
            return Err(WorkspaceError::DialogPending);
        }
        let task = tokio::spawn(async move {
            let _marker = marker;
            if !closing_tab {
                runtime.dialog_policy(false).await?;
            }
            let future = async {
                match work {
                    Work::Evaluate(request) => runtime.evaluate_inner(request, operation_cancel.clone()).await.map(Output::Evaluate),
                    Work::Input(operation) => runtime
                        .perform_action_inner(operation, operation_cancel.clone())
                        .await
                        .map(Output::Input),
                    Work::Command(BrowserTabCommand::Close { target }) => runtime
                        .close_tab_owned(target, operation_cancel.clone(), &worker_id)
                        .await
                        .map(Output::Command),
                    Work::Command(BrowserTabCommand::CloseAll { runtime_generation }) => runtime
                        .close_all_owned(runtime_generation, operation_cancel.clone(), &worker_id, false)
                        .await
                        .map(Output::Command),
                    Work::Command(BrowserTabCommand::ClearSiteData { runtime_generation }) => runtime
                        .close_all_owned(runtime_generation, operation_cancel.clone(), &worker_id, true)
                        .await
                        .map(Output::Command),
                    Work::Command(command) => runtime
                        .execute_inner(command, operation_cancel.clone(), worker_scope.clone())
                        .await
                        .map(Output::Command),
                    Work::Observe(tab) => runtime
                        .observe_inner(tab, operation_cancel.clone())
                        .await
                        .map(Output::Observe),
                    Work::Screenshot(tab) => runtime
                        .screenshot_inner(tab, operation_cancel.clone())
                        .await
                        .map(Output::Screenshot),
                }
            };
            tokio::pin!(future);
            tokio::select! {biased;
                _=operation_cancel.cancelled()=>{
                    let scope=worker_scope.lock().unwrap_or_else(|e|e.into_inner()).clone();
                    let drained=if run_cancel.is_cancelled() || scope.is_none() { runtime.dialog_policy(true).await }
                        else { runtime.drain_tab_dialog(scope.as_deref().unwrap()).await };
                    let result=future.await;
                    drained?;result
                }
                result=&mut future=>result,
            }
        });
        let pending = Arc::new(PendingWork {
            id,
            waiting_for: StdMutex::new(None),
            scope,
            closing_tab,
            fidelity,
            cancel,
            finished,
            task: Mutex::new(TaskState {
                handle: Some(task),
                result: None,
            }),
        });
        slot.insert(pending.id.clone(), pending.clone());
        drop(slot);
        self.wait_owned(pending).await
    }

    async fn drain_tab_dialog(&self, id: &str) -> Result<(), WorkspaceError> {
        let tab = self.state.lock().await.tabs.get(id).cloned();
        match tab {
            Some(tab) => native::script_dialogs::drain(&tab.view).await,
            None => Ok(()),
        }
    }

    async fn close_tab_owned(
        &self,
        target: BrowserTabTarget,
        cancel: CancellationToken,
        work_id: &str,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        let tab = {
            let state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            self.target(&state, &target)?.clone()
        };
        self.retire_native_tab(&tab).await?;
        // Destruction proves this page cannot continue. Cancel only its child
        // operation tokens; the Agent run and other page dialogs remain intact.
        let tasks: Vec<_> = self
            .owned_tasks()
            .await
            .into_iter()
            .filter(|task| {
                task.id != work_id
                    && !task.closing_tab
                    && task
                        .scope
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_deref()
                        == Some(target.tab_id.as_str())
            })
            .collect();
        for task in &tasks {
            task.cancel.cancel();
        }
        for task in tasks {
            let _ = task.wait().await;
            self.clear_pending(&task).await;
        }
        self.snapshot().await
    }

    async fn close_all_owned(&self, generation:u64, cancel:CancellationToken, work_id:&str, clear_site_data:bool) -> Result<BrowserRuntimeSnapshot,WorkspaceError> {
        let admitted=|state:&RuntimeState|->Result<(),WorkspaceError> {
            if state.closed || self.closing.is_cancelled() { return Err(WorkspaceError::WorkspaceClosed); }
            if generation!=self.request.runtime_generation { return Err(WorkspaceError::StaleTarget); }
            if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
            if !state.input_enabled || self.input_locked.load(Ordering::Acquire) { return Err(RunAdmissionError::UserInputLocked.into()); }
            Ok(())
        };
        { let state=self.state.lock().await; admitted(&state)?; }
        // Cancel unfinished page work before waiting for creation. A pending
        // navigation/dialog can itself be holding the creation guard.
        let tasks:Vec<_>=self.owned_tasks().await.into_iter().filter(|task|task.id!=work_id&&!task.closing_tab).collect();
        for task in &tasks { task.cancel.cancel(); }
        self.dialog_policy(true).await?;
        let _creation=tokio::select! {biased;
            _=cancel.cancelled()=>return Err(RunAdmissionError::Cancelled.into()),
            _=self.closing.cancelled()=>return Err(WorkspaceError::WorkspaceClosed),
            guard=self.creating.lock()=>guard,
        };
        let tabs:Vec<_>={
            let state=self.state.lock().await;admitted(&state)?;
            let tabs:Vec<_>=state.tabs.values().cloned().collect();
            // Invalidate every opener before closing the first page. Late
            // window.open requests cannot recreate tabs behind this command.
            for tab in &tabs { tab.popup_stop.cancel(); }
            tabs
        };
        if clear_site_data && tabs.is_empty() { return Err(WorkspaceError::TabNotFound); }
        let maintenance = if clear_site_data { Some(self.create_site_data_view().await?) } else { None };
        let mut failure=None;
        for tab in &tabs {
            // Closing candidates must not cover the error/empty state if a
            // later native Close fails. Layout skips these fenced views.
            let _=native::hide(&tab.view).await;
            // A successful native Close is stronger than a failed preliminary
            // input freeze. On Close failure keep the exact tab for retry.
            let _=native::set_native_user_input_enabled(&tab.view,false).await;
            if let Err(error)=self.retire_native_tab(tab).await { failure=Some(error); }
        }
        for task in tasks {
            let result=task.wait().await;
            if uncertain(&result) {
                failure=Some(RunAdmissionError::WorkerFailed.into());
            } else {
                self.clear_pending(&task).await;
            }
        }
        if let Some(view) = maintenance {
            let result = if let Some(error) = failure.take() { Err(error) }
                else if cancel.is_cancelled() || self.closing.is_cancelled() { Err(RunAdmissionError::Cancelled.into()) }
                else { native::site_data::clear(&view, cancel.clone()).await };
            if matches!(result, Err(WorkspaceError::Admission(RunAdmissionError::WorkerFailed))) { return Err(RunAdmissionError::WorkerFailed.into()); }
            self.close_site_data_view().await?;
            result?;
        }
        if let Some(error)=failure { return Err(error); }
        // Keep runtime identity and download history. Only the explicit user
        // ClearSiteData variant removes this conversation's site data.
        self.snapshot().await
    }
    pub(super) async fn perform_action(
        &self,
        operation: NativeOperation,
        cancel: CancellationToken,
    ) -> Result<BrowserActionResult, WorkspaceError> {
        let fidelity = match &operation {
            NativeOperation::Input(_) | NativeOperation::Download { .. } => InteractionFidelity::BrowserInput,
            _ => InteractionFidelity::BrowserProtocol,
        };
        match self.start_work(Work::Input(operation), cancel).await? {
            Yielded::Finished(Output::Input(result)) => Ok(result),
            Yielded::Dialog(dialog) => Ok(waiting(dialog, fidelity)),
            _ => Err(WorkspaceError::NativeCommandFailed),
        }
    }
    pub(super) async fn evaluate_owned(&self, request: BrowserEvaluation, cancel: CancellationToken) -> Result<BrowserEvaluationResult, WorkspaceError> {
        match self.start_work(Work::Evaluate(request), cancel).await? {
            Yielded::Finished(Output::Evaluate(result)) => Ok(result),
            Yielded::Dialog(dialog) => Ok(BrowserEvaluationResult { target: dialog.target.clone(), execution_kind: "developer_script", outcome: BrowserEvaluationOutcome::AwaitingDialog { dialog } }),
            _ => Err(WorkspaceError::NativeCommandFailed),
        }
    }
    pub(super) async fn observe_owned(
        &self,
        tab: Option<String>,
        cancel: CancellationToken,
    ) -> Result<BrowserObservation, WorkspaceError> {
        match self.start_work(Work::Observe(tab),cancel).await? {
            Yielded::Finished(Output::Observe(result))=>Ok(result),
            Yielded::Dialog(dialog)=>Ok(BrowserObservation {target:dialog.target.clone(), observation_generation:0,
                content:"The page is paused by an untrusted website dialog. Respond to the exact dialog, then observe again.".into(),
                elements:vec![],unobserved_frames:1,script_dialog:Some(dialog)}),
            _=>Err(WorkspaceError::NativeCommandFailed),
        }
    }
    pub(super) async fn capture_owned(
        &self,
        tab: Option<String>,
        cancel: CancellationToken,
    ) -> Result<BrowserScreenshot, WorkspaceError> {
        match self.start_work(Work::Screenshot(tab), cancel).await? {
            Yielded::Finished(Output::Screenshot(result)) => Ok(result),
            Yielded::Dialog(_) => Err(WorkspaceError::DialogPending),
            _ => Err(WorkspaceError::NativeCommandFailed),
        }
    }
    pub(super) async fn execute_owned(
        &self,
        command: BrowserTabCommand,
        cancel: CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if let BrowserTabCommand::Dialog {
            target,
            request_id,
            accept,
            text,
        } = command
        {
            self.respond_native(
                BrowserDialogReply {
                    target,
                    request_id,
                    accept,
                    text,
                },
                cancel,
                true,
            )
            .await?;
            return self.snapshot().await;
        }
        match self.start_work(Work::Command(command), cancel).await? {
            Yielded::Finished(Output::Command(snapshot)) => Ok(snapshot),
            Yielded::Dialog(_) => self.snapshot().await,
            _ => Err(WorkspaceError::NativeCommandFailed),
        }
    }
    async fn respond_native(
        &self,
        reply: BrowserDialogReply,
        cancel: CancellationToken,
        user: bool,
    ) -> Result<BrowserActionResult, WorkspaceError> {
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        let view = {
            let state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if state.input_enabled != user
                || (user && state.active.as_ref() != Some(&reply.target.tab_id))
            {
                return Err(WorkspaceError::NotActionable);
            }
            self.target(&state, &reply.target)?.view.clone()
        };
        let pending = self.owned_tasks().await.into_iter().find(|pending| {
            pending
                .waiting_for
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_deref()
                == Some(reply.request_id.as_str())
        });
        if pending
            .as_ref()
            .is_some_and(|pending| pending.cancel.is_cancelled())
        {
            return Err(RunAdmissionError::Cancelled.into());
        }
        let original = reply.target.clone();
        native::script_dialogs::respond(
            &view,
            reply.target,
            reply.request_id,
            reply.accept,
            reply.text,
        )
        .await?;
        if let Some(pending) = pending {
            pending
                .waiting_for
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            let fidelity = pending.fidelity;
            match self.wait_owned(pending).await? {
                Yielded::Dialog(dialog) => return Ok(waiting(dialog, fidelity)),
                Yielded::Finished(Output::Input(result)) => return Ok(result),
                Yielded::Finished(Output::Evaluate(evaluation)) => return Ok(BrowserActionResult {
                    download: None, target: evaluation.target.clone(), interaction_fidelity: InteractionFidelity::BrowserProtocol,
                    outcome: BrowserActionOutcome::EvaluationResult { evaluation },
                }),
                Yielded::Finished(_) => {}
            }
        }
        let snapshot = self.snapshot().await?;
        let target = snapshot
            .tabs
            .iter()
            .find(|tab| tab.target.tab_id == original.tab_id)
            .map(|tab| tab.target.clone())
            .unwrap_or(original);
        Ok(BrowserActionResult {
            download: None,
            target,
            interaction_fidelity: InteractionFidelity::BrowserProtocol,
            outcome: BrowserActionOutcome::Completed,
        })
    }
    pub(super) async fn reply_to_dialog(
        &self,
        reply: BrowserDialogReply,
        cancel: CancellationToken,
    ) -> Result<BrowserActionResult, WorkspaceError> {
        self.respond_native(reply, cancel, false).await
    }
}
