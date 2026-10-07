//! Input synthesis via enigo.
//!
//! Enigo handles are not `Send`, so each operation constructs a fresh Enigo
//! inside `tokio::task::spawn_blocking`. A 10s deadline marks the result
//! uncertain, but an already-admitted task remains joined until its pressed
//! input is released; no native worker is detached after timeout. On macOS,
//! Enigo's keyboard path queries Carbon
//! TIS/TSM input-source APIs, so the actual Enigo construction and operation
//! are synchronously dispatched to the main queue. Coordinates passed in here
//! are already absolute screen coordinates (mapped from screenshot space by
//! the caller).

use std::time::Duration;

use enigo::{Axis, Button, Direction, Enigo, Keyboard, Mouse, Settings};
// `Coordinate::Abs` is only used on the non-Windows actuation path; Windows
// moves the cursor via SendInput over the virtual desktop instead (see
// `move_abs`).
#[cfg(not(target_os = "windows"))]
use enigo::Coordinate;

use crate::permissions;

const INPUT_TIMEOUT: Duration = Duration::from_secs(10);

/// Pause between press and release (and between repeated clicks) so target
/// apps register distinct events.
const CLICK_PAUSE: Duration = Duration::from_millis(20);
const DRAG_STEPS: i64 = 8;

/// Scroll direction accepted by the `scroll` action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollDirection {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            other => Err(format!(
                "Unknown scroll direction {other:?}. Use one of: up, down, left, right."
            )),
        }
    }
}

/// Map an absolute global virtual-desktop screen coordinate into the 0..=65535
/// normalized range that `SendInput` expects with
/// `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`, relative to the
/// virtual-screen rectangle `(v_left, v_top, v_width, v_height)` reported by
/// `GetSystemMetrics(SM_*VIRTUALSCREEN)`. The endpoints `v_left` and
/// `v_left + v_width - 1` map to 0 and 65535 respectively (round-to-nearest);
/// coordinates outside the desktop clamp into range.
///
/// This is the Windows-only fix for enigo 0.6's `Coordinate::Abs`, which
/// normalizes against the PRIMARY monitor (`GetSystemMetrics(SM_CXSCREEN)`) and
/// omits `MOUSEEVENTF_VIRTUALDESK`, so any target on a secondary monitor — or a
/// monitor whose virtual-desktop origin is negative/non-zero — is mis-projected
/// onto the primary display.
#[cfg(target_os = "windows")]
fn normalize_to_virtual_desktop(
    x: i32,
    y: i32,
    v_left: i32,
    v_top: i32,
    v_width: i32,
    v_height: i32,
) -> (i32, i32) {
    // Map [origin, origin + extent - 1] onto [0, 65535] (round-to-nearest),
    // clamped so out-of-desktop inputs never escape the range.
    fn axis(coord: i32, origin: i32, extent: i32) -> i32 {
        let span = (extent as i64) - 1;
        if span <= 0 {
            return 0;
        }
        let rel = (coord as i64 - origin as i64).max(0);
        let n = (rel * 65535 + span / 2) / span;
        n.clamp(0, 65535) as i32
    }
    (axis(x, v_left, v_width), axis(y, v_top, v_height))
}


/// Move the cursor to an absolute global screen coordinate (the space produced
/// by `to_screen()` / xcap monitor origins).
///
/// On Windows we bypass enigo's `Coordinate::Abs` — it normalizes against the
/// primary monitor only and omits `MOUSEEVENTF_VIRTUALDESK`, so multi-monitor
/// and negative/non-zero-origin targets land on the wrong display — and emit a
/// `SendInput` move across the whole virtual desktop. On macOS / Linux enigo
/// already actuates global coordinates correctly, so its path is unchanged.
fn move_abs(enigo: &mut Enigo, x: i32, y: i32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = enigo;
        move_abs_windows(x, y)
    }
    #[cfg(not(target_os = "windows"))]
    {
        enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_err)
    }
}

/// Windows absolute cursor move over the entire virtual desktop via `SendInput`
/// (`MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`),
/// normalized against `GetSystemMetrics(SM_*VIRTUALSCREEN)`.
#[cfg(target_os = "windows")]
fn move_abs_windows(x: i32, y: i32) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    // SAFETY: GetSystemMetrics reads global display metrics; no preconditions.
    let (v_left, v_top, v_width, v_height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if v_width <= 0 || v_height <= 0 {
        return Err(
            "Could not read the Windows virtual-screen dimensions for absolute \
             cursor positioning."
                .to_string(),
        );
    }
    let (nx, ny) = normalize_to_virtual_desktop(x, y, v_left, v_top, v_width, v_height);
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: nx,
                dy: ny,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: a single well-formed INPUT value; cbsize matches its size.
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(
            "Windows refused the synthetic mouse move (SendInput inserted no \
             events; input may be blocked by a higher-integrity window or the \
             secure desktop)."
                .to_string(),
        )
    }
}

