use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CredentialId, PluginMountId, PluginProjectId, SensitiveString,
};
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PluginOwnerMutationScope {
    pub project_id: Option<PluginProjectId>,
    pub mount_id: Option<PluginMountId>,
}

impl PluginOwnerMutationScope {
    pub fn project(project_id: PluginProjectId) -> Result<Self, OwnerMutationError> {
        Self::new(Some(project_id), None)
    }

    pub fn mount(mount_id: PluginMountId) -> Result<Self, OwnerMutationError> {
        Self::new(None, Some(mount_id))
    }

    pub fn linked(
        project_id: PluginProjectId,
        mount_id: PluginMountId,
    ) -> Result<Self, OwnerMutationError> {
        Self::new(Some(project_id), Some(mount_id))
    }

    pub fn new(
        project_id: Option<PluginProjectId>,
        mount_id: Option<PluginMountId>,
    ) -> Result<Self, OwnerMutationError> {
        if project_id
            .as_ref()
            .is_some_and(|value| value.as_ref().trim().is_empty())
            || mount_id
                .as_ref()
                .is_some_and(|value| value.as_ref().trim().is_empty())
            || (project_id.is_none() && mount_id.is_none())
        {
            return Err(OwnerMutationError::InvalidScope);
        }
        Ok(Self {
            project_id,
            mount_id,
        })
    }

    fn keys(&self) -> Vec<String> {
        let mut keys = BTreeSet::new();
        if let Some(project_id) = &self.project_id {
            keys.insert(format!("project:{}", project_id.as_ref()));
        }
        if let Some(mount_id) = &self.mount_id {
            keys.insert(format!("mount:{}", mount_id.as_ref()));
        }
        keys.into_iter().collect()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OwnerMutationError {
    #[error("Plugin owner mutation scope must contain a non-empty Project or Mount identity")]
    InvalidScope,
    #[error("Plugin owner mutation registry lock is poisoned")]
    RegistryPoisoned,
}

#[derive(Default)]
pub struct OwnerMutationCoordinator {
    locks: Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>,
}

impl std::fmt::Debug for OwnerMutationCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerMutationCoordinator")
            .finish_non_exhaustive()
    }
}

impl OwnerMutationCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(
        &self,
        scope: &PluginOwnerMutationScope,
    ) -> Result<OwnerMutationGuard, OwnerMutationError> {
        let keys = scope.keys();
        let locks = {
            let mut registry = self
                .locks
                .lock()
                .map_err(|_| OwnerMutationError::RegistryPoisoned)?;
            registry.retain(|_, lock| lock.strong_count() > 0);
            keys.iter()
                .map(|key| {
                    let lock = registry
                        .get(key)
                        .and_then(Weak::upgrade)
                        .unwrap_or_else(|| {
                            let lock = Arc::new(AsyncMutex::new(()));
                            registry.insert(key.clone(), Arc::downgrade(&lock));
                            lock
                        });
                    (key.clone(), lock)
                })
                .collect::<Vec<_>>()
        };

        let mut guards = Vec::with_capacity(locks.len());
        for (_, lock) in &locks {
            guards.push(Arc::clone(lock).lock_owned().await);
        }
        Ok(OwnerMutationGuard {
            keys,
            _locks: locks.into_iter().map(|(_, lock)| lock).collect(),
            _guards: guards,
        })
    }
}

pub struct OwnerMutationGuard {
    keys: Vec<String>,
    _locks: Vec<Arc<AsyncMutex<()>>>,
    _guards: Vec<OwnedMutexGuard<()>>,
}

impl std::fmt::Debug for OwnerMutationGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerMutationGuard")
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl OwnerMutationGuard {
    pub fn keys(&self) -> &[String] {
        &self.keys
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginCredentialVaultError {
    #[error("Credential is unavailable")]
    Unavailable,
    #[error("Credential access failed")]
    AccessFailed,
}

#[async_trait]
pub trait PluginCredentialVault: Send + Sync {
    async fn resolve(
        &self,
        credential_id: &CredentialId,
    ) -> Result<SensitiveString, PluginCredentialVaultError>;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;

    #[test]
    fn scope_requires_at_least_one_real_owner() {
        assert_eq!(
            PluginOwnerMutationScope::new(None, None).unwrap_err(),
            OwnerMutationError::InvalidScope
        );
        assert!(PluginOwnerMutationScope::project(PluginProjectId::from("project-1")).is_ok());
        assert!(PluginOwnerMutationScope::mount(PluginMountId::from("mount-1")).is_ok());
    }

    #[tokio::test]
    async fn linked_scope_serializes_project_and_mount_mutations() {
        let coordinator = Arc::new(OwnerMutationCoordinator::new());
        let linked = PluginOwnerMutationScope::linked(
            PluginProjectId::from("project-1"),
            PluginMountId::from("mount-1"),
        )
        .unwrap();
        let linked_guard = coordinator.acquire(&linked).await.unwrap();
        assert_eq!(
            linked_guard.keys(),
            &["mount:mount-1".to_owned(), "project:project-1".to_owned()]
        );

        let acquired = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            let acquired = Arc::clone(&acquired);
            async move {
                let scope =
                    PluginOwnerMutationScope::mount(PluginMountId::from("mount-1")).unwrap();
                let _guard = coordinator.acquire(&scope).await.unwrap();
                acquired.store(true, Ordering::Release);
            }
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!acquired.load(Ordering::Acquire));
        drop(linked_guard);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(acquired.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn unrelated_owners_do_not_block_each_other() {
        let coordinator = OwnerMutationCoordinator::new();
        let first = coordinator
            .acquire(
                &PluginOwnerMutationScope::mount(PluginMountId::from("mount-1")).unwrap(),
            )
            .await
            .unwrap();
        let second = tokio::time::timeout(
            Duration::from_millis(50),
            coordinator.acquire(
                &PluginOwnerMutationScope::mount(PluginMountId::from("mount-2")).unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        drop((first, second));
    }
}
