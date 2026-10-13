//! Public AppKit termination admission without replacing Tao's delegate.
//!
//! Tao's Dock API reads ivars on the actual delegate. Keep that object, its
//! class and all existing methods intact; add only the absent native admission
//! method. Backend cleanup remains owned by the existing ExitCoordinator.
use std::{
    cell::RefCell,
    ffi::{CStr, CString},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}},
    time::Duration,
};

use anyhow::{Context, bail};
use objc2::{
    MainThreadMarker, encode::Encode, ffi, rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, NSObjectProtocol, ProtocolObject, Sel}, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSApplicationTerminateReply,
};
use tauri::{AppHandle, Manager};

use crate::{ExitCoordinator, macos_appkit_main_loop::post_to_main, macos_development_launch::ApplicationExitReporter};

const NATIVE_REPLY_DEADLINE: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Token(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Admission {
    New(Token),
    Pending(Token),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Completion {
    Queue(Token),
    AlreadyQueued,
    NoPending,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    token: Token,
    completion_queued: bool,
}

#[derive(Default)]
struct PendingState {
    last_token: u64,
    pending: Option<Pending>,
}

impl PendingState {
    fn admit(&mut self) -> Result<Admission, &'static str> {
        if let Some(pending) = self.pending {
            return Ok(Admission::Pending(pending.token));
        }
        let next = self.last_token.checked_add(1).ok_or("native termination token exhausted")?;
        let token = Token(next);
        self.last_token = next;
        self.pending = Some(Pending { token, completion_queued: false });
        Ok(Admission::New(token))
    }

    fn queue_completion(&mut self) -> Completion {
        match self.pending.as_mut() {
            None => Completion::NoPending,
            Some(pending) if pending.completion_queued => Completion::AlreadyQueued,
            Some(pending) => {
                pending.completion_queued = true;
                Completion::Queue(pending.token)
            }
        }
    }

    fn consume(&mut self, token: Token) -> bool {
        if self.pending.is_some_and(|pending| pending.token == token) {
            self.pending = None;
            true
        } else {
            false
        }
    }

    fn completion_dispatch_failed(&mut self, token: Token) {
        if let Some(pending) = self.pending.as_mut().filter(|pending| pending.token == token) {
            pending.completion_queued = false;
        }
    }

    fn token(&self) -> Option<Token> { self.pending.map(|pending| pending.token) }
}

struct BridgeState {
    pending: Mutex<PendingState>,
    coordinator: Arc<ExitCoordinator>,
    reporter: ApplicationExitReporter,
    installed: AtomicBool,
}

// Retained AppKit objects never enter the cross-thread managed BridgeState.
struct NativeOwner {
    application: Retained<NSApplication>,
    delegate: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
    app: AppHandle,
    state: Arc<BridgeState>,
}

thread_local! {
    static NATIVE_OWNER: RefCell<Option<NativeOwner>> = const { RefCell::new(None) };
}

struct InstalledClass {
    class: &'static AnyClass,
    recipient: usize,
}

static INSTALLED_CLASS: OnceLock<InstalledClass> = OnceLock::new();

type AdmissionImp = unsafe extern "C-unwind" fn(
    &AnyObject, Sel, &NSApplication,
) -> NSApplicationTerminateReply;

fn admission_imp() -> Imp {
    // SAFETY: Objective-C stores a type-erased IMP. Its exact receiver, _cmd,
    // object argument and NSUInteger return ABI are recorded by the encoding.
    unsafe { std::mem::transmute::<AdmissionImp, Imp>(application_should_terminate) }
}

fn method_encoding() -> CString {
    // This is Q@:@ on both supported 64-bit macOS architectures. Derive it
    // from objc2 rather than encoding a BOOL or a platform-dependent C int.
    CString::new(format!("{}{}{}{}", NSApplicationTerminateReply::ENCODING,
        <*mut AnyObject>::ENCODING, Sel::ENCODING, <&NSApplication>::ENCODING))
        .expect("Objective-C type encodings contain no NUL")
}

fn verified_method(class: &AnyClass, encoding: &CStr) -> bool {
    let Some(method) = class.instance_method(sel!(applicationShouldTerminate:)) else { return false; };
    if !std::ptr::fn_addr_eq(method.implementation(), admission_imp()) { return false; }
    let actual = unsafe { ffi::method_getTypeEncoding(method) };
    // SAFETY: A non-null runtime method encoding is a runtime-owned C string.
    !actual.is_null() && unsafe { CStr::from_ptr(actual) } == encoding
        && method.arguments_count() == 3
}

pub fn install(
    app: &AppHandle,
    coordinator: Arc<ExitCoordinator>,
    reporter: ApplicationExitReporter,
) -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new().context("AppKit termination installation requires main thread")?;
    let application = NSApplication::sharedApplication(mtm);
    let delegate = application.delegate().context("AppKit application delegate is unavailable")?;
    let object: &AnyObject = (&*delegate).as_ref();
    let class = object.class();
    let recipient = object as *const AnyObject as usize;
    let encoding = method_encoding();

    if let Some(installed) = INSTALLED_CLASS.get() {
        if !std::ptr::eq(installed.class, class) || installed.recipient != recipient
            || !verified_method(class, &encoding) {
            bail!("AppKit termination bridge was already installed for a different delegate");
        }
        let same_owner = NATIVE_OWNER.with(|owner| {
            owner.try_borrow().ok().and_then(|owner| owner.as_ref().map(|owner| {
                Arc::ptr_eq(&owner.state.coordinator, &coordinator)
                    && owner.state.installed.load(Ordering::Acquire)
                    && std::ptr::eq::<NSApplication>(&*owner.application, &*application)
            })).unwrap_or(false)
        });
        if !same_owner { bail!("AppKit termination bridge owner is unavailable or changed"); }
        return Ok(());
    }

    // class_addMethod itself does not reject inherited methods. Inspect both
    // the superclass chain and dynamic/forwarded responses before adding.
    if class.instance_method(sel!(applicationShouldTerminate:)).is_some()
        || delegate.respondsToSelector(sel!(applicationShouldTerminate:)) {
        bail!("AppKit delegate already handles native termination; refusing to replace its handler");
    }
    if app.try_state::<Arc<BridgeState>>().is_some() {
        bail!("AppKit termination state already exists without a verified installation");
    }
    let state = Arc::new(BridgeState {
        pending: Mutex::new(PendingState::default()), coordinator, reporter,
        installed: AtomicBool::new(false),
    });
    if !app.manage(state.clone()) { bail!("AppKit termination state registration failed"); }
    NATIVE_OWNER.with(|owner| -> anyhow::Result<()> {
        let mut owner = owner.try_borrow_mut().context("AppKit termination owner is borrowed")?;
        if owner.is_some() { bail!("AppKit termination owner already exists"); }
        *owner = Some(NativeOwner { application, delegate, app: app.clone(), state: state.clone() });
        Ok(())
    })?;

    // SAFETY: The registered runtime class is obtained from the actual live
    // delegate, the selector is absent including its superclass chain, and
    // the IMP/encoding describe the exact public delegate method. This adds
    // no ivars and changes no existing implementation, delegate, class or isa.
    let added = unsafe { ffi::class_addMethod((class as *const AnyClass).cast_mut(),
        sel!(applicationShouldTerminate:), admission_imp(), encoding.as_ptr()) }.as_bool();
    if !added { bail!("AppKit native termination admission method could not be added"); }
    if !verified_method(class, &encoding) {
        bail!("AppKit native termination admission ABI could not be verified");
    }
    INSTALLED_CLASS.set(InstalledClass { class, recipient })
        .map_err(|_| anyhow::anyhow!("AppKit termination class installation raced"))?;
    state.installed.store(true, Ordering::Release);
    // Publicly reannounce the same live delegate after adding its optional
    // method. AppKit may cache delegate capabilities when setDelegate runs.
    // Never install nil or another object, and verify recipient/class again.
    let (application, delegate) = NATIVE_OWNER.with(|owner| -> anyhow::Result<_> {
        let owner = owner.try_borrow().context("AppKit termination owner is borrowed")?;
        let owner = owner.as_ref().context("AppKit termination owner is unavailable")?;
        Ok((owner.application.clone(), owner.delegate.clone()))
    })?;
    application.setDelegate(Some(&delegate));
    let current = application.delegate().context("AppKit termination delegate reannouncement lost its recipient")?;
    let current_object: &AnyObject = (&*current).as_ref();
    if Retained::as_ptr(&current) != Retained::as_ptr(&delegate)
        || !std::ptr::eq(current_object.class(), class) || !verified_method(class, &encoding) {
        state.installed.store(false, Ordering::Release);
        bail!("AppKit termination delegate identity changed during reannouncement");
    }
    tracing::info!("public AppKit native termination admission installed on the existing delegate");
    Ok(())
}

