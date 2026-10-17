import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import type { IProvider } from '@/common/config/storage';
import settings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import common from '@/renderer/services/i18n/locales/zh-CN/common.json';
import { aliasCapability, aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import { capabilityDraftFromResponse } from './providerModelAdvanced';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
const { default: EditModeModal } = await import('./EditModeModal');
const { default: ModelGatewaySetup } = await import('./ModelGatewaySetup');
restoreTransport();
beforeEach(installTransport);

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings, common } } },
  interpolation: { escapeValue: false },
});
afterEach(() => { cleanup(); mock.restore(); restoreTransport(); });

function expectLiteralInput(element: Element | null) {
  expect(element).not.toBeNull();
  expect(element!.getAttribute('autocapitalize')).toBe('none');
  expect(element!.getAttribute('autocorrect')).toBe('off');
  expect(element!.getAttribute('spellcheck')).toBe('false');
}

const provider: IProvider = {
  id: aliasProviderId, name: 'custom-provider', platform: 'custom', base_url: aliasProviderBaseUrl,
  auth_scheme: 'bearer', has_credentials: false, enabled: true, models: [], sort_order: 0,
};

describe('model configuration preserves literal text input', () => {
  for (const modelReadOnly of [false, true]) {
    test(`${modelReadOnly ? 'existing' : 'new'} model IDs, aliases and protocol fields disable system text transformations`, () => {
      const page = render(<I18nextProvider i18n={i18n}>
        <ModelDefinitionEditor value={{ model: 'vendor/custom-chat', displayName: 'custom alias', capabilities: [capabilityDraftFromResponse(aliasCapability)] }}
          onChange={() => {}} modelReadOnly={modelReadOnly} providerBaseUrl={aliasProviderBaseUrl}
          providerAuthScheme='bearer' manifests={{ chat: aliasProtocolManifest }} validationErrors={[]} />
      </I18nextProvider>);
      expectLiteralInput(page.getByLabelText(settings.modelId));
      if (!modelReadOnly) fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-model-alias-disclosure]')!);
      expectLiteralInput(page.container.querySelector('[data-model-alias-input]'));
      expectLiteralInput(page.container.querySelector('[data-effective-base-url]'));
      expectLiteralInput(page.container.querySelector('[data-endpoint-field="endpoint"]'));
      expectLiteralInput(page.container.querySelector('[data-provider-params-json] textarea'));
    });
  }

  test('provider addresses, authentication, API keys and AWS fields forward literal input attributes to native controls', async () => {
    spyOn(ipcBridge.modelProtocol.list, 'invoke').mockResolvedValue(aliasProtocolManifest);
    const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map() }}><ThemeProvider>
      <EditModeModal data={provider} onChange={async () => {}} modalProps={{ visible: true }} modalCtrl={{ close() {} }} />
    </ThemeProvider></SWRConfig></I18nextProvider>);
    expectLiteralInput(await page.findByLabelText(settings.apiEndpoint));
    expectLiteralInput(page.getByPlaceholderText('bearer / header_key:x-api-key'));
    expectLiteralInput(page.getByPlaceholderText(settings.apiKeyPlaceholder));
    expectLiteralInput(page.getByPlaceholderText('AKIA...'));
    expectLiteralInput(page.getByPlaceholderText('default'));
  });

  test('gateway addresses and keys disable system text transformations', async () => {
    spyOn(ipcBridge.modelGateway.meta, 'invoke').mockResolvedValue({
      contract_version: '1.0', operator: { name: 'custom gateway', homepage_url: null, console_url: null,
        purchase_url: null, terms_url: null, privacy_url: null }, capabilities: [], optional_endpoints: [],
    });
    const page = render(<I18nextProvider i18n={i18n}>
      <ModelGatewaySetup initialBaseUrl='https://gateway.example' onCreated={() => {}} onCancel={() => {}} />
    </I18nextProvider>);
    expectLiteralInput(page.getByLabelText(settings.modelGateway.address));
    fireEvent.click(page.getByRole('button', { name: settings.modelGateway.checkOperator }));
    await waitFor(() => expectLiteralInput(page.getByLabelText(settings.apiKey)));
    expectLiteralInput(page.getByLabelText(settings.modelProvider));
  });
});
