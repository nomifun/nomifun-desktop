//! Isolated native X11 regression. Run only against a disposable X server and
//! EWMH window manager; it never selects or interacts with existing windows.
#![cfg(target_os = "linux")]

use std::{sync::Arc, time::{Duration, Instant}};

use nomi_computer::{ComputerTool, capability::ComputerAction, tool::is_proven_cancelled_before_dispatch};
use nomi_config::config::ComputerConfig;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xproto::{ConnectionExt as _, CreateWindowAux, EventMask, WindowClass}};
use x11rb::rust_connection::RustConnection;

static NATIVE_DISPLAY: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn exclusive_display() -> std::sync::MutexGuard<'static, ()> { NATIVE_DISPLAY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

async fn focused_target(connection: &RustConnection, screen_index: usize, tool: &ComputerTool) -> (u32, i32, i32) {
    let screen = &connection.setup().roots[screen_index];
    let window = connection.generate_id().unwrap();
    let events = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION
        | EventMask::KEY_PRESS | EventMask::KEY_RELEASE | EventMask::EXPOSURE;
    connection.create_window(x11rb::COPY_DEPTH_FROM_PARENT, window, screen.root, 60, 90, 420, 300, 0, WindowClass::INPUT_OUTPUT, 0,
        &CreateWindowAux::new().background_pixel(screen.white_pixel).event_mask(events)).unwrap().check().unwrap();
    connection.map_window(window).unwrap().check().unwrap();
    connection.flush().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let focus = tool.execute_authorized(ComputerAction::Input.id(), json!({"action":"focus_window","window_id":window})).await;
    assert!(!focus.is_error, "{}", focus.content);
    let origin = connection.translate_coordinates(window, screen.root, 0, 0).unwrap().reply().unwrap();
    (window, i32::from(origin.dst_x), i32::from(origin.dst_y))
}

fn drain(connection: &RustConnection) -> Vec<Event> {
    let mut events = Vec::new();
    while let Some(event) = connection.poll_for_event().unwrap() { events.push(event); }
    events
}

