import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import messages from '@/renderer/services/i18n/locales/zh-CN/index';
import ChatModelSelector, { filterCompatibleChatModelGroups } from './ChatModelSelector';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: messages } } });
afterEach(cleanup);
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
const chatCapability = (functionCallingUnsupported = false) => ({
  task: 'chat' as const,
  traits: ['vision_input' as const],
  protocol: 'openai.chat_completions',
  connection_role: 'default',
  allow_cross_origin_credentials: false,
  provider_params: {},
  ...(functionCallingUnsupported
    ? { health: { status: 'unknown' as const, unsupported_technical_capabilities: ['function_calling' as const] } }
    : {}),
  created_at: 1,
  updated_at: 1,
});
const provider: IProvider = {
  id: providerId, name: '测试供应商', enabled: true, platform: 'custom',
  base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: false,
  models: [
    { model: 'allowed-model', display_name: '对话模型' },
    { model: 'restricted-model', display_name: '受限模型' },
  ].map(model => ({
    ...model,
    provider_id: providerId,
    enabled: true,
    sort_order: 0,
    capabilities: [chatCapability(model.model === 'restricted-model')],
    created_at: 1,
    updated_at: 1,
  })),
};

test('both hosts can restrict available models without losing display labels or provider identity', () => {
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [provider] }, revalidateOnMount: false }}>
    <ChatModelSelector providers={[provider]} currentModel={{ ...provider, use_model: 'allowed-model' }}
      getAvailableModels={() => ['allowed-model']} onSelectModel={async () => undefined} />
  </SWRConfig></MemoryRouter></I18nextProvider>);
  expect(page.getByRole('button', { name: '对话模型' })).toBeTruthy();
  expect(filterCompatibleChatModelGroups([
    { provider, models: ['allowed-model'] },
  ])[0]?.models).toEqual(['allowed-model']);
});

test('read-only model bindings never open a picker', () => {
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}>
    <ChatModelSelector providers={[]} getAvailableModels={() => []} onSelectModel={async () => { throw new Error('Read-only'); }} disabled readOnlyLabel='固定模型' />
  </SWRConfig></MemoryRouter></I18nextProvider>);
  fireEvent.click(page.getByRole('button', { name: '固定模型' }));
  expect(within(document.body).queryByRole('menu')).toBeNull();
});

test('a capability-constrained picker lists only models without negative evidence', () => {
  const groups = filterCompatibleChatModelGroups(
    [{ provider, models: ['allowed-model', 'restricted-model'] }],
    [],
    ['function_calling']
  );
  expect(groups[0]?.models).toEqual(['allowed-model']);
});

test('an external recovery action can open the controlled compatible-model picker', () => {
  const ControlledPicker = () => {
    const [open, setOpen] = useState(false);
    return <>
      <button onClick={() => setOpen(true)}>选择兼容模型</button>
      <output data-testid='picker-state'>{open ? 'open' : 'closed'}</output>
      <ChatModelSelector
        providers={[provider]}
        currentModel={{ ...provider, use_model: 'restricted-model' }}
        getAvailableModels={() => ['allowed-model', 'restricted-model']}
        requiredTechnicalCapabilities={['function_calling']}
        popupVisible={open}
        onPopupVisibleChange={setOpen}
        onSelectModel={async () => undefined}
      />
    </>;
  };
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [provider] }, revalidateOnMount: false }}>
    <ControlledPicker />
  </SWRConfig></MemoryRouter></I18nextProvider>);

  fireEvent.click(page.getByRole('button', { name: '选择兼容模型' }));
  expect(page.getByTestId('picker-state').textContent).toBe('open');
});

test('keeps model selection on the left and reasoning control on the right', async () => {
  const reasoningProvider: IProvider = {
    ...provider,
    models: provider.models.map(model => ({
      ...model,
      capabilities: model.capabilities.map(capability => ({
        ...capability,
        protocol: 'openai.chat_text',
      })),
    })),
  };
  const changes: Array<string | undefined> = [];
  const modelChanges: string[] = [];
  const ReasoningPicker = () => {
    const [effort, setEffort] = useState<'low' | 'medium' | 'high'>();
    return <ChatModelSelector
      providers={[reasoningProvider]}
      currentModel={{ ...reasoningProvider, use_model: 'allowed-model' }}
      getAvailableModels={() => ['allowed-model']}
      onSelectModel={async (_provider, model) => { modelChanges.push(model); }}
      reasoningEffortSupported
      reasoningEffort={effort}
      onReasoningEffortChange={(value) => {
        changes.push(value);
        setEffort(value);
      }}
    />;
  };
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [reasoningProvider] }, revalidateOnMount: false }}>
    <ReasoningPicker />
  </SWRConfig></MemoryRouter></I18nextProvider>);

  const controls = within(page.getByTestId('chat-model-selector-controls')).getAllByRole('button');
  expect(controls.map(button => button.getAttribute('aria-label'))).toEqual([
    '对话模型',
    '本会话思考深度: 自动',
  ]);
  fireEvent.click(page.getByRole('button', { name: '本会话思考深度: 自动' }));
  const high = await waitFor(() => page.getByTestId('chat-model-selector-reasoning-high'));
  expect(page.queryByTestId('nomi-model-option-allowed-model')).toBeNull();
  fireEvent.click(high);
  expect(changes).toEqual(['high']);
  expect(modelChanges).toEqual([]);
  expect(page.getByRole('button', { name: '对话模型' })).toBeTruthy();
  expect(page.getByRole('button', { name: '本会话思考深度: 高' })).toBeTruthy();
  fireEvent.click(page.getByRole('button', { name: '对话模型' }));
  await waitFor(() => page.getByTestId('nomi-model-option-allowed-model'));
  const activeModelMenu = await waitFor(() => within(document.body).getByRole('menu'));
  expect(within(activeModelMenu).queryByTestId('chat-model-selector-reasoning-auto')).toBeNull();
});
