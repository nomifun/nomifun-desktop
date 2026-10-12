import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { SWRConfig } from 'swr';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { Message } from '@arco-design/web-react';
import ProductAgentBindingSelect from './ProductAgentBindingSelect';
import type { ProductAgentOptions } from '@/common/types/agentPlatform';
import { parseConversationId, parseProviderId } from '@/common/types/ids';
import { BackendHttpError } from '@/common/adapter/httpBridge';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'agentSettings.productBinding.label': 'Agent setup',
  'agentSettings.productBinding.editAgent': 'Edit Agent',
  'agentSettings.template.companion.default.name': 'Companion',
  'agentSettings.template.chat.minimal.name': 'Minimal',
  'agentSettings.template.assistant.general.name': 'Assistant',
  'agentSettings.productBinding.chooseModelLater': 'Choose a model later',
  'agentSettings.productBinding.reasons.web_search': 'Model lacks web search',
  'agentSettings.productBinding.loadFailed': 'Could not load Agent options',
  'agentSettings.actions.retry': 'Retry',
  'agentSettings.productBinding.modelChanged': 'Model compatibility changed. Please choose again.',
  'agentSettings.productBinding.applyLatest': 'Apply latest setup',
  'agentSettings.productBinding.applyLatestHint': 'Apply saved Agent edits to this conversation',
  'agentSettings.productBinding.reasons.companion': 'This Agent needs the Companion capability',
  'agentSettings.productBinding.busy': 'Wait for the current reply to finish',
  'agentSettings.productBinding.partialSave': 'Current conversation updated; retry saving the default setup',
  'agentSettings.productBinding.officialTag': 'Official',
  'agentSettings.productBinding.customTag': 'Custom',
  'agentSettings.productBinding.reasons.removed': 'This Agent was removed',
  'agentSettings.productBinding.currentUnavailable': 'Current Agent is unavailable: {{reason}}',
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

test('compact product Agent selector updates a product binding with its existing session', async () => {
  let state = { ...structuredClone(initial), needs_model: false };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    return { selection: request.selection, needs_model: false };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, success]) restores.push(() => spy.mockRestore());
  const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000002');
  const providerId = parseProviderId('019b0000-0000-7000-8000-000000000003');
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect compact targetKind='customer'
      targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='customer-service.default'
      conversationId={conversationId} model={{ id: providerId, use_model: 'model-a' }} />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  fireEvent.click(await view.findByRole('button', { name: 'Agent setup: Companion' }));
  fireEvent.click(await view.findByText('Minimal'));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0]).toEqual({
    target_kind: 'customer', target_id: '019b0000-0000-7000-8000-000000000001',
    request: { selection: { kind: 'template', template_key: 'chat.minimal' },
      model: { provider_id: providerId, model: 'model-a' }, conversation_id: conversationId },
  });
  await waitFor(() => expect(view.getByRole('button', { name: 'Agent setup: Minimal' })).toBeTruthy());
});

test.each([false, true])('companion selector saves a personal Agent and explicitly reapplies its latest setup (compact=%s)', async (compact) => {
  const companionId = '019b0000-0000-7000-8000-000000000001';
  const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000002');
  const providerId = parseProviderId('019b0000-0000-7000-8000-000000000003');
  const personal = { kind: 'preset' as const, preset_id: '019b0000-0000-7000-8000-000000000004' };
  let state: ProductAgentOptions = {
    ...structuredClone(initial), needs_model: false,
    options: [initial.options[0],
      { selection: personal, display_name: 'My Companion', available: true, reason: null },
      { ...initial.options[1], available: false, reason: 'companion' }],
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    return { selection: request.selection, needs_model: false };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, success]) restores.push(() => spy.mockRestore());
  let refreshed = 0;
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect compact={compact} targetKind='companion' targetId={companionId}
      defaultTemplateKey='companion.default' model={{ id: providerId, use_model: 'model-a' }}
      conversationId={compact ? conversationId : undefined} onChanged={() => { refreshed++; }} />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  const trigger = compact
    ? await view.findByRole('button', { name: 'Agent setup: Companion' })
    : await waitFor(() => { const node = view.container.querySelector('.arco-select'); expect(node).toBeTruthy(); return node!; });
  fireEvent.click(trigger);
  fireEvent.click(await view.findByText('My Companion'));
  await waitFor(() => expect(refreshed).toBe(1));
  expect(save.mock.calls[0][0]).toEqual({
    target_kind: 'companion', target_id: companionId,
    request: { selection: personal, model: { provider_id: providerId, model: 'model-a' },
      ...(compact ? { conversation_id: conversationId } : {}) },
  });
  if (compact) {
    fireEvent.click(await view.findByRole('button', { name: 'Agent setup: My Companion' }));
    fireEvent.click(await view.findByText('Apply latest setup'));
  } else {
    fireEvent.click(await view.findByRole('button', { name: 'Apply latest setup' }));
  }
  await waitFor(() => expect(refreshed).toBe(2));
  expect(save.mock.calls[1][0]).toEqual(save.mock.calls[0][0]);
});

