use async_trait::async_trait;
use nomifun_agent_contracts::*;
use nomifun_js_runtime::{
    CommittedRuntimeProvider, JavaScriptRuntimeError, JavaScriptWorkKind, ResolvedNodeRuntime,
    RuntimeUseLease,
};
use nomifun_plugin_platform::runtime::*;
use std::{path::Path, sync::Arc};

pub struct NoNode;
#[async_trait]
impl CommittedRuntimeProvider for NoNode {
    async fn acquire_use(
        &self,
        _: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        panic!("native services must not acquire a Node lease")
    }
    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        panic!("native services must not resolve Node")
    }
}

pub fn executable() -> Vec<u8> {
    std::fs::read(
        std::env::var_os("NOMIFUN_NATIVE_TEST_EXECUTABLE").expect(
            "build nomifun-plugin-sdk --example echo and set NOMIFUN_NATIVE_TEST_EXECUTABLE",
        ),
    )
    .unwrap()
}

pub fn binding(root: &Path, allow: bool) -> Arc<ProductionPluginRuntimeServiceRuntimeBinding> {
    Arc::new(
        ProductionPluginRuntimeServiceRuntimeBinding::new_with_storage_and_native_policy(
            Arc::new(NoNode),
            Arc::new(PluginRuntimeServiceModuleRegistry::new(root.to_owned()).unwrap()),
            Some(Arc::new(InMemoryPluginRuntimeManagedStorage::new())),
            8,
            allow,
        )
        .unwrap(),
    )
}

pub fn materialization(bytes: Vec<u8>) -> PluginRuntimeStaticServiceMaterialization {
    materialize_native_service_release(PluginRuntimeNativeServiceInput {
        executable: bytes,
        target: NativePluginTarget::current().unwrap(),
        lifecycle: PluginServiceLifecycle::OnDemand,
        uses_files: false,
        uses_private_database: false,
        service_contract_digest: digest_bytes(b"contract"),
        runtime_requirements_digest: digest_bytes(b"native-v1"),
    })
    .unwrap()
}

#[allow(dead_code)]
pub fn spec_input(descriptor: PluginServiceReleaseDescriptor) -> PluginRuntimeServiceSpecInput {
    let plugin_product_id = PluginProductId::from("native-test");
    PluginRuntimeServiceSpecInput {
        plugin_product_id: plugin_product_id.clone(),
        release: PluginReleaseRef {
            release_id: "native-release".into(),
            artifact_id: "native-artifact".into(),
            release_digest: digest_bytes(b"release"),
            manifest_digest: digest_bytes(b"manifest"),
        },
        active_release_epoch: 1,
        descriptor,
        config_schema_digest: digest_bytes(b"config"),
        config_snapshot_digest: digest_bytes(b"snapshot"),
        credential_slots_digest: digest_bytes(b"slots"),
        resource_contract_digest: digest_bytes(b"resources"),
        resource_bindings_digest: digest_bytes(b"bindings"),
        bridge_contract_digest: digest_bytes(b"bridge"),
        contribution_set_digest: digest_bytes(b"contributions"),
        storage: PluginRuntimeServiceStorageResolution::host_kv(plugin_product_id).descriptor,
    }
}
