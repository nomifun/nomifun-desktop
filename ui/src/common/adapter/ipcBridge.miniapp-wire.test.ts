/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { pluginRuntimes } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const MINIAPP_ID = '0190f5fe-7c00-7a00-8000-0000000000b1';
const realFetch = globalThis.fetch;

const rawSummary = (miniappId: unknown = MINIAPP_ID) => ({
  plugin_id: miniappId,
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
  plugin: rawSummary(miniappId),
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
    owner: { owner: 'plugin_runtime', plugin_id: miniappId },
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

describe('PluginRuntime M1 HTTP bridge', () => {
  test('spells the clean-start Library, Workshop, release, lifecycle, and Surface routes', () => {
    expect(
      source.includes(
        "httpGet<PluginRuntimeLibraryResponse, void>('/api/plugins/runtimes')"
      )
    ).toBe(true);
    expect(
      /httpPost<PluginRuntimeWorkshop,\s*CreatePluginRuntimeProjectRequest>\(\s*'\/api\/plugins\/runtimes\/projects'/.test(
        source
      )
    ).toBe(true);
    expect(source.includes("'/api/plugins/runtimes/import/share'")).toBe(true);
    expect(source.includes("'/api/plugins/runtimes/import/artifact'")).toBe(true);
    expect(source.includes("'/api/plugins/runtimes/import/backup'")).toBe(true);
    expect(
      source.includes(
        '`/api/plugins/runtimes/${encodeURIComponent(plugin_id)}/workshop`'
      )
    ).toBe(true);
    for (const route of [
      '/test`',
      '/publish`',
      '/rollback`',
      '/enabled`',
      '/publish-mode`',
      '/service/running`',
      '/service/retry`',
      '/share`',
      '/backup`',
      '/surface/open`',
      '/surface/close`',
      '/source/edit`',
    ]) {
      expect(source.includes(route)).toBe(true);
    }

    for (const retired of [
      '/api/plugins/runtimes/validate',
      "httpPost<PluginRuntimeWorkshop, PluginRuntimeImportRequest>('/api/plugins/runtimes/import')",
      '/workspace`',
      '/api/plugins/runtimes/${p.plugin_id}/serve',
    ]) {
      expect(source.includes(retired)).toBe(false);
    }
    expect(source.includes('/surface/assets/')).toBe(false);
    expect(
      source.includes(
        'httpGet<PluginRuntimeSurfaceLaunchDescriptor, { plugin_id: PluginRuntimeId }>'
      )
    ).toBe(false);
    expect(source.includes('getSurface:')).toBe(false);
    expect(source.includes('/api/plugins/runtimes/${')).toBe(true);
  });

  test('brands every Library plugin_id at the boundary', async () => {
    respondWith({ library_revision: 2, plugins: [rawSummary()] });
    const library = await pluginRuntimes.library.invoke();
    expect(library.library_revision).toBe(2);
    expect(library.plugins[0]?.plugin_id).toBe(MINIAPP_ID);

    respondWith({
      library_revision: 2,
      plugins: [rawSummary(`miniapp_${MINIAPP_ID}`)],
    });
    let caught: unknown;
    try {
      await pluginRuntimes.library.invoke();
    } catch (error) {
      caught = error;
    }
    expect(caught instanceof InvalidEntityIdError).toBe(true);
  });

  test('brands nested Workshop and operation-owner identities', async () => {
    respondWith(rawWorkshop());
    const workshop = await pluginRuntimes.getWorkshop.invoke({
      plugin_id: MINIAPP_ID as never,
    });
    expect(workshop.plugin.plugin_id).toBe(MINIAPP_ID);
    expect(workshop.active_operation?.owner).toEqual({
      owner: 'plugin_runtime',
      plugin_id: MINIAPP_ID,
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

    const workshop = await pluginRuntimes.createProject.invoke({
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
    expect(workshop.plugin.plugin_id).toBe(MINIAPP_ID);
  });

  test('Source read and replace preserve the encoded path and exact CAS body', async () => {
    let requestPath = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input, init) => {
      requestPath = new URL(String(input), 'http://127.0.0.1').pathname;
      requestBody =
        typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
      const responseData =
        init?.method === 'POST'
          ? rawWorkshop()
          : {
              plugin_id: MINIAPP_ID,
              project_id: '0190f5fe-7c00-7a00-8000-0000000000b2',
              path: 'ui/index.html',
              content: '<main>v1</main>',
              source_snapshot_digest: 'a'.repeat(64),
              build_generation: 3,
            };
      return new Response(JSON.stringify({ success: true, data: responseData }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    const sourceFile = await pluginRuntimes.getSourceFile.invoke({
      plugin_id: MINIAPP_ID as never,
      path: 'ui/index.html',
    });
    expect(requestPath).toBe(
      `/api/plugins/runtimes/${MINIAPP_ID}/source/files/ui%2Findex.html`
    );
    expect(sourceFile.plugin_id).toBe(MINIAPP_ID);
    expect(sourceFile.content).toBe('<main>v1</main>');

    await pluginRuntimes.replaceSourceFile.invoke({
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 4,
      project_id: '0190f5fe-7c00-7a00-8000-0000000000b2',
      expected_project_revision: 5,
      expected_build_generation: 3,
      expected_source_snapshot_digest: 'a'.repeat(64),
      path: 'ui/index.html',
      content: '<main>v2</main>',
    });
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/source/edit`);
    expect(requestBody).toEqual({
      plugin_id: MINIAPP_ID,
      expected_product_revision: 4,
      project_id: '0190f5fe-7c00-7a00-8000-0000000000b2',
      expected_project_revision: 5,
      expected_build_generation: 3,
      expected_source_snapshot_digest: 'a'.repeat(64),
      path: 'ui/index.html',
      content: '<main>v2</main>',
    });
  });

  test('Share export and import preserve exact paths, digests, and CAS fields', async () => {
    let requestPath = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input, init) => {
      requestPath = new URL(String(input), 'http://127.0.0.1').pathname;
      requestBody =
        typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
      const responseData =
        requestPath === `/api/plugins/runtimes/${MINIAPP_ID}/share`
          ? {
              operation_id: '0190f5fe-7c00-7a00-8000-0000000000b5',
              operation_revision: 1,
              kind: 'export',
              owner: { owner: 'plugin_runtime', plugin_id: MINIAPP_ID },
              state: 'succeeded',
              cancelable: false,
              progress_percent: 100,
              started_at_ms: 1_780_000_000_000,
              completed_at_ms: 1_780_000_000_001,
            }
          : rawWorkshop();
      return new Response(
        JSON.stringify({ success: true, data: responseData }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }
      );
    }) as typeof fetch;

    const share = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 4,
      expected_pointer_revision: 7,
      content: 'ready_release' as const,
      release_id: 'ready-release',
      expected_release_digest: 'a'.repeat(64),
      destination_path: 'C:\\exports\\status-board.nomifun-plugin',
      include_source: true,
    };
    const operation = await pluginRuntimes.share.invoke(share);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/share`);
    expect(requestBody).toEqual(share);
    expect(operation.owner).toEqual({
      owner: 'plugin_runtime',
      plugin_id: MINIAPP_ID,
    });

    const importShare = {
      expected_library_revision: 8,
      source_path: 'C:\\imports\\status-board.nomifun-plugin',
      expected_bundle_digest: 'b'.repeat(64),
      expected_release_digest: 'c'.repeat(64),
      display_name: 'Status Board Copy',
    };
    await pluginRuntimes.importShare.invoke(importShare);
    expect(requestPath).toBe('/api/plugins/runtimes/import/share');
    expect(requestBody).toEqual(importShare);

    const importArtifact = {
      expected_library_revision: 9,
      source_path: 'C:\\imports\\status-board-runtime',
      expected_artifact_digest: 'd'.repeat(64),
      display_name: 'Status Board Runtime',
    };
    await pluginRuntimes.importArtifact.invoke(importArtifact);
    expect(requestPath).toBe('/api/plugins/runtimes/import/artifact');
    expect(requestBody).toEqual(importArtifact);
  });

  test('Whole-App Backup export and import preserve exact CAS and metadata digest fields', async () => {
    let requestPath = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input, init) => {
      requestPath = new URL(String(input), 'http://127.0.0.1').pathname;
      requestBody =
        typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
      const responseData =
        requestPath === `/api/plugins/runtimes/${MINIAPP_ID}/backup`
          ? {
              operation_id: '0190f5fe-7c00-7a00-8000-0000000000b6',
              operation_revision: 1,
              kind: 'export',
              owner: { owner: 'plugin_runtime', plugin_id: MINIAPP_ID },
              state: 'succeeded',
              cancelable: false,
              progress_percent: 100,
              started_at_ms: 1_780_000_000_000,
              completed_at_ms: 1_780_000_000_001,
            }
          : rawWorkshop();
      return new Response(JSON.stringify({ success: true, data: responseData }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    const exportRequest = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 4,
      expected_lifecycle: 'disabled' as const,
      expected_pointer_revision: 7,
      expected_config_revision: 2,
      expected_credential_bindings_revision: 3,
      destination_path: 'C:\\exports\\status-board.backup',
    };
    const operation = await pluginRuntimes.exportBackup.invoke(exportRequest);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/backup`);
    expect(requestBody).toEqual(exportRequest);
    expect(operation.owner).toEqual({
      owner: 'plugin_runtime',
      plugin_id: MINIAPP_ID,
    });

    const importRequest = {
      expected_library_revision: 8,
      source_path: 'C:\\imports\\status-board.backup',
      expected_backup_metadata_digest: 'e'.repeat(64),
      display_name: 'Status Board Restored',
    };
    await pluginRuntimes.importBackup.invoke(importRequest);
    expect(requestPath).toBe('/api/plugins/runtimes/import/backup');
    expect(requestBody).toEqual(importRequest);
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
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 4,
      expected_pointer_revision: 5,
      expected_active_release_epoch: 6,
      ready_release_id: 'ready-release',
      expected_ready_release_digest: 'a'.repeat(64),
      expected_active_release_digest: 'b'.repeat(64),
      acknowledge_test_warning: false,
    };
    await pluginRuntimes.publish.invoke(publish);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/publish`);
    expect(requestBody).toEqual(publish);

    const rollback = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 5,
      expected_pointer_revision: 6,
      expected_active_release_epoch: 7,
      expected_current_release_digest: 'b'.repeat(64),
      previous_release_id: 'previous-release',
      expected_previous_release_digest: 'c'.repeat(64),
    };
    await pluginRuntimes.rollback.invoke(rollback);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/rollback`);
    expect(requestBody).toEqual(rollback);

    const enabled = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 6,
      expected_pointer_revision: 7,
      expected_active_release_digest: 'c'.repeat(64),
      enabled: true,
    };
    await pluginRuntimes.setEnabled.invoke(enabled);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/enabled`);
    expect(requestBody).toEqual(enabled);

    const publishMode = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 7,
      expected_pointer_revision: 8,
      mode: 'auto_ui_only' as const,
    };
    await pluginRuntimes.setPublishMode.invoke(publishMode);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/publish-mode`);
    expect(requestBody).toEqual(publishMode);

    const bridge = {
      plugin_id: MINIAPP_ID as never,
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
    await pluginRuntimes.bridge.invoke(bridge);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/surface/bridge`);
    expect(requestPath.includes(bridge.surface_capability)).toBe(false);
    expect(requestBody).toEqual({
      surface_capability: bridge.surface_capability,
      active_release_epoch: bridge.active_release_epoch,
      expected_release_digest: bridge.expected_release_digest,
      request: bridge.request,
    });

    const serviceBridge = {
      ...bridge,
      request: {
        call_id: 'call-service',
        target: {
          target: 'service' as const,
          method: 'status',
          payload: { verbose: true },
        },
      },
    };
    await pluginRuntimes.bridge.invoke(serviceBridge);
    expect(requestBody).toEqual({
      surface_capability: serviceBridge.surface_capability,
      active_release_epoch: serviceBridge.active_release_epoch,
      expected_release_digest: serviceBridge.expected_release_digest,
      request: serviceBridge.request,
    });

    const serviceLifecycle = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 10,
      expected_pointer_revision: 11,
      expected_active_release_epoch: 12,
      expected_active_release_digest: 'e'.repeat(64),
      running: true,
    };
    await pluginRuntimes.setServiceRunning.invoke(serviceLifecycle);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/service/running`);
    expect(requestBody).toEqual(serviceLifecycle);

    const retryService = {
      plugin_id: MINIAPP_ID as never,
      expected_product_revision: 11,
      expected_pointer_revision: 12,
      expected_active_release_epoch: 13,
      expected_active_release_digest: 'f'.repeat(64),
    };
    await pluginRuntimes.retryService.invoke(retryService);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/service/retry`);
    expect(requestBody).toEqual(retryService);
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
            plugin_id: MINIAPP_ID,
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
      plugin_id: MINIAPP_ID as never,
    };
    const descriptor = await pluginRuntimes.openSurface.invoke(request);
    expect(requestMethod).toBe('POST');
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/surface/open`);
    expect(requestBody).toEqual(request);
    expect(descriptor.plugin_id).toBe(MINIAPP_ID);
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
      plugin_id: MINIAPP_ID as never,
      surface_session_id: '0190f5fe-7c00-7a00-8000-0000000000b4',
      surface_capability: 'surface-capability',
    };
    expect(await pluginRuntimes.closeSurface.invoke(request)).toBe(true);
    expect(requestPath).toBe(`/api/plugins/runtimes/${MINIAPP_ID}/surface/close`);
    expect(JSON.parse(requestBody)).toEqual(request);
  });
});
