/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { MiniAppId } from './ids';

export type { MiniAppId } from './ids';

export type MiniAppKind = 'ui_only' | 'service';
export type MiniAppServiceLifecycle = 'on_demand' | 'continuous';
export type MiniAppLifecycle = 'enabled' | 'disabled' | 'trashed' | 'deleting';
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
  kind: MiniAppKind;
  service?: MiniAppServiceDescriptor;
  test: MiniAppReleaseTest;
  migration_count: number;
  can_publish: boolean;
  can_auto_publish: boolean;
  blocking_reasons: string[];
}

export type MiniAppConsumerSurface =
  | 'agent'
  | 'gateway'
  | 'knowledge'
  | 'remote'
  | 'automation'
  | 'ui'
  | 'miniapp_service';
export type MiniAppConsumerAvailabilityStatus =
  | 'active'
  | 'disabled'
  | 'unavailable'
  | 'needs_runtime'
  | 'contract_mismatch';

export interface MiniAppConsumerAvailability {
  surface: MiniAppConsumerSurface;
  status: MiniAppConsumerAvailabilityStatus;
  reason_code?: string;
}

export interface MiniAppCapabilityContribution {
  capability_id: string;
  capability_version: string;
  display_name: string;
  description?: string;
  provenance: {
    mount_id: string;
    mount_revision: number;
    artifact_id: string;
    artifact_digest: string;
    manifest_digest: string;
    contribution_id: string;
    contract_digest: string;
  };
  consumer_availability: MiniAppConsumerAvailability[];
}

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
  project_id: string;
  project_revision: number;
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
