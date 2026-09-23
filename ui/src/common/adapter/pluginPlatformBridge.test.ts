/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, test } from 'bun:test';
import type {
  ConfigurePluginRequest,
  DispatchPluginBridgeRequest,
  InstallPluginImportRequest,
} from '../types/pluginPlatform';
import { pluginPlatform } from './pluginPlatformBridge';

const realFetch = globalThis.fetch;
const draftId = 'draft-0190';
const pluginId = 'plugin-0190';

interface RecordedCall {
  method: string;
  path: string;
  body: unknown;
}

const calls: RecordedCall[] = [];

function installFetchFixture(): void {
  globalThis.fetch = (async (input, init) => {
    const path = new URL(String(input), 'http://127.0.0.1').pathname;
    const method = init?.method ?? 'GET';
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, body });

    const data = path === '/api/plugin-drafts'
      ? method === 'GET'
        ? { drafts: [] }
        : { summary: {}, messages: [], files: [] }
      : path === '/api/plugins'
        ? { revision: 1, plugins: [] }
        : path === '/api/plugins/library-state'
          ? { revision: 1, collections: [], items: [] }
          : {};
    return new Response(JSON.stringify({ success: true, data }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = realFetch;
  calls.length = 0;
});

describe('Unified Plugin Core bridge', () => {
  test('exposes one cohesive resource tree', () => {
    expect(Object.keys(pluginPlatform)).toEqual([
      'drafts',
      'plugins',
      'libraryState',
      'credentials',
      'desktop',
      'surface',
    ]);
    expect(Object.keys(pluginPlatform.drafts)).toEqual([
      'list',
      'create',
      'get',
      'generate',
      'cancelGeneration',
      'replaceFile',
      'deleteFile',
      'preview',
      'save',
      'delete',
    ]);
    expect(Object.keys(pluginPlatform.plugins)).toEqual([
      'list',
      'get',
      'inspectImport',
      'installImport',
      'setEnabled',
      'configure',
      'restore',
      'trash',
      'delete',
      'exportPackage',
      'exportBackup',
      'openSurface',
    ]);
    expect(Object.keys(pluginPlatform.surface)).toEqual(['close', 'bridge']);
    expect(Object.keys(pluginPlatform.credentials)).toEqual(['list']);
    expect(Object.keys(pluginPlatform.desktop)).toEqual(['commands', 'invoke', 'emit']);
  });

  test('uses only the canonical Draft resource and keeps identity out of bodies', async () => {
    installFetchFixture();

    await pluginPlatform.drafts.list.invoke();
    await pluginPlatform.drafts.create.invoke({ template: 'agent.before_tool' });
    await pluginPlatform.drafts.get.invoke({ draft_id: draftId });
    await pluginPlatform.drafts.generate.invoke({
      draft_id: draftId,
      request: {
        expected_revision: 1,
        provider_id: 'provider-1',
        model: 'model-1',
        requirement: 'Create a file organizer',
      },
    });
    await pluginPlatform.drafts.cancelGeneration.invoke({
      draft_id: draftId,
      request: { expected_revision: 2 },
    });
    await pluginPlatform.drafts.replaceFile.invoke({
      draft_id: draftId,
      request: {
        expected_revision: 3,
        path: 'ui/index.html',
        content_base64: 'PGgxPkhlbGxvPC9oMT4=',
      },
    });
    await pluginPlatform.drafts.deleteFile.invoke({
      draft_id: draftId,
      request: { expected_revision: 4, path: 'source/old.ts' },
    });
    await pluginPlatform.drafts.preview.invoke({
      draft_id: draftId,
      request: {
        expected_revision: 5,
        config: { theme: 'dark' },
        access: { permissions: [], credential_bindings: {} },
      },
    });
    await pluginPlatform.drafts.save.invoke({
      draft_id: draftId,
      request: {
        expected_revision: 6,
        expected_plugin_revision: 2,
        permission_confirmation_id: 'confirmation-1',
        config: { theme: 'dark' },
        credential_bindings: { api_key: 'provider:credential-1' },
      },
    });
    await pluginPlatform.drafts.delete.invoke({
      draft_id: draftId,
      request: { expected_revision: 7 },
    });

    expect(calls.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'GET', path: '/api/plugin-drafts' },
      { method: 'POST', path: '/api/plugin-drafts' },
      { method: 'GET', path: `/api/plugin-drafts/${draftId}` },
      { method: 'POST', path: `/api/plugin-drafts/${draftId}/generate` },
      { method: 'POST', path: `/api/plugin-drafts/${draftId}/cancel` },
      { method: 'PUT', path: `/api/plugin-drafts/${draftId}/files` },
      { method: 'DELETE', path: `/api/plugin-drafts/${draftId}/files` },
      { method: 'POST', path: `/api/plugin-drafts/${draftId}/preview` },
      { method: 'POST', path: `/api/plugin-drafts/${draftId}/save` },
      { method: 'DELETE', path: `/api/plugin-drafts/${draftId}` },
    ]);
    expect(calls.slice(3).every(({ body }) =>
      !JSON.stringify(body).includes('draft_id'))).toBe(true);
    expect(calls[8]?.body).toEqual({
      expected_revision: 6,
      expected_plugin_revision: 2,
      permission_confirmation_id: 'confirmation-1',
      config: { theme: 'dark' },
      credential_bindings: { api_key: 'provider:credential-1' },
    });
  });

  test('uses one Plugin resource for install, lifecycle, export and organization', async () => {
    installFetchFixture();
    const importRequest: InstallPluginImportRequest = {
      source_path: 'C:/plugins/todo.zip',
      kind: 'zip',
      expected_plugin_revision: 8,
      create_copy: false,
      permission_confirmation_id: 'confirmation-2',
      config: { color: 'blue' },
      credential_bindings: { api_key: 'provider:credential-2' },
    };
    const configRequest: ConfigurePluginRequest = {
      expected_revision: 9,
      config: { color: 'blue' },
      credential_bindings: { api_key: 'credential-0190' },
      grants: { network: true },
    };

    await pluginPlatform.plugins.list.invoke();
    await pluginPlatform.plugins.get.invoke({ plugin_id: pluginId });
    await pluginPlatform.plugins.inspectImport.invoke({
      source_path: 'C:/plugins/todo.zip',
      kind: 'zip',
    });
    await pluginPlatform.plugins.installImport.invoke(importRequest);
    await pluginPlatform.plugins.setEnabled.invoke({
      plugin_id: pluginId,
      request: { expected_revision: 9, enabled: false },
    });
    await pluginPlatform.plugins.configure.invoke({
      plugin_id: pluginId,
      request: configRequest,
    });
    await pluginPlatform.plugins.restore.invoke({
      plugin_id: pluginId,
      request: {
        expected_revision: 10,
        mode: 'previous_code_and_data',
        acknowledge_data_loss: true,
      },
    });
    await pluginPlatform.plugins.trash.invoke({
      plugin_id: pluginId,
      request: { expected_revision: 11 },
    });
    await pluginPlatform.plugins.exportPackage.invoke({
      plugin_id: pluginId,
      request: {
        expected_revision: 11,
        destination_path: 'C:/exports/todo.zip',
        include_source: true,
      },
    });
    await pluginPlatform.plugins.exportBackup.invoke({
      plugin_id: pluginId,
      request: {
        expected_revision: 11,
        destination_path: 'C:/exports/todo-backup.zip',
      },
    });
    await pluginPlatform.libraryState.get.invoke();
    await pluginPlatform.libraryState.update.invoke({
      expected_revision: 1,
      collections: [],
      items: [{ plugin_id: pluginId, pinned: true }],
    });
    await pluginPlatform.plugins.delete.invoke({
      plugin_id: pluginId,
      request: {
        expected_revision: 12,
        acknowledge_permanent_delete: true,
      },
    });

    expect(calls.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'GET', path: '/api/plugins' },
      { method: 'GET', path: `/api/plugins/${pluginId}` },
      { method: 'POST', path: '/api/plugins/import/inspect' },
      { method: 'POST', path: '/api/plugins/import' },
      { method: 'PUT', path: `/api/plugins/${pluginId}/enabled` },
      { method: 'PUT', path: `/api/plugins/${pluginId}/config` },
      { method: 'POST', path: `/api/plugins/${pluginId}/restore` },
      { method: 'POST', path: `/api/plugins/${pluginId}/trash` },
      { method: 'POST', path: `/api/plugins/${pluginId}/export` },
      { method: 'POST', path: `/api/plugins/${pluginId}/backup` },
      { method: 'GET', path: '/api/plugins/library-state' },
      { method: 'PUT', path: '/api/plugins/library-state' },
      { method: 'DELETE', path: `/api/plugins/${pluginId}` },
    ]);
    expect(calls[3]?.body).toEqual(importRequest);
    expect(JSON.stringify(calls[3]?.body)).not.toContain('digest');
    expect(calls[5]?.body).toEqual(configRequest);
    expect(JSON.stringify(calls[5]?.body)).not.toContain('secret_value');
    expect(
      JSON.stringify([4, 5, 6, 7, 8, 9, 12].map((index) => calls[index]?.body)),
    ).not.toContain('plugin_id');
  });

  test('shares one fenced Surface bridge between installed and preview sessions', async () => {
    installFetchFixture();
    const installedBridge: DispatchPluginBridgeRequest = {
      plugin_id: pluginId,
      artifact_digest: 'a'.repeat(64),
      surface_session_id: 'surface-installed',
      surface_generation: 3,
      is_preview: false,
      request: {
        call_id: 'call-1',
        target: {
          target: 'db',
          request: { operation: 'query', sql: 'SELECT 1', parameters: [] },
        },
      },
    };
    const previewBridge: DispatchPluginBridgeRequest = {
      draft_id: draftId,
      artifact_digest: 'b'.repeat(64),
      surface_session_id: 'surface-preview',
      surface_generation: 4,
      is_preview: true,
      request: { call_id: 'call-2', target: { target: 'config' } },
    };

    await pluginPlatform.plugins.openSurface.invoke({
      plugin_id: pluginId,
      request: { expected_revision: 5 },
    });
    await pluginPlatform.surface.bridge.invoke(installedBridge);
    await pluginPlatform.surface.bridge.invoke(previewBridge);
    await pluginPlatform.surface.close.invoke({
      plugin_id: pluginId,
      is_preview: false,
      request: {
        surface_session_id: 'surface-installed',
        surface_generation: 3,
      },
    });
    await pluginPlatform.surface.close.invoke({
      draft_id: draftId,
      is_preview: true,
      request: {
        surface_session_id: 'surface-preview',
        surface_generation: 4,
      },
    });

    expect(calls.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'POST', path: `/api/plugins/${pluginId}/surface/open` },
      { method: 'POST', path: `/api/plugins/${pluginId}/surface/bridge` },
      { method: 'POST', path: `/api/plugin-drafts/${draftId}/surface/bridge` },
      { method: 'POST', path: `/api/plugins/${pluginId}/surface/close` },
      { method: 'POST', path: `/api/plugin-drafts/${draftId}/surface/close` },
    ]);
    expect(calls[1]?.body).toEqual(installedBridge);
    expect(calls[2]?.body).toEqual(previewBridge);
    expect(calls[3]?.body).toEqual({
      surface_session_id: 'surface-installed',
      surface_generation: 3,
    });
  });

  test('contains no retired routes, identities, digest inputs, fallback or alias', () => {
    const bridge = readFileSync(
      new URL('./pluginPlatformBridge.ts', import.meta.url),
      'utf8',
    );
    const types = readFileSync(
      new URL('../types/pluginPlatform.ts', import.meta.url),
      'utf8',
    );
    for (const route of [
      '/runtimes',
      '/projects',
      '/installations',
      '/operations',
      '/authoring',
    ]) {
      expect(bridge).not.toContain(route);
    }
    for (const retiredType of [
      /PluginProject/,
      /PluginMount/,
      /PluginCandidate/,
      /PluginProduct/,
      /PluginReady/,
      /PluginPublish/,
      /PluginRuntime/,
    ]) {
      expect(types).not.toMatch(retiredType);
      expect(bridge).not.toMatch(retiredType);
    }
    expect(`${types}\n${bridge}`).not.toMatch(
      /expected_(?:artifact|bundle|candidate|release)_digest/,
    );
    expect(bridge).not.toContain('export const plugins');
    expect(bridge.match(/export const pluginPlatform/g)).toHaveLength(1);
    expect(bridge).not.toMatch(/fallback|legacy/i);
  });
});
