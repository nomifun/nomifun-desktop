import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Modal } from '@arco-design/web-react';
import { relayPairing, type IRelayPairingStatus } from '@/common/adapter/ipcBridge';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import RelayPairingPanel from './RelayPairingPanel';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { settings, common: { cancel: 'Cancel' } } } } });
afterEach(() => { cleanup(); mock.restore(); });

function fixture(initial: IRelayPairingStatus = { state: 'disconnected' }, pendingStatus?: Promise<IRelayPairingStatus>) {
  const status = spyOn(relayPairing.getStatus, 'invoke').mockResolvedValue({ runtimeAvailable: true, ...initial });
  if (pendingStatus) status.mockImplementation(() => pendingStatus);
  const bootstrap = spyOn(relayPairing.bootstrap, 'invoke').mockResolvedValue({ state: 'connected', runtimeAvailable: true });
  const restart = spyOn(relayPairing.restart, 'invoke').mockResolvedValue({ state: 'connected', runtimeAvailable: true });
  const stop = spyOn(relayPairing.stop, 'invoke').mockResolvedValue({ state: 'paused', runtimeAvailable: true });
  const disconnect = spyOn(relayPairing.disconnect, 'invoke').mockResolvedValue({ state: 'disconnected', runtimeAvailable: true });
  const forgetLocal = spyOn(relayPairing.forgetLocal, 'invoke').mockResolvedValue({ state: 'disconnected', runtimeAvailable: true });
  const view = render(<I18nextProvider i18n={i18n}><RelayPairingPanel onStatusChange={() => {}} /></I18nextProvider>);
  return { ...view, status, bootstrap, restart, stop, disconnect, forgetLocal };
}

test('a rejected relay lifecycle action shows its mapped action error message', async () => {
  const view = fixture({ state: 'error', actionErrorCode: 'RELAY_INVITE_USED', relay: 'relay.example.test:443', pendingBootstrap: true });
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayErrorInviteUsed))).toBe(true));
  expect(Boolean(view.queryByText(settings.deviceConnections.relayOperationFailed))).toBe(false);
});

test('an unconfirmed revoke offers local forget which removes the saved relay state', async () => {
  const view = fixture({ state: 'error', actionErrorCode: 'RELAY_REVOKE_UNCONFIRMED', relay: 'relay.example.test:443' });
  await waitFor(() => expect(view.getByRole('button', { name: settings.deviceConnections.relayForgetLocal })).toBeTruthy());
  let confirm: Parameters<typeof Modal.confirm>[0] | undefined;
  spyOn(Modal, 'confirm').mockImplementation((options) => { confirm = options; return { close: () => {}, update: () => {} }; });
  view.getByRole('button', { name: settings.deviceConnections.relayForgetLocal }).click();
  expect(confirm?.title).toBe(settings.deviceConnections.relayForgetLocal);
  expect(confirm?.content).toBe(settings.deviceConnections.relayForgetLocalConfirm);
  await act(async () => { await confirm?.onOk?.(); });
  expect(view.forgetLocal).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayForgetLocal }))).toBe(false));
});

test('unreadable relay state hides reconnect controls and offers local forget', async () => {
  const view = fixture({ state: 'error', errorCode: 'RELAY_STATE_UNREADABLE', enabled: false, relay: 'relay.example.test:443' });
  await waitFor(() => expect(view.getByRole('button', { name: settings.deviceConnections.relayForgetLocal })).toBeTruthy());
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayReconnect }))).toBe(false);
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayPause }))).toBe(false);
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayRemove }))).toBe(false);
  expect(view.getByText(settings.deviceConnections.relayErrorStateUnreadable)).toBeTruthy();
});

test('an existing relay connection hides the enrollment field and connect button', async () => {
  const view = fixture({ state: 'connected', relay: 'relay.example.test:443' });
  await waitFor(() => expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayReconnect }))).toBe(true));
  expect(Boolean(view.container.querySelector('input[type="password"]'))).toBe(false);
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayConnect }))).toBe(false);
  expect(view.getByRole('button', { name: settings.deviceConnections.consoleOpen })).toBeTruthy();
});

test('a refused second enrollment reports already-paired and keeps the field hidden', async () => {
  const view = fixture({ state: 'error', relay: 'relay.example.test:443', actionErrorCode: 'RELAY_ALREADY_PAIRED' });
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayErrorAlreadyPaired))).toBe(true));
  expect(Boolean(view.container.querySelector('input[type="password"]'))).toBe(false);
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayConnect }))).toBe(false);
});

test('a rejected bootstrap refreshes status and shows the action error instead of a generic one', async () => {
  const view = fixture();
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayRuntimeStatusPending))).toBe(false));
  view.bootstrap.mockRejectedValue(new Error('ipc rejected'));
  view.status.mockResolvedValue({ state: 'error', actionErrorCode: 'RELAY_ENROL_EXPIRED', relay: 'relay.example.test:443', pendingBootstrap: true });
  const envelope = view.container.querySelector('input[type="password"]') as HTMLInputElement;
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayConnect }));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayErrorEnrolExpired))).toBe(true));
  expect(view.bootstrap).toHaveBeenCalledWith({ pairingEnvelope: 'nomifun-relay-pair:v2:opaque', allowRuntimeDownload: false });
  expect(Boolean(view.queryByText(settings.deviceConnections.relayOperationFailed))).toBe(false);
});

