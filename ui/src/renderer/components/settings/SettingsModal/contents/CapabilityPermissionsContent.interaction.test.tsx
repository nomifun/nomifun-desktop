import '../../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
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
      kind: 'accessibility', state: 'not_determined', can_request: true, can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['computer_use'],
    },
    {
      kind: 'screen_recording', state: 'not_determined', can_request: true, can_open_settings: true,
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
  expect(screen.getAllByText(settings.capabilityPermissions.states.not_determined).length).toBe(2);

  fireEvent.click(screen.getAllByRole('button', { name: settings.capabilityPermissions.requestAccess })[0]);
  await waitFor(() => expect(ipcBridge.systemPermissions.request.invoke).toHaveBeenCalledWith({ kind: 'accessibility' }));
  await waitFor(() => expect(ipcBridge.systemPermissions.openSettings.invoke).toHaveBeenCalledWith({ kind: 'accessibility' }));
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
