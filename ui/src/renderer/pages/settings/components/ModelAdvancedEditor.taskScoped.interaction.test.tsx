/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig, unstable_serialize } from 'swr';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import {
  aliasCapability,
  aliasProtocolManifest,
  aliasProviderBaseUrl,
  aliasProviderId,
} from '../../../../../test/fixtures/modelAliasEditor';
import { capabilityInputFromResponse, type ModelProtocolManifest } from './providerModelAdvanced';
import type { ModelAdvancedPatch } from './ModelAdvancedEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
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

const audioCapabilities: ProviderModelCapabilityResponse[] = [
  {
    task: 'speech_recognition', traits: [], protocol: 'openai.audio_transcriptions', connection_role: 'default',
    endpoint: '/audio/transcriptions', allow_cross_origin_credentials: false, provider_params: { language: 'zh' }, created_at: 1, updated_at: 1,
  },
  {
    task: 'speech_synthesis', traits: [], protocol: 'openai.audio_speech', connection_role: 'default',
    endpoint: '/audio/speech', allow_cross_origin_credentials: false, provider_params: { voice: 'alloy', speed: 1.2 }, created_at: 1, updated_at: 1,
  },
];
const capabilities = [aliasCapability, ...audioCapabilities];

function manifestFor(task: ModelTask): ModelProtocolManifest {
  const capability = capabilities.find((candidate) => candidate.task === task)!;
  return {
    ...aliasProtocolManifest,
    requested_task: task,
    tasks: [task],
    recommendation: { ...aliasProtocolManifest.recommendation!, protocol_id: capability.protocol },
    protocols: [{
      ...aliasProtocolManifest.protocols[0], protocol_id: capability.protocol, supported_tasks: [task],
      endpoints: [{ ...aliasProtocolManifest.protocols[0].endpoints[0], task, default_value: capability.endpoint! }],
    }],
  };
}

function mount(task: ModelTask, failSaves = 0, initialCapabilities = capabilities, description?: string | null) {
  const saved: ModelAdvancedPatch[] = [];
  const closed: boolean[] = [];
  const handled: string[] = [];
  const requestKey = JSON.stringify(['preview', aliasProviderBaseUrl, [task]]);
  const config = {
    provider: () => new Map(), revalidateOnMount: false,
    fallback: {
      [unstable_serialize(['model-protocol-manifests', requestKey])]: {
        requestKey, manifests: { [task]: manifestFor(task) }, errorTasks: [],
      },
      [`provider-connections:${aliasProviderId}`]: [],
    },
  };
  function Harness() {
    const [rows, setRows] = useState(initialCapabilities);
    return <ModelAdvancedEditor providerId={aliasProviderId} providerName='测试服务商' preset='preview'
      providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' model='same-model-id'
      displayName='共享别名' description={description} capabilities={rows} task={task} hideTrigger openRequest='scenario-edit-1'
      onOpenRequestHandled={(request) => handled.push(request)} onClose={() => closed.push(true)}
      onSave={async (patch) => {
        saved.push(patch);
        if (saved.length <= failSaves) throw new Error('local save failure');
        setRows(patch.capabilities.map((capability) => ({
          ...capability,
          traits: capability.traits ?? [],
          allow_cross_origin_credentials: capability.allow_cross_origin_credentials ?? false,
          provider_params: capability.provider_params ?? {},
          created_at: 1,
          updated_at: 2,
        })));
      }} />;
  }
  const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={config}><ThemeProvider><Harness /></ThemeProvider></SWRConfig></I18nextProvider>);
  const ready = async () => {
    await page.findByLabelText(zhSettings.modelId);
    await waitFor(() => expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(false));
  };
  const editParameters = async (parameters: object) => {
    fireEvent.click(page.baseElement.querySelector<HTMLButtonElement>(`[data-capability-disclosure="${task}"]`)!);
    fireEvent.click(page.baseElement.querySelector<HTMLButtonElement>(`[data-capability-details="${task}"] [data-call-config-tab="protocol"]`)!);
    const paramsDisclosure = page.baseElement.querySelector<HTMLButtonElement>(`[data-provider-params-disclosure="${task}"]`)!;
    if (paramsDisclosure.getAttribute('aria-expanded') !== 'true') fireEvent.click(paramsDisclosure);
    const input = page.baseElement.querySelector<HTMLTextAreaElement>(`[data-capability-details="${task}"] [data-provider-params-json] textarea`)!;
    fireEvent.change(input, { target: { value: JSON.stringify(parameters) } });
    fireEvent.click(page.getByRole('button', { name: `应用到${zhSettings.modelTask[task]}` }));
  };
  return { page, ready, editParameters, saved, closed, handled };
}

