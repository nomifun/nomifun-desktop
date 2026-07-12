use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomifun_common::AppError;
use nomifun_creation::{
    CreationError, LocalImageBackend, LocalImageRequest, ProducedAsset,
};
use nomifun_db::{IModelProfileRepository, IProviderRepository};
use tokio::sync::{Mutex, OnceCell};

use crate::{
    ImageModelService, LocalModelServer, LocalModelService, reconcile_local_catalog_profiles,
    start_and_provision_local_model,
};

struct LocalModelServices {
    local: Arc<LocalModelService>,
    image: Arc<ImageModelService>,
    _server: LocalModelServer,
}

/// Lazily owns all local-model control planes and the loopback OpenAI facade.
///
/// A fresh installation keeps this cell empty: no local-model directories,
/// HTTP listener, downloader clients, or model services are created at boot.
/// The first explicit install/enable/resume mutation initializes the bundle.
pub struct LazyLocalModelRuntime {
    data_dir: PathBuf,
    provider_repo: Arc<dyn IProviderRepository>,
    model_profile_repo: Arc<dyn IModelProfileRepository>,
    encryption_key: [u8; 32],
    services: OnceCell<Arc<LocalModelServices>>,
    init_lock: Mutex<()>,
}

impl LazyLocalModelRuntime {
    pub fn new(
        data_dir: impl AsRef<Path>,
        provider_repo: Arc<dyn IProviderRepository>,
        model_profile_repo: Arc<dyn IModelProfileRepository>,
        encryption_key: [u8; 32],
    ) -> Arc<Self> {
        Arc::new(Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            provider_repo,
            model_profile_repo,
            encryption_key,
            services: OnceCell::new(),
            init_lock: Mutex::new(()),
        })
    }

    pub fn is_started(&self) -> bool {
        self.services.get().is_some()
    }

    /// Start a previously opted-in local runtime during application bootstrap.
    /// Fresh installations never call this; the persisted provider row is the
    /// opt-in marker created by the first explicit local-model mutation.
    pub async fn start(&self) -> Result<(), AppError> {
        self.ensure().await.map(|_| ())
    }

    async fn ensure(&self) -> Result<Arc<LocalModelServices>, AppError> {
        if let Some(services) = self.services.get() {
            return Ok(services.clone());
        }

        let _guard = self.init_lock.lock().await;
        if let Some(services) = self.services.get() {
            return Ok(services.clone());
        }

        let (local, server) = start_and_provision_local_model(
            &self.data_dir,
            self.provider_repo.clone(),
            self.encryption_key,
        )
        .await?;
        let image = match ImageModelService::new(&self.data_dir).await {
            Ok(service) => service,
            Err(error) => {
                let _ = crate::disable_local_model_provider(self.provider_repo.clone()).await;
                return Err(error);
            }
        };
        if let Err(error) = image.bind_projection_service(&local).await {
            let _ = crate::disable_local_model_provider(self.provider_repo.clone()).await;
            return Err(error);
        }

        let catalog = local.catalog().await;
        if let Err(error) = reconcile_local_catalog_profiles(
            self.model_profile_repo.as_ref(),
            crate::LOCAL_MODEL_PROVIDER_ID,
            &catalog,
        )
        .await
        {
            let _ = crate::disable_local_model_provider(self.provider_repo.clone()).await;
            return Err(error);
        }
        if let Err(error) = image
            .reconcile_profile(
                self.model_profile_repo.as_ref(),
                crate::LOCAL_MODEL_PROVIDER_ID,
            )
            .await
        {
            let _ = crate::disable_local_model_provider(self.provider_repo.clone()).await;
            return Err(error);
        }

        let services = Arc::new(LocalModelServices {
            local,
            image,
            _server: server,
        });
        self.services
            .set(services.clone())
            .map_err(|_| AppError::Internal("local model runtime initialized twice".into()))?;
        Ok(services)
    }

    pub fn local_if_started(&self) -> Option<Arc<LocalModelService>> {
        self.services.get().map(|services| services.local.clone())
    }

    pub fn image_if_started(&self) -> Option<Arc<ImageModelService>> {
        self.services.get().map(|services| services.image.clone())
    }

    pub async fn local(&self) -> Result<Arc<LocalModelService>, AppError> {
        Ok(self.ensure().await?.local.clone())
    }

    pub async fn image(&self) -> Result<Arc<ImageModelService>, AppError> {
        Ok(self.ensure().await?.image.clone())
    }

    pub fn local_existing(&self) -> Result<Arc<LocalModelService>, AppError> {
        self.local_if_started().ok_or_else(|| {
            AppError::ProviderUnavailable("local model service has not been enabled".into())
        })
    }

    pub fn image_existing(&self) -> Result<Arc<ImageModelService>, AppError> {
        self.image_if_started().ok_or_else(|| {
            AppError::ProviderUnavailable("image model service has not been enabled".into())
        })
    }

    pub fn creation_backend(self: &Arc<Self>) -> Arc<dyn LocalImageBackend> {
        Arc::new(LazyImageBackend {
            runtime: self.clone(),
        })
    }
}

