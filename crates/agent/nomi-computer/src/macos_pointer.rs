//! macOS pointer events keep the point admitted by the caller through the whole
//! gesture. Reading the system cursor after an asynchronous move is not a
//! substitute for that point. The caller retains the main-thread, TCC and
//! joined-worker boundary; this module owns matching mouse release obligations.

use std::ffi::c_void;
use std::time::Duration;

use enigo::Button;

use super::{
    CLICK_PAUSE, DRAG_STEPS, ReleaseReport, ScrollDirection, combine_input_and_cleanup_error,
    drag_axis, release_obligations, release_reports_result,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Self { Self { x, y } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerButton { Left, Right, Middle, Back, Forward }

impl PointerButton {
    fn from_enigo(button: Button) -> Result<Self, String> {
        match button {
            Button::Left => Ok(Self::Left),
            Button::Right => Ok(Self::Right),
            Button::Middle => Ok(Self::Middle),
            Button::Back => Ok(Self::Back),
            Button::Forward => Ok(Self::Forward),
            _ => Err("A macOS pointer button must be left, right, middle, back or forward".into()),
        }
    }

    fn number(self) -> u32 {
        match self { Self::Left => 0, Self::Right => 1, Self::Middle => 2, Self::Back => 3, Self::Forward => 4 }
    }

    fn event_type(self, pressed: bool) -> u32 {
        match (self, pressed) {
            (Self::Left, true) => 1, (Self::Left, false) => 2,
            (Self::Right, true) => 3, (Self::Right, false) => 4,
            (_, true) => 25, (_, false) => 26,
        }
    }

    fn dragged_type(self) -> u32 {
        match self { Self::Left => 6, Self::Right => 7, _ => 27 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MousePress {
    button: PointerButton,
    point: Point,
    click_count: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Move { point: Point, held: Option<PointerButton>, delta: Option<(i64, i64)> },
    Button { press: MousePress, down: bool },
    Scroll { point: Point, vertical: i32, horizontal: i32 },
}

trait PointerPort {
    /// An error can follow a partial effect. It never releases the caller's
    /// matching obligation or authorizes a repeated press/move.
    fn post(&mut self, event: Event) -> Result<(), String>;
}

struct Pointer<P: PointerPort> {
    port: P,
    position: Option<Point>,
    pressed: Vec<MousePress>,
}

impl<P: PointerPort> Pointer<P> {
    fn new(port: P) -> Self { Self { port, position: None, pressed: Vec::new() } }

    fn move_to(&mut self, point: Point) -> Result<(), String> {
        let delta = self.position.map(|previous| (
            i64::from(point.x) - i64::from(previous.x),
            i64::from(point.y) - i64::from(previous.y),
        ));
        let held = self.pressed.last().map(|press| press.button);
        // Preserve the latest attempted point even if post reports a partial
        // failure. The retained release must not go back to an old start point
        // or re-read a system cursor that could now belong to another gesture.
        self.position = Some(point);
        for press in &mut self.pressed { press.point = point; }
        self.port.post(Event::Move { point, held, delta })
    }

    fn press(&mut self, button: PointerButton, click_count: i64) -> Result<(), String> {
        let point = self.position.ok_or_else(|| "Mouse press has no admitted point".to_owned())?;
        let press = MousePress { button, point, click_count };
        self.pressed.push(press);
        if let Err(error) = self.port.post(Event::Button { press, down: true }) {
            let cleanup = self.release_all();
            return Err(combine_input_and_cleanup_error(error, cleanup));
        }
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), String> {
        let report = release_obligations(&mut self.pressed, |press| {
            self.port.post(Event::Button { press, down: false })
        });
        release_reports_result(ReleaseReport::Clean, report)
    }
}

impl<P: PointerPort> Drop for Pointer<P> {
    fn drop(&mut self) {
        if !self.pressed.is_empty() { let _ = self.release_all(); }
    }
}

fn click_with<P: PointerPort>(
    port: P, point: Point, button: PointerButton, count: u32, mut pause: impl FnMut(Duration),
) -> Result<(), String> {
    let mut pointer = Pointer::new(port);
    pointer.move_to(point)?;
    for index in 0..count {
        if index > 0 { pause(CLICK_PAUSE); }
        pointer.press(button, i64::from(index) + 1)?;
        pause(CLICK_PAUSE);
        pointer.release_all()?;
    }
    Ok(())
}

fn drag_with<P: PointerPort>(
    port: P, start: Point, end: Point, mut pause: impl FnMut(Duration),
) -> Result<(), String> {
    let mut pointer = Pointer::new(port);
    pointer.move_to(start)?;
    pointer.press(PointerButton::Left, 1)?;
    pause(CLICK_PAUSE);
    let movement = (|| {
        for step in 1..=DRAG_STEPS {
            pointer.move_to(Point::new(drag_axis(start.x, end.x, step), drag_axis(start.y, end.y, step)))?;
            pause(Duration::from_millis(10));
        }
        Ok(())
    })();
    let cleanup = pointer.release_all();
    match (movement, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), cleanup) => Err(combine_input_and_cleanup_error(error, cleanup)),
    }
}

fn scroll_wheels(direction: ScrollDirection, amount: i32) -> Result<(i32, i32), String> {
    let negative = || amount.checked_neg().ok_or_else(|| "Scroll amount exceeds the macOS wheel range".to_owned());
    match direction {
        ScrollDirection::Up => Ok((amount, 0)),
        ScrollDirection::Down => Ok((negative()?, 0)),
        ScrollDirection::Left => Ok((0, amount)),
        ScrollDirection::Right => Ok((0, negative()?)),
    }
}

fn scroll_with<P: PointerPort>(
    port: P, point: Point, move_first: bool, vertical: i32, horizontal: i32,
) -> Result<(), String> {
    let mut pointer = Pointer::new(port);
    if move_first { pointer.move_to(point)?; }
    pointer.port.post(Event::Scroll { point, vertical, horizontal })
}

pub(super) fn move_absolute(x: i32, y: i32) -> Result<(), String> {
    Pointer::new(QuartzPort::new()?).move_to(Point::new(x, y))
}

pub(super) fn click(x: i32, y: i32, button: Button, count: u32) -> Result<(), String> {
    let button = PointerButton::from_enigo(button)?;
    click_with(QuartzPort::new()?, Point::new(x, y), button, count, std::thread::sleep)
}

pub(super) fn drag(start_x: i32, start_y: i32, end_x: i32, end_y: i32) -> Result<(), String> {
    drag_with(QuartzPort::new()?, Point::new(start_x, start_y), Point::new(end_x, end_y), std::thread::sleep)
}

pub(super) fn scroll(at: Option<(i32, i32)>, direction: ScrollDirection, amount: i32) -> Result<(), String> {
    let (vertical, horizontal) = scroll_wheels(direction, amount)?;
    let (x, y) = match at { Some(point) => point, None => super::macos_cursor::position()? };
    scroll_with(QuartzPort::new()?, Point::new(x, y), at.is_some(), vertical, horizontal)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct QuartzPoint { x: f64, y: f64 }

impl From<Point> for QuartzPoint {
    fn from(point: Point) -> Self { Self { x: f64::from(point.x), y: f64::from(point.y) } }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceCreate(state_id: i32) -> *const c_void;
    fn CGEventCreateMouseEvent(source: *const c_void, event_type: u32, point: QuartzPoint, button: u32) -> *const c_void;
    fn CGEventCreateScrollWheelEvent2(source: *const c_void, units: u32, wheel_count: u32, wheel1: i32, wheel2: i32, wheel3: i32) -> *const c_void;
    fn CGEventSetLocation(event: *const c_void, point: QuartzPoint);
    fn CGEventSetIntegerValueField(event: *const c_void, field: u32, value: i64);
    fn CGEventSetFlags(event: *const c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *const c_void);
}

struct QuartzPort { source: *const c_void }

impl QuartzPort {
    fn new() -> Result<Self, String> {
        // Preserve Enigo's independent-of-physical-keyboard-state contract.
        // This source belongs to one admitted operation and its release guard.
        let source = unsafe { CGEventSourceCreate(-1) };
        if source.is_null() { return Err("macOS could not create the pointer event source".into()); }
        Ok(Self { source })
    }
}

impl Drop for QuartzPort {
    fn drop(&mut self) { unsafe { core_foundation::base::CFRelease(self.source) }; }
}

impl PointerPort for QuartzPort {
    fn post(&mut self, event: Event) -> Result<(), String> {
        // Public CGEventTypes.h constants: mouse move=5, clickState=1,
        // buttonNumber=3, deltaX/Y=4/5, HID tap=0, line scroll unit=1.
        let native = match event {
            Event::Move { point, held, .. } => unsafe {
                CGEventCreateMouseEvent(self.source, held.map_or(5, PointerButton::dragged_type), point.into(), held.map_or(0, PointerButton::number))
            },
            Event::Button { press, down } => unsafe {
                CGEventCreateMouseEvent(self.source, press.button.event_type(down), press.point.into(), press.button.number())
            },
            Event::Scroll { vertical, horizontal, .. } => unsafe {
                CGEventCreateScrollWheelEvent2(self.source, 1, if horizontal == 0 { 1 } else { 2 }, vertical, horizontal, 0)
            },
        };
        if native.is_null() { return Err("macOS could not create a pointer event; its input effect is unproven".into()); }
        // Every event owns a +1 CF reference. No fallible work follows creation.
        unsafe {
            match event {
                Event::Move { delta: Some((x, y)), .. } => {
                    CGEventSetIntegerValueField(native, 4, x);
                    CGEventSetIntegerValueField(native, 5, y);
                }
                Event::Button { press, .. } => {
                    CGEventSetIntegerValueField(native, 1, press.click_count);
                    CGEventSetIntegerValueField(native, 3, i64::from(press.button.number()));
                }
                Event::Scroll { point, .. } => CGEventSetLocation(native, point.into()),
                _ => {}
            }
            // kCGEventFlagMaskNonCoalesced; no physical modifier inheritance.
            CGEventSetFlags(native, 0x100);
            // CGEventPost returns void: this proves publication only. The tool
            // still asks the caller to observe the result; it has no receiver ACK.
            CGEventPost(0, native);
            core_foundation::base::CFRelease(native);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Recorder {
        events: Arc<Mutex<Vec<Event>>>,
        fail: Box<dyn FnMut(Event, usize) -> bool>,
    }

    impl PointerPort for Recorder {
        fn post(&mut self, event: Event) -> Result<(), String> {
            let index = {
                let mut events = self.events.lock().unwrap();
                events.push(event);
                events.len()
            };
            if (self.fail)(event, index) { Err("injected pointer publication failure".into()) }
            else { Ok(()) }
        }
    }

    fn recorder(fail: impl FnMut(Event, usize) -> bool + 'static) -> (Recorder, Arc<Mutex<Vec<Event>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (Recorder { events: events.clone(), fail: Box::new(fail) }, events)
    }

    #[test]
    fn clicks_keep_the_admitted_global_point_for_every_button_event() {
        for point in [Point::new(314, 277), Point::new(-1160, 52), Point::new(1800, -450)] {
            for (button, count) in [(PointerButton::Left, 1), (PointerButton::Right, 1), (PointerButton::Left, 2)] {
                let (port, events) = recorder(|_, _| false);
                click_with(port, point, button, count, |_| {}).unwrap();
                let events = events.lock().unwrap();
                assert_eq!(events[0], Event::Move { point, held: None, delta: None });
                for index in 0..count {
                    let press = MousePress { button, point, click_count: i64::from(index) + 1 };
                    assert_eq!(events[1 + index as usize * 2], Event::Button { press, down: true });
                    assert_eq!(events[2 + index as usize * 2], Event::Button { press, down: false });
                }
                assert_eq!(events.len(), 1 + count as usize * 2);
            }
        }
    }

    #[test]
    fn quartz_points_preserve_negative_and_full_integer_coordinates() {
        for point in [Point::new(i32::MIN, i32::MAX), Point::new(-1160, -900), Point::new(314, 277)] {
            let quartz: QuartzPoint = point.into();
            assert_eq!(quartz.x, f64::from(point.x));
            assert_eq!(quartz.y, f64::from(point.y));
        }
    }

    #[test]
    fn partial_press_failure_releases_exact_button_point_and_count_without_retrying_press() {
        let (port, events) = recorder(|event, _| matches!(event, Event::Button { down: true, .. }));
        let point = Point::new(-500, 800);
        let error = click_with(port, point, PointerButton::Right, 1, |_| {}).unwrap_err();
        assert!(error.contains("Input cleanup released every recorded key/button"));
        let events = events.lock().unwrap();
        let press = MousePress { button: PointerButton::Right, point, click_count: 1 };
        assert_eq!(&events[1..], &[Event::Button { press, down: true }, Event::Button { press, down: false }]);
    }

    #[test]
    fn panic_after_press_admission_keeps_the_exact_release_obligation() {
        let (port, events) = recorder(|event, _| {
            if matches!(event, Event::Button { down: true, .. }) { panic!("injected post panic"); }
            false
        });
        let point = Point::new(-500, -800);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            click_with(port, point, PointerButton::Left, 1, |_| {}).unwrap();
        }));
        assert!(outcome.is_err());
        let press = MousePress { button: PointerButton::Left, point, click_count: 1 };
        assert_eq!(&events.lock().unwrap()[1..], &[Event::Button { press, down: true }, Event::Button { press, down: false }]);
    }

    #[test]
    fn release_failure_retries_only_the_matching_release_and_remains_uncertain() {
        let (port, events) = recorder(|event, index| matches!(event, Event::Button { down: false, .. }) && index == 3);
        let error = click_with(port, Point::new(500, 800), PointerButton::Left, 1, |_| {}).unwrap_err();
        assert!(error.contains("effect is uncertain"));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[2], events[3]);
    }

    #[test]
    fn persistent_release_failure_is_unproven_and_keeps_obligation_for_drop() {
        let (port, events) = recorder(|event, _| matches!(event, Event::Button { down: false, .. }));
        let error = click_with(port, Point::new(500, 800), PointerButton::Left, 1, |_| {}).unwrap_err();
        assert!(error.contains("cleanup is unproven"));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 6);
        assert!(events[2..].iter().all(|event| *event == events[2]));
    }

    #[test]
    fn move_failure_before_press_never_admits_a_button() {
        let (port, events) = recorder(|event, _| matches!(event, Event::Move { .. }));
        click_with(port, Point::new(500, 800), PointerButton::Left, 1, |_| {}).unwrap_err();
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[test]
    fn drag_uses_local_held_state_and_releases_at_the_latest_point() {
        let (port, events) = recorder(|_, _| false);
        let start = Point::new(-500, 800);
        let end = Point::new(800, -450);
        drag_with(port, start, end, |_| {}).unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), DRAG_STEPS as usize + 3);
        for event in &events[2..events.len() - 1] {
            assert!(matches!(event, Event::Move { held: Some(PointerButton::Left), .. }));
        }
        assert_eq!(*events.last().unwrap(), Event::Button { press: MousePress { button: PointerButton::Left, point: end, click_count: 1 }, down: false });
    }

    #[test]
    fn partial_drag_move_failure_releases_at_latest_attempt_without_movement_retry() {
        let (port, events) = recorder(|event, index| matches!(event, Event::Move { held: Some(_), .. }) && index == 4);
        let error = drag_with(port, Point::new(0, 0), Point::new(80, -80), |_| {}).unwrap_err();
        assert!(error.contains("Input cleanup released every recorded key/button"));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 5);
        assert_eq!(events[3], Event::Move { point: Point::new(20, -20), held: Some(PointerButton::Left), delta: Some((10, -10)) });
        assert_eq!(events[4], Event::Button { press: MousePress { button: PointerButton::Left, point: Point::new(20, -20), click_count: 1 }, down: false });
    }

    #[test]
    fn drag_delta_math_handles_full_coordinate_range_without_overflow() {
        let (port, events) = recorder(|_, _| false);
        let mut pointer = Pointer::new(port);
        pointer.move_to(Point::new(i32::MIN, i32::MAX)).unwrap();
        pointer.press(PointerButton::Left, 1).unwrap();
        pointer.move_to(Point::new(i32::MAX, i32::MIN)).unwrap();
        pointer.release_all().unwrap();
        assert!(matches!(events.lock().unwrap()[2], Event::Move { delta: Some((4_294_967_295, -4_294_967_295)), .. }));
    }

    #[test]
    fn targeted_scroll_keeps_exact_move_point_in_the_scroll_event() {
        let (port, events) = recorder(|_, _| false);
        let point = Point::new(-850, -100);
        scroll_with(port, point, true, 0, 3).unwrap();
        assert_eq!(*events.lock().unwrap(), vec![Event::Move { point, held: None, delta: None }, Event::Scroll { point, vertical: 0, horizontal: 3 }]);
        assert_eq!(scroll_wheels(ScrollDirection::Up, 3).unwrap(), (3, 0));
        assert_eq!(scroll_wheels(ScrollDirection::Down, 3).unwrap(), (-3, 0));
        assert_eq!(scroll_wheels(ScrollDirection::Left, 3).unwrap(), (0, 3));
        assert_eq!(scroll_wheels(ScrollDirection::Right, 3).unwrap(), (0, -3));
        assert!(scroll_wheels(ScrollDirection::Down, i32::MIN).is_err());
    }

    #[test]
    fn targeted_scroll_stops_before_scroll_if_the_initial_move_fails() {
        let (port, events) = recorder(|event, _| matches!(event, Event::Move { .. }));
        scroll_with(port, Point::new(-850, -100), true, 3, 0).unwrap_err();
        assert_eq!(events.lock().unwrap().len(), 1);
    }
}
