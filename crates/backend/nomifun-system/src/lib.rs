//! System services: provider management, model fetching, settings, and version checks.
pub mod bedrock_probe;
pub mod client_pref;
pub mod model_fetcher;
pub mod provider;
pub mod provider_connection;
pub mod provider_deletion;
pub mod provider_model;
pub mod routes;
pub mod settings;
pub mod sysinfo;
pub mod version;

pub use bedrock_probe::{ConnectionTestRouterState, ConnectionTestService, connection_test_routes};
pub use client_pref::ClientPrefService;
pub use model_fetcher::ModelFetchService;
pub use provider::{ProviderService, disable_retired_provider_platforms};
pub use provider_connection::ProviderConnectionService;
pub use provider_deletion::{ProviderDeletionCoordinator, SharedProviderDeletionCoordinator};
pub use provider_model::ProviderModelService;
pub use routes::{SystemRouterState, system_routes};
pub use settings::SettingsService;
pub use version::VersionCheckService;
