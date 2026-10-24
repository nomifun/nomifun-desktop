//! macOS process commands retain the existing physical shutdown owner until
//! cleanup and the appropriate development or release handoff are verified.
use std::sync::{Arc, atomic::Ordering};

use tauri::{AppHandle, Manager};

use crate::{ExitCoordinator, MacosHandoffPurpose, EXIT_PHASE_CLEANUP_FAILED,
    EXIT_PHASE_FATAL, EXIT_PHASE_RESTARTING, EXIT_PHASE_SHUTTING_DOWN,
    macos_appkit_main_loop::post_to_main,
    macos_development_launch::{ApplicationExitReporter, development_application_is_eligible},
    macos_process_commands::MacosProcessRequest};

pub(crate) async fn request(app: AppHandle, request: MacosProcessRequest) -> Result<(), String> {
    let (reply, response) = tokio::sync::oneshot::channel();
    post_to_main(move || {
        let result = match request {
            MacosProcessRequest::Exit { code } => {
                if code == tauri::RESTART_EXIT_CODE { Err("PROCESS_EXIT_RESERVED_RESTART_CODE".into()) }
                else { app.exit(code); Ok(()) }
            }
            MacosProcessRequest::Restart => {
                begin_restart(&app, reply);
                return;
            }
        };
        let _ = reply.send(result);
    }).map_err(|_| "DESKTOP_PROCESS_DISPATCH_FAILED".to_owned())?;
    response.await.map_err(|_| "DESKTOP_PROCESS_COMPLETION_LOST".to_owned())?
}

fn begin_restart(app: &AppHandle, reply: tokio::sync::oneshot::Sender<Result<(), String>>) {
    let Some(coordinator) = app.try_state::<Arc<ExitCoordinator>>().map(|state| state.inner().clone()) else {
        let _ = reply.send(Err("DESKTOP_SHUTDOWN_OWNER_UNAVAILABLE".into())); return;
    };
    let Some(reporter) = app.try_state::<ApplicationExitReporter>().map(|state| state.inner().clone()) else {
        let _ = reply.send(Err("DESKTOP_RESTART_OWNER_UNAVAILABLE".into())); return;
    };
    if coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_FATAL
        || coordinator.original_code() != 0 || reporter.terminal_attempt_started()
        || (coordinator.forced_handoff_allowed.load(Ordering::Acquire)
            && !coordinator.cleanup_verified.load(Ordering::Acquire))
    {
        let _ = reply.send(Err("DESKTOP_RESTART_CONFLICTS_WITH_EXIT".into())); return;
    }
    if development_application_is_eligible() && !reporter.has_launch_owner() {
        let _ = reply.send(Err("DESKTOP_DEVELOPMENT_RESTART_REQUIRES_LAUNCH_OWNER".into())); return;
    }
    if crate::macos_appkit_termination::cancel_pending_on_main(app).is_err() {
        let _ = reply.send(Err("DESKTOP_NATIVE_TERMINATION_HANDOFF_UNAVAILABLE".into())); return;
    }
    {
        let Ok(mut handoff) = coordinator.macos_handoff.lock() else {
            let _ = reply.send(Err("DESKTOP_RESTART_OWNER_UNAVAILABLE".into())); return;
        };
        if handoff.committed.is_some()
            || coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_FATAL
            || coordinator.original_code() != 0
            || matches!(handoff.deferred, Some(MacosHandoffPurpose::Exit(code)) if code != 0)
        {
            let _ = reply.send(Err("DESKTOP_EXIT_HANDOFF_ALREADY_COMMITTED".into())); return;
        }
        // Serialize accepted intent against normal handoff commit; a worker
        // may already have pre-read the prior normal purpose.
        coordinator.request_restart();
        if coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_CLEANUP_FAILED {
            coordinator.phase.store(EXIT_PHASE_RESTARTING, Ordering::Release);
        }
        handoff.restart_waiters.push(reply);
    }
    if coordinator.defer_macos_handoff(MacosHandoffPurpose::Restart) { return; }
    if coordinator.cleanup_verified.load(Ordering::Acquire) {
        coordinator.mark_cleanup_verified();
        finish_restart(app, &coordinator);
    } else {
        app.state::<crate::QuitFlag>().0.store(true, Ordering::SeqCst);
        if let Some(server) = app.try_state::<Arc<nomifun_app::DesktopServer>>()
            .map(|state| state.inner().clone()).or_else(|| coordinator.backend_server())
        {
            crate::start_shutdown_if_needed(app, server, coordinator);
        } else {
            crate::spawn_deferred_exit_shutdown(app.clone(), coordinator);
        }
    }
}

