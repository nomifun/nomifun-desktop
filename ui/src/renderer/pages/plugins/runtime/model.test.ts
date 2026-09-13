/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { PluginRuntimeWorkshop } from '@/common/types/pluginRuntimePlatform';
import {
  EMPTY_MINIAPP_TEST_INPUT_DIGEST,
  pluginRuntimeBuildRequest,
  pluginRuntimeCanOpenSurface,
  pluginRuntimePublishRequest,
  pluginRuntimeDeleteRequest,
  pluginRuntimeRestoreRequest,
  pluginRuntimeRetryDeleteRequest,
  pluginRuntimeTrashRequest,
  pluginRuntimeRetryServiceRequest,
  pluginRuntimeReleaseStage,
  pluginRuntimeRollbackRequest,
  pluginRuntimeSetEnabledRequest,
  pluginRuntimeSetServiceRunningRequest,
  pluginRuntimeSetPublishModeRequest,
  pluginRuntimeShareRequest,
  pluginRuntimeSurfaceAssetPath,
  pluginRuntimeTestRequest,
  pluginRuntimeWorkflowState,
  shortPluginRuntimeIdentity,
} from './model';

const workshop = (overrides: Partial<PluginRuntimeWorkshop> = {}): PluginRuntimeWorkshop =>
  ({
    plugin: {
      plugin_id: '0190f5fe-7c00-7a00-8000-0000000000b1' as never,
      product_revision: 1,
      display_name: 'Status Board',
      kind: 'ui_only',
      lifecycle: 'enabled',
      releases: {
        pointer_revision: 1,
        active_release_epoch: 0,
      },
      service_health: { state: 'not_applicable' },
      surface_available: false,
      updated_at_ms: 1,
    },
    project_id: '0190f5fe-7c00-7a00-8000-0000000000b2',
    project_revision: 1,
    publish_mode: 'manual',
    source_state: 'empty',
    build_generation: 0,
    config_schema: { schema_digest: 'a'.repeat(64), schema: {} },
    config: {
      config_revision: 1,
      schema_digest: 'a'.repeat(64),
      values: {},
      valid: true,
      validation_errors: [],
    },
    credential_bindings_revision: 1,
    credential_slots: [],
    capabilities: [],
    ...overrides,
  }) as PluginRuntimeWorkshop;

