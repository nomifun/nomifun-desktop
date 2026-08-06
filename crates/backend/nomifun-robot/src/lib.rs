//! Robot gateway: LAN-attached physical robots (xiaozhi firmware) acting as the
//! physical embodiment of a desktop companion.
//!
//! Byte sources are abstracted behind [`link::RobotLinkSource`] so a future
//! public relay reuses the same session core; model capabilities sit behind the
//! [`services`] trait seam so the pipeline is testable with mocks.

pub mod dto;
pub mod endpoint;
pub mod link;
pub mod protocol;
pub mod registry;
pub mod routes;

/// Domain name used in log fields and event prefixes.
pub fn robot_domain_name() -> &'static str {
    "robot"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_name_is_robot() {
        assert_eq!(robot_domain_name(), "robot");
    }
}
