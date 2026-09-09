/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  PluginArtifactId,
  PluginCandidateId,
  PluginMountId,
  PluginOperationId,
  PluginProjectId,
} from './ids';

export type {
  PluginArtifactId,
  PluginCandidateId,
  PluginMountId,
  PluginOperationId,
  PluginProjectId,
} from './ids';

export type PluginLifecycle =
  | 'enabled'
  | 'disabled'
  | 'uninstalled_data_retained'
  | 'delete_pending'
  | 'error';

export type PluginProjectSourceState = 'empty' | 'editable' | 'runtime_only';
export type PluginApplyMode = 'ask_before_apply' | 'auto_compatible_when_idle';
export type PluginCandidateOrigin = 'build' | 'import';
export type PluginCompatibility = 'compatible' | 'breaking' | 'unknown';
export type PluginCandidateTestStatus =
  | 'not_run'
  | 'passed'
  | 'failed'
  | 'needs_test_input'
  | 'stale';
export type PluginConsumerSurface =
  | 'agent'
  | 'gateway'
  | 'knowledge'
  | 'remote'
  | 'automation'
  | 'ui'
  | 'miniapp_service';
export type PluginConsumerAvailabilityStatus =
  | 'active'
  | 'disabled'
  | 'unavailable'
  | 'needs_runtime'
  | 'contract_mismatch';
export type PluginCredentialBindingStatus = 'unbound' | 'bound' | 'missing' | 'invalid';

export interface PluginTargetRef {
  package_id: string;
  package_version: string;
  artifact_id: PluginArtifactId;
  artifact_digest: string;
  manifest_digest: string;
}

export interface PluginCandidateRef {
  candidate_id: PluginCandidateId;
  candidate_digest: string;
}

export interface PluginSummary {
  mount_id: PluginMountId;
  mount_revision: number;
  display_name: string;
  description?: string;
  lifecycle: PluginLifecycle;
  current?: PluginTargetRef;
  previous?: PluginTargetRef;
  linked_project_id?: PluginProjectId;
  contribution_count: number;
  updated_at_ms: number;
}

export interface PluginProjectSummary {
  project_id: PluginProjectId;
  project_revision: number;
  display_name: string;
  description?: string;
  linked_mount_id?: PluginMountId;
  source_state: PluginProjectSourceState;
  build_generation: number;
  ready_candidate?: PluginCandidateRef;
  apply_mode: PluginApplyMode;
  updated_at_ms: number;
}

export interface PluginLibraryResponse {
  library_revision: number;
  plugins: PluginSummary[];
  projects: PluginProjectSummary[];
}

export interface PluginConsumerAvailability {
  surface: PluginConsumerSurface;
  status: PluginConsumerAvailabilityStatus;
  reason_code?: string;
}

export interface PluginContributionProvenance {
  mount_id: PluginMountId;
  mount_revision: number;
  artifact_id: PluginArtifactId;
  artifact_digest: string;
  manifest_digest: string;
  contribution_id: string;
  contract_digest: string;
}

export interface PluginCapabilityContribution {
  capability_id: string;
  capability_version: string;
  display_name: string;
  description?: string;
  provenance: PluginContributionProvenance;
  consumer_availability: PluginConsumerAvailability[];
}

export interface PluginConfigSchema {
  schema_digest: string;
  schema: unknown;
}

export type PluginConfigValues = Record<string, unknown>;

export interface PluginConfigState {
  config_revision: number;
  schema_digest: string;
  values: PluginConfigValues;
  valid: boolean;
  validation_errors: string[];
}

export interface PluginCredentialSlotBinding {
  slot_key: string;
  display_name: string;
  required: boolean;
  status: PluginCredentialBindingStatus;
  credential_id?: string;
}

export interface PluginCandidateTest {
  status: PluginCandidateTestStatus;
  receipt_id?: string;
  candidate_id: PluginCandidateId;
  candidate_digest: string;
  runtime?: {
    runtime_installation_id: string;
    node_version: string;
    runtime_target: string;
    executable_digest: string;
  };
  resolved_test_input_digest?: string;
  issued_at_ms?: number;
  error_code?: string;
}

export interface PluginAffectedConsumer {
  surface: PluginConsumerSurface;
  consumer_id: string;
  contribution_id: string;
  expected_contract_digest: string;
}

export interface PluginCandidateImpact {
  compatibility: PluginCompatibility;
  changed_contracts: string[];
  affected_consumers: PluginAffectedConsumer[];
  can_apply: boolean;
  can_auto_apply: boolean;
  blocking_reasons: string[];
}

export interface PluginReadyCandidate {
  candidate: PluginCandidateRef;
  origin: PluginCandidateOrigin;
  target: PluginTargetRef;
  project_build_generation: number;
  base_target_digest?: string;
  test: PluginCandidateTest;
  impact: PluginCandidateImpact;
}

export interface PluginDetail {
  summary: PluginSummary;
  capabilities: PluginCapabilityContribution[];
  config_schema: PluginConfigSchema;
  config: PluginConfigState;
  credential_bindings_revision: number;
  credential_slots: PluginCredentialSlotBinding[];
  retained_data: boolean;
  last_error_code?: string;
}

