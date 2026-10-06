/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig, unstable_serialize } from 'swr';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import {
  aliasCapability,
  aliasProtocolManifest,
  aliasProviderBaseUrl,
  aliasProviderId,
} from '../../../../../test/fixtures/modelAliasEditor';
import type { ModelAdvancedPatch } from './ModelAdvancedEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
// Hooks consume isolated SWR fixtures; incidental app initialization also uses
// this local response rather than an account, credential, or remote provider.
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { default: ModelAdvancedEditor } = await import('./ModelAdvancedEditor');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
restoreTransport();
beforeEach(installTransport);
afterEach(() => { cleanup(); restoreTransport(); });

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings, common: zhCommon } } },
  interpolation: { escapeValue: false },
});
const requestKey = JSON.stringify(['preview', aliasProviderBaseUrl, ['chat']]);

function mount(initialOutput?: number) {
  const saved: ModelAdvancedPatch[] = [];
  const config = {
    provider: () => new Map(),
    revalidateOnMount: false,
    fallback: {
      [unstable_serialize(['model-protocol-manifests', requestKey])]: {
        requestKey,
        manifests: { chat: aliasProtocolManifest },
        errorTasks: [],
      },
      [`provider-connections:${aliasProviderId}`]: [],
    },
  };
  function Harness() {
    const [capabilities, setCapabilities] = useState<ProviderModelCapabilityResponse[]>([{
      ...aliasCapability,
      ...(initialOutput === undefined ? {} : { output_limit: initialOutput }),
      provider_params: { temperature: 0.4, reasoning_effort: 'medium' },
    }]);
    return <ModelAdvancedEditor providerId={aliasProviderId} providerName='测试服务商' preset='preview'
      providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' model='immutable-model-id'
      capabilities={capabilities} onSave={async (patch) => {
        saved.push(patch);
        // Reconstruct from the saved patch so an omitted output_limit remains
        // removed when reopening the actual existing-model modal.
        setCapabilities(patch.capabilities.map((capability) => ({
          ...aliasCapability,
          ...capability,
        })));
      }} />;
  }
  const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={config}>
    <ThemeProvider><Harness /></ThemeProvider>
  </SWRConfig></I18nextProvider>);
  const outputSelect = () => page.getByRole('combobox', { name: zhSettings.outputLimit });
  const open = async () => {
    fireEvent.click(page.getByRole('button', { name: zhSettings.editModelCapabilities }));
    return await page.findByRole('combobox', { name: zhSettings.outputLimit });
  };
  const chooseOutput = async (label: string) => {
    fireEvent.click(outputSelect());
    fireEvent.click(await page.findByRole('option', { name: label }));
  };
  const openReasoning = () => {
    const disclosure = page.baseElement.querySelector<HTMLButtonElement>('[data-capability-disclosure="chat"]');
    expect(disclosure).not.toBeNull();
    fireEvent.click(disclosure!);
    fireEvent.click(page.getByRole('tab', { name: '思考深度' }));
    const branch = page.baseElement.querySelector<HTMLElement>('[data-token-limits][data-call-config-branch="limits"]');
    expect(branch).not.toBeNull();
    expect(branch!.querySelector('[data-output-limit-input]')).toBeNull();
    return branch!;
  };
  const save = async () => {
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    return saved[0].capabilities[0];
  };
  return { page, saved, open, outputSelect, chooseOutput, openReasoning, save };
}

describe('model homepage maximum output editing', () => {
  test('sets the output preset beside context and persists it through the existing-model modal', async () => {
    const { page, open, chooseOutput, outputSelect, save } = mount();
    await open();
    const home = page.baseElement.querySelector<HTMLElement>('[data-model-context-settings="chat"]');
    expect(home).not.toBeNull();
    expect(home!.hidden).toBe(false);
    expect(home!.contains(outputSelect())).toBe(true);
    expect(home!.textContent).toContain('上下文窗口');
    expect(home!.textContent).toContain('上下文与输出');

    await chooseOutput('8,192 tokens');
    const capability = await save();
    expect(capability.output_limit).toBe(8_192);
    expect(capability.context_limit).toBe(64_000);
    expect(capability.provider_params).toEqual({ temperature: 0.4, reasoning_effort: 'medium' });
    expect(capability.protocol).toBe('openai.chat_text');
    expect((await open()).textContent).toContain('8,192 tokens');
  });

  test('choosing the provider default removes the saved output override and keeps that choice on reopen', async () => {
    const { open, chooseOutput, save } = mount(32_768);
    expect((await open()).textContent).toContain('32,768 tokens');
    await chooseOutput(zhSettings.outputLimitDefaultOption);
    const capability = await save();
    expect(capability).not.toHaveProperty('output_limit');
    expect(capability.context_limit).toBe(64_000);
    expect((await open()).textContent).toContain(zhSettings.outputLimitDefaultOption);
  });

  test('canceling an advanced reasoning adjustment preserves the output chosen on the homepage', async () => {
    const { page, open, chooseOutput, outputSelect, openReasoning, save } = mount(4_096);
    await open();
    await chooseOutput('16,384 tokens');
    const branch = openReasoning();
    fireEvent.click(branch.querySelector<HTMLButtonElement>('[data-reasoning-effort="low"]')!);
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelAdvanced.cancelAdjustment }));

    expect(outputSelect().textContent).toContain('16,384 tokens');
    const capability = await save();
    expect(capability.output_limit).toBe(16_384);
    expect(capability.provider_params).toEqual({ temperature: 0.4, reasoning_effort: 'medium' });
  });

  test('restoring advanced reasoning defaults and applying them leaves homepage output and context intact', async () => {
    const { page, open, chooseOutput, outputSelect, openReasoning, save } = mount(4_096);
    await open();
    await chooseOutput('32,768 tokens');
    const branch = openReasoning();
    fireEvent.click(branch.querySelector<HTMLButtonElement>('[data-reasoning-effort="high"]')!);
    fireEvent.click(within(branch).getByRole('button', { name: zhSettings.restoreProtocolDefault }));
    fireEvent.click(page.getByRole('button', { name: '应用到对话' }));

    expect(outputSelect().textContent).toContain('32,768 tokens');
    const capability = await save();
    expect(capability.output_limit).toBe(32_768);
    expect(capability.context_limit).toBe(64_000);
    expect(capability.provider_params).toEqual({ temperature: 0.4 });
  });
});
