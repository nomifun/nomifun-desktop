//! Nomi-owned Wave 3 Plugin capability adapter.
//!
//! These four capabilities operate on one exact owner-scoped Plugin selected
//! by the frozen `plugin` resource binding. They deliberately do not proxy a
//! caller-supplied Plugin id and they do not reuse an Active Release's dynamic
//! Agent actions as a substitute for the first-party lifecycle operations.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave3::{
    Wave3CapabilityOperation, Wave3HostPort, Wave3HostPortError, Wave3HostRequest,
};
use nomifun_api_types::{
    PublishPluginRuntimeRequest, ReplacePluginRuntimeSourceFileRequest,
};
use nomifun_plugin_platform::runtime::{PluginRuntimeApplicationError, PluginRuntimeApplicationService};
use serde::Deserialize;

pub(crate) const WAVE3_PLUGIN_NOT_FOUND: &str = "WAVE3_PLUGIN_NOT_FOUND";
pub(crate) const WAVE3_PLUGIN_CONFLICT: &str = "WAVE3_PLUGIN_CONFLICT";
pub(crate) const WAVE3_PLUGIN_RUNTIME_FAILED: &str = "WAVE3_PLUGIN_RUNTIME_FAILED";

#[derive(Clone)]
pub(crate) struct NomiWave3PluginHost {
    application: Arc<PluginRuntimeApplicationService>,
}

impl NomiWave3PluginHost {
    pub(crate) fn new(application: Arc<PluginRuntimeApplicationService>) -> Self {
        Self { application }
    }

