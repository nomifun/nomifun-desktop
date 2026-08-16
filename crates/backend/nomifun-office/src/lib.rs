//! Office document preview, proxy, and snapshot management.
pub mod error;
pub mod port;
pub mod proxy;
pub mod routes;
pub mod snapshot;
pub mod state;
pub mod types;
pub mod watch_manager;

pub use error::OfficeError;
pub use proxy::{ProxyError, ProxyService};
pub use routes::{office_proxy_routes, office_routes};
pub use snapshot::SnapshotService;
pub use state::OfficeRouterState;
pub use types::DocType;
pub use watch_manager::{
    DefaultProcessSpawner, OfficecliWatchManager, PreviewAccess, PreviewProxyTarget, ProcessHandle, ProcessSpawner,
};