fn new_enigo() -> Result<Enigo, String> {
    let settings = Settings {
        // Never block the agent on an interactive permission prompt.
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    };
    Enigo::new(&settings).map_err(|e| {
        format!(
            "Failed to initialize input synthesis: {e}. {}",
            permissions::accessibility_hint_detailed()
        )
    })
}

fn run_enigo_operation_blocking<T, F>(op: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut Enigo) -> Result<T, String> + Send + 'static,
{
    crate::macos_main::run_blocking(move || {
        let mut enigo = new_enigo()?;
        op(&mut enigo)
    })
}

/// Run an input operation on a fresh Enigo instance inside spawn_blocking.
/// The 10s deadline bounds when success can be reported, while an admitted
/// worker stays joined through cleanup before any timeout error is returned.
async fn with_enigo<T, F>(op: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut Enigo) -> Result<T, String> + Send + 'static,
{
    let handle = tokio::task::spawn_blocking(move || run_enigo_operation_blocking(op));
    join_input_task(handle, INPUT_TIMEOUT).await
}

async fn join_input_task<T>(
    mut handle: tokio::task::JoinHandle<Result<T, String>>,
    timeout: Duration,
) -> Result<T, String>
where
    T: Send + 'static,
{
    match tokio::time::timeout(timeout, &mut handle).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_err)) => Err(format!("Input task failed: {join_err}")),
        Err(_) => {
            // `spawn_blocking` tasks cannot be cancelled after they start. A
            // detached input task could still hold a key/button or apply a
            // late effect after the caller observed the timeout. Retain and
            // join the exact admitted task before reporting its uncertain
            // result; the Engine host can then withhold cleanup proof while a
            // native input operation remains live.
            let settled = match handle.await {
                Ok(Ok(_)) => "The admitted input task settled after the deadline; no background input task remains, but its effect may already be visible.".to_owned(),
                Ok(Err(error)) => format!(
                    "The admitted input task settled after the deadline with an error ({error}); no background input task remains, but OS input cleanup may be incomplete."
                ),
                Err(join_error) => format!(
                    "The admitted input task failed after the deadline ({join_error}); no background input task remains, but OS input cleanup is unproven."
                ),
            };
            Err(format!(
                "Input operation timed out after {}s. {settled} Do not retry automatically; \
                 observe the current desktop first. The system may be blocking synthetic input. {}",
                timeout.as_secs_f64(),
                permissions::accessibility_hint_detailed()
            ))
        }
    }
}

fn input_err(e: enigo::InputError) -> String {
    format!(
        "Input synthesis failed: {e}. {}",
        permissions::accessibility_hint()
    )
}

#[derive(Debug, PartialEq, Eq)]
enum ReleaseReport {
    Clean,
    Recovered(Vec<String>),
    Unproven(Vec<String>),
}

/// Release in reverse press order. A first failure is retried once while the
/// exact obligation remains recorded; a failed retry stays in `pressed` so a
/// surrounding guard can make another best-effort attempt during unwinding.
fn release_obligations<T: Copy>(
    pressed: &mut Vec<T>,
    mut release: impl FnMut(T) -> Result<(), String>,
) -> ReleaseReport {
    let mut first_errors = Vec::new();
    let mut retry_reverse = Vec::new();
    while let Some(input) = pressed.pop() {
        if let Err(error) = release(input) {
            first_errors.push(error);
            retry_reverse.push(input);
        }
    }
    retry_reverse.reverse();
    *pressed = retry_reverse;
    if pressed.is_empty() {
        return ReleaseReport::Clean;
    }

    let mut retry_errors = Vec::new();
    let mut remaining_reverse = Vec::new();
    while let Some(input) = pressed.pop() {
        if let Err(error) = release(input) {
            retry_errors.push(error);
            remaining_reverse.push(input);
        }
    }
    remaining_reverse.reverse();
    *pressed = remaining_reverse;
    first_errors.extend(retry_errors);
    if pressed.is_empty() {
        ReleaseReport::Recovered(first_errors)
    } else {
        ReleaseReport::Unproven(first_errors)
    }
}

