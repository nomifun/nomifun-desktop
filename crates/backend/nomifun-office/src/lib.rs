//! Office document preview, format conversion, proxy, and snapshot management.
pub mod conversion;
pub mod error;
pub mod port;
pub mod proxy;
pub mod routes;
pub mod snapshot;
pub mod star_office;
pub mod state;
pub mod types;
pub mod watch_manager;

pub use conversion::ConversionService;
pub use error::OfficeError;
pub use proxy::{ProxyError, ProxyService};
pub use routes::{office_proxy_routes, office_routes};
pub use snapshot::SnapshotService;
pub use star_office::StarOfficeDetector;
pub use state::OfficeRouterState;
pub use types::{DocType, OfficecliStatus};
pub use watch_manager::{DefaultProcessSpawner, OfficecliWatchManager, ProcessHandle, ProcessSpawner};