    pub(crate) fn into_port(self) -> Arc<dyn Wave3HostPort> {
        Arc::new(self)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginReadInput {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginEditInput {
    expected_product_revision: u64,
    project_id: String,
    expected_project_revision: u64,
    expected_build_generation: u64,
    expected_source_snapshot_digest: String,
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginPublishInput {
    expected_product_revision: u64,
    expected_pointer_revision: u64,
    expected_active_release_epoch: u64,
    ready_release_id: String,
    expected_ready_release_digest: String,
    #[serde(default)]
    expected_active_release_digest: Option<String>,
    #[serde(default)]
    expected_service_test_receipt_id: Option<String>,
    acknowledge_test_warning: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

impl Wave3HostPort for NomiWave3PluginHost {
    fn invoke<'a>(
        &'a self,
        request: Wave3HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave3HostPortError>> + Send + 'a>>
    {
        Box::pin(async move {
            request.validate()?;
            let owner_id = request.context.principal.principal_id.clone();
            let plugin_id = bound_resource_id(&request, "plugin")?;
            let result = match request.operation {
                Wave3CapabilityOperation::PluginRead { input } => {
                    let input: PluginReadInput = parse_input(input)?;
                    match input.path {
                        Some(path) => serde_json::to_value(
                            self.application
                                .source_file(&owner_id, &plugin_id, &path)
                                .await
                                .map_err(map_plugin_error)?,
                        ),
                        None => serde_json::to_value(
                            self.application
                                .workshop(&owner_id, &plugin_id)
                                .await
                                .map_err(map_plugin_error)?,
                        ),
                    }
                }
                Wave3CapabilityOperation::PluginEdit { input } => {
                    let input: PluginEditInput = parse_input(input)?;
                    serde_json::to_value(
                        self.application
                            .replace_source_file(
                                &owner_id,
                                ReplacePluginRuntimeSourceFileRequest {
                                    plugin_id,
                                    expected_product_revision: input.expected_product_revision,
                                    project_id: input.project_id,
                                    expected_project_revision: input.expected_project_revision,
                                    expected_build_generation: input.expected_build_generation,
                                    expected_source_snapshot_digest: input
                                        .expected_source_snapshot_digest,
                                    path: input.path,
                                    content: input.content,
                                },
                            )
                            .await
                            .map_err(map_plugin_error)?,
                    )
                }
                Wave3CapabilityOperation::PluginPublish { input } => {
                    let input: PluginPublishInput = parse_input(input)?;
                    serde_json::to_value(
                        self.application
                            .publish(
                                &owner_id,
                                PublishPluginRuntimeRequest {
                                    plugin_id,
                                    expected_product_revision: input.expected_product_revision,
                                    expected_pointer_revision: input.expected_pointer_revision,
                                    expected_active_release_epoch: input
                                        .expected_active_release_epoch,
                                    ready_release_id: input.ready_release_id,
                                    expected_ready_release_digest: input
                                        .expected_ready_release_digest,
                                    expected_active_release_digest: input
                                        .expected_active_release_digest,
                                    expected_service_test_receipt_id: input
                                        .expected_service_test_receipt_id,
                                    acknowledge_test_warning: input.acknowledge_test_warning,
                                },
                            )
                            .await
                            .map_err(map_plugin_error)?,
                    )
                }
                Wave3CapabilityOperation::PluginServe { input } => {
                    let _: EmptyInput = parse_input(input)?;
                    // Agent callers may inspect the exact published serving
                    // projection but must not receive a bearer-like Surface
                    // capability or create an orphan UI session.
                    let workshop = self
                        .application
                        .workshop(&owner_id, &plugin_id)
                        .await
                        .map_err(map_plugin_error)?;
                    if !workshop.plugin.surface_available {
                        return Err(Wave3HostPortError::invalid_request(
                            "bound Plugin has no enabled Active Release to serve",
                        ));
                    }
                    serde_json::to_value(workshop)
                }
                operation => {
                    return Err(Wave3HostPortError::invalid_request(format!(
                        "Plugin host cannot execute {}",
                        operation.capability_id().as_ref()
                    )));
                }
            }
            .map_err(|error| {
                Wave3HostPortError::new(
                    "WAVE3_PLUGIN_SERIALIZATION_FAILED",
                    format!("Plugin result serialization failed: {error}"),
                )
            })?;
            Ok(StrictJsonValue(result))
        })
    }
}

fn bound_resource_id(
    request: &Wave3HostRequest,
    expected_kind: &str,
) -> Result<String, Wave3HostPortError> {
    request
        .context
        .resource_bindings
        .iter()
        .find(|binding| binding.resource_kind.as_ref() == expected_kind)
        .map(|binding| binding.resource_id.as_ref().to_owned())
        .ok_or_else(|| {
            Wave3HostPortError::resource_binding_invalid(format!(
                "{} requires one {expected_kind} resource binding",
                request.context.capability_id.as_ref()
            ))
        })
}

fn parse_input<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave3HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave3HostPortError::invalid_request(format!("Plugin action input is invalid: {error}"))
    })
}

fn map_plugin_error(error: PluginRuntimeApplicationError) -> Wave3HostPortError {
    match error {
        PluginRuntimeApplicationError::AgentSession { message, .. } => {
            Wave3HostPortError::new(WAVE3_PLUGIN_RUNTIME_FAILED, message)
        }
        PluginRuntimeApplicationError::NotFound => {
            Wave3HostPortError::new(WAVE3_PLUGIN_NOT_FOUND, "bound Plugin was not found")
        }
        PluginRuntimeApplicationError::Invalid(message) => {
            Wave3HostPortError::invalid_request(message)
        }
        PluginRuntimeApplicationError::Runtime(message) => {
            Wave3HostPortError::new(WAVE3_PLUGIN_RUNTIME_FAILED, message)
        }
        PluginRuntimeApplicationError::Database(error) => {
            let message = error.to_string();
            let code = if message.to_ascii_lowercase().contains("conflict") {
                WAVE3_PLUGIN_CONFLICT
            } else {
                WAVE3_PLUGIN_RUNTIME_FAILED
            };
            Wave3HostPortError::new(code, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, IdempotencyKey,
        PluginProductId, OperationId, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        ResourceBindingId, ResourceId, ResourceKind, ScopeKey, TypedResourceBinding,
    };
    use nomifun_agent_domain_wave3::PLUGIN_RESOURCE_KIND;
    use nomifun_api_types::{
        BuildPluginRuntimeRequest, CreatePluginRuntimeProjectRequest, PluginRuntimeWorkshopDto,
        SetPluginRuntimeEnabledRequest,
    };
    use nomifun_db::{
        IPluginRuntimeRepository, SqlitePluginRuntimeRepository, init_database_memory,
        installation_owner_id,
    };
    use serde_json::json;

    use super::*;

    fn request(
        owner_id: &str,
        plugin_id: &str,
        action_id: &str,
        operation: Wave3CapabilityOperation,
    ) -> Wave3HostRequest {
        Wave3HostRequest {
            context: nomifun_agent_domain_wave3::Wave3HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.to_owned(),
                },
                agent_session_id: AgentSessionId::from("plugin-wave3-session"),
                operation_id: OperationId::from("plugin-wave3-operation"),
                idempotency_key: IdempotencyKey::from("plugin-wave3-idempotency"),
                correlation_id: CorrelationId::from("plugin-wave3-correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: ResolvedSnapshotId::from("plugin-wave3-snapshot"),
                    snapshot_digest: DigestHex::from("a".repeat(64)),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(
                    nomifun_agent_domain_wave3::PLUGIN_DEVELOPMENT_MODULE_ID,
                ),
                action_id: ActionId::from(action_id),
                state_scope_key: ScopeKey::from("session:plugin-wave3"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("plugin-binding"),
                    resource_kind: ResourceKind::from(PLUGIN_RESOURCE_KIND),
                    resource_id: ResourceId::from(plugin_id.to_owned()),
                    owner_id: owner_id.to_owned(),
                    operations: BTreeSet::from([
                        "read".into(),
                        "edit".into(),
                        "publish".into(),
                        "serve".into(),
                    ]),
                    connection_config_ref: None,
                    typed_parameters: Default::default(),
                }],
            },
            operation,
        }
    }

    #[tokio::test]
    async fn plugin_actions_use_bound_identity_and_persist_edit_publish_and_serve_state() {
        let database = init_database_memory().await.expect("database");
        let owner_id = installation_owner_id(database.pool()).await.expect("owner");
        let repository: Arc<dyn IPluginRuntimeRepository> =
            Arc::new(SqlitePluginRuntimeRepository::new(database.pool().clone()));
        let root = tempfile::tempdir().expect("root");
        let application = Arc::new(
            PluginRuntimeApplicationService::new_with_root(repository, root.path())
                .expect("application"),
        );
        let created = application
            .create(
                &owner_id,
                CreatePluginRuntimeProjectRequest {
                    expected_library_revision: 0,
                    display_name: "Wave3 Plugin".into(),
                    description: None,
                    service_source: None,
                },
            )
            .await
            .expect("create Plugin");
        let plugin_id = created.plugin.plugin_id.clone();
        let source_digest = created
            .source_snapshot_digest
            .clone()
            .expect("source digest");
        let host = NomiWave3PluginHost::new(Arc::clone(&application));

        let output = host
            .invoke(request(
                &owner_id,
                &plugin_id,
                "plugin.development/edit",
                Wave3CapabilityOperation::PluginEdit {
                    input: StrictJsonValue(json!({
                        "expected_product_revision": created.plugin.product_revision,
                        "project_id": created.project_id,
                        "expected_project_revision": created.project_revision,
                        "expected_build_generation": created.build_generation,
                        "expected_source_snapshot_digest": source_digest,
                        "path": "ui/index.html",
                        "content": "<!doctype html><title>Wave3 persisted</title>"
                    })),
                },
            ))
            .await
            .expect("edit through Wave3");
        assert_eq!(output.0["plugin"]["plugin_id"], plugin_id);
        let edited: PluginRuntimeWorkshopDto =
            serde_json::from_value(output.0.clone()).expect("edited workshop");

        let saved = application
            .source_file(&owner_id, &plugin_id, "ui/index.html")
            .await
            .expect("saved source");
        assert_eq!(
            saved.content,
            "<!doctype html><title>Wave3 persisted</title>"
        );

        let read = host
            .invoke(request(
                &owner_id,
                &plugin_id,
                "plugin.development/read",
                Wave3CapabilityOperation::PluginRead {
                    input: StrictJsonValue(json!({ "path": "ui/index.html" })),
                },
            ))
            .await
            .expect("read through Wave3");
        assert_eq!(read.0["content"], saved.content);
        assert_eq!(PluginProductId::from(plugin_id).as_ref(), read.0["plugin_id"]);

        let built = application
            .build(
                &owner_id,
                BuildPluginRuntimeRequest {
                    plugin_id: edited.plugin.plugin_id.clone(),
                    expected_product_revision: edited.plugin.product_revision,
                    project_id: edited.project_id.clone(),
                    expected_project_revision: edited.project_revision,
                    expected_build_generation: edited.build_generation,
                    expected_source_snapshot_digest: edited
                        .source_snapshot_digest
                        .clone()
                        .expect("source digest"),
                    expected_dependency_lock_digest: edited
                        .dependency_lock_digest
                        .clone()
                        .expect("dependency digest"),
                    service_lifecycle: None,
                },
            )
            .await
            .expect("build Ready Release");
        let ready = built.ready.as_ref().expect("ready release");
        let published = host
            .invoke(request(
                &owner_id,
                &built.plugin.plugin_id,
                "plugin.development/publish",
                Wave3CapabilityOperation::PluginPublish {
                    input: StrictJsonValue(json!({
                        "expected_product_revision": built.plugin.product_revision,
                        "expected_pointer_revision": built.plugin.releases.pointer_revision,
                        "expected_active_release_epoch": built.plugin.releases.active_release_epoch,
                        "ready_release_id": ready.release.release_id,
                        "expected_ready_release_digest": ready.release.release_digest,
                        "acknowledge_test_warning": false
                    })),
                },
            ))
            .await
            .expect("publish through Wave3");
        let published: PluginRuntimeWorkshopDto =
            serde_json::from_value(published.0).expect("published workshop");
        let active_digest = published
            .plugin
            .releases
            .active
            .as_ref()
            .map(|release| release.release_digest.clone());
        let enabled = application
            .set_enabled(
                &owner_id,
                SetPluginRuntimeEnabledRequest {
                    plugin_id: published.plugin.plugin_id.clone(),
                    expected_product_revision: published.plugin.product_revision,
                    expected_pointer_revision: published.plugin.releases.pointer_revision,
                    expected_active_release_digest: active_digest,
                    enabled: true,
                },
            )
            .await
            .expect("enable published Plugin");
        assert!(enabled.plugin.surface_available);

        let serving = host
            .invoke(request(
                &owner_id,
                &enabled.plugin.plugin_id,
                "plugin.development/serve",
                Wave3CapabilityOperation::PluginServe {
                    input: StrictJsonValue(json!({})),
                },
            ))
            .await
            .expect("read serving projection through Wave3");
        assert_eq!(serving.0["plugin"]["surface_available"], true);
        assert!(serving.0.get("surface_capability").is_none());
    }
}
