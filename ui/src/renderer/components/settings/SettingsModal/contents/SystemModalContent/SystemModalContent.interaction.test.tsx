import '../../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { StorageRelocationResponse } from '@/common/protocolBindings/StorageRelocationResponse';
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
const systemInfo = { dataDir: 'D:\\current-data', workDir: 'D:\\projects', cacheDir: 'D:\\cache', logDir: 'D:\\logs', storageGeneration: 'test', agentDataGeneration: 1, platform: 'windows', arch: 'x86_64' };
const relocation = (patch: Partial<StorageRelocationResponse> = {}): StorageRelocationResponse => ({
  operation_id: null, phase: 'idle', current_dir: systemInfo.dataDir, target_dir: null,
  total_bytes: 0, file_count: 0, blockers: [], warnings: [], restart_required: false,
  cancellable: false, error: null, ...patch,
});
const prepared = () => relocation({ operation_id: 'reviewed-plan', phase: 'prepared', target_dir: 'D:\\selected', total_bytes: 1024, file_count: 4, cancellable: true });
const scheduled = () => ({ ...prepared(), phase: 'awaiting_restart', restart_required: true });
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
  const status = spyOn(ipcBridge.storageRelocation.status, 'invoke').mockResolvedValue(relocation());
  const prepare = spyOn(ipcBridge.storageRelocation.prepare, 'invoke').mockResolvedValue(prepared());
  const start = spyOn(ipcBridge.storageRelocation.start, 'invoke').mockResolvedValue(scheduled());
  const cancel = spyOn(ipcBridge.storageRelocation.cancel, 'invoke').mockResolvedValue(relocation({ phase: 'canceled' }));
  const restart = spyOn(ipcBridge.application.restart, 'invoke').mockResolvedValue(undefined);
  restores.push(() => set.mockRestore(), () => info.mockRestore(), () => boot.mockRestore(), () => permission.mockRestore(), () => request.mockRestore(), () => pick.mockRestore(), () => status.mockRestore(), () => prepare.mockRestore(), () => start.mockRestore(), () => cancel.mockRestore(), () => restart.mockRestore());
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
  await act(async () => { fireEvent.input(search, { target: { value: settings.autoPreviewOfficeFiles } }); });
  fireEvent.click(await screen.findByRole('switch', { name: settings.autoPreviewOfficeFiles }));
  await waitFor(() => expect(configService.set).toHaveBeenCalledWith('system.autoPreviewOfficeFiles', false));
  fireEvent.keyDown(search, { key: 'Escape' });
  expect(screen.getByRole('tab', { name: settings.workspace.preferences }).getAttribute('aria-selected')).toBe('true');
  expect(screen.queryByRole('switch', { name: settings.autoPreviewOfficeFiles })).toBeNull();
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
  const preview = screen.getByRole('switch', { name: settings.autoPreviewOfficeFiles }) as HTMLButtonElement;
  fireEvent.click(preview);
  expect(preview.disabled).toBe(true);
  expect(preview.getAttribute('aria-checked')).toBe('false');
  fireEvent.click(preview);
  expect(set).toHaveBeenCalledTimes(1);
  await act(async () => { fail(new Error('offline')); });
  expect(preview.disabled).toBe(false);
  expect(preview.getAttribute('aria-checked')).toBe('true');
  expect(configService.get('system.autoPreviewOfficeFiles')).toBe(true);
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

