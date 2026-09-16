//! Product-facing DTOs for the Plugin runtime platform.
//!
//! Surface launch data contains only Host-consumable descriptors. MessageChannel
//! Bridge sessions and internal Service/storage handles remain private.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CapabilityCatalogItemDto, CredentialSlotBindingDto, DurableOperationSummaryDto,
    PluginConfigSchemaDto, PluginConfigStateDto,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeKindDto {
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeServiceLifecycleDto {
    OnDemand,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeLifecycleDto {
    Enabled,
    Disabled,
    Trashed,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginRuntimeServiceHealthDto {
    NotApplicable,
    Stopped,
    Starting {
        release_id: String,
        expected_release_digest: String,
    },
    Ready {
        release_id: String,
        expected_release_digest: String,
        started_at_ms: i64,
    },
    Failed {
        release_id: String,
        expected_release_digest: String,
        error_code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeReleaseRefDto {
    pub release_id: String,
    pub artifact_id: String,
    pub release_digest: String,
    pub manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeReleasePointersDto {
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<PluginRuntimeReleaseRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<PluginRuntimeReleaseRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PluginRuntimeReleaseRefDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeSummaryDto {
    pub plugin_id: String,
    pub product_revision: u64,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_asset_id: Option<String>,
    pub kind: PluginRuntimeKindDto,
    pub lifecycle: PluginRuntimeLifecycleDto,
    pub releases: PluginRuntimeReleasePointersDto,
    pub service_health: PluginRuntimeServiceHealthDto,
    pub surface_available: bool,
    pub contribution_count: u32,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeLibraryResponseDto {
    pub library_revision: u64,
    pub plugins: Vec<PluginRuntimeSummaryDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeProjectSourceStateDto {
    Empty,
    Editable,
    RuntimeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeServiceDescriptorDto {
    pub lifecycle: PluginRuntimeServiceLifecycleDto,
    pub uses_files: bool,
    pub uses_private_database: bool,
    pub service_contract_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeTestStatusDto {
    NotRequired,
    NotRun,
    Passed,
    Failed,
    NeedsTestInput,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeReleaseTestDto {
    pub status: PluginRuntimeTestStatusDto,
    pub release_id: String,
    pub expected_release_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_service_run_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeReadyReleaseDto {
    pub release: PluginRuntimeReleaseRefDto,
    pub project_build_generation: u64,
    pub created_at_ms: i64,
    pub kind: PluginRuntimeKindDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<PluginRuntimeServiceDescriptorDto>,
    pub test: PluginRuntimeReleaseTestDto,
    pub migration_count: u32,
    pub can_publish: bool,
    pub can_auto_publish: bool,
    #[serde(default)]
    pub blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeWorkshopDto {
    pub plugin: PluginRuntimeSummaryDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_lifecycle: Option<PluginRuntimeServiceLifecycleDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_service: Option<PluginRuntimeServiceDescriptorDto>,
    pub publish_mode: PluginRuntimePublishModeDto,
    pub project_id: String,
    pub project_revision: u64,
    pub source_state: PluginRuntimeProjectSourceStateDto,
    pub build_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_lock_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<PluginRuntimeReadyReleaseDto>,
    pub config_schema: PluginConfigSchemaDto,
    pub config: PluginConfigStateDto,
    pub credential_bindings_revision: u64,
    pub credential_slots: Vec<CredentialSlotBindingDto>,
    pub capabilities: Vec<CapabilityCatalogItemDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_operation: Option<DurableOperationSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeSurfaceLaunchDescriptorDto {
    pub plugin_id: String,
    pub product_revision: u64,
    pub release_id: String,
    pub expected_release_digest: String,
    pub active_release_epoch: u64,
    pub surface_session_id: String,
    pub surface_generation: u64,
    pub surface_capability: String,
    pub ui_entrypoint: String,
    pub kind: PluginRuntimeKindDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePluginRuntimeProjectRequest {
    pub expected_library_revision: u64,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional service entrypoint; UI and service roles may coexist and change in later releases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeSourceFileDto {
    pub plugin_id: String,
    pub project_id: String,
    pub path: String,
    pub content: String,
    pub source_snapshot_digest: String,
    pub build_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacePluginRuntimeSourceFileRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub expected_source_snapshot_digest: String,
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub expected_source_snapshot_digest: String,
    pub expected_dependency_lock_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_lifecycle: Option<PluginRuntimeServiceLifecycleDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelPluginRuntimeBuildRequest {
    pub expected_operation_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestPluginRuntimeReleaseRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub release_id: String,
    pub expected_release_digest: String,
    pub expected_config_revision: u64,
    pub expected_credential_bindings_revision: u64,
    pub resolved_test_input_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishPluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub ready_release_id: String,
    pub expected_ready_release_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_service_test_receipt_id: Option<String>,
    pub acknowledge_test_warning: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimePublishModeDto {
    Manual,
    AutoUiOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginRuntimePublishModeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub mode: PluginRuntimePublishModeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardPluginRuntimeReadyReleaseRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub ready_release_id: String,
    pub expected_ready_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackPluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_current_release_digest: String,
    pub previous_release_id: String,
    pub expected_previous_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginRuntimeEnabledRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenPluginRuntimeSurfaceRequest {
    pub plugin_id: String,
    /// Explicit host UI consent to observe/send/cancel this existing Session.
    /// Omitted for a normal standalone plugin page. Explicit null is not a grant.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_surface_session_grant")]
    pub agent_session: Option<PluginAgentSurfaceGrantDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAgentSurfaceGrantDto {
    pub agent_session_id: String,
    /// Optional exact Catalog choice for a replacement Agent page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_capability: Option<crate::ExactCatalogRefDto>,
    /// Consent applies only to the release displayed by the trusted host UI.
    pub expected_release_digest: String,
}

fn deserialize_surface_session_grant<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<PluginAgentSurfaceGrantDto>, D::Error> {
    PluginAgentSurfaceGrantDto::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosePluginRuntimeSurfaceRequest {
    pub plugin_id: String,
    pub surface_session_id: String,
    pub surface_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashPluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: PluginRuntimeLifecycleDto,
    pub expected_pointer_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPluginRuntimeServiceRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_active_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginRuntimeServiceRunningRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_active_release_digest: String,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: PluginRuntimeLifecycleDto,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPluginRuntimeDeleteRequest {
    pub plugin_id: String,
    pub failed_operation_id: String,
    pub expected_operation_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeShareContentDto {
    ReadyRelease,
    ActiveRelease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharePluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub content: PluginRuntimeShareContentDto,
    pub release_id: String,
    pub expected_release_digest: String,
    pub destination_path: String,
    pub include_source: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPluginRuntimeShareRequest {
    pub expected_library_revision: u64,
    pub source_path: String,
    pub expected_bundle_digest: String,
    pub expected_release_digest: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPluginRuntimeArtifactRequest {
    pub expected_library_revision: u64,
    pub source_path: String,
    pub expected_artifact_digest: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPluginRuntimeBackupRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: PluginRuntimeLifecycleDto,
    pub expected_pointer_revision: u64,
    pub expected_config_revision: u64,
    pub expected_credential_bindings_revision: u64,
    pub destination_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPluginRuntimeBackupRequest {
    pub expected_library_revision: u64,
    pub source_path: String,
    pub expected_backup_metadata_digest: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurePluginRuntimeRequest {
    pub plugin_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_config_revision: u64,
    pub expected_schema_digest: String,
    pub values: Value,
    pub credential_bindings: std::collections::BTreeMap<String, Option<String>>,
    pub expected_credential_bindings_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn serialized_field_names(value: &Value, output: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    output.push(key.clone());
                    serialized_field_names(value, output);
                }
            }
            Value::Array(values) => {
                for value in values {
                    serialized_field_names(value, output);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn plugin_surface_launch_has_no_bridge_or_localhost_wire() {
        let value = serde_json::to_value(PluginRuntimeSurfaceLaunchDescriptorDto {
            plugin_id: "plugin-1".into(),
            product_revision: 3,
            release_id: "release-2".into(),
            expected_release_digest: "a".repeat(64),
            active_release_epoch: 8,
            surface_session_id: "surface-session-1".into(),
            surface_generation: 2,
            surface_capability: "surface-capability".into(),
            ui_entrypoint: "ui/index.html".into(),
            kind: PluginRuntimeKindDto::Plugin,
        })
        .unwrap();

        let mut fields = Vec::new();
        serialized_field_names(&value, &mut fields);
        for forbidden in [
            "bridge",
            "message_channel",
            "localhost",
            "port",
            "host_generation",
            "service_run_key",
        ] {
            assert!(
                fields.iter().all(|field| !field.contains(forbidden)),
                "internal field leaked: {forbidden}"
            );
        }
        assert_eq!(value["kind"], "plugin");
    }

    #[test]
    fn plugin_requests_reject_unknown_fields() {
        let error = serde_json::from_value::<BuildPluginRuntimeRequest>(json!({
            "plugin_id": "plugin-1",
            "expected_product_revision": 3,
            "project_id": "project-1",
            "expected_project_revision": 4,
            "expected_build_generation": 5,
            "expected_source_snapshot_digest": "a".repeat(64),
            "expected_dependency_lock_digest": "b".repeat(64),
            "html": "<html></html>"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn plugin_surface_open_request_is_explicit_and_rejects_extra_authority() {
        let request = serde_json::to_value(OpenPluginRuntimeSurfaceRequest {
            plugin_id: "plugin-1".into(),
            agent_session: None,
        })
        .unwrap();
        assert_eq!(request, json!({"plugin_id": "plugin-1"}));

        let error = serde_json::from_value::<OpenPluginRuntimeSurfaceRequest>(json!({
            "plugin_id": "plugin-1",
            "surface_capability": "caller-must-not-supply-capability"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn plugin_surface_session_consent_requires_exact_release_and_no_extra_authority() {
        let grant = json!({"agent_session_id": "session", "expected_release_digest": "a".repeat(64)});
        let request: OpenPluginRuntimeSurfaceRequest = serde_json::from_value(json!({
            "plugin_id": "plugin", "agent_session": grant,
        })).unwrap();
        assert_eq!(serde_json::to_value(request.agent_session.unwrap()).unwrap(), grant);
        for invalid in [
            serde_json::Value::Null,
            json!("session"),
            json!({"agent_session_id": "session"}),
            json!({"agent_session_id": "session", "expected_release_digest": "a".repeat(64), "owner_user_id": "other"}),
        ] {
            assert!(serde_json::from_value::<OpenPluginRuntimeSurfaceRequest>(json!({
                "plugin_id": "plugin", "agent_session": invalid,
            })).is_err());
        }
    }

    #[test]
    fn plugin_publish_and_rollback_bind_exact_release_pointers() {
        let publish = serde_json::to_value(PublishPluginRuntimeRequest {
            plugin_id: "plugin-1".into(),
            expected_product_revision: 3,
            expected_pointer_revision: 5,
            expected_active_release_epoch: 7,
            ready_release_id: "release-2".into(),
            expected_ready_release_digest: "a".repeat(64),
            expected_active_release_digest: Some("b".repeat(64)),
            expected_service_test_receipt_id: Some("receipt-1".into()),
            acknowledge_test_warning: false,
        })
        .unwrap();
        let rollback = serde_json::to_value(RollbackPluginRuntimeRequest {
            plugin_id: "plugin-1".into(),
            expected_product_revision: 4,
            expected_pointer_revision: 6,
            expected_active_release_epoch: 8,
            expected_current_release_digest: "a".repeat(64),
            previous_release_id: "release-1".into(),
            expected_previous_release_digest: "b".repeat(64),
        })
        .unwrap();

        assert_eq!(publish["expected_pointer_revision"], 5);
        assert_eq!(publish["expected_active_release_epoch"], 7);
        assert_eq!(rollback["expected_pointer_revision"], 6);
        assert_eq!(rollback["expected_active_release_epoch"], 8);
        assert!(publish.get("conversation_id").is_none());
        assert!(publish.get("guid_mode").is_none());
    }

    #[test]
    fn plugin_credential_binding_wire_contains_references_only() {
        let request = ConfigurePluginRuntimeRequest {
            plugin_id: "plugin-1".into(),
            expected_product_revision: 3,
            expected_pointer_revision: 5,
            expected_config_revision: 2,
            expected_schema_digest: "a".repeat(64),
            values: json!({"region": "us-east"}),
            credential_bindings: std::collections::BTreeMap::from([(
                "provider".into(),
                Some("credential-1".into()),
            )]),
            expected_credential_bindings_revision: 4,
        };

        let value = serde_json::to_value(request).unwrap();
        let mut fields = Vec::new();
        serialized_field_names(&value, &mut fields);
        assert!(fields.iter().all(|field| !field.contains("secret")));
        assert!(fields.iter().all(|field| !field.contains("plaintext")));
        assert_eq!(value["credential_bindings"]["provider"], "credential-1");
    }
}
