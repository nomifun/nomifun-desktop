//! What address to tell a device to connect to.
//!
//! The OTA response is the **only** channel that configures the firmware's
//! server address, and the vision URL rides MCP `initialize`, so both come from
//! one place. Today the only implementation is LAN; a future relay implements
//! the same trait and returns its public wss/https base, leaving the OTA handler
//! untouched.

use std::net::{IpAddr, Ipv4Addr};

use tokio::sync::watch;

/// WebSocket path devices connect to.
pub const WS_PATH: &str = "/robot/v1";
/// OTA report path (the one address a user types into the firmware).
pub const OTA_PATH: &str = "/robot/ota";
/// Vision explain path (delivered via MCP `initialize`, not OTA).
pub const VISION_PATH: &str = "/robot/vision/explain";

/// Live view of the desktop LAN listener. Fed by `nomifun-app` from its
/// `DesktopServer` status watch; this crate never depends on `nomifun-app`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanEndpointSnapshot {
    pub enabled: bool,
    pub port: u16,
    pub ipv4s: Vec<Ipv4Addr>,
}

/// Resolves the addresses a device should use.
pub trait EndpointAdvertiser: Send + Sync {
    /// `ws://…/robot/v1` for a device reporting from `peer`, or `None` when the
    /// transport is unavailable.
    fn websocket_url(&self, peer: IpAddr) -> Option<String>;
    /// Scheme+host+port for HTTP endpoints (vision), same origin family as
    /// [`websocket_url`](Self::websocket_url).
    fn http_base(&self, peer: IpAddr) -> Option<String>;
    /// Every OTA URL worth showing in the UI (multi-homed hosts get several).
    fn ota_urls(&self) -> Vec<String>;
    /// Whether a device could reach us at all right now.
    fn is_available(&self) -> bool;
}

/// LAN advertiser: picks the local interface that shares the longest prefix
/// with the reporting peer, so a multi-homed host (VPN + Wi-Fi + docker bridge)
/// hands the robot an address on the robot's own segment.
pub struct LanAdvertiser {
    status: watch::Receiver<LanEndpointSnapshot>,
}

impl LanAdvertiser {
    pub fn new(status: watch::Receiver<LanEndpointSnapshot>) -> Self {
        Self { status }
    }

    fn snapshot(&self) -> LanEndpointSnapshot {
        self.status.borrow().clone()
    }

    /// Interface with the most leading octets in common with `peer`; falls back
    /// to the first detected interface.
    fn best_host(&self, snap: &LanEndpointSnapshot, peer: IpAddr) -> Option<Ipv4Addr> {
        let peer_octets = match peer {
            IpAddr::V4(v4) => Some(v4.octets()),
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(|v4| v4.octets()),
        };
        let Some(peer_octets) = peer_octets else {
            return snap.ipv4s.first().copied();
        };
        snap.ipv4s
            .iter()
            .copied()
            .max_by_key(|candidate| {
                candidate
                    .octets()
                    .iter()
                    .zip(peer_octets.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .or_else(|| snap.ipv4s.first().copied())
    }

    fn authority(&self, peer: IpAddr) -> Option<String> {
        let snap = self.snapshot();
        if !snap.enabled || snap.port == 0 {
            return None;
        }
        let host = self.best_host(&snap, peer)?;
        Some(format!("{host}:{}", snap.port))
    }
}

impl EndpointAdvertiser for LanAdvertiser {
    fn websocket_url(&self, peer: IpAddr) -> Option<String> {
        self.authority(peer).map(|a| format!("ws://{a}{WS_PATH}"))
    }

    fn http_base(&self, peer: IpAddr) -> Option<String> {
        self.authority(peer).map(|a| format!("http://{a}"))
    }

    fn ota_urls(&self) -> Vec<String> {
        let snap = self.snapshot();
        if !snap.enabled || snap.port == 0 {
            return Vec::new();
        }
        snap.ipv4s
            .iter()
            .map(|ip| format!("http://{ip}:{}{OTA_PATH}", snap.port))
            .collect()
    }

    fn is_available(&self) -> bool {
        let snap = self.snapshot();
        snap.enabled && snap.port != 0 && !snap.ipv4s.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn snapshot(enabled: bool, port: u16, ips: &[[u8; 4]]) -> LanEndpointSnapshot {
        LanEndpointSnapshot {
            enabled,
            port,
            ipv4s: ips
                .iter()
                .map(|o| Ipv4Addr::new(o[0], o[1], o[2], o[3]))
                .collect(),
        }
    }

    #[test]
    fn picks_the_interface_sharing_the_peer_prefix() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(
            true,
            25808,
            &[[10, 0, 0, 5], [192, 168, 1, 20]],
        ));
        let adv = LanAdvertiser::new(rx);

        let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77));
        assert_eq!(
            adv.websocket_url(peer).as_deref(),
            Some("ws://192.168.1.20:25808/robot/v1")
        );
        assert_eq!(
            adv.http_base(peer).as_deref(),
            Some("http://192.168.1.20:25808")
        );
    }

    #[test]
    fn falls_back_to_the_first_interface_when_no_prefix_matches() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(true, 25809, &[[10, 0, 0, 5]]));
        let adv = LanAdvertiser::new(rx);
        let peer = IpAddr::V4(Ipv4Addr::new(172, 20, 3, 4));
        assert_eq!(
            adv.websocket_url(peer).as_deref(),
            Some("ws://10.0.0.5:25809/robot/v1")
        );
    }

    #[test]
    fn unavailable_when_lan_listener_is_off() {
        let (_tx, rx) =
            tokio::sync::watch::channel(snapshot(false, 25808, &[[192, 168, 1, 20]]));
        let adv = LanAdvertiser::new(rx);
        assert!(!adv.is_available());
        assert!(
            adv.websocket_url(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77)))
                .is_none()
        );
        assert!(adv.ota_urls().is_empty());
    }

    #[test]
    fn unavailable_when_no_interface_was_detected() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(true, 25808, &[]));
        let adv = LanAdvertiser::new(rx);
        assert!(!adv.is_available());
    }

    #[test]
    fn ota_urls_list_every_candidate_interface() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(
            true,
            25808,
            &[[192, 168, 1, 20], [10, 8, 0, 2]],
        ));
        let adv = LanAdvertiser::new(rx);
        assert_eq!(
            adv.ota_urls(),
            vec![
                "http://192.168.1.20:25808/robot/ota".to_owned(),
                "http://10.8.0.2:25808/robot/ota".to_owned(),
            ]
        );
    }

    #[test]
    fn reflects_live_snapshot_changes() {
        let (tx, rx) = tokio::sync::watch::channel(snapshot(false, 0, &[]));
        let adv = LanAdvertiser::new(rx);
        assert!(!adv.is_available());
        tx.send(snapshot(true, 25808, &[[192, 168, 1, 20]])).unwrap();
        assert!(adv.is_available());
    }
}
