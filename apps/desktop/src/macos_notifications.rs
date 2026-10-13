//! Native macOS notifications under the notification plugin's existing ACL.
//!
//! The upstream desktop implementation reports Granted without consulting the
//! OS and borrows Terminal's identity in development. Use the public
//! UserNotifications API with the running application bundle instead. A
//! successful submission means the OS accepted the request; it is not proof
//! that a banner appeared or the user saw it.
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    ptr::NonNull,
    sync::{Arc, Mutex},
    time::Duration,
};

use block2::{DynBlock, RcBlock};
use objc2::{AnyThread, MainThreadMarker, define_class, msg_send, rc::Retained, runtime::{Bool, ProtocolObject}};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotification, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use serde::Deserialize;
use tauri::{AppHandle, Runtime, plugin::{Builder, PermissionState, TauriPlugin}};
use tokio::sync::oneshot;

type NativeResult<T> = Result<T, String>;

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "NomiFunUserNotificationDelegate"]
    #[ivars = ()]
    struct NotificationDelegate;
    unsafe impl NSObjectProtocol for NotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Without this public delegate method UN suppresses foreground
            // presentation. The OS still decides which allowed presentation
            // is appropriate under its authorization/focus/user settings.
            let options = guard_native("foreground presentation", || {
                let mut options = UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List;
                if notification.request().content().sound().is_some() {
                    options |= UNNotificationPresentationOptions::Sound;
                }
                Ok(options)
            }).unwrap_or_else(|_| UNNotificationPresentationOptions::empty());
            // Exactly one invocation attempt. Never retry a completion block
            // after an exception because it may already have consumed it.
            let _ = guard_native("foreground presentation callback", || {
                completion.call((options,));
                Ok(())
            });
        }
    }
);

thread_local! {
    // UN keeps its delegate weak. Its stateless callback can run on the OS
    // queue; the retaining owner is created/released on the main thread.
    static FOREGROUND_DELEGATE: RefCell<Option<Retained<NotificationDelegate>>> = const { RefCell::new(None) };
}

fn ensure_foreground_delegate(center: &UNUserNotificationCenter) -> NativeResult<()> {
    MainThreadMarker::new().ok_or("Notification delegate registration requires the main thread")?;
    FOREGROUND_DELEGATE.with(|owner| {
        let mut owner = owner.borrow_mut();
        if owner.is_none() {
            if center.delegate().is_some() {
                return Err("macOS notification delegate is already owned by another service".into());
            }
            let allocated = NotificationDelegate::alloc().set_ivars(());
            let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(allocated), init] };
            *owner = Some(delegate);
        }
        let delegate = owner.as_ref().ok_or("Notification delegate owner is unavailable")?;
        let protocol = ProtocolObject::from_ref(&**delegate);
        if let Some(current) = center.delegate() {
            if !std::ptr::eq(&*current, protocol) {
                return Err("macOS notification delegate ownership changed".into());
            }
        } else {
            center.setDelegate(Some(protocol));
        }
        if !center.delegate().is_some_and(|current| std::ptr::eq(&*current, protocol)) {
            return Err("macOS notification delegate registration was not verified".into());
        }
        Ok(())
    })
}

/// The old desktop implementation consumed only title/body/icon/sound.
/// Other SDK options (including schedules, IDs and mobile fields) remain
/// accepted and ignored, matching its desktop behavior. On macOS notify-rust
/// never consumed icon; the OS continues to use the actual app bundle icon.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotifyOptions {
    title: Option<String>,
    body: Option<String>,
    #[serde(rename = "icon")]
    _icon: Option<String>,
    sound: Option<String>,
}

/// Only Rust values cross queues. The heap block retained by the OS holds the
/// one-shot sender until callback completion, including after caller drop.
struct CallbackReply<T>(Arc<Mutex<Option<oneshot::Sender<NativeResult<T>>>>>);

impl<T> Clone for CallbackReply<T> {
    fn clone(&self) -> Self { Self(self.0.clone()) }
}

