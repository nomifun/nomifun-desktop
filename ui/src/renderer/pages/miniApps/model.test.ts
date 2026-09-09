/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { MiniAppWorkshop } from '@/common/types/miniAppPlatform';
import {
  EMPTY_MINIAPP_TEST_INPUT_DIGEST,
  miniAppBuildRequest,
  miniAppCanOpenSurface,
  miniAppPublishRequest,
  miniAppDeleteRequest,
  miniAppRestoreRequest,
  miniAppRetryDeleteRequest,
  miniAppTrashRequest,
  miniAppRetryServiceRequest,
  miniAppReleaseStage,
  miniAppRollbackRequest,
  miniAppSetEnabledRequest,
  miniAppSetServiceRunningRequest,
  miniAppSetPublishModeRequest,
  miniAppShareRequest,
  miniAppSurfaceAssetPath,
  miniAppTestRequest,
  miniAppWorkflowState,
  shortMiniAppIdentity,
} from './model';

const workshop = (overrides: Partial<MiniAppWorkshop> = {}): MiniAppWorkshop =>
  ({
    miniapp: {
      miniapp_id: '0190f5fe-7c00-7a00-8000-0000000000b1' as never,
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
  }) as MiniAppWorkshop;

describe('MiniApp M1 view model', () => {
  test('derives Draft, Ready, and Active stages from release pointers', () => {
    const draft = workshop();
    expect(miniAppReleaseStage(draft.miniapp)).toBe('draft');

    const ready = workshop();
    ready.miniapp.releases.ready = {
      release_id: 'ready',
      artifact_id: 'artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    };
    expect(miniAppReleaseStage(ready.miniapp)).toBe('ready');

    ready.miniapp.releases.active = {
      release_id: 'active',
      artifact_id: 'artifact',
      release_digest: 'c'.repeat(64),
      manifest_digest: 'd'.repeat(64),
    };
    expect(miniAppReleaseStage(ready.miniapp)).toBe('active');
  });

  test('does not claim Build or Ready completion without durable facts', () => {
    expect(miniAppWorkflowState(workshop())).toEqual({
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
    expect(miniAppBuildRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
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
      owner: { owner: 'miniapp', miniapp_id: value.miniapp.miniapp_id },
      state: 'running',
      cancelable: true,
      started_at_ms: 1,
    };
    expect(miniAppBuildRequest(value)).toBeNull();
    expect(shortMiniAppIdentity('1234567890abcdefghij', 4)).toBe(
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
    value.miniapp.kind = 'service';
    value.miniapp.releases.active = {
      release_id: 'active-service',
      artifact_id: 'service-artifact',
      release_digest: 'd'.repeat(64),
      manifest_digest: 'e'.repeat(64),
    };
    value.miniapp.releases.active_release_epoch = 4;
    value.miniapp.service_health = { state: 'stopped' };

    expect(miniAppBuildRequest(value, 'continuous')?.service_lifecycle).toBe(
      'continuous'
    );
    expect(miniAppSetServiceRunningRequest(value, true)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 1,
      expected_pointer_revision: 1,
      expected_active_release_epoch: 4,
      expected_active_release_digest: 'd'.repeat(64),
      running: true,
    });
    expect(miniAppRetryServiceRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
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
    value.miniapp.releases.ready = readyRelease;
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
    expect(miniAppTestRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
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
    value.miniapp.product_revision = 4;
    value.miniapp.releases = {
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

    expect(miniAppPublishRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      expected_active_release_epoch: 9,
      ready_release_id: readyRelease.release_id,
      expected_ready_release_digest: readyRelease.release_digest,
      expected_active_release_digest: activeRelease.release_digest,
      acknowledge_test_warning: false,
    });
    value.miniapp.kind = 'service';
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
    expect(miniAppPublishRequest(value)).toMatchObject({
      expected_service_test_receipt_id: 'receipt-1',
      acknowledge_test_warning: false,
    });
    value.ready.test.status = 'needs_test_input';
    expect(miniAppPublishRequest(value)).toMatchObject({
      expected_service_test_receipt_id: 'receipt-1',
      acknowledge_test_warning: true,
    });
    value.ready.test.status = 'stale';
    expect(miniAppPublishRequest(value)).toMatchObject({
      acknowledge_test_warning: true,
    });
    expect(
      miniAppPublishRequest(value)?.expected_service_test_receipt_id
    ).toBeUndefined();
    value.miniapp.kind = 'ui_only';
    value.ready.kind = 'ui_only';
    delete value.ready.service;
    value.ready.test = {
      status: 'not_required',
      release_id: readyRelease.release_id,
      expected_release_digest: readyRelease.release_digest,
    };
    expect(miniAppRollbackRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      expected_active_release_epoch: 9,
      expected_current_release_digest: activeRelease.release_digest,
      previous_release_id: previousRelease.release_id,
      expected_previous_release_digest: previousRelease.release_digest,
    });
    expect(miniAppSetEnabledRequest(value, false)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      expected_active_release_digest: activeRelease.release_digest,
      enabled: false,
    });
    expect(miniAppSetPublishModeRequest(value, 'auto_ui_only')).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
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
        owner: 'miniapp',
        miniapp_id: value.miniapp.miniapp_id,
      },
      state: 'running',
      cancelable: true,
      started_at_ms: 4,
    };
    expect(miniAppSetPublishModeRequest(value, 'manual')).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
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
    value.miniapp.lifecycle = 'disabled';
    value.miniapp.product_revision = 4;
    value.miniapp.releases.pointer_revision = 7;
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
    value.miniapp.releases.ready = readyRelease;
    value.miniapp.releases.active = activeRelease;
    value.miniapp.releases.active_release_epoch = 3;
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
      miniAppShareRequest(
        value,
        'ready_release',
        ' C:\\exports\\ready.nomifun-miniapp ',
        true
      )
    ).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      content: 'ready_release',
      release_id: readyRelease.release_id,
      expected_release_digest: readyRelease.release_digest,
      destination_path: 'C:\\exports\\ready.nomifun-miniapp',
      include_source: true,
    });
    expect(
      miniAppShareRequest(
        value,
        'active_release',
        'C:\\exports\\active.nomifun-miniapp',
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
      miniAppShareRequest(
        value,
        'active_release',
        'C:\\exports\\runtime.nomifun-miniapp',
        true
      )
    ).toBeNull();
    expect(
      miniAppShareRequest(value, 'active_release', '', false)
    ).toBeNull();
  });

  test('requires an Active Release and enabled lifecycle before opening Surface', () => {
    const value = workshop({
      publish_mode: 'manual',
    });
    expect(miniAppCanOpenSurface(value)).toBe(false);
    expect(miniAppSetEnabledRequest(value, true)).toBeNull();

    value.miniapp.releases.active = {
      release_id: 'active-release',
      artifact_id: 'active-artifact',
      release_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    };
    value.miniapp.releases.active_release_epoch = 3;
    value.miniapp.lifecycle = 'disabled';
    expect(miniAppCanOpenSurface(value)).toBe(false);
    expect(miniAppSetEnabledRequest(value, true)?.enabled).toBe(true);

    value.miniapp.lifecycle = 'enabled';
    value.miniapp.surface_available = true;
    expect(miniAppCanOpenSurface(value)).toBe(true);
  });

  test('constructs exact Trash, Restore, Delete, and failed Delete Retry requests', () => {
    const value = workshop({
      miniapp: {
        ...workshop().miniapp,
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
    expect(miniAppTrashRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 1,
      expected_pointer_revision: 4,
      expected_active_release_digest: 'a'.repeat(64),
    });

    value.miniapp.lifecycle = 'trashed';
    expect(miniAppRestoreRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 1,
      expected_lifecycle: 'trashed',
      expected_pointer_revision: 4,
    });
    expect(miniAppDeleteRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      expected_product_revision: 1,
      expected_lifecycle: 'trashed',
      expected_pointer_revision: 4,
      expected_active_release_digest: 'a'.repeat(64),
    });

    value.miniapp.lifecycle = 'deleting';
    value.active_operation = {
      operation_id: 'delete-operation',
      operation_revision: 2,
      kind: 'miniapp_permanent_delete',
      owner: { owner: 'miniapp', miniapp_id: value.miniapp.miniapp_id },
      state: 'failed',
      cancelable: false,
      started_at_ms: 5,
      completed_at_ms: 6,
    };
    expect(miniAppRetryDeleteRequest(value)).toEqual({
      miniapp_id: value.miniapp.miniapp_id,
      failed_operation_id: 'delete-operation',
      expected_operation_revision: 2,
    });
  });

  test('constructs the versioned Surface asset path and rejects unsafe entrypoints', () => {
    const descriptor = {
      miniapp_id: '0190f5fe-7c00-7000-8000-000000000301' as never,
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
    expect(miniAppSurfaceAssetPath(descriptor)).toBe(
      `/api/miniapps/${descriptor.miniapp_id}/surface/assets/capability-token/12/${'a'.repeat(64)}/ui/index.html`
    );
    expect(
      miniAppSurfaceAssetPath({ ...descriptor, ui_entrypoint: '../index.html' })
    ).toBeNull();
    expect(
      miniAppSurfaceAssetPath({ ...descriptor, ui_entrypoint: '/ui/index.html' })
    ).toBeNull();
  });
});
