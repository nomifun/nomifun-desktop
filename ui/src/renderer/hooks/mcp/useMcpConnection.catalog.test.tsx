import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import type { ReactNode } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import { useSessionCapabilityCatalog } from '@/renderer/components/chat/SessionCapabilityPicker/useSessionCapabilityCatalog';
import { subscribeMcpCatalogChanged } from './catalog';
import { useMcpConnection } from './useMcpConnection';
import { useMcpServers } from './useMcpServers';
import { useMcpServerCRUD } from './useMcpServerCRUD';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} });
const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => {
  restores.push(() => spy.mockRestore());
  return spy;
};
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

const server: IMcpServer = {
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000301'),
  name: 'probe-server', enabled: true,
  transport: { type: 'http', url: 'https://mcp.example.test/mcp' },
  last_test_status: 'disconnected', created_at: 1, updated_at: 1, original_json: '{}',
};
type ProbeResult = Awaited<ReturnType<typeof mcpService.testMcpConnection.invoke>>;
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};

for (const outcome of [
  { label: 'success', result: { success: true, tools: [{ name: 'lookup' }] } },
  { label: 'failed response', result: { success: false, error: 'Unavailable' } },
  { label: 'authentication required', result: { success: false, needs_auth: true, auth_method: 'oauth' } },
] satisfies Array<{ label: string; result: ProbeResult }>) {
  test(`only a completed ${outcome.label} probe invalidates the catalog`, async () => {
    const pending = deferred<ProbeResult>();
    track(spyOn(mcpService.testMcpConnection, 'invoke').mockImplementation(() => pending.promise));
    const changed = mock(() => {});
    restores.push(subscribeMcpCatalogChanged(changed));
    const authRequired = mock(() => {});
    const hook = renderHook(() => useMcpConnection(authRequired), { wrapper });
    let probe!: Promise<void>;
    act(() => { probe = hook.result.current.handleTestMcpConnection(server, { notify: false }); });
    expect(hook.result.current.testingServers[server.mcp_server_id]).toBe(true);
    expect(changed).toHaveBeenCalledTimes(0);
    await act(async () => { pending.resolve(outcome.result); await probe; });
    expect(changed).toHaveBeenCalledTimes(1);
    expect(hook.result.current.testingServers[server.mcp_server_id]).toBe(false);
    expect(authRequired).toHaveBeenCalledTimes(outcome.label === 'authentication required' ? 1 : 0);
  });
}

test('a persisted HTTP failure reloads canonical status and clears stale tools', async () => {
  const original = { ...server, last_test_status: 'connected' as const, tools: [{ name: 'old-tool' }] };
  const failed = { ...server, last_test_status: 'error' as const, tools: [], updated_at: 2 };
  const list = track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([original]));
  track(spyOn(mcpService.testMcpConnection, 'invoke').mockImplementation(async () => {
    list.mockResolvedValue([failed]);
    throw new BackendHttpError({
      method: 'POST', path: '/api/mcp/test-connection', status: 422,
      body: { code: 'MCP_COMMAND_NOT_FOUND', error: 'uvx not found' },
    });
  }));
  const hook = renderHook(() => ({ catalog: useMcpServers(), connection: useMcpConnection() }), { wrapper });
  await waitFor(() => expect(hook.result.current.catalog.mcpServers).toEqual([original]));
  await act(async () => { await hook.result.current.connection.handleTestMcpConnection(original, { notify: false }); });
  await waitFor(() => expect(hook.result.current.catalog.mcpServers).toEqual([failed]));
});

test('a stale probe error preserves the newer canonical successful row', async () => {
  const newer = { ...server, last_test_status: 'connected' as const, tools: [{ name: 'newest-tool' }], updated_at: 20 };
  track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([newer]));
  track(spyOn(mcpService.testMcpConnection, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'POST', path: '/api/mcp/test-connection', status: 409,
    body: { code: 'MCP_PROBE_STALE', error: 'Probe revision is stale' },
  })));
  const hook = renderHook(() => ({ catalog: useMcpServers(), connection: useMcpConnection() }), { wrapper });
  await waitFor(() => expect(hook.result.current.catalog.mcpServers).toEqual([newer]));
  await act(async () => { await hook.result.current.connection.handleTestMcpConnection(server, { notify: false }); });
  expect(hook.result.current.catalog.mcpServers).toEqual([newer]);
});

