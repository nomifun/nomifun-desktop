//! Isolated native X11 regression. Run only against a disposable X server and
//! EWMH window manager; it never selects or interacts with existing windows.
#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use nomi_computer::{ComputerTool, capability::ComputerAction};
use nomi_config::config::ComputerConfig;
use serde_json::json;
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xproto::{ConnectionExt as _, CreateWindowAux, EventMask, WindowClass}};

#[tokio::test]
#[ignore = "requires an isolated native X11 server and EWMH window manager"]
async fn exact_focus_capture_and_input_use_product_tool_without_focus_clicks() {
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
