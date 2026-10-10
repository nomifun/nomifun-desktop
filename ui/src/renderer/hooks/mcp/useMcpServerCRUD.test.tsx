import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { PropsWithChildren } from 'react';
import { mcpService } from '@/common/adapter/ipcBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import { subscribeMcpCatalogChanged } from './catalog';
import { useMcpServerCRUD } from './useMcpServerCRUD';

const locale = createInstance();
const restores: Array<() => void> = [];
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
});
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((dispose) => dispose());
});
const wrapper = ({ children }: PropsWithChildren) => <I18nextProvider i18n={locale}>{children}</I18nextProvider>;
const server = (overrides: Partial<IMcpServer> = {}): IMcpServer => ({
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000069'),
  name: 'alpha',
  enabled: false,
  transport: { type: 'stdio', command: 'fixture-command' },
  original_json: '{}',
  created_at: 1,
  updated_at: 1,
  ...overrides,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function fixture(initial = server()) {
  let servers = [initial];
  const save = mock(async (updater: IMcpServer[] | ((previous: IMcpServer[]) => IMcpServer[])) => {
    servers = typeof updater === 'function' ? updater(servers) : updater;
  });
  const changed = mock(() => {});
  restores.push(subscribeMcpCatalogChanged(changed));
  const errors = spyOn(Message, 'error').mockReturnValue(() => {});
  const tests = spyOn(mcpService.testMcpConnection, 'invoke').mockResolvedValue({ success: true });
  restores.push(() => errors.mockRestore(), () => tests.mockRestore());
  return {
    ...renderHook(() => useMcpServerCRUD(save), { wrapper }),
    save, changed, errors, tests,
    get servers() { return servers; },
    replace(next: IMcpServer[]) { servers = next; },
  };
}

describe('MCP enabled state persistence', () => {
  test('guards duplicate toggles, waits for persisted state and preserves a newer detection result', async () => {
    const request = deferred<IMcpServer>();
    const toggle = spyOn(mcpService.toggleServer, 'invoke').mockImplementation(() => request.promise);
    restores.push(() => toggle.mockRestore());
    const f = fixture();
    let pending!: Promise<IMcpServer | undefined>;
    let duplicate!: Promise<IMcpServer | undefined>;
    act(() => {
      pending = f.result.current.handleToggleMcpServer(f.servers[0]!);
      duplicate = f.result.current.handleToggleMcpServer(f.servers[0]!);
    });
    expect(toggle).toHaveBeenCalledTimes(1);
    expect(f.result.current.togglingServers[f.servers[0]!.mcp_server_id]).toBe(true);
    expect(f.servers[0]!.enabled).toBe(false);
    expect(f.changed).not.toHaveBeenCalled();
    expect(await duplicate).toBeUndefined();

    f.replace([server({ enabled: true, last_test_status: 'connected', tools: [{ name: 'new-tool' }], last_connected: 50, updated_at: 50 })]);
    await act(async () => {
      request.resolve(server({ enabled: true, updated_at: 20 }));
      await pending;
    });
    expect(f.servers[0]).toMatchObject({
      enabled: true, last_test_status: 'connected', tools: [{ name: 'new-tool' }], last_connected: 50, updated_at: 50,
    });
    expect(f.result.current.togglingServers[f.servers[0]!.mcp_server_id]).toBeUndefined();
    expect(f.changed).toHaveBeenCalledTimes(1);
    expect(f.tests).not.toHaveBeenCalled();
    expect(f.errors).not.toHaveBeenCalled();
  });

  test('a late toggle response cannot overwrite a newer canonical enabled state', async () => {
    const request = deferred<IMcpServer>();
    const toggle = spyOn(mcpService.toggleServer, 'invoke').mockImplementation(() => request.promise);
    restores.push(() => toggle.mockRestore());
    const f = fixture(server({ enabled: true }));
    let pending!: Promise<IMcpServer | undefined>;
    act(() => { pending = f.result.current.handleToggleMcpServer(f.servers[0]!); });
    const newer = server({ enabled: true, updated_at: 50, tools: [{ name: 'new-tool' }] });
    f.replace([newer]);
    await act(async () => { request.resolve(server({ enabled: false, updated_at: 20 })); await pending; });
    expect(f.servers).toEqual([newer]);
    expect(f.changed).toHaveBeenCalledTimes(1);
  });

  test('uses the backend response even when it does not match the requested flip', async () => {
    const toggle = spyOn(mcpService.toggleServer, 'invoke').mockResolvedValue(server({
      enabled: false, updated_at: 10, last_test_status: 'connected', tools: [{ name: 'persisted-tool' }],
    }));
    restores.push(() => toggle.mockRestore());
    const f = fixture();
    await act(async () => { await f.result.current.handleToggleMcpServer(f.servers[0]!); });
    expect(f.servers[0]!.enabled).toBe(false);
    expect(f.servers[0]!.updated_at).toBe(10);
    expect(f.servers[0]!.tools).toEqual([{ name: 'persisted-tool' }]);
    expect(f.servers[0]!.last_test_status).toBe('connected');
    expect(f.changed).toHaveBeenCalledTimes(1);
  });

  test('disables without discarding discovered tools', async () => {
    const connected = server({ enabled: true, last_test_status: 'connected', tools: [{ name: 'alpha-tool' }] });
    const toggle = spyOn(mcpService.toggleServer, 'invoke').mockResolvedValue({ ...connected, enabled: false });
    restores.push(() => toggle.mockRestore());
    const f = fixture(connected);
    await act(async () => { await f.result.current.handleToggleMcpServer(connected); });
    expect(f.servers[0]).toMatchObject({ enabled: false, last_test_status: 'connected', tools: [{ name: 'alpha-tool' }] });
    expect(f.tests).not.toHaveBeenCalled();
  });

  test('editing uses canonical cleared detection fields and notifies after updating local state', async () => {
    const previous = server({ last_test_status: 'connected', tools: [{ name: 'obsolete-tool' }], last_connected: 5 });
    const persisted = server({ transport: { type: 'stdio', command: 'edited-command' }, updated_at: 10 });
    const update = spyOn(mcpService.updateServer, 'invoke').mockResolvedValue(persisted);
    const toggle = spyOn(mcpService.toggleServer, 'invoke').mockResolvedValue(persisted);
    const success = spyOn(Message, 'success').mockReturnValue(() => {});
    restores.push(() => update.mockRestore(), () => toggle.mockRestore(), () => success.mockRestore());
    const f = fixture(previous);
    const observedAtNotification: IMcpServer[][] = [];
    f.changed.mockImplementation(() => { observedAtNotification.push([...f.servers]); });
    await act(async () => { await f.result.current.handleEditMcpServer(previous, { ...previous, transport: persisted.transport }); });
    expect(f.servers).toEqual([persisted]);
    expect(f.servers[0]!.tools).toBeUndefined();
    expect(f.servers[0]!.last_test_status).toBeUndefined();
    expect(f.servers[0]!.last_connected).toBeUndefined();
    expect(observedAtNotification).toEqual([[persisted]]);
    expect(toggle).not.toHaveBeenCalled();
    expect(f.tests).not.toHaveBeenCalled();
  });

  test.each([
    [new BackendHttpError({ method: 'POST', path: '/api/mcp/servers/alpha/toggle', status: 503,
      body: { code: 'MCP_STORAGE_UNAVAILABLE', error: 'MCP catalog is unavailable' } }), 'MCP catalog is unavailable'],
    [{}, settings.mcpToggleFailed],
  ])('a failed toggle retains the current state and allows a retry', async (error, expected) => {
    const request = deferred<IMcpServer>();
    const toggle = spyOn(mcpService.toggleServer, 'invoke').mockImplementationOnce(() => request.promise).mockResolvedValue(server({ enabled: true }));
    restores.push(() => toggle.mockRestore());
    const f = fixture();
    let pending!: Promise<IMcpServer | undefined>;
    act(() => { pending = f.result.current.handleToggleMcpServer(f.servers[0]!); });
    await act(async () => { request.reject(error); await pending; });
    expect(f.servers[0]!.enabled).toBe(false);
    expect(f.save).not.toHaveBeenCalled();
    expect(f.changed).toHaveBeenCalledTimes(1);
    expect(f.errors).toHaveBeenLastCalledWith(expected);
    expect(f.result.current.togglingServers[f.servers[0]!.mcp_server_id]).toBeUndefined();
    await act(async () => { await f.result.current.handleToggleMcpServer(f.servers[0]!); });
    expect(toggle).toHaveBeenCalledTimes(2);
    expect(f.servers[0]!.enabled).toBe(true);
    expect(f.changed).toHaveBeenCalledTimes(2);
  });
});