struct LazyImageBackend {
    runtime: Arc<LazyLocalModelRuntime>,
}

#[async_trait::async_trait]
impl LocalImageBackend for LazyImageBackend {
    async fn generate(
        &self,
        request: LocalImageRequest,
    ) -> Result<Vec<ProducedAsset>, CreationError> {
        let services = self.runtime.ensure().await.map_err(|error| {
            CreationError::config(format!("local image runtime initialization failed: {error}"))
        })?;
        services
            .image
            .creation_backend(services.local.workload_gate())
            .generate(request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{
        IProviderRepository, SqliteModelProfileRepository, SqliteProviderRepository,
        init_database_memory,
    };
    use tempfile::TempDir;

    #[tokio::test]
    async fn construction_is_side_effect_free_until_first_mutation() {
        let db = init_database_memory().await.unwrap();
        let temp = TempDir::new().unwrap();
        let provider_repo: Arc<dyn IProviderRepository> =
            Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let profile_repo: Arc<dyn IModelProfileRepository> =
            Arc::new(SqliteModelProfileRepository::new(db.pool().clone()));
        let runtime = LazyLocalModelRuntime::new(
            temp.path(),
            provider_repo.clone(),
            profile_repo,
            [7_u8; 32],
        );

        assert!(!runtime.is_started());
        assert!(!temp.path().join("local-ai").exists());
        assert!(
            provider_repo
                .find_by_id(crate::LOCAL_MODEL_PROVIDER_ID)
                .await
                .unwrap()
                .is_none()
        );

        let _ = runtime.local().await.unwrap();

        assert!(runtime.is_started());
        assert!(temp.path().join("local-ai").is_dir());
        assert!(
            provider_repo
                .find_by_id(crate::LOCAL_MODEL_PROVIDER_ID)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn concurrent_first_use_initializes_only_once() {
        let db = init_database_memory().await.unwrap();
        let temp = TempDir::new().unwrap();
        let provider_repo: Arc<dyn IProviderRepository> =
            Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let profile_repo: Arc<dyn IModelProfileRepository> =
            Arc::new(SqliteModelProfileRepository::new(db.pool().clone()));
        let runtime = LazyLocalModelRuntime::new(
            temp.path(),
            provider_repo,
            profile_repo,
            [9_u8; 32],
        );

        let (local, image) = tokio::join!(runtime.local(), runtime.image());
        assert!(Arc::ptr_eq(
            &local.unwrap(),
            &runtime.local_if_started().unwrap()
        ));
        assert!(Arc::ptr_eq(
            &image.unwrap(),
            &runtime.image_if_started().unwrap()
        ));
    }
}
