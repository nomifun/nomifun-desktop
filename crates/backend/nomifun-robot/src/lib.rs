//! Robot gateway: LAN-attached physical robots (xiaozhi firmware) acting as the
//! physical embodiment of a desktop companion.
//!
//! Byte sources are abstracted behind [`link::RobotLinkSource`] so a future
//! public relay reuses the same session core; model capabilities sit behind the
//! [`services`] trait seam so the pipeline is testable with mocks.

pub mod audio;
pub mod dto;
pub mod endpoint;
pub mod events;
pub mod lan_source;
pub mod link;
pub mod pipeline;
pub mod protocol;
pub mod registry;
pub mod routes;
pub mod session;
pub mod status;
pub mod vad;

use std::sync::Arc;

/// Domain name used in log fields and event prefixes.
pub fn robot_domain_name() -> &'static str {
    "robot"
}

/// Owns the accept loop: every [`link::RobotLinkSource`] feeds one channel, and
/// each accepted link becomes a detached [`session::run_session`] task.
pub struct RobotGateway {
    deps: session::SessionDeps,
}

impl RobotGateway {
    pub fn new(deps: session::SessionDeps) -> Self {
        Self { deps }
    }

    /// Run until every source has finished.
    pub async fn serve(self: Arc<Self>, sources: Vec<Arc<dyn link::RobotLinkSource>>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<link::AcceptedLink>(8);
        for source in sources {
            let tx = tx.clone();
            let name = source.name();
            tokio::spawn(async move {
                if let Err(error) = source.run(tx).await {
                    tracing::error!(source = name, %error, "robot: link source stopped");
                }
            });
        }
        drop(tx);
        while let Some(link) = rx.recv().await {
            let deps = self.deps.clone();
            tokio::spawn(session::run_session(link, deps));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_name_is_robot() {
        assert_eq!(robot_domain_name(), "robot");
    }
}
