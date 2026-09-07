/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { MiniAppWorkshop } from '@/common/types/miniAppPlatform';
import {
  miniAppPublishBlockingReasons,
  miniAppReleaseStage,
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

  test('does not claim Build or Publish completion without durable facts', () => {
    expect(miniAppWorkflowState(workshop())).toEqual({
      source: 'pending',
      build: 'blocked',
      ready: 'pending',
      publish: 'pending',
      surface: 'pending',
    });
  });

  test('deduplicates publish blocking reasons and shortens opaque identities', () => {
    const value = workshop({
      ready: {
        release: {
          release_id: 'ready',
          artifact_id: 'artifact',
          release_digest: 'a'.repeat(64),
          manifest_digest: 'b'.repeat(64),
        },
        project_build_generation: 1,
        kind: 'ui_only',
        test: {
          status: 'not_required',
          release_id: 'ready',
          expected_release_digest: 'a'.repeat(64),
        },
        migration_count: 0,
        can_publish: false,
        can_auto_publish: false,
        blocking_reasons: [' NEEDS_SOURCE ', 'NEEDS_SOURCE', ''],
      },
    });
    expect(miniAppPublishBlockingReasons(value)).toEqual(['NEEDS_SOURCE']);
    expect(shortMiniAppIdentity('1234567890abcdefghij', 4)).toBe(
      '1234…ghij'
    );
  });
});
