//! Input through the native RemoteDesktop portal session used by capture.
//!
//! These synchronous entry points run in the existing joined input worker.
//! The portal owns its persistent Tokio runtime; no input task is detached and
//! the pressed-state guard performs synchronous, acknowledged cleanup.

use std::{sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

use enigo::{Button, Key};

use crate::{input::ScrollDirection, linux_wayland::{self, PortalSession, PortalStream}};

const INPUT_TIMEOUT: Duration = Duration::from_secs(10);
const CLICK_PAUSE: Duration = Duration::from_millis(20);
const DRAG_STEPS: i64 = 8;
const DRAG_PAUSE: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObservedCapture {
    generation: u64,
    node_id: u32,
    origin: (i32, i32),
    size: (u32, u32),
}

static OBSERVATION: OnceLock<Mutex<Option<ObservedCapture>>> = OnceLock::new();
static INPUT_GATE: Mutex<()> = Mutex::new(());

fn observations() -> &'static Mutex<Option<ObservedCapture>> {
    OBSERVATION.get_or_init(Mutex::default)
}

/// Register only a successfully consumed frame, with the portal's logical
/// stream geometry. Raw PipeWire frame dimensions are a different space.
pub fn record_capture(
    generation: u64,
    node_id: u32,
    origin: (i32, i32),
    size: (u32, u32),
) -> Result<(), String> {
    validate_geometry(origin, size)?;
    let mut capture = observations().lock().map_err(|_| "WAYLAND_CAPTURE_STATE_UNAVAILABLE: Capture state was poisoned".to_owned())?;
    *capture = Some(ObservedCapture { generation, node_id, origin, size });
    Ok(())
}

/// Clear the coordinate authority whenever the owning tool clears observation.
pub fn clear_capture() {
    if let Ok(mut capture) = observations().lock() { *capture = None; }
}

fn clear_generation(generation: u64) {
    if let Ok(mut capture) = observations().lock() {
        if capture.as_ref().is_some_and(|capture| capture.generation == generation) {
            *capture = None;
        }
    }
}

fn validate_geometry(origin: (i32, i32), size: (u32, u32)) -> Result<(), String> {
    for (start, extent) in [(origin.0, size.0), (origin.1, size.1)] {
        if extent == 0 || extent > i32::MAX as u32 || i64::from(start) + i64::from(extent) - 1 > i64::from(i32::MAX) {
            return Err("WAYLAND_CAPTURE_GEOMETRY_UNAVAILABLE: The captured stream has no usable logical input geometry".to_owned());
        }
    }
    Ok(())
}

impl ObservedCapture {
    fn validate(&self, generation: u64, closed: bool, streams: &[PortalStream]) -> Result<(), String> {
        if closed || self.generation != generation {
            return Err("WAYLAND_STALE_CAPTURE: The screenshot belongs to a closed or replaced desktop session. Observe the desktop again before input.".to_owned());
        }
        if !streams.iter().any(|stream| stream.node_id == self.node_id && stream.position == Some(self.origin) && stream.size == Some(self.size)) {
            return Err("WAYLAND_STALE_CAPTURE: The screenshot's stream or logical geometry no longer matches the active desktop session. Observe the desktop again before input.".to_owned());
        }
        Ok(())
    }