fn managed(app: &AppHandle) -> Option<Arc<BridgeState>> {
    app.try_state::<Arc<BridgeState>>().map(|state| state.inner().clone())
}

pub fn has_pending(app: &AppHandle) -> bool {
    managed(app).is_some_and(|state| state.pending.lock()
        .map(|pending| pending.token().is_some()).unwrap_or(true))
}

fn main_application(state: &Arc<BridgeState>) -> anyhow::Result<Retained<NSApplication>> {
    let mtm = MainThreadMarker::new().context("AppKit termination reply requires main thread")?;
    NATIVE_OWNER.with(|owner| {
        let owner = owner.try_borrow().context("AppKit termination owner is borrowed")?;
        let owner = owner.as_ref().context("AppKit termination owner is unavailable")?;
        if !Arc::ptr_eq(&owner.state, state) || !state.installed.load(Ordering::Acquire) {
            bail!("AppKit termination owner does not match the pending request");
        }
        let current = NSApplication::sharedApplication(mtm);
        let delegate = current.delegate().context("AppKit termination delegate was removed")?;
        if !std::ptr::eq::<NSApplication>(&*current, &*owner.application)
            || Retained::as_ptr(&delegate) != Retained::as_ptr(&owner.delegate) {
            bail!("AppKit termination recipient changed before reply");
        }
        Ok(owner.application.clone())
    })
}

