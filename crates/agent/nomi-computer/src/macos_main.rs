//! Main-thread dispatch for macOS native APIs with AppKit / input-source
//! affinity. Non-macOS builds run the task inline.

#[cfg(target_os = "macos")]
pub(crate) type MainTask<T> = Box<dyn FnOnce() -> Result<T, String> + Send + 'static>;

#[cfg(target_os = "macos")]
pub(crate) fn run_task_with<T, D, F>(dispatch: D, task: F) -> Result<T, String>
where
    T: Send + 'static,
    D: FnOnce(MainTask<T>) -> Result<T, String>,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    dispatch(Box::new(task))
}

#[cfg(target_os = "macos")]
pub(crate) fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    if objc2::MainThreadMarker::new().is_some() {
        return task();
    }

    // `dispatch2::run_on_main` explicitly waits forever when the process has
    // no running AppKit/CFRunLoop (unit tests, CLI hosts, early startup). Queue
    // the owned closure asynchronously and retain a cancellable slot instead.
    // If the main queue cannot begin it within the bounded admission window,
    // remove the closure before returning so input/launch work can never run
    // late after the caller observed failure. Once the queue has taken the
    // closure we join its exact result rather than abandoning an in-flight OS
    // side effect.
    run_task_with(
        |task| {
            let scheduled = std::sync::Arc::new(std::sync::Mutex::new(Some(task)));
            let queued = scheduled.clone();
            let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
            dispatch2::DispatchQueue::main().exec_async(move || {
                let task = queued
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(task) = task {
                    let _ = result_tx.send(task());
                }
            });
            match result_rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(result) => result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    Err("macOS main-thread dispatcher closed before completion".into())
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let cancelled = scheduled
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                        .is_some();
                    if cancelled {
                        Err("macOS main event loop is unavailable".into())
                    } else {
                        result_rx
                            .recv()
                            .map_err(|_| {
                                "macOS main-thread dispatcher lost an admitted task".to_owned()
                            })?
                    }
                }
            }
        },
        task,
    )
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    task()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn task_runs_inside_dispatcher() {
        use std::sync::{Arc, Mutex};

        let caller_thread = std::thread::current().id();
        let dispatcher_thread = Arc::new(Mutex::new(None));
        let work_thread = Arc::new(Mutex::new(None));
        let dispatcher_thread_seen = dispatcher_thread.clone();
        let work_thread_seen = work_thread.clone();

        let result = crate::macos_main::run_task_with(
            move |task| {
                *dispatcher_thread_seen.lock().unwrap() = Some(std::thread::current().id());
                let handle = std::thread::spawn(task);
                handle.join().expect("dispatched task should not panic")
            },
            move || {
                *work_thread_seen.lock().unwrap() = Some(std::thread::current().id());
                Ok("ok")
            },
        )
        .expect("task should complete");

        assert_eq!(result, "ok");
        assert_eq!(dispatcher_thread.lock().unwrap().unwrap(), caller_thread);
        assert_ne!(
            work_thread.lock().unwrap().unwrap(),
            caller_thread,
            "work must run inside the dispatcher task, not on the caller thread"
        );
    }
}