impl<T> CallbackReply<T> {
    fn new() -> (Self, oneshot::Receiver<NativeResult<T>>) {
        let (sender, receiver) = oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(sender)))), receiver)
    }

    fn pending(&self) -> bool {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
            .as_ref().is_some_and(|sender| !sender.is_closed())
    }

    fn finish(&self, result: NativeResult<T>) {
        let sender = self.0.lock().unwrap_or_else(|error| error.into_inner()).take();
        if let Some(sender) = sender { let _ = sender.send(result); }
    }

    fn finish_with(&self, operation: &'static str, work: impl FnOnce() -> NativeResult<T>) {
        self.finish(guard_native(operation, work));
    }
}

/// Catch Objective-C exceptions separately from Rust panics. Neither may
/// unwind through an OS completion block or the public run-loop dispatcher.
fn guard_native<T>(operation: &str, work: impl FnOnce() -> NativeResult<T>) -> NativeResult<T> {
    match catch_unwind(AssertUnwindSafe(|| objc2::exception::catch(AssertUnwindSafe(work)))) {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(format!("macOS notification {operation} raised a native exception; completion is unverified")),
        Err(_) => Err(format!("macOS notification {operation} panicked; completion is unverified")),
    }
}

fn valid_bundle_identity(identifier: Option<&str>, bundle_path: &str) -> bool {
    identifier.is_some_and(|id| !id.is_empty()
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')))
        && Path::new(bundle_path).is_absolute()
        && Path::new(bundle_path).extension().and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

fn require_application_bundle() -> NativeResult<()> {
    // A CLI/test executable must fail before currentNotificationCenter, which
    // requires a real application bundle. Never substitute another app ID.
    let bundle = NSBundle::mainBundle();
    let identifier = bundle.bundleIdentifier().map(|value| value.to_string());
    let path = bundle.bundlePath().to_string();
    if !valid_bundle_identity(identifier.as_deref(), &path) {
        return Err("macOS notifications require the running application bundle and its real identifier".into());
    }
    Ok(())
}

async fn await_callback<T>(
    operation: &str,
    receiver: oneshot::Receiver<NativeResult<T>>,
    deadline: Duration,
) -> NativeResult<T> {
    match tokio::time::timeout(deadline, receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(format!("macOS notification {operation} completion was lost")),
        Err(_) => Err(format!("macOS notification {operation} timed out; completion is unverified")),
    }
}

async fn on_main<T: Send + 'static>(
    operation: &'static str,
    deadline: Duration,
    register: impl FnOnce(CallbackReply<T>) -> NativeResult<()> + Send + 'static,
) -> NativeResult<T> {
    let (reply, receiver) = CallbackReply::new();
    crate::macos_appkit_main_loop::post_to_main(move || {
        // Do not begin a native operation if the queued request was already
        // canceled/timed out. A started OS call cannot be canceled by dropping
        // the caller; its block remains alive and it is never retried here.
        if !reply.pending() { return; }
        let result = guard_native(operation, || {
            require_application_bundle()?;
            register(reply.clone())
        });
        if let Err(error) = result { reply.finish(Err(error)); }
    }).map_err(|error| format!("Could not dispatch macOS notification {operation}: {error}"))?;
    await_callback(operation, receiver, deadline).await
}

fn permission_for_status(status: UNAuthorizationStatus) -> NativeResult<Option<bool>> {
    match status {
        UNAuthorizationStatus::NotDetermined => Ok(None),
        UNAuthorizationStatus::Denied => Ok(Some(false)),
        // Provisional permits non-interruptive delivery. This is access to
        // post a notification, not a guarantee of an interrupting banner.
        UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional => Ok(Some(true)),
        // Ephemeral is an iOS App Clip status and is unavailable on macOS.
        _ => Err(format!("Unknown macOS notification authorization status {}", status.0)),
    }
}

pub(crate) async fn permission_state() -> NativeResult<Option<bool>> {
    on_main("settings query", Duration::from_secs(10), |reply| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            reply.finish_with("settings query", || {
                // The framework owns settings for the callback. Copy only
                // the scalar state; no ObjC pointer leaves this invocation.
                permission_for_status(unsafe { settings.as_ref() }.authorizationStatus())
            });
        });
        center.getNotificationSettingsWithCompletionHandler(&completion);
        Ok(())
    }).await
}

