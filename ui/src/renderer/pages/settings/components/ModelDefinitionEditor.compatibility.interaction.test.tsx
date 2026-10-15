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

function openProtocol(page: ReturnType<typeof render>) {
  const disclosure = page.container.querySelector<HTMLButtonElement>('[data-capability-disclosure="chat"]')!;
  if (disclosure.getAttribute('aria-expanded') !== 'true') fireEvent.click(disclosure);
  fireEvent.click(page.container.querySelector<HTMLButtonElement>('[data-call-config-tab="protocol"]')!);
}
async function chooseProtocol(page: ReturnType<typeof render>, protocol: string) {
  fireEvent.click(page.getByRole('combobox', { name: zhSettings.modelAdvanced.protocol }));
  fireEvent.click(await page.findByRole('option', { name: new RegExp(protocol.replace('.', '\\.')) }));
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
  ]) test(`switching ${scenario.protocol} suspends ${scenario.parameter}, permits save, and restores it when switching back`, async () => {
    const raw = JSON.stringify({ [scenario.parameter]: scenario.value, future: { mode: 'fast' } });
    const page = render(<Harness raw={raw} protocol={scenario.protocol} />);
    openProtocol(page);
    await chooseProtocol(page, scenario.target);
    const saved = JSON.parse(page.getByTestId('serialized').textContent!)[0];
    expect(saved.protocol).toBe(scenario.target);
    expect(saved.provider_params).toEqual({ future: { mode: 'fast' } });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
    expect(page.container.querySelector('[data-protocol-parameter-transition]')?.textContent).toContain('已暂存旧协议设置');
    expect(page.container.querySelector('[data-protocol-parameter-transition]')?.textContent).toContain('本次编辑');
    expect(saved).not.toHaveProperty('protocolParameterStash');
    await chooseProtocol(page, scenario.protocol);
    expect(JSON.parse(page.getByTestId('serialized').textContent!)[0].provider_params).toEqual({ [scenario.parameter]: scenario.value, future: { mode: 'fast' } });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
  });

  test('converts structured output with stream controls directly into a saveable Responses draft', async () => {
    const page = render(<Harness raw={JSON.stringify({ response_format: { type: 'json_schema', json_schema: { name: 'answer', schema: { type: 'object' }, strict: true } },
      verbosity: 'high', reasoning: { effort: 'low' }, stream_options: null, max_tokens_field: 'max_completion_tokens', vendor: { enabled: true } })} />);
    openProtocol(page); await chooseProtocol(page, 'openai.responses');
    const saved = JSON.parse(page.getByTestId('serialized').textContent!)[0];
    expect(saved.provider_params).toEqual({ text: { format: { type: 'json_schema', name: 'answer', schema: { type: 'object' }, strict: true }, verbosity: 'high' },
      reasoning_effort: 'low', vendor: { enabled: true } });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
    expect(page.container.querySelector('[data-protocol-parameter-transition]')?.textContent).toContain('输出格式');
    expect(page.container.querySelector('[data-protocol-parameter-transition]')?.textContent).toContain('回答详略');
  });

  test('format conflicts require an explicit choice and unrelated edits cannot dismiss the review', async () => {
    const raw = JSON.stringify({ response_format: { type: 'json_object' }, text: { format: { type: 'text' } } });
    const page = render(<Harness raw={raw} />);
    openProtocol(page); await chooseProtocol(page, 'openai.responses');
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    expect(page.container.querySelector('[data-protocol-transition-conflicts]')?.textContent).toContain('输出格式');
    expect(page.container.querySelector('[data-protocol-transition-conflicts]')?.textContent).toContain('不参与当前调用');
    const textarea = page.container.querySelector<HTMLTextAreaElement>('[data-provider-params-json] textarea')!;
    fireEvent.change(textarea, { target: { value: JSON.stringify({ ...JSON.parse(raw), temperature: 0.2 }) } });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(page.getByRole('button', { name: zhSettings.protocolTransition.useTarget }));
    expect(JSON.parse(page.getByTestId('serialized').textContent!)[0].provider_params).toEqual({ text: { format: { type: 'text' } }, temperature: 0.2 });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
    await chooseProtocol(page, 'openai.chat_text');
    expect(JSON.parse(page.getByTestId('serialized').textContent!)[0].provider_params.response_format).toEqual({ type: 'text' });
    fireEvent.click(page.getByRole('button', { name: zhSettings.protocolTransition.restore }));
    expect(JSON.parse(page.getByTestId('serialized').textContent!)[0].provider_params).toEqual({ response_format: { type: 'json_object' }, temperature: 0.2 });
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
  });

  test('invalid JSON stays verbatim and offers a return action without requiring its deletion', async () => {
    const raw = '{  "unfinished":';
    const page = render(<Harness raw={raw} />);
    openProtocol(page); await chooseProtocol(page, 'openai.responses');
    const textarea = page.container.querySelector<HTMLTextAreaElement>('[data-provider-params-json] textarea')!;
    expect(textarea.value).toBe(raw);
    fireEvent.click(page.getByRole('button', { name: '切回 openai.chat_text' }));
    expect(textarea.value).toBe(raw);
    expect(page.queryByRole('button', { name: '切回 openai.chat_text' })).toBeNull();
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
  });
});
