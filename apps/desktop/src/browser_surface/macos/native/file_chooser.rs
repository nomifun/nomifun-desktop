//! Scoped CEF file-chooser events. Paths only come from the owning workspace or
//! an explicitly authorized native picker, never from a web page or this event.
use super::*;
use nomifun_browser_platform::{runtime::WorkspaceError, run_guard::RunAdmissionError};
use tokio_util::sync::CancellationToken;

pub(crate) struct Choice {
    invalid: Arc<AtomicBool>,
    page: Arc<Page>,
    pub session: String,
    pub frame: String,
    pub backend_node: i64,
    pub multiple: bool,
}
impl Choice {
    pub(crate) fn require_current(&self) -> Result<(), WorkspaceError> {
        if self.invalid.load(Ordering::Acquire) || self.page.protocol.is_closed() { Err(WorkspaceError::ActionInterrupted) } else { Ok(()) }
    }
}
pub(crate) struct FileChooser {
    armed: Arc<AtomicBool>,
    invalid: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Choice, WorkspaceError>>,
    subscription: CallbackSubscription,
}
impl Drop for FileChooser {
    fn drop(&mut self) { self.armed.store(false, Ordering::Release); self.invalid.store(true, Ordering::Release); }
}
impl FileChooser {
    pub(crate) async fn listen(view: &View) -> Result<Self, WorkspaceError> {
        let armed = Arc::new(AtomicBool::new(false));
        let invalid = Arc::new(AtomicBool::new(false));
        let arm = armed.clone(); let fail = invalid.clone();
        let page = view.page.clone();
        let seen = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel(1);
        let subscription = view.page.protocol.subscribe_callback(&["Page.fileChooserOpened"], Arc::new(move |event| {
            if !arm.load(Ordering::Acquire) { return true; }
            if seen.swap(true, Ordering::AcqRel) { fail.store(true, Ordering::Release); return false; }
            let choice = (|| {
                let frame = event.params["frameId"].as_str().filter(|id| !id.is_empty() && id.len() <= 256).ok_or(WorkspaceError::UnsupportedAction)?.to_owned();
                let backend_node = event.params["backendNodeId"].as_i64().filter(|id| *id > 0).ok_or(WorkspaceError::UnsupportedAction)?;
                let multiple = match event.params["mode"].as_str() { Some("selectSingle") => false, Some("selectMultiple") => true, _ => return Err(WorkspaceError::UnsupportedAction) };
                Ok(Choice { invalid: fail.clone(), page: page.clone(), session: event.session.unwrap_or_default(), frame, backend_node, multiple })
            })();
            if sender.try_send(choice).is_err() { fail.store(true, Ordering::Release); return false; }
            true
        })).map_err(|_| WorkspaceError::NativeCommandFailed)?;
        Ok(Self { armed, invalid, receiver, subscription })
    }
    pub(crate) fn arm(&self) { self.armed.store(true, Ordering::Release); }
    pub(crate) async fn next(&mut self, cancel: &CancellationToken) -> Result<Choice, WorkspaceError> {
        let choice = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(RunAdmissionError::Cancelled.into()),
            value = tokio::time::timeout(std::time::Duration::from_secs(3), self.receiver.recv()) => value,
        }.map_err(|_| WorkspaceError::ActionInterrupted)?.ok_or(WorkspaceError::ActionInterrupted)??;
        if self.subscription.failed.load(Ordering::Acquire) { return Err(WorkspaceError::ActionInterrupted); }
        choice.require_current()?;
        Ok(choice)
    }
}
