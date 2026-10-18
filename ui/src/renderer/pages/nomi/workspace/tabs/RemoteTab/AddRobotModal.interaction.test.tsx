import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { agentPlatform, robot, webui, type IApiRobotEndpoints } from '@/common/adapter/ipcBridge';
import { parseCompanionId } from '@/common/types/ids';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import AddRobotModal from './AddRobotModal';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.robot.inheritHint': 'Uses {{companionName}} conversation and Agent settings',
  'nomi.robot.endpointLoading': 'Loading robot connection information',
  'nomi.robot.endpointLoadFailed': 'Failed to load robot connection information',
  'nomi.robot.otaNone': 'No LAN address is available yet',
  'nomi.robot.lanOff': 'LAN access is disabled',
  'nomi.robot.lanEnable': 'Enable LAN access',
  'nomi.robot.codePlaceholder': 'Activation code',
  'nomi.robot.claim': 'Pair',
  'common.retry': 'Retry',
} } } });
const restores: Array<() => void> = [];
const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
const otaUrl = 'http://192.168.1.5:25808/robot/ota';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function renderModal() {
  const onClaimed = mock(() => {});
  const onCancel = mock(() => {});
  const component = (visible: boolean) => <I18nextProvider i18n={i18n}>
    <AddRobotModal visible={visible} companionId={companionId} companionName='Nomi' onClaimed={onClaimed} onCancel={onCancel} />
  </I18nextProvider>;
  const view = render(component(true));
  return { ...view, onClaimed, onCancel, setVisible: (visible: boolean) => view.rerender(component(visible)) };
}

beforeEach(() => {
  const themeSpy = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', fontScale: 1, colorScheme: 'default',
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  restores.push(() => themeSpy.mockRestore());
});

afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('a pending endpoint read shows loading until the OTA addresses arrive', async () => {
  const pending = deferred<IApiRobotEndpoints>();
  const endpoints = spyOn(robot.endpoints, 'invoke').mockReturnValue(pending.promise);
  restores.push(() => endpoints.mockRestore());
  const view = renderModal();

  expect(view.getByRole('status').textContent).toBe('Loading robot connection information');
  expect(view.queryByText('No LAN address is available yet')).toBeNull();
  expect(view.queryByRole('alert')).toBeNull();
  await act(async () => pending.resolve({ ota_urls: [otaUrl], lan_enabled: true }));
  expect(view.getByText(otaUrl)).toBeTruthy();
  expect(view.queryByRole('status')).toBeNull();
});

test('a failed endpoint read explains the failure and can be retried', async () => {
  const retry = deferred<IApiRobotEndpoints>();
  const endpoints = spyOn(robot.endpoints, 'invoke')
    .mockRejectedValueOnce(new Error('Robot service unavailable'))
    .mockReturnValueOnce(retry.promise);
  const errorLog = spyOn(console, 'error').mockImplementation(() => {});
  for (const spy of [endpoints, errorLog]) restores.push(() => spy.mockRestore());
  const view = renderModal();

  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('Failed to load robot connection information'));
  expect(view.queryByText('No LAN address is available yet')).toBeNull();
  expect(view.queryByText('LAN access is disabled')).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  expect(endpoints).toHaveBeenCalledTimes(2);
  expect(view.getByRole('status')).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
  await act(async () => retry.resolve({ ota_urls: [otaUrl], lan_enabled: true }));
  expect(view.getByText(otaUrl)).toBeTruthy();
  expect(view.queryByRole('button', { name: 'Retry' })).toBeNull();
});

test('a successful read with no addresses shows the empty state', async () => {
  const endpoints = spyOn(robot.endpoints, 'invoke').mockResolvedValue({ ota_urls: [], lan_enabled: true });
  restores.push(() => endpoints.mockRestore());
  const view = renderModal();

  await waitFor(() => expect(view.getByText('No LAN address is available yet')).toBeTruthy());
  expect(view.queryByRole('status')).toBeNull();
  expect(view.queryByRole('alert')).toBeNull();
});

test('reopening clears the previous addresses and activation code while it loads again', async () => {
  const reopened = deferred<IApiRobotEndpoints>();
  const endpoints = spyOn(robot.endpoints, 'invoke')
    .mockResolvedValueOnce({ ota_urls: [otaUrl], lan_enabled: true })
    .mockReturnValueOnce(reopened.promise);
  restores.push(() => endpoints.mockRestore());
  const view = renderModal();
  await waitFor(() => expect(view.getByText(otaUrl)).toBeTruthy());
  fireEvent.change(view.getByPlaceholderText('Activation code'), { target: { value: '123456' } });

  view.setVisible(false);
  view.setVisible(true);
  expect(endpoints).toHaveBeenCalledTimes(2);
  expect(view.queryByText(otaUrl)).toBeNull();
  expect(view.getByRole('status')).toBeTruthy();
  expect((view.getByPlaceholderText('Activation code') as HTMLInputElement).value).toBe('');
  await act(async () => reopened.resolve({ ota_urls: [], lan_enabled: true }));
  expect(view.getByText('No LAN address is available yet')).toBeTruthy();
});

