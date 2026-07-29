//! [`ModelInvokeService`] — the invoke layer's entry point: catalog
//! repositories + credential decryption key + shared HTTP client + the
//! protocol adapter registry. This module carries the constructor; the
//! catalog resolution pipeline lives in [`crate::resolve`], and the
//! invoke/poll/probe orchestration arrives in a later task (T5).

use std::sync::Arc;

use crate::adapter::AdapterRegistry;

/// The unified multimodal model invocation service.
pub struct ModelInvokeService {
    pub(crate) provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
    pub(crate) provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
    pub(crate) provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository>,
    /// AES-256-GCM key used to decrypt stored provider/connection credentials.
    pub(crate) encryption_key: [u8; 32],
    /// Shared client for all adapter calls (consumed by the T5 orchestration).
    #[allow(dead_code)]
    pub(crate) http: reqwest::Client,
    pub(crate) registry: AdapterRegistry,
}

impl ModelInvokeService {
    pub fn new(
        provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
        provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
        provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository>,
        encryption_key: [u8; 32],
        http: reqwest::Client,
        registry: AdapterRegistry,
    ) -> Self {
        Self { provider_repo, provider_model_repo, provider_connection_repo, encryption_key, http, registry }
    }
}
