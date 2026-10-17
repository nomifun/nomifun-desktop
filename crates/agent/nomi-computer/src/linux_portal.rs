//! Read-only discovery of the real Linux desktop services.
//!
//! A Wayland socket or an XWayland display does not establish global capture or
//! input capability. Query the portal frontend exposed by the user's selected
//! backend, and connect to the separate accessibility bus before reporting
//! those facilities. Discovery may D-Bus-activate services, but never creates a
//! portal session or asks for a screen/input grant.
//!
//! An available interface is an engineering prerequisite, not an active portal
//! session or an OS permission. The operation path owns the native sharing
//! dialog, active session, PipeWire stream and input notifications separately.

use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use zbus::{Connection, Proxy, zvariant::OwnedValue};

const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const CALL_TIMEOUT: Duration = Duration::from_millis(750);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

/// `Available` means the native service answered the required read-only probe.
/// It does not mean a portal grant, a readable screen stream, or a usable tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityState {
    Available,
    Missing,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalInterface {
    pub state: FacilityState,
    pub version: Option<u32>,
    /// RemoteDesktop device types or ScreenCast source types, when advertised.
    pub available_types: Option<u32>,
    pub detail: Option<String>,
}

impl PortalInterface {
    pub fn is_available(&self) -> bool {
        self.state == FacilityState::Available
    }

