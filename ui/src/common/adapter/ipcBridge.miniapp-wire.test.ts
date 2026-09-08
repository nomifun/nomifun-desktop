/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { miniapps } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const MINIAPP_ID = '0190f5fe-7c00-7a00-8000-0000000000b1';
const realFetch = globalThis.fetch;

const rawSummary = (miniappId: unknown = MINIAPP_ID) => ({
  miniapp_id: miniappId,
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
  updated_at_ms: 1_780_000_000_000,
});

const rawWorkshop = (miniappId: unknown = MINIAPP_ID) => ({
  miniapp: rawSummary(miniappId),
  project_id: '0190f5fe-7c00-7a00-8000-0000000000b2',
  project_revision: 1,
  publish_mode: 'manual',
  source_state: 'empty',
  build_generation: 0,
  config_schema: {
    schema_digest: 'a'.repeat(64),
    schema: { type: 'object' },
  },
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
  active_operation: {
    operation_id: '0190f5fe-7c00-7a00-8000-0000000000b3',
    operation_revision: 1,
    kind: 'build',
    owner: { owner: 'miniapp', miniapp_id: miniappId },
    state: 'running',
    cancelable: true,
    started_at_ms: 1_780_000_000_000,
  },
});

function respondWith(data: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify({ success: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('MiniApp M1 HTTP bridge', () => {
  test('spells the clean-start Library, Workshop, release, lifecycle, and Surface routes', () => {
    expect(
      source.includes(
        "httpGet<MiniAppLibraryResponse, void>('/api/miniapps')"
      )
    ).toBe(true);
    expect(
      /httpPost<MiniAppWorkshop,\s*CreateMiniAppProjectRequest>\(\s*'\/api\/miniapps\/projects'/.test(
        source
      )
    ).toBe(true);
    expect(
      source.includes(
        '`/api/miniapps/${encodeURIComponent(miniapp_id)}/workshop`'
      )
    ).toBe(true);
    for (const route of [
      '/publish`',
      '/rollback`',
      '/enabled`',
      '/publish-mode`',
      '/surface/open`',
      '/surface/close`',
    ]) {
      expect(source.includes(route)).toBe(true);
    }

    for (const retired of [
      '/api/miniapps/validate',
      '/api/miniapps/import',
      '/workspace`',
      '/api/miniapps/${p.miniapp_id}/serve',
    ]) {
      expect(source.includes(retired)).toBe(false);
    }
    expect(source.includes('/surface/assets/')).toBe(false);
    expect(
      source.includes(
        'httpGet<MiniAppSurfaceLaunchDescriptor, { miniapp_id: MiniAppId }>'
      )
    ).toBe(false);
    expect(source.includes('getSurface:')).toBe(false);
    expect(source.includes('/api/miniapps/${')).toBe(true);
  });

  test('brands every Library miniapp_id at the boundary', async () => {
    respondWith({ library_revision: 2, miniapps: [rawSummary()] });
    const library = await miniapps.library.invoke();
    expect(library.library_revision).toBe(2);
    expect(library.miniapps[0]?.miniapp_id).toBe(MINIAPP_ID);

    respondWith({
      library_revision: 2,
      miniapps: [rawSummary(`miniapp_${MINIAPP_ID}`)],
    });
    let caught: unknown;
    try {
      await miniapps.library.invoke();
    } catch (error) {
      caught = error;
    }
    expect(caught instanceof InvalidEntityIdError).toBe(true);
  });

  test('brands nested Workshop and operation-owner identities', async () => {
    respondWith(rawWorkshop());
    const workshop = await miniapps.getWorkshop.invoke({
      miniapp_id: MINIAPP_ID as never,
    });
    expect(workshop.miniapp.miniapp_id).toBe(MINIAPP_ID);
    expect(workshop.active_operation?.owner).toEqual({
      owner: 'miniapp',
      miniapp_id: MINIAPP_ID,
    });
  });

  test('Create Project preserves exact CAS and product fields', async () => {
    let requestBody: unknown;
    globalThis.fetch = (async (_input, init) => {
      requestBody =
        typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
      return new Response(
        JSON.stringify({ success: true, data: rawWorkshop() }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }
      );
    }) as typeof fetch;

    const workshop = await miniapps.createProject.invoke({
      expected_library_revision: 7,
      display_name: 'Status Board',
      description: 'Tracks delivery state',
      kind: 'ui_only',
    });
    expect(requestBody).toEqual({
      expected_library_revision: 7,
      display_name: 'Status Board',
      description: 'Tracks delivery state',
      kind: 'ui_only',
    });
    expect(workshop.miniapp.miniapp_id).toBe(MINIAPP_ID);
  });

  test('release, lifecycle, and publish-mode mutations preserve exact request bodies', async () => {
    let requestPath = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input, init) => {
      requestPath = new URL(String(input), 'http://127.0.0.1').pathname;
      requestBody =
        typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
      return new Response(
        JSON.stringify({ success: true, data: rawWorkshop() }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }
      );
    }) as typeof fetch;

    const publish = {
      miniapp_id: MINIAPP_ID as never,
      expected_product_revision: 4,
      expected_pointer_revision: 5,
      expected_active_release_epoch: 6,
      ready_release_id: 'ready-release',
      expected_ready_release_digest: 'a'.repeat(64),
      expected_active_release_digest: 'b'.repeat(64),
      acknowledge_test_warning: false,
    };
    await miniapps.publish.invoke(publish);
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/publish`);
    expect(requestBody).toEqual(publish);

    const rollback = {
      miniapp_id: MINIAPP_ID as never,
      expected_product_revision: 5,
      expected_pointer_revision: 6,
      expected_active_release_epoch: 7,
      expected_current_release_digest: 'b'.repeat(64),
      previous_release_id: 'previous-release',
      expected_previous_release_digest: 'c'.repeat(64),
    };
    await miniapps.rollback.invoke(rollback);
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/rollback`);
    expect(requestBody).toEqual(rollback);

    const enabled = {
      miniapp_id: MINIAPP_ID as never,
      expected_product_revision: 6,
      expected_pointer_revision: 7,
      expected_active_release_digest: 'c'.repeat(64),
      enabled: true,
    };
    await miniapps.setEnabled.invoke(enabled);
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/enabled`);
    expect(requestBody).toEqual(enabled);

    const publishMode = {
      miniapp_id: MINIAPP_ID as never,
      expected_product_revision: 7,
      expected_pointer_revision: 8,
      mode: 'auto_ui_only' as const,
    };
    await miniapps.setPublishMode.invoke(publishMode);
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/publish-mode`);
    expect(requestBody).toEqual(publishMode);

    const bridge = {
      miniapp_id: MINIAPP_ID as never,
      surface_capability: 'surface-capability',
      active_release_epoch: 9,
      expected_release_digest: 'd'.repeat(64),
      request: {
        call_id: 'call-1',
        target: {
          target: 'host_kv' as const,
          request: {
            operation: 'set' as const,
            key: 'preference',
            value: { density: 'compact' },
          },
        },
      },
    };
    await miniapps.bridge.invoke(bridge);
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/surface/bridge`);
    expect(requestPath.includes(bridge.surface_capability)).toBe(false);
    expect(requestBody).toEqual({
      surface_capability: bridge.surface_capability,
      active_release_epoch: bridge.active_release_epoch,
      expected_release_digest: bridge.expected_release_digest,
      request: bridge.request,
    });
  });

  test('opens and brands the authenticated Surface descriptor with an explicit POST body', async () => {
    let requestPath = '';
    let requestMethod = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input, init) => {
      requestPath = new URL(String(input), 'http://127.0.0.1').pathname;
      requestMethod = init?.method ?? '';
      requestBody =
        typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            miniapp_id: MINIAPP_ID,
            product_revision: 8,
            release_id: 'active-release',
            expected_release_digest: 'a'.repeat(64),
            active_release_epoch: 9,
            surface_session_id: '0190f5fe-7c00-7a00-8000-0000000000b4',
            surface_generation: 3,
            surface_capability: 'temporary-capability',
            ui_entrypoint: 'ui/index.html',
            kind: 'ui_only',
          },
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }
      );
    }) as typeof fetch;

    const request = {
      miniapp_id: MINIAPP_ID as never,
    };
    const descriptor = await miniapps.openSurface.invoke(request);
    expect(requestMethod).toBe('POST');
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/surface/open`);
    expect(requestBody).toEqual(request);
    expect(descriptor.miniapp_id).toBe(MINIAPP_ID);
    expect(descriptor.surface_generation).toBe(3);
    expect(descriptor.surface_capability).toBe('temporary-capability');
    expect(source.includes('/surface/assets/')).toBe(false);
  });

  test('closes the exact Host-owned Surface session', async () => {
    let requestPath = '';
    let requestBody = '';
    globalThis.fetch = (async (input, init) => {
      requestPath = new URL(String(input), 'http://127.0.0.1').pathname;
      requestBody = String(init?.body ?? '');
      return new Response(JSON.stringify({ success: true, data: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    const request = {
      miniapp_id: MINIAPP_ID as never,
      surface_session_id: '0190f5fe-7c00-7a00-8000-0000000000b4',
      surface_capability: 'surface-capability',
    };
    expect(await miniapps.closeSurface.invoke(request)).toBe(true);
    expect(requestPath).toBe(`/api/miniapps/${MINIAPP_ID}/surface/close`);
    expect(JSON.parse(requestBody)).toEqual(request);
  });
});