test('first enrollment waits for local availability and requires the explicit download button when missing', async () => {
  let resolve!: (next: IRelayPairingStatus) => void;
  const pending = new Promise<IRelayPairingStatus>((done) => { resolve = done; });
  const view = fixture({ state: 'disconnected' }, pending);
  const envelope = view.container.querySelector('input[type="password"]') as HTMLInputElement;
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  const connect = view.getByRole('button', { name: settings.deviceConnections.relayConnect }) as HTMLButtonElement;
  expect(connect.disabled).toBe(true);
  expect(view.getByText(settings.deviceConnections.relayRuntimeStatusPending)).toBeTruthy();
  fireEvent.click(connect);
  fireEvent.keyDown(envelope, { key: 'Enter', keyCode: 13 });
  expect(view.bootstrap).not.toHaveBeenCalled();
  await act(async () => { resolve({ state: 'disconnected', runtimeAvailable: false }); });
  expect(view.getByText(settings.deviceConnections.relayDownloadHint)).toBeTruthy();
  expect(view.getByRole('button', { name: settings.deviceConnections.relayDownloadSource })).toBeTruthy();
  fireEvent.keyDown(envelope, { key: 'Enter', keyCode: 13 });
  expect(view.bootstrap).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayDownloadAndConnect }));
  await waitFor(() => expect(view.bootstrap).toHaveBeenCalledTimes(1));
  expect(view.bootstrap).toHaveBeenCalledWith({ pairingEnvelope: 'nomifun-relay-pair:v2:opaque', allowRuntimeDownload: true });
});

test('a cached component connects without a download or an extra confirmation', async () => {
  const view = fixture({ state: 'disconnected', runtimeAvailable: true });
  const confirm = spyOn(Modal, 'confirm');
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayRuntimeStatusPending))).toBe(false));
  const envelope = view.container.querySelector('input[type="password"]') as HTMLInputElement;
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayConnect }));
  await waitFor(() => expect(view.bootstrap).toHaveBeenCalledTimes(1));
  expect(view.bootstrap).toHaveBeenCalledWith({ pairingEnvelope: 'nomifun-relay-pair:v2:opaque', allowRuntimeDownload: false });
  expect(confirm).not.toHaveBeenCalled();
  expect(view.queryByText(settings.deviceConnections.relayDownloadHint)).toBeNull();
});

test('a missing component on a restored pairing offers explicit download and reconnect', async () => {
  const view = fixture({ state: 'error', relay: 'relay.example.test:443', runtimeAvailable: false, actionErrorCode: 'RELAY_AGENT_INSTALL_REQUIRED' });
  await waitFor(() => expect(view.getByRole('button', { name: settings.deviceConnections.relayDownloadAndReconnect })).toBeTruthy());
  expect(view.getByText(settings.deviceConnections.relayErrorAgentInstallRequired)).toBeTruthy();
  expect(view.getByText(settings.deviceConnections.relayDownloadHint)).toBeTruthy();
  expect(view.restart).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayDownloadAndReconnect }));
  await waitFor(() => expect(view.restart).toHaveBeenCalledTimes(1));
  expect(view.restart).toHaveBeenCalledWith({ allowRuntimeDownload: true });
});

test('a cached component reconnects with download disallowed', async () => {
  const view = fixture({ state: 'paused', relay: 'relay.example.test:443', runtimeAvailable: true });
  const confirm = spyOn(Modal, 'confirm');
  await waitFor(() => expect((view.getByRole('button', { name: settings.deviceConnections.relayReconnect }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayReconnect }));
  await waitFor(() => expect(view.restart).toHaveBeenCalledTimes(1));
  expect(view.restart).toHaveBeenCalledWith({ allowRuntimeDownload: false });
  expect(confirm).not.toHaveBeenCalled();
});

test('a component disappearing after status observation does not turn reconnect into a download', async () => {
  const view = fixture({ state: 'paused', relay: 'relay.example.test:443', runtimeAvailable: true });
  await waitFor(() => expect((view.getByRole('button', { name: settings.deviceConnections.relayReconnect }) as HTMLButtonElement).disabled).toBe(false));
  view.restart.mockRejectedValue(new Error('runtime missing'));
  view.status.mockResolvedValue({ state: 'error', relay: 'relay.example.test:443', runtimeAvailable: false, actionErrorCode: 'RELAY_AGENT_INSTALL_REQUIRED' });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayReconnect }));
  await waitFor(() => expect(view.getByRole('button', { name: settings.deviceConnections.relayDownloadAndReconnect })).toBeTruthy());
  expect(view.restart).toHaveBeenCalledTimes(1);
  expect(view.restart).toHaveBeenCalledWith({ allowRuntimeDownload: false });
});
