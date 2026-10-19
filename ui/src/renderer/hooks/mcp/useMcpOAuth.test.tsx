import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { mcpService, shell } from '@/common/adapter/ipcBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { IMcpServer } from '@/common/config/storage';
import { parseMcpServerId } from '@/common/types/ids';
import type { McpAuthDiagnostic, McpAuthStatus, McpOAuthAttempt } from '@/common/types/mcpAuth';
import { useMcpOAuth, validatedMcpAuthorizationUrl } from './useMcpOAuth';

const server: IMcpServer = {
  mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000501'), name: 'canva', enabled: true,
  transport: { type: 'http', url: 'https://mcp.canva.com/mcp' }, original_json: '{}', created_at: 1, updated_at: 1,
};
const waiting: McpOAuthAttempt = { attempt_id: 'attempt-1', status: 'waiting_for_user', authorization_url: 'https://auth.example.test/authorize?state=fixture' };
const authorized: McpAuthStatus = { state: 'authorized', granted_scopes: ['mcp:connect'] };
const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T): T => { restores.push(() => spy.mockRestore()); return spy; };
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); delete window.__backendPort; });

function browser() {
  const replace = mock();
  const close = mock();
  const popup = { opener: undefined, location: { replace }, close } as unknown as Window;
  const open = track(spyOn(window, 'open').mockReturnValue(popup));
  return { replace, close, open };
}

test('reserves the browser tab during the click, polls by server identity, then checks auth without claiming connectivity', async () => {
  const popup = browser();
  const start = deferred<McpOAuthAttempt>();
  const startApi = track(spyOn(mcpService.startMcpOAuth, 'invoke').mockReturnValue(start.promise));
  track(spyOn(mcpService.getMcpOAuthAttempt, 'invoke').mockResolvedValue({ ...waiting, status: 'authorized', authorization_url: undefined }));
  const status = track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue(authorized));
  const probe = track(spyOn(mcpService.testMcpConnection, 'invoke'));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 1 }));
  let login!: ReturnType<typeof hook.result.current.login>;
  act(() => { login = hook.result.current.login(server); });
  expect(popup.open).toHaveBeenCalledWith('about:blank', '_blank');
  expect(popup.replace).not.toHaveBeenCalled();
  expect(startApi).toHaveBeenCalledWith({ mcp_server_id: server.mcp_server_id, client: undefined });
  await act(async () => { start.resolve(waiting); expect((await login).success).toBe(true); });
  expect(popup.replace).toHaveBeenCalledWith(waiting.authorization_url);
  expect(status).toHaveBeenCalledWith({ mcp_server_id: server.mcp_server_id });
  expect(hook.result.current.oauthStatus[server.mcp_server_id]?.auth).toEqual(authorized);
  expect(probe).not.toHaveBeenCalled();
});

test('desktop opens on the user shell and does not create a remote WebUI popup', async () => {
  window.__backendPort = 47000;
  const popup = track(spyOn(window, 'open'));
  const openShell = track(spyOn(shell.openExternal, 'invoke').mockResolvedValue(undefined));
  track(spyOn(mcpService.startMcpOAuth, 'invoke').mockResolvedValue(waiting));
  track(spyOn(mcpService.getMcpOAuthAttempt, 'invoke').mockResolvedValue({ ...waiting, status: 'authorized' }));
  track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue(authorized));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 1 }));
  await act(async () => { await hook.result.current.login(server); });
  expect(popup).not.toHaveBeenCalled();
  expect(openShell).toHaveBeenCalledWith(waiting.authorization_url);
});

