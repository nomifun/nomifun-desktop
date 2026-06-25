//! Backend-served static logo assets.
pub mod routes;
pub mod service;
pub mod state;

pub use routes::asset_routes;
pub use service::AssetService;
pub use state::AssetRouterState;
