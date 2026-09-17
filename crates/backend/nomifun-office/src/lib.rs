//! Office document preview, proxy, and snapshot management.
pub mod agent;
pub mod error;
pub mod port;
pub mod proxy;
pub mod routes;
pub mod snapshot;
pub mod state;
pub mod types;
pub mod watch_manager;

pub use error::OfficeError;
pub use agent::{
    OfficeAgentError, OfficeAssetFormat, OfficeDocumentEditRequest, OfficePreview,
    OfficePreviewRequest, OfficeRevisionDraft, OfficeSheetEditRequest, OfficeSlideInput,
    OfficeSlidesEditRequest, bounded_preview, build_document_revision, build_sheet_revision,
    build_slides_revision,
};
pub use proxy::{ProxyError, ProxyService};
pub use routes::{office_proxy_routes, office_routes};
pub use snapshot::SnapshotService;
pub use state::OfficeRouterState;
pub use types::DocType;
pub use watch_manager::{
    DefaultProcessSpawner, OfficecliWatchManager, PreviewAccess, PreviewProxyTarget, ProcessHandle, ProcessSpawner,
};