test('cancelling before discovery returns fences the late attempt and never navigates', async () => {
  const popup = browser();
  const start = deferred<McpOAuthAttempt>();
  track(spyOn(mcpService.startMcpOAuth, 'invoke').mockReturnValue(start.promise));
  const cancel = track(spyOn(mcpService.cancelMcpOAuth, 'invoke').mockResolvedValue(undefined));
  track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue({ state: 'required', granted_scopes: [] }));
  const poll = track(spyOn(mcpService.getMcpOAuthAttempt, 'invoke'));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 1 }));
  let login!: ReturnType<typeof hook.result.current.login>;
  act(() => { login = hook.result.current.login(server); });
  await act(async () => { await hook.result.current.cancelLogin(server); });
  await act(async () => { start.resolve(waiting); expect((await login).cancelled).toBe(true); });
  expect(cancel).toHaveBeenCalledWith({ mcp_server_id: server.mcp_server_id, attempt_id: waiting.attempt_id });
  expect(popup.replace).not.toHaveBeenCalled();
  expect(poll).not.toHaveBeenCalled();
});

test('cancel wakes polling and a late authorized response cannot revive local login', async () => {
  browser();
  track(spyOn(mcpService.startMcpOAuth, 'invoke').mockResolvedValue(waiting));
  const result = deferred<McpOAuthAttempt>();
  const poll = track(spyOn(mcpService.getMcpOAuthAttempt, 'invoke').mockReturnValue(result.promise));
  track(spyOn(mcpService.cancelMcpOAuth, 'invoke').mockResolvedValue(undefined));
  track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue({ state: 'required', granted_scopes: [] }));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 1 }));
  let login!: ReturnType<typeof hook.result.current.login>;
  act(() => { login = hook.result.current.login(server); });
  await waitFor(() => expect(poll).toHaveBeenCalled());
  await act(async () => { await hook.result.current.cancelLogin(server); expect((await login).cancelled).toBe(true); });
  await act(async () => { result.resolve({ ...waiting, status: 'authorized' }); });
  expect(hook.result.current.oauthStatus[server.mcp_server_id]?.auth?.state).toBe('required');
  expect(hook.result.current.loggingIn[server.mcp_server_id]).toBe(false);
});

test('configuration-required diagnostics and client secrets stay in their intended channels', async () => {
  browser();
  const diagnostic = { code: 'CLIENT_REGISTRATION_REQUIRED', stage: 'client_registration' as const, retryable: false, user_action: 'configure_client' as const, trace_id: 'trace-1', message: 'Register this client with the provider.' };
  const start = track(spyOn(mcpService.startMcpOAuth, 'invoke').mockResolvedValue({ attempt_id: 'attempt-2', status: 'failed', diagnostic }));
  track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue({ state: 'configuration_required', granted_scopes: [], diagnostic }));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 1 }));
  const client = { mode: 'pre_registered' as const, client_id: 'approved', client_secret: 'fixture-private-secret' };
  await act(async () => { expect((await hook.result.current.login(server, client)).diagnostic).toEqual(diagnostic); });
  expect(start).toHaveBeenCalledWith({ mcp_server_id: server.mcp_server_id, client });
  expect(JSON.stringify(hook.result.current)).not.toContain('fixture-private-secret');
  expect(server.original_json).toBe('{}');
});

test('authorization URLs reject executable schemes, public plaintext hosts, and userinfo', () => {
  for (const url of ['javascript:alert(1)', 'http://auth.example.test/login', 'https://user:secret@example.test/auth']) {
    expect(() => validatedMcpAuthorizationUrl(url)).toThrow();
  }
  expect(validatedMcpAuthorizationUrl('https://auth.example.test/auth')).toBe('https://auth.example.test/auth');
  expect(validatedMcpAuthorizationUrl('http://127.0.0.1:4321/auth')).toBe('http://127.0.0.1:4321/auth');
});

