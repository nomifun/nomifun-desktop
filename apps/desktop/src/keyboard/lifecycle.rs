//! Native session/power observation. A resume only re-arms the application
//! policy; the device service still requires old keys to be released.

use std::sync::Arc;
type Callback = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Default)]
struct PauseState {
    sleeping: bool,
    inactive: bool,
    locked: bool,
    shutting_down: bool,
}
impl PauseState {
    fn paused(&self) -> bool {
        self.sleeping || self.inactive || self.locked || self.shutting_down
    }
}

pub struct LifecycleGuard {
    _observer: platform::Guard,
}

pub fn install(
    app: &tauri::AppHandle,
    callback: impl Fn(bool) + Send + Sync + 'static,
) -> anyhow::Result<LifecycleGuard> {
    let callback: Callback = Arc::new(callback);
    // Capture remains disabled until the native subscription and initial
    // session state are known. Failure leaves this pause in force.
    callback(true);
    platform::install(app, callback).map(|observer| LifecycleGuard { _observer: observer })
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        sync::{Mutex, mpsc},
        thread::JoinHandle,
    };
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, WPARAM},
            System::RemoteDesktop::{
                NOTIFY_FOR_THIS_SESSION, WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK, WTSActive,
                WTSFreeMemory, WTSINFOEXW, WTSQuerySessionInformationW,
                WTSRegisterSessionNotification, WTSSessionInfoEx, WTSUnRegisterSessionNotification,
            },
            UI::WindowsAndMessaging::*,
        },
        core::{PWSTR, w},
    };

    struct State {
        pause: PauseState,
        callback: Callback,
    }
    pub struct Guard {
        hwnd: usize,
        worker: Mutex<Option<JoinHandle<()>>>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: this is the hidden window created by our observer thread;
            // posting its close is thread safe and all native cleanup runs there.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            if let Ok(worker) = self.worker.get_mut() {
                if let Some(worker) = worker.take() {
                    let _ = worker.join();
                }
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        // SAFETY: WM_NCCREATE carries CREATESTRUCTW until this callback returns.
        // The State Box outlives the window and is freed only by its owner thread.
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(lp.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            if !state.is_null() {
                let state = &mut *state;
                let before = state.pause.paused();
                match message {
                    WM_POWERBROADCAST => match wp.0 as u32 {
                        PBT_APMSUSPEND => state.pause.sleeping = true,
                        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
                            state.pause.sleeping = false;
                            state.pause.inactive = initially_locked().unwrap_or(true);
                        }
                        _ => {}
                    },
                    WM_WTSSESSION_CHANGE => match wp.0 as u32 {
                        WTS_SESSION_LOCK
                        | WTS_SESSION_LOGOFF
                        | WTS_CONSOLE_DISCONNECT
                        | WTS_REMOTE_DISCONNECT => state.pause.inactive = true,
                        WTS_SESSION_UNLOCK | WTS_SESSION_LOGON => state.pause.inactive = false,
                        WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
                            state.pause.inactive = initially_locked().unwrap_or(true)
                        }
                        _ => {}
                    },
                    WM_QUERYENDSESSION => state.pause.shutting_down = true,
                    WM_ENDSESSION => state.pause.shutting_down = wp.0 != 0,
                    WM_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    WM_DESTROY => {
                        PostQuitMessage(0);
                        return LRESULT(0);
                    }
                    WM_NCDESTROY => {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    }
                    _ => {}
                }
                if before != state.pause.paused() {
                    (state.callback)(state.pause.paused());
                }
            }
            if message == WM_QUERYENDSESSION {
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, message, wp, lp)
        }
    }

    fn initially_locked() -> anyhow::Result<bool> {
        let mut buffer = PWSTR::null();
        let mut size = 0u32;
        // SAFETY: the WTS allocation and declared size are checked before the
        // level-one union is accessed, then freed exactly once on all paths.
        unsafe {
            WTSQuerySessionInformationW(
                None,
                WTS_CURRENT_SESSION,
                WTSSessionInfoEx,
                &mut buffer,
                &mut size,
            )?;
            let result = if size as usize >= std::mem::size_of::<WTSINFOEXW>() && !buffer.is_null()
            {
                let info = &*(buffer.0 as *const WTSINFOEXW);
                if info.Level != 1 {
                    Err(anyhow::anyhow!(
                        "WTS returned an unsupported session information level"
                    ))
                } else {
                    let session = info.Data.WTSInfoExLevel1;
                    match session.SessionFlags {
                        0 | 1 => Ok(session.SessionFlags == WTS_SESSIONSTATE_LOCK as i32
                            || session.SessionState != WTSActive),
                        _ => Err(anyhow::anyhow!("WTS returned an unknown lock state")),
                    }
                }
            } else {
                Err(anyhow::anyhow!("WTS returned incomplete session state"))
            };
            WTSFreeMemory(buffer.0 as *mut _);
            result
        }
    }

    pub fn install(_: &tauri::AppHandle, callback: Callback) -> anyhow::Result<Guard> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("nomifun-keyboard-lifecycle".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let mut state = Box::new(State {
                        pause: PauseState {
                            inactive: true,
                            ..Default::default()
                        },
                        callback: callback.clone(),
                    });
                    // A hidden top-level window receives broadcast power/logoff
                    // messages; a message-only HWND would miss WM_POWERBROADCAST.
                    unsafe {
                        let class = WNDCLASSW {
                            lpfnWndProc: Some(window_proc),
                            lpszClassName: w!("NomiFunKeyboardLifecycle"),
                            ..Default::default()
                        };
                        if RegisterClassW(&class) == 0
                            && windows::Win32::Foundation::GetLastError()
                                != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
                        {
                            return Err(windows::core::Error::from_win32().into());
                        }
                        let hwnd = CreateWindowExW(
                            WINDOW_EX_STYLE::default(),
                            w!("NomiFunKeyboardLifecycle"),
                            w!(""),
                            WINDOW_STYLE::default(),
                            0,
                            0,
                            0,
                            0,
                            None,
                            None,
                            None,
                            Some((&mut *state as *mut State).cast()),
                        )?;
                        if let Err(error) =
                            WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)
                        {
                            let _ = DestroyWindow(hwnd);
                            return Err(error.into());
                        }
                        match initially_locked() {
                            Ok(locked) => state.pause.inactive = locked,
                            Err(error) => {
                                let _ = WTSUnRegisterSessionNotification(hwnd);
                                let _ = DestroyWindow(hwnd);
                                return Err(error);
                            }
                        }
                        if ready_tx.send(Ok(hwnd.0 as usize)).is_err() {
                            let _ = WTSUnRegisterSessionNotification(hwnd);
                            let _ = DestroyWindow(hwnd);
                            return Ok(());
                        }
                        callback(state.pause.paused());
                        let mut message = MSG::default();
                        loop {
                            let result = GetMessageW(&mut message, None, 0, 0).0;
                            if result <= 0 {
                                break;
                            }
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                        let _ = WTSUnRegisterSessionNotification(hwnd);
                        if IsWindow(Some(hwnd)).as_bool() {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                    callback(true);
                    Ok(())
                })();
                if let Err(error) = result {
                    callback(true);
                    let _ = ready_tx.send(Err(error.to_string()));
                }
            })?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Ok(hwnd)) => Ok(Guard {
                hwnd,
                worker: Mutex::new(Some(worker)),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(anyhow::anyhow!(error))
            }
            Err(error) => Err(anyhow::anyhow!(
                "native lifecycle observer did not initialize: {error}"
            )),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use block2::RcBlock;
    use objc2::{MainThreadMarker, class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::NSString;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    pub struct Guard {
        app: tauri::AppHandle,
        tokens: Vec<(usize, bool)>,
        live: Arc<AtomicBool>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            self.live.store(false, Ordering::Release);
            let tokens = std::mem::take(&mut self.tokens);
            let _ = self.app.run_on_main_thread(move || remove_tokens(tokens));
        }
    }
    fn remove_tokens(tokens: Vec<(usize, bool)>) {
        unsafe {
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            let center: Retained<AnyObject> = msg_send![&*workspace, notificationCenter];
            let distributed: Retained<AnyObject> =
                msg_send![class!(NSDistributedNotificationCenter), defaultCenter];
            for (raw, is_distributed) in tokens {
                // SAFETY: each observer was retained and transferred to
                // this guard once; removal and release occur on AppKit's thread.
                if let Some(token) = Retained::from_raw(raw as *mut AnyObject) {
                    let observer_center = if is_distributed {
                        &*distributed
                    } else {
                        &*center
                    };
                    let _: () = msg_send![observer_center, removeObserver:&*token];
                }
            }
        }
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *mut AnyObject;
    }

    fn current_session() -> PauseState {
        // Quartz owns the initial session snapshot. Its dictionary is toll-free
        // bridged with NSDictionary; the Copy retain is released by Retained.
        unsafe {
            match Retained::from_raw(CGSessionCopyCurrentDictionary()) {
                Some(dictionary) => {
                    let key = NSString::from_str("CGSSessionScreenIsLocked");
                    let value: *mut AnyObject = msg_send![&*dictionary, objectForKey:&*key];
                    let locked = if value.is_null() {
                        false
                    } else {
                        let flag: objc2::runtime::Bool = msg_send![value, boolValue];
                        flag.as_bool()
                    };
                    // CGSession.h defines kCGSessionOnConsoleKey as this string;
                    // the dictionary key has an extra S after "CG".
                    let key = NSString::from_str("kCGSSessionOnConsoleKey");
                    let console: *mut AnyObject = msg_send![&*dictionary, objectForKey:&*key];
                    let inactive = if console.is_null() {
                        true
                    } else {
                        let flag: objc2::runtime::Bool = msg_send![console, boolValue];
                        !flag.as_bool()
                    };
                    PauseState {
                        locked,
                        inactive,
                        ..Default::default()
                    }
                }
                None => PauseState {
                    inactive: true,
                    ..Default::default()
                },
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn current_session_matches_quartz_console_flag() {
            // Exercise the real Quartz boundary: the SDK constant's name is
            // different from the string stored in the session dictionary.
            unsafe {
                let Some(dictionary) = Retained::from_raw(CGSessionCopyCurrentDictionary()) else {
                    assert!(current_session().inactive);
                    return;
                };
                let key = NSString::from_str("kCGSSessionOnConsoleKey");
                let console: *mut AnyObject = msg_send![&*dictionary, objectForKey:&*key];
                let expected_inactive = if console.is_null() {
                    true
                } else {
                    let on_console: objc2::runtime::Bool = msg_send![console, boolValue];
                    !on_console.as_bool()
                };
                assert_eq!(current_session().inactive, expected_inactive);
            }
        }
    }

    fn register(callback: Callback, live: Arc<AtomicBool>) -> Vec<(usize, bool)> {
        if !live.load(Ordering::Acquire) {
            return Vec::new();
        }
        let state = Arc::new(Mutex::new(PauseState {
            inactive: true,
            ..Default::default()
        }));
        let mut tokens = Vec::new();
        // SAFETY: install dispatches this entire registration to AppKit's main
        // thread. NSNotificationCenter retains/copies each block until removal.
        unsafe {
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            let center: Retained<AnyObject> = msg_send![&*workspace, notificationCenter];
            let distributed: Retained<AnyObject> =
                msg_send![class!(NSDistributedNotificationCenter), defaultCenter];
            for (name, reason, paused, is_distributed) in [
                ("NSWorkspaceWillSleepNotification", 0, true, false),
                ("NSWorkspaceDidWakeNotification", 0, false, false),
                (
                    "NSWorkspaceSessionDidResignActiveNotification",
                    1,
                    true,
                    false,
                ),
                (
                    "NSWorkspaceSessionDidBecomeActiveNotification",
                    1,
                    false,
                    false,
                ),
                ("NSWorkspaceWillPowerOffNotification", 2, true, false),
                ("com.apple.screenIsLocked", 3, true, true),
                ("com.apple.screenIsUnlocked", 3, false, true),
            ] {
                let state = state.clone();
                let callback = callback.clone();
                let live = live.clone();
                let block = RcBlock::new(move |_: *mut AnyObject| {
                    if !live.load(Ordering::Acquire) {
                        return;
                    }
                    if let Ok(mut state) = state.lock() {
                        let before = state.paused();
                        match reason {
                            0 => state.sleeping = paused,
                            1 => state.inactive = paused,
                            2 => state.shutting_down = paused,
                            _ => state.locked = paused,
                        }
                        if before != state.paused() {
                            callback(state.paused());
                        }
                    } else {
                        callback(true);
                    }
                });
                let name = NSString::from_str(name);
                let observer_center = if is_distributed {
                    &*distributed
                } else {
                    &*center
                };
                let token: Retained<AnyObject> = msg_send![observer_center, addObserverForName:&*name,
                    object:std::ptr::null::<AnyObject>(), queue:std::ptr::null::<AnyObject>(), usingBlock:&*block];
                tokens.push((Retained::into_raw(token) as usize, is_distributed));
            }
        }
        // Read after observers exist so a lock during installation cannot be
        // missed between a stale snapshot and notification registration.
        if let Ok(mut initial) = state.lock() {
            *initial = current_session();
        }
        if live.load(Ordering::Acquire) {
            callback(state.lock().map_or(true, |state| state.paused()));
        }
        tokens
    }
    pub fn install(app: &tauri::AppHandle, callback: Callback) -> anyhow::Result<Guard> {
        let live = Arc::new(AtomicBool::new(true));
        let tokens = if MainThreadMarker::new().is_some() {
            register(callback, live.clone())
        } else {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let active = live.clone();
            app.run_on_main_thread(move || {
                if let Err(undelivered) = tx.send(register(callback, active)) {
                    remove_tokens(undelivered.0);
                }
            })?;
            match rx.recv_timeout(std::time::Duration::from_secs(3)) {
                Ok(tokens) => tokens,
                Err(error) => {
                    live.store(false, Ordering::Release);
                    return Err(error.into());
                }
            }
        };
        Ok(Guard {
            app: app.clone(),
            tokens,
            live,
        })
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use futures_util::StreamExt;
    use tokio_util::sync::CancellationToken;
    pub struct Guard {
        cancel: CancellationToken,
        worker: tauri::async_runtime::JoinHandle<()>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            self.cancel.cancel();
            self.worker.abort();
        }
    }

    async fn observe(callback: &Callback, cancel: &CancellationToken) -> anyhow::Result<()> {
        let connection = zbus::Connection::system().await?;
        let manager = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await?;
        let path: zbus::zvariant::OwnedObjectPath = if let Ok(id) = std::env::var("XDG_SESSION_ID")
        {
            manager.call("GetSession", &(id,)).await?
        } else {
            manager
                .call("GetSessionByPID", &(std::process::id(),))
                .await?
        };
        let session = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            path.clone(),
            "org.freedesktop.login1.Session",
        )
        .await?;
        let properties = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            path.clone(),
            "org.freedesktop.DBus.Properties",
        )
        .await?;
        // Subscribe before reading initial properties, so a lock/suspend racing
        // installation is either reflected in the read or retained as a signal.
        let mut sleep = manager.receive_signal("PrepareForSleep").await?;
        let mut shutdown = manager.receive_signal("PrepareForShutdown").await?;
        let mut removed = manager.receive_signal("SessionRemoved").await?;
        let mut changed = properties.receive_signal("PropertiesChanged").await?;
        let mut state = PauseState {
            sleeping: manager.get_property("PreparingForSleep").await?,
            inactive: !session.get_property::<bool>("Active").await?,
            locked: session.get_property::<bool>("LockedHint").await?,
            shutting_down: manager.get_property("PreparingForShutdown").await?,
        };
        callback(state.paused());
        loop {
            let before = state.paused();
            tokio::select! { biased;
                _ = cancel.cancelled() => return Ok(()),
                signal = sleep.next() => { let signal=signal.ok_or_else(||anyhow::anyhow!("login1 sleep subscription ended"))?;
                    let (sleeping,):(bool,) = signal.body().deserialize()?; state.sleeping=sleeping; },
                signal = shutdown.next() => { let signal=signal.ok_or_else(||anyhow::anyhow!("login1 shutdown subscription ended"))?;
                    let (stopping,):(bool,) = signal.body().deserialize()?; state.shutting_down=stopping; },
                signal = removed.next() => { let signal=signal.ok_or_else(||anyhow::anyhow!("login1 session subscription ended"))?;
                    let (_, removed):(String,zbus::zvariant::OwnedObjectPath) = signal.body().deserialize()?;
                    if removed==path { callback(true); anyhow::bail!("graphical login session was removed"); } },
                signal = changed.next() => { let _=signal.ok_or_else(||anyhow::anyhow!("login1 property subscription ended"))?;
                    state.locked=session.get_property::<bool>("LockedHint").await?; state.inactive=!session.get_property::<bool>("Active").await?; },
            }
            if before != state.paused() {
                callback(state.paused());
            }
        }
    }
    pub fn install(_: &tauri::AppHandle, callback: Callback) -> anyhow::Result<Guard> {
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let worker = tauri::async_runtime::spawn(async move {
            while !stopped.is_cancelled() {
                let result = tokio::select! { biased; _=stopped.cancelled()=>break, result=observe(&callback,&stopped)=>result };
                if let Err(error) = result {
                    callback(true);
                    tracing::warn!(%error,"keyboard lifecycle session observer lost; capture disabled");
                }
                tokio::select! { _=stopped.cancelled()=>break, _=tokio::time::sleep(std::time::Duration::from_secs(2))=>{} }
            }
        });
        Ok(Guard { cancel, worker })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_does_not_clear_a_lock_or_logout() {
        let mut state = PauseState {
            sleeping: true,
            inactive: true,
            locked: true,
            shutting_down: false,
        };
        state.sleeping = false;
        assert!(state.paused());
        state.inactive = false;
        assert!(state.paused());
        state.locked = false;
        assert!(!state.paused());
        state.shutting_down = true;
        state.sleeping = true;
        state.sleeping = false;
        assert!(state.paused());
    }
}
