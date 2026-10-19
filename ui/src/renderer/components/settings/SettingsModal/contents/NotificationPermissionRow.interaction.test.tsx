import '../../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import NotificationPermissionRow from './NotificationPermissionRow';
import { useNotificationPermission } from '@/renderer/hooks/system/useNotificationPermission';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { settings } } },
  interpolation: { escapeValue: false },
});

const blocked: SystemPermissionStatus = {
  platform: 'macos',
  app_label: 'NomiFun',
  permissions: [
    {
      kind: 'microphone', state: 'granted', can_request: false, can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['voice_input'],
    },
    {
      kind: 'accessibility', state: 'not_granted', can_request: true, can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['computer_use'],
    },
    {
      kind: 'screen_recording', state: 'not_granted', can_request: true, can_open_settings: true,
      requires_restart_after_grant: true, capabilities: ['computer_use'],
    },
  ],
};

const restores: Array<() => void> = [];
beforeEach(() => {
  (window as Window & { __backendPort?: number }).__backendPort = 13400;
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(blocked);
  const request = spyOn(ipcBridge.systemPermissions.request, 'invoke').mockResolvedValue(blocked);
  const open = spyOn(ipcBridge.systemPermissions.openSettings, 'invoke').mockResolvedValue(undefined);
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('denied');
  restores.push(() => get.mockRestore(), () => request.mockRestore(), () => open.mockRestore(), () => notification.mockRestore());
});
afterEach(() => {
  cleanup();
  delete (window as Window & { __backendPort?: number }).__backendPort;
  restores.splice(0).forEach((restore) => restore());
});

const title = (state: 'enabled' | 'denied' | 'pending' | 'unavailable') =>
  i18n.t(`settings.notificationPermission.${state}Title`);
const Notifications = () => {
  const permission = useNotificationPermission();
  return <NotificationPermissionRow permission={permission} />;
};
const renderNotifications = () => render(<I18nextProvider i18n={i18n}><Notifications /></I18nextProvider>);

test('a denied grant keeps its recovery action while the native request remains on demand', async () => {
  const screen = renderNotifications();
  expect(await screen.findByText(title('denied'))).toBeTruthy();
  expect(screen.queryByRole('button', { name: settings.capabilityPermissions.requestAccess })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.openSystemSettings }));
  await waitFor(() => expect(ipcBridge.systemPermissions.openSettings.invoke).toHaveBeenCalledWith({ kind: 'notifications' }));
});

test('a first grant is requested in place and the same permission state enables recovery management', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('default');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockResolvedValue('granted');
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('pending'))).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  expect(await screen.findByText(title('enabled'))).toBeTruthy();
  expect(screen.getByRole('button', { name: settings.capabilityPermissions.openSystemSettings })).toBeTruthy();
  expect(request).toHaveBeenCalledTimes(1);
});

test('an unreadable grant offers a local retry and clears its failure after a successful read', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockRejectedValueOnce(new Error('Could not read permission')).mockResolvedValue('granted');
  restores.push(() => notification.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(settings.capabilityPermissions.loadFailed)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.workspace.retry }));
  expect(await screen.findByText(title('enabled'))).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.loadFailed)).toBeNull();
});

test('non-macOS desktops do not offer an unsupported system-settings action', async () => {
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue({ ...blocked, platform: 'windows' });
  restores.push(() => get.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('denied'))).toBeTruthy();
  expect(screen.queryByRole('button', { name: settings.capabilityPermissions.openSystemSettings })).toBeNull();
});

test('WebUI explains the desktop boundary without native permission queries or actions', async () => {
  delete (window as Window & { __backendPort?: number }).__backendPort;
  const screen = renderNotifications();
  expect(await screen.findByText(settings.capabilityPermissions.desktopOnly)).toBeTruthy();
  expect(screen.queryByRole('button')).toBeNull();
  expect(ipcBridge.notification.permissionState.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.systemPermissions.get.invoke).not.toHaveBeenCalled();
});

test('preference authorization updates the same live grant snapshot and keeps a denied preference off', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('denied');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockResolvedValue('denied');
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const { result } = renderHook(() => useNotificationPermission());
  await waitFor(() => expect(result.current.state).toBe('denied'));
  let granted = true;
  await act(async () => { granted = await result.current.ensureGranted(); });
  expect(granted).toBe(false);
  expect(result.current.state).toBe('denied');
  request.mockResolvedValue('granted');
  await act(async () => { granted = await result.current.ensureGranted(); });
  expect(granted).toBe(true);
  expect(result.current.state).toBe('granted');
});

test('granted notifications still open the exact macOS notification settings', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('granted');
  restores.push(() => notification.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('enabled'))).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.openSystemSettings }));
  await waitFor(() => expect(ipcBridge.systemPermissions.openSettings.invoke).toHaveBeenCalledWith({ kind: 'notifications' }));
  expect(ipcBridge.systemPermissions.request.invoke).not.toHaveBeenCalled();
});

