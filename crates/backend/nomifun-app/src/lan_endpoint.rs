//! Which address of this host a LAN client should dial — and, for the robot
//! gateway, publishing that answer to the endpoint advertiser.
//!
//! Two callers, one filter. The desktop's remote-browser LAN listener and the
//! robot advertiser both need "the IPv4 addresses of this host that a device on
//! the user's network could actually reach", so the NIC scan and its
//! special-purpose-range filter live here instead of inside `desktop.rs`, where
//! only a desktop host could reach them.
//!
//! The robot half exists because the advertiser is the ONLY thing that tells a
//! device where to connect (via its OTA response), and until now only the
//! desktop host ever wrote to it. A headless / Docker `nomifun-web` already
//! serves `/robot/*` and `/api/robots*` from the shared router — it just never
//! said where it lives, so every OTA response carried an empty websocket URL
//! and `GET /api/robots/endpoints` returned no OTA URLs at all.
//!
//! Docker is why the explicit override exists. Inside a container the NIC scan
//! yields the container's own private address (`172.17.0.2`), which no robot on
//! the user's LAN can route to, and the published port may differ from the bound
//! one (`-p 9000:8787` advertises 9000 while the process binds 8787). Detection
//! is wrong there in principle, not by accident, so the operator states the
//! answer with [`ROBOT_ADVERTISE_ENV`] and a malformed value fails the boot
//! rather than silently falling back to a detection that cannot work.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Result, anyhow};
use nomifun_robot::endpoint::LanEndpointSnapshot;
use tokio::sync::watch;

use crate::AppServices;

/// Address robots must dial to reach this installation: `<ipv4>` or
/// `<ipv4>:<port>`.
pub const ROBOT_ADVERTISE_ENV: &str = "NOMIFUN_ROBOT_ADVERTISE";

/// An operator-stated robot endpoint: where devices reach this host from the
/// LAN, which is not necessarily where this process listens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RobotAdvertiseAddr {
    /// The address a device dials. IPv4 only: a device is handed one bare
    /// `ws://<ipv4>:<port>` URL and speaks plain HTTP/WS to it.
    pub ip: Ipv4Addr,
    /// The port a device dials, when it differs from the bound one (a remapped
    /// container port). `None` means "whatever port this process bound".
    pub port: Option<u16>,
}

/// Read [`ROBOT_ADVERTISE_ENV`], if set.
///
/// Call this **before** the host does any real startup work: an unusable value
/// must stop the boot with a legible message, not surface hours later as "the
/// robot won't connect".
pub fn robot_advertise_from_env() -> Result<Option<RobotAdvertiseAddr>> {
    match std::env::var(ROBOT_ADVERTISE_ENV) {
        Ok(raw) => parse_robot_advertise(&raw),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(anyhow!(
            "{ROBOT_ADVERTISE_ENV} is not valid Unicode; expected an IPv4 literal \
             like `192.168.1.50` or `192.168.1.50:9000`"
        )),
    }
}

/// Parse an advertise value. `Ok(None)` means "not configured, detect instead" —
/// an empty value counts as unset, because a Compose file that passes the name
/// through without a value must not fail the boot.
pub fn parse_robot_advertise(raw: &str) -> Result<Option<RobotAdvertiseAddr>> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }

    let malformed = || {
        anyhow!(
            "{ROBOT_ADVERTISE_ENV}={raw:?} is not an address a robot can dial: expected an IPv4 \
             literal like `192.168.1.50`, or `192.168.1.50:9000` when the port devices reach \
             differs from the bound one. Host names and IPv6 are not supported — the OTA response \
             hands a device exactly one plain `ws://<ipv4>:<port>` URL."
        )
    };

    let (host, port) = match value.rsplit_once(':') {
        Some((host, port)) => (host.trim(), Some(port.trim())),
        None => (value, None),
    };
    let ip: Ipv4Addr = host.parse().map_err(|_| malformed())?;
    let port = match port {
        // Port 0 is "let the OS choose" for a bind and meaningless as a
        // destination, so it is rejected with everything else out of range.
        Some(port) => Some(
            port.parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    anyhow!(
                        "{ROBOT_ADVERTISE_ENV}={raw:?} has an invalid port: expected 1-65535 \
                         (the port a robot connects to, e.g. the host side of `-p 9000:8787`)"
                    )
                })?,
        ),
        None => None,
    };

    if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return Err(anyhow!(
            "{ROBOT_ADVERTISE_ENV}={raw:?} is a bind/group address, not a destination: use the \
             IPv4 address robots see this host at on the LAN (inside Docker that is the *host \
             machine's* LAN address, never the container's)"
        ));
    }

    Ok(Some(RobotAdvertiseAddr { ip, port }))
}