test.each([false, true])('companion editing follows the selected official or personal Agent (compact=%s)', async (compact) => {
  const personal = { kind: 'preset' as const, preset_id: 'personal-companion' };
  let state: ProductAgentOptions = { ...structuredClone(initial), needs_model: false,
    options: [initial.options[0], { selection: personal, display_name: 'My Companion', available: true, reason: null }],
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    return { selection: request.selection, needs_model: false };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, success]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect compact={compact} targetKind='companion' targetId='companion-editor'
      defaultTemplateKey='companion.default' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  if (compact) fireEvent.click(await view.findByRole('button', { name: 'Agent setup: Companion' }));
  expect((await view.findByRole('link', { name: 'Edit Agent' })).getAttribute('href')).toBe('/agent?template=companion.default');
  if (!compact) fireEvent.click(view.container.querySelector('.arco-select')!);
  fireEvent.click(await view.findByText('My Companion'));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  if (compact) fireEvent.click(await view.findByRole('button', { name: 'Agent setup: My Companion' }));
  await waitFor(() => expect(view.getByRole('link', { name: 'Edit Agent' }).getAttribute('href')).toBe('/agent?preset=personal-companion'));
});

test.each([false, true])('companion menu only lists Companion presets and distinguishes authored origin (compact=%s)', async (compact) => {
  const state: ProductAgentOptions = {
    ...structuredClone(initial), needs_model: false,
    options: [initial.options[0],
      { ...initial.options[1], available: false, reason: 'companion' },
      { selection: { kind: 'preset', preset_id: 'personal-companion' }, display_name: 'companion.default', available: true, reason: null },
      { selection: { kind: 'preset', preset_id: 'companion-web' }, display_name: 'Companion Web', available: false, reason: 'web_search' },
      { selection: { kind: 'preset', preset_id: 'removed-companion' }, display_name: 'Removed Companion', available: false, reason: 'removed' }],
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockResolvedValue(state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke');
  for (const spy of [options, save]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect compact={compact} targetKind='companion' targetId='companion-filter'
      defaultTemplateKey='companion.default' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  const trigger = compact
    ? await view.findByRole('button', { name: 'Agent setup: Companion' })
    : await waitFor(() => { const node = view.container.querySelector('.arco-select-size-mini.nomi-select'); expect(node).toBeTruthy(); return node!; });
  fireEvent.click(trigger);
  const menu = await waitFor(() => {
    const node = view.baseElement.querySelector(compact ? '.arco-dropdown-menu' : '.arco-select-popup');
    expect(node).toBeTruthy(); return node!;
  });
  expect(menu.textContent).not.toContain('Minimal');
  expect(menu.textContent).not.toContain('Removed Companion');
  expect(menu.textContent).toContain('companion.default');
  expect(menu.textContent).toContain('Companion Web');
  expect(menu.textContent).toContain('Model lacks web search');
  expect(menu.querySelectorAll('[data-agent-origin="official"]')).toHaveLength(1);
  expect(menu.querySelectorAll('[data-agent-origin="custom"]')).toHaveLength(2);
  expect(save).not.toHaveBeenCalled();
});

test.each([false, true])('a removed current Companion stays visible while its menu only offers valid replacements (compact=%s)', async (compact) => {
  const state: ProductAgentOptions = { ...structuredClone(initial), needs_model: false,
    selection: { kind: 'preset', preset_id: 'removed-companion' },
    options: [initial.options[0], { selection: { kind: 'preset', preset_id: 'removed-companion' },
      display_name: 'My Removed Companion', available: false, reason: 'removed' }],
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockResolvedValue(state);
  restores.push(() => options.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect compact={compact} targetKind='companion' targetId='companion-removed' defaultTemplateKey='companion.default' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  const trigger = compact
    ? await view.findByRole('button', { name: 'Agent setup: My Removed Companion' })
    : await waitFor(() => { const node = view.container.querySelector('.arco-select'); expect(node).toBeTruthy(); return node!; });
  if (compact) {
    fireEvent.mouseEnter(trigger.parentElement!);
    expect((await view.findByRole('tooltip')).textContent).toContain('This Agent was removed');
  } else {
    const value = view.container.querySelector('.arco-select-view-value')!;
    expect(value.textContent).toContain('My Removed Companion');
    expect(value.textContent).toContain('Custom');
    expect(value.textContent).not.toContain('This Agent was removed');
    expect(view.getByRole('status').textContent).toContain('This Agent was removed');
    expect((view.getByRole('button', { name: 'Apply latest setup' }) as HTMLButtonElement).disabled).toBe(true);
  }
  fireEvent.click(trigger);
  const menu = await waitFor(() => {
    const node = view.baseElement.querySelector(compact ? '.arco-dropdown-menu' : '.arco-select-popup');
    expect(node).toBeTruthy(); return node!;
  });
  expect(menu.textContent).toContain('Companion');
  expect(menu.textContent).not.toContain('My Removed Companion');
  if (compact) expect(view.getByRole('menuitem', { name: 'Apply latest setup' }).getAttribute('aria-disabled')).toBe('true');
});

test('companion selector preserves its Agent when the existing conversation is busy', async () => {
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockResolvedValue(initial);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'PUT', path: '/api/product-agent-bindings/companion/private', status: 409,
    body: { code: 'AGENT_SESSION_TURN_ACTIVE', error: 'private operation identity' },
  }));
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  for (const spy of [options, save, toast]) restores.push(() => spy.mockRestore());
  let refreshed = false;
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='companion' targetId='019b0000-0000-7000-8000-000000000001'
      defaultTemplateKey='companion.default' onChanged={() => { refreshed = true; }} />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  const selector = await waitFor(() => { const node = view.container.querySelector('.arco-select'); expect(node).toBeTruthy(); return node!; });
  fireEvent.click(selector);
  fireEvent.click(await view.findByText('Minimal'));
  await waitFor(() => expect(toast).toHaveBeenCalledWith('Wait for the current reply to finish'));
  expect(refreshed).toBe(false);
  expect(view.container.querySelector('.arco-select-view')?.textContent).toContain('Companion');
  expect(view.baseElement.textContent).not.toContain('private operation identity');
});

test('companion selector shows the active Agent and can retry a failed default save after reopening', async () => {
  let state: ProductAgentOptions = { ...structuredClone(initial),
    selection: { kind: 'preset', preset_id: 'personal-companion' },
    options: [initial.options[0], { selection: { kind: 'preset', preset_id: 'personal-companion' },
      display_name: 'My Companion', available: true, reason: null }],
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    if (save.mock.calls.length === 1) throw new BackendHttpError({
      method: 'PUT', path: '/api/product-agent-bindings/companion/private', status: 500,
      body: { code: 'COMPANION_AGENT_DEFAULT_SAVE_FAILED', error: 'private storage detail' },
    });
    return { selection: request.selection, needs_model: true };
  });
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, toast, success]) restores.push(() => spy.mockRestore());
  const renderSelector = () => render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='companion' targetId='019b0000-0000-7000-8000-000000000001'
      defaultTemplateKey='companion.default' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  let view = renderSelector();
  const selector = await waitFor(() => { const node = view.container.querySelector('.arco-select'); expect(node).toBeTruthy(); return node!; });
  fireEvent.click(selector);
  fireEvent.click(await view.findByText('Companion'));
  await waitFor(() => expect(toast).toHaveBeenCalledWith('Current conversation updated; retry saving the default setup'));
  await waitFor(() => expect(view.container.querySelector('.arco-select-view')?.textContent).toContain('Companion'));
  expect(success).not.toHaveBeenCalled();
  expect(view.baseElement.textContent).not.toContain('private storage detail');
  view.unmount();
  view = renderSelector();
  fireEvent.click(await view.findByRole('button', { name: 'Apply latest setup' }));
  await waitFor(() => expect(success).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[1][0].request.selection).toEqual({ kind: 'template', template_key: 'companion.default' });
});
test('allows Agent selection before model setup and prevents selecting incompatible options', async () => {
  let state = structuredClone(initial);
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    return { selection: request.selection, needs_model: true };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, success]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='customer' targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='customer-service.default' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  await waitFor(() => expect(view.getByText('Choose a model later')).toBeTruthy());
  expect(view.queryByText('Using Agent')).toBeNull();
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
});

