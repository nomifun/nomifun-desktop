import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message, Modal } from '@arco-design/web-react';
import { connections, type ConnectionDevice } from '@/common/adapter/connections';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import * as webuiContext from '@/renderer/hooks/context/WebuiServerContext';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import PhoneConnectionPanel from './PhoneConnectionPanel';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { settings, common: { cancel: 'Cancel' } } } } });
afterEach(() => { cleanup(); mock.restore(); });

function captureTicketPoll() {
  const callbacks: Array<() => void | Promise<void>> = [];
  const original = window.setInterval.bind(window);
  const capture = ((handler: TimerHandler, timeout?: number, ...args: any[]) => {
    if (timeout === 2_000) {
      callbacks.push(handler as () => void | Promise<void>);
      return original(() => {}, 3_600_000);
    }
    return original(handler, timeout, ...args);
  }) as typeof window.setInterval;
  spyOn(window, 'setInterval').mockImplementation(capture);
  return async () => { await act(async () => { for (const callback of callbacks) await callback(); }); };
}

function fixture(rows: ConnectionDevice[] = [], native = true, allowed = true) {
  const accessUrls = ['http://192.168.1.50:25808', 'http://10.0.0.50:25808'];
  spyOn(webuiContext, 'useWebuiServer').mockReturnValue({
    status: { running: true, port: 25808, allowRemote: true, localUrl: 'http://127.0.0.1:25808', networkUrls: accessUrls, adminUsername: 'admin' },
    running: native, starting: false, lifecycleSupported: native, initialPassword: null,
    accessUrls, start: async () => {}, stop: async () => {}, refresh: async () => {}, clearInitialPassword: () => {},
  });
  spyOn(connections.info, 'invoke').mockResolvedValue({ desktop_id: 'desktop-A', desktop_name: 'Work Desktop', protocol_version: 2, min_client_protocol: 2, capabilities: ['device_pairing'] });
  const devices = spyOn(connections.devices, 'invoke').mockResolvedValue(rows);
  if (!allowed) devices.mockRejectedValue(new BackendHttpError({ method: 'GET', path: '/api/connections/devices', status: 403, body: { error: 'installation owner required' } }));
  const pairing = spyOn(connections.pairing, 'invoke').mockResolvedValue({ ticket_id: 'ticket-1', qr_token: 'synthetic-1', expires_at_ms: Date.now() + 120_000 });
  const status = spyOn(connections.pairingStatus, 'invoke').mockResolvedValue({ ticket_id: 'ticket-1', state: 'ready', device_id: null, expires_at_ms: Date.now() + 120_000 });
  const view = render(<I18nextProvider i18n={i18n}><PhoneConnectionPanel /></I18nextProvider>);
  return { ...view, devices, pairing, status };
}

