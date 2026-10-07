/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { SWRConfig, unstable_serialize } from 'swr';
import type { IProvider } from '@/common/config/storage';
import type { SaveProviderModelRequest } from '@/common/types/provider/providerModel';
import { toProviderModelInput } from '@/common/utils/providerModels';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import {
  scenarioConnections,
  scenarioManifests,
  scenarioModelId,
  scenarioProvider,
  scenarioProviderBaseUrl,
  scenarioProviderId,
} from '../../../../test/fixtures/scenarioModelEditing';
import { MODALITY_SPECS, type ModalityKey } from './modalityModels';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
// Incidental application initialization stays on a local credential-free
// transport; model saves/list refreshes are observed at the actual IPC bridge.
const localFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = localFetch; window.fetch = localFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { default: ModalityModelsPanel } = await import('./ModalityModelsPanel');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
restoreTransport();

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings, common: zhCommon } } },
  interpolation: { escapeValue: false },
});

const restorers: Array<() => void> = [];
beforeEach(installTransport);
afterEach(() => {
  cleanup();
  for (const restore of restorers.splice(0).reverse()) restore();
  restoreTransport();
});

function LocationProbe() {
  const location = useLocation();
  return <output data-testid='model-scene-route'>{location.pathname}{location.search}</output>;
}

function mount(modality: ModalityKey = 'chat', failFirstSave = false, limitedTaskFixture = false) {
  let provider: IProvider = structuredClone(scenarioProvider);
  if (limitedTaskFixture) {
    const task = MODALITY_SPECS[modality].task;
    provider.models[0].capabilities = provider.models[0].capabilities.filter((capability) =>
      capability.task === task || capability.task === (task === 'chat' ? 'speech_recognition' : 'chat'));
  }
  let notifyProviderChange: (() => void) | undefined;
  const saved: SaveProviderModelRequest[] = [];
  const list = spyOn(ipcBridge.mode.listProviders, 'invoke').mockImplementation(async () => [structuredClone(provider)]);
  const save = spyOn(ipcBridge.providerModel.save, 'invoke').mockImplementation(async (request) => {
    saved.push(structuredClone(request));
    if (failFirstSave && saved.length === 1) throw new Error('fixture save failure');
    const original = provider.models[0];
    provider = {
      ...provider,
      models: [{
        ...original,
        ...request.model,
        enabled: request.model.enabled ?? original.enabled,
        sort_order: request.model.sort_order ?? original.sort_order,
        capabilities: request.model.capabilities.map((capability) => ({
          ...capability,
          traits: capability.traits ?? [],
          allow_cross_origin_credentials: capability.allow_cross_origin_credentials ?? false,
          provider_params: capability.provider_params ?? {},
          created_at: 11,
          updated_at: 20,
        })),
        updated_at: 20,
      }],
    };
    return structuredClone(provider.models[0]);
  });
  const changed = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation((callback) => {
    notifyProviderChange = () => callback({ provider_id: scenarioProviderId });
    return () => {};
  });
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const connections = spyOn(ipcBridge.providerConnection.list, 'invoke').mockResolvedValue(scenarioConnections);
  const manifests = spyOn(ipcBridge.modelProtocol.list, 'invoke').mockImplementation(async ({ task }) => scenarioManifests[task]!);
  restorers.push(() => list.mockRestore(), () => save.mockRestore(), () => changed.mockRestore(),
    () => reconnect.mockRestore(), () => connections.mockRestore(), () => manifests.mockRestore());

  const fallback: Record<string, unknown> = {
    providers: [provider],
    [`provider-connections:${scenarioProviderId}`]: scenarioConnections,
  };
  // Supply both aggregate and task-focused manifest keys. All editor calls
  // remain real; any additional manifest request resolves through the spy.
  const taskSets = [provider.models[0].capabilities.map(({ task }) => task),
    ...provider.models[0].capabilities.map(({ task }) => [task])];
  for (const tasks of taskSets) {
    const requestKey = JSON.stringify(['preview', scenarioProviderBaseUrl, tasks]);
    fallback[unstable_serialize(['model-protocol-manifests', requestKey])] = {
      requestKey,
      manifests: Object.fromEntries(tasks.map((task) => [task, scenarioManifests[task]])),
      errorTasks: [],
    };
  }

  const route = `/models?section=${modality}`;
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={[route]}>
    <SWRConfig value={{ provider: () => new Map(), fallback, revalidateOnMount: false, dedupingInterval: 0 }}>
      <ThemeProvider><LocationProbe /><ModalityModelsPanel modality={modality}
        titleKey='settings.modelHub.title' subtitleKey='settings.modelHub.subtitle' /></ThemeProvider>
    </SWRConfig>
  </MemoryRouter></I18nextProvider>);

  const open = async () => {
    fireEvent.click(page.getByRole('button', { name: i18n.t('settings.modelHub.modality.editModelConfiguration') }));
    const input = await page.findByLabelText(zhSettings.modelId) as HTMLInputElement;
    await waitFor(() => expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(false));
    return input;
  };
  const chooseOutput = async (label: string) => {
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.outputLimit }));
    fireEvent.click(await page.findByRole('option', { name: label }));
  };
  const currentRoute = () => page.getByTestId('model-scene-route').textContent;
  const publishProviderUpdate = async (update: (current: IProvider) => IProvider) => {
    provider = update(structuredClone(provider));
    const requestsBefore = list.mock.calls.length;
    await act(async () => { notifyProviderChange?.(); });
    await waitFor(() => expect(list.mock.calls.length).toBe(requestsBefore + 1));
  };
  return { page, open, chooseOutput, saved, list, currentRoute, route, publishProviderUpdate };
}

