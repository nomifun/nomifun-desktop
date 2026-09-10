/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { MiniAppId } from './ids';
import type { CapabilityCatalogItem } from './agentPlatform/contracts';

export type { MiniAppId } from './ids';

export type MiniAppKind = 'ui_only' | 'service';
export type MiniAppServiceLifecycle = 'on_demand' | 'continuous';
export type MiniAppLifecycle = 'enabled' | 'disabled' | 'trashed' | 'deleting';
export type MiniAppPublishMode = 'manual' | 'auto_ui_only';
export type MiniAppProjectSourceState = 'empty' | 'editable' | 'runtime_only';
export type MiniAppTestStatus =
  | 'not_required'
  | 'not_run'
  | 'passed'
  | 'failed'
  | 'needs_test_input'
  | 'stale';

export type MiniAppServiceHealth =
  | { state: 'not_applicable' }
  | { state: 'stopped' }
  | {
      state: 'starting';
      release_id: string;
      expected_release_digest: string;
    }
  | {
      state: 'ready';
      release_id: string;
      expected_release_digest: string;
      started_at_ms: number;
    }
  | {
      state: 'failed';
      release_id: string;
      expected_release_digest: string;
      error_code: string;
    };

export interface MiniAppReleaseRef {
  release_id: string;
  artifact_id: string;
  release_digest: string;
  manifest_digest: string;
}

export interface MiniAppReleasePointers {
  pointer_revision: number;
  active_release_epoch: number;
  ready?: MiniAppReleaseRef;
  active?: MiniAppReleaseRef;
  previous?: MiniAppReleaseRef;
}

export interface MiniAppSummary {
  miniapp_id: MiniAppId;
  product_revision: number;
  display_name: string;
  description?: string;
  icon_asset_id?: string;
  kind: MiniAppKind;
  lifecycle: MiniAppLifecycle;
  releases: MiniAppReleasePointers;
  service_health: MiniAppServiceHealth;
  surface_available: boolean;
  updated_at_ms: number;
}

export interface MiniAppLibraryResponse {
  library_revision: number;
  miniapps: MiniAppSummary[];
}

export interface MiniAppServiceDescriptor {
  lifecycle: MiniAppServiceLifecycle;
  uses_files: boolean;
  uses_private_database: boolean;
  service_contract_digest: string;
}

export interface MiniAppReleaseTest {
  status: MiniAppTestStatus;
  release_id: string;
  expected_release_digest: string;
  receipt_id?: string;
  expected_service_run_key?: string;
  issued_at_ms?: number;
  error_code?: string;
}

export interface MiniAppReadyRelease {
  release: MiniAppReleaseRef;
  project_build_generation: number;
  created_at_ms: number;
  kind: MiniAppKind;
  service?: MiniAppServiceDescriptor;
  test: MiniAppReleaseTest;
  migration_count: number;
  can_publish: boolean;
  can_auto_publish: boolean;
  blocking_reasons: string[];
}

export type MiniAppCapabilityContribution = CapabilityCatalogItem;

export interface MiniAppConfigSchema {
  schema_digest: string;
  schema: unknown;
}

export interface MiniAppConfigState {
  config_revision: number;
  schema_digest: string;
  values: unknown;
  valid: boolean;
  validation_errors: string[];
}

export type MiniAppCredentialBindingStatus =
  | 'unbound'
  | 'bound'
  | 'missing'
  | 'invalid';

export interface MiniAppCredentialSlotBinding {
  slot_key: string;
  display_name: string;
  required: boolean;
  status: MiniAppCredentialBindingStatus;
  credential_id?: string;
}

export type MiniAppOperationKind =
  | 'build'
  | 'import'
  | 'export'
  | 'miniapp_permanent_delete';
export type MiniAppOperationState =
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'canceled';
export type MiniAppOperationOwner =
  | { owner: 'plugin_project'; project_id: string }
  | { owner: 'plugin_mount'; mount_id: string }
  | { owner: 'miniapp'; miniapp_id: MiniAppId };

export interface MiniAppOperationSummary {
  operation_id: string;
  operation_revision: number;
  kind: MiniAppOperationKind;
  owner: MiniAppOperationOwner;
  state: MiniAppOperationState;
  cancelable: boolean;
  progress_percent?: number;
  started_at_ms: number;
  completed_at_ms?: number;
}

export interface MiniAppWorkshop {
  miniapp: MiniAppSummary;
  service_lifecycle?: MiniAppServiceLifecycle;
  active_service?: MiniAppServiceDescriptor;
  project_id: string;
  project_revision: number;
  publish_mode: MiniAppPublishMode;
  source_state: MiniAppProjectSourceState;
  build_generation: number;
  source_snapshot_digest?: string;
  dependency_lock_digest?: string;
  ready?: MiniAppReadyRelease;
  config_schema: MiniAppConfigSchema;
  config: MiniAppConfigState;
  credential_bindings_revision: number;
  credential_slots: MiniAppCredentialSlotBinding[];
  capabilities: MiniAppCapabilityContribution[];
  active_operation?: MiniAppOperationSummary;
}

