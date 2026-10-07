/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { parseProviderId } from '@/common/types/ids';
import type { ModelGatewayMetaResponse } from '@/common/types/provider/modelGateway';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import { fromApiConversation } from '@/common/adapter/apiModelMapper';
import { currentModelProviderTarget } from '../../platforms/nomi/currentModelProviderTarget';
import GatewayBillingAction from './GatewayBillingAction';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
const model = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000096'), platform: 'nomifun-model-gateway', use_model: 'gpt' };
const meta: ModelGatewayMetaResponse = { contract_version: '1.0', operator: { name: 'Partner', homepage_url: null, console_url: 'https://operator.example/keys', purchase_url: 'https://operator.example/buy', terms_url: null, privacy_url: null }, capabilities: [], optional_endpoints: [] };
function mount(code = 'USER_LLM_PROVIDER_BILLING_REQUIRED', selected = model) {
  const result = render(<I18nextProvider i18n={i18n}><GatewayBillingAction code={code} model={selected} /></I18nextProvider>);
  return { ...result, page: within(result.container) };
}
afterEach(() => { cleanup(); mock.restore(); });
describe('gateway account action', () => {
  test('shows recharge for the real mapped conversation model shape after resolving its configured provider', async () => {
    const conversation = fromApiConversation({ conversation_id: '0190f5fe-7c00-7a00-8000-000000000001',
      name: 'Real account shape', type: 'nomi', created_at: 1, modified_at: 2, extra: {},
      model: { provider_id: model.id, model: 'mock-compatible' },
    });
    expect(conversation.model.platform).toBe('');
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    const target = currentModelProviderTarget(conversation.model, [model]);
    const result = render(<I18nextProvider i18n={i18n}><GatewayBillingAction code='USER_LLM_PROVIDER_BILLING_REQUIRED' model={target} /></I18nextProvider>);
    const page = within(result.container);
    await waitFor(() => expect(page.getByRole('button', { name: settings.modelGateway.recharge })).toBeDefined());
    expect(page.getByRole('link', { name: settings.modelGateway.openSettings })).toBeDefined();
  });
  test('resolves billing from the selected gateway metadata and opens its HTTPS purchase page through the system browser', async () => {
    const query = spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    const open = spyOn(ipcBridge.shell.openExternal, 'invoke').mockResolvedValue(undefined);
    const { page } = mount();
    await waitFor(() => expect(page.getByRole('button', { name: settings.modelGateway.recharge })).toBeDefined());
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.recharge }));
    expect(query.mock.calls).toEqual([[{ provider_id: model.id }]]);
    expect(open.mock.calls).toEqual([['https://operator.example/buy']]);
  });
  test('key or rate-limit failures offer the operator console instead of a payment action', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue(meta);
    const { page } = mount('USER_LLM_PROVIDER_AUTH_FAILED');
    await waitFor(() => expect(page.getByRole('button', { name: settings.modelGateway.links.console })).toBeDefined());
    expect(page.queryByRole('button', { name: settings.modelGateway.recharge })).toBeNull();
  });
  test('unsafe URLs leave model settings as the only action', async () => {
    spyOn(ipcBridge.modelGateway.providerMeta, 'invoke').mockResolvedValue({ ...meta, operator: { ...meta.operator, purchase_url: 'http://operator.example/buy?api_key=synthetic' } });
    const { page } = mount();
    await Promise.resolve();
    expect(page.queryByRole('button')).toBeNull();
    expect(page.getByRole('link', { name: settings.modelGateway.openSettings }).getAttribute('href')).toBe('#/settings/model');
  });
  test('does not infer gateway actions for another provider or untyped failure prose', () => {
    const query = spyOn(ipcBridge.modelGateway.providerMeta, 'invoke');
    const { container } = mount('UNKNOWN_ERROR', { ...model, platform: 'openai' });
    expect(container.textContent).toBe(''); expect(query.mock.calls).toHaveLength(0);
  });
});