fn callback_error(error: *mut NSError) -> NativeResult<()> {
    // NSError is optional and borrowed for this callback only.
    match unsafe { error.as_ref() } {
        Some(error) => Err(format!("macOS notification request failed: {} ({}:{})",
            error.localizedDescription().to_string().chars().take(512).collect::<String>(),
            error.domain(), error.code())),
        None => Ok(()),
    }
}

#[tauri::command]
async fn is_permission_granted() -> NativeResult<Option<bool>> {
    permission_state().await
}

#[tauri::command]
async fn request_permission() -> NativeResult<PermissionState> {
    on_main("authorization request", Duration::from_secs(300), |reply| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let completion = RcBlock::new(move |_granted: Bool, error: *mut NSError| {
            reply.finish_with("authorization request", || callback_error(error));
        });
        // Only this explicit IPC requests permission. Delivery and status
        // queries never request it or retry it. No badges/critical privileges
        // are requested because this desktop API does not use them.
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &completion);
        Ok(())
    }).await?;
    // Read actual settings after the request instead of guessing from a
    // callback boolean when the OS did not establish a permission state.
    Ok(match permission_state().await? {
        Some(true) => PermissionState::Granted,
        Some(false) => PermissionState::Denied,
        None => PermissionState::Prompt,
    })
}

#[tauri::command]
async fn notify<R: Runtime>(app: AppHandle<R>, options: NotifyOptions) -> NativeResult<()> {
    match permission_state().await? {
        Some(true) => {},
        Some(false) => return Err("macOS notification permission is denied".into()),
        None => return Err("macOS notification permission has not been requested".into()),
    }
    let title = options.title.or_else(|| app.config().product_name.clone()).unwrap_or_default();
    let body = options.body.unwrap_or_default();
    let sound = options.sound;
    on_main("submission", Duration::from_secs(30), move |reply| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        ensure_foreground_delegate(&center)?;
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));
        if let Some(sound) = sound.filter(|sound| !sound.is_empty()) {
            let sound = if sound == "NSUserNotificationDefaultSoundName" {
                UNNotificationSound::defaultSound()
            } else {
                UNNotificationSound::soundNamed(&NSString::from_str(&sound))
            };
            content.setSound(Some(&sound));
        }
        // The old desktop sender created a fresh notification for every
        // call, irrespective of the optional mobile notification ID.
        let identifier = NSString::from_str(&uuid::Uuid::now_v7().to_string());
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
        let completion = RcBlock::new(move |error: *mut NSError| {
            reply.finish_with("submission", || callback_error(error));
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
        Ok(())
    }).await
}

// Compatibility for the official guest API in first-party documents. The
// wrapper applies its existing top-level/main-or-companion guard. Product
// code queries IPC directly; this shim also refreshes real state on focus.
const GUEST_SCRIPT: &str = r#"
(() => {
  const invoke = (command, payload = {}) => window.__TAURI_INTERNALS__.invoke(`plugin:notification|${command}`, payload);
  let permission = 'default';
  let version = 0;
  const query = async () => {
    const current = ++version;
    try {
      const state = await invoke('is_permission_granted');
      if (state !== null && state !== true && state !== false) throw new Error('Invalid native notification permission state');
      const next = state === null ? 'default' : state ? 'granted' : 'denied';
      if (version === current) permission = next;
      return next;
    } catch (error) {
      if (version === current) permission = 'default';
      throw error;
    }
  };
  window.Notification = function(title, options) {
    void invoke('notify', { options: { ...(options || {}), title } }).catch(error => console.error('Native notification submission failed', error));
  };
  Object.defineProperty(window.Notification, 'permission', { enumerable: true, get: () => permission });
  window.Notification.requestPermission = async () => {
    const current = ++version;
    try {
      const state = await invoke('request_permission');
      if (!['granted', 'denied', 'prompt', 'prompt-with-rationale'].includes(state)) throw new Error('Invalid native notification request state');
      if (version === current) permission = state === 'granted' || state === 'denied' ? state : 'default';
      return state;
    } catch (error) {
      if (version === current) permission = 'default';
      throw error;
    }
  };
  window.addEventListener('focus', () => { void query().catch(() => {}); });
  void query().catch(() => {});
})();
"#;

pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("notification")
        .invoke_handler(tauri::generate_handler![notify, request_permission, is_permission_granted])
        .js_init_script(GUEST_SCRIPT)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_non_macos_authorization_never_become_granted() {
        assert_eq!(permission_for_status(UNAuthorizationStatus::NotDetermined).unwrap(), None);
        assert_eq!(permission_for_status(UNAuthorizationStatus::Denied).unwrap(), Some(false));
        assert_eq!(permission_for_status(UNAuthorizationStatus::Authorized).unwrap(), Some(true));
        assert_eq!(permission_for_status(UNAuthorizationStatus::Provisional).unwrap(), Some(true));
        for status in [-1, 4, 5, isize::MAX] {
            assert!(permission_for_status(UNAuthorizationStatus(status)).is_err());
        }
    }

    #[test]
    fn bare_process_and_fabricated_notification_identity_are_rejected() {
        assert!(valid_bundle_identity(Some("com.nomifun.desktop.dev"), "/owned/NomiFun Dev.app"));
        for (id, path) in [(None, "/owned/NomiFun.app"), (Some(""), "/owned/NomiFun.app"),
            (Some("com.test&target=Terminal"), "/owned/NomiFun.app"),
            (Some("com.test"), "/owned/target/debug/nomifun-desktop"), (Some("com.test"), "NomiFun.app")] {
            assert!(!valid_bundle_identity(id, path));
        }
    }

    #[tokio::test]
    async fn callback_error_is_preserved_when_a_later_completion_claims_success() {
        let (reply, receiver) = CallbackReply::<()>::new();
        reply.finish(Err("first native rejection".into()));
        reply.finish(Ok(()));
        assert_eq!(receiver.await.unwrap(), Err("first native rejection".into()));
        assert!(!reply.pending());
    }

    #[tokio::test]
    async fn callback_panic_rejects_instead_of_crossing_ffi_or_hanging_the_waiter() {
        let (reply, receiver) = CallbackReply::<()>::new();
        reply.finish_with("fixture", || panic!("pure callback fixture"));
        let result = receiver.await.unwrap();
        assert!(result.unwrap_err().contains("completion is unverified"));
    }

    #[tokio::test]
    async fn canceled_caller_closes_admission_and_accepts_a_late_callback_safely() {
        let (reply, receiver) = CallbackReply::<()>::new();
        assert!(reply.pending());
        drop(receiver);
        assert!(!reply.pending());
        reply.finish(Ok(()));
        reply.finish(Err("late second callback".into()));
    }

    #[tokio::test(start_paused = true)]
    async fn missing_callback_times_out_unverified_without_changing_it_to_success() {
        let (reply, receiver) = CallbackReply::<()>::new();
        let result = await_callback("fixture", receiver, Duration::from_secs(10)).await;
        assert!(result.unwrap_err().contains("timed out; completion is unverified"));
        assert!(!reply.pending());
        reply.finish(Ok(()));
    }

    #[tokio::test]
    async fn dropped_callback_sender_rejects_the_waiter() {
        let (reply, receiver) = CallbackReply::<()>::new();
        drop(reply);
        let result = await_callback("fixture", receiver, Duration::from_secs(10)).await;
        assert!(result.unwrap_err().contains("completion was lost"));
    }

    #[test]
    fn existing_desktop_options_accept_the_sdk_extended_payload() {
        let options: NotifyOptions = serde_json::from_value(serde_json::json!({
            "title": "中文Ω😀", "body": "body", "icon": "/old-ignored-icon.png", "sound": "Ping",
            "id": 42, "schedule": { "at": { "date": "2026-10-13T00:00:00Z" } },
            "attachments": [], "extra": { "fixture": true }, "silent": true
        })).unwrap();
        assert_eq!(options.title.as_deref(), Some("中文Ω😀"));
        assert_eq!(options.body.as_deref(), Some("body"));
        assert_eq!(options.sound.as_deref(), Some("Ping"));
        assert_eq!(options._icon.as_deref(), Some("/old-ignored-icon.png"));
    }
}
