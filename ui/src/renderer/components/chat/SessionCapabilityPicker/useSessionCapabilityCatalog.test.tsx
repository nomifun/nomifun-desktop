import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { OfficeStatus } from '@/common/types/officeHub';
import * as mcpCatalog from '@/renderer/hooks/mcp/catalog';
import { useSessionCapabilityCatalog } from './useSessionCapabilityCatalog';
import { defaultSessionCapabilityDraft } from './model';

const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('catalog retains unavailable packages for inspection while auto defaults exclude them', async () => {
  const bad = { name: 'bad-auto', description: 'Auto instructions', location: '', is_custom: false, source: 'builtin' as const, session_available: false, session_error: 'Resource exceeds the size limit' };
  const good = { name: 'good-auto', description: 'Valid instructions', location: '', is_custom: false, source: 'builtin' as const };
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([bad, good]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([bad, good]));
  track(spyOn(officeHub, 'status').mockRejectedValue(new Error('offline')));
  track(spyOn(mcpCatalog, 'ensureBackendMcpCatalog').mockResolvedValue({ allServers: [], builtinServers: [], userServers: [] }));
  track(spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([{ app_id: '0190f5fe-7c00-7a00-8000-000000000077', name: 'My app',
    url: 'http://127.0.0.1:4317/mcp', state: 'running', connection_status: 'connected', tools: [{ name: 'read' }], reason: null }]));
  const hook = renderHook(() => useSessionCapabilityCatalog());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  expect(hook.result.current.error).toBeUndefined();
  expect(hook.result.current.catalog.skills).toHaveLength(2);
  expect(hook.result.current.catalog.skills.find((skill) => skill.name === 'bad-auto')?.session_error).toBe(bad.session_error);
  expect(defaultSessionCapabilityDraft(hook.result.current.catalog).skillNames).toEqual(['good-auto']);
  expect(hook.result.current.catalog.mcpServers[0].appSource).toBe(true);
  expect(defaultSessionCapabilityDraft(hook.result.current.catalog).mcpServerIds).toEqual([]);
});

test('MCP invalidation refreshes application sources without accepting a late older source response', async () => {
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(officeHub, 'status').mockRejectedValue(new Error('offline')));
  track(spyOn(mcpCatalog, 'ensureBackendMcpCatalog').mockResolvedValue({ allServers: [], builtinServers: [], userServers: [] }));
  const source = { app_id: '0190f5fe-7c00-7a00-8000-000000000077', name: 'My app',
    url: 'http://127.0.0.1:4317/mcp', state: 'running' as const, connection_status: 'connected' as const,
    tools: [{ name: 'read' }], reason: null };
  const sources = track(spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([source]));
  const hook = renderHook(() => useSessionCapabilityCatalog());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  let resolveOld!: (value: typeof source[]) => void;
  sources.mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }));
  act(() => mcpCatalog.notifyMcpCatalogChanged());
  sources.mockResolvedValue([{ ...source, name: 'Latest app' }]);
  await act(async () => mcpCatalog.notifyMcpCatalogChanged());
  await waitFor(() => expect(hook.result.current.catalog.mcpServers[0].name).toBe('Latest app'));
  await act(async () => resolveOld([source]));
  expect(hook.result.current.catalog.mcpServers[0].name).toBe('Latest app');
  expect(hook.result.current.catalog.mcpServers[0].appSource).toBe(true);
  expect(ipcBridge.fs.listAvailableSkills.invoke).toHaveBeenCalledTimes(1);
});

test('connector readiness reflects the office connection and fails closed', async () => {
  const status = (phase: OfficeStatus['phase'], enabled = true): OfficeStatus => ({
    provider: 'feishu',
    connection: { id: 'c', name: 'Work', enabled, profile: 'work', executable: null, created_at: '', updated_at: '' },
    runtime: { phase: 'ready', executable: '/x', version: '1.0.98', source: 'external', message: null, official_install_command: '' },
    phase, actor: phase === 'ready' ? { app_id: 'a', user_id: 'u', name: 'Ada' } : null,
    host_name: 'host', host_os: 'windows', message: null, permissions: [],
  });
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(mcpCatalog, 'ensureBackendMcpCatalog').mockResolvedValue({ allServers: [], builtinServers: [], userServers: [] }));
  track(spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]));
  track(spyOn(officeHub, 'status').mockResolvedValue(status('ready')));
  const hook = renderHook(() => useSessionCapabilityCatalog());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  expect(hook.result.current.catalog.connectorsReady?.feishu).toBe(true);
});

