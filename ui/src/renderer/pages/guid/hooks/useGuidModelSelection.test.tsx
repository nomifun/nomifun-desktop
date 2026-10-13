import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import { configService } from '@/common/config/configService';
import { parseProviderId } from '@/common/types/ids';
import * as taskModels from '@/renderer/hooks/agent/useModelsForTask';
import { useGuidModelSelection, type GuidAgentModelDefaults } from './useGuidModelSelection';

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000201');
const provider: IProvider = {
  id: providerId, name: 'Configured provider', platform: 'custom', enabled: true,
  base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: true,
  models: ['global-model', 'agent-model', 'user-model', 'other-agent-model'].map(model => ({
    provider_id: providerId, model, enabled: true, sort_order: 0, created_at: 1, updated_at: 1,
    capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }],
  })),
};
const groups = [{ provider, models: provider.models.map(model => model.model) }];
const defaults = (model: string | null, selectionKey = 'agent-a:revision-1', ready = true): GuidAgentModelDefaults => ({
  selectionKey, ready, defaultModel: model ? { provider_id: providerId, model } : null,
});
const choice = (model: string) => ({ ...provider, use_model: model });

beforeEach(() => {
  configService.reset();
  configService.setLocal('nomi.defaultModel', { provider_id: providerId, model: 'global-model' });
  spyOn(configService, 'set').mockResolvedValue();
  spyOn(taskModels, 'useModelsForTask').mockReturnValue({ groups, isLoading: false, refresh: () => {} });
});
afterEach(() => { cleanup(); mock.restore(); configService.reset(); });

test('the selected Agent default becomes the displayed launch model without saving it as installation default', () => {
  const hook = renderHook(() => useGuidModelSelection('nomi', defaults('agent-model')));
  expect(hook.result.current.current_model?.use_model).toBe('agent-model');
  expect(configService.get('nomi.defaultModel')).toEqual({ provider_id: providerId, model: 'global-model' });
  expect(configService.set).not.toHaveBeenCalled();
});

test('an empty Agent default inherits the global model and keeps an explicit user choice', async () => {
  const hook = renderHook(({ scope }) => useGuidModelSelection('nomi', scope), { initialProps: { scope: defaults(null) } });
  await waitFor(() => expect(hook.result.current.current_model?.use_model).toBe('global-model'));
  await act(async () => { await hook.result.current.setCurrentModel(choice('user-model')); });
  hook.rerender({ scope: defaults('agent-model') });
  expect(hook.result.current.current_model?.use_model).toBe('user-model');
  expect(configService.set).toHaveBeenCalledTimes(1);
  expect(configService.set).toHaveBeenCalledWith('nomi.defaultModel', { provider_id: providerId, model: 'user-model' });
});

test('late defaults and delayed preference writes cannot overwrite the latest manual choice', async () => {
  const pendingWrites: (() => void)[] = [];
  spyOn(configService, 'set').mockImplementation(() => new Promise<void>(resolve => { pendingWrites.push(resolve); }));
  const hook = renderHook(({ scope }) => useGuidModelSelection('nomi', scope), { initialProps: { scope: defaults(null, 'agent-a:revision-1', false) } });
  expect(hook.result.current.current_model).toBeUndefined();
  let firstWrite!: Promise<void>;
  let secondWrite!: Promise<void>;
  act(() => { firstWrite = hook.result.current.setCurrentModel(choice('global-model')); });
  act(() => { secondWrite = hook.result.current.setCurrentModel(choice('user-model')); });
  hook.rerender({ scope: defaults('agent-model') });
  expect(hook.result.current.current_model?.use_model).toBe('user-model');
  await act(async () => { pendingWrites[1](); await secondWrite; pendingWrites[0](); await firstWrite; });
  expect(hook.result.current.current_model?.use_model).toBe('user-model');
});

test('another Agent or saved Revision waits for its own default and excludes the prior manual override', async () => {
  const hook = renderHook(({ scope }) => useGuidModelSelection('nomi', scope), { initialProps: { scope: defaults('agent-model') } });
  await act(async () => { await hook.result.current.setCurrentModel(choice('user-model')); });
  hook.rerender({ scope: defaults(null, 'agent-b:revision-1', false) });
  expect(hook.result.current.current_model).toBeUndefined();
  hook.rerender({ scope: defaults('other-agent-model', 'agent-b:revision-1') });
  expect(hook.result.current.current_model?.use_model).toBe('other-agent-model');
  await act(async () => { await hook.result.current.setCurrentModel(choice('user-model')); });
  hook.rerender({ scope: defaults(null, 'agent-b:revision-2', false) });
  expect(hook.result.current.current_model).toBeUndefined();
  hook.rerender({ scope: defaults('agent-model', 'agent-b:revision-2') });
  expect(hook.result.current.current_model?.use_model).toBe('agent-model');
  expect(configService.set).toHaveBeenCalledTimes(2);
});

test('an unavailable Agent default uses an enabled global fallback until the saved model becomes available', async () => {
  const modelGroups = spyOn(taskModels, 'useModelsForTask').mockReturnValue({
    groups: [{ provider, models: ['global-model'] }], isLoading: false, refresh: () => {},
  });
  const hook = renderHook(() => useGuidModelSelection('nomi', defaults('agent-model')));
  await waitFor(() => expect(hook.result.current.current_model?.use_model).toBe('global-model'));
  modelGroups.mockReturnValue({ groups, isLoading: false, refresh: () => {} });
  hook.rerender();
  expect(hook.result.current.current_model?.use_model).toBe('agent-model');
  expect(configService.set).not.toHaveBeenCalled();
});

test('a blank default with no saved installation model uses the first enabled Chat model locally', async () => {
  configService.reset();
  const hook = renderHook(() => useGuidModelSelection('nomi', defaults(null)));
  await waitFor(() => expect(hook.result.current.current_model?.use_model).toBe('global-model'));
  expect(configService.set).not.toHaveBeenCalled();
});
