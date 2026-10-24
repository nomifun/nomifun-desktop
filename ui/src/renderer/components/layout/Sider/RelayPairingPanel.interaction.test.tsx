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
  const view = fixture({ state: 'connected', relay: 'relay.example.test:443', health: { backendReachable: true, tunnelReachable: true, checkedAtMs: 1 } });
  await waitFor(() => expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayReconnect }))).toBe(true));
  expect(Boolean(view.container.querySelector('input[type="password"]'))).toBe(false);
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayConnect }))).toBe(false);
  expect(view.getByLabelText(settings.deviceConnections.consoleUrl)).toBeTruthy();
  expect(view.getByRole('button', { name: settings.deviceConnections.consoleOpen })).toBeTruthy();
  expect(view.getByText(settings.deviceConnections.backendReady)).toBeTruthy();
  expect(view.getByText(settings.deviceConnections.tunnelReady)).toBeTruthy();
});

test('a refused second enrollment reports already-paired and keeps the field hidden', async () => {
  const view = fixture({ state: 'error', relay: 'relay.example.test:443', actionErrorCode: 'RELAY_ALREADY_PAIRED' });
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayErrorAlreadyPaired))).toBe(true));
  expect(Boolean(view.container.querySelector('input[type="password"]'))).toBe(false);
  expect(Boolean(view.queryByRole('button', { name: settings.deviceConnections.relayConnect }))).toBe(false);
});

test('a rejected bootstrap refreshes status and shows the action error instead of a generic one', async () => {
  const view = fixture();
  view.bootstrap.mockRejectedValue(new Error('ipc rejected'));
  view.status.mockResolvedValue({ state: 'error', actionErrorCode: 'RELAY_ENROL_EXPIRED', relay: 'relay.example.test:443', pendingBootstrap: true });
  const envelope = view.container.querySelector('input[type="password"]') as HTMLInputElement;
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayConnect }));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.relayErrorEnrolExpired))).toBe(true));
  expect(view.bootstrap).toHaveBeenCalledWith({ pairingEnvelope: 'nomifun-relay-pair:v2:opaque' });
  expect(Boolean(view.queryByText(settings.deviceConnections.relayOperationFailed))).toBe(false);
});

test('enrollment starts while the read-only component inventory is still pending', async () => {
  let resolve!: (next: IRelayPairingStatus) => void;
  const pending = new Promise<IRelayPairingStatus>((done) => { resolve = done; });
  const view = fixture({ state: 'disconnected' }, pending);
  const envelope = view.container.querySelector('input[type="password"]') as HTMLInputElement;
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  const connect = view.getByRole('button', { name: settings.deviceConnections.relayConnect }) as HTMLButtonElement;
  expect(connect.disabled).toBe(false);
  fireEvent.keyDown(envelope, { key: 'Enter', keyCode: 13 });
  await waitFor(() => expect(view.bootstrap).toHaveBeenCalledTimes(1));
  expect(view.bootstrap).toHaveBeenCalledWith({ pairingEnvelope: 'nomifun-relay-pair:v2:opaque' });
  await act(async () => { resolve({ state: 'disconnected', runtimeAvailable: false }); });
  expect(view.getByText(settings.deviceConnections.relayConnected)).toBeTruthy();
});

test('a missing component connects through automatic preparation with no extra confirmation', async () => {
  const view = fixture({ state: 'disconnected', runtimeAvailable: false, health: { backendReachable: false, tunnelReachable: false, checkedAtMs: 1 } });
  const confirm = spyOn(Modal, 'confirm');
  await waitFor(() => expect(view.status).toHaveBeenCalled());
  expect(Boolean(view.queryByText(settings.deviceConnections.backendUnavailable))).toBe(false);
  expect(Boolean(view.queryByText(settings.deviceConnections.tunnelUnavailable))).toBe(false);
  expect(view.getByLabelText(settings.deviceConnections.consoleUrl)).toBeTruthy();
  expect(view.getByText(settings.deviceConnections.consoleUrl)).toBeTruthy();
  const envelope = view.getByLabelText(settings.deviceConnections.envelopeLabel) as HTMLInputElement;
  const introHelp = view.getByRole('button', { name: i18n.t('settings.deviceConnections.helpLabel', { label: settings.deviceConnections.relayTitle }) });
  const enrollmentHelp = view.getByRole('button', { name: i18n.t('settings.deviceConnections.helpLabel', { label: settings.deviceConnections.envelopeLabel }) });
  const introDescription = document.getElementById(introHelp.getAttribute('aria-describedby')!);
  const enrollmentDescription = document.getElementById(enrollmentHelp.getAttribute('aria-describedby')!);
  expect(introDescription?.textContent).toBe(settings.deviceConnections.relayIntro);
  expect(Boolean(introDescription?.classList.contains('sr-only'))).toBe(true);
  expect(enrollmentDescription?.textContent).toBe(settings.deviceConnections.relayEnrollmentHint);
  expect(Boolean(enrollmentDescription?.classList.contains('sr-only'))).toBe(true);
  await act(async () => introHelp.focus());
  await waitFor(() => expect(view.getByRole('tooltip').textContent).toBe(settings.deviceConnections.relayIntro));
  await act(async () => introHelp.blur());
  await waitFor(() => expect(Boolean(view.queryByRole('tooltip'))).toBe(false));
  fireEvent.mouseEnter(enrollmentHelp);
  await waitFor(() => expect(view.getByRole('tooltip').textContent).toBe(settings.deviceConnections.relayEnrollmentHint));
  fireEvent.mouseLeave(enrollmentHelp);
  await waitFor(() => expect(Boolean(view.queryByRole('tooltip'))).toBe(false));
  const deploymentSummary = view.getByText(settings.deviceConnections.relaySetupHelp, { selector: 'summary' });
  const deploymentDetails = deploymentSummary.closest('details')!;
  expect(deploymentDetails.open).toBe(false);
  fireEvent.click(deploymentSummary);
  expect(deploymentDetails.open).toBe(true);
  expect(view.getByText(settings.deviceConnections.relayDeploymentHint)).toBeTruthy();
  expect(view.getByRole('button', { name: settings.deviceConnections.relayDeploymentGuide })).toBeTruthy();
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayConnect }));
  await waitFor(() => expect(view.bootstrap).toHaveBeenCalledTimes(1));
  expect(view.bootstrap).toHaveBeenCalledWith({ pairingEnvelope: 'nomifun-relay-pair:v2:opaque' });
  expect(confirm).not.toHaveBeenCalled();
});