    fn unavailable(state: FacilityState, detail: impl Into<String>) -> Self {
        Self {
            state,
            version: None,
            available_types: None,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityBus {
    pub state: FacilityState,
    pub detail: Option<String>,
}

impl AccessibilityBus {
    pub fn is_available(&self) -> bool {
        self.state == FacilityState::Available
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopFacilities {
    pub remote_desktop: PortalInterface,
    pub screen_cast: PortalInterface,
    pub clipboard: PortalInterface,
    pub accessibility: AccessibilityBus,
}

impl DesktopFacilities {
    fn unreachable(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            remote_desktop: PortalInterface::unavailable(FacilityState::Unreachable, &detail),
            screen_cast: PortalInterface::unavailable(FacilityState::Unreachable, &detail),
            clipboard: PortalInterface::unavailable(FacilityState::Unreachable, &detail),
            accessibility: AccessibilityBus {
                state: FacilityState::Unreachable,
                detail: Some(detail),
            },
        }
    }

    /// Explain the actual missing native facilities independently of the
    /// product adapter's implementation state.
    pub fn wayland_guidance(&self) -> String {
        let mut missing = Vec::new();
        if !self.remote_desktop.is_available() {
            missing.push(format!("RemoteDesktop: {:?}", self.remote_desktop.state));
        } else if self
            .remote_desktop
            .available_types
            .is_none_or(|types| types & 3 != 3)
        {
            missing.push("RemoteDesktop does not advertise both keyboard and pointer".to_owned());
        }
        if !self.screen_cast.is_available() {
            missing.push(format!("ScreenCast: {:?}", self.screen_cast.state));
        } else if self
            .screen_cast
            .available_types
            .is_none_or(|types| types & 1 == 0)
        {
            missing.push("ScreenCast does not advertise monitor capture".to_owned());
        }
        let facilities = if missing.is_empty() {
            "The portal advertises the required native interfaces.".to_owned()
        } else {
            format!(
                "The current desktop portal cannot provide global control ({}).",
                missing.join("; ")
            )
        };
        format!(
            "{facilities} A real screenshot operation must obtain the compositor's RemoteDesktop and ScreenCast grant through its native sharing dialog. Input requires that same active screenshot session and never reauthorizes automatically. An XWayland window is not evidence of global Wayland support. AT-SPI accessibility is probed separately."
        )
    }
}

/// Discover services through the login session's D-Bus connection. The total
/// operation is bounded even if activation or one desktop service stalls.
pub async fn probe() -> DesktopFacilities {
    match tokio::time::timeout(DISCOVERY_TIMEOUT, probe_inner()).await {
        Ok(result) => result,
        Err(_) => DesktopFacilities::unreachable("Linux desktop service discovery timed out"),
    }
}

/// Synchronous host projections can run inside Tokio. A separate thread and
/// runtime avoid nesting `block_on` in that caller's runtime. No service or
/// connection survives this bounded discovery call.
pub fn probe_blocking() -> DesktopFacilities {
    let worker = std::thread::Builder::new()
        .name("nomifun-desktop-probe".to_owned())
        .spawn(|| {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    return DesktopFacilities::unreachable(format!(
                        "Cannot start desktop discovery: {error}"
                    ));
                }
            };
            runtime.block_on(probe())
        });
    match worker {
        Ok(worker) => worker.join().unwrap_or_else(|_| {
            DesktopFacilities::unreachable("Linux desktop discovery worker failed")
        }),
        Err(error) => {
            DesktopFacilities::unreachable(format!("Cannot spawn desktop discovery: {error}"))
        }
    }
}

async fn probe_inner() -> DesktopFacilities {
    let connection = match zbus::connection::Builder::session() {
        Ok(builder) => match builder.method_timeout(CALL_TIMEOUT).build().await {
            Ok(connection) => connection,
            Err(error) => {
                return DesktopFacilities::unreachable(format!(
                    "Cannot connect to the login session D-Bus: {error}"
                ));
            }
        },
        Err(error) => {
            return DesktopFacilities::unreachable(format!(
                "No login session D-Bus address: {error}"
            ));
        }
    };
    // GLib's Properties.GetAll reports absent interfaces as InvalidArgs.
    // Query the standard interface inventory instead of parsing error prose.
    let exported = exported_interfaces(&connection).await;
    let (remote_desktop, screen_cast, clipboard, accessibility) = tokio::join!(
        portal_interface(
            &connection,
            "org.freedesktop.portal.RemoteDesktop",
            Some("AvailableDeviceTypes"),
            exported.as_ref(),
        ),
        portal_interface(
            &connection,
            "org.freedesktop.portal.ScreenCast",
            Some("AvailableSourceTypes"),
            exported.as_ref(),
        ),
        portal_interface(
            &connection,
            "org.freedesktop.portal.Clipboard",
            None,
            exported.as_ref()
        ),
        accessibility_bus(&connection),
    );
    DesktopFacilities {
        remote_desktop,
        screen_cast,
        clipboard,
        accessibility,
    }
}

async fn exported_interfaces(connection: &Connection) -> Option<BTreeSet<String>> {
    let proxy = Proxy::new(
        connection,
        PORTAL_DESTINATION,
        PORTAL_PATH,
        "org.freedesktop.DBus.Introspectable",
    )
    .await
    .ok()?;
    let xml: String = proxy.call("Introspect", &()).await.ok()?;
    let node = zbus_xml::Node::from_reader(xml.as_bytes()).ok()?;
    Some(
        node.interfaces()
            .iter()
            .map(|interface| interface.name().as_str().to_owned())
            .collect(),
    )
}

async fn portal_interface(
    connection: &Connection,
    interface: &str,
    types_property: Option<&str>,
    exported: Option<&BTreeSet<String>>,
) -> PortalInterface {
    if exported.is_some_and(|interfaces| !interfaces.contains(interface)) {
        return PortalInterface::unavailable(
            FacilityState::Missing,
            format!("The current desktop portal does not export {interface}"),
        );
    }
    let properties = match Proxy::new(
        connection,
        PORTAL_DESTINATION,
        PORTAL_PATH,
        "org.freedesktop.DBus.Properties",
    )
    .await
    {
        Ok(proxy) => proxy,
        Err(error) => return portal_error(error),
    };
    let result: Result<HashMap<String, OwnedValue>, _> =
        properties.call("GetAll", &(interface,)).await;
    match result {
        Ok(properties) => decode_interface(&properties, interface, types_property),
        Err(error) => portal_error(error),
    }
}

fn decode_interface(
    properties: &HashMap<String, OwnedValue>,
    interface: &str,
    types_property: Option<&str>,
) -> PortalInterface {
    let version = properties
        .get("version")
        .and_then(|value| u32::try_from(value).ok());
    // GetAll may return an empty map for an interface not exported by this
    // backend. Empty or malformed version data must never manufacture support.
    let Some(version) = version.filter(|version| *version > 0) else {
        return PortalInterface::unavailable(
            FacilityState::Missing,
            format!("The current portal backend does not advertise {interface}"),
        );
    };
    let available_types = types_property
        .and_then(|name| properties.get(name))
        .and_then(|value| u32::try_from(value).ok());
    if types_property.is_some() && available_types.is_none() {
        return PortalInterface::unavailable(
            FacilityState::Unreachable,
            format!("{interface} returned no valid available types property"),
        );
    }
    PortalInterface {
        state: FacilityState::Available,
        version: Some(version),
        available_types,
        detail: None,
    }
}

fn error_state(error: &zbus::Error) -> FacilityState {
    match error {
        zbus::Error::MethodError(name, _, _)
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
                    | "org.freedesktop.DBus.Error.UnknownInterface"
                    | "org.freedesktop.DBus.Error.UnknownObject"
                    | "org.freedesktop.DBus.Error.UnknownMethod"
            ) =>
        {
            FacilityState::Missing
        }
        _ => FacilityState::Unreachable,
    }
}

fn portal_error(error: zbus::Error) -> PortalInterface {
    PortalInterface::unavailable(error_state(&error), error.to_string())
}

async fn accessibility_bus(connection: &Connection) -> AccessibilityBus {
    let result = async {
        let proxy = Proxy::new(connection, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus").await?;
        let address: String = proxy.call("GetAddress", &()).await?;
        let bus = zbus::connection::Builder::address(address.as_str())?
            .method_timeout(CALL_TIMEOUT)
            .build()
            .await?;
        // GetAddress alone does not prove the AT-SPI bus or registry works.
        // Ping the registry on that separate bus, without enumerating or
        // changing any application tree or accessibility setting.
        let registry = Proxy::new(
            &bus,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/registry",
            "org.freedesktop.DBus.Peer",
        )
        .await?;
        registry.call::<_, _, ()>("Ping", &()).await
    }
    .await;
    match result {
        Ok(()) => AccessibilityBus {
            state: FacilityState::Available,
            detail: None,
        },
        Err(error) => AccessibilityBus {
            state: error_state(&error),
            detail: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_portal_interfaces_are_not_desktop_permission_denials() {
        let interface = decode_interface(
            &HashMap::new(),
            "org.freedesktop.portal.RemoteDesktop",
            Some("AvailableDeviceTypes"),
        );
        assert_eq!(interface.state, FacilityState::Missing);
        assert!(!interface.is_available());
        assert_eq!(interface.version, None);
    }

    #[test]
    fn portal_version_requires_valid_native_device_metadata() {
        let mut properties = HashMap::new();
        properties.insert("version".to_owned(), OwnedValue::from(2_u32));
        assert_eq!(
            decode_interface(&properties, "RemoteDesktop", Some("AvailableDeviceTypes")).state,
            FacilityState::Unreachable
        );
        properties.insert("AvailableDeviceTypes".to_owned(), OwnedValue::from(3_u32));
        let interface =
            decode_interface(&properties, "RemoteDesktop", Some("AvailableDeviceTypes"));
        assert!(interface.is_available());
        assert_eq!(interface.version, Some(2));
        assert_eq!(interface.available_types, Some(3));
    }

    #[test]
    fn interfaces_do_not_claim_a_wayland_session_or_permission() {
        let mut facilities = DesktopFacilities::unreachable("test");
        facilities.remote_desktop = PortalInterface {
            state: FacilityState::Available,
            version: Some(2),
            available_types: Some(3),
            detail: None,
        };
        facilities.screen_cast = PortalInterface {
            state: FacilityState::Available,
            version: Some(5),
            available_types: Some(1),
            detail: None,
        };
        assert!(
            facilities
                .wayland_guidance()
                .contains("native sharing dialog")
        );
        facilities.remote_desktop.available_types = Some(1);
        assert!(
            facilities
                .wayland_guidance()
                .contains("both keyboard and pointer")
        );
    }

    /// Run in the actual user's desktop session. This emits observed native
    /// facility state; it intentionally does not assume a compositor backend.
    #[test]
    #[ignore = "requires a login desktop D-Bus/AT-SPI session"]
    fn native_desktop_facilities() {
        let facilities = probe_blocking();
        println!("{}", serde_json::to_string_pretty(&facilities).unwrap());
    }
}
