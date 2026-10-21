import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { useSystemPermissions } from './useSystemPermissions';

const status = (state: 'granted' | 'denied'): SystemPermissionStatus => ({ platform: 'macos', app_label: 'NomiFun',
  permissions: [{ kind: 'accessibility', state, can_request: false, can_open_settings: true,
    requires_restart_after_grant: false, capabilities: ['computer_use'] }] });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
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

test('focus reads and duplicate requests cannot supersede an explicit capture failure', async () => {
  const capture = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(status('granted'));
  const request = spyOn(ipcBridge.systemPermissions.request, 'invoke').mockReturnValue(capture.promise);
  const open = spyOn(ipcBridge.systemPermissions.openSettings, 'invoke').mockResolvedValue(undefined);
  const onError = mock(() => {});
  try {
    const hook = renderHook(() => useSystemPermissions());
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    let pending!: Promise<SystemPermissionStatus | null>;
    await act(async () => {
      pending = hook.result.current.request('screen_recording', onError);
      expect(await hook.result.current.request('screen_recording', onError)).toBeNull();
      window.dispatchEvent(new Event('focus'));
      expect(await hook.result.current.refresh()).toBeNull();
      await hook.result.current.openSettings('screen_recording');
    });
    expect(request).toHaveBeenCalledTimes(1);
    expect(get).toHaveBeenCalledTimes(1);
    expect(open).not.toHaveBeenCalled();
    expect(hook.result.current.loading).toBe(true);
    await act(async () => { capture.reject(new Error('Native capture probe failed')); await pending; });
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.error).toBe('Native capture probe failed');
    expect(hook.result.current.status?.permissions[0]?.state).toBe('granted');
    expect(onError).toHaveBeenCalledTimes(1);
    await act(async () => { await hook.result.current.refresh(); });
    expect(get).toHaveBeenCalledTimes(2);
    expect(request).toHaveBeenCalledTimes(1);
  } finally { get.mockRestore(); request.mockRestore(); open.mockRestore(); }
});

test('an obsolete scope cannot unlock or overwrite the new scopes explicit request', async () => {
  const older = deferred<SystemPermissionStatus>();
  const newer = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(status('denied'));
  const request = spyOn(ipcBridge.systemPermissions.request, 'invoke').mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
  const oldError = mock(() => {});
  try {
    const hook = renderHook(({ scope }) => useSystemPermissions(true, scope), { initialProps: { scope: 'old' } });
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    let oldPending!: Promise<SystemPermissionStatus | null>;
    act(() => { oldPending = hook.result.current.request('screen_recording', oldError); });
    hook.rerender({ scope: 'new' });
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    expect(get).toHaveBeenCalledTimes(2);
    let newPending!: Promise<SystemPermissionStatus | null>;
    act(() => { newPending = hook.result.current.request('screen_recording'); });
    await act(async () => { older.reject(new Error('Obsolete capture failure')); await oldPending; });
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      expect(await hook.result.current.request('screen_recording')).toBeNull();
    });
    expect(hook.result.current.loading).toBe(true);
    expect(hook.result.current.error).toBeNull();
    expect(oldError).not.toHaveBeenCalled();
    expect(get).toHaveBeenCalledTimes(2);
    expect(request).toHaveBeenCalledTimes(2);
    await act(async () => { newer.resolve(status('granted')); await newPending; });
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.status?.permissions[0]?.state).toBe('granted');
  } finally { get.mockRestore(); request.mockRestore(); }
});

test('disable and unmount fence pending explicit requests without blocking a later consumer', async () => {
  const disabled = deferred<SystemPermissionStatus>();
  const unmounted = deferred<SystemPermissionStatus>();
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(status('denied'));
  const request = spyOn(ipcBridge.systemPermissions.request, 'invoke').mockReturnValueOnce(disabled.promise).mockReturnValueOnce(unmounted.promise);
  const onError = mock(() => {});
  try {
    const hook = renderHook(({ enabled }) => useSystemPermissions(enabled), { initialProps: { enabled: true } });
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    let disabledPending!: Promise<SystemPermissionStatus | null>;
    act(() => { disabledPending = hook.result.current.request('screen_recording', onError); });
    hook.rerender({ enabled: false });
    await act(async () => { disabled.reject(new Error('Disabled consumer')); await disabledPending; });
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.error).toBeNull();
    hook.rerender({ enabled: true });
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    let unmountedPending!: Promise<SystemPermissionStatus | null>;
    act(() => { unmountedPending = hook.result.current.request('screen_recording', onError); });
    hook.unmount();
    await act(async () => { unmounted.reject(new Error('Unmounted consumer')); await unmountedPending; });
    expect(onError).not.toHaveBeenCalled();
    const next = renderHook(() => useSystemPermissions());
    await waitFor(() => expect(next.result.current.loading).toBe(false));
    expect(next.result.current.status?.permissions[0]?.state).toBe('denied');
    expect(request).toHaveBeenCalledTimes(2);
  } finally { get.mockRestore(); request.mockRestore(); }
});
