//! Native X11 desktop admission and EWMH activation. XWayland is deliberately
//! not a global-desktop backend: its root pixels and input cover X clients only.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, MapState,
};
use x11rb::rust_connection::RustConnection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopState {
    X11 { input: bool, window_management: bool },
    Wayland,
    Unavailable(String),
}

fn connect() -> Result<(RustConnection, usize), String> {
    x11rb::connect(None).map_err(|error| format!("Cannot connect to the current X11 display: {error}"))
}

fn atom(connection: &RustConnection, name: &[u8]) -> Result<u32, String> {
    connection.intern_atom(false, name).map_err(|e| e.to_string())?
        .reply().map(|reply| reply.atom).map_err(|e| e.to_string())
}

fn property(connection: &RustConnection, window: u32, name: u32, type_: AtomEnum) -> Result<Vec<u32>, String> {
    let reply = connection.get_property(false, window, name, type_, 0, 4096)
        .map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
    if reply.format == 0 {
        return Ok(Vec::new());
    }
    if reply.format != 32 || reply.type_ != u32::from(type_) || reply.bytes_after != 0 {
        return Err("The X11 window manager returned an invalid or oversized property".to_owned());
    }
    Ok(reply.value32().ok_or("The X11 property is not a 32-bit value")?.collect())
}

fn has_extension(connection: &RustConnection, name: &[u8]) -> Result<bool, String> {
    connection.query_extension(name).map_err(|e| e.to_string())?
        .reply().map(|reply| reply.present).map_err(|e| e.to_string())
}

fn supports_activation(connection: &RustConnection, root: u32) -> Result<bool, String> {
    let check = atom(connection, b"_NET_SUPPORTING_WM_CHECK")?;
    let manager = property(connection, root, check, AtomEnum::WINDOW)?;
    let Some(&manager) = manager.first() else { return Ok(false) };
    // EWMH requires the manager to identify itself on both windows. A stale
    // root property is not evidence that any manager will handle our request.
    if property(connection, manager, check, AtomEnum::WINDOW)? != [manager] {
        return Ok(false);
    }
    let supported = atom(connection, b"_NET_SUPPORTED")?;
    let active = atom(connection, b"_NET_ACTIVE_WINDOW")?;
    Ok(property(connection, root, supported, AtomEnum::ATOM)?.contains(&active))
}

pub fn desktop_state() -> DesktopState {
    let result = (|| {
        let (connection, screen) = connect()?;
        // Check the actual server first. Unsetting WAYLAND_DISPLAY must not
        // make XWayland appear to be a complete X11 desktop.
        if has_extension(&connection, b"XWAYLAND")? {
            return Ok(DesktopState::Wayland);
        }
        let input = has_extension(&connection, b"XTEST")?;
        let root = connection.setup().roots[screen].root;
        Ok(DesktopState::X11 { input, window_management: supports_activation(&connection, root)? })
    })();
    match result {
        Ok(state) => state,
        Err(error) => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var("XDG_SESSION_TYPE").is_ok_and(|session| session.eq_ignore_ascii_case("wayland"))
            {
                DesktopState::Wayland
            } else {
                DesktopState::Unavailable(error)
            }
        }
    }
}

pub fn require_global_x11(input: bool) -> Result<(), String> {
    match desktop_state() {
        DesktopState::X11 { input: available, .. } if !input || available => Ok(()),
        DesktopState::X11 { .. } => Err("COMPUTER_NATIVE_INPUT_UNAVAILABLE: the current X11 server does not expose XTEST".to_owned()),
        DesktopState::Wayland => Err("COMPUTER_WAYLAND_BACKEND_UNAVAILABLE: global Wayland capture/input requires a granted RemoteDesktop/ScreenCast portal session; XWayland is not a global desktop backend".to_owned()),
        DesktopState::Unavailable(error) => Err(format!("COMPUTER_NO_DESKTOP: {error}")),
    }
}

/// Request activation through the window manager, then verify the exact
/// window became active. Never click application content as a focus fallback.
pub fn focus_window(window_id: u32) -> Result<(), String> {
    require_global_x11(false)?;
    let (connection, screen) = connect()?;
    let root = connection.setup().roots[screen].root;
    if !supports_activation(&connection, root)? {
        return Err("COMPUTER_FOCUS_UNAVAILABLE: this X11 window manager does not implement EWMH _NET_ACTIVE_WINDOW".to_owned());
    }
    let attributes = connection.get_window_attributes(window_id).map_err(|e| e.to_string())?
        .reply().map_err(|e| format!("Window {window_id} no longer exists: {e}"))?;
    if attributes.map_state != MapState::VIEWABLE {
        return Err(format!("Window {window_id} is not viewable; focus was not requested"));
    }
    let active = atom(&connection, b"_NET_ACTIVE_WINDOW")?;
    let previous = property(&connection, root, active, AtomEnum::WINDOW)?.first().copied().unwrap_or(0);
    if previous == window_id {
        return Ok(());
    }
    // Source 2 is an external window-control tool. CurrentTime (0) is the
    // honest value because this operation is not backed by a user input event.
    let event = ClientMessageEvent::new(32, window_id, active, [2, 0, previous, 0, 0]);
    connection.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)
        .map_err(|e| e.to_string())?.check().map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if property(&connection, root, active, AtomEnum::WINDOW)?.first() == Some(&window_id) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("COMPUTER_FOCUS_NOT_CONFIRMED: the X11 window manager did not activate window {window_id}; observe again before input"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run under a real X11 login or WSLg to retain truthful native admission
    /// evidence. This probe does not synthesize input or capture screen data.
    #[test]
    #[ignore = "requires the target user's display server"]
    fn native_desktop_probe() {
        let state = desktop_state();
        println!("native Linux desktop: {state:?}");
        assert!(!matches!(state, DesktopState::Unavailable(_)));
    }
}