test('the LAN address value and arrow toggle the dropdown once per click', async () => {
  const view = fixture();
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  const ready = view.getByRole('status');
  expect(ready.textContent).toBe(settings.deviceConnections.pairReady);
  expect(ready.closest('.arco-alert')).toBeNull();
  expect(ready.parentElement?.contains(view.getByTestId('generate-phone-pairing'))).toBe(true);
  const select = view.getByRole('combobox', { name: settings.deviceConnections.addressLabel });
  const value = view.getByText('http://192.168.1.50:25808');
  const arrow = select.querySelector('svg')!;

  fireEvent.click(value);
  expect(select.getAttribute('aria-expanded')).toBe('true');
  expect(view.getByRole('option', { name: 'http://10.0.0.50:25808' })).toBeTruthy();
  fireEvent.click(value);
  expect(select.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(arrow);
  expect(select.getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(arrow);
  expect(select.getAttribute('aria-expanded')).toBe('false');
});

test('connection help stays out of the layout and opens by hover or focus without opening the address dropdown', async () => {
  const view = fixture();
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  const select = view.getByRole('combobox', { name: settings.deviceConnections.addressLabel });
  const addressHelp = view.getByRole('button', { name: i18n.t('settings.deviceConnections.helpLabel', { label: settings.deviceConnections.addressLabel }) });
  const pathHelp = view.getByRole('button', { name: i18n.t('settings.deviceConnections.helpLabel', { label: settings.deviceConnections.choosePath }) });
  const permissionsHelp = view.getByRole('button', { name: i18n.t('settings.deviceConnections.helpLabel', { label: settings.deviceConnections.scanStep }) });
  const pathHeading = view.getByRole('heading', { name: settings.deviceConnections.choosePath });
  const scanHeading = view.getByRole('heading', { name: settings.deviceConnections.scanStep });
  expect(pathHeading.nextElementSibling === pathHelp).toBe(true);
  expect(scanHeading.nextElementSibling === permissionsHelp).toBe(true);
  expect(Boolean(view.queryByText('Only pair your own trusted phones', { exact: true }))).toBe(false);
  const addressDescription = document.getElementById(select.getAttribute('aria-describedby')!);
  expect(addressDescription?.textContent).toBe(settings.deviceConnections.addressHint);
  expect(addressHelp.getAttribute('aria-describedby')).toBe(addressDescription?.id);
  const pathDescription = document.getElementById(pathHelp.getAttribute('aria-describedby')!);
  expect(pathDescription?.textContent).toContain(settings.deviceConnections.lanHint);
  expect(pathDescription?.textContent).toContain(settings.deviceConnections.crossHint);
  for (const text of [settings.deviceConnections.phoneIntro, settings.deviceConnections.addressHint, settings.deviceConnections.lanHint, settings.deviceConnections.crossHint, settings.deviceConnections.phonePermissions, settings.deviceConnections.devicesHint]) {
    expect(Boolean(view.getByText(text).closest('.sr-only'))).toBe(true);
  }
  for (const target of [view.getByText(settings.deviceConnections.addressLabel), addressHelp, pathHelp, permissionsHelp]) {
    fireEvent.click(target);
    expect(select.getAttribute('aria-expanded')).toBe('false');
  }

  expect(Boolean(view.queryByRole('tooltip'))).toBe(false);
  fireEvent.mouseEnter(addressHelp);
  await waitFor(() => expect(view.getByRole('tooltip').textContent).toBe(settings.deviceConnections.addressHint));
  expect(select.getAttribute('aria-expanded')).toBe('false');
  fireEvent.mouseLeave(addressHelp);
  await waitFor(() => expect(Boolean(view.queryByRole('tooltip'))).toBe(false));

  await act(async () => pathHelp.focus());
  expect(document.activeElement === pathHelp).toBe(true);
  await waitFor(() => {
    const tooltip = view.getByRole('tooltip');
    expect(tooltip.textContent).toContain(settings.deviceConnections.lanHint);
    expect(tooltip.textContent).toContain(settings.deviceConnections.crossHint);
  });
  expect(select.getAttribute('aria-expanded')).toBe('false');
  await act(async () => pathHelp.blur());
  await waitFor(() => expect(Boolean(view.queryByRole('tooltip'))).toBe(false));
});

test('selecting another LAN address clears the old code and generates pairing for that address', async () => {
  const view = fixture();
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(view.queryByTestId('phone-pairing-link')).toBeTruthy());

  const select = view.getByRole('combobox', { name: settings.deviceConnections.addressLabel });
  fireEvent.click(select);
  fireEvent.click(view.getByRole('option', { name: 'http://10.0.0.50:25808' }));
  await waitFor(() => expect(select.getAttribute('aria-expanded')).toBe('false'));
  expect(select.textContent).toContain('http://10.0.0.50:25808');
  expect(view.queryByTestId('phone-pairing-link')).toBeNull();
  expect(view.queryByTestId('phone-pairing-qr')).toBeNull();

  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(view.queryByTestId('phone-pairing-link')).toBeTruthy());
  const link = view.getByTestId('phone-pairing-link') as HTMLTextAreaElement;
  const pairingUrl = new URL(new URL(link.value).searchParams.get('url')!);
  expect(pairingUrl.origin).toBe('http://10.0.0.50:25808');
});

test('a generated ticket waits for backend redemption instead of claiming the phone is online', async () => {
  const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
  spyOn(Message, 'success').mockReturnValue(() => {});
  const view = fixture();
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByRole('button', { name: 'Generate phone pairing code' }));
  await waitFor(() => expect(view.getByText('Waiting for the phone to pair')).toBeTruthy());
  expect(view.pairing).toHaveBeenCalledWith({});
  expect(Boolean(view.queryByText('Phone has paired successfully'))).toBe(false);
  expect(Boolean(view.container.querySelector('[data-testid="phone-pairing-qr"] svg'))).toBe(true);
  const copyButton = view.getByRole('button', { name: settings.deviceConnections.pairCopy });
  const summary = view.getByText(settings.deviceConnections.pairManualCopy, { selector: 'summary' });
  const details = summary.closest('details')!;
  expect(copyButton.textContent?.trim()).toBe('');
  expect(Boolean(copyButton.querySelector('svg'))).toBe(true);
  expect(copyButton.parentElement === details.parentElement).toBe(true);
  expect(Boolean(copyButton.closest('summary, details'))).toBe(false);
  expect(details.open).toBe(false);
  const manualLink = view.getByTestId('phone-pairing-link') as HTMLTextAreaElement;
  fireEvent.click(copyButton);
  await waitFor(() => expect(copy).toHaveBeenCalledWith(manualLink.value));
  expect(details.open).toBe(false);
  fireEvent.click(summary);
  expect(details.open).toBe(true);
  expect(Boolean(view.queryByText(settings.deviceConnections.pairLinkLabel, { exact: true }))).toBe(false);
  expect(view.getByRole('textbox', { name: settings.deviceConnections.pairLinkLabel }) === manualLink).toBe(true);
  expect(manualLink.readOnly).toBe(true);
  const inner = new URL(new URL(manualLink.value).searchParams.get('url')!);
  expect(inner.origin).toBe('http://192.168.1.50:25808');
  expect(inner.pathname).toBe('/device-pair');
  expect(inner.searchParams.get('token')).toBe('synthetic-1');
});