describe('PluginRuntime M1 view model', () => {
  test('derives Draft, Ready, and Active stages from release pointers', () => {
    const draft = workshop();
    expect(pluginRuntimeReleaseStage(draft.plugin)).toBe('draft');

    const ready = workshop();
    ready.plugin.releases.ready = {
      release_id: 'ready',
      artifact_id: 'artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    };
    expect(pluginRuntimeReleaseStage(ready.plugin)).toBe('ready');

    ready.plugin.releases.active = {
      release_id: 'active',
      artifact_id: 'artifact',
      release_digest: 'c'.repeat(64),
      manifest_digest: 'd'.repeat(64),
    };
    expect(pluginRuntimeReleaseStage(ready.plugin)).toBe('active');
  });

  test('does not claim Build or Ready completion without durable facts', () => {
    expect(pluginRuntimeWorkflowState(workshop())).toEqual({
      source: 'pending',
      build: 'blocked',
      ready: 'pending',
      publish: 'pending',
      surface: 'pending',
    });
  });

  test('build request binds the exact editable Source and CAS revisions', () => {
    const value = workshop({
      source_state: 'editable',
      build_generation: 3,
      project_revision: 4,
      source_snapshot_digest: 'b'.repeat(64),
      dependency_lock_digest: 'c'.repeat(64),
    });
    expect(pluginRuntimeBuildRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      project_id: value.project_id,
      expected_project_revision: 4,
      expected_build_generation: 3,
      expected_source_snapshot_digest: 'b'.repeat(64),
      expected_dependency_lock_digest: 'c'.repeat(64),
    });
    value.active_operation = {
      operation_id: 'operation',
      operation_revision: 1,
      kind: 'build',
      owner: { owner: 'plugin_runtime', plugin_id: value.plugin.plugin_id },
      state: 'running',
      cancelable: true,
      started_at_ms: 1,
    };
    expect(pluginRuntimeBuildRequest(value)).toBeNull();
    expect(shortPluginRuntimeIdentity('1234567890abcdefghij', 4)).toBe(
      '1234…ghij'
    );
  });

  test('build and service lifecycle requests bind the selected Service lifecycle and Active fence', () => {
    const value = workshop({
      source_state: 'editable',
      build_generation: 3,
      source_snapshot_digest: 'b'.repeat(64),
      dependency_lock_digest: 'c'.repeat(64),
    });
    value.plugin.kind = 'service';
    value.plugin.releases.active = {
      release_id: 'active-service',
      artifact_id: 'service-artifact',
      release_digest: 'd'.repeat(64),
      manifest_digest: 'e'.repeat(64),
    };
    value.plugin.releases.active_release_epoch = 4;
    value.plugin.service_health = { state: 'stopped' };

    expect(pluginRuntimeBuildRequest(value, 'continuous')?.service_lifecycle).toBe(
      'continuous'
    );
    expect(pluginRuntimeSetServiceRunningRequest(value, true)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      expected_pointer_revision: 1,
      expected_active_release_epoch: 4,
      expected_active_release_digest: 'd'.repeat(64),
      running: true,
    });
    expect(pluginRuntimeRetryServiceRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      expected_pointer_revision: 1,
      expected_active_release_epoch: 4,
      expected_active_release_digest: 'd'.repeat(64),
    });

    const readyRelease = {
      release_id: 'ready-service',
      artifact_id: 'ready-artifact',
      release_digest: 'f'.repeat(64),
      manifest_digest: '0'.repeat(64),
    };
    value.plugin.releases.ready = readyRelease;
    value.ready = {
      release: readyRelease,
      project_build_generation: 3,
      created_at_ms: 2,
      kind: 'service',
      service: {
        lifecycle: 'continuous',
        uses_files: true,
        uses_private_database: true,
        service_contract_digest: '1'.repeat(64),
      },
      test: {
        status: 'not_run',
        release_id: readyRelease.release_id,
        expected_release_digest: readyRelease.release_digest,
      },
      migration_count: 1,
      can_publish: true,
      can_auto_publish: false,
      blocking_reasons: ['service_test_not_run'],
    };
    expect(pluginRuntimeTestRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      expected_pointer_revision: 1,
      project_id: value.project_id,
      expected_project_revision: 1,
      expected_build_generation: 3,
      release_id: readyRelease.release_id,
      expected_release_digest: readyRelease.release_digest,
      expected_config_revision: 1,
      expected_credential_bindings_revision: 1,
      resolved_test_input_digest: EMPTY_MINIAPP_TEST_INPUT_DIGEST,
    });
  });

  test('builds exact Publish, Rollback, lifecycle, and publish-mode CAS requests', () => {
    const readyRelease = {
      release_id: 'ready-release',
      artifact_id: 'ui-capability-artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    };
    const activeRelease = {
      release_id: 'active-release',
      artifact_id: 'active-artifact',
      release_digest: 'c'.repeat(64),
      manifest_digest: 'd'.repeat(64),
    };
    const previousRelease = {
      release_id: 'previous-release',
      artifact_id: 'previous-artifact',
      release_digest: 'e'.repeat(64),
      manifest_digest: 'f'.repeat(64),
    };
    const value = workshop({
      publish_mode: 'manual',
      source_state: 'editable',
      build_generation: 2,
      project_revision: 3,
      source_snapshot_digest: '1'.repeat(64),
      dependency_lock_digest: '2'.repeat(64),
    });
    value.plugin.product_revision = 4;
    value.plugin.releases = {
      pointer_revision: 7,
      active_release_epoch: 9,
      ready: readyRelease,
      active: activeRelease,
      previous: previousRelease,
    };
    value.ready = {
      release: readyRelease,
      project_build_generation: 2,
      created_at_ms: 3,
      kind: 'ui_only',
      test: {
        status: 'not_required',
        release_id: readyRelease.release_id,
        expected_release_digest: readyRelease.release_digest,
      },
      migration_count: 0,
      can_publish: true,
      can_auto_publish: true,
      blocking_reasons: [],
    };

    expect(pluginRuntimePublishRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      expected_active_release_epoch: 9,
      ready_release_id: readyRelease.release_id,
      expected_ready_release_digest: readyRelease.release_digest,
      expected_active_release_digest: activeRelease.release_digest,
      acknowledge_test_warning: false,
    });
    value.plugin.kind = 'service';
    value.ready.kind = 'service';
    value.ready.service = {
      lifecycle: 'on_demand',
      uses_files: false,
      uses_private_database: false,
      service_contract_digest: '9'.repeat(64),
    };
    value.ready.test = {
      status: 'passed',
      release_id: readyRelease.release_id,
      expected_release_digest: readyRelease.release_digest,
      receipt_id: 'receipt-1',
      expected_service_run_key: '8'.repeat(64),
    };
    expect(pluginRuntimePublishRequest(value)).toMatchObject({
      expected_service_test_receipt_id: 'receipt-1',
      acknowledge_test_warning: false,
    });
    value.ready.test.status = 'needs_test_input';
    expect(pluginRuntimePublishRequest(value)).toMatchObject({
      expected_service_test_receipt_id: 'receipt-1',
      acknowledge_test_warning: true,
    });
    value.ready.test.status = 'stale';
    expect(pluginRuntimePublishRequest(value)).toMatchObject({
      acknowledge_test_warning: true,
    });
    expect(
      pluginRuntimePublishRequest(value)?.expected_service_test_receipt_id
    ).toBeUndefined();
    value.plugin.kind = 'ui_only';
    value.ready.kind = 'ui_only';
    delete value.ready.service;
    value.ready.test = {
      status: 'not_required',
      release_id: readyRelease.release_id,
      expected_release_digest: readyRelease.release_digest,
    };
    expect(pluginRuntimeRollbackRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      expected_active_release_epoch: 9,
      expected_current_release_digest: activeRelease.release_digest,
      previous_release_id: previousRelease.release_id,
      expected_previous_release_digest: previousRelease.release_digest,
    });
    expect(pluginRuntimeSetEnabledRequest(value, false)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      expected_active_release_digest: activeRelease.release_digest,
      enabled: false,
    });
    expect(pluginRuntimeSetPublishModeRequest(value, 'auto_ui_only')).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      mode: 'auto_ui_only',
    });
    value.publish_mode = 'auto_ui_only';
    value.active_operation = {
      operation_id: '0190f5fe-7c00-7000-8000-000000000401',
      operation_revision: 1,
      kind: 'build',
      owner: {
        owner: 'plugin_runtime',
        plugin_id: value.plugin.plugin_id,
      },
      state: 'running',
      cancelable: true,
      started_at_ms: 4,
    };
    expect(pluginRuntimeSetPublishModeRequest(value, 'manual')).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      mode: 'manual',
    });
  });

  test('builds exact Share requests for Ready or Active releases', () => {
    const value = workshop({
      source_state: 'editable',
      build_generation: 2,
      source_snapshot_digest: '1'.repeat(64),
      dependency_lock_digest: '2'.repeat(64),
    });
    value.plugin.lifecycle = 'disabled';
    value.plugin.product_revision = 4;
    value.plugin.releases.pointer_revision = 7;
    const readyRelease = {
      release_id: 'ready-release',
      artifact_id: 'ready-artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    };
    const activeRelease = {
      release_id: 'active-release',
      artifact_id: 'active-artifact',
      release_digest: 'c'.repeat(64),
      manifest_digest: 'd'.repeat(64),
    };
    value.plugin.releases.ready = readyRelease;
    value.plugin.releases.active = activeRelease;
    value.plugin.releases.active_release_epoch = 3;
    value.ready = {
      release: readyRelease,
      project_build_generation: 2,
      created_at_ms: 3,
      kind: 'ui_only',
      test: {
        status: 'not_required',
        release_id: readyRelease.release_id,
        expected_release_digest: readyRelease.release_digest,
      },
      migration_count: 0,
      can_publish: true,
      can_auto_publish: false,
      blocking_reasons: [],
    };

    expect(
      pluginRuntimeShareRequest(
        value,
        'ready_release',
        ' C:\\exports\\ready.nomifun-plugin ',
        true
      )
    ).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      content: 'ready_release',
      release_id: readyRelease.release_id,
      expected_release_digest: readyRelease.release_digest,
      destination_path: 'C:\\exports\\ready.nomifun-plugin',
      include_source: true,
    });
    expect(
      pluginRuntimeShareRequest(
        value,
        'active_release',
        'C:\\exports\\active.nomifun-plugin',
        false
      )
    ).toMatchObject({
      content: 'active_release',
      release_id: activeRelease.release_id,
      expected_release_digest: activeRelease.release_digest,
      include_source: false,
    });

    value.source_state = 'runtime_only';
    expect(
      pluginRuntimeShareRequest(
        value,
        'active_release',
        'C:\\exports\\runtime.nomifun-plugin',
        true
      )
    ).toBeNull();
    expect(
      pluginRuntimeShareRequest(value, 'active_release', '', false)
    ).toBeNull();
  });

  test('requires an Active Release and enabled lifecycle before opening Surface', () => {
    const value = workshop({
      publish_mode: 'manual',
    });
    expect(pluginRuntimeCanOpenSurface(value)).toBe(false);
    expect(pluginRuntimeSetEnabledRequest(value, true)).toBeNull();

    value.plugin.releases.active = {
      release_id: 'active-release',
      artifact_id: 'active-artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    };
    value.plugin.releases.active_release_epoch = 3;
    value.plugin.lifecycle = 'disabled';
    expect(pluginRuntimeCanOpenSurface(value)).toBe(false);
    expect(pluginRuntimeSetEnabledRequest(value, true)?.enabled).toBe(true);

    value.plugin.lifecycle = 'enabled';
    value.plugin.surface_available = true;
    expect(pluginRuntimeCanOpenSurface(value)).toBe(true);
  });

  test('constructs exact Trash, Restore, Delete, and failed Delete Retry requests', () => {
    const value = workshop({
      plugin: {
        ...workshop().plugin,
        lifecycle: 'disabled',
        releases: {
          pointer_revision: 4,
          active_release_epoch: 2,
          active: {
            release_id: 'active',
            artifact_id: 'artifact',
            release_digest: 'a'.repeat(64),
            manifest_digest: 'b'.repeat(64),
          },
        },
      },
    });
    expect(pluginRuntimeTrashRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      expected_pointer_revision: 4,
      expected_active_release_digest: 'a'.repeat(64),
    });

    value.plugin.lifecycle = 'trashed';
    expect(pluginRuntimeRestoreRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      expected_lifecycle: 'trashed',
      expected_pointer_revision: 4,
    });
    expect(pluginRuntimeDeleteRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      expected_product_revision: 1,
      expected_lifecycle: 'trashed',
      expected_pointer_revision: 4,
      expected_active_release_digest: 'a'.repeat(64),
    });

    value.plugin.lifecycle = 'deleting';
    value.active_operation = {
      operation_id: 'delete-operation',
      operation_revision: 2,
      kind: 'plugin_permanent_delete',
      owner: { owner: 'plugin_runtime', plugin_id: value.plugin.plugin_id },
      state: 'failed',
      cancelable: false,
      started_at_ms: 5,
      completed_at_ms: 6,
    };
    expect(pluginRuntimeRetryDeleteRequest(value)).toEqual({
      plugin_id: value.plugin.plugin_id,
      failed_operation_id: 'delete-operation',
      expected_operation_revision: 2,
    });
  });

  test('constructs the versioned Surface asset path and rejects unsafe entrypoints', () => {
    const descriptor = {
      plugin_id: '0190f5fe-7c00-7000-8000-000000000301' as never,
      product_revision: 4,
      release_id: 'release-1',
      expected_release_digest: 'a'.repeat(64),
      active_release_epoch: 12,
      surface_session_id: '0190f5fe-7c00-7000-8000-000000000302',
      surface_generation: 2,
      surface_capability: 'capability-token',
      ui_entrypoint: 'ui/index.html',
      kind: 'ui_only' as const,
    };
    expect(pluginRuntimeSurfaceAssetPath(descriptor)).toBe(
      `/api/plugins/runtimes/${descriptor.plugin_id}/surface/assets/capability-token/12/${'a'.repeat(64)}/ui/index.html`
    );
    expect(
      pluginRuntimeSurfaceAssetPath({ ...descriptor, ui_entrypoint: '../index.html' })
    ).toBeNull();
    expect(
      pluginRuntimeSurfaceAssetPath({ ...descriptor, ui_entrypoint: '/ui/index.html' })
    ).toBeNull();
  });
});