const openStorageMigration = async () => {
  const screen = renderPage('files');
  await screen.findByText(systemInfo.dataDir);
  expect(screen.queryByText(systemInfo.workDir)).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: settings.workspace.changeDirectory }));
  await screen.findByText(settings.storageRelocation.title);
  await waitFor(() => expect((screen.getByRole('button', { name: settings.storageRelocation.chooseDirectory }) as HTMLButtonElement).disabled).toBe(false));
  return screen;
};
const openPreparedMigration = async () => {
  const screen = await openStorageMigration();
  fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.chooseDirectory }));
  await screen.findByText(settings.storageRelocation.prepared);
  return screen;
};
test('preflight shows totals and cancel uses the reviewed operation without scheduling or restarting', async () => {
  const screen = await openPreparedMigration();
  expect(screen.getByText('Data to move: 1 KB across 4 files')).toBeTruthy();
  expect(ipcBridge.storageRelocation.prepare.invoke).toHaveBeenCalledWith({ target_dir: 'D:\\selected' });
  fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.cancel }));
  await waitFor(() => expect(ipcBridge.storageRelocation.cancel.invoke).toHaveBeenCalledWith({ operation_id: 'reviewed-plan' }));
  expect(ipcBridge.storageRelocation.start.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
  expect(screen.getAllByText(systemInfo.dataDir).length).toBeGreaterThan(0);
});
test('a blocked target cannot be scheduled and keeps the original active location', async () => {
  const blocked = spyOn(ipcBridge.storageRelocation.prepare, 'invoke').mockResolvedValue({ ...prepared(), blockers: [{ code: 'TARGET_OWNED', message: 'The selected directory belongs to another installation.' }] });
  restores.push(() => blocked.mockRestore());
  const screen = await openPreparedMigration();
  expect(screen.getByText('The selected directory belongs to another installation.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: settings.storageRelocation.saveAndRestart })).toBeNull();
  expect(ipcBridge.storageRelocation.start.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
});
test('a failed plan save keeps the current path and never relaunches', async () => {
  const fail = spyOn(ipcBridge.storageRelocation.start, 'invoke').mockRejectedValue(new Error('save failed'));
  restores.push(() => fail.mockRestore());
  const screen = await openPreparedMigration();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.saveAndRestart })); });
  expect(await screen.findByText('save failed')).toBeTruthy();
  expect(screen.getAllByText(systemInfo.dataDir).length).toBeGreaterThan(0);
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
});
test('a scheduled plan retains the old location after relaunch failure and allows retrying the restart', async () => {
  const fail = spyOn(ipcBridge.application.restart, 'invoke').mockRejectedValue(new Error('restart failed'));
  restores.push(() => fail.mockRestore());
  const screen = await openPreparedMigration();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.saveAndRestart })); });
  expect(await screen.findByText(settings.storageRelocation.restartFailed + ' restart failed')).toBeTruthy();
  expect(screen.getAllByText(systemInfo.dataDir).length).toBeGreaterThan(0);
  expect(ipcBridge.storageRelocation.start.invoke).toHaveBeenCalledWith({ operation_id: 'reviewed-plan' });
  fail.mockResolvedValue(undefined);
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.restartNow })); });
  expect(ipcBridge.application.restart.invoke).toHaveBeenCalledTimes(2);
  expect(ipcBridge.storageRelocation.start.invoke).toHaveBeenCalledTimes(1);
  expect(configService.set).not.toHaveBeenCalled();
});
test('reopening settings restores a pending plan and cancel does not change the dataset identity', async () => {
  const status = spyOn(ipcBridge.storageRelocation.status, 'invoke').mockResolvedValue(scheduled());
  restores.push(() => status.mockRestore());
  const screen = renderPage('files');
  fireEvent.click(await screen.findByRole('button', { name: settings.storageRelocation.manage }));
  await screen.findByText(settings.storageRelocation.awaitingRestart);
  await waitFor(() => expect((screen.getByRole('button', { name: settings.storageRelocation.cancel }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.cancel }));
  await waitFor(() => expect(ipcBridge.storageRelocation.cancel.invoke).toHaveBeenCalledWith({ operation_id: 'reviewed-plan' }));
  expect(ipcBridge.storageRelocation.prepare.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.storageRelocation.start.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
  expect(configService.set).not.toHaveBeenCalled();
});
test('WebUI schedules the reviewed plan with server restart guidance and no native relaunch', async () => {
  delete window.__backendPort;
  const status = spyOn(ipcBridge.storageRelocation.status, 'invoke').mockResolvedValue(prepared());
  restores.push(() => status.mockRestore());
  const screen = renderPage('files');
  await screen.findByText(systemInfo.dataDir);
  fireEvent.click(screen.getByRole('button', { name: settings.workspace.changeDirectory }));
  await screen.findByText(settings.storageRelocation.prepared);
  expect(screen.getByText(settings.storageRelocation.confirmDescriptionWeb)).toBeTruthy();
  expect(screen.queryByRole('button', { name: settings.storageRelocation.saveAndRestart })).toBeNull();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.savePlan })); });
  expect(await screen.findByText(settings.storageRelocation.restartServer)).toBeTruthy();
  expect(screen.queryByRole('button', { name: settings.storageRelocation.restartNow })).toBeNull();
  expect(ipcBridge.storageRelocation.start.invoke).toHaveBeenCalledWith({ operation_id: 'reviewed-plan' });
  expect(ipcBridge.application.restart.invoke).not.toHaveBeenCalled();
});
test('a pending start blocks duplicate requests and prevents dismissing the operation', async () => {
  let finish!: (value: StorageRelocationResponse) => void;
  const start = spyOn(ipcBridge.storageRelocation.start, 'invoke').mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  restores.push(() => start.mockRestore());
  const screen = await openPreparedMigration();
  const save = screen.getByRole('button', { name: settings.storageRelocation.saveAndRestart }) as HTMLButtonElement;
  fireEvent.click(save);
  fireEvent.click(save);
  expect(start).toHaveBeenCalledTimes(1);
  expect((screen.getByRole('button', { name: settings.storageRelocation.close }) as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { finish(scheduled()); });
  expect(ipcBridge.application.restart.invoke).toHaveBeenCalledTimes(1);
});

test('failure to verify the durable plan prevents starting from a stale prepared cache and allows retry', async () => {
  const status = spyOn(ipcBridge.storageRelocation.status, 'invoke')
    .mockResolvedValueOnce(prepared()).mockRejectedValue(new Error('status unavailable'));
  restores.push(() => status.mockRestore());
  const screen = renderPage('files');
  await waitFor(() => expect(ipcBridge.storageRelocation.status.invoke).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole('button', { name: settings.workspace.changeDirectory }));
  expect(await screen.findByText('status unavailable')).toBeTruthy();
  expect((screen.getByRole('button', { name: settings.storageRelocation.saveAndRestart }) as HTMLButtonElement).disabled).toBe(true);
  expect(ipcBridge.storageRelocation.start.invoke).not.toHaveBeenCalled();
  status.mockResolvedValue(prepared());
  fireEvent.click(screen.getByRole('button', { name: settings.storageRelocation.retryCheck }));
  await waitFor(() => expect((screen.getByRole('button', { name: settings.storageRelocation.saveAndRestart }) as HTMLButtonElement).disabled).toBe(false));
});

test('a recoverable offline failure can retry the same operation without preparing another dataset', async () => {
  const status = spyOn(ipcBridge.storageRelocation.status, 'invoke').mockResolvedValue({ ...prepared(), phase: 'failed', error: 'copy interrupted' });
  restores.push(() => status.mockRestore());
  const screen = renderPage('files');
  fireEvent.click(await screen.findByRole('button', { name: settings.workspace.changeDirectory }));
  await screen.findByText('copy interrupted');
  const save = screen.getByRole('button', { name: settings.storageRelocation.saveAndRestart }) as HTMLButtonElement;
  await waitFor(() => expect(save.disabled).toBe(false));
  await act(async () => { fireEvent.click(save); });
  expect(ipcBridge.storageRelocation.start.invoke).toHaveBeenCalledWith({ operation_id: 'reviewed-plan' });
  expect(ipcBridge.storageRelocation.prepare.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.application.restart.invoke).toHaveBeenCalledTimes(1);
});

test('an incomplete published migration requires restart recovery and cannot reactivate the original location', async () => {
  const status = spyOn(ipcBridge.storageRelocation.status, 'invoke').mockResolvedValue({
    ...scheduled(), phase: 'failed', cancellable: false, error: 'publication interrupted',
  });
  const restartFailure = spyOn(ipcBridge.application.restart, 'invoke').mockRejectedValue(new Error('relaunch blocked'));
  restores.push(() => status.mockRestore(), () => restartFailure.mockRestore());
  const screen = renderPage('files');
  fireEvent.click(await screen.findByRole('button', { name: settings.storageRelocation.manage }));
  await screen.findByText('publication interrupted');
  expect(screen.getAllByText(settings.storageRelocation.recoveryRequired).length).toBeGreaterThan(0);
  expect(screen.queryByText(settings.storageRelocation.failed)).toBeNull();
  expect(screen.queryByText(settings.storageRelocation.restartNotice)).toBeNull();
  expect(screen.queryByText(settings.storageRelocation.pending.replace('{{path}}', 'D:\\selected'))).toBeNull();
  expect(screen.queryByRole('button', { name: settings.storageRelocation.cancel })).toBeNull();
  expect(screen.queryByRole('button', { name: settings.storageRelocation.saveAndRestart })).toBeNull();
  const restart = screen.getByRole('button', { name: settings.storageRelocation.restartNow }) as HTMLButtonElement;
  await waitFor(() => expect(restart.disabled).toBe(false));
  await act(async () => { fireEvent.click(restart); });
  expect(await screen.findByText(settings.storageRelocation.recoveryRequired + ' relaunch blocked')).toBeTruthy();
  expect(screen.queryByText(settings.storageRelocation.restartFailed + ' relaunch blocked')).toBeNull();
  expect(ipcBridge.application.restart.invoke).toHaveBeenCalledTimes(1);
  expect(ipcBridge.storageRelocation.cancel.invoke).not.toHaveBeenCalled();
  expect(ipcBridge.storageRelocation.start.invoke).not.toHaveBeenCalled();
});