pub(crate) fn cancel_restart_for_exit(coordinator: &Arc<ExitCoordinator>, code: Option<i32>) {
    if !coordinator.is_restart_requested() { return; }
    let Ok(handoff) = coordinator.macos_handoff.lock() else { return; };
    if handoff.committed.is_some() { return; }
    if coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_FATAL { return; }
    coordinator.restart_requested.store(false, Ordering::Release);
    if let Some(code) = code {
        if code != 0 && coordinator.original_code() == 0 {
            if let Ok(mut original) = coordinator.original_code.lock() { *original = Some(code); }
        }
    }
    if coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_RESTARTING {
        coordinator.phase.store(EXIT_PHASE_SHUTTING_DOWN, Ordering::Release);
        if let Ok(mut original) = coordinator.original_code.lock() { *original = code; }
    }
    drop(handoff);
    coordinator.notify_macos_restart(Err("DESKTOP_RESTART_CANCELLED_BY_EXIT".into()));
}

pub(crate) fn failed_restart(coordinator: &Arc<ExitCoordinator>) -> bool {
    // Only this application-owned path has not yet called SDK request_restart.
    // Its failed attempt may return to the existing retained cleanup authority.
    let mut handoff = match coordinator.macos_handoff.lock() {
        Ok(handoff) => handoff,
        Err(poisoned) => {
            // Poison never grants a physical handoff. Waiting command futures
            // still receive failure rather than remaining silently pending.
            let mut handoff = poisoned.into_inner();
            let waiters = std::mem::take(&mut handoff.restart_waiters);
            drop(handoff);
            for waiter in waiters { let _ = waiter.send(Err("DESKTOP_RESTART_OWNER_UNAVAILABLE".into())); }
            return true;
        }
    };
    if handoff.restart_handoff_issued { return true; }
    if !coordinator.is_restart_requested() { return false; }
    if handoff.committed == Some(MacosHandoffPurpose::Restart) { handoff.committed = None; }
    coordinator.restart_requested.store(false, Ordering::Release);
    if coordinator.phase.load(Ordering::Acquire) != EXIT_PHASE_FATAL {
        coordinator.phase.store(EXIT_PHASE_CLEANUP_FAILED, Ordering::Release);
    }
    coordinator.shutdown_started.store(false, Ordering::Release);
    drop(handoff);
    coordinator.notify_macos_restart(Err("DESKTOP_RESTART_CLEANUP_UNVERIFIED".into()));
    true
}

struct RestartPreparationOwner(Arc<ExitCoordinator>);
impl Drop for RestartPreparationOwner {
    fn drop(&mut self) {
        // A caught preparation unwind must still complete waiting callers.
        // Already-issued handoffs remain immutable in failed_restart.
        failed_restart(&self.0);
    }
}