pub fn cancel_pending_on_main(app: &AppHandle) -> anyhow::Result<()> {
    let Some(state) = managed(app) else { return Ok(()); };
    let token = state.pending.lock().map_err(|_| anyhow::anyhow!("AppKit pending state poisoned"))?.token();
    let Some(token) = token else { return Ok(()); };
    let application = main_application(&state)?;
    // Keep the deadline witness until AppKit has accepted the cancellation.
    // A panic during native reply must not strand an outstanding Later wait.
    application.replyToApplicationShouldTerminate(false);
    let consumed = state.pending.lock().map_err(|_| anyhow::anyhow!("AppKit pending state poisoned"))?.consume(token);
    if consumed {
        tracing::info!(token = token.0, "cancelling pending native termination for the existing Tauri handoff");
    }
    Ok(())
}

fn schedule_deadline(app: AppHandle, state: Arc<BridgeState>, token: Token) -> std::io::Result<()> {
    std::thread::Builder::new().name("nomifun-appkit-reply-deadline".into()).spawn(move || {
        std::thread::sleep(NATIVE_REPLY_DEADLINE);
        let finish_state = state.clone();
        if let Err(error) = post_to_main(move || {
            let pending = finish_state.pending.lock().ok().and_then(|pending| pending.token());
            if pending != Some(token) { return; }
            let application = match main_application(&finish_state) {
                Ok(application) => application,
                Err(error) => { tracing::error!(%error, "native termination deadline cannot reach its exact recipient"); return; }
            };
            application.replyToApplicationShouldTerminate(false);
            let consumed = finish_state.pending.lock().map(|mut pending| pending.consume(token)).unwrap_or(false);
            if consumed {
                tracing::error!(token = token.0, deadline_secs = NATIVE_REPLY_DEADLINE.as_secs(),
                    "native termination wait timed out; cancelling AppKit request and retaining cleanup authority");
            }
        }) {
            tracing::error!(%error, "native termination deadline dispatch failed; cleanup remains unverified");
        }
        // Keep only Rust owners on the worker; no main-thread-only object is
        // retained or touched here, and timeout grants no exit authority.
        drop(app);
    }).map(|_| ())
}

