import '../../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
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

test('overview keeps Browser permissions on demand and notification preferences out of capabilities', async () => {
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
  expect(screen.queryByRole('tab', { name: settings.capabilityPermissions.tabs.notifications })).toBeNull();
  expect(screen.queryByText(settings.capabilityPermissions.notifications.title)).toBeNull();
  expect(ipcBridge.notification.permissionState.invoke).not.toHaveBeenCalled();
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

function LocationProbe() {
  const location = useLocation();
  return <output data-testid='location'>{location.pathname + location.search}</output>;
}

test('the old application notification tab redirects to the preferences notification card', async () => {
  const screen = render(<I18nextProvider i18n={i18n}>
    <MemoryRouter initialEntries={['/settings/permissions?tab=notifications']}>
      <Routes>
        <Route path='/settings/permissions' element={<CapabilityPermissionsContent />} />
        <Route path='/settings/system' element={<LocationProbe />} />
      </Routes>
    </MemoryRouter>
  </I18nextProvider>);
  await waitFor(() => expect(screen.getByTestId('location').textContent).toBe('/settings/system?section=preferences&focus=notifications'));
  expect(ipcBridge.systemPermissions.get.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.notification.permissionState.invoke).not.toHaveBeenCalled();
});