async fn assert_only_barrier_input(connection: &RustConnection, tool: &ComputerTool, window: u32) {
    let barrier = tool.execute_authorized(ComputerAction::Input.id(), json!({"action":"key","key":"x"})).await;
    assert!(!barrier.is_error, "{}", barrier.content);
    let deadline = Instant::now() + Duration::from_secs(2);
    let (mut presses, mut releases) = (0, 0);
    while Instant::now() < deadline && (presses, releases) != (1, 1) {
        for event in drain(connection) {
            match event {
                Event::KeyPress(event) => { assert_eq!(event.event, window); presses += 1; }
                Event::KeyRelease(event) => { assert_eq!(event.event, window); releases += 1; }
                Event::ButtonPress(_) | Event::ButtonRelease(_) | Event::MotionNotify(_) => panic!("a withdrawn pointer input reached the target: {event:?}"),
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!((presses, releases), (1, 1), "only the barrier key may reach the target after Stop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated native X11 server and EWMH window manager"]
async fn cancelled_token_is_rejected_at_entry_without_x11_input() {
    let _display = exclusive_display();
    assert!(matches!(nomi_computer::linux_x11::desktop_state(), nomi_computer::linux_x11::DesktopState::X11 { input: true, window_management: true }));
    let (connection, screen_index) = x11rb::connect(None).unwrap();
    let tool = ComputerTool::new(&ComputerConfig { enabled: true, max_screenshot_edge: 800 });
    let (window, x, y) = focused_target(&connection, screen_index, &tool).await;
    let root = connection.setup().roots[screen_index].root;
    let pointer_before = connection.query_pointer(root).unwrap().reply().unwrap();
    drain(&connection);
    let stopped = CancellationToken::new();
    stopped.cancel();
    for input in [json!({"action":"key","key":"ctrl+shift+t"}), json!({"action":"type","text":"withdrawn"}),
        json!({"action":"left_click","x":x + 40,"y":y + 40}), json!({"action":"scroll","x":x + 40,"y":y + 40,"direction":"down"}),
        json!({"action":"left_click_drag","start_x":x + 30,"start_y":y + 30,"end_x":x + 200,"end_y":y + 150}),
        json!({"action":"focus_window","window_id":window})] {
        let result = tool.execute_authorized_controlled(ComputerAction::Input.id(), input.clone(), &stopped).await;
        assert!(is_proven_cancelled_before_dispatch(&result), "{input}: {}", result.content);
    }
    let pointer_after = connection.query_pointer(root).unwrap().reply().unwrap();
    assert_eq!((pointer_before.root_x, pointer_before.root_y), (pointer_after.root_x, pointer_after.root_y), "withdrawn input cannot move the pointer");
    assert_only_barrier_input(&connection, &tool, window).await;
    connection.destroy_window(window).unwrap().check().unwrap();
    println!("native X11 Stop before dispatch: six withdrawn inputs admitted no device event; barrier key alone reached the target");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated native X11 server and EWMH window manager"]
async fn stop_during_dispatched_drag_joins_the_original_input_and_releases_its_button() {
    let _display = exclusive_display();
    assert!(matches!(nomi_computer::linux_x11::desktop_state(), nomi_computer::linux_x11::DesktopState::X11 { input: true, window_management: true }));
    let (connection, screen_index) = x11rb::connect(None).unwrap();
    let tool = Arc::new(ComputerTool::new(&ComputerConfig { enabled: true, max_screenshot_edge: 800 }));
    let (window, x, y) = focused_target(&connection, screen_index, &tool).await;
    drain(&connection);
    let stop = CancellationToken::new();
    let (end_x, end_y) = (x + 200, y + 150);
    let drag = tokio::spawn({
        let (tool, stop) = (tool.clone(), stop.clone());
        async move { tool.execute_authorized_controlled(ComputerAction::Input.id(),
            json!({"action":"left_click_drag","start_x":x + 30,"start_y":y + 30,"end_x":end_x,"end_y":end_y}), &stop).await }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut pressed, mut released, mut motions) = (0, 0, 0);
    let mut release_seen_before_stop = None;
    while Instant::now() < deadline && released == 0 {
        for event in drain(&connection) {
            match event {
                Event::ButtonPress(event) => { assert_eq!(event.event, window); pressed += 1; }
                Event::ButtonRelease(event) => { assert_eq!(event.event, window); released += 1; }
                Event::MotionNotify(_) => motions += 1,
                Event::KeyPress(_) | Event::KeyRelease(_) => panic!("a drag cannot emit keyboard input"),
                _ => {}
            }
        }
        if pressed == 1 && release_seen_before_stop.is_none() {
            release_seen_before_stop = Some(released > 0);
            stop.cancel();
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(release_seen_before_stop, Some(false), "Stop must be requested while the dispatched button is still pressed");
    let result = drag.await.unwrap();
    assert!(!result.is_error && !is_proven_cancelled_before_dispatch(&result), "the admitted input keeps its real result: {}", result.content);
    for event in drain(&connection) {
        match event {
            Event::ButtonRelease(event) => { assert_eq!(event.event, window); released += 1; }
            Event::ButtonPress(_) => pressed += 1,
            Event::MotionNotify(_) => motions += 1,
            _ => {}
        }
    }
    assert_eq!((pressed, released), (1, 1), "the original press must be released exactly once after Stop");
    assert!(motions > 0, "the admitted drag effect is real and is not rolled back");
    let root = connection.setup().roots[screen_index].root;
    let pointer = connection.query_pointer(root).unwrap().reply().unwrap();
    assert_eq!((i32::from(pointer.root_x), i32::from(pointer.root_y)), (end_x, end_y));
    assert_eq!(u16::from(pointer.mask) & 0x1f00, 0, "no pointer button may remain held");
    let late = tool.execute_authorized_controlled(ComputerAction::Input.id(), json!({"action":"key","key":"ctrl+shift+t"}), &stop).await;
    assert!(is_proven_cancelled_before_dispatch(&late), "the stopped invocation admits no further input: {}", late.content);
    assert_only_barrier_input(&connection, &tool, window).await;
    connection.destroy_window(window).unwrap().check().unwrap();
    println!("native X11 Stop during dispatched drag: original worker joined, button released once, real result retained, no late input");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated native X11 server and EWMH window manager"]
async fn queued_input_cancelled_before_dispatch_produces_no_native_effect() {
    let _display = exclusive_display();
    assert!(matches!(nomi_computer::linux_x11::desktop_state(), nomi_computer::linux_x11::DesktopState::X11 { input: true, window_management: true }));
    let (connection, screen_index) = x11rb::connect(None).unwrap();
    let tool = Arc::new(ComputerTool::new(&ComputerConfig { enabled: true, max_screenshot_edge: 800 }));
    let (window, x, y) = focused_target(&connection, screen_index, &tool).await;
    drain(&connection);
    let stop = CancellationToken::new();
    let drag = tokio::spawn({
        let (tool, stop) = (tool.clone(), stop.clone());
        async move { tool.execute_authorized_controlled(ComputerAction::Input.id(),
            json!({"action":"left_click_drag","start_x":x + 30,"start_y":y + 30,"end_x":x + 200,"end_y":y + 150}), &stop).await }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut pressed, mut released, mut motions) = (0, 0, 0);
    while Instant::now() < deadline && pressed == 0 {
        for event in drain(&connection) {
            match event {
                Event::ButtonPress(event) => { assert_eq!(event.event, window); pressed += 1; }
                Event::ButtonRelease(_) => released += 1,
                Event::MotionNotify(_) => motions += 1,
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(pressed, 1, "the drag must be dispatched with the button held before the queued input is admitted");
    assert_eq!(released, 0, "the drag must still be in flight when the queued input is admitted");

    let stop2 = CancellationToken::new();
    let queued = {
        let (tool, stop2) = (tool.clone(), stop2.clone());
        tokio::spawn(async move {
            stop2.cancelled().await;
            tool.execute_authorized_controlled(ComputerAction::Input.id(),
                json!({"action":"key","key":"q"}), &stop2).await
        })
    };
    stop2.cancel();
    let withdrawn = queued.await.unwrap();
    assert!(is_proven_cancelled_before_dispatch(&withdrawn),
        "the queued input must return the product's cancelled-before-dispatch result: {}", withdrawn.content);

    stop.cancel();
    let drag_result = drag.await.unwrap();
    assert!(!drag_result.is_error && !is_proven_cancelled_before_dispatch(&drag_result),
        "the admitted drag keeps its real result: {}", drag_result.content);
    for event in drain(&connection) {
        match event {
            Event::ButtonRelease(event) => { assert_eq!(event.event, window); released += 1; }
            Event::ButtonPress(_) => pressed += 1,
            Event::MotionNotify(_) => motions += 1,
            Event::KeyPress(_) | Event::KeyRelease(_) => panic!("the withdrawn queued input must not reach the target: {event:?}"),
            _ => {}
        }
    }
    assert_eq!((pressed, released), (1, 1), "the original press must be released exactly once");
    assert!(motions > 0, "the admitted drag effect is real");
    assert_only_barrier_input(&connection, &tool, window).await;
    connection.destroy_window(window).unwrap().check().unwrap();
    println!("native X11 queued input withdrawn before dispatch: no key event, drag completed, barrier key is the only key");
}

#[tokio::test]
#[ignore = "requires an isolated native X11 server and EWMH window manager"]
async fn exact_focus_capture_and_input_use_product_tool_without_focus_clicks() {
    let _display = exclusive_display();
    assert!(matches!(nomi_computer::linux_x11::desktop_state(), nomi_computer::linux_x11::DesktopState::X11 { input: true, window_management: true }));
    let (connection, screen_index) = x11rb::connect(None).unwrap();
    let screen = &connection.setup().roots[screen_index];
    let mut windows = Vec::new();
    for x in [40, 400] {
        let window = connection.generate_id().unwrap();
        connection.create_window(x11rb::COPY_DEPTH_FROM_PARENT, window, screen.root, x, 80, 300, 220, 0, WindowClass::INPUT_OUTPUT, 0,
            &CreateWindowAux::new().background_pixel(screen.white_pixel).event_mask(EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::KEY_PRESS | EventMask::KEY_RELEASE | EventMask::EXPOSURE)).unwrap().check().unwrap();
        connection.map_window(window).unwrap().check().unwrap();
        windows.push(window);
    }
    connection.flush().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    while connection.poll_for_event().unwrap().is_some() {}
    let pointer_before = connection.query_pointer(screen.root).unwrap().reply().unwrap();
    let tool = ComputerTool::new(&ComputerConfig { enabled: true, max_screenshot_edge: 800 });
    for &window in &windows {
        let focus = tool.execute_authorized(ComputerAction::Input.id(), json!({"action":"focus_window","window_id":window})).await;
        assert!(!focus.is_error, "{}", focus.content);
    }
    let pointer_after = connection.query_pointer(screen.root).unwrap().reply().unwrap();
    assert_eq!((pointer_before.root_x, pointer_before.root_y), (pointer_after.root_x, pointer_after.root_y), "focus must not move the cursor");
    while let Some(event) = connection.poll_for_event().unwrap() {
        assert!(!matches!(event, Event::ButtonPress(_) | Event::ButtonRelease(_)), "focus must not click application content");
    }
    let screenshot = tool.execute_authorized(ComputerAction::Observe.id(), json!({"action":"screenshot"})).await;
    assert!(!screenshot.is_error, "{}", screenshot.content);
    assert_eq!(screenshot.images.len(), 1);
    let key = tool.execute_authorized(ComputerAction::Input.id(), json!({"action":"key","key":"ctrl+shift+t"})).await;
    assert!(!key.is_error, "{}", key.content);
    connection.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut presses = Vec::new();
    let mut releases = Vec::new();
    while Instant::now() < deadline {
        while let Some(event) = connection.poll_for_event().unwrap() {
            match event {
                Event::KeyPress(event) => { assert_eq!(event.event, windows[1]); presses.push(event.detail); }
                Event::KeyRelease(event) => { assert_eq!(event.event, windows[1]); releases.push(event.detail); }
                _ => {}
            }
        }
        if presses.len() >= 3 && releases.len() >= 3 { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(presses.len(), 3, "the exact target must receive all key presses");
    presses.sort_unstable(); releases.sort_unstable();
    assert_eq!(presses, releases, "every pressed key must be released");
    for window in windows { connection.destroy_window(window).unwrap().check().unwrap(); }
    connection.flush().unwrap();
    println!("native X11 product tool: exact EWMH focus; no focus click; screenshot PNG; target key press/release confirmed");
}
