import '../../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import CapabilityPermissionsContent from './CapabilityPermissionsContent';

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

test('Computer Use tab requests the exact grant and opens recovery settings while still blocked', async () => {
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/settings/permissions?tab=computer-use']}>
        <CapabilityPermissionsContent />
      </MemoryRouter>
    </I18nextProvider>
  );

  expect(await screen.findByText(settings.capabilityPermissions.permissions.accessibility)).toBeTruthy();
  expect(screen.getByText(settings.capabilityPermissions.permissions.screenRecording)).toBeTruthy();
  expect(screen.getAllByText(settings.capabilityPermissions.states.not_granted).length).toBe(2);

  fireEvent.click(screen.getAllByRole('button', { name: settings.capabilityPermissions.requestAccess })[0]);
  await waitFor(() => expect(ipcBridge.systemPermissions.request.invoke).toHaveBeenCalledWith({ kind: 'accessibility' }));
  await waitFor(() => expect(ipcBridge.systemPermissions.openSettings.invoke).toHaveBeenCalledWith({ kind: 'accessibility' }));
});

test('host camera permission uses its native request while the website decision stays separate', async () => {
  const cameraStatus: SystemPermissionStatus = { ...blocked, permissions: [...blocked.permissions, {
    kind: 'camera', state: 'not_determined', can_request: true, can_open_settings: true,
    requires_restart_after_grant: false, capabilities: ['browser_use'],
  }] };
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(cameraStatus);
  const request = spyOn(ipcBridge.systemPermissions.request, 'invoke').mockResolvedValue(cameraStatus);
  restores.push(() => get.mockRestore(), () => request.mockRestore());
  const screen = render(<I18nextProvider i18n={i18n}>
    <MemoryRouter initialEntries={['/settings/permissions?tab=browser-use']}><CapabilityPermissionsContent /></MemoryRouter>
  </I18nextProvider>);
  expect(await screen.findByText(settings.capabilityPermissions.browser.hostMediaTitle)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  await waitFor(() => expect(request).toHaveBeenCalledWith({ kind: 'camera' }));
  expect(screen.getByText(settings.capabilityPermissions.browser.websiteDescription)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.permissions.location)).toBeNull();
  expect(screen.getByText(settings.capabilityPermissions.browser.macosPermissionLimit)).toBeTruthy();
});

test('unprobeable file, automation and local-network access stays unknown and opens exact recovery settings', async () => {
  const unknownStatus: SystemPermissionStatus = { ...blocked, permissions: [...blocked.permissions,
    ...(['full_disk_access', 'automation', 'local_network'] as const).map(kind => ({ kind, state: 'unknown' as const,
      can_request: false, can_open_settings: true, requires_restart_after_grant: false, capabilities: [] })),
  ] };
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(unknownStatus);
  restores.push(() => get.mockRestore());
  const screen = render(<I18nextProvider i18n={i18n}>
    <MemoryRouter initialEntries={['/settings/permissions']}><CapabilityPermissionsContent /></MemoryRouter>
  </I18nextProvider>);
  expect(await screen.findByText(settings.capabilityPermissions.automation.title)).toBeTruthy();
  expect(screen.getAllByText(settings.capabilityPermissions.states.unknown)).toHaveLength(3);
  const buttons = screen.getAllByRole('button', { name: settings.capabilityPermissions.openSystemSettings });
  fireEvent.click(buttons[2]);
  await waitFor(() => expect(ipcBridge.systemPermissions.openSettings.invoke).toHaveBeenCalledWith({ kind: 'automation' }));
});

test('overview keeps Browser permissions on demand and surfaces notification denial', async () => {
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/settings/permissions']}>
        <CapabilityPermissionsContent />
      </MemoryRouter>
    </I18nextProvider>
  );

  expect(await screen.findByText(settings.capabilityPermissions.browser.title)).toBeTruthy();
  expect(screen.getByText(settings.capabilityPermissions.summaryStates.onDemand)).toBeTruthy();
  expect(screen.getByText(settings.capabilityPermissions.localNetwork.title)).toBeTruthy();
  expect(screen.getByText(settings.capabilityPermissions.files.title)).toBeTruthy();
  expect(screen.getAllByText(settings.capabilityPermissions.summaryStates.attention).length).toBeGreaterThan(0);
});