unsafe extern "C-unwind" fn application_should_terminate(
    receiver: &AnyObject, selector: Sel, application: &NSApplication,
) -> NSApplicationTerminateReply {
    let armed = RefCell::new(None::<(Arc<BridgeState>, Token)>);
    let outcome = catch_unwind(AssertUnwindSafe(|| -> anyhow::Result<NSApplicationTerminateReply> {
        MainThreadMarker::new().context("native termination arrived off the main thread")?;
        if selector != sel!(applicationShouldTerminate:) { bail!("unexpected native termination selector"); }
        let (app, state) = NATIVE_OWNER.with(|owner| -> anyhow::Result<_> {
            let owner = owner.try_borrow().context("native termination owner is borrowed")?;
            let owner = owner.as_ref().context("native termination owner is unavailable")?;
            let object: &AnyObject = (&*owner.delegate).as_ref();
            if !std::ptr::eq(receiver, object) || !std::ptr::eq::<NSApplication>(application, &*owner.application)
                || !owner.state.installed.load(Ordering::Acquire) {
                bail!("native termination does not belong to the installed recipient");
            }
            Ok((owner.app.clone(), owner.state.clone()))
        })?;
        main_application(&state)?;
        if state.coordinator.is_restart_requested() {
            tracing::info!("native termination cancelled because Tauri already owns restart");
            return Ok(NSApplicationTerminateReply::TerminateCancel);
        }
        let admission = state.pending.lock().map_err(|_| anyhow::anyhow!("AppKit pending state poisoned"))?
            .admit().map_err(anyhow::Error::msg)?;
        match admission {
            Admission::Pending(token) => {
                tracing::info!(token = token.0, "native termination already awaits the same cleanup");
            }
            Admission::New(token) => {
                *armed.borrow_mut() = Some((state.clone(), token));
                schedule_deadline(app.clone(), state.clone(), token)
                    .context("native termination deadline could not start")?;
                tracing::info!(token = token.0, "native AppKit termination deferred for verified desktop cleanup");
                crate::request_macos_appkit_shutdown(&app, &state.coordinator);
            }
        }
        Ok(NSApplicationTerminateReply::TerminateLater)
    }));
    match outcome {
        Ok(Ok(reply)) => reply,
        failure => {
            // We return Cancel, so AppKit has no outstanding Later reply for
            // this admission. Disarm its worker before returning to AppKit.
            if let Some((state, token)) = armed.into_inner() {
                if let Ok(mut pending) = state.pending.lock() { pending.consume(token); }
            }
            // Diagnostics themselves must not unwind out of the native IMP.
            let _ = catch_unwind(AssertUnwindSafe(|| match failure {
                Ok(Err(error)) => tracing::error!(%error, "native termination admission failed closed"),
                Err(_) => tracing::error!("native termination admission panicked and was cancelled"),
                Ok(Ok(_)) => unreachable!(),
            }));
            NSApplicationTerminateReply::TerminateCancel
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FinishDecision { NativeYes, Tauri(i32), RestartOwned }

fn finish_decision(cleanup_verified: bool, restarting: bool, final_code: i32, receipt_sent: bool) -> FinishDecision {
    if restarting {
        // Restart owns its existing sentinel and final handoff. Even requesting
        // app.exit(RESTART_EXIT_CODE) again may replace or repeat that handoff.
        FinishDecision::RestartOwned
    } else if cleanup_verified && final_code == 0 && receipt_sent {
        FinishDecision::NativeYes
    } else {
        // A failed receipt must never become AppKit's successful kernel exit.
        FinishDecision::Tauri(if final_code == 0 && (!cleanup_verified || !receipt_sent) { 1 } else { final_code })
    }
}

fn normal_handoff_decision(coordinator: &ExitCoordinator, requested_code: i32) -> FinishDecision {
    if coordinator.is_restart_requested() {
        FinishDecision::RestartOwned
    } else {
        FinishDecision::Tauri(coordinator.process_exit_code(requested_code))
    }
}

fn normal_handoff_on_main(app: &AppHandle, coordinator: &ExitCoordinator, code: i32) {
    if let FinishDecision::Tauri(code) = normal_handoff_decision(coordinator, code) {
        app.exit(code);
    }
}

pub fn complete_or_exit(app: &AppHandle, coordinator: &Arc<ExitCoordinator>, code: i32) {
    let Some(state) = managed(app) else {
        let app = app.clone();
        let coordinator = coordinator.clone();
        if let Err(error) = post_to_main(move || normal_handoff_on_main(&app, &coordinator, code)) {
            tracing::error!(%error, "desktop final exit dispatch failed; refusing unverified handoff");
        }
        return;
    };
    if !Arc::ptr_eq(&state.coordinator, coordinator) {
        tracing::error!("native termination completion has a different cleanup owner; refusing exit");
        return;
    }
    let completion = match state.pending.lock() {
        Ok(mut pending) => pending.queue_completion(),
        Err(_) => { tracing::error!("native termination pending state is poisoned; refusing exit"); return; }
    };
    match completion {
        Completion::NoPending => {
            let app = app.clone();
            let coordinator = coordinator.clone();
            if let Err(error) = post_to_main(move || normal_handoff_on_main(&app, &coordinator, code)) {
                tracing::error!(%error, "desktop final exit dispatch failed; refusing unverified handoff");
            }
        },
        Completion::AlreadyQueued => {},
        Completion::Queue(token) => {
            let app = app.clone();
            let finish_state = state.clone();
            if let Err(error) = post_to_main(move || finish_on_main(app, finish_state, token, code)) {
                if let Ok(mut pending) = state.pending.lock() { pending.completion_dispatch_failed(token); }
                tracing::error!(%error, "native termination completion dispatch failed; retaining pending request");
            }
        }
    }
}

fn finish_on_main(app: AppHandle, state: Arc<BridgeState>, token: Token, requested_code: i32) {
    match state.coordinator.claim_macos_terminal_handoff(crate::MacosHandoffPurpose::Exit(requested_code)) {
        crate::MacosTerminalAdmission::Held => {
            // Its continuation will requeue this exact reply after install.
            if let Ok(mut pending) = state.pending.lock() { pending.completion_dispatch_failed(token); }
            return;
        }
        crate::MacosTerminalAdmission::Restart => {
            crate::macos_lifecycle::finish_restart(&app, &state.coordinator);
            return;
        }
        crate::MacosTerminalAdmission::Normal => {},
    }
    let application = match main_application(&state) {
        Ok(application) => application,
        Err(error) => { tracing::error!(%error, "native termination completion cannot reach its exact recipient"); return; }
    };
    let current = match state.pending.lock() {
        Ok(pending) => pending.token(),
        Err(_) => { tracing::error!("native termination completion state is poisoned"); return; }
    };
    if current != Some(token) {
        if current.is_none() {
            // Its native request was cancelled before this queued result.
            // Recheck restart ownership before any normal exit handoff.
            normal_handoff_on_main(&app, &state.coordinator, requested_code);
        }
        // A later request owns another token: never reply to it here.
        return;
    }
    let verified = state.coordinator.cleanup_verified.load(Ordering::Acquire);
    // The canonical shell exit calculation also preserves fatal failures whose
    // stored normal code is zero. Native termination must not bypass it.
    let final_code = state.coordinator.process_exit_code(requested_code);
    let restarting = state.coordinator.is_restart_requested();
    let eligible = verified && !restarting && final_code == 0;
    let receipt_sent = if eligible {
        match state.reporter.report_exit_once(0, true) {
            Ok(()) => true,
            Err(error) => { tracing::error!(%error, "verified cleanup receipt failed; refusing native successful termination"); false }
        }
    } else { true };
    let decision = finish_decision(verified, restarting, final_code, receipt_sent);
    // All fallible preparation happens while the exact pending token remains
    // available to its deadline. The shared main-loop helper catches a Rust
    // panic, so consuming before preparation/reply would orphan AppKit's wait.
    tracing::info!(token = token.0, cleanup_verified = verified, ?decision, "native AppKit termination reply prepared");
    match decision {
        FinishDecision::NativeYes => application.replyToApplicationShouldTerminate(true),
        FinishDecision::Tauri(_) => application.replyToApplicationShouldTerminate(false),
        FinishDecision::RestartOwned => application.replyToApplicationShouldTerminate(false),
    }
    // Native YES normally exits the process without returning. If a reply
    // returns, its pending request is now settled and may be consumed once.
    if let Ok(mut pending) = state.pending.lock() { pending.consume(token); }
    if let FinishDecision::Tauri(code) = decision {
        normal_handoff_on_main(&app, &state.coordinator, code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appkit_termination_duplicate_request_and_completion_are_single_flight() {
        let mut pending = PendingState::default();
        assert_eq!(pending.admit(), Ok(Admission::New(Token(1))));
        assert_eq!(pending.admit(), Ok(Admission::Pending(Token(1))));
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(1)));
        assert_eq!(pending.queue_completion(), Completion::AlreadyQueued);
        assert!(pending.consume(Token(1)));
        assert!(!pending.consume(Token(1)));
        assert_eq!(pending.queue_completion(), Completion::NoPending);
    }

    #[test]
    fn appkit_termination_late_completion_cannot_consume_successor_request() {
        let mut pending = PendingState::default();
        pending.admit().unwrap();
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(1)));
        assert!(pending.consume(Token(1))); // timeout or restart cancellation
        assert_eq!(pending.admit(), Ok(Admission::New(Token(2))));
        assert!(!pending.consume(Token(1)));
        assert_eq!(pending.token(), Some(Token(2)));
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(2)));
    }

    #[test]
    fn appkit_termination_token_exhaustion_never_wraps_or_reuses() {
        let mut pending = PendingState { last_token: u64::MAX - 1, pending: None };
        assert_eq!(pending.admit(), Ok(Admission::New(Token(u64::MAX))));
        assert!(pending.consume(Token(u64::MAX)));
        assert!(pending.admit().is_err());
        assert_eq!(pending.last_token, u64::MAX);
        assert_eq!(pending.token(), None);
    }

    #[test]
    fn appkit_termination_native_yes_requires_verified_cleanup_and_receipt() {
        assert_eq!(finish_decision(true, false, 0, true), FinishDecision::NativeYes);
        assert_eq!(finish_decision(false, false, 1, true), FinishDecision::Tauri(1));
        assert_eq!(finish_decision(false, false, 0, true), FinishDecision::Tauri(1));
        assert_eq!(finish_decision(true, false, 0, false), FinishDecision::Tauri(1));
        assert_eq!(finish_decision(true, false, 7, true), FinishDecision::Tauri(7));
        assert_eq!(finish_decision(true, true, tauri::RESTART_EXIT_CODE, true), FinishDecision::RestartOwned);
        assert_eq!(finish_decision(true, true, 0, true), FinishDecision::RestartOwned);
    }

    #[test]
    fn appkit_termination_actual_forced_coordinator_never_grants_native_success() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.request_normal_exit(Some(0)));
        coordinator.allow_unverified_handoff();
        assert!(coordinator.is_exit_allowed());
        let verified = coordinator.cleanup_verified.load(Ordering::Acquire);
        assert!(!verified);
        assert_eq!(coordinator.final_exit_code(), 1);
        assert_eq!(finish_decision(verified, coordinator.is_restart_requested(), coordinator.final_exit_code(), true), FinishDecision::Tauri(1));
    }

    #[test]
    fn appkit_termination_failed_dispatch_retries_only_its_current_token() {
        let mut pending = PendingState::default();
        pending.admit().unwrap();
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(1)));
        pending.completion_dispatch_failed(Token(1));
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(1)));
        assert!(pending.consume(Token(1)));
        pending.admit().unwrap();
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(2)));
        pending.completion_dispatch_failed(Token(1));
        assert_eq!(pending.queue_completion(), Completion::AlreadyQueued);
    }

    #[test]
    fn appkit_termination_queued_normal_completion_cannot_replace_restart() {
        let coordinator = ExitCoordinator::default();
        let mut pending = PendingState::default();
        assert!(coordinator.request_normal_exit(Some(0)));
        assert_eq!(pending.admit(), Ok(Admission::New(Token(1))));
        assert_eq!(pending.queue_completion(), Completion::Queue(Token(1)));
        // Restart may take ownership after this normal completion was queued.
        // The normal phase makes its CAS return false, but the authoritative
        // restart flag still becomes true, as in the actual shell contract.
        assert!(!coordinator.request_restart());
        assert!(coordinator.is_restart_requested());
        assert!(pending.consume(Token(1))); // native cancellation for restart
        coordinator.mark_cleanup_verified();
        assert!(!pending.consume(Token(1))); // queued normal callback is late
        assert_eq!(normal_handoff_decision(&coordinator, 0), FinishDecision::RestartOwned);
        assert_eq!(finish_decision(true, coordinator.is_restart_requested(),
            coordinator.process_exit_code(0), true), FinishDecision::RestartOwned);
        assert_eq!(pending.token(), None);
    }

    #[test]
    fn appkit_termination_verified_fatal_cleanup_preserves_failure_exit() {
        let coordinator = ExitCoordinator::default();
        coordinator.mark_cleanup_verified();
        assert!(coordinator.claim_fatal_exit());
        assert_eq!(coordinator.final_exit_code(), 0);
        let code = coordinator.process_exit_code(0);
        assert_eq!(code, 1);
        assert_eq!(finish_decision(true, false, code, true), FinishDecision::Tauri(1));
        assert_eq!(normal_handoff_decision(&coordinator, 0), FinishDecision::Tauri(1));
    }

    #[test]
    fn appkit_termination_preparation_panic_preserves_deadline_token() {
        let pending = Mutex::new(PendingState::default());
        assert_eq!(pending.lock().unwrap().admit(), Ok(Admission::New(Token(1))));
        assert_eq!(pending.lock().unwrap().queue_completion(), Completion::Queue(Token(1)));
        let preparation = catch_unwind(AssertUnwindSafe(|| {
            assert_eq!(pending.lock().unwrap().token(), Some(Token(1)));
            // Receipt/logging preparation may unwind outside the state lock.
            panic!("fixture reply preparation failure");
        }));
        assert!(preparation.is_err());
        let mut pending = pending.lock().unwrap();
        assert_eq!(pending.token(), Some(Token(1)));
        assert!(pending.consume(Token(1))); // deadline can still cancel it
        assert_eq!(pending.admit(), Ok(Admission::New(Token(2))));
        assert!(!pending.consume(Token(1))); // old deadline cannot cancel new
        assert_eq!(pending.token(), Some(Token(2)));
    }

    #[test]
    fn appkit_termination_admission_encoding_uses_unsigned_native_reply() {
        assert_eq!(method_encoding().to_bytes(), b"Q@:@");
        assert_eq!(std::mem::size_of::<NSApplicationTerminateReply>(), std::mem::size_of::<usize>());
    }
}
