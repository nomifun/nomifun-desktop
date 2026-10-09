import '../../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';

// Isolate the global language listener before loading the real settings page.
// This test has no WebSocket service or authenticated config transport.
const languageListener = spyOn(ipcBridge.systemSettings.languageChanged, 'on').mockImplementation(() => () => {});
const initialize = spyOn(configService, 'initialize').mockResolvedValue(undefined);
const { default: SystemModalContent } = await import('./index');
languageListener.mockRestore();
initialize.mockRestore();

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } }, interpolation: { escapeValue: false } });
const restores: Array<() => void> = [];
const systemInfo = { workDir: 'D:\\current', cacheDir: 'D:\\cache', logDir: 'D:\\logs', storageGeneration: 'test', agentDataGeneration: 1, platform: 'windows', arch: 'x86_64' };
beforeEach(() => {
  window.__backendPort = 13400;
  configService.reset();
  configService.setLocal('chat.thinking.visible', true);
  const set = spyOn(configService, 'set').mockImplementation(async (key, value) => { configService.setLocal(key, value); });
  const info = spyOn(ipcBridge.application.systemInfo, 'invoke').mockResolvedValue(systemInfo);
  const boot = spyOn(ipcBridge.application.getStartOnBootStatus, 'invoke').mockResolvedValue({ success: true, data: { supported: true, enabled: false, isPackaged: true, platform: 'windows' } });
  const permission = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('granted');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockResolvedValue('granted');
  const pick = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValue(['D:\\selected']);
  const save = spyOn(ipcBridge.application.updateSystemInfo, 'invoke').mockResolvedValue(undefined);
  const restart = spyOn(ipcBridge.application.restart, 'invoke').mockResolvedValue(undefined);
  restores.push(() => set.mockRestore(), () => info.mockRestore(), () => boot.mockRestore(), () => permission.mockRestore(), () => request.mockRestore(), () => pick.mockRestore(), () => save.mockRestore(), () => restart.mockRestore());
});
afterEach(() => { cleanup(); restores.splice(0).forEach((restore) => restore()); configService.reset(); delete window.__backendPort; });

function LocationProbe() { return <output data-testid='location'>{useLocation().search}</output>; }
const renderPage = (section = 'preferences') => render(<I18nextProvider i18n={i18n}>
  <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, revalidateOnFocus: false }}>
    <MemoryRouter initialEntries={['/settings/system?from=test&section=' + section]}><SystemModalContent /><LocationProbe /></MemoryRouter>
  </SWRConfig>
</I18nextProvider>);

test('search finds controls across groups, saves the canonical preference and Escape returns to the selected group', async () => {
  const screen = renderPage();
  const search = screen.getByRole('searchbox', { name: settings.workspace.search });
  await act(async () => { fireEvent.input(search, { target: { value: settings.saveUploadToWorkspace } }); });
  fireEvent.click(await screen.findByRole('switch', { name: settings.saveUploadToWorkspace }));
  await waitFor(() => expect(configService.set).toHaveBeenCalledWith('upload.saveToWorkspace', true));
  fireEvent.keyDown(search, { key: 'Escape' });
  expect(screen.getByRole('tab', { name: settings.workspace.preferences }).getAttribute('aria-selected')).toBe('true');
  expect(screen.queryByRole('switch', { name: settings.saveUploadToWorkspace })).toBeNull();
  await act(async () => { fireEvent.input(search, { target: { value: 'does-not-exist' } }); });
  expect(screen.getByRole('heading', { name: settings.workspace.noResults })).toBeTruthy();
});

