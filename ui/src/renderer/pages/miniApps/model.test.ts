/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { MiniAppWorkshop } from '@/common/types/miniAppPlatform';
import {
  miniAppBuildRequest,
  miniAppCanOpenSurface,
  miniAppPublishRequest,
  miniAppReleaseStage,
  miniAppRollbackRequest,
  miniAppSetEnabledRequest,
  miniAppSetPublishModeRequest,
  miniAppSurfaceAssetPath,
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
