import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { SWRConfig } from 'swr';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { agentPlatform, companion } from '@/common/adapter/ipcBridge';
import { Message } from '@arco-design/web-react';
import ProductAgentBindingSelect from './ProductAgentBindingSelect';
import type { ProductAgentOptions } from '@/common/types/agentPlatform';
import { parseProviderId } from '@/common/types/ids';
import { BackendHttpError } from '@/common/adapter/httpBridge';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'agentSettings.productBinding.label': 'Agent setup',
  'agentSettings.template.companion.default.name': 'Companion',
  'agentSettings.template.chat.minimal.name': 'Minimal',
  'agentSettings.template.assistant.general.name': 'Assistant',
  'agentSettings.productBinding.chooseModelLater': 'Choose a model later',
  'agentSettings.productBinding.reasons.web_search': 'Model lacks web search',
  'agentSettings.productBinding.loadFailed': 'Could not load Agent options',
  'agentSettings.actions.retry': 'Retry',
  'agentSettings.productBinding.modelChanged': 'Model compatibility changed. Please choose again.',
} } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });
const initial: ProductAgentOptions = {
  selection: { kind: 'template', template_key: 'companion.default' }, needs_model: true,
  options: [
    { selection: { kind: 'template', template_key: 'companion.default' }, display_name: '', available: true, reason: null },
    { selection: { kind: 'template', template_key: 'chat.minimal' }, display_name: '', available: true, reason: null },
    { selection: { kind: 'template', template_key: 'assistant.general' }, display_name: '', available: false, reason: 'web_search' },
  ],
};
test('allows Agent selection before model setup and prevents selecting incompatible options', async () => {
  let state = structuredClone(initial);
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    return { selection: request.selection, needs_model: true };
  });
  const active = spyOn(companion.getCompanionSession, 'invoke').mockResolvedValue({ conversation_id: null });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, active, success]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='companion' targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='companion.default' />
  </I18nextProvider></SWRConfig>);
  await waitFor(() => expect(view.getByText('Choose a model later')).toBeTruthy());
  const selector = view.container.querySelector('.arco-select')!;
  expect(selector.className.includes('arco-select-disabled')).toBe(false);
  fireEvent.click(selector);
  await waitFor(() => expect(view.baseElement.textContent).toContain('Model lacks web search'));
  const blocked = view.baseElement.querySelector('.arco-select-option-disabled')!;
  await act(async () => { fireEvent.click(blocked); });
  expect(save).not.toHaveBeenCalled();
  fireEvent.click(view.getByText('Minimal'));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0].request).toEqual({ selection: { kind: 'template', template_key: 'chat.minimal' } });
  expect(active).not.toHaveBeenCalled();
});

test('rechecks options when the model changes and displays fetch failures instead of a default selection', async () => {
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async ({ model }) => {
    if (model?.model === 'unreachable') throw new Error('private raw HTTP error');
    return { ...initial, needs_model: !model, options: initial.options.map((option) => ({ ...option,
      available: option.selection.kind !== 'template' || option.selection.template_key !== 'assistant.general' || model?.model === 'compatible',
    })) };
  });
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke');
  for (const spy of [options, save]) restores.push(() => spy.mockRestore());
  const providerId = parseProviderId('019b0000-0000-7000-8000-000000000001');
  const cache = new Map();
  const panel = (model: string) => <SWRConfig value={{ provider: () => cache, dedupingInterval: 0 }}><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='customer' targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='customer-service.default' model={{ id: providerId, use_model: model }} />
  </I18nextProvider></SWRConfig>;
  const view = render(panel('compatible'));
  await waitFor(() => expect(view.container.querySelector('.arco-select')).toBeTruthy());
  view.rerender(panel('incompatible'));
  await waitFor(() => expect(options.mock.calls.some(([request]) => request.model?.model === 'incompatible')).toBe(true));
  await waitFor(() => expect(view.container.querySelector('.arco-select')).toBeTruthy());
  fireEvent.click(view.container.querySelector('.arco-select')!);
  await waitFor(() => expect(view.baseElement.querySelector('.arco-select-option-disabled')).toBeTruthy());
  fireEvent.click(view.baseElement.querySelector('.arco-select-option-disabled')!);
  expect(save).not.toHaveBeenCalled();
  view.rerender(panel('unreachable'));
  await waitFor(() => expect(view.getByText('Could not load Agent options')).toBeTruthy());
  expect(view.queryByText('private raw HTTP error')).toBeNull();
  expect(view.getByRole('button', { name: 'Retry' })).toBeTruthy();
});

test('a stale selection failure preserves the selected Agent and shows readable feedback', async () => {
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockResolvedValue(initial);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'PUT', path: '/api/product-agent-bindings/customer/private', status: 422,
    body: { code: 'MODEL_ROUTE_FEATURES_MISSING', error: 'private raw protocol envelope' },
  }));
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  for (const spy of [options, save, toast]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='customer' targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='customer-service.default' />
  </I18nextProvider></SWRConfig>);
  await waitFor(() => expect(view.container.querySelector('.arco-select')).toBeTruthy());
  fireEvent.click(view.container.querySelector('.arco-select')!);
  fireEvent.click(await view.findByText('Minimal'));
  await waitFor(() => expect(toast).toHaveBeenCalledWith('Model compatibility changed. Please choose again.'));
  expect(view.container.querySelector('.arco-select-view')?.textContent).toContain('Companion');
  expect(view.baseElement.textContent).not.toContain('private raw protocol envelope');
});
