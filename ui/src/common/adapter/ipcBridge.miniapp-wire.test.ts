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
  test('spells only the Library, Create Project, and Workshop routes', () => {
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

    for (const retired of [
      '/api/miniapps/validate',
      '/api/miniapps/import',
      '/publish`',
      '/workspace`',
    ]) {
      expect(source.includes(retired)).toBe(false);
    }
    expect(source.includes('/api/miniapps/${')).toBe(true);
    expect(source.includes('/api/miniapps/${p.miniapp_id}/serve')).toBe(false);
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
});