struct PressedInputGuard<'a> {
    enigo: &'a mut Enigo,
    buttons: Vec<Button>,
    keys: Vec<enigo::Key>,
}

impl<'a> PressedInputGuard<'a> {
    fn new(enigo: &'a mut Enigo) -> Self {
        Self {
            enigo,
            buttons: Vec::new(),
            keys: Vec::new(),
        }
    }

    fn press_button(&mut self, button: Button) -> Result<(), String> {
        // Record before invoking the OS. A reported press failure can still be
        // partial, so cleanup must issue the matching release.
        self.buttons.push(button);
        if let Err(error) = self.enigo.button(button, Direction::Press) {
            let press_error = input_err(error);
            let cleanup = self.release_all();
            return Err(combine_input_and_cleanup_error(press_error, cleanup));
        }
        Ok(())
    }

    fn press_key(&mut self, key: enigo::Key) -> Result<(), String> {
        self.keys.push(key);
        if let Err(error) = self.enigo.key(key, Direction::Press) {
            let press_error = input_err(error);
            let cleanup = self.release_all();
            return Err(combine_input_and_cleanup_error(press_error, cleanup));
        }
        Ok(())
    }

    fn move_abs(&mut self, x: i32, y: i32) -> Result<(), String> {
        move_abs(self.enigo, x, y)
    }

    fn release_all(&mut self) -> Result<(), String> {
        let key_report = release_obligations(&mut self.keys, |key| {
            self.enigo
                .key(key, Direction::Release)
                .map_err(input_err)
        });
        let button_report = release_obligations(&mut self.buttons, |button| {
            self.enigo
                .button(button, Direction::Release)
                .map_err(input_err)
        });
        release_reports_result(key_report, button_report)
    }
}

impl Drop for PressedInputGuard<'_> {
    fn drop(&mut self) {
        if !self.keys.is_empty() || !self.buttons.is_empty() {
            let _ = self.release_all();
        }
    }
}

fn release_reports_result(
    key_report: ReleaseReport,
    button_report: ReleaseReport,
) -> Result<(), String> {
    let mut recovered = Vec::new();
    let mut unproven = Vec::new();
    for (label, report) in [("key", key_report), ("mouse button", button_report)] {
        match report {
            ReleaseReport::Clean => {}
            ReleaseReport::Recovered(errors) => recovered.push(format!(
                "{label} release initially failed but its exact retry succeeded: {}",
                errors.join(" | ")
            )),
            ReleaseReport::Unproven(errors) => unproven.push(format!(
                "{label} release remained unproven after an exact retry: {}",
                errors.join(" | ")
            )),
        }
    }
    if !unproven.is_empty() {
        return Err(format!(
            "Input cleanup is unproven; do not retry or continue input until the desktop is re-observed. {}",
            unproven.join("; ")
        ));
    }
    if !recovered.is_empty() {
        return Err(format!(
            "Input cleanup recovered after a release failure; all recorded keys/buttons are released, but the input effect is uncertain. {}",
            recovered.join("; ")
        ));
    }
    Ok(())
}

fn combine_input_and_cleanup_error(input_error: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => format!("{input_error} Input cleanup released every recorded key/button."),
        Err(cleanup_error) => format!("{input_error} {cleanup_error}"),
    }
}

/// Move the cursor to absolute screen coordinates.
pub async fn mouse_move(x: i32, y: i32) -> Result<(), String> {
    with_enigo(move |enigo| move_abs(enigo, x, y)).await
}

/// Click `button` `count` times at absolute screen coordinates.
pub async fn click(x: i32, y: i32, button: Button, count: u32) -> Result<(), String> {
    with_enigo(move |enigo| {
        move_abs(enigo, x, y)?;
        for i in 0..count {
            if i > 0 {
                std::thread::sleep(CLICK_PAUSE);
            }
            enigo.button(button, Direction::Click).map_err(input_err)?;
        }
        Ok(())
    })
    .await
}

fn drag_axis(start: i32, end: i32, step: i64) -> i32 {
    (i64::from(start) + (i64::from(end) - i64::from(start)) * step / DRAG_STEPS) as i32
}

