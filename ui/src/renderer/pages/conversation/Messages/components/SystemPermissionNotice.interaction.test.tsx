import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { parseMessageId } from '@/common/types/ids';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import SystemPermissionNotice from './SystemPermissionNotice';
import type { SystemPermissionNotice as Notice } from '../systemPermissionNoticeModel';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages } } } });
const status = (state: 'granted' | 'denied' | 'unknown'): SystemPermissionStatus => ({ platform: 'macos', app_label: 'NomiFun',
  permissions: [{ kind: 'accessibility', state, can_request: false, can_open_settings: true,
    requires_restart_after_grant: false, capabilities: ['computer_use'] }] });
const notice: Notice = { id: 'permission-notice', turn_id: parseMessageId('01900000-0000-7000-8000-000000000001'),
  created_at: 1, sourceMessageIds: [], refusal_id: '10:call-1', request: { schema: 'system-permission-required.v1', capability: 'computer',
    action_id: 'computer/input', operation: 'left_click', permissions: ['accessibility'], execution: 'not_executed' } };
const wrap = (value: Notice) => <MemoryRouter initialEntries={['/conversation/session-1']}><I18nextProvider i18n={i18n}>
  <SystemPermissionNotice notice={value} /><Location /></I18nextProvider></MemoryRouter>;
const mount = () => render(wrap(notice));
const Location = () => <span data-testid='location'>{useLocation().pathname}{useLocation().search}</span>;
afterEach(() => cleanup());

test('a real request shows only its missing permission and opens existing settings', async () => {
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(status('denied'));
  try {
    const view = mount();
    await waitFor(() => expect(view.getByText('Required: Accessibility')).toBeTruthy());
    expect(view.queryByText(/Screen Recording/)).toBeNull();
    fireEvent.click(view.getByRole('button', { name: messages.systemPermission.grant }));
    await waitFor(() => expect(view.getByTestId('location').textContent).toBe('/settings/permissions?tab=computer-use'));
  } finally { get.mockRestore(); }
});

test('focus after grant removes the prompt without issuing a task or input request', async () => {
  let live = status('denied');
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockImplementation(async () => live);
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke');
  try {
    const view = mount();
    await waitFor(() => expect(get).toHaveBeenCalledTimes(1));
    live = status('granted');
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    await waitFor(() => expect(view.queryByTestId('system-permission-notice')).toBeNull());
    const checks = get.mock.calls.length;
    live = status('denied');
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    expect(get.mock.calls.length).toBe(checks);
    expect(send).not.toHaveBeenCalled();
  } finally { get.mockRestore(); send.mockRestore(); }
});

test('Later removes the request and focus does not reopen it', async () => {
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue(status('denied'));
  try {
    const view = mount();
    await waitFor(() => expect(get).toHaveBeenCalledTimes(1));
    fireEvent.click(view.getByRole('button', { name: messages.systemPermission.later }));
    expect(view.queryByTestId('system-permission-notice')).toBeNull();
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    expect(get).toHaveBeenCalledTimes(1);
    view.rerender(wrap({ ...notice, refusal_id: '11:call-2' }));
    expect(view.queryByTestId('system-permission-notice')).toBeNull();
    expect(get).toHaveBeenCalledTimes(1);
  } finally { get.mockRestore(); }
});

test('a new real refusal in the same Turn rechecks after an earlier grant', async () => {
  let live = status('granted');
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockImplementation(async () => live);
  try {
    const view = mount();
    await waitFor(() => expect(view.queryByTestId('system-permission-notice')).toBeNull());
    live = status('denied');
    view.rerender(wrap({ ...notice, refusal_id: '11:call-2' }));
    await waitFor(() => expect(view.getByText('Required: Accessibility')).toBeTruthy());
    expect(get).toHaveBeenCalledTimes(2);
  } finally { get.mockRestore(); }
});

test('failed or unknown live checks use an explicit retry instead of claiming missing access', async () => {
  const get = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockRejectedValue(new Error('offline'));
  try {
    const view = mount();
    await view.findByText(messages.systemPermission.checkFailedTitle);
    expect(view.queryByText('Required: Accessibility')).toBeNull();
    expect(view.queryByRole('button', { name: messages.systemPermission.grant })).toBeNull();
    get.mockResolvedValue(status('unknown'));
    fireEvent.click(view.getByRole('button', { name: messages.systemPermission.retryCheck }));
    await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
    expect(view.getByText(messages.systemPermission.checkFailedTitle)).toBeTruthy();
    get.mockResolvedValue(status('granted'));
    fireEvent.click(view.getByRole('button', { name: messages.systemPermission.retryCheck }));
    await waitFor(() => expect(view.queryByTestId('system-permission-notice')).toBeNull());
  } finally { get.mockRestore(); }
});
