use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use nomifun_agent_contracts::{ResourceId, ResourceKind};

use super::*;
use crate::{InMemoryPluginStatePersistence, ResourceHandleIdentity};

struct TestHandle {
    identity: ResourceHandleIdentity,
    releases: AtomicUsize,
    fail: bool,
}

impl TestHandle {
    fn new(id: &str, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            identity: ResourceHandleIdentity {
                binding_id: ResourceBindingId::from(id),
                resource_kind: ResourceKind::from("test"),
                resource_id: ResourceId::from(id),
            },
            releases: AtomicUsize::new(0),
            fail,
        })
    }

    fn result(self: &Arc<Self>) -> ResourceProviderResult {
        ResourceProviderResult { handle: self.clone() }
    }

    fn binding(&self) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: self.identity.binding_id.clone(),
            resource_kind: self.identity.resource_kind.clone(),
            resource_id: self.identity.resource_id.clone(),
            owner_id: "owner".to_owned(),
            operations: BTreeSet::new(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        }
    }
}

#[async_trait]
impl ResourceHandle for TestHandle {
    fn identity(&self) -> &ResourceHandleIdentity {
        &self.identity
    }

    async fn release(&self) -> Result<(), KernelError> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(KernelError::CapabilityExecution {
                reason: format!("release {} failed", self.identity.binding_id.as_ref()),
            })
        } else {
            Ok(())
        }
    }
}

fn registry() -> KernelRegistry {
    KernelRegistry::new(
        MaterializationPolicy::stable("1.0.0"),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap()
}

fn key(id: &str, scope: &str, mount: &str) -> ResourceHandleKey {
    ResourceHandleKey {
        scope_key: ScopeKey::from(scope),
        role_id: None,
        mount_id: PluginMountId::from(mount),
        target_digest: DigestHex::from("a".repeat(64)),
        binding_id: ResourceBindingId::from(id),
    }
}

#[tokio::test]
async fn cleanup_attempts_every_selected_handle_and_preserves_other_scopes() {
    for mode in ["scope", "mount", "all"] {
        let registry = registry();
        let failed = TestHandle::new("a", true);
        let succeeded = TestHandle::new("b", false);
        let unrelated = TestHandle::new("c", false);
        {
            let mut handles = registry.resource_handles.lock().await;
            handles.insert(key("a", "scope", "mount"), failed.clone());
            handles.insert(key("b", "scope", "mount"), succeeded.clone());
            handles.insert(key("c", "unrelated", "other"), unrelated.clone());
        }
        let result = match mode {
            "scope" => registry.release_resources(&ScopeKey::from("scope")).await,
            "mount" => registry.release_resources_for_mount(&PluginMountId::from("mount")).await,
            _ => registry.release_all_resources().await,
        };
        assert!(result.unwrap_err().to_string().contains("release a failed"));
        assert_eq!(failed.releases.load(Ordering::SeqCst), 1);
        assert_eq!(succeeded.releases.load(Ordering::SeqCst), 1);
        assert_eq!(unrelated.releases.load(Ordering::SeqCst), usize::from(mode == "all"));
        registry.release_all_resources().await.unwrap();
        assert_eq!(succeeded.releases.load(Ordering::SeqCst), 1);
        assert_eq!(unrelated.releases.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn rejected_handles_are_released_without_entering_the_registry() {
    for mode in ["unbound", "wrong-kind", "wrong-id", "cleanup-error"] {
        let registry = registry();
        let handle = TestHandle::new("binding", mode == "cleanup-error");
        let mut binding = handle.binding();
        match mode {
            "wrong-kind" => binding.resource_kind = ResourceKind::from("other"),
            "wrong-id" => binding.resource_id = ResourceId::from("other"),
            _ => binding.binding_id = ResourceBindingId::from("other"),
        }
        let error = registry
            .retain_bound_resource_handle(key("binding", "scope", "mount"), &[binding], handle.result())
            .await
            .err()
            .expect("invalid handle must be rejected");
        assert!(error.to_string().contains("resource provider returned a handle"));
        if mode == "cleanup-error" {
            assert!(error.to_string().contains("releasing rejected handle failed"));
        }
        assert_eq!(handle.releases.load(Ordering::SeqCst), 1);
        assert!(registry.resource_handles.lock().await.is_empty());
    }
}

#[tokio::test]
async fn duplicate_acquisition_releases_only_the_losing_handle() {
    let registry = registry();
    let first = TestHandle::new("binding", false);
    let duplicate = TestHandle::new("binding", false);
    for handle in [&first, &duplicate, &first] {
        let result = registry
            .retain_bound_resource_handle(
                key("binding", "scope", "mount"), &[handle.binding()], handle.result(),
            )
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&result.handle, &first.result().handle));
    }
    assert_eq!(first.releases.load(Ordering::SeqCst), 0);
    assert_eq!(duplicate.releases.load(Ordering::SeqCst), 1);
    registry.release_all_resources().await.unwrap();
    assert_eq!(first.releases.load(Ordering::SeqCst), 1);
}