/// Press at (start), drag to (end), release. Includes intermediate moves so
/// apps that track motion register the drag.
pub async fn drag(start_x: i32, start_y: i32, end_x: i32, end_y: i32) -> Result<(), String> {
    with_enigo(move |enigo| {
        move_abs(enigo, start_x, start_y)?;
        let mut pressed = PressedInputGuard::new(enigo);
        pressed.press_button(Button::Left)?;
        std::thread::sleep(CLICK_PAUSE);
        let movement = (|| {
            // A few intermediate steps make drags more reliable than a teleport.
            for i in 1..=DRAG_STEPS {
                pressed.move_abs(
                    drag_axis(start_x, end_x, i),
                    drag_axis(start_y, end_y, i),
                )?;
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        })();
        // Even a failed intermediate move must release the button we pressed.
        let cleanup = pressed.release_all();
        match (movement, cleanup) {
            (Ok(()), cleanup) => cleanup,
            (Err(error), cleanup) => Err(combine_input_and_cleanup_error(error, cleanup)),
        }
    })
    .await
}

/// Type a unicode string (layout-independent).
pub async fn type_text(text: String) -> Result<(), String> {
    with_enigo(move |enigo| enigo.text(&text).map_err(input_err)).await
}

/// Press a key combo: press front-to-back, release back-to-front.
pub async fn key_combo(keys: Vec<enigo::Key>) -> Result<(), String> {
    with_enigo(move |enigo| {
        let mut pressed = PressedInputGuard::new(enigo);
        for key in &keys {
            pressed.press_key(*key)?;
        }
        std::thread::sleep(CLICK_PAUSE);
        pressed.release_all()
    })
    .await
}

/// Scroll by `amount` wheel clicks in `direction` (optionally moving the
/// cursor to (x, y) first so the scroll lands on the right surface).
pub async fn scroll(
    at: Option<(i32, i32)>,
    direction: ScrollDirection,
    amount: i32,
) -> Result<(), String> {
    with_enigo(move |enigo| {
        if let Some((x, y)) = at {
            move_abs(enigo, x, y)?;
        }
        let (axis, length) = match direction {
            ScrollDirection::Up => (Axis::Vertical, -amount),
            ScrollDirection::Down => (Axis::Vertical, amount),
            ScrollDirection::Left => (Axis::Horizontal, -amount),
            ScrollDirection::Right => (Axis::Horizontal, amount),
        };
        enigo.scroll(length, axis).map_err(input_err)
    })
    .await
}

/// Current cursor location in absolute screen coordinates.
pub async fn cursor_position() -> Result<(i32, i32), String> {
    with_enigo(|enigo| enigo.location().map_err(input_err)).await
}

/// Size (width, height) of the main display in enigo's coordinate system.
/// Blocking variant for use inside other spawn_blocking sections.
pub fn main_display_size_blocking() -> Result<(i32, i32), String> {
    run_enigo_operation_blocking(|enigo| enigo.main_display().map_err(input_err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_interpolation_handles_full_coordinate_range() {
        assert_eq!(drag_axis(i32::MIN, i32::MAX, 0), i32::MIN);
        assert_eq!(drag_axis(i32::MIN, i32::MAX, DRAG_STEPS), i32::MAX);
        assert_eq!(drag_axis(i32::MIN, i32::MAX, DRAG_STEPS / 2), -1);
        assert_eq!(drag_axis(i32::MAX, i32::MIN, DRAG_STEPS), i32::MIN);
        assert_eq!(drag_axis(10, 90, 1), 20);
        assert_eq!(drag_axis(90, 10, 1), 80);
    }

    #[test]
    fn scroll_direction_parses_all_variants() {
        assert_eq!(ScrollDirection::parse("up").unwrap(), ScrollDirection::Up);
        assert_eq!(
            ScrollDirection::parse("down").unwrap(),
            ScrollDirection::Down
        );
        assert_eq!(
            ScrollDirection::parse("left").unwrap(),
            ScrollDirection::Left
        );
        assert_eq!(
            ScrollDirection::parse("right").unwrap(),
            ScrollDirection::Right
        );
    }

    #[test]
    fn scroll_direction_unknown_is_error() {
        let err = ScrollDirection::parse("diagonal").unwrap_err();
        assert!(err.contains("diagonal"));
    }

    #[tokio::test]
    async fn admitted_input_timeout_waits_for_pressed_state_cleanup() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let pressed = Arc::new(AtomicBool::new(false));
        let worker_pressed = Arc::clone(&pressed);
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let handle = tokio::task::spawn_blocking(move || {
            worker_pressed.store(true, Ordering::SeqCst);
            started_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(500));
            worker_pressed.store(false, Ordering::SeqCst);
            Ok::<_, String>(())
        });
        started_rx.recv().unwrap();

        let started = std::time::Instant::now();
        let error = join_input_task(handle, Duration::from_millis(10))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert!(
            !pressed.load(Ordering::SeqCst),
            "a reported timeout must not abandon an admitted task with input still pressed"
        );
    }

    #[test]
    fn release_failure_retries_only_the_exact_remaining_obligation() {
        let mut pressed = vec![1_u8, 2_u8];
        let mut attempts = Vec::new();
        let mut failed_once = false;
        let report = release_obligations(&mut pressed, |input| {
            attempts.push(input);
            if input == 2 && !failed_once {
                failed_once = true;
                Err("injected release failure".to_owned())
            } else {
                Ok(())
            }
        });
        assert_eq!(attempts, vec![2, 1, 2]);
        assert!(pressed.is_empty());
        assert_eq!(
            report,
            ReleaseReport::Recovered(vec!["injected release failure".to_owned()])
        );
    }

    #[test]
    fn repeated_release_failure_retains_the_exact_cleanup_obligation() {
        let mut pressed = vec![1_u8, 2_u8];
        let mut attempts = Vec::new();
        let report = release_obligations(&mut pressed, |input| {
            attempts.push(input);
            (input != 2)
                .then_some(())
                .ok_or_else(|| format!("release {input} rejected"))
        });
        assert_eq!(attempts, vec![2, 1, 2]);
        assert_eq!(pressed, vec![2]);
        assert_eq!(
            report,
            ReleaseReport::Unproven(vec![
                "release 2 rejected".to_owned(),
                "release 2 rejected".to_owned(),
            ])
        );
    }

    // --- virtual-desktop coordinate normalization (Windows actuation fix) ---

    #[cfg(target_os = "windows")]
    #[test]
    fn vd_center_of_single_primary_maps_to_midrange() {
        // One 1920x1080 monitor at the virtual-desktop origin.
        let (nx, ny) = normalize_to_virtual_desktop(960, 540, 0, 0, 1920, 1080);
        assert!((32000..=33500).contains(&nx), "nx={nx}");
        assert!((32000..=33500).contains(&ny), "ny={ny}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn vd_point_on_secondary_monitor_maps_to_upper_range() {
        // Two 1920-wide monitors side by side; target is the centre of the RIGHT
        // (secondary) monitor. The fixed mapping must land in the upper half of
        // the 0..65535 range — enigo's primary-only normalization would divide
        // 2880 by the primary width (1920) and overflow past 65535 onto the
        // primary display.
        let (nx, _) = normalize_to_virtual_desktop(2880, 540, 0, 0, 3840, 1080);
        assert!(nx > 40000 && nx <= 65535, "nx={nx}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn vd_point_on_negative_origin_monitor_maps_to_lower_range() {
        // A monitor to the LEFT of the primary (negative virtual-desktop origin).
        // Target is the centre of that left monitor; it must map to the lower
        // half — enigo would produce a negative normalized value (off-screen).
        let (nx, _) = normalize_to_virtual_desktop(-960, 540, -1920, 0, 3840, 1080);
        assert!(nx > 10000 && nx < 25000, "nx={nx}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn vd_endpoints_map_to_full_range() {
        // Left/top edge -> 0, right/bottom edge -> 65535, with a non-zero origin.
        assert_eq!(normalize_to_virtual_desktop(100, 50, 100, 50, 1920, 1080).0, 0);
        assert_eq!(
            normalize_to_virtual_desktop(100 + 1920 - 1, 50, 100, 50, 1920, 1080).0,
            65535
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn vd_out_of_desktop_clamps() {
        // Beyond the right/below the left edge -> clamped, never out of [0,65535].
        assert_eq!(normalize_to_virtual_desktop(99999, 0, 0, 0, 1920, 1080).0, 65535);
        assert_eq!(normalize_to_virtual_desktop(-99999, 0, 0, 0, 1920, 1080).0, 0);
    }

    // Requires a real input device and (on macOS) Accessibility permission.
    #[tokio::test]
    #[ignore]
    async fn cursor_position_real() {
        let (x, y) = cursor_position().await.expect("should read cursor");
        assert!(x >= -20_000 && x <= 20_000);
        assert!(y >= -20_000 && y <= 20_000);
    }

    #[tokio::test]
    #[ignore]
    async fn mouse_move_real() {
        mouse_move(10, 10).await.expect("should move cursor");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_input_task_runs_inside_dispatcher() {
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
