/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { PluginRuntimeId } from './ids';
import type { CapabilityCatalogItem } from './agentPlatform/contracts';

export type { PluginRuntimeId } from './ids';

export type PluginRuntimeKind = 'ui_only' | 'service';
export type PluginRuntimeServiceLifecycle = 'on_demand' | 'continuous';
export type PluginRuntimeLifecycle = 'enabled' | 'disabled' | 'trashed' | 'deleting';
export type PluginRuntimePublishMode = 'manual' | 'auto_ui_only';
export type PluginRuntimeProjectSourceState = 'empty' | 'editable' | 'runtime_only';
export type PluginRuntimeTestStatus =
  | 'not_required'
  | 'not_run'
  | 'passed'
  | 'failed'
  | 'needs_test_input'
  | 'stale';

export type PluginRuntimeServiceHealth =
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

export interface PluginRuntimeReleaseRef {
  release_id: string;
  artifact_id: string;
  release_digest: string;
  manifest_digest: string;
}

export interface PluginRuntimeReleasePointers {
  pointer_revision: number;
  active_release_epoch: number;
  ready?: PluginRuntimeReleaseRef;
  active?: PluginRuntimeReleaseRef;
  previous?: PluginRuntimeReleaseRef;
}

export interface PluginRuntimeSummary {
  plugin_id: PluginRuntimeId;
  product_revision: number;
  display_name: string;
  description?: string;
  icon_asset_id?: string;
  kind: PluginRuntimeKind;
  lifecycle: PluginRuntimeLifecycle;
  releases: PluginRuntimeReleasePointers;
  service_health: PluginRuntimeServiceHealth;
  surface_available: boolean;
  contribution_count?: number;
  updated_at_ms: number;
}

export interface PluginRuntimeLibraryResponse {
  library_revision: number;
  plugins: PluginRuntimeSummary[];
}

export interface PluginRuntimeServiceDescriptor {
  lifecycle: PluginRuntimeServiceLifecycle;
  uses_files: boolean;
  uses_private_database: boolean;
  service_contract_digest: string;
}

export interface PluginRuntimeReleaseTest {
  status: PluginRuntimeTestStatus;
  release_id: string;
  expected_release_digest: string;
  receipt_id?: string;
  expected_service_run_key?: string;
  issued_at_ms?: number;
  error_code?: string;
}

export interface PluginRuntimeReadyRelease {
  release: PluginRuntimeReleaseRef;
  project_build_generation: number;
  created_at_ms: number;
  kind: PluginRuntimeKind;
  service?: PluginRuntimeServiceDescriptor;
  test: PluginRuntimeReleaseTest;
  migration_count: number;
  can_publish: boolean;
  can_auto_publish: boolean;
  blocking_reasons: string[];
}

export type PluginRuntimeCapabilityContribution = CapabilityCatalogItem;

export interface PluginRuntimeConfigSchema {
  schema_digest: string;
  schema: unknown;
}

export interface PluginRuntimeConfigState {
  config_revision: number;
  schema_digest: string;
  values: unknown;
  valid: boolean;
  validation_errors: string[];
}

export type PluginRuntimeCredentialBindingStatus =
  | 'unbound'
  | 'bound'
  | 'missing'
  | 'invalid';

export interface PluginRuntimeCredentialSlotBinding {
  slot_key: string;
  display_name: string;
  required: boolean;
  status: PluginRuntimeCredentialBindingStatus;
  credential_id?: string;
}

export type PluginRuntimeOperationKind =
  | 'build'
  | 'import'
  | 'export'
  | 'plugin_permanent_delete';
export type PluginRuntimeOperationState =
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'canceled';
export type PluginRuntimeOperationOwner =
  | { owner: 'plugin_project'; project_id: string }
  | { owner: 'plugin_mount'; mount_id: string }
  | { owner: 'plugin_runtime'; plugin_id: PluginRuntimeId };

export interface PluginRuntimeOperationSummary {
  operation_id: string;
  operation_revision: number;
  kind: PluginRuntimeOperationKind;
  owner: PluginRuntimeOperationOwner;
  state: PluginRuntimeOperationState;
  cancelable: boolean;
  progress_percent?: number;
  started_at_ms: number;
  completed_at_ms?: number;
}

export interface PluginRuntimeWorkshop {
  plugin: PluginRuntimeSummary;
  service_lifecycle?: PluginRuntimeServiceLifecycle;
  active_service?: PluginRuntimeServiceDescriptor;
  project_id: string;
  project_revision: number;
  publish_mode: PluginRuntimePublishMode;
  source_state: PluginRuntimeProjectSourceState;
  build_generation: number;
  source_snapshot_digest?: string;
  dependency_lock_digest?: string;
  ready?: PluginRuntimeReadyRelease;
  config_schema: PluginRuntimeConfigSchema;
  config: PluginRuntimeConfigState;
  credential_bindings_revision: number;
  credential_slots: PluginRuntimeCredentialSlotBinding[];
  capabilities: PluginRuntimeCapabilityContribution[];
  active_operation?: PluginRuntimeOperationSummary;
}

export interface CreatePluginRuntimeProjectRequest {
  expected_library_revision: number;
  display_name: string;
  description?: string;
  kind: PluginRuntimeKind;
}