export interface CreateMiniAppProjectRequest {
  expected_library_revision: number;
  display_name: string;
  description?: string;
  kind: MiniAppKind;
}

export interface MiniAppSourceFile {
  miniapp_id: MiniAppId;
  project_id: string;
  path: string;
  content: string;
  source_snapshot_digest: string;
  build_generation: number;
}

export interface GetMiniAppSourceFileRequest {
  miniapp_id: MiniAppId;
  path: string;
}

export interface ReplaceMiniAppSourceFileRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  project_id: string;
  expected_project_revision: number;
  expected_build_generation: number;
  expected_source_snapshot_digest: string;
  path: string;
  content: string;
}

export interface BuildMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  project_id: string;
  expected_project_revision: number;
  expected_build_generation: number;
  expected_source_snapshot_digest: string;
  expected_dependency_lock_digest: string;
  service_lifecycle?: MiniAppServiceLifecycle;
}

export interface TestMiniAppReleaseRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  project_id: string;
  expected_project_revision: number;
  expected_build_generation: number;
  release_id: string;
  expected_release_digest: string;
  expected_config_revision: number;
  expected_credential_bindings_revision: number;
  resolved_test_input_digest: string;
}

export type MiniAppShareContent = 'ready_release' | 'active_release';

export interface ShareMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  content: MiniAppShareContent;
  release_id: string;
  expected_release_digest: string;
  destination_path: string;
  include_source: boolean;
}

export interface ImportMiniAppShareRequest {
  expected_library_revision: number;
  source_path: string;
  expected_bundle_digest: string;
  expected_release_digest: string;
  display_name: string;
}

export interface ImportMiniAppArtifactRequest {
  expected_library_revision: number;
  source_path: string;
  expected_artifact_digest: string;
  display_name: string;
}

export interface ExportMiniAppBackupRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_lifecycle: 'disabled';
  expected_pointer_revision: number;
  expected_config_revision: number;
  expected_credential_bindings_revision: number;
  destination_path: string;
}

export interface ImportMiniAppBackupRequest {
  expected_library_revision: number;
  source_path: string;
  expected_backup_metadata_digest: string;
  display_name: string;
}

export interface CancelMiniAppBuildRequest {
  miniapp_id: MiniAppId;
  operation_id: string;
  expected_operation_revision: number;
}

export interface PublishMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  ready_release_id: string;
  expected_ready_release_digest: string;
  expected_active_release_digest?: string;
  expected_service_test_receipt_id?: string;
  acknowledge_test_warning: boolean;
}

export interface RollbackMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  expected_current_release_digest: string;
  previous_release_id: string;
  expected_previous_release_digest: string;
}

export interface SetMiniAppEnabledRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_digest?: string;
  enabled: boolean;
}

export interface SetMiniAppPublishModeRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  mode: MiniAppPublishMode;
}

export interface SetMiniAppServiceRunningRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  expected_active_release_digest: string;
  running: boolean;
}

export interface RetryMiniAppDeleteRequest {
  miniapp_id: MiniAppId;
  failed_operation_id: string;
  expected_operation_revision: number;
}

export interface TrashMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_digest?: string;
}

export interface RestoreMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_lifecycle: 'trashed';
  expected_pointer_revision: number;
}

export interface DeleteMiniAppRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_lifecycle: 'trashed';
  expected_pointer_revision: number;
  expected_active_release_digest?: string;
}

export interface RetryMiniAppServiceRequest {
  miniapp_id: MiniAppId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  expected_active_release_digest: string;
}

export interface OpenMiniAppSurfaceRequest {
  miniapp_id: MiniAppId;
}

export interface MiniAppSurfaceLaunchDescriptor {
  miniapp_id: MiniAppId;
  product_revision: number;
  release_id: string;
  expected_release_digest: string;
  active_release_epoch: number;
  surface_session_id: string;
  surface_generation: number;
  surface_capability: string;
  ui_entrypoint: string;
  kind: MiniAppKind;
}

export interface CloseMiniAppSurfaceRequest {
  miniapp_id: MiniAppId;
  surface_session_id: string;
  surface_capability: string;
}

export type MiniAppBridgeKvRequest =
  | { operation: 'get'; key: string }
  | { operation: 'set'; key: string; value: unknown }
  | { operation: 'delete'; key: string }
  | {
      operation: 'compare_and_swap';
      key: string;
      expected_revision?: number;
      value?: unknown;
    };

export interface MiniAppBridgeRequest {
  call_id: string;
  target:
    | {
        target: 'host_kv';
        request: MiniAppBridgeKvRequest;
      }
    | {
        target: 'service';
        method: string;
        payload: Record<string, unknown>;
      };
}

export type MiniAppKvResponse =
  | { outcome: 'value'; value?: unknown; revision?: number }
  | { outcome: 'written'; revision: number }
  | { outcome: 'deleted'; existed: boolean }
  | {
      outcome: 'compare_and_swap';
      applied: boolean;
      current_revision?: number;
    };

export interface MiniAppSurfaceBridgeRequest {
  miniapp_id: MiniAppId;
  surface_capability: string;
  active_release_epoch: number;
  expected_release_digest: string;
  request: MiniAppBridgeRequest;
}