describe('scenario entry existing model editor', () => {
  for (const task of ['speech_recognition', 'speech_synthesis'] as const) {
    test(`${task} edits its own interface while retaining chat and the other audio interface`, async () => {
      const { page, ready, editParameters, saved, closed, handled } = mount(task);
      await ready();
      expect(handled).toEqual(['scenario-edit-1']);
      expect((page.getByLabelText(zhSettings.modelId) as HTMLInputElement).readOnly).toBe(true);
      const cards = page.baseElement.querySelectorAll('[data-capability-card]');
      expect(cards).toHaveLength(1);
      expect(cards[0].getAttribute('data-capability-card')).toBe(task);
      expect(page.queryByRole('button', { name: zhSettings.editModelCapabilities })).toBeNull();
      expect(page.queryByRole('button', { name: zhSettings.removeModelTask })).toBeNull();
      const params = task === 'speech_recognition' ? { language: 'en' } : { voice: 'nova', speed: 0.9 };
      await editParameters(params);
      fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
      await waitFor(() => expect(saved).toHaveLength(1));
      expect(saved[0].capabilities).toHaveLength(3);
      expect(saved[0].capabilities.find((capability) => capability.task === task)?.provider_params).toEqual(params);
      for (const capability of capabilities.filter((candidate) => candidate.task !== task)) {
        expect(saved[0].capabilities.find((candidate) => candidate.task === capability.task)).toEqual(capabilityInputFromResponse(capability));
      }
      expect(closed).toEqual([true]);
    });
  }

  test('cancel discards the scenario draft without saving and notifies the parent once', async () => {
    const { page, ready, saved, closed } = mount('speech_recognition');
    await ready();
    fireEvent.change(page.getByLabelText(zhSettings.modelDisplayNameTitle), { target: { value: '未保存别名' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.cancel }));
    expect(saved).toHaveLength(0);
    expect(closed).toEqual([true]);
  });

  test('failed persistence retains the scenario draft and only closes after a successful retry', async () => {
    const { page, ready, editParameters, saved, closed } = mount('speech_synthesis', 1);
    await ready();
    await editParameters({ voice: 'nova', speed: 0.9 });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(closed).toHaveLength(0);
    await waitFor(() => expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(2));
    expect(saved[1]).toEqual(saved[0]);
    expect(closed).toEqual([true]);
  });

  test('a removed scenario capability cannot be recreated by the scoped editor', async () => {
    const { page, saved } = mount('speech_recognition', 0, [aliasCapability]);
    await page.findByLabelText(zhSettings.modelId);
    expect(page.baseElement.querySelectorAll('[data-capability-card]')).toHaveLength(0);
    expect((page.getByRole('button', { name: zhCommon.save }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    expect(saved).toHaveLength(0);
  });

  test('scenario description shares the model save while the provider form can still omit it', async () => {
    const { page, ready, saved } = mount('speech_recognition', 0, capabilities, '原有描述');
    await ready();
    const input = page.getByLabelText(zhSettings.modelDescriptionTitle) as HTMLTextAreaElement;
    expect(input.value).toBe('原有描述');
    fireEvent.change(input, { target: { value: '  新描述  ' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].description).toBe('新描述');
    expect(saved[0].display_name).toBe('共享别名');
  });

  test('clearing a scenario description saves explicit removal', async () => {
    const { page, ready, saved } = mount('speech_synthesis', 0, capabilities, '原有描述');
    await ready();
    fireEvent.change(page.getByLabelText(zhSettings.modelDescriptionTitle), { target: { value: '' } });
    fireEvent.click(page.getByRole('button', { name: zhCommon.save }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].description).toBeNull();
  });
});
