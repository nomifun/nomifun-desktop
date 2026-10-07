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
import type { ModelGatewayCatalogResponse, ModelGatewayMetaResponse } from '@/common/types/provider/modelGateway';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import ModelGatewaySetup from './ModelGatewaySetup';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
const meta: ModelGatewayMetaResponse = { contract_version: '1.0', operator: { name: 'Example Partner', homepage_url: null, console_url: null, purchase_url: 'https://operator.example/purchase', terms_url: null, privacy_url: null }, capabilities: ['openai-response'], optional_endpoints: [] };
const catalog: ModelGatewayCatalogResponse = { contract_version: '1.0', imports: [], models: ['gpt', 'claude'].map((id) => ({ id, display_name: id.toUpperCase(), vendor: id, tasks: ['chat'], task_endpoints: { chat: { endpoints: ['openai-response'], preferred_endpoint: 'openai-response' } }, context_window: 128000, max_output_tokens: 8192, input_modalities: ['text'], traits: [], pricing: [], included_in_plan: true, status: 'available' })) };
const provider: IProvider = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000095'), platform: 'nomifun-model-gateway', name: 'Example Partner', base_url: 'https://gateway.example/v1', auth_scheme: 'bearer', has_credentials: true, models: [], enabled: true, sort_order: 0 };
const mount = (onCreated = mock()) => {
  const result = render(<I18nextProvider i18n={i18n}><ModelGatewaySetup initialBaseUrl='https://gateway.example/v1' onCreated={onCreated} onCancel={() => {}} /></I18nextProvider>);
  return { ...result, page: within(result.container), onCreated };
};
afterEach(() => { cleanup(); mock.restore(); });
describe('gateway setup', () => {
  test('checks operator before accepting a key, selects all, searches, and saves the selected model graph once', async () => {
    const metaCall = spyOn(ipcBridge.modelGateway.meta, 'invoke').mockResolvedValue(meta);
    const catalogCall = spyOn(ipcBridge.modelGateway.catalog, 'invoke').mockResolvedValue(catalog);
    const createCall = spyOn(ipcBridge.modelGateway.create, 'invoke').mockResolvedValue(provider);
    const { page, onCreated } = mount();
    expect(page.queryByLabelText(settings.apiKey)).toBeNull();
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.checkOperator }));
    await waitFor(() => expect(page.getByText('Example Partner')).toBeDefined());
    expect(metaCall.mock.calls).toEqual([[{ base_url: 'https://gateway.example' }]]);
    fireEvent.change(page.getByLabelText(settings.apiKey), { target: { value: 'synthetic-test-key' } });
    expect(page.getByText('Will be sent to gateway.example')).toBeDefined();
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.loadCatalog }));
    await waitFor(() => expect(page.getByText('Select all (2 / 2)')).toBeDefined());
    expect(catalogCall.mock.calls).toEqual([[{ base_url: 'https://gateway.example', api_key: 'synthetic-test-key' }]]);
    fireEvent.change(page.getByRole('textbox', { name: settings.modelGateway.searchModels }), { target: { value: 'claude' } });
    expect(page.queryByText('GPT')).toBeNull();
    fireEvent.click(page.getByRole('checkbox', { name: /CLAUDE/ }));
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.addSelected }));
    await waitFor(() => expect(onCreated.mock.calls).toHaveLength(1));
    expect(createCall.mock.calls).toEqual([[{ base_url: 'https://gateway.example', api_key: 'synthetic-test-key', name: 'Example Partner', models: ['gpt'] }]]);
  });
  test('changing the address invalidates stale operator replies and clears the key and catalog', async () => {
    let complete!: (value: ModelGatewayMetaResponse) => void;
    spyOn(ipcBridge.modelGateway.meta, 'invoke').mockImplementation(() => new Promise((resolve) => { complete = resolve; }));
    const { page } = mount();
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.checkOperator }));
    fireEvent.change(page.getByLabelText(settings.modelGateway.address), { target: { value: 'https://other.example' } });
    complete(meta);
    await Promise.resolve();
    expect(page.queryByText('Example Partner')).toBeNull();
    expect(page.queryByLabelText(settings.apiKey)).toBeNull();
  });
  test('a failed catalog fetch preserves the operator and cannot save a partial model graph', async () => {
    spyOn(ipcBridge.modelGateway.meta, 'invoke').mockResolvedValue(meta);
    spyOn(ipcBridge.modelGateway.catalog, 'invoke').mockRejectedValue(new Error('sensitive response body'));
    const createCall = spyOn(ipcBridge.modelGateway.create, 'invoke');
    const { page, container } = mount();
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.checkOperator }));
    await waitFor(() => expect(page.getByText('Example Partner')).toBeDefined());
    fireEvent.change(page.getByLabelText(settings.apiKey), { target: { value: 'synthetic-test-key' } });
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.loadCatalog }));
    await waitFor(() => expect(page.getByText(settings.modelGateway.catalogFailed)).toBeDefined());
    expect((page.getByRole('button', { name: settings.modelGateway.addSelected }) as HTMLButtonElement).disabled).toBe(true);
    expect(createCall.mock.calls).toHaveLength(0);
    expect(container.textContent).not.toContain('sensitive response body');
  });
});
