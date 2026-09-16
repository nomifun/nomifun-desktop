import '../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import messages from '@/renderer/services/i18n/locales/zh-CN/index';
import ChatModelSelector from './ChatModelSelector';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: messages } } });
afterEach(cleanup);
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
const provider: IProvider = {
  id: providerId, name: '测试供应商', enabled: true, platform: 'custom',
  base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: false,
  models: [
    { model: 'allowed-model', display_name: '对话模型' },
    { model: 'restricted-model', display_name: '受限模型' },
  ].map(model => ({ ...model, provider_id: providerId, enabled: true, sort_order: 0, capabilities: [], created_at: 1, updated_at: 1 })),
};

test('both hosts can restrict available models without losing display labels or provider identity', async () => {
  const choices: Array<[IProvider['id'], string]> = [];
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [provider] }, revalidateOnMount: false }}>
    <ChatModelSelector providers={[provider]} currentModel={{ ...provider, use_model: 'allowed-model' }}
      getAvailableModels={() => ['allowed-model']} onSelectModel={async (selected, model) => { choices.push([selected.id, model]); }} />
  </SWRConfig></MemoryRouter></I18nextProvider>);
  fireEvent.click(page.getByRole('button', { name: '对话模型' }));
  const menu = await within(document.body).findByRole('menu');
  expect(within(menu).queryByText('受限模型')).toBeNull();
  fireEvent.click(within(menu).getByText('对话模型'));
  expect(choices).toEqual([[provider.id, 'allowed-model']]);
});

test('read-only model bindings never open a picker', () => {
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}>
    <ChatModelSelector providers={[]} getAvailableModels={() => []} onSelectModel={async () => { throw new Error('Read-only'); }} disabled readOnlyLabel='固定模型' />
  </SWRConfig></MemoryRouter></I18nextProvider>);
  fireEvent.click(page.getByRole('button', { name: '固定模型' }));
  expect(within(document.body).queryByRole('menu')).toBeNull();
});
