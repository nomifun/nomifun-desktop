//! Managed Office artifact preview, product actions and snapshot history.
pub mod agent;
pub mod artifact;
pub mod error;
pub mod routes;
pub mod snapshot;
pub mod state;
pub mod types;

pub use agent::{
    OfficeAgentError, OfficeAssetFormat, OfficeDocumentEditRequest, OfficePreview,
    OfficePreviewRequest, OfficeRevisionDraft, OfficeSheetEditRequest, OfficeSlideInput,
    OfficeSlidesEditRequest, bounded_preview, build_document_revision, build_sheet_revision,
    build_slides_revision,
};
pub use artifact::{
    ManagedOfficeEngineSupplier, OfficeEngine, OfficeEngineSupplier, OfficePreviewService,
    PreviewLimits,
};
pub use error::OfficeError;
pub use routes::{office_artifact_routes, office_routes};
pub use snapshot::SnapshotService;
pub use state::OfficeRouterState;
pub use types::DocType;
