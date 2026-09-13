//! Product-facing DTOs for the Phase M1 MiniApp Platform.
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
    UiOnly,
    Service,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugins")]
    pub miniapps: Vec<PluginRuntimeSummaryDto>,
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
    #[serde(rename = "plugin")]
    pub miniapp: PluginRuntimeSummaryDto,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    pub kind: PluginRuntimeKindDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeSourceFileDto {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub project_id: String,
    pub path: String,
    pub content: String,
    pub source_snapshot_digest: String,
    pub build_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacePluginRuntimeSourceFileRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub mode: PluginRuntimePublishModeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardPluginRuntimeReadyReleaseRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub ready_release_id: String,
    pub expected_ready_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackPluginRuntimeRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenPluginRuntimeSurfaceRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosePluginRuntimeSurfaceRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub surface_session_id: String,
    pub surface_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashPluginRuntimeRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePluginRuntimeRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: PluginRuntimeLifecycleDto,
    pub expected_pointer_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPluginRuntimeServiceRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_active_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPluginRuntimeServiceRunningRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_active_release_digest: String,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePluginRuntimeRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: PluginRuntimeLifecycleDto,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPluginRuntimeDeleteRequest {
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    #[serde(rename = "plugin_id")]
    pub miniapp_id: String,
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
    fn surface_launch_has_no_bridge_or_localhost_wire() {
        let value = serde_json::to_value(PluginRuntimeSurfaceLaunchDescriptorDto {
            miniapp_id: "miniapp-1".into(),
            product_revision: 3,
            release_id: "release-2".into(),
            expected_release_digest: "a".repeat(64),
            active_release_epoch: 8,
            surface_session_id: "surface-session-1".into(),
            surface_generation: 2,
            surface_capability: "surface-capability".into(),
            ui_entrypoint: "ui/index.html".into(),
            kind: PluginRuntimeKindDto::Service,
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
        assert_eq!(value["kind"], "service");
    }

    #[test]
    fn miniapp_requests_reject_unknown_and_legacy_fields() {
        let error = serde_json::from_value::<BuildPluginRuntimeRequest>(json!({
            "plugin_id": "miniapp-1",
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
    fn surface_open_request_is_explicit_and_rejects_extra_authority() {
        let request = serde_json::to_value(OpenPluginRuntimeSurfaceRequest {
            miniapp_id: "miniapp-1".into(),
        })
        .unwrap();
        assert_eq!(request, json!({"plugin_id": "miniapp-1"}));

        let error = serde_json::from_value::<OpenPluginRuntimeSurfaceRequest>(json!({
            "plugin_id": "miniapp-1",
            "surface_capability": "caller-must-not-supply-capability"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn publish_and_rollback_bind_exact_release_pointers() {
        let publish = serde_json::to_value(PublishPluginRuntimeRequest {
            miniapp_id: "miniapp-1".into(),
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
            miniapp_id: "miniapp-1".into(),
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
    fn miniapp_credential_binding_wire_contains_references_only() {
        let request = ConfigurePluginRuntimeRequest {
            miniapp_id: "miniapp-1".into(),
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