test('a failed fresh notification query clears a stale grant and can recover on focus', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('granted')
    .mockRejectedValueOnce(new Error('Native permission query failed'))
    .mockResolvedValue('denied');
  restores.push(() => notification.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('enabled'))).toBeTruthy();
  fireEvent.focus(window);
  expect(await screen.findByText(settings.capabilityPermissions.loadFailed)).toBeTruthy();
  expect(screen.getByText(title('unavailable'))).toBeTruthy();
  expect(screen.queryByText(title('enabled'))).toBeNull();
  fireEvent.focus(window);
  expect(await screen.findByText(title('denied'))).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.loadFailed)).toBeNull();
  expect(notification).toHaveBeenCalledTimes(3);
});

test('a rejected notification permission request is visible and leaves no granted state', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('default');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockRejectedValue(new Error('Native permission request failed'));
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('pending'))).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  expect(await screen.findByText(settings.capabilityPermissions.loadFailed)).toBeTruthy();
  expect(screen.getByText(title('unavailable'))).toBeTruthy();
  expect(screen.queryByText(title('enabled'))).toBeNull();
  expect(request).toHaveBeenCalledTimes(1);
});

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
};

test.each(['granted', 'denied'] as const)('preference authorization follows the newer %s focus check after an OS prompt', async (freshState) => {
  const prompt = deferred<'granted'>();
  const focusCheck = deferred<typeof freshState>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('default')
    .mockResolvedValueOnce('default')
    .mockImplementationOnce(() => focusCheck.promise);
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockImplementationOnce(() => prompt.promise);
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const { result } = renderHook(() => useNotificationPermission());
  await waitFor(() => expect(result.current.state).toBe('not_determined'));
  let outcome: boolean | undefined;
  let authorization!: Promise<void>;
  act(() => { authorization = result.current.ensureGranted().then(value => { outcome = value; }); });
  await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  await waitFor(() => expect(notification).toHaveBeenCalledTimes(3));
  await act(async () => { prompt.resolve('granted'); await prompt.promise; });
  // The prompt's older grant cannot authorize a write while a newer check is pending.
  expect(outcome).toBeUndefined();
  await act(async () => { focusCheck.resolve(freshState); await authorization; });
  expect(outcome).toBe(freshState === 'granted');
  expect(result.current.state).toBe(freshState);
});

test('a completed granted focus check preserves a successful preference authorization', async () => {
  const prompt = deferred<'granted'>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('default')
    .mockResolvedValueOnce('default')
    .mockResolvedValue('granted');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockImplementationOnce(() => prompt.promise);
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const { result } = renderHook(() => useNotificationPermission());
  await waitFor(() => expect(result.current.state).toBe('not_determined'));
  let authorization!: Promise<boolean>;
  act(() => { authorization = result.current.ensureGranted(); });
  await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  await waitFor(() => expect(result.current.state).toBe('granted'));
  let granted = false;
  await act(async () => { prompt.resolve('granted'); granted = await authorization; });
  expect(granted).toBe(true);
  expect(result.current.state).toBe('granted');
});

test.each(['granted', 'error'] as const)('an older %s query cannot overwrite a newer notification denial', async (lateResult) => {
  const oldQuery = deferred<'granted'>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockImplementationOnce(() => oldQuery.promise)
    .mockResolvedValue('denied');
  restores.push(() => notification.mockRestore());
  const screen = renderNotifications();
  await waitFor(() => expect(notification).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  expect(await screen.findByText(title('denied'))).toBeTruthy();
  await act(async () => {
    if (lateResult === 'error') oldQuery.reject(new Error('Retired permission query failed'));
    else oldQuery.resolve('granted');
    await oldQuery.promise.catch(() => {});
  });
  expect(screen.getByText(title('denied'))).toBeTruthy();
  expect(screen.queryByText(title('enabled'))).toBeNull();
  expect(screen.queryByText(settings.capabilityPermissions.loadFailed)).toBeNull();
});

test('an older query cannot restore a grant after a newer permission request denies it', async () => {
  const oldQuery = deferred<'granted'>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('default')
    .mockImplementationOnce(() => oldQuery.promise);
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockResolvedValue('denied');
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('pending'))).toBeTruthy();
  fireEvent.focus(window);
  await waitFor(() => expect(notification).toHaveBeenCalledTimes(2));
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  expect(await screen.findByText(title('denied'))).toBeTruthy();
  await act(async () => {
    oldQuery.resolve('granted');
    await oldQuery.promise;
  });
  expect(screen.getByText(title('denied'))).toBeTruthy();
  expect(screen.queryByText(title('enabled'))).toBeNull();
});

test('an older permission request cannot replace a newer fresh notification denial', async () => {
  const oldRequest = deferred<'granted'>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('default')
    .mockResolvedValue('denied');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockImplementationOnce(() => oldRequest.promise);
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(title('pending'))).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  expect(await screen.findByText(title('denied'))).toBeTruthy();
  await act(async () => {
    oldRequest.resolve('granted');
    await oldRequest.promise;
  });
  expect(screen.getByText(title('denied'))).toBeTruthy();
  expect(screen.queryByText(title('enabled'))).toBeNull();
});
