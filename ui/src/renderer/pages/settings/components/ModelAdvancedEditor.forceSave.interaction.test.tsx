/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig, unstable_serialize } from 'swr';
import type { DiscoverModelProtocolsResponse } from '@/common/protocolBindings/DiscoverModelProtocolsResponse';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasCapability, aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';
import { capabilityInputFromResponse, type ProviderModelCapabilityInput } from './providerModelAdvanced';
import type { ModelAdvancedPatch } from './ModelAdvancedEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: { candidates: [], elapsed_ms: 1 } }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { default: ModelAdvancedEditor } = await import('./ModelAdvancedEditor');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
restoreTransport();

const restorers: Array<() => void> = [];
beforeEach(installTransport);
afterEach(() => { cleanup(); restorers.splice(0).reverse().forEach(restore => restore()); restoreTransport(); });

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings, common: zhCommon } } },
  interpolation: { escapeValue: false },
});

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
};
const currentInput = capabilityInputFromResponse(aliasCapability);
const result = (selected: ProviderModelCapabilityInput = currentInput): DiscoverModelProtocolsResponse => ({
  selected_protocol: selected.protocol, selected_capability: selected, elapsed_ms: 1,
  candidates: [{ protocol: selected.protocol, capability: selected, status: 'available', evidence: 'verified_stream', recommended: true, elapsed_ms: 1 }],
});

function mount(options: {
  capability?: ProviderModelCapabilityResponse;
  scoped?: boolean;
  persist?: (patch: ModelAdvancedPatch, attempt: number) => Promise<void>;
} = {}) {
  const saved: ModelAdvancedPatch[] = [];
  const closed: boolean[] = [];
  const requests: Array<{ response: ReturnType<typeof deferred<DiscoverModelProtocolsResponse>>; signal?: AbortSignal }> = [];
  const discover = spyOn(ipcBridge.mode.discoverProviderModelProtocols, 'invoke').mockImplementation((_request, signal) => {
    const response = deferred<DiscoverModelProtocolsResponse>();
    requests.push({ response, signal });
    // Deliberately ignore abort in this network fixture: the component must
    // reject a late successful result even if the transport cannot stop it.
    return response.promise;
  });
  restorers.push(() => discover.mockRestore());
  const model = 'immutable-model-id';
  const requestKey = JSON.stringify(['preview', aliasProviderBaseUrl, ['chat'], model]);
  const config = {
    provider: () => new Map(), revalidateOnMount: false,
    fallback: {
      [unstable_serialize(['model-protocol-manifests', requestKey])]: {
        requestKey, errorTasks: [], manifests: { chat: { ...aliasProtocolManifest, protocols: [
          ...aliasProtocolManifest.protocols,
          { ...aliasProtocolManifest.protocols[0], protocol_id: 'openai.responses', endpoints: [
            { ...aliasProtocolManifest.protocols[0].endpoints[0], default_value: '/responses' },
          ] },
        ] } },
      },
      [`provider-connections:${aliasProviderId}`]: [],
    },
  };
  let updateCapabilities!: (rows: ProviderModelCapabilityResponse[]) => void;
  function Harness() {
    const [rows, setRows] = useState([options.capability ?? aliasCapability]);
    updateCapabilities = setRows;
    return <ModelAdvancedEditor providerId={aliasProviderId} providerName='测试服务商' preset='preview'
      providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' model={model} displayName='Original alias'
      capabilities={rows} task={options.scoped ? 'chat' : undefined} onClose={() => closed.push(true)}
      onSave={async patch => {
        saved.push(patch);
        await options.persist?.(patch, saved.length);
      }} />;
  }
  const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={config}>
    <ThemeProvider><Harness /></ThemeProvider>
  </SWRConfig></I18nextProvider>);
  const saveButton = () => page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement;
  const forceButton = () => page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave }) as HTMLButtonElement;
  const open = async (requestCount = 1) => {
    fireEvent.click(page.getByRole('button', { name: zhSettings.editModelCapabilities }));
    await page.findByLabelText(zhSettings.modelId);
    await waitFor(() => expect(requests).toHaveLength(requestCount));
    expect(saveButton().disabled).toBe(true);
    expect(saveButton().getAttribute('aria-busy')).toBe('true');
  };
  return { page, saved, closed, requests, open, saveButton, forceButton, updateCapabilities: (rows: ProviderModelCapabilityResponse[]) => updateCapabilities(rows) };
}

