//! Product-facing DTOs for the Phase M1 MiniApp Platform.
//!
//! Surface launch data contains only Host-consumable descriptors. MessageChannel
//! Bridge sessions and internal Service/storage handles remain private.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CredentialSlotBindingDto, DurableOperationSummaryDto, PluginCapabilityContributionDto,
    PluginConfigSchemaDto, PluginConfigStateDto,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppKindDto {
    UiOnly,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppServiceLifecycleDto {
    OnDemand,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppLifecycleDto {
    Enabled,
    Disabled,
    Trashed,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum MiniAppServiceHealthDto {
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
pub struct MiniAppReleaseRefDto {
    pub release_id: String,
    pub artifact_id: String,
    pub release_digest: String,
    pub manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReleasePointersDto {
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<MiniAppReleaseRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<MiniAppReleaseRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<MiniAppReleaseRefDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppSummaryDto {
    pub miniapp_id: String,
    pub product_revision: u64,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_asset_id: Option<String>,
    pub kind: MiniAppKindDto,
    pub lifecycle: MiniAppLifecycleDto,
    pub releases: MiniAppReleasePointersDto,
    pub service_health: MiniAppServiceHealthDto,
    pub surface_available: bool,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppLibraryResponseDto {
    pub library_revision: u64,
    pub miniapps: Vec<MiniAppSummaryDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppProjectSourceStateDto {
    Empty,
    Editable,
    RuntimeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppServiceDescriptorDto {
    pub lifecycle: MiniAppServiceLifecycleDto,
    pub uses_files: bool,
    pub uses_private_database: bool,
    pub service_contract_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppTestStatusDto {
    NotRequired,
    NotRun,
    Passed,
    Failed,
    NeedsTestInput,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReleaseTestDto {
    pub status: MiniAppTestStatusDto,
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
pub struct MiniAppReadyReleaseDto {
    pub release: MiniAppReleaseRefDto,
    pub project_build_generation: u64,
    pub created_at_ms: i64,
    pub kind: MiniAppKindDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<MiniAppServiceDescriptorDto>,
    pub test: MiniAppReleaseTestDto,
    pub migration_count: u32,
    pub can_publish: bool,
    pub can_auto_publish: bool,
    #[serde(default)]
    pub blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppWorkshopDto {
    pub miniapp: MiniAppSummaryDto,
    pub project_id: String,
    pub project_revision: u64,
    pub source_state: MiniAppProjectSourceStateDto,
    pub build_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_lock_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<MiniAppReadyReleaseDto>,
    pub config_schema: PluginConfigSchemaDto,
    pub config: PluginConfigStateDto,
    pub credential_bindings_revision: u64,
    pub credential_slots: Vec<CredentialSlotBindingDto>,
    pub capabilities: Vec<PluginCapabilityContributionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_operation: Option<DurableOperationSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MiniAppSurfaceLaunchDescriptorDto {
    pub miniapp_id: String,
    pub product_revision: u64,
    pub release_id: String,
    pub expected_release_digest: String,
    pub active_release_epoch: u64,
    pub ui_asset_id: String,
    pub ui_entrypoint: String,
    pub kind: MiniAppKindDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateMiniAppProjectRequest {
    pub expected_library_revision: u64,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: MiniAppKindDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildMiniAppRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub project_id: String,
    pub expected_project_revision: u64,
    pub expected_build_generation: u64,
    pub expected_source_snapshot_digest: String,
    pub expected_dependency_lock_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelMiniAppBuildRequest {
    pub expected_operation_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestMiniAppReleaseRequest {
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
pub struct PublishMiniAppRequest {
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
pub enum MiniAppPublishModeDto {
    Manual,
    AutoUiOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetMiniAppPublishModeRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub mode: MiniAppPublishModeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardMiniAppReadyReleaseRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub ready_release_id: String,
    pub expected_ready_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackMiniAppRequest {
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
pub struct SetMiniAppEnabledRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashMiniAppRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreMiniAppRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: MiniAppLifecycleDto,
    pub expected_pointer_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryMiniAppServiceRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_active_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetMiniAppServiceRunningRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub expected_active_release_epoch: u64,
    pub expected_active_release_digest: String,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteMiniAppRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: MiniAppLifecycleDto,
    pub expected_pointer_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_active_release_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppShareContentDto {
    ReadyRelease,
    ActiveRelease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareMiniAppRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_pointer_revision: u64,
    pub content: MiniAppShareContentDto,
    pub release_id: String,
    pub expected_release_digest: String,
    pub destination_path: String,
    pub include_source: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportMiniAppShareRequest {
    pub expected_library_revision: u64,
    pub source_path: String,
    pub expected_bundle_digest: String,
    pub expected_release_digest: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportMiniAppArtifactRequest {
    pub expected_library_revision: u64,
    pub source_path: String,
    pub expected_artifact_digest: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportMiniAppBackupRequest {
    pub miniapp_id: String,
    pub expected_product_revision: u64,
    pub expected_lifecycle: MiniAppLifecycleDto,
    pub expected_pointer_revision: u64,
    pub expected_config_revision: u64,
    pub expected_credential_bindings_revision: u64,
    pub destination_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportMiniAppBackupRequest {
    pub expected_library_revision: u64,
    pub source_path: String,
    pub expected_backup_metadata_digest: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureMiniAppRequest {
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
        let value = serde_json::to_value(MiniAppSurfaceLaunchDescriptorDto {
            miniapp_id: "miniapp-1".into(),
            product_revision: 3,
            release_id: "release-2".into(),
            expected_release_digest: "a".repeat(64),
            active_release_epoch: 8,
            ui_asset_id: "artifact-2".into(),
            ui_entrypoint: "ui/index.html".into(),
            kind: MiniAppKindDto::Service,
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
        let error = serde_json::from_value::<BuildMiniAppRequest>(json!({
            "miniapp_id": "miniapp-1",
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
    fn publish_and_rollback_bind_exact_release_pointers() {
        let publish = serde_json::to_value(PublishMiniAppRequest {
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
        let rollback = serde_json::to_value(RollbackMiniAppRequest {
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
        let request = ConfigureMiniAppRequest {
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