export type DurablePluginOperationKind =
  | 'build'
  | 'import'
  | 'export'
  | 'miniapp_permanent_delete';
export type DurablePluginOperationState = 'running' | 'succeeded' | 'failed' | 'canceled';
export type DurablePluginOperationOwner =
  | { owner: 'plugin_project'; project_id: PluginProjectId }
  | { owner: 'plugin_mount'; mount_id: PluginMountId }
  | { owner: 'miniapp'; miniapp_id: string };

export interface DurablePluginOperationSummary {
  operation_id: PluginOperationId;
  operation_revision: number;
  kind: DurablePluginOperationKind;
  owner: DurablePluginOperationOwner;
  state: DurablePluginOperationState;
  cancelable: boolean;
  progress_percent?: number;
  started_at_ms: number;
  completed_at_ms?: number;
}

export interface DurablePluginOperationDetail {
  summary: DurablePluginOperationSummary;
  bounded_log_tail: string[];
  result_artifact_digests: Record<string, string>;
  last_error_code?: string;
}

export interface PluginProjectDetail {
  summary: PluginProjectSummary;
  source_snapshot_digest?: string;
  dependency_lock_digest?: string;
  ready?: PluginReadyCandidate;
  active_operation?: DurablePluginOperationSummary;
}

export type PluginSourceFileEdit =
  | { kind: 'replace'; path: string; content: string }
  | { kind: 'delete'; path: string };

export interface ApplyPluginSourceEditRequest {
  project_id: PluginProjectId;
  expected_source_snapshot_digest: string;
  edit: PluginSourceFileEdit;
}

export interface CreatePluginProjectRequest {
  expected_library_revision: number;
  package_id: string;
  package_version: string;
  display_name: string;
  description: string;
  language: 'java_script' | 'type_script';
  linked_mount_id?: PluginMountId;
  expected_linked_mount_revision?: number;
  expected_linked_target_digest?: string;
}

export interface ImportPluginRequest {
  expected_library_revision: number;
  import_kind: 'prebuilt_artifact' | 'share_bundle' | 'source_bundle';
  source_path: string;
  expected_bundle_or_artifact_digest: string;
  target_project_id?: PluginProjectId;
  expected_project_revision?: number;
}

export interface BuildPluginProjectRequest {
  project_id: PluginProjectId;
  expected_project_revision: number;
  expected_build_generation: number;
  expected_source_snapshot_digest: string;
  expected_dependency_lock_digest: string;
}

export interface TestPluginCandidateRequest {
  project_id: PluginProjectId;
  expected_project_revision: number;
  expected_build_generation: number;
  candidate_id: PluginCandidateId;
  expected_candidate_digest: string;
  expected_config_revision: number;
  expected_credential_bindings_revision: number;
  resolved_test_input_digest: string;
}

export type ApplyPluginTarget =
  | {
      target: 'initial_install';
      expected_library_revision: number;
    }
  | {
      target: 'existing_mount';
      mount_id: PluginMountId;
      expected_mount_revision: number;
      expected_current_target_digest: string;
    };

export interface ApplyPluginCandidateRequest {
  project_id: PluginProjectId;
  expected_project_revision: number;
  expected_build_generation: number;
  candidate_id: PluginCandidateId;
  expected_candidate_digest: string;
  target: ApplyPluginTarget;
  allow_breaking: boolean;
  acknowledge_test_warning: boolean;
}

export interface DeletePluginProjectRequest {
  project_id: PluginProjectId;
  expected_project_revision: number;
  expected_build_generation: number;
  expected_ready_candidate_id?: PluginCandidateId;
  expected_ready_candidate_digest?: string;
}

export interface ConfigurePluginRequest {
  mount_id: PluginMountId;
  expected_mount_revision: number;
  expected_current_target_digest: string;
  expected_config_revision: number;
  expected_schema_digest: string;
  values: PluginConfigValues;
  credential_bindings: Record<string, string | null>;
  expected_credential_bindings_revision: number;
}

export interface SetPluginEnabledRequest {
  mount_id: PluginMountId;
  expected_mount_revision: number;
  expected_current_target_digest: string;
  enabled: boolean;
}

export interface RetryPluginRequest {
  mount_id: PluginMountId;
  expected_mount_revision: number;
  expected_current_target_digest: string;
}

export interface RestorePluginPreviousRequest {
  mount_id: PluginMountId;
  expected_mount_revision: number;
  expected_current_target_digest: string;
  expected_previous_target_digest: string;
}

export interface UninstallPluginRequest {
  mount_id: PluginMountId;
  expected_mount_revision: number;
  expected_current_target_digest: string;
}

export interface DeletePluginDataRequest {
  mount_id: PluginMountId;
  expected_mount_revision: number;
  expected_lifecycle: 'uninstalled_data_retained';
  expected_data_revision: number;
}

export interface CancelPluginOperationRequest {
  operation_id: PluginOperationId;
  expected_operation_revision: number;
}