test('the sidebar Agent dropdown can retry a partial default save without management-page actions', async () => {
  let state: ProductAgentOptions = { ...structuredClone(initial), needs_model: false,
    options: [initial.options[0], { selection: { kind: 'preset', preset_id: 'personal-companion' }, display_name: 'My Companion', available: true, reason: null }],
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    state = { ...state, selection: request.selection };
    if (save.mock.calls.length === 1) throw new BackendHttpError({ method: 'PUT', path: '/private', status: 500,
      body: { code: 'COMPANION_AGENT_DEFAULT_SAVE_FAILED', error: 'private disk detail' } });
    return { selection: request.selection, needs_model: false };
  });
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, toast, success]) restores.push(() => spy.mockRestore());
  let refreshes = 0;
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='companion' targetId='sidebar-companion' defaultTemplateKey='companion.default'
      layout='stacked' showManagementActions={false} onChanged={() => { refreshes++; }} />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  await waitFor(() => expect(view.container.querySelector('.arco-select')).toBeTruthy());
  fireEvent.click(view.container.querySelector('.arco-select')!);
  fireEvent.click(await view.findByText('My Companion'));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('Current conversation updated; retry saving the default setup'));
  expect(refreshes).toBe(1);
  expect(success).not.toHaveBeenCalled();
  expect(view.queryByRole('link', { name: 'Edit Agent' })).toBeNull();
  expect(view.queryByRole('button', { name: 'Apply latest setup' })).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(success).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[1][0].request.selection).toEqual({ kind: 'preset', preset_id: 'personal-companion' });
  expect(refreshes).toBe(2);
});