test('a start rejection before attempt creation preserves typed actions and rejects lookalike raw error details', async () => {
  const popup = browser();
  const diagnostic: McpAuthDiagnostic = {
    code: 'INVALID_CONFIGURATION', stage: 'authorization', retryable: false, user_action: 'configure_client',
    trace_id: 'start-http-trace', message: 'Use a registered OAuth client for this connection.',
  };
  track(spyOn(mcpService.startMcpOAuth, 'invoke')
    .mockRejectedValueOnce(new BackendHttpError({ method: 'POST', path: '/api/mcp/servers/fixture/auth/attempts', status: 400,
      body: { code: diagnostic.code, error: 'outer-error-must-not-be-shown', details: diagnostic } }))
    .mockRejectedValueOnce(new BackendHttpError({ method: 'POST', path: '/api/mcp/servers/fixture/auth/attempts', status: 500,
      body: { code: 'INVALID_CONFIGURATION', error: 'unsafe-untyped-body', details: { ...diagnostic, retryable: 'false', message: 'unsafe-untyped-message' } } })));
  track(spyOn(mcpService.getMcpAuth, 'invoke').mockResolvedValue({ state: 'anonymous', granted_scopes: [] }));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 1 }));
  await act(async () => {
    const result = await hook.result.current.login(server);
    expect(result.diagnostic).toEqual(diagnostic);
    expect(result.error).toBe(diagnostic.message);
  });
  expect(hook.result.current.oauthStatus[server.mcp_server_id]?.diagnostic).toEqual(diagnostic);
  expect(popup.close).toHaveBeenCalled();
  await act(async () => {
    const result = await hook.result.current.login(server);
    expect(result.diagnostic).toBeUndefined();
    expect(result.error).not.toContain('unsafe-untyped');
    expect(result.error).not.toContain('outer-error');
  });
  expect(JSON.stringify(hook.result.current.oauthStatus)).not.toContain('unsafe-untyped');
});

test('status, cancellation, and logout retain authored HTTP diagnostics without pretending the operation succeeded', async () => {
  browser();
  const diagnostic: McpAuthDiagnostic = {
    code: 'SECRET_STORAGE_FAILED', stage: 'storage', retryable: false, user_action: 'none',
    trace_id: 'storage-http-trace', message: 'The MCP credential store is unavailable.',
  };
  const failure = () => new BackendHttpError({ method: 'DELETE', path: '/api/mcp/servers/fixture/auth', status: 500,
    body: { code: diagnostic.code, error: 'outer-storage-error', details: diagnostic } });
  const status = track(spyOn(mcpService.getMcpAuth, 'invoke').mockRejectedValue(failure()));
  track(spyOn(mcpService.logoutMcpOAuth, 'invoke').mockRejectedValue(failure()));
  track(spyOn(mcpService.startMcpOAuth, 'invoke').mockResolvedValue(waiting));
  const cancel = track(spyOn(mcpService.cancelMcpOAuth, 'invoke').mockRejectedValue(failure()));
  const hook = renderHook(() => useMcpOAuth({ pollIntervalMs: 60_000 }));
  await act(async () => { await hook.result.current.checkOAuthStatus(server); });
  expect(hook.result.current.oauthStatus[server.mcp_server_id]?.diagnostic).toEqual(diagnostic);
  expect(hook.result.current.oauthStatus[server.mcp_server_id]?.error).toBe(diagnostic.message);
  await act(async () => {
    const result = await hook.result.current.logout(server);
    expect(result.success).toBe(false);
    expect(result.diagnostic).toEqual(diagnostic);
    expect(result.error).toBe(diagnostic.message);
  });
  let login!: ReturnType<typeof hook.result.current.login>;
  act(() => { login = hook.result.current.login(server); });
  await waitFor(() => expect(hook.result.current.attempts[server.mcp_server_id]?.attempt_id).toBe(waiting.attempt_id));
  await act(async () => {
    const result = await hook.result.current.cancelLogin(server);
    expect(result.success).toBe(false);
    expect(result.diagnostic).toEqual(diagnostic);
  });
  expect(hook.result.current.loggingIn[server.mcp_server_id]).toBe(true);
  expect(hook.result.current.oauthStatus[server.mcp_server_id]?.diagnostic?.trace_id).toBe('storage-http-trace');
  cancel.mockResolvedValue(undefined);
  status.mockResolvedValue({ state: 'required', granted_scopes: [] });
  await act(async () => { await hook.result.current.cancelLogin(server); expect((await login).cancelled).toBe(true); });
});