test('a probe completed after management unmount still invalidates other mounted views', async () => {
  const pending = deferred<ProbeResult>();
  track(spyOn(mcpService.testMcpConnection, 'invoke').mockImplementation(() => pending.promise));
  const changed = mock(() => {});
  restores.push(subscribeMcpCatalogChanged(changed));
  const hook = renderHook(() => useMcpConnection(), { wrapper });
  let probe!: Promise<void>;
  act(() => { probe = hook.result.current.handleTestMcpConnection(server, { notify: false }); });
  hook.unmount();
  await act(async () => { pending.resolve({ success: true }); await probe; });
  expect(changed).toHaveBeenCalledTimes(1);
});

test('a saved mutation completed after management unmount still refreshes a mounted session catalog', async () => {
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]));
  const list = track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([]));
  const pending = deferred<IMcpServer>();
  track(spyOn(mcpService.createServer, 'invoke').mockImplementation(() => pending.promise));
  const management = renderHook(() => {
    const catalog = useMcpServers();
    return { catalog, crud: useMcpServerCRUD(catalog.saveMcpServers) };
  }, { wrapper });
  const session = renderHook(() => useSessionCapabilityCatalog());
  await waitFor(() => expect(session.result.current.loading).toBe(false));
  await waitFor(() => expect(management.result.current.catalog.isMcpServersLoading).toBe(false));
  let mutation!: ReturnType<typeof management.result.current.crud.handleAddMcpServer>;
  act(() => {
    mutation = management.result.current.crud.handleAddMcpServer({
      name: server.name, enabled: server.enabled, transport: server.transport,
      original_json: server.original_json, last_test_status: server.last_test_status,
    });
  });
  management.unmount();
  list.mockResolvedValue([server]);
  await act(async () => { pending.resolve(server); await mutation; });
  await waitFor(() => expect(session.result.current.catalog.mcpServers).toEqual([server]));
});

test('placeholder configuration never starts a probe or invalidates saved catalogs', async () => {
  const probe = track(spyOn(mcpService.testMcpConnection, 'invoke'));
  const changed = mock(() => {});
  restores.push(subscribeMcpCatalogChanged(changed));
  const hook = renderHook(() => useMcpConnection(), { wrapper });
  await act(async () => {
    await hook.result.current.handleTestMcpConnection({
      ...server, transport: { type: 'http', url: 'https://mcp.example.test/mcp', headers: { Authorization: 'Bearer YOUR_API_KEY' } },
    }, { notify: false });
  });
  expect(probe).toHaveBeenCalledTimes(0);
  expect(changed).toHaveBeenCalledTimes(0);
});

test('overlapping probes keep the busy indicator until every request settles', async () => {
  const first = deferred<ProbeResult>();
  const second = deferred<ProbeResult>();
  track(spyOn(mcpService.testMcpConnection, 'invoke')
    .mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise));
  const hook = renderHook(() => useMcpConnection(), { wrapper });
  let firstProbe!: Promise<void>;
  let secondProbe!: Promise<void>;
  act(() => {
    firstProbe = hook.result.current.handleTestMcpConnection(server, { notify: false });
    secondProbe = hook.result.current.handleTestMcpConnection(server, { notify: false });
  });
  await act(async () => { first.resolve({ success: true }); await firstProbe; });
  expect(hook.result.current.testingServers[server.mcp_server_id]).toBe(true);
  await act(async () => { second.resolve({ success: false }); await secondProbe; });
  expect(hook.result.current.testingServers[server.mcp_server_id]).toBe(false);
});
