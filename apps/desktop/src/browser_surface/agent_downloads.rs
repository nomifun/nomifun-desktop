//! One explicit observed click may admit one native download. No Save picker,
//! injected anchor, HTTP side client, or ambient permission for ordinary acts.
use super::*;
use nomifun_browser_platform::downloads::BrowserDownloadArtifact;
use tauri::Manager;

pub(crate) struct AgentRequest {
    accepting: AtomicBool,
    pub(super) target: BrowserTabTarget,
    pub(super) file: Arc<PreparedBrowserDownload>,
    pub(super) cancel: CancellationToken,
    pub(super) job: watch::Sender<Option<Arc<Job>>>,
}
#[derive(Clone)]
pub(super) struct Admission {
    pub(super) request: Arc<AgentRequest>,
    pub(super) target: BrowserTabTarget,
}
impl Admission {
    pub(super) fn claim(&self, target: &BrowserTabTarget) -> bool {
        self.target == *target
            && !self.request.cancel.is_cancelled()
            && self.request.accepting.swap(false, Ordering::AcqRel)
    }
}
/// Captured inside the original NewWindowRequested callback, not when its
/// asynchronous child construction later happens to finish.
pub(crate) fn capture_agent(target: &BrowserTabTarget) -> Option<Arc<AgentRequest>> {
    let control = control(&target.tab_id)?;
    let pending = control.agent.lock().unwrap_or_else(|e| e.into_inner());
    pending
        .as_ref()
        .filter(|grant| {
            grant.target == *target
                && grant.request.accepting.load(Ordering::Acquire)
                && !grant.request.cancel.is_cancelled()
        })
        .map(|grant| grant.request.clone())
}
pub(crate) async fn inherit_agent(
    view: &tauri::Webview,
    opener: BrowserTabTarget,
    request: Arc<AgentRequest>,
) -> Result<(), WorkspaceError> {
    if view.label() == opener.tab_id {
        return Err(WorkspaceError::StaleTarget);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let result = (|| {
            if request.target != opener
                || request.cancel.is_cancelled()
                || !request.accepting.load(Ordering::Acquire)
            {
                return Ok(());
            }
            let Some(source) = control(&opener.tab_id) else {
                return Ok(());
            };
            let destination = control(&label).ok_or(WorkspaceError::TabNotFound)?;
            if source.closed.load(Ordering::Acquire)
                || destination.closed.load(Ordering::Acquire)
                || !source.locked.load(Ordering::Acquire)
                || !destination.locked.load(Ordering::Acquire)
                || source
                    .metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    != opener
            {
                return Err(WorkspaceError::StaleTarget);
            }
            let mut pending = source.agent.lock().unwrap_or_else(|e| e.into_inner());
            if !pending
                .as_ref()
                .is_some_and(|grant| Arc::ptr_eq(&grant.request, &request))
            {
                return Ok(());
            }
            let mut recipient = destination.agent.lock().unwrap_or_else(|e| e.into_inner());
            if recipient.is_some() {
                return Err(WorkspaceError::NotActionable);
            }
            pending.take();
            *recipient = Some(Admission {
                request,
                target: destination
                    .metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    .clone(),
            });
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
pub(crate) async fn settle_agent(view: &tauri::Webview) -> Result<(), WorkspaceError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let jobs = control(&label)
            .map(|control| {
                if let Some(request) = control
                    .agent
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    request.request.accepting.store(false, Ordering::Release);
                    request.request.cancel.cancel();
                }
                let jobs: Vec<_> = control
                    .jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .filter(|job| job.download.is_some())
                    .cloned()
                    .collect();
                for job in &jobs {
                    job.cancel();
                }
                jobs
            })
            .unwrap_or_default();
        let _ = tx.send(jobs);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    for job in rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)? {
        if job.wait().await.is_err() {
            cleanup(view, &job)
                .await
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            job.finish(Ok(()));
        }
        job.history.update(&job.id, |entry| {
            if matches!(
                entry.state,
                BrowserDownloadState::Choosing
                    | BrowserDownloadState::InProgress
                    | BrowserDownloadState::Cancelling
            ) {
                entry.state = BrowserDownloadState::Cancelled;
                entry.can_cancel = false;
            }
        });
    }
    Ok(())
}
pub(crate) async fn wait_page_downloads(view: &tauri::Webview) -> Result<(), WorkspaceError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |_| {
        let jobs = control(&label)
            .map(|control| {
                control
                    .jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .filter(|job| job.download.is_some())
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let _ = tx.send(jobs);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    for job in rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)? {
        job.wait()
            .await
            .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    }
    Ok(())
}
pub(crate) async fn arm_agent(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    file: Arc<PreparedBrowserDownload>,
    cancel: CancellationToken,
) -> Result<Arc<AgentRequest>, WorkspaceError> {
    let request = Arc::new(AgentRequest {
        accepting: AtomicBool::new(true),
        target,
        file,
        cancel: cancel.child_token(),
        job: watch::channel(None).0,
    });
    let native_request = request.clone();
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = (|| {
            let control = control(&label).ok_or(WorkspaceError::TabNotFound)?;
            if control.closed.load(Ordering::Acquire)
                || !control.locked.load(Ordering::Acquire)
                || native_request.cancel.is_cancelled()
                || control
                    .metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    != native_request.target
            {
                return Err(WorkspaceError::StaleTarget);
            }
            let mut pending = control.agent.lock().unwrap_or_else(|e| e.into_inner());
            if pending.is_some() {
                return Err(WorkspaceError::NotActionable);
            }
            *pending = Some(Admission {
                target: native_request.target.clone(),
                request: native_request,
            });
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await
        .map_err(|_| WorkspaceError::NativeCommandFailed)??;
    Ok(request)
}
pub(crate) async fn finish_agent(
    view: &tauri::Webview,
    request: Arc<AgentRequest>,
    action: Result<(), WorkspaceError>,
) -> Result<BrowserDownloadArtifact, WorkspaceError> {
    let mut observed = request.job.subscribe();
    let arrival = async {
        loop {
            if observed.borrow().is_some() {
                return Ok(());
            }
            observed
                .changed()
                .await
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        }
    };
    let arrived = if action.is_ok() {
        tokio::select! { biased;
            _ = request.cancel.cancelled() => Err(WorkspaceError::ActionInterrupted),
            result = tokio::time::timeout(std::time::Duration::from_secs(10), arrival) => result.unwrap_or(Err(WorkspaceError::DownloadDenied)),
        }
    } else {
        Err(action.unwrap_err())
    };
    // Clear admission on the native event thread even if no download arrived.
    // Its acknowledgement fences a late DownloadStarting before cleanup.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let native_request = request.clone();
    view.with_webview(move |_| {
        native_request.accepting.store(false, Ordering::Release);
        let controls = REGISTRATIONS.with(|entries| {
            entries
                .borrow()
                .values()
                .map(|entry| entry.control.clone())
                .collect::<Vec<_>>()
        });
        for control in controls {
            let mut pending = control.agent.lock().unwrap_or_else(|e| e.into_inner());
            if pending
                .as_ref()
                .is_some_and(|pending| Arc::ptr_eq(&pending.request, &native_request))
            {
                pending.take();
            }
        }
        let _ = tx.send(());
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?;
    let job = request.job.send_replace(None);
    let Some(job) = job else {
        return arrived.and(Err(WorkspaceError::DownloadDenied));
    };
    if arrived.is_err() {
        job.cancel();
    }
    // Bounded transfer wait. Cancellation always joins native cleanup before
    // the prepared directory can be removed or Agent terminal can be released.
    let waited = tokio::time::timeout(std::time::Duration::from_secs(120), job.wait()).await;
    if waited.is_err() {
        job.cancel();
    }
    if !matches!(waited, Ok(Ok(()))) {
        if job.wait().await.is_err() {
            let owner = view
                .app_handle()
                .get_webview(&job.target.tab_id)
                .ok_or(WorkspaceError::NativeCommandFailed)?;
            cleanup(&owner, &job)
                .await
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            job.finish(Ok(()));
        }
    }
    let result = async {
        arrived?;
        if request.cancel.is_cancelled() {
            return Err(WorkspaceError::ActionInterrupted);
        }
        if let Some(error) = *job.policy_error.lock().unwrap_or_else(|e| e.into_inner()) {
            return Err(error);
        }
        if *job.native.borrow() != Some(true) {
            return Err(WorkspaceError::DownloadDenied);
        }
        request
            .file
            .publish(
                job.filename.clone(),
                |name, bytes| {
                    !nomi_browser_engine::download::is_executable_denylist(name)
                        && !nomi_browser_engine::download::sniff_is_executable(bytes)
                },
                request.cancel.clone(),
            )
            .await
    }
    .await;
    job.history.update(&job.id, |entry| {
        entry.state = if result.is_ok() {
            BrowserDownloadState::Completed
        } else if request.cancel.is_cancelled() {
            BrowserDownloadState::Cancelled
        } else {
            BrowserDownloadState::Failed
        };
        entry.can_cancel = false;
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> (tempfile::TempDir, Arc<AgentRequest>) {
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(
            nomifun_browser_platform::downloads::BrowserDownloadScope::open(root.path()).unwrap(),
        );
        let request = Arc::new(AgentRequest {
            target: BrowserTabTarget {
                tab_id: "parent".into(),
                runtime_generation: 1,
                document_generation: 1,
            },
            file: scope.prepare().unwrap(),
            cancel: CancellationToken::new(),
            job: watch::channel(None).0,
            accepting: AtomicBool::new(true),
        });
        (root, request)
    }
    #[test]
    fn popup_and_opener_share_exactly_one_claim_and_stale_targets_do_not_consume_it() {
        let (_root, request) = request();
        let parent = Admission {
            request: request.clone(),
            target: request.target.clone(),
        };
        let child_target = BrowserTabTarget {
            tab_id: "popup".into(),
            runtime_generation: 1,
            document_generation: 0,
        };
        let child = Admission {
            request: request.clone(),
            target: child_target.clone(),
        };
        assert!(!child.claim(&parent.target));
        assert!(child.claim(&child_target));
        assert!(!parent.claim(&parent.target));
        assert!(!child.claim(&child_target));
    }
    #[test]
    fn cancelled_or_finished_request_cannot_authorize_a_late_popup_download() {
        for cancelled in [true, false] {
            let (_root, request) = request();
            let grant = Admission {
                request: request.clone(),
                target: request.target.clone(),
            };
            if cancelled {
                request.cancel.cancel();
            } else {
                request.accepting.store(false, Ordering::Release);
            }
            assert!(!grant.claim(&grant.target));
        }
    }
}
