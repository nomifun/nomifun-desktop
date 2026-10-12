/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import type { ModelGatewayAccountResponse, ModelGatewayMetaResponse } from '@/common/types/provider/modelGateway';
import { parseProviderId } from '@/common/types/ids';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import ModelGatewayDetails from './ModelGatewayDetails';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } }, interpolation: { escapeValue: false } });
const provider: IProvider = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000097'), platform: 'nomifun-model-gateway', name: 'Partner', base_url: 'https://gateway.example/v1', auth_scheme: 'bearer', has_credentials: true, models: [], enabled: true, sort_order: 0 };
const meta: ModelGatewayMetaResponse = { contract_version: '1.0', operator: { name: 'Partner', homepage_url: null, console_url: null, purchase_url: 'https://operator.example/buy', terms_url: null, privacy_url: null }, capabilities: [], optional_endpoints: [] };
const account: ModelGatewayAccountResponse = { contract_version: '1.0', plan: { name: 'Starter', period_start: null, period_end: null, quota: { unit: 'tokens', total: null, used: '345' } }, balance: { amount: '12345', currency: 'USD' }, key: { name: 'Desktop', expires_at: null, quota_unit: 'tokens', remaining_quota: null }, rate_limits: { requests_per_minute: null, tokens_per_minute: null, concurrent_requests: null } };
function mount(onChanged = mock(async () => undefined)) {
  const result = render(<I18nextProvider i18n={i18n}><ModelGatewayDetails provider={provider} onChanged={onChanged} /></I18nextProvider>);
  return { ...result, page: within(result.container), onChanged };
}
afterEach(() => { cleanup(); mock.restore(); });
describe('gateway account and catalog sync', () => {
  test('displays large balance, usage and key quotas without rounding financial strings', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    spyOn(ipcBridge.modelGateway.account, 'invoke').mockResolvedValue({ ...account,
      balance: { ...account.balance, amount: '9007199254740993' },
      plan: { ...account.plan!, quota: { ...account.plan!.quota, used: '9223372036854775807' } },
      key: { ...account.key, remaining_quota: '9007199254740993' },
    });
    const { page } = mount();
    await waitFor(() => expect(page.getByText('$90,071,992,547,409.93')).toBeDefined());
    expect(page.getByText('9,223,372,036,854,775,807 / Not disclosed tokens')).toBeDefined();
    expect(page.getByText('9,007,199,254,740,993 tokens')).toBeDefined();
  });
  test('shows minor-unit balance and unknown quotas without treating them as unlimited', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    spyOn(ipcBridge.modelGateway.account, 'invoke').mockResolvedValue(account);
    const open = spyOn(ipcBridge.shell.openExternal, 'invoke').mockResolvedValue(undefined);
    const { page, container } = mount();
    await waitFor(() => expect(page.getByText('$123.45')).toBeDefined());
    expect(page.getByText('345 / Not disclosed tokens')).toBeDefined();
    expect(page.getByText(settings.modelGateway.noExpiry)).toBeDefined();
    expect(container.textContent).not.toContain('Unlimited');
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.recharge }));
    expect(open.mock.calls).toEqual([['https://operator.example/buy']]);
  });
  test('syncs once and refreshes the canonical provider projection after success', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    spyOn(ipcBridge.modelGateway.account, 'invoke').mockResolvedValue(account);
    const sync = spyOn(ipcBridge.modelGateway.sync, 'invoke').mockResolvedValue({ added: 1, updated: 2, models: [] });
    const { page, onChanged } = mount();
    await waitFor(() => expect(page.getByText('$123.45')).toBeDefined());
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.syncCatalog }));
    await waitFor(() => expect(onChanged.mock.calls).toHaveLength(1));
    expect(sync.mock.calls).toEqual([[{ provider_id: provider.id }]]);
  });
  test('clears account and operator actions while the saved gateway destination is changing', async () => {
    const nextProvider = { ...provider, base_url: 'https://other.example/v1' };
    const metaCall = spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValueOnce(meta)
      .mockImplementation(() => new Promise(() => {}));
    spyOn(ipcBridge.modelGateway.account, 'invoke').mockResolvedValueOnce(account)
      .mockImplementation(() => new Promise(() => {}));
    const { page, rerender, onChanged } = mount();
    await waitFor(() => expect(page.getByText('$123.45')).toBeDefined());
    rerender(<I18nextProvider i18n={i18n}><ModelGatewayDetails provider={nextProvider} onChanged={onChanged} /></I18nextProvider>);
    await waitFor(() => expect(metaCall.mock.calls).toHaveLength(2));
    expect(page.queryByText('$123.45') === null).toBe(true);
    expect(page.queryByText('Partner') === null).toBe(true);
    expect(page.queryByRole('button', { name: settings.modelGateway.recharge }) === null).toBe(true);
  });
  test('ignores account replies from the previous saved gateway destination', async () => {
    let completeMeta!: (value: ModelGatewayMetaResponse) => void;
    let completeAccount!: (value: ModelGatewayAccountResponse) => void;
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke')
      .mockImplementationOnce(() => new Promise((resolve) => { completeMeta = resolve; }))
      .mockResolvedValue({ ...meta, operator: { ...meta.operator, name: 'Other operator' } });
    spyOn(ipcBridge.modelGateway.account, 'invoke')
      .mockImplementationOnce(() => new Promise((resolve) => { completeAccount = resolve; }))
      .mockResolvedValue({ ...account, balance: { ...account.balance, amount: '678' } });
    const { page, rerender, onChanged } = mount();
    rerender(<I18nextProvider i18n={i18n}><ModelGatewayDetails provider={{ ...provider, base_url: 'https://other.example/v1' }} onChanged={onChanged} /></I18nextProvider>);
    await waitFor(() => expect(page.getByText('$6.78')).toBeDefined());
    await act(async () => { completeMeta(meta); completeAccount(account); });
    expect(page.queryByText('$6.78') !== null).toBe(true);
    expect(page.queryByText('Other operator') !== null).toBe(true);
    expect(page.queryByText('$123.45') === null).toBe(true);
    expect(page.queryByText('Partner') === null).toBe(true);
  });
  test('refreshes the account after key-only connection edits without relying on public credential presence', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    const accountCall = spyOn(ipcBridge.modelGateway.account, 'invoke').mockResolvedValueOnce(account)
      .mockResolvedValue({ ...account, key: { ...account.key, name: 'Replacement key' } });
    const { page, rerender, onChanged } = mount();
    await waitFor(() => expect(page.getByText('Desktop')).toBeDefined());
    rerender(<I18nextProvider i18n={i18n}><ModelGatewayDetails provider={provider} onChanged={onChanged} refreshToken={1} /></I18nextProvider>);
    await waitFor(() => expect(page.queryByText('Replacement key') !== null).toBe(true));
    expect(accountCall.mock.calls).toEqual([[{ provider_id: provider.id }], [{ provider_id: provider.id }]]);
    expect(page.queryByText('Desktop') === null).toBe(true);
  });
  test('a failed catalog sync preserves the configured provider and reports a safe error', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    spyOn(ipcBridge.modelGateway.account, 'invoke').mockResolvedValue(account);
    const sync = spyOn(ipcBridge.modelGateway.sync, 'invoke').mockRejectedValue(new Error('sensitive response body'));
    const { page, container, onChanged } = mount();
    await waitFor(() => expect(page.getByText('$123.45')).toBeDefined());
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.syncCatalog }));
    await waitFor(() => expect(page.getByText(settings.modelGateway.syncFailed)).toBeDefined());
    expect(sync.mock.calls).toEqual([[{ provider_id: provider.id }]]);
    expect(onChanged.mock.calls).toHaveLength(0);
    expect(page.queryByText('$123.45') !== null).toBe(true);
    expect(container.textContent).not.toContain('sensitive response body');
  });
});