/// Tell the robot endpoint advertiser where devices can reach this host.
///
/// A no-op when the robot gateway is unavailable this boot (`services.robot` is
/// `None`, e.g. its registry failed to load): the host serves everything else
/// exactly as before.
///
/// One shot, not a subscription: unlike the desktop's toggleable LAN listener,
/// a server host's listener is up for the whole process lifetime, so there is
/// nothing to follow. Call it once the real port is known.
pub fn publish_robot_endpoint(
    services: &AppServices,
    bound_port: u16,
    advertise: Option<RobotAdvertiseAddr>,
) {
    publish_to(
        services.robot.as_ref().map(|robot| &robot.endpoint_tx),
        bound_port,
        advertise,
    );
}

/// The publish seam, split out so both branches are testable without an
/// `AppServices`. Returns what was published, or `None` when there was no
/// advertiser to publish to.
fn publish_to(
    endpoint_tx: Option<&watch::Sender<LanEndpointSnapshot>>,
    bound_port: u16,
    advertise: Option<RobotAdvertiseAddr>,
) -> Option<LanEndpointSnapshot> {
    let endpoint_tx = endpoint_tx?;
    let snapshot = robot_endpoint_snapshot(bound_port, advertise);
    if snapshot.ipv4s.is_empty() {
        tracing::warn!(
            port = snapshot.port,
            "robot: no LAN-reachable IPv4 detected, so devices cannot be told where to connect; \
             set {ROBOT_ADVERTISE_ENV}=<ipv4>[:<port>] to the address robots reach this host at \
             (required under Docker, where the detected address is the container's)"
        );
    } else {
        tracing::info!(
            port = snapshot.port,
            explicit = advertise.is_some(),
            interfaces = snapshot.ipv4s.len(),
            addresses = %snapshot
                .ipv4s
                .iter()
                .map(|ip| ip.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            "robot: advertising this endpoint to devices"
        );
    }
    endpoint_tx.send_replace(snapshot.clone());
    Some(snapshot)
}

/// What a server host advertises: the stated address when configured, the
/// detected interfaces otherwise.
///
/// `enabled` is unconditionally true because a server host's listener is always
/// serving; reachability is carried by `ipv4s` (the advertiser reports itself
/// unavailable while that list is empty).
fn robot_endpoint_snapshot(
    bound_port: u16,
    advertise: Option<RobotAdvertiseAddr>,
) -> LanEndpointSnapshot {
    match advertise {
        Some(addr) => LanEndpointSnapshot {
            enabled: true,
            port: addr.port.unwrap_or(bound_port),
            ipv4s: vec![addr.ip],
        },
        None => LanEndpointSnapshot {
            enabled: true,
            port: bound_port,
            ipv4s: detect_all_lan_ipv4s(),
        },
    }
}

/// Routing-preferred source IPv4 (the address used to reach off-box hosts).
/// Connecting a UDP socket only sets its local address; no packets are sent.
fn routing_primary_ipv4() -> Option<Ipv4Addr> {
    let sock = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    match sock.local_addr().ok()? {
        SocketAddr::V4(a) => {
            let ip = *a.ip();
            is_lan_ip_candidate(ip).then_some(ip)
        }
        _ => None,
    }
}

/// Whether an interface address is worth offering to a remote browser or a
/// robot on the user's network.
fn is_lan_ip_candidate(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    if ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() || ip.is_multicast() {
        return false;
    }
    if octets == [255, 255, 255, 255] {
        return false;
    }
    // RFC 2544 benchmarking addresses are commonly created by virtual network
    // adapters and are not reachable from a phone on the user's LAN.
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return false;
    }
    // Documentation-only ranges should never be offered as a real WebUI target.
    if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
    {
        return false;
    }
    true
}