test('a redeemed ticket produces a pairing receipt', async () => {
  const view = fixture();
  view.status.mockResolvedValue({ ticket_id: 'ticket-1', state: 'used', device_id: 'phone-A', expires_at_ms: Date.now() + 120_000 });
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByRole('button', { name: 'Generate phone pairing code' }));
  await waitFor(() => expect(view.getByText('Phone has paired successfully')).toBeTruthy());
  expect(Boolean(view.queryByText('Waiting for the phone to pair'))).toBe(false);
  expect(Boolean(view.queryByTestId('phone-pairing-link'))).toBe(false);
});

test('a failed redemption at the device limit reports the cause instead of a receipt', async () => {
  const view = fixture();
  view.status.mockResolvedValue({ ticket_id: 'ticket-1', state: 'failed', device_id: null, failure_code: 'DEVICE_LIMIT_REACHED', expires_at_ms: Date.now() + 120_000 });
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.pairFailedLimit))).toBe(true));
  expect(Boolean(view.queryByText('Phone has paired successfully'))).toBe(false);
  expect(Boolean(view.queryByTestId('phone-pairing-qr'))).toBe(false);
});

test('a used status without a device credential is a failure, not a receipt', async () => {
  const view = fixture();
  view.status.mockResolvedValue({ ticket_id: 'ticket-1', state: 'used', device_id: null, expires_at_ms: Date.now() + 120_000 });
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.pairFailed))).toBe(true));
  expect(Boolean(view.queryByText('Phone has paired successfully'))).toBe(false);
  expect(Boolean(view.queryByTestId('phone-pairing-completed'))).toBe(false);
});

test('a successful receipt clears prior status errors and stops polling before cleanup returns 404', async () => {
  const tick = captureTicketPoll();
  const view = fixture();
  view.status.mockRejectedValueOnce(new Error('temporary status failure'));
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.pairStatusFailed))).toBe(true));
  view.status.mockResolvedValue({ ticket_id: 'ticket-1', state: 'used', device_id: 'phone-A', expires_at_ms: Date.now() + 120_000 });
  await tick();
  expect(Boolean(view.queryByTestId('phone-pairing-completed'))).toBe(true);
  expect(Boolean(view.queryByText(settings.deviceConnections.pairStatusFailed))).toBe(false);
  const calls = view.status.mock.calls.length;
  view.status.mockRejectedValue(new BackendHttpError({ method: 'GET', path: '/api/connections/pairing/ticket-1', status: 404, body: { error: 'ticket cleaned up' } }));
  await tick();
  expect(view.status.mock.calls.length).toBe(calls);
  expect(Boolean(view.queryByTestId('phone-pairing-completed'))).toBe(true);
  expect(Boolean(view.queryByText(settings.deviceConnections.pairStatusFailed))).toBe(false);
});

test('an expired ticket stops polling and a missing ready ticket asks for a new code', async () => {
  const tick = captureTicketPoll();
  const view = fixture();
  view.status.mockResolvedValue({ ticket_id: 'ticket-1', state: 'expired', device_id: null, expires_at_ms: Date.now() + 120_000 });
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.pairExpired))).toBe(true));
  const calls = view.status.mock.calls.length;
  await tick();
  expect(view.status.mock.calls.length).toBe(calls);
  view.pairing.mockResolvedValue({ ticket_id: 'ticket-2', qr_token: 'synthetic-2', expires_at_ms: Date.now() + 120_000 });
  view.status.mockRejectedValue(new BackendHttpError({ method: 'GET', path: '/api/connections/pairing/ticket-2', status: 404, body: { error: 'ticket not found' } }));
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(Boolean(view.queryByText(settings.deviceConnections.pairInvalidated))).toBe(true));
  expect(Boolean(view.queryByTestId('phone-pairing-qr'))).toBe(false);
  expect(Boolean(view.queryByText(settings.deviceConnections.pairStatusFailed))).toBe(false);
});

