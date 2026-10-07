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
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import { ThemeProvider } from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import EditModelGatewayModal from './EditModelGatewayModal';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
const provider: IProvider = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000098'), platform: 'nomifun-model-gateway', name: 'Partner', base_url: 'https://gateway.example/v1', auth_scheme: 'bearer', has_credentials: true, models: [], enabled: true, sort_order: 0 };
function mount() {
  const onChanged = mock(async () => undefined);
  const close = mock();
  const result = render(<I18nextProvider i18n={i18n}><ThemeProvider><EditModelGatewayModal data={provider} onChanged={onChanged} modalProps={{ visible: true }} modalCtrl={{ close }} /></ThemeProvider></I18nextProvider>);
  return { ...result, page: within(document.body), onChanged, close };
}
afterEach(() => { cleanup(); mock.restore(); });
describe('atomic gateway connection edit', () => {
  test('rotates one key using one aggregate update without reading saved plaintext', async () => {
    const update = spyOn(ipcBridge.modelGateway.updateConnection, 'invoke').mockResolvedValue(provider);
    const read = spyOn(ipcBridge.mode.getProviderApiKeys, 'invoke');
    const { page, close, onChanged } = mount();
    expect((page.getByLabelText(settings.apiKey) as HTMLInputElement).value).toBe('');
    fireEvent.change(page.getByLabelText(settings.apiKey), { target: { value: 'synthetic-replacement' } });
    fireEvent.click(page.getByRole('button', { name: common.save }));
    await waitFor(() => expect(close.mock.calls).toHaveLength(1));
    expect(update.mock.calls).toEqual([[{ provider_id: provider.id, base_url: 'https://gateway.example', name: 'Partner', api_key: 'synthetic-replacement' }]]);
    expect(onChanged.mock.calls).toHaveLength(1); expect(read.mock.calls).toHaveLength(0);
  });
  test('requires an explicit key before saving a changed address', async () => {
    const update = spyOn(ipcBridge.modelGateway.updateConnection, 'invoke').mockResolvedValue(provider);
    const { page } = mount();
    fireEvent.change(page.getByLabelText(settings.modelGateway.address), { target: { value: 'https://other.example' } });
    expect((page.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(true);
    expect(page.getByText('Will be sent to other.example')).toBeDefined();
    fireEvent.click(page.getByRole('button', { name: common.save }));
    expect(update.mock.calls).toHaveLength(0);
    fireEvent.change(page.getByLabelText(settings.apiKey), { target: { value: 'synthetic-key' } });
    expect((page.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);
  });
});
