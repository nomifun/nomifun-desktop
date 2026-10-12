/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import { configService } from '@/common/config/configService';
import type { ModelGatewayAccountResponse, ModelGatewayMetaResponse } from '@/common/types/provider/modelGateway';
import { parseProviderId } from '@/common/types/ids';
import { ThemeProvider } from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import ModelModalContent from './ModelModalContent';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
const provider: IProvider = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000098'), platform: 'nomifun-model-gateway', name: 'Partner', base_url: 'https://gateway.example/v1', auth_scheme: 'bearer', has_credentials: true, models: [], enabled: true, sort_order: 0 };
const otherProvider: IProvider = { ...provider, id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000099'), name: 'Other partner' };
const meta: ModelGatewayMetaResponse = { contract_version: '1.0', operator: { name: 'Operator', homepage_url: null, console_url: null, purchase_url: null, terms_url: null, privacy_url: null }, capabilities: [], optional_endpoints: [] };
const account: ModelGatewayAccountResponse = { contract_version: '1.0', plan: null, balance: { amount: '12345', currency: 'USD' }, key: { name: 'Original key', expires_at: null, quota_unit: 'tokens', remaining_quota: null }, rate_limits: { requests_per_minute: null, tokens_per_minute: null, concurrent_requests: null } };

afterEach(() => { cleanup(); mock.restore(); });

test('successful key-only edits refresh only the affected expanded gateway account even when public provider data is unchanged', async () => {
  spyOn(configService, 'whenReady').mockResolvedValue(undefined);
  spyOn(ipcBridge.application.applyKeepAwake, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  const list = spyOn(ipcBridge.mode.listProviders, 'invoke').mockImplementation(async () => [{ ...provider }, { ...otherProvider }]);
  spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
  let rotated = false;
  const accountCall = spyOn(ipcBridge.modelGateway.account, 'invoke').mockImplementation(async ({ provider_id }) => ({
    ...account, key: { ...account.key, name: provider_id === otherProvider.id ? 'Other key' : rotated ? 'Replacement key' : 'Original key' },
  }));
  const update = spyOn(ipcBridge.modelGateway.updateConnection, 'invoke').mockImplementation(async () => { rotated = true; return { ...provider }; });
  const result = render(<I18nextProvider i18n={i18n}><ThemeProvider><MemoryRouter><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><ModelModalContent /></SWRConfig></MemoryRouter></ThemeProvider></I18nextProvider>);
  const page = within(result.container);
  await waitFor(() => expect(page.queryByText('Partner') !== null).toBe(true));
  const partnerCard = page.getByText('Partner').closest('.arco-collapse-item')!;
  const otherCard = page.getByText('Other partner').closest('.arco-collapse-item')!;
  fireEvent.click(within(partnerCard as HTMLElement).getByRole('button', { name: settings.modelGateway.syncCatalog }));
  fireEvent.click(within(otherCard as HTMLElement).getByRole('button', { name: settings.modelGateway.syncCatalog }));
  await waitFor(() => expect(page.queryByText('Original key') !== null && page.queryByText('Other key') !== null).toBe(true));
  fireEvent.click(page.getByText('Partner').closest('.arco-collapse-item')!.querySelector('.i-icon-write')!.closest('button')!);
  const dialog = within(document.body);
  await waitFor(() => expect(dialog.queryByLabelText(settings.apiKey) !== null).toBe(true));
  fireEvent.change(dialog.getByLabelText(settings.apiKey), { target: { value: 'synthetic-replacement-key' } });
  fireEvent.click(dialog.getByRole('button', { name: common.save }));
  await waitFor(() => expect(page.queryByText('Replacement key') !== null).toBe(true));
  expect(update.mock.calls).toEqual([[{ provider_id: provider.id, base_url: 'https://gateway.example', name: 'Partner', api_key: 'synthetic-replacement-key' }]]);
  expect(list.mock.calls.length >= 2).toBe(true);
  expect(accountCall.mock.calls.filter(([request]) => request.provider_id === provider.id)).toHaveLength(2);
  expect(accountCall.mock.calls.filter(([request]) => request.provider_id === otherProvider.id)).toHaveLength(1);
  expect(page.queryByText('Original key') === null).toBe(true);
});