test('keyboard tabs preserve route parameters and hidden reasoning disables dependent choices', async () => {
  const screen = renderPage('conversation');
  const thinking = screen.getByRole('switch', { name: settings.thinkingProcessVisible });
  fireEvent.click(thinking);
  await waitFor(() => expect(thinking.getAttribute('aria-checked')).toBe('false'));
  expect((screen.getByRole('radio', { name: settings.thinkingContentFull }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('radio', { name: settings.sendKeyModEnter }));
  expect(configService.set).toHaveBeenCalledWith('chat.sendKey', 'mod-enter');
  fireEvent.keyDown(screen.getByRole('tab', { name: settings.workspace.conversation }), { key: 'End' });
  expect(screen.getByRole('tab', { name: settings.workspace.data }).getAttribute('aria-selected')).toBe('true');
  expect(screen.getByTestId('location').textContent).toContain('from=test');
});

test('disabled notifications retain the scheduled preference and re-enable its control after authorization', async () => {
  configService.setLocal('system.notificationEnabled', false);
  configService.setLocal('system.cronNotificationEnabled', true);
  const screen = renderPage('notifications');
  const scheduled = screen.getByRole('switch', { name: settings.cronNotificationEnabled }) as HTMLButtonElement;
  expect(scheduled.disabled).toBe(true);
  expect(scheduled.getAttribute('aria-checked')).toBe('true');
  fireEvent.click(screen.getByRole('switch', { name: settings.notification }));
  await waitFor(() => expect(scheduled.disabled).toBe(false));
  fireEvent.click(scheduled);
  await waitFor(() => expect(configService.set).toHaveBeenCalledWith('system.cronNotificationEnabled', false));
});

test('a pending preference write prevents duplicate clicks and a failure restores the old value', async () => {
  let fail!: (error: Error) => void;
  const set = spyOn(configService, 'set').mockImplementation(() => new Promise((_resolve, reject) => { fail = reject; }));
  restores.push(() => set.mockRestore());
  const screen = renderPage('files');
  const upload = screen.getByRole('switch', { name: settings.saveUploadToWorkspace }) as HTMLButtonElement;
  fireEvent.click(upload);
  expect(upload.disabled).toBe(true);
  expect(upload.getAttribute('aria-checked')).toBe('true');
  fireEvent.click(upload);
  expect(set).toHaveBeenCalledTimes(1);
  await act(async () => { fail(new Error('offline')); });
  expect(upload.disabled).toBe(false);
  expect(upload.getAttribute('aria-checked')).toBe('false');
  expect(configService.get('upload.saveToWorkspace')).toBe(false);
});

test('denied notification access leaves notifications off and does not persist an enabled value', async () => {
  configService.setLocal('system.notificationEnabled', false);
  const permission = spyOn(ipcBridge.notification.permissionState, 'invoke').mockResolvedValue('denied');
  const request = spyOn(ipcBridge.notification.requestPermission, 'invoke').mockResolvedValue('denied');
  restores.push(() => permission.mockRestore(), () => request.mockRestore());
  const screen = renderPage('notifications');
  const notification = screen.getByRole('switch', { name: settings.notification });
  fireEvent.click(notification);
  await waitFor(() => expect(ipcBridge.notification.requestPermission.invoke).toHaveBeenCalled());
  await waitFor(() => expect(notification.getAttribute('aria-busy')).toBe('false'));
  expect(notification.getAttribute('aria-checked')).toBe('false');
  expect(configService.set).not.toHaveBeenCalled();
});

const openDirectoryConfirmation = async () => {
  const screen = renderPage('files');
  await screen.findByText(systemInfo.workDir);
  fireEvent.click(screen.getByRole('button', { name: settings.workspace.changeDirectory }));
  await screen.findByText(settings.workDirChangeConfirmTitle);
  return screen;
};
test('canceling a directory change leaves storage and restart untouched', async () => {
  const screen = await openDirectoryConfirmation();
  fireEvent.click(screen.getByRole('button', { name: /Cancel|取消/ }));
  await waitFor(() => expect((screen.getByRole('button', { name: settings.workspace.changeDirectory }) as HTMLButtonElement).disabled).toBe(false));
  expect(ipcBridge.application.updateSystemInfo.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
  expect(screen.getByText(systemInfo.workDir)).toBeTruthy();
});
test('a failed directory save keeps the current path and never relaunches', async () => {
  const fail = spyOn(ipcBridge.application.updateSystemInfo, 'invoke').mockRejectedValue(new Error('save failed'));
  restores.push(() => fail.mockRestore());
  const screen = await openDirectoryConfirmation();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: /OK|确定/ })); });
  expect(await screen.findByText('save failed')).toBeTruthy();
  expect(screen.getByText(systemInfo.workDir)).toBeTruthy();
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
});
test('a committed directory remains visible when relaunch fails, preserving the cache path', async () => {
  const fail = spyOn(ipcBridge.application.restart, 'invoke').mockRejectedValue(new Error('restart failed'));
  restores.push(() => fail.mockRestore());
  const screen = await openDirectoryConfirmation();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: /OK|确定/ })); });
  expect(await screen.findByText('restart failed')).toBeTruthy();
  expect(screen.getByText('D:\\selected')).toBeTruthy();
  expect(screen.queryByText(systemInfo.workDir)).toBeNull();
  expect(ipcBridge.application.updateSystemInfo.invoke).toHaveBeenCalledWith({ cacheDir: systemInfo.cacheDir, workDir: 'D:\\selected' });
});
