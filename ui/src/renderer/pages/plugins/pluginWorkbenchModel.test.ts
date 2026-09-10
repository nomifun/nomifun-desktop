/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  BackendHttpError,
  BackendRequestError,
} from '@/common/adapter/httpBridge';
import {
  parsePluginArtifactId,
  parsePluginCandidateId,
  parsePluginMountId,
  parsePluginProjectId,
} from '@/common/types/ids';
import type { PluginDetail, PluginProjectDetail } from '@/common/types/pluginPlatform';
import {
  buildPluginProjectRequest,
  deletePluginDataRequest,
  deletePluginProjectRequest,
  pluginLoadFailure,
  pluginMountActions,
  projectDeleteAvailable,
  restorePluginRequest,
  setPluginAutoApplyRequest,
  setPluginEnabledRequest,
  testPluginCandidateRequest,
  uninstallPluginRequest,
} from './pluginWorkbenchModel';

const mountId = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000011');
const artifactId = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000012');
const projectId = parsePluginProjectId('0190f5fe-7c00-7a00-8000-000000000013');
const candidateId = parsePluginCandidateId('0190f5fe-7c00-7a00-8000-000000000014');

const detail = (lifecycle: PluginDetail['summary']['lifecycle']): PluginDetail => ({
  summary: {
    mount_id: mountId,
    mount_revision: 9,
    display_name: 'Example',
    lifecycle,
    current:
      lifecycle === 'uninstalled_data_retained'
        ? undefined
        : {
            package_id: 'com.nomifun.example',
            package_version: '2.0.0',
            artifact_id: artifactId,
            artifact_digest: 'a'.repeat(64),
            manifest_digest: 'b'.repeat(64),
          },
    previous:
      lifecycle === 'uninstalled_data_retained'
        ? undefined
        : {
            package_id: 'com.nomifun.example',
            package_version: '1.0.0',
            artifact_id: artifactId,
            artifact_digest: 'c'.repeat(64),
            manifest_digest: 'd'.repeat(64),
          },
    contribution_count: 1,
    updated_at_ms: 1,
  },
  capabilities: [],
  config_schema: { schema_digest: 'e'.repeat(64), schema: {} },
  config: {
    config_revision: 2,
    schema_digest: 'e'.repeat(64),
    values: {},
    valid: true,
    validation_errors: [],
  },
  credential_bindings_revision: 3,
  credential_slots: [],
  retained_data: lifecycle === 'uninstalled_data_retained',
});