test('a missing component on a restored pairing reconnects through automatic preparation', async () => {
  const view = fixture({ state: 'error', relay: 'relay.example.test:443', runtimeAvailable: false });
  await waitFor(() => expect(view.getByRole('button', { name: settings.deviceConnections.relayReconnect })).toBeTruthy());
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayReconnect }));
  await waitFor(() => expect(view.restart).toHaveBeenCalledTimes(1));
  expect(view.restart).toHaveBeenCalledWith();
});

test('a cached component reconnects without an extra confirmation', async () => {
  const view = fixture({ state: 'paused', relay: 'relay.example.test:443', runtimeAvailable: true });
  const confirm = spyOn(Modal, 'confirm');
  await waitFor(() => expect((view.getByRole('button', { name: settings.deviceConnections.relayReconnect }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayReconnect }));
  await waitFor(() => expect(view.restart).toHaveBeenCalledTimes(1));
  expect(view.restart).toHaveBeenCalledWith();
  expect(confirm).not.toHaveBeenCalled();
});

test('a failed preparation keeps reconnect available for an explicit retry', async () => {
  const view = fixture({ state: 'paused', relay: 'relay.example.test:443', runtimeAvailable: true });
  await waitFor(() => expect((view.getByRole('button', { name: settings.deviceConnections.relayReconnect }) as HTMLButtonElement).disabled).toBe(false));
  view.restart.mockRejectedValueOnce(new Error('runtime download interrupted'));
  view.status.mockResolvedValue({ state: 'error', relay: 'relay.example.test:443', runtimeAvailable: false, actionErrorCode: 'RELAY_OPERATION_FAILED' });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayReconnect }));
  await waitFor(() => expect(view.getByText(settings.deviceConnections.relayOperationFailed)).toBeTruthy());
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayReconnect }));
  await waitFor(() => expect(view.restart).toHaveBeenCalledTimes(2));
  expect(view.restart).toHaveBeenCalledWith();
});

test('cancelling connection preparation stops it and ignores its late failure', async () => {
  const view = fixture({ state: 'disconnected', runtimeAvailable: false });
  let reject!: (error: Error) => void;
  view.bootstrap.mockImplementation(() => new Promise<IRelayPairingStatus>((_resolve, fail) => { reject = fail; }));
  view.stop.mockResolvedValue({ state: 'disconnected', runtimeAvailable: false });
  await waitFor(() => expect(view.status).toHaveBeenCalled());
  const envelope = view.container.querySelector('input[type="password"]') as HTMLInputElement;
  fireEvent.change(envelope, { target: { value: 'nomifun-relay-pair:v2:opaque' } });
  fireEvent.click(view.getByRole('button', { name: settings.deviceConnections.relayConnect }));
  await waitFor(() => expect(view.bootstrap).toHaveBeenCalledTimes(1));
  fireEvent.click(view.getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(view.stop).toHaveBeenCalledTimes(1));
  await act(async () => { reject(new Error('preparation cancelled')); });
  expect(view.getByText(settings.deviceConnections.relayDisconnected)).toBeTruthy();
  expect(view.queryByText(settings.deviceConnections.relayOperationFailed)).toBeNull();
  expect(view.getByRole('button', { name: settings.deviceConnections.relayConnect })).toBeTruthy();
});
