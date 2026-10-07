import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { uuidv7 } from '@/common/utils';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { parseProviderId, parseExecutionTemplateId } from '@/common/types/ids';
import type { TProviderWithModel } from '@/common/config/storage';
import * as catalog from '@/renderer/pages/conversation/execution/useExecutionModelPool';
import { useGuidCollaboration } from './useGuidCollaboration';

const provider = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
const main = { provider_id: provider, model: 'main' };
const helper = { provider_id: provider, model: 'helper' };
const model = { id: provider, use_model: 'main' } as TProviderWithModel;
const templateId = parseExecutionTemplateId('0190f5fe-7c00-7a00-8000-000000000106');
let spy: ReturnType<typeof spyOn<typeof catalog, 'useExecutionModelPool'>>;
let available = [main, helper];
let configured = [main, helper];
let loading = false;
beforeEach(() => {
  setBrowserStorageGeneration(uuidv7());
  available = [main, helper]; configured = [main, helper]; loading = false;
  spy = spyOn(catalog, 'useExecutionModelPool').mockImplementation(() => ({
    providers: [], getAvailableModels: () => [], formatModelLabel: (_provider, name) => name ?? '',
    isLoading: loading, configuredPairs: configured, allPairs: available, hasModels: true, buildModelPool: () => null,
  }));
});
afterEach(() => { cleanup(); spy.mockRestore(); sessionStorage.clear(); });

test('pins the lead once, retains collaboration settings across navigation and resets an explicit new draft', () => {
  const hook = renderHook(() => useGuidCollaboration(model));
  act(() => {
    hook.result.current.setCollaborators([main, helper, helper]);
    hook.result.current.setPolicy({ delegationPolicy: 'prefer_parallel', decisionPolicy: 'ask_user' });
    hook.result.current.setTemplate({ execution_template_id: templateId, name: '审阅', participantCount: 2, models: [main, helper] });
  });
  expect(hook.result.current.config).toEqual({ execution_model_pool: { mode: 'range', models: [main, helper] }, execution_template_id: templateId, delegation_policy: 'prefer_parallel', decision_policy: 'ask_user' });
  hook.unmount();
  const restored = renderHook(() => useGuidCollaboration(model));
  expect(restored.result.current.activeCollaborators).toEqual([helper]);
  expect(restored.result.current.policy.delegationPolicy).toBe('prefer_parallel');
  act(() => restored.result.current.reset());
  expect(restored.result.current.config).toEqual({ execution_model_pool: { mode: 'single', model: main }, execution_template_id: null, delegation_policy: 'automatic', decision_policy: 'automatic' });
});

test('loading never deletes selections; disabled models can return, removed models cannot', () => {
  const hook = renderHook(() => useGuidCollaboration(model));
  act(() => hook.result.current.setCollaborators([helper]));
  loading = true; available = []; configured = [];
  hook.rerender();
  expect(hook.result.current.ready).toBe(false);
  loading = false; configured = [main, helper]; available = [main];
  hook.rerender();
  expect(hook.result.current.activeCollaborators).toEqual([]);
  available = [main, helper]; hook.rerender();
  expect(hook.result.current.activeCollaborators).toEqual([helper]);
  configured = [main]; available = [main]; hook.rerender();
  configured = [main, helper]; available = [main, helper]; hook.rerender();
  expect(hook.result.current.activeCollaborators).toEqual([]);
});

test('changing the lead clears an incompatible collaboration plan', () => {
  const hook = renderHook(({ current }) => useGuidCollaboration(current), { initialProps: { current: model } });
  act(() => hook.result.current.setTemplate({ execution_template_id: templateId, name: '审阅', participantCount: 1, models: [main] }));
  hook.rerender({ current: { ...model, use_model: 'helper' } });
  expect(hook.result.current.selectedTemplate).toBeNull();
  expect(hook.result.current.config?.execution_template_id).toBeNull();
  expect(hook.result.current.config?.execution_model_pool).toEqual({ mode: 'single', model: helper });
});