    fn local_point(&self, x: i32, y: i32) -> Result<(f64, f64), String> {
        let x = i64::from(x) - i64::from(self.origin.0);
        let y = i64::from(y) - i64::from(self.origin.1);
        if x < 0 || y < 0 || x >= i64::from(self.size.0) || y >= i64::from(self.size.1) {
            return Err("WAYLAND_COORDINATE_OUTSIDE_CAPTURE: The target lies outside the observed stream. Capture the target monitor before input.".to_owned());
        }
        Ok((x as f64, y as f64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressedInput { Button(i32), Keysym(u32) }

trait InputSink {
    fn send(&self, input: PressedInput, pressed: bool) -> Result<(), String>;
    fn move_absolute(&self, node_id: u32, x: f64, y: f64) -> Result<(), String>;
    fn scroll(&self, axis: u32, steps: i32) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
}

struct NativeSink { session: Arc<PortalSession>, cancel: Arc<AtomicBool> }

impl InputSink for NativeSink {
    fn send(&self, input: PressedInput, pressed: bool) -> Result<(), String> {
        if pressed { check_cancelled(&self.cancel)?; }
        let session = self.session.clone();
        let cancel = self.cancel.clone();
        linux_wayland::run_blocking(async move {
            if pressed && cancel.load(Ordering::Acquire) { return Err(cancelled_portal_error()); }
            match input {
                PressedInput::Button(button) if pressed => session.pointer_button_cancellable(cancel, button, true).await,
                PressedInput::Button(button) => session.pointer_button(button, false).await,
                PressedInput::Keysym(keysym) if pressed => session.keyboard_keysym_cancellable(cancel, keysym, true).await,
                PressedInput::Keysym(keysym) => session.keyboard_keysym(keysym, false).await,
            }
        }).map_err(|error| error.to_string())
    }

    fn move_absolute(&self, node_id: u32, x: f64, y: f64) -> Result<(), String> {
        check_cancelled(&self.cancel)?;
        let session = self.session.clone();
        let cancel = self.cancel.clone();
        linux_wayland::run_blocking(async move {
            if cancel.load(Ordering::Acquire) { return Err(cancelled_portal_error()); }
            session.notify_pointer_absolute_cancellable(cancel, node_id, x, y).await
        }).map_err(|error| error.to_string())
    }

    fn scroll(&self, axis: u32, steps: i32) -> Result<(), String> {
        check_cancelled(&self.cancel)?;
        let session = self.session.clone();
        let cancel = self.cancel.clone();
        linux_wayland::run_blocking(async move {
            if cancel.load(Ordering::Acquire) { return Err(cancelled_portal_error()); }
            session.scroll_cancellable(cancel, axis, steps).await
        }).map_err(|error| error.to_string())
    }
    fn close(&self) -> Result<(), String> {
        let session = self.session.clone();
        linux_wayland::run_blocking(async move { session.close().await }).map_err(|error| error.to_string())
    }
}

/// Record before submitting a press: an error can arrive after native delivery.
/// Only matching releases may be retried; a press is never replayed.
struct PressedInputGuard<S: InputSink> {
    sink: S,
    pressed: Vec<PressedInput>,
}

impl<S: InputSink> PressedInputGuard<S> {
    fn new(sink: S) -> Self { Self { sink, pressed: Vec::new() } }

    fn press(&mut self, input: PressedInput) -> Result<(), String> {
        self.pressed.push(input);
        self.sink.send(input, true)
    }

    fn release_all(&mut self) -> Result<(), String> {
        let mut failed_reverse = Vec::new();
        let mut errors = Vec::new();
        while let Some(input) = self.pressed.pop() {
            if let Err(error) = self.sink.send(input, false) {
                failed_reverse.push(input);
                errors.push(error);
            }
        }
        failed_reverse.reverse();
        self.pressed = failed_reverse;
        if errors.is_empty() { return Ok(()); }
        // A portal failure invalidates the session, so an acknowledged native
        // Close is the cleanup boundary. Local `is_closed` alone is no proof.
        match self.sink.close() {
            Ok(()) => {
                self.pressed.clear();
                Err(format!("WAYLAND_INPUT_RESULT_UNKNOWN: A release failed; native Session.Close acknowledged cleanup. Input effects may be partial. Do not retry automatically; observe again. {}", errors.join(" | ")))
            }
            Err(error) => Err(format!("WAYLAND_INPUT_CLEANUP_UNPROVEN: Exact releases and native Session.Close were not confirmed. Input effects may be partial; do not retry. {} | {error}", errors.join(" | "))),
        }
    }
}

impl<S: InputSink> Drop for PressedInputGuard<S> {
    fn drop(&mut self) {
        if !self.pressed.is_empty() {
            // This guard lives on the joined blocking worker, never on the
            // portal runtime. Unwinding waits for the exact release/Close work.
            let _ = self.release_all();
        }
    }
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("WAYLAND_INPUT_CANCELLED: Input was cancelled. Further native effects were stopped; earlier effects may already be visible. Do not retry automatically; observe the desktop before deciding what to do next.".to_owned())
    } else { Ok(()) }
}

fn cancelled_portal_error() -> linux_wayland::WaylandError {
    linux_wayland::WaylandError::new("WAYLAND_INPUT_CANCELLED", "Input was cancelled before the next native event was submitted")
}

struct OperationControl {
    cancel: Arc<AtomicBool>,
    deadline: Instant,
}

impl OperationControl {
    fn check(&self) -> Result<(), String> {
        check_cancelled(&self.cancel)?;
        if Instant::now() >= self.deadline {
            Err("WAYLAND_INPUT_RESULT_UNKNOWN: The input deadline elapsed. Further effects were stopped; observe the desktop before deciding what to do next.".to_owned())
        } else { Ok(()) }
    }

    fn sleep(&self, duration: Duration) -> Result<(), String> {
        self.check()?;
        std::thread::sleep(duration);
        self.check()
    }
}

struct Operation<S: InputSink = NativeSink> {
    guard: PressedInputGuard<S>,
    capture: ObservedCapture,
    control: OperationControl,
}

impl Operation<NativeSink> {
    fn admit(cancel: Arc<AtomicBool>) -> Result<Self, String> {
        // This runs after INPUT_GATE acquisition: a cancelled queued worker
        // must not acquire a session or send its first native event later.
        check_cancelled(&cancel)?;
        let capture = observations().lock().map_err(|_| "WAYLAND_CAPTURE_STATE_UNAVAILABLE: Capture state was poisoned".to_owned())?
            .ok_or_else(|| "WAYLAND_OBSERVATION_REQUIRED: Capture the desktop before sending input; input never creates or restores a desktop session.".to_owned())?;
        let session = linux_wayland::run_blocking(linux_wayland::active_session()).map_err(|error| error.to_string())?;
        check_cancelled(&cancel)?;
        capture.validate(session.generation(), session.is_closed(), session.streams())?;
        Ok(Self {
            guard: PressedInputGuard::new(NativeSink { session, cancel: cancel.clone() }),
            capture,
            control: OperationControl { cancel, deadline: Instant::now() + INPUT_TIMEOUT },
        })
    }

}

impl<S: InputSink> Operation<S> {
    fn press(&mut self, input: PressedInput) -> Result<(), String> {
        self.control.check()?;
        self.guard.press(input)
    }

    fn move_absolute(&self, x: i32, y: i32) -> Result<(), String> {
        self.control.check()?;
        let (x, y) = self.capture.local_point(x, y)?;
        self.guard.sink.move_absolute(self.capture.node_id, x, y)
    }
}

fn with_input_gate<T>(gate: &Mutex<()>, cancel: &AtomicBool, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let _input_gate = gate.lock().map_err(|_| "WAYLAND_INPUT_OWNER_UNAVAILABLE: An earlier input worker failed".to_owned())?;
    check_cancelled(cancel)?;
    operation()
}

fn settle_input<S: InputSink>(guard: &mut PressedInputGuard<S>, result: Result<(), String>, cancel: &AtomicBool) -> Result<(), String> {
    // Cleanup deliberately ignores cancellation. A cancelled caller still owns
    // every exact release and acknowledged native Close until this returns.
    let cleanup = guard.release_all();
    let result = result.and_then(|()| check_cancelled(cancel));
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (result, cleanup) => {
            let close = guard.sink.close();
            if close.is_ok() { guard.pressed.clear(); }
            let mut detail = Vec::new();
            if let Err(error) = result { detail.push(error); }
            if let Err(error) = cleanup { detail.push(error); }
            match close {
                Ok(()) => detail.push("Native Session.Close acknowledged; no held synthetic input remains in this session.".to_owned()),
                Err(error) => detail.push(format!("Native cleanup remains unproven: {error}")),
            }
            Err(detail.join(" "))
        }
    }
}

fn with_operation(cancel: Arc<AtomicBool>, operation: impl FnOnce(&mut Operation) -> Result<(), String>) -> Result<(), String> {
    with_input_gate(&INPUT_GATE, &cancel, || {
        let mut admitted = Operation::admit(cancel.clone())?;
        let result = operation(&mut admitted).and_then(|()| admitted.control.check());
        let result = settle_input(&mut admitted.guard, result, &cancel);
        if result.is_err() { clear_generation(admitted.capture.generation); }
        result
    })
}
pub fn mouse_move_blocking(cancel: Arc<AtomicBool>, x: i32, y: i32) -> Result<(), String> {
    with_operation(cancel, |operation| operation.move_absolute(x, y))
}

pub fn click_blocking(cancel: Arc<AtomicBool>, x: i32, y: i32, button: Button, count: u32) -> Result<(), String> {
    let button = button_code(button)?;
    with_operation(cancel, |operation| {
        operation.move_absolute(x, y)?;
        for index in 0..count {
            if index > 0 { operation.control.sleep(CLICK_PAUSE)?; }
            operation.press(PressedInput::Button(button))?;
            operation.control.sleep(CLICK_PAUSE)?;
            operation.guard.release_all()?;
        }
        Ok(())
    })
}

fn drag_axis(start: i32, end: i32, step: i64) -> i32 {
    (i64::from(start) + (i64::from(end) - i64::from(start)) * step / DRAG_STEPS) as i32
}

pub fn drag_blocking(cancel: Arc<AtomicBool>, start_x: i32, start_y: i32, end_x: i32, end_y: i32) -> Result<(), String> {
    with_operation(cancel, |operation| {
        // Validate the entire path before moving or pressing.
        operation.capture.local_point(start_x, start_y)?;
        operation.capture.local_point(end_x, end_y)?;
        operation.move_absolute(start_x, start_y)?;
        operation.press(PressedInput::Button(0x110))?;
        operation.control.sleep(CLICK_PAUSE)?;
        for step in 1..=DRAG_STEPS {
            operation.move_absolute(drag_axis(start_x, end_x, step), drag_axis(start_y, end_y, step))?;
            operation.control.sleep(DRAG_PAUSE)?;
        }
        Ok(())
    })
}

pub fn type_text_blocking(cancel: Arc<AtomicBool>, text: String) -> Result<(), String> {
    check_cancelled(&cancel)?;
    // Resolve the whole string before any input, so unsupported control data
    // cannot leave an otherwise known-prefix text effect.
    let keysyms = text.chars().map(unicode_keysym).collect::<Result<Vec<_>, _>>()?;
    with_operation(cancel, |operation| {
        for keysym in keysyms {
            operation.press(PressedInput::Keysym(keysym))?;
            operation.guard.release_all()?;
        }
        Ok(())
    })
}

pub fn key_combo_blocking(cancel: Arc<AtomicBool>, keys: Vec<Key>) -> Result<(), String> {
    check_cancelled(&cancel)?;
    let keysyms = keys.into_iter().map(keysym).collect::<Result<Vec<_>, _>>()?;
    with_operation(cancel, |operation| {
        for keysym in keysyms { operation.press(PressedInput::Keysym(keysym))?; }
        operation.control.sleep(CLICK_PAUSE)?;
        Ok(())
    })
}

pub fn scroll_blocking(cancel: Arc<AtomicBool>, at: Option<(i32, i32)>, direction: ScrollDirection, amount: i32) -> Result<(), String> {
    let (axis, steps) = scroll_event(direction, amount)?;
    with_operation(cancel, |operation| {
        if let Some((x, y)) = at { operation.move_absolute(x, y)?; }
        operation.control.check()?;
        operation.guard.sink.scroll(axis, steps)
    })
}

/// RemoteDesktop has no cursor-position observation method. Remembering the
/// last requested point would report commanded state as observed OS state.
pub fn cursor_position_blocking() -> Result<(i32, i32), String> {
    Err("WAYLAND_CURSOR_OBSERVATION_UNAVAILABLE: The native RemoteDesktop portal does not expose the current global cursor position; a previous input target is not a cursor observation.".to_owned())
}

fn scroll_event(direction: ScrollDirection, amount: i32) -> Result<(u32, i32), String> {
    let (axis, reverse) = match direction {
        ScrollDirection::Up => (0, true), ScrollDirection::Down => (0, false),
        ScrollDirection::Left => (1, true), ScrollDirection::Right => (1, false),
    };
    let steps = if reverse { amount.checked_neg().ok_or_else(|| "WAYLAND_SCROLL_RANGE_INVALID: The scroll amount cannot be negated".to_owned())? } else { amount };
    Ok((axis, steps))
}

fn button_code(button: Button) -> Result<i32, String> {
    match button {
        Button::Left => Ok(0x110), Button::Right => Ok(0x111), Button::Middle => Ok(0x112),
        Button::Back => Ok(0x113), Button::Forward => Ok(0x114),
        _ => Err("WAYLAND_BUTTON_UNSUPPORTED: Use the scroll operation for wheel input".to_owned()),
    }
}

fn unicode_keysym(character: char) -> Result<u32, String> {
    match character {
        '\n' | '\r' => Ok(0xff0d), '\t' => Ok(0xff09),
        character if character.is_control() => Err(format!("WAYLAND_KEY_UNSUPPORTED: Control character U+{:04X} has no text-input mapping", u32::from(character))),
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => Ok(u32::from(character)),
        character => Ok(0x0100_0000 | u32::from(character)),
    }
}

fn keysym(key: Key) -> Result<u32, String> {
    let value = match key {
        Key::Unicode(character) => return unicode_keysym(character),
        Key::Control | Key::LControl => 0xffe3, Key::RControl => 0xffe4,
        Key::Shift | Key::LShift => 0xffe1, Key::RShift => 0xffe2,
        Key::Alt | Key::Option => 0xffe9, Key::Meta => 0xffeb,
        Key::Return => 0xff0d, Key::Escape => 0xff1b, Key::Tab => 0xff09,
        Key::Space => 0x20, Key::Backspace => 0xff08, Key::Delete => 0xffff,
        Key::UpArrow => 0xff52, Key::DownArrow => 0xff54,
        Key::LeftArrow => 0xff51, Key::RightArrow => 0xff53,
        Key::Home => 0xff50, Key::End => 0xff57, Key::PageUp => 0xff55, Key::PageDown => 0xff56,
        Key::F1 => 0xffbe, Key::F2 => 0xffbf, Key::F3 => 0xffc0, Key::F4 => 0xffc1,
        Key::F5 => 0xffc2, Key::F6 => 0xffc3, Key::F7 => 0xffc4, Key::F8 => 0xffc5,
        Key::F9 => 0xffc6, Key::F10 => 0xffc7, Key::F11 => 0xffc8, Key::F12 => 0xffc9,
        _ => return Err(format!("WAYLAND_KEY_UNSUPPORTED: No native keysym mapping for {key:?}")),
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn capture() -> ObservedCapture {
        ObservedCapture { generation: 7, node_id: 41, origin: (-1920, 30), size: (1920, 1080) }
    }
    fn stream() -> PortalStream {
        PortalStream { node_id: 41, pipewire_serial: None, position: Some((-1920, 30)), size: Some((1920, 1080)) }
    }

    #[test]
    fn capture_generation_and_exact_stream_geometry_are_required() {
        let capture = capture();
        assert!(capture.validate(7, false, &[stream()]).is_ok());
        assert!(capture.validate(8, false, &[stream()]).unwrap_err().contains("STALE_CAPTURE"));
        assert!(capture.validate(7, true, &[stream()]).is_err());
        assert!(capture.validate(7, false, &[]).is_err());
        let mut wrong_stream = stream();
        wrong_stream.size = Some((3840, 2160));
        assert!(capture.validate(7, false, &[wrong_stream]).is_err());
        let mut wrong_stream = stream();
        wrong_stream.position = None;
        assert!(capture.validate(7, false, &[wrong_stream]).is_err());
    }

    #[test]
    fn coordinates_are_local_to_observed_logical_stream_and_do_not_clamp() {
        let capture = capture();
        assert_eq!(capture.local_point(-1920, 30).unwrap(), (0.0, 0.0));
        assert_eq!(capture.local_point(-1, 1109).unwrap(), (1919.0, 1079.0));
        for point in [(-1921, 30), (0, 30), (-1920, 29), (-1920, 1110)] {
            assert!(capture.local_point(point.0, point.1).is_err());
        }
        assert!(validate_geometry((i32::MAX, 0), (2, 1)).is_err());
        assert!(validate_geometry((0, 0), (0, 1)).is_err());
    }

    #[test]
    fn every_public_combo_parser_key_has_native_keysym_mapping() {
        for combo in ["cmd+shift+t", "super+alt+ctrl", "enter+esc+tab+space+backspace+delete", "up+down+left+right+home+end+pageup+pagedown", "f1+f2+f3+f4+f5+f6+f7+f8+f9+f10+f11+f12", "a+0+/"] {
            for key in crate::keys::parse_key_combo(combo).unwrap() { keysym(key).unwrap(); }
        }
        assert_eq!(unicode_keysym('A').unwrap(), 0x41);
        assert_eq!(unicode_keysym('é').unwrap(), 0xe9);
        assert_eq!(unicode_keysym('中').unwrap(), 0x0100_4e2d);
        assert_eq!(unicode_keysym('😀').unwrap(), 0x0101_f600);
        assert!(unicode_keysym('\0').is_err());
        assert!(keysym(Key::Other(u32::MAX)).is_err());
    }

    #[test]
    fn wheel_axes_and_evdev_buttons_preserve_native_contract() {
        assert_eq!(scroll_event(ScrollDirection::Up, 3).unwrap(), (0, -3));
        assert_eq!(scroll_event(ScrollDirection::Down, 3).unwrap(), (0, 3));
        assert_eq!(scroll_event(ScrollDirection::Left, 3).unwrap(), (1, -3));
        assert_eq!(scroll_event(ScrollDirection::Right, 3).unwrap(), (1, 3));
        assert!(scroll_event(ScrollDirection::Up, i32::MIN).is_err());
        assert_eq!(button_code(Button::Left).unwrap(), 272);
        assert_eq!(button_code(Button::Right).unwrap(), 273);
        assert_eq!(button_code(Button::Middle).unwrap(), 274);
        assert!(button_code(Button::ScrollDown).is_err());
        assert!(cursor_position_blocking().unwrap_err().contains("CURSOR_OBSERVATION_UNAVAILABLE"));
    }

    #[derive(Default)]
    struct MockSink {
        calls: RefCell<Vec<(PressedInput, bool)>>,
        moves: RefCell<Vec<(u32, f64, f64)>>,
        fail_press: bool,
        fail_release: bool,
        fail_close: bool,
        close_delay: Duration,
        close_count: RefCell<usize>,
    }
    impl InputSink for &MockSink {
        fn send(&self, input: PressedInput, pressed: bool) -> Result<(), String> {
            self.calls.borrow_mut().push((input, pressed));
            if (pressed && self.fail_press) || (!pressed && self.fail_release) { Err("injected native error".to_owned()) } else { Ok(()) }
        }
        fn move_absolute(&self, node_id: u32, x: f64, y: f64) -> Result<(), String> { self.moves.borrow_mut().push((node_id, x, y)); Ok(()) }
        fn scroll(&self, _axis: u32, _steps: i32) -> Result<(), String> { Ok(()) }
        fn close(&self) -> Result<(), String> { std::thread::sleep(self.close_delay); *self.close_count.borrow_mut() += 1; if self.fail_close { Err("injected Close failure".to_owned()) } else { Ok(()) } }
    }

    #[test]
    fn failed_press_is_recorded_then_released_once_without_replay() {
        let sink = MockSink { fail_press: true, ..MockSink::default() };
        let mut guard = PressedInputGuard::new(&sink);
        assert!(guard.press(PressedInput::Keysym(0xffe3)).is_err());
        guard.release_all().unwrap();
        drop(guard);
        assert_eq!(*sink.calls.borrow(), vec![(PressedInput::Keysym(0xffe3), true), (PressedInput::Keysym(0xffe3), false)]);
    }

    #[test]
    fn exact_releases_reverse_order_and_close_ack_is_required_on_failure() {
        let sink = MockSink { fail_release: true, ..MockSink::default() };
        let mut guard = PressedInputGuard::new(&sink);
        guard.press(PressedInput::Keysym(0xffe3)).unwrap();
        guard.press(PressedInput::Keysym(0x61)).unwrap();
        let error = guard.release_all().unwrap_err();
        assert!(error.contains("Session.Close acknowledged"));
        assert!(guard.pressed.is_empty());
        drop(guard);
        assert_eq!(*sink.calls.borrow(), vec![(PressedInput::Keysym(0xffe3), true), (PressedInput::Keysym(0x61), true), (PressedInput::Keysym(0x61), false), (PressedInput::Keysym(0xffe3), false)]);
        assert_eq!(*sink.close_count.borrow(), 1);
    }

    #[test]
    fn failed_native_close_retains_exact_release_obligation_without_press_retry() {
        let sink = MockSink { fail_release: true, fail_close: true, ..MockSink::default() };
        let mut guard = PressedInputGuard::new(&sink);
        guard.press(PressedInput::Button(272)).unwrap();
        assert!(guard.release_all().unwrap_err().contains("CLEANUP_UNPROVEN"));
        assert_eq!(guard.pressed, vec![PressedInput::Button(272)]);
        drop(guard);
        assert_eq!(*sink.calls.borrow(), vec![(PressedInput::Button(272), true), (PressedInput::Button(272), false), (PressedInput::Button(272), false)]);
        assert_eq!(*sink.close_count.borrow(), 2);
    }
    #[test]
    fn cancelled_queued_worker_never_enters_native_operation_after_gate_wait() {
        let gate = Arc::new(Mutex::new(()));
        let held_gate = gate.lock().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let native_called = Arc::new(AtomicBool::new(false));
        let worker_gate = gate.clone();
        let worker_cancel = cancel.clone();
        let worker_called = native_called.clone();
        let (queued_tx, queued_rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            queued_tx.send(()).unwrap();
            with_input_gate(&worker_gate, &worker_cancel, || {
                worker_called.store(true, Ordering::Release);
                Ok(())
            })
        });
        queued_rx.recv().unwrap();
        cancel.store(true, Ordering::Release);
        drop(held_gate);
        assert!(worker.join().unwrap().unwrap_err().contains("INPUT_CANCELLED"));
        assert!(!native_called.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_while_pressed_stops_next_move_and_waits_for_release_and_close() {
        let cancel = Arc::new(AtomicBool::new(false));
        let control = OperationControl { cancel: cancel.clone(), deadline: Instant::now() + INPUT_TIMEOUT };
        let sink = MockSink { close_delay: Duration::from_millis(25), ..MockSink::default() };
        let mut operation = Operation { guard: PressedInputGuard::new(&sink), capture: capture(), control };
        operation.press(PressedInput::Button(272)).unwrap();
        operation.move_absolute(-1910, 40).unwrap();
        // A caller drops its future after the first pressed movement.
        cancel.store(true, Ordering::Release);
        let next_move = operation.move_absolute(-1900, 50);
        assert!(next_move.is_err());
        let started = Instant::now();
        let result = settle_input(&mut operation.guard, next_move, &cancel);
        assert!(result.unwrap_err().contains("INPUT_CANCELLED"));
        assert_eq!(*sink.moves.borrow(), vec![(41, 10.0, 10.0)]);
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert_eq!(*sink.close_count.borrow(), 1);
        assert!(operation.guard.pressed.is_empty());
        assert_eq!(*sink.calls.borrow(), vec![(PressedInput::Button(272), true), (PressedInput::Button(272), false)]);
        assert!(operation.control.sleep(Duration::from_secs(1)).unwrap_err().contains("INPUT_CANCELLED"));
    }
    #[test]
    fn unwind_releases_exact_obligation_before_worker_returns() {
        let sink = MockSink::default();
        {
            let mut guard = PressedInputGuard::new(&sink);
            guard.press(PressedInput::Button(272)).unwrap();
        }
        assert_eq!(*sink.calls.borrow(), vec![(PressedInput::Button(272), true), (PressedInput::Button(272), false)]);
    }

    #[test]
    fn drag_interpolation_does_not_overflow_signed_coordinate_space() {
        assert_eq!(drag_axis(i32::MIN, i32::MAX, 0), i32::MIN);
        assert_eq!(drag_axis(i32::MIN, i32::MAX, DRAG_STEPS), i32::MAX);
    }
}