const normalizeWire = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
const effectiveRequest = (request: SaveProviderModelRequest) => normalizeWire({
  ...request,
  model: { ...request.model, capabilities: request.model.capabilities.map((capability) => ({
    ...capability,
    traits: capability.traits ?? [],
    allow_cross_origin_credentials: capability.allow_cross_origin_credentials ?? false,
  })) },
});

describe('editing models from task-specific model management pages', () => {
  test.each(Object.entries(MODALITY_SPECS) as Array<[ModalityKey, (typeof MODALITY_SPECS)[ModalityKey]]>)(
    '%s opens the shared editor for the current task without navigating to providers', async (modality, spec) => {
      const { page, open, saved, currentRoute, route } = mount(modality, false, true);
      expect(page.getByText('多用途模型')).toBeTruthy();
      const id = await open();
      expect(id.value).toBe(scenarioModelId);
      expect(id.readOnly).toBe(true);
      expect(currentRoute()).toBe(route);
      const cards = [...page.baseElement.querySelectorAll<HTMLElement>('[data-capability-card]')]
        .filter((card) => !card.hidden);
      expect(cards.map((card) => card.dataset.capabilityCard)).toEqual([spec.task]);
      expect(page.baseElement.querySelector('[data-add-call-route]')).toBeNull();
      expect(page.baseElement.querySelector('[data-remove-model-task]')).toBeNull();
      fireEvent.click(page.getByRole('button', { name: zhCommon.cancel }));
      expect(saved).toHaveLength(0);
      expect(currentRoute()).toBe(route);
    },
  );

  test('saving alias, description, output, and task endpoint refreshes the current page while retaining all other tasks and model metadata', async () => {
    const { page, open, chooseOutput, saved, list, currentRoute, route } = mount();
    await open();
    fireEvent.change(page.getByLabelText(zhSettings.modelDisplayNameTitle), { target: { value: '  场景新别名  ' } });
    fireEvent.change(page.getByLabelText(zhSettings.modelDescriptionTitle), { target: { value: '  场景新描述  ' } });
    await chooseOutput('8,192 tokens');

    const disclosure = page.baseElement.querySelector<HTMLButtonElement>('[data-capability-disclosure="chat"]');
    expect(disclosure).not.toBeNull();
    fireEvent.click(disclosure!);
    fireEvent.click(page.getByRole('tab', { name: '兼容特殊调用方式' }));
    const addressDisclosure = page.baseElement.querySelector<HTMLButtonElement>('[data-protocol-transport-disclosure="chat"]');
    expect(addressDisclosure).not.toBeNull();
    fireEvent.click(addressDisclosure!);
    const endpoint = page.baseElement.querySelector<HTMLInputElement>('[data-endpoint-field="endpoint"]');
    expect(endpoint).not.toBeNull();
    fireEvent.change(endpoint!, { target: { value: '/chat/scenario-completions' } });
    fireEvent.click(page.getByRole('button', { name: '应用到对话' }));
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));

    await waitFor(() => expect(saved).toHaveLength(1));
    await waitFor(() => expect(page.getByText('场景新别名')).toBeTruthy());
    expect(list).toHaveBeenCalledTimes(1);
    expect(currentRoute()).toBe(route);
    const original = toProviderModelInput(scenarioProvider.models[0]);
    expect(effectiveRequest(saved[0])).toEqual(effectiveRequest({
      provider_id: scenarioProviderId,
      model: {
        ...original,
        display_name: '场景新别名',
        description: '场景新描述',
        capabilities: original.capabilities.map((capability) => capability.task === 'chat'
          ? { ...capability, output_limit: 8_192, endpoint: '/chat/scenario-completions' }
          : capability),
      },
    }));

    await open();
    expect((page.getByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement).value).toBe('场景新别名');
    expect(page.getByRole('combobox', { name: zhSettings.outputLimit }).textContent).toContain('8,192 tokens');
  });

  test('clearing alias and description omits both from the full save and reopens empty fields', async () => {
    const { page, open, saved, list } = mount();
    await open();
    fireEvent.change(page.getByLabelText(zhSettings.modelDisplayNameTitle), { target: { value: '' } });
    fireEvent.change(page.getByLabelText(zhSettings.modelDescriptionTitle), { target: { value: '' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));

    await waitFor(() => expect(saved).toHaveLength(1));
    await waitFor(() => expect(page.queryByLabelText(zhSettings.modelDisplayNameTitle)).toBeNull());
    expect(saved[0].model.display_name).toBeUndefined();
    expect(saved[0].model.description).toBeUndefined();
    expect(normalizeWire(saved[0].model)).not.toHaveProperty('display_name');
    expect(normalizeWire(saved[0].model)).not.toHaveProperty('description');
    expect(list).toHaveBeenCalledTimes(1);
    expect(page.queryByText('多用途模型')).toBeNull();
    expect(page.queryByText('保留的模型描述')).toBeNull();

    await open();
    expect((page.getByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement).value).toBe('');
    expect((page.getByLabelText(zhSettings.modelDescriptionTitle) as HTMLTextAreaElement).value).toBe('');
  });

  test('canceling a scene edit discards the draft without writing or refreshing providers', async () => {
    const { page, open, chooseOutput, saved, list, currentRoute, route } = mount('vision');
    await open();
    fireEvent.change(page.getByLabelText(zhSettings.modelDisplayNameTitle), { target: { value: '丢弃的别名' } });
    await chooseOutput('16,384 tokens');
    fireEvent.click(page.getByRole('button', { name: zhCommon.cancel }));
    expect(saved).toHaveLength(0);
    expect(list).toHaveBeenCalledTimes(0);
    expect(currentRoute()).toBe(route);
    await open();
    expect((page.getByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement).value).toBe('多用途模型');
    expect(page.getByRole('combobox', { name: zhSettings.outputLimit }).textContent).toContain('4,096 tokens');
  });

  test('a failed save retains the scene draft and can be retried without losing other task configuration', async () => {
    const { page, open, chooseOutput, saved, list, currentRoute, route } = mount('chat', true);
    await open();
    fireEvent.change(page.getByLabelText(zhSettings.modelDisplayNameTitle), { target: { value: '等待重试的别名' } });
    await chooseOutput('32,768 tokens');
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    await waitFor(() => expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(false));
    expect((page.getByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement).value).toBe('等待重试的别名');
    expect(page.getByRole('combobox', { name: zhSettings.outputLimit }).textContent).toContain('32,768 tokens');
    expect(list).toHaveBeenCalledTimes(0);
    expect(currentRoute()).toBe(route);

    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(2));
    await waitFor(() => expect(page.getByText('等待重试的别名')).toBeTruthy());
    expect(normalizeWire(saved[1])).toEqual(normalizeWire(saved[0]));
    expect(list).toHaveBeenCalledTimes(1);
  });

  test('a scene save preserves another task and model metadata updated while the editor was open', async () => {
    const { page, open, chooseOutput, saved, publishProviderUpdate } = mount();
    await open();
    await chooseOutput('8,192 tokens');
    await publishProviderUpdate((current) => ({
      ...current,
      models: [{
        ...current.models[0],
        enabled: true,
        sort_order: 42,
        description: '同时更新的描述',
        capabilities: current.models[0].capabilities.map((capability) => capability.task === 'speech_recognition'
          ? { ...capability, endpoint: '/audio/updated-transcriptions', provider_params: { language: 'zh' } }
          : capability),
      }],
    }));
    await waitFor(() => expect(page.getByText('同时更新的描述')).toBeTruthy());
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].model.enabled).toBe(true);
    expect(saved[0].model.sort_order).toBe(42);
    expect(saved[0].model.description).toBe('同时更新的描述');
    expect(saved[0].model.capabilities.find(({ task }) => task === 'speech_recognition')).toMatchObject({
      connection_role: 'speech', endpoint: '/audio/updated-transcriptions', provider_params: { language: 'zh' },
    });
    expect(saved[0].model.capabilities.find(({ task }) => task === 'chat')?.output_limit).toBe(8_192);
  });

  test('a task updated elsewhere is not silently overwritten by a scene draft', async () => {
    const { page, open, chooseOutput, saved, publishProviderUpdate } = mount();
    await open();
    await chooseOutput('8,192 tokens');
    await publishProviderUpdate((current) => ({
      ...current,
      models: [{ ...current.models[0], capabilities: current.models[0].capabilities.map((capability) => capability.task === 'chat'
        ? { ...capability, output_limit: 16_384 }
        : capability) }],
    }));
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await page.findByText(i18n.t('settings.modelAdvanced.scopedTaskChanged'));
    expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(true);
    expect(saved).toHaveLength(0);
    expect(page.getByRole('combobox', { name: zhSettings.outputLimit }).textContent).toContain('8,192 tokens');
  });
});