/// All LAN-usable IPv4 NIC addresses — routing-preferred first, then the rest
/// (private ranges before public). A multi-homed / VPN host still yields
/// several; obvious virtual/special-purpose addresses are filtered out.
pub fn detect_all_lan_ipv4s() -> Vec<Ipv4Addr> {
    let mut addrs: Vec<Ipv4Addr> = Vec::new();
    if let Some(primary) = routing_primary_ipv4() {
        addrs.push(primary);
    }
    let mut rest: Vec<Ipv4Addr> = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            if let IpAddr::V4(v4) = iface.ip()
                && is_lan_ip_candidate(v4)
                && !addrs.contains(&v4)
                && !rest.contains(&v4)
            {
                rest.push(v4);
            }
        }
    }
    rest.sort_by_key(|ip| !ip.is_private()); // private (false→0) first
    addrs.extend(rest);
    addrs
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_robot::endpoint::{EndpointAdvertiser, LanAdvertiser};

    fn parsed(raw: &str) -> RobotAdvertiseAddr {
        parse_robot_advertise(raw)
            .unwrap_or_else(|error| panic!("{raw:?} should parse: {error}"))
            .unwrap_or_else(|| panic!("{raw:?} should not read as unset"))
    }

    #[test]
    fn a_bare_address_keeps_the_bound_port() {
        assert_eq!(
            parsed("192.168.1.50"),
            RobotAdvertiseAddr {
                ip: Ipv4Addr::new(192, 168, 1, 50),
                port: None,
            }
        );
        let snapshot = robot_endpoint_snapshot(8787, Some(parsed(" 192.168.1.50 ")));
        assert_eq!(snapshot.port, 8787);
        assert_eq!(snapshot.ipv4s, vec![Ipv4Addr::new(192, 168, 1, 50)]);
        assert!(snapshot.enabled);
    }

    #[test]
    fn an_explicit_port_wins_over_the_bound_one() {
        // `-p 9000:8787`: the robot dials 9000, this process binds 8787.
        assert_eq!(parsed("192.168.1.50:9000").port, Some(9000));
        assert_eq!(robot_endpoint_snapshot(8787, Some(parsed("192.168.1.50:9000"))).port, 9000);
    }

    #[test]
    fn a_stated_address_beats_detection() {
        let stated = robot_endpoint_snapshot(8787, Some(parsed("192.168.1.50")));
        assert_eq!(
            stated.ipv4s,
            vec![Ipv4Addr::new(192, 168, 1, 50)],
            "an operator-stated address must be advertised verbatim, whatever this host's NICs say"
        );
    }

    #[test]
    fn an_unset_value_falls_back_to_detection() {
        assert_eq!(parse_robot_advertise("").unwrap(), None);
        assert_eq!(parse_robot_advertise("   ").unwrap(), None);
        let detected = robot_endpoint_snapshot(8787, None);
        assert_eq!(detected.port, 8787);
        assert_eq!(detected.ipv4s, detect_all_lan_ipv4s());
    }

    #[test]
    fn an_unusable_value_is_rejected_instead_of_silently_detected() {
        for raw in [
            "nomi.example.com",   // host names would need TLS + a relay
            "nomi.local:8787",    // …with or without a port
            "not-an-address",     //
            "::1",                // IPv6 is not what the firmware is handed
            "[fe80::1]:8787",     //
            "192.168.1.50:0",     // port 0 is a bind trick, not a destination
            "192.168.1.50:99999", // out of range
            "192.168.1.50:http",  // named ports are not resolved
            "192.168.1.50:",      // truncated
            "0.0.0.0",            // a bind address, the classic copy-paste
            "0.0.0.0:8787",       //
            "255.255.255.255",    // broadcast
            "239.1.1.1",          // multicast
        ] {
            let error = match parse_robot_advertise(raw) {
                Err(error) => error,
                Ok(accepted) => {
                    panic!("{raw:?} must stop the boot, not be accepted as {accepted:?}")
                }
            };
            let text = format!("{error}");
            assert!(
                text.contains(ROBOT_ADVERTISE_ENV),
                "the error for {raw:?} must name the variable to fix: {text}"
            );
        }
    }

    #[test]
    fn publishing_without_a_robot_gateway_is_a_no_op() {
        assert_eq!(
            publish_to(None, 8787, Some(parsed("192.168.1.50"))),
            None,
            "a host whose robot gateway failed to load must still boot"
        );
    }

    #[test]
    fn a_published_endpoint_reaches_the_advertiser() {
        let (tx, rx) = watch::channel(LanEndpointSnapshot::default());
        let advertiser = LanAdvertiser::new(rx);
        assert!(!advertiser.is_available(), "nothing published yet");

        publish_to(Some(&tx), 8787, Some(parsed("192.168.1.50:9000")))
            .expect("publishing to a live advertiser reports what it published");

        assert!(advertiser.is_available());
        assert_eq!(
            advertiser.ota_urls(),
            vec!["http://192.168.1.50:9000/robot/ota".to_owned()]
        );
        assert_eq!(
            advertiser.websocket_url(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77))),
            Some("ws://192.168.1.50:9000/robot/v1".to_owned())
        );
    }

    #[test]
    fn lan_ip_candidate_filters_special_purpose_ranges() {
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(0, 0, 0, 0)));
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(169, 254, 10, 20)));
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(198, 18, 0, 1)));
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(198, 19, 255, 1)));
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(224, 0, 0, 1)));
        assert!(!is_lan_ip_candidate(Ipv4Addr::new(255, 255, 255, 255)));
    }

    #[test]
    fn lan_ip_candidate_keeps_private_lan_ranges() {
        assert!(is_lan_ip_candidate(Ipv4Addr::new(10, 8, 0, 2)));
        assert!(is_lan_ip_candidate(Ipv4Addr::new(172, 16, 1, 20)));
        assert!(is_lan_ip_candidate(Ipv4Addr::new(192, 168, 31, 5)));
    }
}
