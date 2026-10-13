import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { useSystemPermissions } from './useSystemPermissions';

const status = (state: 'granted' | 'denied'): SystemPermissionStatus => ({ platform: 'macos', app_label: 'NomiFun',
  permissions: [{ kind: 'accessibility', state, can_request: false, can_open_settings: true,
    requires_restart_after_grant: false, capabilities: ['computer_use'] }] });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
};
afterEach(() => cleanup());

test('a stale mounted check cannot overwrite a newer focus result', async () => {
  const older = deferred<SystemPermissionStatus>();
  const newer = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
  try {
    const hook = renderHook(() => useSystemPermissions());
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    await act(async () => { newer.resolve(status('denied')); });
    await waitFor(() => expect(hook.result.current.status?.permissions[0]?.state).toBe('denied'));
    await act(async () => { older.resolve(status('granted')); });
    expect(hook.result.current.status?.permissions[0]?.state).toBe('denied');
  } finally { get.mockRestore(); }
});

test('a grant request fences out an earlier get result', async () => {
  const older = deferred<SystemPermissionStatus>();
  const grant = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockReturnValue(older.promise);
  const request = spyOn(ipcBridge.systemPermissions.request, 'invoke').mockReturnValue(grant.promise);
  try {
    const hook = renderHook(() => useSystemPermissions());
    let granting!: Promise<SystemPermissionStatus | null>;
    act(() => { granting = hook.result.current.request('accessibility'); });
    await act(async () => { grant.resolve(status('denied')); await granting; });
    await act(async () => { older.resolve(status('granted')); });
    expect(hook.result.current.status?.permissions[0]?.state).toBe('denied');
  } finally { get.mockRestore(); request.mockRestore(); }
});

test('opening settings and disabling a consumer reject obsolete checks', async () => {
  const older = deferred<SystemPermissionStatus>();
  const fresh = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockReturnValueOnce(older.promise).mockReturnValueOnce(fresh.promise);
  const open = spyOn(ipcBridge.systemPermissions.openSettings, 'invoke').mockResolvedValue(undefined);
  try {
    const hook = renderHook(({ enabled }) => useSystemPermissions(enabled), { initialProps: { enabled: true } });
    await act(async () => { await hook.result.current.openSettings('accessibility'); });
    await act(async () => { older.resolve(status('granted')); });
    expect(hook.result.current.status).toBeNull();
    hook.rerender({ enabled: false });
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    expect(get).toHaveBeenCalledTimes(1);
    hook.rerender({ enabled: true });
    hook.rerender({ enabled: false });
    await act(async () => { fresh.resolve(status('granted')); });
    expect(hook.result.current.status).toBeNull();
  } finally { get.mockRestore(); open.mockRestore(); }
});

test('unmount fences a pending request and a new scope starts without stale readiness', async () => {
  const older = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockReturnValueOnce(older.promise).mockResolvedValue(status('denied'));
  try {
    const first = renderHook(() => useSystemPermissions());
    first.unmount();
    await act(async () => { older.resolve(status('granted')); });
    const next = renderHook(({ scope }) => useSystemPermissions(true, scope), { initialProps: { scope: 'refusal-1' } });
    await waitFor(() => expect(next.result.current.status?.permissions[0]?.state).toBe('denied'));
    get.mockImplementation(() => new Promise(() => {}));
    next.rerender({ scope: 'refusal-2' });
    expect(next.result.current.status).toBeNull();
    expect(next.result.current.loading).toBe(true);
  } finally { get.mockRestore(); }
});