test('a request from a closed dialog cannot replace the next opening with stale addresses', async () => {
  const previous = deferred<IApiRobotEndpoints>();
  const current = deferred<IApiRobotEndpoints>();
  const endpoints = spyOn(robot.endpoints, 'invoke')
    .mockReturnValueOnce(previous.promise)
    .mockReturnValueOnce(current.promise);
  restores.push(() => endpoints.mockRestore());
  const view = renderModal();
  view.setVisible(false);
  view.setVisible(true);

  await act(async () => previous.resolve({ ota_urls: [otaUrl], lan_enabled: true }));
  expect(view.queryByText(otaUrl)).toBeNull();
  expect(view.getByRole('status')).toBeTruthy();
  await act(async () => current.resolve({ ota_urls: [], lan_enabled: true }));
  expect(view.getByText('No LAN address is available yet')).toBeTruthy();
});

test('a failed request from a closed dialog cannot replace the latest successful read', async () => {
  const previous = deferred<IApiRobotEndpoints>();
  const endpoints = spyOn(robot.endpoints, 'invoke')
    .mockReturnValueOnce(previous.promise)
    .mockResolvedValueOnce({ ota_urls: [otaUrl], lan_enabled: true });
  restores.push(() => endpoints.mockRestore());
  const view = renderModal();
  view.setVisible(false);
  view.setVisible(true);
  await waitFor(() => expect(view.getByText(otaUrl)).toBeTruthy());

  await act(async () => previous.reject(new Error('Obsolete endpoint request')));
  expect(view.getByText(otaUrl)).toBeTruthy();
  expect(view.queryByRole('alert')).toBeNull();
});

test('LAN access can still be enabled before refreshing the robot addresses', async () => {
  const lifecycleSupported = webui.lifecycleSupported;
  webui.lifecycleSupported = true;
  restores.push(() => { webui.lifecycleSupported = lifecycleSupported; });
  const endpoints = spyOn(robot.endpoints, 'invoke')
    .mockResolvedValueOnce({ ota_urls: [], lan_enabled: false })
    .mockResolvedValueOnce({ ota_urls: [otaUrl], lan_enabled: true });
  const start = spyOn(webui.start, 'invoke').mockResolvedValue({
    running: true, port: 25808, allowRemote: true, localUrl: 'http://localhost:25809', adminUsername: 'admin',
  });
  const toast = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [endpoints, start, toast]) restores.push(() => spy.mockRestore());
  const view = renderModal();

  await waitFor(() => expect(view.getByText('LAN access is disabled')).toBeTruthy());
  fireEvent.click(view.getByRole('button', { name: 'Enable LAN access' }));
  await waitFor(() => expect(view.getByText(otaUrl)).toBeTruthy());
  expect(start).toHaveBeenCalledTimes(1);
  expect(endpoints).toHaveBeenCalledTimes(2);
  expect(view.queryByText('LAN access is disabled')).toBeNull();
});

test('pairing completes with the companion identity without creating a device Agent selection', async () => {
  const endpoints = spyOn(robot.endpoints, 'invoke').mockResolvedValue({ ota_urls: [], lan_enabled: true });
  const claim = spyOn(robot.claim, 'invoke').mockResolvedValue({
    robot_id: 'aa:bb:cc:dd:ee:ff', companion_id: companionId, name: 'Robot', board: '',
    firmware_version: '', last_seen: null, created_at: '', capability_revision: 0,
    supported_permissions: ['proactive_speech'],
    permissions: { vision: false, motion: false, display: true, device_tools: false,
      proactive_speech: false, continuous_vision: false },
  });
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke');
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke');
  const toast = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [endpoints, claim, options, save, toast]) restores.push(() => spy.mockRestore());
  const view = renderModal();
  expect(view.getByText('Uses Nomi conversation and Agent settings')).toBeTruthy();
  fireEvent.change(view.getByPlaceholderText('Activation code'), { target: { value: '123456' } });
  fireEvent.click(view.getByRole('button', { name: 'Pair' }));
  await waitFor(() => expect(view.onCancel).toHaveBeenCalledTimes(1));
  expect(view.onClaimed).toHaveBeenCalledTimes(1);
  expect(claim).toHaveBeenCalledWith({ code: '123456', companion_id: companionId });
  expect(options).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});