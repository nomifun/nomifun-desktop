//! Native foreground activation without input-queue attachment or synthetic keys.
use std::fmt;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        GetForegroundWindow, IsIconic, IsWindow, SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT,
        SW_RESTORE, SendMessageTimeoutW, SetForegroundWindow, ShowWindowAsync, WM_NULL,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowsFocusError {
    /// The native window handle was invalid before any mutating request.
    TargetMissing,
    /// The one SetForegroundWindow request returned zero, before any other
    /// mutating native call. This is a native rejection, not an unknown effect.
    NotApplied,
    /// Activation was accepted or its acknowledgement was lost. Never retry it
    /// automatically, even if a later read sees a different foreground window.
    Uncertain(String),
}

impl fmt::Display for WindowsFocusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetMissing => formatter.write_str("The native target window no longer exists; no activation was requested. Acquire a fresh observation."),
            Self::NotApplied => formatter.write_str("Windows denied foreground activation before any window restore or other input. The OS foreground lock requires a real activation condition, such as the user activating NomiFun; no window was changed by this request."),
            Self::Uncertain(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for WindowsFocusError {}

trait FocusTarget {
    fn exists(&mut self) -> bool;
    fn activate_once(&mut self) -> bool;
    fn restore_if_minimized(&mut self) -> Result<(), String>;
    fn synchronize(&mut self) -> bool;
    fn activation_confirmed(&mut self) -> bool;
}

fn focus(target: &mut impl FocusTarget) -> Result<(), WindowsFocusError> {
    if !target.exists() {
        return Err(WindowsFocusError::TargetMissing);
    }
    if !target.activate_once() {
        return Err(WindowsFocusError::NotApplied);
    }
    target
        .restore_if_minimized()
        .map_err(WindowsFocusError::Uncertain)?;
    if !target.synchronize() {
        return Err(WindowsFocusError::Uncertain("Windows accepted foreground activation but the bounded native acknowledgement failed; the effect may have occurred. Do not automatically retry.".into()));
    }
    if !target.activation_confirmed() {
        return Err(WindowsFocusError::Uncertain("Windows accepted foreground activation but the target was not confirmed as the restored foreground window; the effect may have occurred. Do not automatically retry.".into()));
    }
    Ok(())
}

struct NativeTarget(HWND);
impl FocusTarget for NativeTarget {
    fn exists(&mut self) -> bool {
        unsafe { IsWindow(Some(self.0)).as_bool() }
    }
    fn activate_once(&mut self) -> bool {
        unsafe { SetForegroundWindow(self.0).as_bool() }
    }
    fn restore_if_minimized(&mut self) -> Result<(), String> {
        if unsafe { IsIconic(self.0).as_bool() }
            && !unsafe { ShowWindowAsync(self.0, SW_RESTORE).as_bool() }
        {
            return Err("Windows accepted activation but did not acknowledge the asynchronous restore; the effect may have occurred.".into());
        }
        Ok(())
    }
    fn synchronize(&mut self) -> bool {
        let mut result = 0;
        unsafe {
            SendMessageTimeoutW(
                self.0,
                WM_NULL,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                2_000,
                Some(&mut result),
            )
            .0 != 0
        }
    }
    fn activation_confirmed(&mut self) -> bool {
        unsafe { GetForegroundWindow() == self.0 && !IsIconic(self.0).as_bool() }
    }
}

/// Activate the exact HWND on the existing login desktop. Foreground policy is
/// enforced by Windows. No input queue attachment, keys, timeout configuration,
/// UIA fallback, or second activation attempt is used.
pub fn focus_windows_window(window_id: u32) -> Result<(), WindowsFocusError> {
    focus_hwnd(HWND(window_id as usize as *mut core::ffi::c_void))
}

pub(super) fn focus_hwnd(window: HWND) -> Result<(), WindowsFocusError> {
    focus(&mut NativeTarget(window))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MockTarget {
        exists: bool,
        accepted: bool,
        restore: bool,
        synchronized: bool,
        confirmed: bool,
        calls: Vec<&'static str>,
    }
    impl FocusTarget for MockTarget {
        fn exists(&mut self) -> bool {
            self.calls.push("exists");
            self.exists
        }
        fn activate_once(&mut self) -> bool {
            self.calls.push("activate_once");
            self.accepted
        }
        fn restore_if_minimized(&mut self) -> Result<(), String> {
            self.calls.push("restore_if_minimized");
            if self.restore {
                Ok(())
            } else {
                Err("accepted restore acknowledgement lost".into())
            }
        }
        fn synchronize(&mut self) -> bool {
            self.calls.push("synchronize");
            self.synchronized
        }
        fn activation_confirmed(&mut self) -> bool {
            self.calls.push("confirm");
            self.confirmed
        }
    }
    fn target() -> MockTarget {
        MockTarget {
            exists: true,
            accepted: true,
            restore: true,
            synchronized: true,
            confirmed: true,
            calls: Vec::new(),
        }
    }
    #[test]
    fn disappeared_target_is_rejected_without_an_activation_request() {
        let mut target = target();
        target.exists = false;
        assert_eq!(focus(&mut target), Err(WindowsFocusError::TargetMissing));
        assert_eq!(target.calls, ["exists"]);
    }
    #[test]
    fn known_rejection_precedes_every_other_native_effect_and_is_not_retried() {
        let mut target = target();
        target.accepted = false;
        assert_eq!(focus(&mut target), Err(WindowsFocusError::NotApplied));
        assert_eq!(target.calls, ["exists", "activate_once"]);
    }
    #[test]
    fn accepted_success_requires_native_acknowledgement_and_foreground_confirmation() {
        let mut target = target();
        assert_eq!(focus(&mut target), Ok(()));
        assert_eq!(
            target.calls,
            [
                "exists",
                "activate_once",
                "restore_if_minimized",
                "synchronize",
                "confirm"
            ]
        );
    }
    #[test]
    fn accepted_failures_remain_uncertain_without_a_second_activation() {
        for failure in 0..3 {
            let mut target = target();
            match failure {
                0 => target.restore = false,
                1 => target.synchronized = false,
                _ => target.confirmed = false,
            }
            assert!(matches!(
                focus(&mut target),
                Err(WindowsFocusError::Uncertain(_))
            ));
            assert_eq!(
                target
                    .calls
                    .iter()
                    .filter(|call| **call == "activate_once")
                    .count(),
                1
            );
        }
    }
}