export interface PluginRuntimeSourceFile {
  plugin_id: PluginRuntimeId;
  project_id: string;
  path: string;
  content: string;
  source_snapshot_digest: string;
  build_generation: number;
}

export interface GetPluginRuntimeSourceFileRequest {
  plugin_id: PluginRuntimeId;
  path: string;
}

export interface ReplacePluginRuntimeSourceFileRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  project_id: string;
  expected_project_revision: number;
  expected_build_generation: number;
  expected_source_snapshot_digest: string;
  path: string;
  content: string;
}

export interface BuildPluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  project_id: string;
  expected_project_revision: number;
  expected_build_generation: number;
  expected_source_snapshot_digest: string;
  expected_dependency_lock_digest: string;
  service_lifecycle?: PluginRuntimeServiceLifecycle;
}

export interface TestPluginRuntimeReleaseRequest {
  plugin_id: PluginRuntimeId;
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

export type PluginRuntimeShareContent = 'ready_release' | 'active_release';

export interface SharePluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  content: PluginRuntimeShareContent;
  release_id: string;
  expected_release_digest: string;
  destination_path: string;
  include_source: boolean;
}

export interface ImportPluginRuntimeShareRequest {
  expected_library_revision: number;
  source_path: string;
  expected_bundle_digest: string;
  expected_release_digest: string;
  display_name: string;
}

export interface ImportPluginRuntimeArtifactRequest {
  expected_library_revision: number;
  source_path: string;
  expected_artifact_digest: string;
  display_name: string;
}

export interface ExportPluginRuntimeBackupRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_lifecycle: 'disabled';
  expected_pointer_revision: number;
  expected_config_revision: number;
  expected_credential_bindings_revision: number;
  destination_path: string;
}

export interface ImportPluginRuntimeBackupRequest {
  expected_library_revision: number;
  source_path: string;
  expected_backup_metadata_digest: string;
  display_name: string;
}

export interface CancelPluginRuntimeBuildRequest {
  plugin_id: PluginRuntimeId;
  operation_id: string;
  expected_operation_revision: number;
}

export interface PublishPluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  ready_release_id: string;
  expected_ready_release_digest: string;
  expected_active_release_digest?: string;
  expected_service_test_receipt_id?: string;
  acknowledge_test_warning: boolean;
}

export interface RollbackPluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  expected_current_release_digest: string;
  previous_release_id: string;
  expected_previous_release_digest: string;
}

export interface SetPluginRuntimeEnabledRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_digest?: string;
  enabled: boolean;
}

export interface SetPluginRuntimePublishModeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  mode: PluginRuntimePublishMode;
}

export interface SetPluginRuntimeServiceRunningRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  expected_active_release_digest: string;
  running: boolean;
}

export interface RetryPluginRuntimeDeleteRequest {
  plugin_id: PluginRuntimeId;
  failed_operation_id: string;
  expected_operation_revision: number;
}

export interface TrashPluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_digest?: string;
}

export interface RestorePluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_lifecycle: 'trashed';
  expected_pointer_revision: number;
}

export interface DeletePluginRuntimeRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_lifecycle: 'trashed';
  expected_pointer_revision: number;
  expected_active_release_digest?: string;
}

export interface RetryPluginRuntimeServiceRequest {
  plugin_id: PluginRuntimeId;
  expected_product_revision: number;
  expected_pointer_revision: number;
  expected_active_release_epoch: number;
  expected_active_release_digest: string;
}

export interface OpenPluginRuntimeSurfaceRequest {
  plugin_id: PluginRuntimeId;
}

export interface PluginRuntimeSurfaceLaunchDescriptor {
  plugin_id: PluginRuntimeId;
  product_revision: number;
  release_id: string;
  expected_release_digest: string;
  active_release_epoch: number;
  surface_session_id: string;
  surface_generation: number;
  surface_capability: string;
  ui_entrypoint: string;
  kind: PluginRuntimeKind;
}

export interface ClosePluginRuntimeSurfaceRequest {
  plugin_id: PluginRuntimeId;
  surface_session_id: string;
  surface_capability: string;
}

export type PluginRuntimeBridgeKvRequest =
  | { operation: 'get'; key: string }
  | { operation: 'set'; key: string; value: unknown }
  | { operation: 'delete'; key: string }
  | {
      operation: 'compare_and_swap';
      key: string;
      expected_revision?: number;
      value?: unknown;
    };

export interface PluginRuntimeBridgeRequest {
  call_id: string;
  target:
    | {
        target: 'host_kv';
        request: PluginRuntimeBridgeKvRequest;
      }
    | {
        target: 'service';
        method: string;
        payload: Record<string, unknown>;
      };
}

export type PluginRuntimeKvResponse =
  | { outcome: 'value'; value?: unknown; revision?: number }
  | { outcome: 'written'; revision: number }
  | { outcome: 'deleted'; existed: boolean }
  | {
      outcome: 'compare_and_swap';
      applied: boolean;
      current_revision?: number;
    };

export interface PluginRuntimeSurfaceBridgeRequest {
  plugin_id: PluginRuntimeId;
  surface_capability: string;
  active_release_epoch: number;
  expected_release_digest: string;
  request: PluginRuntimeBridgeRequest;
}