pub(crate) fn finish_restart(app: &AppHandle, coordinator: &Arc<ExitCoordinator>) {
    let app = app.clone();
    let coordinator = coordinator.clone();
    let failure_owner = coordinator.clone();
    if post_to_main(move || {
        if !coordinator.is_restart_requested() { return; }
        if coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_FATAL
            || !coordinator.cleanup_verified.load(Ordering::Acquire)
            || coordinator.original_code() != 0
        {
            failed_restart(&coordinator); return;
        }
        if coordinator.defer_macos_handoff(MacosHandoffPurpose::Restart) { return; }
        let Some(reporter) = app.try_state::<ApplicationExitReporter>().map(|state| state.inner().clone()) else {
            failed_restart(&coordinator); return;
        };
        let mut handoff = match coordinator.macos_handoff.lock() {
            Ok(handoff) => handoff,
            Err(poisoned) => { drop(poisoned.into_inner()); failed_restart(&coordinator); return; }
        };
        if handoff.installation_active {
            if !matches!(handoff.deferred, Some(MacosHandoffPurpose::Exit(code)) if code != 0) {
                handoff.deferred = Some(MacosHandoffPurpose::Restart);
            }
            return;
        }
        if coordinator.phase.load(Ordering::Acquire) == EXIT_PHASE_FATAL || coordinator.original_code() != 0 {
            drop(handoff);
            failed_restart(&coordinator);
            return;
        }
        if handoff.committed.is_some() { return; }
        if reporter.terminal_attempt_started() { drop(handoff); failed_restart(&coordinator); return; }
        handoff.committed = Some(MacosHandoffPurpose::Restart);
        drop(handoff);
        let preparation_owner = RestartPreparationOwner(coordinator.clone());
        if development_application_is_eligible() {
            if !reporter.has_launch_owner()
                || reporter.report_exit_once_with_restart(0, true, true).is_err()
            {
                failed_restart(&coordinator); return;
            }
            if coordinator.macos_handoff.lock().map(|mut handoff| handoff.restart_handoff_issued = true).is_err() { return; }
            coordinator.notify_macos_restart(Ok(()));
            // SDK restart would bypass main's receipt and reuse its descriptor.
            // The surviving launcher owns the fresh LaunchServices generation.
            app.exit(0);
        } else {
            if coordinator.macos_handoff.lock().map(|mut handoff| handoff.restart_handoff_issued = true).is_err() { return; }
            coordinator.notify_macos_restart(Ok(()));
            // This is the sole macOS SDK restart site: physical cleanup has
            // already completed, and its sentinel performs no second wait.
            app.request_restart();
        }
        drop(preparation_owner);
    }).is_err() {
        failed_restart(&failure_owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_preparation_unwind_retires_unissued_reservation_and_completes_waiter() {
        let coordinator = Arc::new(ExitCoordinator::default());
        coordinator.request_restart();
        let (reply, mut response) = tokio::sync::oneshot::channel();
        {
            let mut handoff = coordinator.macos_handoff.lock().unwrap();
            handoff.committed = Some(MacosHandoffPurpose::Restart);
            handoff.restart_waiters.push(reply);
        }
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owner = RestartPreparationOwner(coordinator.clone());
            panic!("fixture restart preparation failure");
        }));
        assert!(prepared.is_err());
        assert!(response.try_recv().unwrap().is_err());
        assert!(!coordinator.is_restart_requested());
        assert!(!coordinator.cleanup_verified.load(Ordering::Acquire));
        let handoff = coordinator.macos_handoff.lock().unwrap();
        assert!(handoff.committed.is_none());
        assert!(!handoff.restart_handoff_issued);
    }

    #[test]
    fn issued_restart_preparation_owner_does_not_change_delivered_intent() {
        let coordinator = Arc::new(ExitCoordinator::default());
        coordinator.request_restart();
        {
            let mut handoff = coordinator.macos_handoff.lock().unwrap();
            handoff.committed = Some(MacosHandoffPurpose::Restart);
            handoff.restart_handoff_issued = true;
        }
        drop(RestartPreparationOwner(coordinator.clone()));
        assert!(coordinator.is_restart_requested());
        assert_eq!(coordinator.macos_handoff.lock().unwrap().committed, Some(MacosHandoffPurpose::Restart));
    }

    #[test]
    fn poisoned_restart_owner_completes_waiter_with_failure_without_handoff() {
        let coordinator = Arc::new(ExitCoordinator::default());
        coordinator.request_restart();
        let (reply, mut response) = tokio::sync::oneshot::channel();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut handoff = coordinator.macos_handoff.lock().unwrap();
            handoff.restart_waiters.push(reply);
            panic!("fixture lifecycle owner failure");
        }));
        assert!(poisoned.is_err());
        assert!(failed_restart(&coordinator));
        assert!(response.try_recv().unwrap().is_err());
        assert!(!coordinator.cleanup_verified.load(Ordering::Acquire));
        let handoff = coordinator.macos_handoff.lock().err().unwrap().into_inner();
        assert!(!handoff.restart_handoff_issued);
        assert!(handoff.restart_waiters.is_empty());
    }
}

pub(crate) fn release_installation(app: AppHandle, coordinator: Arc<ExitCoordinator>, result: Result<(), String>) {
    let _ = post_to_main(move || {
        let purpose = coordinator.finish_macos_installation(&result);
        if let Err(error) = result {
            coordinator.notify_macos_restart(Err(error));
        }
        match purpose {
            Some(MacosHandoffPurpose::Restart) => finish_restart(&app, &coordinator),
            Some(MacosHandoffPurpose::Exit(code)) => crate::request_normal_shutdown(&app, coordinator, Some(code)),
            None => {},
        }
    });
}