test('force save persists the current snapshot and aborts pending discovery before late success', async () => {
  const persistence = deferred<void>();
  const { page, saved, closed, requests, open, saveButton, forceButton } = mount({ persist: () => persistence.promise });
  await open();
  fireEvent.change(page.getByLabelText(zhSettings.modelDisplayNameTitle), { target: { value: 'Current snapshot' } });
  fireEvent.click(saveButton());
  expect(saved).toHaveLength(0);
  fireEvent.click(forceButton());
  await waitFor(() => expect(saved).toHaveLength(1));
  expect(saved[0]).toEqual({ display_name: 'Current snapshot', capabilities: [currentInput] });
  expect(requests[0].signal?.aborted).toBe(true);
  expect(page.baseElement.querySelector('[data-model-protocol-discovery]')).toBeNull();
  expect(saveButton().disabled).toBe(true);

  await act(async () => requests[0].response.resolve(result({ ...currentInput, protocol: 'openai.responses', endpoint: '/responses', output_limit: 1234 })));
  expect((page.getByLabelText(zhSettings.modelDisplayNameTitle) as HTMLInputElement).value).toBe('Current snapshot');
  fireEvent.click(saveButton());
  expect(saved).toHaveLength(1);
  await act(async () => persistence.resolve());
  await waitFor(() => expect(closed).toEqual([true]));
});

test('ordinary save becomes available when automatic configuration finishes normally', async () => {
  const { page, saved, requests, open, saveButton } = mount();
  await open();
  await act(async () => requests[0].response.resolve(result()));
  await waitFor(() => expect(saveButton().disabled).toBe(false));
  expect(saveButton().getAttribute('aria-busy')).toBe('false');
  expect(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave })).toBeNull();
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved).toEqual([{ display_name: 'Original alias', capabilities: [currentInput] }]));
});

for (const [name, capability] of [
  ['invalid token limit', { ...aliasCapability, output_limit: 0 }],
  ['invalid provider JSON controls', { ...aliasCapability, provider_params: { max_tokens: 2048 } }],
] as const) {
  test(`force save cannot bypass ${name}`, async () => {
    const { saved, requests, open, saveButton, forceButton } = mount({ capability });
    await open();
    expect(forceButton().disabled).toBe(true);
    fireEvent.click(forceButton());
    fireEvent.click(saveButton());
    expect(saved).toHaveLength(0);
    expect(requests[0].signal?.aborted).toBe(false);
  });
}

test('force save keeps task concurrency checks and does not cancel discovery after a rejected snapshot', async () => {
  const { page, saved, requests, open, forceButton, updateCapabilities } = mount({ scoped: true });
  await open();
  await act(async () => updateCapabilities([{ ...aliasCapability, provider_params: { temperature: 0.9 } }]));
  fireEvent.click(forceButton());
  await page.findByText(zhSettings.modelAdvanced.scopedTaskChanged);
  expect(saved).toHaveLength(0);
  expect(forceButton().disabled).toBe(true);
  expect(requests[0].signal?.aborted).toBe(false);
});

test('failed force save keeps discovery skipped for retry and reopening starts a fresh check', async () => {
  const firstSave = deferred<void>();
  const { page, saved, closed, requests, open, saveButton, forceButton } = mount({ persist: (_patch, attempt) => attempt === 1 ? firstSave.promise : Promise.resolve() });
  await open();
  fireEvent.click(forceButton());
  await waitFor(() => expect(saved).toHaveLength(1));
  expect(requests[0].signal?.aborted).toBe(true);
  await act(async () => requests[0].response.resolve(result({ ...currentInput, protocol: 'openai.responses', endpoint: '/responses' })));
  await act(async () => firstSave.reject(new Error('local persistence fixture failure')));
  await waitFor(() => expect(saveButton().disabled).toBe(false));
  expect(closed).toHaveLength(0);
  expect(page.baseElement.querySelector('[data-model-protocol-discovery]')).toBeNull();
  expect(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave })).toBeNull();
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved).toHaveLength(2));
  expect(saved[1]).toEqual(saved[0]);
  await waitFor(() => expect(closed).toEqual([true]));
  await open(2);
  expect(requests[1].signal?.aborted).toBe(false);
  expect(forceButton().disabled).toBe(false);
});