test('an immediate partial-save retry keeps the submitted Agent while its options refresh is pending', async () => {
  const personal = { kind: 'preset' as const, preset_id: 'personal-companion' };
  const state: ProductAgentOptions = { ...structuredClone(initial), needs_model: false,
    options: [initial.options[0], { selection: personal, display_name: 'My Companion', available: true, reason: null }],
  };
  const pendingRefreshes: Array<(options: ProductAgentOptions) => void> = [];
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(() => {
    if (options.mock.calls.length === 1) return Promise.resolve(state);
    return new Promise((resolve) => pendingRefreshes.push(resolve));
  });
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    if (save.mock.calls.length === 1) throw new BackendHttpError({ method: 'PUT', path: '/private', status: 500,
      body: { code: 'COMPANION_AGENT_DEFAULT_SAVE_FAILED', error: 'private disk detail' } });
    return { selection: request.selection, needs_model: false };
  });
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [options, save, toast, success]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='companion' targetId='sidebar-pending-refresh' defaultTemplateKey='companion.default'
      layout='stacked' showManagementActions={false} />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  await waitFor(() => expect(view.container.querySelector('.arco-select')).toBeTruthy());
  fireEvent.click(view.container.querySelector('.arco-select')!);
  fireEvent.click(await view.findByText('My Companion'));
  await waitFor(() => expect(view.getByRole('button', { name: 'Retry' })).toBeTruthy());
  expect(pendingRefreshes.length).toBeGreaterThan(0);
  expect(view.container.querySelector('.arco-select-view')?.textContent).toContain('Companion');
  expect(view.container.querySelector('.arco-select-view')?.textContent).not.toContain('My Companion');
  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(save.mock.calls[1][0].request.selection).toEqual(personal);
  await act(async () => { pendingRefreshes.splice(0).forEach((resolve) => resolve({ ...state, selection: personal })); });
  await waitFor(() => expect(success).toHaveBeenCalledTimes(1));
  expect(view.queryByRole('button', { name: 'Retry' })).toBeNull();
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
  const panel = (model: string) => <SWRConfig value={{ provider: () => cache, dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect targetKind='customer' targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='customer-service.default' model={{ id: providerId, use_model: model }} />
  </I18nextProvider></MemoryRouter></SWRConfig>;
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

test.each([false, true])('a stale selection failure preserves the selected Agent (compact=%s)', async (compact) => {
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockResolvedValue(initial);
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'PUT', path: '/api/product-agent-bindings/customer/private', status: 422,
    body: { code: 'MODEL_ROUTE_FEATURES_MISSING', error: 'private raw protocol envelope' },
  }));
  const toast = spyOn(Message, 'error').mockImplementation(() => () => {});
  for (const spy of [options, save, toast]) restores.push(() => spy.mockRestore());
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <ProductAgentBindingSelect compact={compact} targetKind='customer' targetId='019b0000-0000-7000-8000-000000000001' defaultTemplateKey='customer-service.default' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  const trigger = compact
    ? await view.findByRole('button', { name: 'Agent setup: Companion' })
    : await waitFor(() => { const node = view.container.querySelector('.arco-select'); expect(node).toBeTruthy(); return node!; });
  fireEvent.click(trigger);
  fireEvent.click(await view.findByText('Minimal'));
  await waitFor(() => expect(toast).toHaveBeenCalledWith('Model compatibility changed. Please choose again.'));
  if (compact) expect(view.getByRole('button', { name: 'Agent setup: Companion' })).toBeTruthy();
  else expect(view.container.querySelector('.arco-select-view')?.textContent).toContain('Companion');
  expect(view.baseElement.textContent).not.toContain('private raw protocol envelope');
});
