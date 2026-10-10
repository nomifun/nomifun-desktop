import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import { useSessionCapabilityCatalog } from '@/renderer/components/chat/SessionCapabilityPicker/useSessionCapabilityCatalog';
import { notifyMcpCatalogChanged } from './catalog';
import { useMcpServers } from './useMcpServers';

const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => {
  restores.push(() => spy.mockRestore());
  return spy;
};
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

const server = (extra: Partial<IMcpServer> = {}): IMcpServer => ({
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000201'),
  name: 'market-server', enabled: false,
  transport: { type: 'stdio', command: 'npx', args: ['example-mcp'] },
  last_test_status: 'disconnected', created_at: 1, updated_at: 1, original_json: '{}',
  ...extra,
});

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};

const mountCatalogs = () => {
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]));
  return renderHook(() => ({ management: useMcpServers(), session: useSessionCapabilityCatalog() }));
};

test('saved catalog invalidation refreshes management and mounted session views from the backend', async () => {
  const list = track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([server()]));
  const hook = mountCatalogs();
  await waitFor(() => expect(hook.result.current.session.loading).toBe(false));
  const updated = server({ enabled: true, last_test_status: 'connected', tools: [{ name: 'lookup' }], updated_at: 2 });
  const builtin = server({ mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000202'), name: 'builtin', builtin: true });
  list.mockResolvedValue([updated, builtin]);
  await act(async () => { notifyMcpCatalogChanged(); });
  await waitFor(() => expect(hook.result.current.management.mcpServers).toEqual([updated, builtin]));
  expect(hook.result.current.session.catalog.mcpServers).toEqual([updated]);
  expect(ipcBridge.fs.listAvailableSkills.invoke).toHaveBeenCalledTimes(1);
});

test('older refresh results cannot restore a disabled or failed row after a newer mutation', async () => {
  const list = track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([server()]));
  const hook = mountCatalogs();
  await waitFor(() => expect(hook.result.current.session.loading).toBe(false));
  const oldManagement = deferred<IMcpServer[]>();
  const oldSession = deferred<IMcpServer[]>();
  list.mockImplementationOnce(() => oldManagement.promise).mockImplementationOnce(() => oldSession.promise);
  act(() => { notifyMcpCatalogChanged(); });
  const updated = server({ enabled: true, last_test_status: 'connected', tools: [{ name: 'latest' }], updated_at: 5 });
  list.mockResolvedValue([updated]);
  await act(async () => { notifyMcpCatalogChanged(); });
  await waitFor(() => expect(hook.result.current.session.catalog.mcpServers).toEqual([updated]));
  await act(async () => {
    oldManagement.resolve([server({ last_test_status: 'error' })]);
    oldSession.resolve([server({ last_test_status: 'error' })]);
  });
  expect(hook.result.current.management.mcpServers).toEqual([updated]);
  expect(hook.result.current.session.catalog.mcpServers).toEqual([updated]);
});

test('invalidation also fences pending initial catalog reads', async () => {
  const oldManagement = deferred<IMcpServer[]>();
  const oldSession = deferred<IMcpServer[]>();
  const updated = server({ enabled: true, updated_at: 3 });
  track(spyOn(mcpService.listServers, 'invoke')
    .mockImplementationOnce(() => oldManagement.promise)
    .mockImplementationOnce(() => oldSession.promise)
    .mockResolvedValue([updated]));
  const hook = mountCatalogs();
  await act(async () => { notifyMcpCatalogChanged(); });
  await waitFor(() => expect(hook.result.current.session.loading).toBe(false));
  await act(async () => { oldManagement.resolve([server()]); oldSession.resolve([server()]); });
  expect(hook.result.current.management.mcpServers).toEqual([updated]);
  expect(hook.result.current.session.catalog.mcpServers).toEqual([updated]);
});

test('a local saved row invalidates older reads before the CRUD notification is sent', async () => {
  const list = track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([server()]));
  const hook = mountCatalogs();
  await waitFor(() => expect(hook.result.current.session.loading).toBe(false));
  const oldRead = deferred<IMcpServer[]>();
  list.mockImplementationOnce(() => oldRead.promise);
  act(() => { notifyMcpCatalogChanged(); });
  const updated = server({ enabled: true, updated_at: 4 });
  await act(async () => { await hook.result.current.management.saveMcpServers([updated]); });
  await act(async () => { oldRead.resolve([server()]); });
  expect(hook.result.current.management.mcpServers).toEqual([updated]);
  list.mockResolvedValue([updated]);
  await act(async () => { notifyMcpCatalogChanged(); });
  await waitFor(() => expect(hook.result.current.session.catalog.mcpServers).toEqual([updated]));
});

