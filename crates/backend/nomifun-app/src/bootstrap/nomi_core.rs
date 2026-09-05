use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;

use nomifun_auth::{AuthPolicy, hash_password, validate_password, validate_username};

use crate::lan_endpoint::RobotAdvertiseAddr;
use crate::router::create_router;
use crate::services::AppServices;

use super::{ServerEnvironment, finalize_data_layer, init_data_layer};

#[cfg(test)]
mod registry_probe {
    #[test]
    fn declarative_catalog_materializes() {
        let registrations = nomifun_agent_domain_support::registrations(
            nomifun_agent_domain_support::c7_package_specs(),
        )
        .expect("declarative registrations");
        let registry = nomifun_agent_kernel::Materializer::materialize(
            &nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
            &registrations,
            1,
        )
        .expect("declarative catalog");
        assert!(!registry.capabilities.is_empty());
    }
}


/// The current product composition backed by NomiFun's in-process Nomi engine.
///
/// Runtime alternatives remain separate host compositions. They do not share a
/// mutable engine selector and an existing Conversation never changes runtime
/// families in place.
#[derive(Clone)]
pub struct NomiCoreApplication {
    services: Arc<AppServices>,
    router: Router,
}

impl NomiCoreApplication {
    pub(crate) fn from_parts(services: AppServices, router: Router) -> Self {
        Self {
            services: Arc::new(services),
            router,
        }
    }

    pub async fn compose(environment: &ServerEnvironment) -> Result<Self> {
        Self::compose_with_config(environment, &environment.config).await
    }

    /// Compose the Nomi core against an explicit host policy.
    ///
    /// Desktop uses this to replace the CLI's authentication policy with its
    /// per-boot local-trust policy while keeping database, service, router, and
    /// runtime construction in the same composition root as Web and `nomicore`.
    pub(crate) async fn compose_with_config(
        environment: &ServerEnvironment,
        config: &crate::AppConfig,
    ) -> Result<Self> {
        let database = init_data_layer(config).await?;
        let services = AppServices::from_config(database, config)
            .await?
            .with_boot_reconciliation_authority(
                environment.boot_reconciliation_authority(),
                config,
            )
            .await?;
        if let Err(error) = finalize_data_layer(config) {
            return Err(services.cleanup_after_startup_failure(error).await);
        }
        let router = create_router(&services).await;
        Ok(Self::from_parts(services, router))
    }

    pub fn router(&self) -> Router {
        self.router.clone()
    }

    pub(crate) fn services(&self) -> &AppServices {
        self.services.as_ref()
    }

    pub fn auth_policy(&self) -> AuthPolicy {
        self.services.auth_policy
    }

    pub async fn services_has_users(&self) -> Result<bool> {
        self.services
            .user_repo
            .has_users()
            .await
            .map_err(|error| anyhow::anyhow!("read configured users: {error}"))
    }

    pub async fn ensure_admin_credentials(
        &self,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<bool> {
        if !self.auth_policy().requires_admin_provisioning() {
            return Ok(false);
        }
        if self
            .services
            .user_repo
            .has_users()
            .await
            .map_err(|error| anyhow::anyhow!("admin bootstrap query failed: {error}"))?
        {
            return Ok(false);
        }
        let Some(password) = password else {
            return Ok(true);
        };
        let username = username.unwrap_or("admin");
        validate_username(username)
            .map_err(|error| anyhow::anyhow!("invalid admin username: {error}"))?;
        validate_password(password)
            .map_err(|error| anyhow::anyhow!("invalid admin password: {error}"))?;

        let password = password.to_owned();
        let hash = tokio::task::spawn_blocking(move || hash_password(&password))
            .await
            .context("admin password hash task failed")?
            .map_err(|error| anyhow::anyhow!("admin password hashing failed: {error}"))?;
        let provisioned = self
            .services
            .user_repo
            .set_system_user_credentials_if_uninitialized(username, &hash)
            .await
            .map_err(|error| anyhow::anyhow!("admin credential write failed: {error}"))?;
        if provisioned {
            Ok(false)
        } else {
            Ok(!self.services.user_repo.has_users().await.unwrap_or(false))
        }
    }

    pub fn publish_robot_endpoint(
        &self,
        actual_port: u16,
        advertise: Option<RobotAdvertiseAddr>,
    ) {
        crate::lan_endpoint::publish_robot_endpoint(
            self.services.as_ref(),
            actual_port,
            advertise,
        );
    }

    pub async fn close(self) -> Result<()> {
        // The HTTP listener is already quiescent when this method is called.
        // Desktop startup-failure cleanup uses this exact same AppServices
        // sequence, including the background-task join barrier before SQLite.
        self.services.shutdown_nomi_core_host().await
    }
}
