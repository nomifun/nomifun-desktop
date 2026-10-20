import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import { computer, type ComputerStatus } from '@/common/adapter/ipcBridge';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import ComputerDriverStatus from './ComputerDriverStatus';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
const snapshot: ComputerStatus = { phase: 'ready', worker_version: 'test-worker', platform: 'linux', backend: 'x11', session_type: 'x11',
  wayland: false, atspi: true, last_error: null, running: true };
const show = () => render(<SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}>
  <I18nextProvider i18n={i18n}><ComputerDriverStatus /></I18nextProvider></SWRConfig>);
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).forEach(restore => restore()); });

test('shows the read-only driver snapshot and refreshes through the status API', async () => {
  const get = spyOn(computer.status, 'invoke').mockResolvedValue(snapshot);
  restores.push(() => get.mockRestore());
  const page = show();
  expect(await page.findByText('test-worker')).toBeTruthy();
  expect(page.getByText(settings.capabilityPermissions.computer.driver.running)).toBeTruthy();
  expect(page.queryByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeNull();
  expect(page.getByText('AT-SPI')).toBeTruthy();
  expect(page.getByText(settings.capabilityPermissions.computer.driver.unavailable)).toBeTruthy();
  fireEvent.click(page.getByRole('button', { name: settings.capabilityPermissions.refresh }));
  await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
});

test('keeps an idle worker and unprobed backend explicit', async () => {
  const get = spyOn(computer.status, 'invoke').mockResolvedValue({ ...snapshot, platform: 'windows', backend: null,
    phase: 'stopped', session_type: null, worker_version: null, running: false, last_error: 'The last worker exited.' });
  restores.push(() => get.mockRestore());
  const page = show();
  expect(await page.findByText(settings.capabilityPermissions.computer.driver.stopped)).toBeTruthy();
  expect(page.getByText('The last worker exited.')).toBeTruthy();
  expect(page.queryByText('AT-SPI')).toBeNull();
  expect(page.getAllByText(settings.capabilityPermissions.states.unknown).length).toBeGreaterThan(0);
  expect(page.queryByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeNull();
});

test('shows a status read failure without claiming driver availability', async () => {
  const get = spyOn(computer.status, 'invoke').mockRejectedValue(new Error('test read failure'));
  restores.push(() => get.mockRestore());
  const page = show();
  expect(await page.findByText(settings.capabilityPermissions.computer.driver.loadFailed)).toBeTruthy();
  expect(page.queryByText(settings.capabilityPermissions.computer.driver.running)).toBeNull();
  expect(page.queryByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeNull();
});

test('automatically removes the in-flight operation wait when the global driver becomes idle', async () => {
  const get = spyOn(computer.status, 'invoke').mockResolvedValue(snapshot)
    .mockResolvedValueOnce({ ...snapshot, phase: 'waiting_for_current_operation' });
  restores.push(() => get.mockRestore());
  const page = show();
  expect(await page.findByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeTruthy();
  await waitFor(() => {
    expect(get.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(page.queryByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeNull();
    expect(page.getByText(settings.capabilityPermissions.computer.driver.running)).toBeTruthy();
  }, { timeout: 3500 });
  page.unmount();
  const reads = get.mock.calls.length;
  await new Promise(resolve => setTimeout(resolve, 1200));
  expect(get).toHaveBeenCalledTimes(reads);
});

test('does not keep cached waiting as current state after an automatic status read fails', async () => {
  const get = spyOn(computer.status, 'invoke').mockRejectedValue(new Error('test refresh failure'))
    .mockResolvedValueOnce({ ...snapshot, phase: 'waiting_for_current_operation' });
  restores.push(() => get.mockRestore());
  const page = show();
  expect(await page.findByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeTruthy();
  expect(await page.findByText(settings.capabilityPermissions.computer.driver.loadFailed, {}, { timeout: 3500 })).toBeTruthy();
  expect(page.queryByText(settings.capabilityPermissions.computer.driver.waitingForCurrentOperation)).toBeNull();
  expect(page.queryByText(settings.capabilityPermissions.computer.driver.running)).toBeNull();
  expect(page.getAllByText(settings.capabilityPermissions.states.unknown).length).toBeGreaterThan(0);
});
