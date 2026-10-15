import '../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useMemo, useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasProtocolManifest, aliasProviderBaseUrl } from '../../../../../test/fixtures/modelAliasEditor';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import { capabilityInputsFromDefinition, emptyCapabilityDraft, validateModelDefinition, type ModelDefinitionDraft } from './providerModelAdvanced';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings } } },
  interpolation: { escapeValue: false },
});
afterEach(cleanup);
const manifests = { chat: { ...aliasProtocolManifest, protocols: [...aliasProtocolManifest.protocols, {
  ...aliasProtocolManifest.protocols[0], protocol_id: 'openai.responses',
}] } };

function Harness({ raw, protocol = 'openai.chat_text' }: { raw: string; protocol?: string }) {
  const [value, setValue] = useState<ModelDefinitionDraft>({ model: 'arbitrary/future-model', capabilities: [{
    ...emptyCapabilityDraft('chat'), transportSource: 'persisted', protocol,
    providerParamsJson: raw, outputLimit: 16_384,
  }] });
  const validation = useMemo(() => validateModelDefinition(value, manifests, aliasProviderBaseUrl), [value]);
  return <I18nextProvider i18n={i18n}>
    <ModelDefinitionEditor value={value} onChange={setValue} modelReadOnly
      providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' manifests={manifests}
      validationErrors={validation.errors} />
    <button data-testid='save' disabled={!validation.valid}>Save</button>
    <output data-testid='serialized'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
  </I18nextProvider>;
}

describe('Chat compatibility controls', () => {
  test('edits max output and stream settings through the form while preserving authored parameters', async () => {
    const page = render(<Harness raw='{"temperature":0.4,"reasoning_effort":"high","future":{"enabled":true}}' />);
    fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-capability-disclosure="chat"]')!);
    fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-call-config-tab="protocol"]')!);
    const paramsDisclosure = page.container.querySelector<HTMLButtonElement>('[data-provider-params-disclosure="chat"]')!;
    if (paramsDisclosure.getAttribute('aria-expanded') !== 'true') fireEvent.click(paramsDisclosure);
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.modelAdvanced.maxTokensField }));
    fireEvent.click(await page.findByRole('option', { name: 'max_completion_tokens' }));
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.modelAdvanced.streamUsage }));
    fireEvent.click(await page.findByRole('option', { name: zhSettings.modelAdvanced.streamUsageOmit }));
    let saved = JSON.parse(page.getByTestId('serialized').textContent!)[0];
    expect(saved).toMatchObject({ output_limit: 16_384, protocol: 'openai.chat_text',
      provider_params: { temperature: 0.4, reasoning_effort: 'high', future: { enabled: true },
        max_tokens_field: 'max_completion_tokens', stream_options: null } });
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.modelAdvanced.maxTokensField }));
    fireEvent.click(await page.findByRole('option', { name: zhSettings.modelAdvanced.protocolDefault }));
    saved = JSON.parse(page.getByTestId('serialized').textContent!)[0];
    expect(saved.provider_params).not.toHaveProperty('max_tokens_field');
    expect(saved.provider_params.stream_options).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelAdvanced.cancelAdjustment }));
    saved = JSON.parse(page.getByTestId('serialized').textContent!)[0];
    expect(saved.provider_params).toEqual({ temperature: 0.4, reasoning_effort: 'high', future: { enabled: true } });
    expect(saved.output_limit).toBe(16_384);
  });

  for (const scenario of [
    { protocol: 'openai.chat_text', target: 'openai.responses', parameter: 'stream_options', value: null },
    { protocol: 'openai.responses', target: 'openai.chat_text', parameter: 'chain_rounds', value: true },
  ]) test(`switching ${scenario.protocol} to ${scenario.target} preserves ${scenario.parameter} and blocks save with a named correction`, async () => {
    const raw = JSON.stringify({ [scenario.parameter]: scenario.value, future: { mode: 'fast' } });
    const page = render(<Harness raw={raw} protocol={scenario.protocol} />);
    fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-capability-disclosure="chat"]')!);
    fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-call-config-tab="protocol"]')!);
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.modelAdvanced.protocol }));
    fireEvent.click(await page.findByRole('option', { name: new RegExp(scenario.target.replace('.', '\\.')) }));
    const saved = JSON.parse(page.getByTestId('serialized').textContent!)[0];
    expect(saved.protocol).toBe(scenario.target);
    expect(saved.provider_params).toEqual({ [scenario.parameter]: scenario.value, future: { mode: 'fast' } });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    const error = page.container.querySelector<HTMLElement>('[data-capability-error="provider_param_scope"]')!;
    expect(error.textContent).toContain(scenario.parameter);
    expect(error.textContent).toContain('切换已保留原值');
    const textarea = page.container.querySelector<HTMLTextAreaElement>('[data-provider-params-json] textarea')!;
    expect(textarea.value).toBe(raw);
    fireEvent.change(textarea, { target: { value: '{"future":{"mode":"fast"}}' } });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
  });
});
