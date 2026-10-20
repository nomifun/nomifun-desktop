//! Private, content-free diagnostics for this initialized native generation.

use cua_driver_sdk::CuaDriver;
use serde_json::{Value, json};

pub(crate) async fn collect(driver: &CuaDriver, embedded: bool) -> Value {
    if !embedded {
        return json!({ "backend": null, "atspi": null });
    }
    // These are the initialized native factory/dispatch families, not proof
    // of permissions, UIA availability, capture readiness or every action.
    json!({
        "backend": selected_backend(),
        "atspi": atspi_status(driver).await,
    })
}

fn selected_backend() -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    let backend = Some("win32");
    #[cfg(target_os = "macos")]
    let backend = Some("macos");
    #[cfg(target_os = "linux")]
    let backend = Some(if platform_linux::wayland::is_wayland() {
        "wayland"
    } else {
        "x11"
    });
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let backend = None;
    backend
}

#[cfg(target_os = "linux")]
async fn atspi_status(driver: &CuaDriver) -> Option<bool> {
    // This existing session-free tool performs only X11 connectivity and a
    // real D-Bus NameHasOwner(org.a11y.Bus) probe. Never retain its text or bus
    // address. Await its owned blocking probes rather than abandoning them
    // through a detached timeout; the host's existing startup/process-owner
    // budget bounds initialization and retains any required native cleanup.
    let result = driver
        .call_tool(
            "check_permissions".into(),
            json!({ "prompt": false }).to_string(),
        )
        .await
        .ok()?;
    if result.is_error {
        return None;
    }
    let structured: Value = serde_json::from_str(result.structured_json.as_deref()?).ok()?;
    structured.get("atspi").and_then(Value::as_bool)
}

#[cfg(not(target_os = "linux"))]
async fn atspi_status(_driver: &CuaDriver) -> Option<bool> {
    None
}
