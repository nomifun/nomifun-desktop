//! Main-thread dispatch for macOS native APIs with AppKit / input-source
//! affinity. Non-macOS builds run the task inline.

use tokio_util::sync::CancellationToken;

#[cfg(all(test, target_os = "macos"))]
pub(crate) type MainTask<T> = Box<dyn FnOnce() -> Result<T, String> + Send + 'static>;

#[cfg(all(test, target_os = "macos"))]
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
    run_blocking_controlled(&CancellationToken::new(), task)
}

#[cfg(target_os = "macos")]
pub(crate) fn run_blocking_controlled<T, F>(cancellation: &CancellationToken, task: F) -> Result<T, String>
where T: Send + 'static, F: FnOnce() -> Result<T, String> + Send + 'static,
{
    crate::native_control::require_open(cancellation)?;
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
    run_queued_controlled(cancellation, |task| dispatch2::DispatchQueue::main().exec_async(task), task)
}

/// Testable original main-queue slot. Taking the slot begins native admission;
/// a queued cancellation removes it, while an admitted operation stays joined.
#[cfg(any(target_os = "macos", test))]
fn run_queued_controlled<T, D, F>(cancellation: &CancellationToken, enqueue: D, task: F) -> Result<T, String>
where T: Send + 'static, D: FnOnce(Box<dyn FnOnce() + Send>),
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    use std::{sync::{Arc, Mutex, mpsc}, time::{Duration, Instant}};
    crate::native_control::require_open(cancellation)?;
    let scheduled = Arc::new(Mutex::new(Some(task)));
    let queued = scheduled.clone();
    let native_cancellation = cancellation.clone();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    enqueue(Box::new(move || {
        let task = queued.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some(task) = task {
            let result = crate::native_control::require_open(&native_cancellation).and_then(|()| task());
            let _ = result_tx.send(result);
        }
    }));
    // This is the existing one-second main-loop admission deadline, not an
    // operation lifetime. Once native code starts, only its real result ends it.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let cancelled = cancellation.is_cancelled();
        if cancelled || Instant::now() >= deadline {
            if scheduled.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().is_some() {
                return Err(if cancelled { crate::native_control::CANCELLED_BEFORE_DISPATCH.into() }
                    else { "macOS main event loop is unavailable".into() });
            }
            return result_rx.recv().map_err(|_| "macOS main-thread dispatcher lost an admitted task".to_owned())?;
        }
        match result_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(20))) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("macOS main-thread dispatcher closed before completion".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {},
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    run_blocking_controlled(&CancellationToken::new(), task)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn run_blocking_controlled<T, F>(cancellation: &CancellationToken, task: F) -> Result<T, String>
where T: Send + 'static, F: FnOnce() -> Result<T, String> + Send + 'static,
{
    crate::native_control::require_open(cancellation)?;
    task()
}

#[cfg(test)]
mod controlled_queue_tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}, mpsc};

    #[test]
    fn stop_removes_queued_main_task_before_the_event_loop_recovers() {
        let cancellation = CancellationToken::new();
        let (queued, queue) = mpsc::sync_channel(1);
        let effects = Arc::new(AtomicUsize::new(0));
        let seen = effects.clone();
        let worker = std::thread::spawn({ let cancellation = cancellation.clone(); move || {
            run_queued_controlled(&cancellation, |task| queued.send(task).unwrap(), move || {
                seen.fetch_add(1, Ordering::SeqCst); Ok(())
            })
        }});
        let delayed = queue.recv().unwrap();
        cancellation.cancel();
        assert_eq!(worker.join().unwrap().unwrap_err(), crate::native_control::CANCELLED_BEFORE_DISPATCH);
        delayed();
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stop_after_main_dispatch_waits_for_original_native_result() {
        let cancellation = CancellationToken::new();
        let (queued, queue) = mpsc::sync_channel(1);
        let (entered, started) = mpsc::sync_channel(1);
        let (release, finish) = mpsc::sync_channel(1);
        let worker = std::thread::spawn({ let cancellation = cancellation.clone(); move || {
            run_queued_controlled(&cancellation, |task| queued.send(task).unwrap(), move || {
                entered.send(()).unwrap(); finish.recv().unwrap(); Ok("native result")
            })
        }});
        let native = std::thread::spawn(queue.recv().unwrap());
        started.recv().unwrap();
        cancellation.cancel();
        assert!(!worker.is_finished());
        release.send(()).unwrap();
        native.join().unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), "native result");
    }
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