test('authoritative revocation replaces the old pairing receipt with a removed-device warning', async () => {
  const row: ConnectionDevice = { device_id: 'phone-A', device_name: 'Phone A', created_at_ms: 1, last_seen_at_ms: 2, revoked_at_ms: null };
  const view = fixture([row]);
  view.status.mockResolvedValue({ ticket_id: 'ticket-1', state: 'used', device_id: 'phone-A', expires_at_ms: Date.now() + 120_000 });
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(Boolean(view.queryByTestId('phone-pairing-completed'))).toBe(true));
  view.devices.mockResolvedValue([{ ...row, revoked_at_ms: 3 }]);
  fireEvent.click(view.getByRole('button', { name: 'Refresh devices' }));
  await waitFor(() => expect(Boolean(view.queryByTestId('phone-pairing-revoked'))).toBe(true));
  expect(view.getByText(settings.deviceConnections.pairRemoved)).toBeTruthy();
  expect(Boolean(view.queryByTestId('phone-pairing-completed'))).toBe(false);
  expect(Boolean(view.queryByTestId('phone-pairing-link'))).toBe(false);
});

test('expired codes disappear and can be refreshed', async () => {
  const view = fixture();
  view.pairing.mockResolvedValue({ ticket_id: 'ticket-1', qr_token: 'synthetic-1', expires_at_ms: Date.now() - 1000 });
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  fireEvent.click(view.getByRole('button', { name: 'Generate phone pairing code' }));
  await waitFor(() => expect(view.getByText('The pairing code expired. Generate a new QR code.')).toBeTruthy());
  expect(Boolean(view.container.querySelector('[data-testid="phone-pairing-qr"] svg'))).toBe(false);
  expect(Boolean(view.queryByTestId('phone-pairing-link'))).toBe(false);
  expect(view.getByRole('button', { name: 'Generate a new pairing code' })).toBeTruthy();
});

test('removing one independently authorized phone preserves the second device', async () => {
  const row = (id: string): ConnectionDevice => ({ device_id: id, device_name: id, created_at_ms: 1, last_seen_at_ms: 2, revoked_at_ms: null });
  const view = fixture([row('Phone A'), row('Phone B')]);
  const revoke = spyOn(connections.revoke, 'invoke').mockResolvedValue({ ...row('Phone A'), revoked_at_ms: 3 });
  let confirm: Parameters<typeof Modal.confirm>[0] | undefined;
  spyOn(Modal, 'confirm').mockImplementation((options) => { confirm = options; return { close: () => {}, update: () => {} }; });
  await waitFor(() => expect(view.getByText('Phone B')).toBeTruthy());
  fireEvent.click(view.getAllByRole('button', { name: 'Remove phone' })[0]);
  expect(confirm?.content).toContain('Phone A');
  view.devices.mockResolvedValue([row('Phone B')]);
  await act(async () => { await confirm?.onOk?.(); });
  expect(revoke).toHaveBeenCalledWith('Phone A');
  expect(Boolean(view.queryByText('Phone A'))).toBe(false);
  expect(view.getByText('Phone B')).toBeTruthy();
});

test('an owner browser pairs through its real origin without native lifecycle controls', async () => {
  const view = fixture([], false);
  await waitFor(() => expect(view.getByText('Work Desktop')).toBeTruthy());
  expect(view.getByTestId('browser-pairing-address').textContent).toBe(window.location.origin);
  expect(Boolean(view.queryByText('Same network'))).toBe(false);
  expect(Boolean(view.queryByText('Enable network access'))).toBe(false);
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  await waitFor(() => expect(view.getByText('Waiting for the phone to pair')).toBeTruthy());
  expect(view.pairing).toHaveBeenCalledWith({});
});

test('a device credential is told to use an owner account and cannot issue more device credentials', async () => {
  const view = fixture([], false, false);
  await waitFor(() => expect(view.getAllByText(settings.deviceConnections.ownerRequired).length).toBeGreaterThan(0));
  expect(view.getByTestId('generate-phone-pairing').hasAttribute('disabled')).toBe(true);
  fireEvent.click(view.getByTestId('generate-phone-pairing'));
  expect(view.pairing).not.toHaveBeenCalled();
});
