/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { SWRConfig, unstable_serialize } from 'swr';
import type { SaveProviderModelRequest } from '@/common/types/provider/providerModel';
import { toProviderModelInput } from '@/common/utils/providerModels';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import {
  scenarioConnections, scenarioManifests, scenarioModelId, scenarioProvider, scenarioProviderBaseUrl, scenarioProviderId,
} from '../../../../test/fixtures/scenarioModelEditing';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const localFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = localFetch; window.fetch = localFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { default: ModelHubPage } = await import('./index');
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
  return <output data-testid='model-hub-route'>{location.pathname}{location.search}</output>;
}

const effectiveRequest = (request: SaveProviderModelRequest) => JSON.parse(JSON.stringify({
  ...request,
  model: { ...request.model, capabilities: request.model.capabilities.map((capability) => ({
    ...capability, traits: capability.traits ?? [],
    allow_cross_origin_credentials: capability.allow_cross_origin_credentials ?? false,
  })) },
}));

function mount(section: 'chat' | 'models' = 'chat') {
  const list = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([scenarioProvider]);
  const changed = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockReturnValue(() => {});
  const reconnected = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const connections = spyOn(ipcBridge.providerConnection.list, 'invoke').mockResolvedValue(scenarioConnections);
  const manifests = spyOn(ipcBridge.modelProtocol.list, 'invoke').mockImplementation(async ({ task }) => scenarioManifests[task]!);
  const save = spyOn(ipcBridge.providerModel.save, 'invoke').mockResolvedValue(scenarioProvider.models[0]);
  restorers.push(() => list.mockRestore(), () => changed.mockRestore(), () => reconnected.mockRestore(),
    () => connections.mockRestore(), () => manifests.mockRestore(), () => save.mockRestore());
  const fallback: Record<string, unknown> = {
    providers: [scenarioProvider],
    [`provider-connections:${scenarioProviderId}`]: scenarioConnections,
  };
  const allTasks = scenarioProvider.models[0].capabilities.map(({ task }) => task);
  for (const tasks of [allTasks, ['chat']]) {
    const requestKey = JSON.stringify(['preview', scenarioProviderBaseUrl, tasks, scenarioModelId]);
    fallback[unstable_serialize(['model-protocol-manifests', requestKey])] = {
      requestKey, manifests: scenarioManifests, errorTasks: [],
    };
  }
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={[`/models?section=${section}`]}>
    <SWRConfig value={{ provider: () => new Map(), fallback, revalidateOnMount: false }}>
      <ThemeProvider><LocationProbe /><ModelHubPage /></ThemeProvider>
    </SWRConfig>
  </MemoryRouter></I18nextProvider>);
  return { page, save };
}

test('model hub keeps a model draft open during sidebar clicks and Escape, then enables navigation after back', async () => {
  const { page, save } = mount();
  const chatTab = page.getByRole('tab', { name: zhSettings.modelHub.sectionChat });
  const asrTab = page.getByRole('tab', { name: zhSettings.modelHub.sectionAsr });
  fireEvent.click(page.getByRole('button', { name: zhSettings.modelHub.modality.editModelConfiguration }));
  const alias = await page.findByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement;
  await waitFor(() => expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.change(alias, { target: { value: '保持编辑的别名' } });

  expect(chatTab.getAttribute('aria-disabled')).toBe('false');
  for (const tab of page.getAllByRole('tab').filter((tab) => tab.id.startsWith('model-hub-tab-') && tab !== chatTab)) {
    expect(tab.getAttribute('aria-disabled')).toBe('true');
  }
  fireEvent.click(asrTab);
  fireEvent.keyDown(asrTab, { key: 'Enter', code: 'Enter' });
  fireEvent.keyDown(chatTab, { key: 'ArrowDown', code: 'ArrowDown' });
  fireEvent.keyDown(alias, { key: 'Escape', code: 'Escape' });
  fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });

  expect(page.getByTestId('model-hub-route').textContent).toBe('/models?section=chat');
  expect(chatTab.getAttribute('aria-selected')).toBe('true');
  expect(page.getByLabelText(zhSettings.modelDisplayNameTitle)).toBe(alias);
  expect(alias.value).toBe('保持编辑的别名');
  expect(save).toHaveBeenCalledTimes(0);
  expect(page.queryByRole('dialog')).toBeNull();

  fireEvent.click(page.getByRole('button', { name: zhSettings.modelConfiguration.back }));
  expect(page.queryByLabelText(zhSettings.modelDisplayNameTitle)).toBeNull();
  expect(page.getByText('多用途模型')).toBeTruthy();
  expect(asrTab.getAttribute('aria-disabled')).toBe('false');
  fireEvent.click(asrTab);
  expect(page.getByTestId('model-hub-route').textContent).toBe('/models?section=asr');
  expect(asrTab.getAttribute('aria-selected')).toBe('true');
  expect(page.getByRole('heading', { name: zhSettings.modelHub.speech.asrTitle })).toBeTruthy();
});

test('provider model metadata uses the shared page and saves alias and description with every existing capability', async () => {
  const { page, save } = mount('models');
  fireEvent.click(page.getByText(scenarioProvider.name));
  await page.findByText('多用途模型');
  expect(page.queryByPlaceholderText(zhSettings.modelDisplayNamePlaceholder)).toBeNull();
  expect(page.queryByPlaceholderText(zhSettings.modelDescriptionPlaceholder)).toBeNull();

  fireEvent.click(page.getByRole('button', { name: zhSettings.editModelCapabilities }));
  const alias = await page.findByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement;
  const description = page.getByLabelText(zhSettings.modelDescriptionTitle) as HTMLTextAreaElement;
  const editor = page.baseElement.querySelector('[data-model-configuration-page]');
  expect(alias.value).toBe('多用途模型');
  expect(description.value).toBe('保留的模型描述');
  expect(editor?.contains(alias)).toBe(true);
  expect(editor?.contains(description)).toBe(true);
  expect(page.getAllByPlaceholderText(zhSettings.modelDisplayNamePlaceholder)).toHaveLength(1);
  expect(page.getAllByPlaceholderText(zhSettings.modelDescriptionPlaceholder)).toHaveLength(1);
  expect(page.baseElement.querySelector('.arco-popover input, .arco-popover textarea')).toBeNull();
  expect(page.queryByRole('dialog')).toBeNull();

  fireEvent.change(alias, { target: { value: '  统一页面新别名  ' } });
  fireEvent.change(description, { target: { value: '  统一页面新描述  ' } });
  await waitFor(() => expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(page.getByRole('button', { name: zhCommon.save }));

  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(effectiveRequest(save.mock.calls[0][0])).toEqual(effectiveRequest({
    provider_id: scenarioProviderId,
    model: {
      ...toProviderModelInput(scenarioProvider.models[0]),
      display_name: '统一页面新别名',
      description: '统一页面新描述',
    },
  }));
  await waitFor(() => expect(page.queryByLabelText(zhSettings.modelDisplayNameTitle)).toBeNull());
  expect(page.getByTestId('model-hub-route').textContent).toBe('/models?section=models');
});