test('granted notifications still open the exact macOS notification settings', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('granted');
  restores.push(() => notification.mockRestore());
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/settings/permissions?tab=notifications']}>
        <CapabilityPermissionsContent />
      </MemoryRouter>
    </I18nextProvider>
  );
  expect(await screen.findByText(settings.capabilityPermissions.states.granted)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.openSystemSettings }));
  await waitFor(() => expect(ipcBridge.systemPermissions.openSettings.invoke).toHaveBeenCalledWith({ kind: 'notifications' }));
  expect(ipcBridge.systemPermissions.request.invoke).not.toHaveBeenCalled();
});

test('the granted microphone retains its existing settings action behavior', async () => {
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/settings/permissions?tab=voice-input']}>
        <CapabilityPermissionsContent />
      </MemoryRouter>
    </I18nextProvider>
  );
  expect(await screen.findByText(settings.capabilityPermissions.states.granted)).toBeTruthy();
  expect(screen.queryByRole('button', { name: settings.capabilityPermissions.openSystemSettings })).toBeNull();
});

test('a failed fresh notification query clears a stale grant and can recover on focus', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('granted')
    .mockRejectedValueOnce(new Error('Native permission query failed'))
    .mockResolvedValue('denied');
  restores.push(() => notification.mockRestore());
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/settings/permissions?tab=notifications']}>
        <CapabilityPermissionsContent />
      </MemoryRouter>
    </I18nextProvider>
  );
  expect(await screen.findByText(settings.capabilityPermissions.states.granted)).toBeTruthy();
  fireEvent.focus(window);
  expect(await screen.findByText(settings.capabilityPermissions.loadFailed)).toBeTruthy();
  expect(screen.getByText(settings.capabilityPermissions.states.unknown)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.states.granted)).toBeNull();
  fireEvent.focus(window);
  expect(await screen.findByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.loadFailed)).toBeNull();
  expect(notification).toHaveBeenCalledTimes(3);
});

test('a rejected notification permission request is visible and leaves no granted state', async () => {
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('default');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockRejectedValue(new Error('Native permission request failed'));
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const screen = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/settings/permissions?tab=notifications']}>
        <CapabilityPermissionsContent />
      </MemoryRouter>
    </I18nextProvider>
  );
  expect(await screen.findByText(settings.capabilityPermissions.states.not_determined)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  expect(await screen.findByText(settings.capabilityPermissions.loadFailed)).toBeTruthy();
  expect(screen.getByText(settings.capabilityPermissions.states.unknown)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.states.granted)).toBeNull();
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

const renderNotifications = () => render(
  <I18nextProvider i18n={i18n}>
    <MemoryRouter initialEntries={['/settings/permissions?tab=notifications']}>
      <CapabilityPermissionsContent />
    </MemoryRouter>
  </I18nextProvider>
);

test.each(['granted', 'error'] as const)('an older %s query cannot overwrite a newer notification denial', async (lateResult) => {
  const oldQuery = deferred<'granted'>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockImplementationOnce(() => oldQuery.promise)
    .mockResolvedValue('denied');
  restores.push(() => notification.mockRestore());
  const screen = renderNotifications();
  await waitFor(() => expect(notification).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  expect(await screen.findByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  await act(async () => {
    if (lateResult === 'error') oldQuery.reject(new Error('Retired permission query failed'));
    else oldQuery.resolve('granted');
    await oldQuery.promise.catch(() => {});
  });
  expect(screen.getByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.states.granted)).toBeNull();
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
  expect(await screen.findByText(settings.capabilityPermissions.states.not_determined)).toBeTruthy();
  fireEvent.focus(window);
  await waitFor(() => expect(notification).toHaveBeenCalledTimes(2));
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  expect(await screen.findByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  await act(async () => {
    oldQuery.resolve('granted');
    await oldQuery.promise;
  });
  expect(screen.getByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.states.granted)).toBeNull();
});

test('an older permission request cannot replace a newer fresh notification denial', async () => {
  const oldRequest = deferred<'granted'>();
  const notification = spyOn(ipcBridge.notification.permissionState, 'invoke')
    .mockResolvedValueOnce('default')
    .mockResolvedValue('denied');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockImplementationOnce(() => oldRequest.promise);
  restores.push(() => notification.mockRestore(), () => request.mockRestore());
  const screen = renderNotifications();
  expect(await screen.findByText(settings.capabilityPermissions.states.not_determined)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: settings.capabilityPermissions.requestAccess }));
  await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  expect(await screen.findByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  await act(async () => {
    oldRequest.resolve('granted');
    await oldRequest.promise;
  });
  expect(screen.getByText(settings.capabilityPermissions.states.denied)).toBeTruthy();
  expect(screen.queryByText(settings.capabilityPermissions.states.granted)).toBeNull();
});