describe('Plugin Workbench lifecycle model', () => {
  test('builds lifecycle requests from the exact visible Mount revision and digests', () => {
    const enabled = detail('enabled');
    expect(setPluginEnabledRequest(enabled, false)).toEqual({
      mount_id: mountId,
      expected_mount_revision: 9,
      expected_current_target_digest: 'a'.repeat(64),
      enabled: false,
    });
    expect(restorePluginRequest(enabled)).toEqual({
      mount_id: mountId,
      expected_mount_revision: 9,
      expected_current_target_digest: 'a'.repeat(64),
      expected_previous_target_digest: 'c'.repeat(64),
    });
    expect(uninstallPluginRequest(enabled)).toEqual({
      mount_id: mountId,
      expected_mount_revision: 9,
      expected_current_target_digest: 'a'.repeat(64),
    });
  });

  test('only permits permanent data deletion for an exact retained Mount', () => {
    const retained = detail('uninstalled_data_retained');
    expect(pluginMountActions(retained).canDeleteData).toBe(true);
    expect(deletePluginDataRequest(retained)).toEqual({
      mount_id: mountId,
      expected_mount_revision: 9,
      expected_lifecycle: 'uninstalled_data_retained',
      expected_data_revision: 9,
    });
    expect(pluginMountActions(detail('enabled')).canDeleteData).toBe(false);
  });

  test('exposes only lifecycle actions valid for each product state', () => {
    expect(pluginMountActions(detail('enabled'))).toEqual({
      canToggleEnabled: true,
      canRetry: false,
      canRestore: true,
      canUninstall: true,
      canDeleteData: false,
    });
    expect(pluginMountActions(detail('error'))).toEqual({
      canToggleEnabled: false,
      canRetry: true,
      canRestore: true,
      canUninstall: true,
      canDeleteData: false,
    });
  });

  test('distinguishes an unavailable platform from an ordinary resource error', () => {
    const unavailableHttp = new BackendHttpError({
      method: 'GET',
      path: '/api/plugins',
      status: 503,
      body: { error: 'Plugin Platform unavailable', code: 'PLUGIN_INTEGRATION' },
    });
    const network = new BackendRequestError(
      'network',
      'Plugin Platform is unreachable'
    );
    expect(pluginLoadFailure(unavailableHttp, 'platform').kind).toBe('unavailable');
    expect(pluginLoadFailure(network, 'platform').kind).toBe('unavailable');
    expect(
      pluginLoadFailure(new Error('selected Mount disappeared'), 'resource').kind
    ).toBe('error');
  });

  test('builds exact project build, test, and delete CAS requests', () => {
    const project = {
      summary: {
        project_id: projectId,
        project_revision: 1,
        display_name: 'Example',
        linked_mount_id: mountId,
        source_state: 'editable',
        build_generation: 1,
        apply_mode: 'ask_before_apply',
        auto_apply_authorization_revision: 0,
        updated_at_ms: 1,
      },
      source_snapshot_digest: 'f'.repeat(64),
      dependency_lock_digest: '1'.repeat(64),
      direct_dependencies: {},
      ready: {
        candidate: {
          candidate_id: candidateId,
          candidate_digest: '2'.repeat(64),
        },
        origin: 'build',
        target: {
          package_id: 'com.nomifun.example',
          package_version: '1.0.0',
          artifact_id: artifactId,
          artifact_digest: '3'.repeat(64),
          manifest_digest: '4'.repeat(64),
        },
        project_build_generation: 1,
        test: {
          status: 'not_run',
          candidate_id: candidateId,
          candidate_digest: '2'.repeat(64),
        },
        impact: {
          compatibility: 'compatible',
          changed_contracts: [],
          affected_consumers: [],
          can_apply: true,
          can_auto_apply: false,
          blocking_reasons: [],
        },
      },
    } satisfies PluginProjectDetail;
    expect(buildPluginProjectRequest(project)).toEqual({
      project_id: projectId,
      expected_project_revision: 1,
      expected_build_generation: 1,
      expected_source_snapshot_digest: 'f'.repeat(64),
      expected_dependency_lock_digest: '1'.repeat(64),
    });
    expect(testPluginCandidateRequest(project, 2, 3, 'a'.repeat(64))).toMatchObject({
      project_id: projectId,
      expected_config_revision: 2,
      expected_credential_bindings_revision: 3,
      resolved_test_input_digest: 'a'.repeat(64),
    });
    expect(deletePluginProjectRequest(project)).toMatchObject({
      project_id: projectId,
      expected_project_revision: 1,
      expected_build_generation: 1,
      expected_ready_candidate_digest: '2'.repeat(64),
    });
    expect(projectDeleteAvailable(project)).toBe(true);
    expect(setPluginAutoApplyRequest(project, true, detail('enabled').summary)).toEqual({
      project_id: projectId,
      expected_project_revision: 1,
      expected_build_generation: 1,
      linked_mount_id: mountId,
      expected_linked_mount_revision: 9,
      expected_linked_target_digest: 'a'.repeat(64),
      apply_mode: 'auto_compatible_when_idle',
    });
    expect(setPluginAutoApplyRequest(project, false)).toEqual({
      project_id: projectId,
      expected_project_revision: 1,
      expected_build_generation: 1,
      apply_mode: 'ask_before_apply',
    });
  });
});