test('background failure retains catalog rows, reports the failure, and retries without stale overwrites', async () => {
  track(spyOn(console, 'error').mockImplementation(() => {}));
  const original = server({ enabled: true, last_test_status: 'connected', tools: [{ name: 'lookup' }] });
  const list = track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([original]));
  const hook = mountCatalogs();
  await waitFor(() => expect(hook.result.current.session.loading).toBe(false));
  const pendingManagement = deferred<IMcpServer[]>();
  const pendingSession = deferred<IMcpServer[]>();
  list.mockImplementationOnce(() => pendingManagement.promise).mockImplementationOnce(() => pendingSession.promise);
  act(() => { notifyMcpCatalogChanged(); });
  expect(hook.result.current.management.isMcpServersLoading).toBe(false);
  expect(hook.result.current.session.loading).toBe(false);
  expect(hook.result.current.session.catalog.mcpServers).toEqual([original]);
  list.mockRejectedValue(new Error('Temporary connection loss'));
  await act(async () => { notifyMcpCatalogChanged(); });
  expect(hook.result.current.management.mcpServers).toEqual([original]);
  expect(hook.result.current.management.mcpServersLoadFailed).toBe(true);
  expect(hook.result.current.session.catalog.mcpServers).toEqual([original]);
  expect(hook.result.current.session.error?.message).toBe('Temporary connection loss');
  const updated = server({ enabled: false, updated_at: 3 });
  const retryManagement = deferred<IMcpServer[]>();
  const retrySession = deferred<IMcpServer[]>();
  list.mockImplementationOnce(() => retryManagement.promise).mockImplementationOnce(() => retrySession.promise);
  act(() => {
    hook.result.current.management.retryMcpServers();
    hook.result.current.session.retry();
  });
  expect(hook.result.current.management.mcpServersLoadFailed).toBe(true);
  expect(hook.result.current.session.error?.message).toBe('Temporary connection loss');
  expect(hook.result.current.session.catalog.mcpServers).toEqual([original]);
  await act(async () => { retryManagement.resolve([updated]); retrySession.resolve([updated]); });
  await waitFor(() => expect(hook.result.current.session.catalog.mcpServers).toEqual([updated]));
  expect(hook.result.current.management.mcpServersLoadFailed).toBe(false);
  expect(hook.result.current.session.error).toBeUndefined();
  await act(async () => { pendingManagement.resolve([original]); pendingSession.resolve([original]); });
  expect(hook.result.current.management.mcpServers).toEqual([updated]);
});

test('initial errors remain explicit and both catalog hooks can retry', async () => {
  track(spyOn(console, 'error').mockImplementation(() => {}));
  const list = track(spyOn(mcpService.listServers, 'invoke').mockRejectedValue(new Error('Unavailable')));
  const hook = mountCatalogs();
  await waitFor(() => expect(hook.result.current.management.mcpServersLoadFailed).toBe(true));
  expect(hook.result.current.management.isMcpServersLoading).toBe(false);
  expect(hook.result.current.session.loading).toBe(false);
  expect(hook.result.current.session.error?.message).toBe('Unavailable');
  list.mockResolvedValue([server()]);
  await act(async () => {
    hook.result.current.management.retryMcpServers();
    hook.result.current.session.retry();
  });
  await waitFor(() => expect(hook.result.current.session.catalog.mcpServers).toEqual([server()]));
  expect(hook.result.current.management.mcpServersLoadFailed).toBe(false);
  expect(hook.result.current.session.error).toBeUndefined();
});

test('unmounted views unsubscribe and ignore pending responses', async () => {
  const pendingManagement = deferred<IMcpServer[]>();
  const pendingSession = deferred<IMcpServer[]>();
  const list = track(spyOn(mcpService.listServers, 'invoke')
    .mockImplementationOnce(() => pendingManagement.promise)
    .mockImplementationOnce(() => pendingSession.promise));
  const hook = mountCatalogs();
  expect(list).toHaveBeenCalledTimes(2);
  hook.unmount();
  await act(async () => {
    notifyMcpCatalogChanged();
    pendingManagement.resolve([server()]);
    pendingSession.resolve([server()]);
  });
  expect(list).toHaveBeenCalledTimes(2);
  expect(hook.result.current.management.mcpServers).toEqual([]);
  expect(hook.result.current.session.catalog.mcpServers).toEqual([]);
});
