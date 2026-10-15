import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { DetectChatProtocolResponse } from '@/common/protocolBindings/DetectChatProtocolResponse';
import type { ProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import ModelProtocolDetectionControl from './ModelProtocolDetectionControl';
import { chatProtocolDetectionSnapshot } from './modelProtocolDetection';
import { capabilityDraftFromResponse, capabilityInputsFromDefinition, type ModelDefinitionDraft } from './providerModelAdvanced';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings } } }, interpolation: { escapeValue: false } });
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); });
const capability: ProviderModelCapabilityInput = {
  task: 'chat', protocol: 'openai.responses', connection_role: 'default', endpoint: '/responses',
  context_limit: 400_000, output_limit: 64_000, traits: ['vision_input'],
  provider_params: { reasoning_effort: 'high', temperature: 0.4, future: { enabled: true } },
};
const selected = { ...capability, protocol: 'openai.chat_text', endpoint: '/chat/completions' };
const verified = (input = selected): DetectChatProtocolResponse => ({
  selected_protocol: input.protocol, selected_capability: input, elapsed_ms: 12,
  attempts: [
    { protocol: 'openai.responses', status: 'incompatible', message: 'Model does not support Responses', error_kind: 'invalid_request', elapsed_ms: 5, probe_output_limit: 4096 },
    { protocol: input.protocol, status: 'verified', elapsed_ms: 7, probe_output_limit: 4096 },
  ],
});
const manifests = { chat: { ...aliasProtocolManifest, protocols: [
  ...aliasProtocolManifest.protocols, { ...aliasProtocolManifest.protocols[0], protocol_id: 'openai.responses' },
] } };
const noErrors: never[] = [];
function Harness({ protocol = 'openai.responses', stored = false }: { protocol?: string; stored?: boolean }) {
  const [value, setValue] = useState<ModelDefinitionDraft>({ model: 'step-5-preview', capabilities: [capabilityDraftFromResponse({ ...capability, protocol,
    endpoint: protocol === 'openai.chat_text' ? '/chat/completions' : capability.endpoint,
  })] });
  const [pending, setPending] = useState(false);
  const [credentialVersion, setCredentialVersion] = useState(0);
  return <I18nextProvider i18n={i18n}>
    <button onClick={() => setValue(current => ({ ...current, capabilities: current.capabilities.map(row => ({ ...row,
      protocol: 'openai.chat_text', endpoint: '/manual/chat', transportSource: 'user',
    })) }))}>Choose manual Chat</button>
    <button onClick={() => setCredentialVersion(version => version + 1)}>Change credentials</button>
    <ModelDefinitionEditor value={value} onChange={setValue} providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer'
      manifests={manifests} validationErrors={noErrors}
      protocolDetection={{ platform: 'stepfun', baseUrl: aliasProviderBaseUrl, authScheme: 'bearer',
        ...(stored ? { providerId: aliasProviderId } : { credentials: { api_keys: [`test-key-${credentialVersion}`] } }),
      }} onProtocolDetectionPendingChange={setPending} />
    <button data-testid='save' disabled={pending}>Save</button>
    <output data-testid='draft'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
  </I18nextProvider>;
}

describe('explicit actual-model protocol detection', () => {
  test('does not make paid calls while typing and applies the exact verified Chat fallback only after clicking', async () => {
    const detect = spyOn(ipcBridge.mode.detectChatProtocol, 'invoke').mockResolvedValue(verified());
    const savedDetect = spyOn(ipcBridge.mode.detectProviderChatProtocol, 'invoke').mockResolvedValue(verified());
    restorers.push(() => detect.mockRestore(), () => savedDetect.mockRestore());
    const page = render(<Harness />);
    fireEvent.change(page.getByLabelText(zhSettings.modelId), { target: { value: 'step-5-preview-edited' } });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 600)); });
    expect(detect).not.toHaveBeenCalled(); expect(savedDetect).not.toHaveBeenCalled();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDetection.action }));
    await waitFor(() => expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(selected));
    expect(detect.mock.calls[0][0]).toMatchObject({ model: 'step-5-preview-edited', capability,
      platform: 'stepfun', base_url: aliasProviderBaseUrl, credentials: { api_keys: ['test-key-0'] } });
    expect(page.getByText(/已验证并选择 Chat Completions/)).toBeTruthy();
    expect(page.getByText(/Responses · 不兼容/)).toBeTruthy();
    expect(page.getByText(/Chat Completions · 模型调用通过/)).toBeTruthy();
    expect(page.getByText(/可能产生少量费用/)).toBeTruthy();
  });

  test('passes the current saved manual Chat protocol and keeps it when verified', async () => {
    const detect = spyOn(ipcBridge.mode.detectProviderChatProtocol, 'invoke').mockResolvedValue({
      selected_protocol: selected.protocol, selected_capability: selected, elapsed_ms: 5,
      attempts: [{ protocol: selected.protocol, status: 'verified', elapsed_ms: 5, probe_output_limit: 4096 }],
    });
    restorers.push(() => detect.mockRestore());
    const page = render(<Harness protocol='openai.chat_text' stored />);
    expect(JSON.parse(page.getByTestId('draft').textContent!)[0].protocol).toBe('openai.chat_text');
    expect(detect).not.toHaveBeenCalled();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDetection.action }));
    await waitFor(() => expect(detect).toHaveBeenCalledTimes(1));
    expect(detect.mock.calls[0][0]).toMatchObject({ provider_id: aliasProviderId, capability: { protocol: 'openai.chat_text' } });
    await waitFor(() => expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(selected));
  });

  test('retains the original capability when neither protocol is verified', async () => {
    const detect = spyOn(ipcBridge.mode.detectChatProtocol, 'invoke').mockResolvedValue({ elapsed_ms: 10,
      attempts: [{ protocol: 'openai.responses', status: 'failed', message: 'Credentials rejected', error_kind: 'unauthorized', http_status: 401, elapsed_ms: 10, probe_output_limit: 4096 }],
    });
    restorers.push(() => detect.mockRestore());
    const page = render(<Harness />);
    const original = page.getByTestId('draft').textContent;
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDetection.action }));
    await page.findByText(/尚未找到可用协议/);
    expect(page.getByTestId('draft').textContent).toBe(original);
    expect(page.getByText(/Credentials rejected/)).toBeTruthy();
  });

  for (const change of ['Choose manual Chat', 'Change credentials']) test(`ignores an in-flight result after ${change}`, async () => {
    let complete!: (value: DetectChatProtocolResponse) => void;
    const detect = spyOn(ipcBridge.mode.detectChatProtocol, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    restorers.push(() => detect.mockRestore());
    const page = render(<Harness />);
    const button = page.getByRole('button', { name: zhSettings.modelProtocolDetection.action }) as HTMLButtonElement;
    fireEvent.click(button); fireEvent.click(button);
    expect(button.disabled).toBe(true); expect(detect).toHaveBeenCalledTimes(1);
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(page.getByRole('button', { name: change }));
    const changed = page.getByTestId('draft').textContent;
    await act(async () => complete(verified()));
    await page.findByText(zhSettings.modelProtocolDetection.stale);
    expect(page.getByTestId('draft').textContent).toBe(changed);
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
    if (change === 'Choose manual Chat') expect(JSON.parse(changed!)[0].endpoint).toBe('/manual/chat');
  });

  test('ignores the bounded result after closing the editor', async () => {
    let complete!: (value: DetectChatProtocolResponse) => void;
    const detect = spyOn(ipcBridge.mode.detectChatProtocol, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    restorers.push(() => detect.mockRestore());
    const applied: unknown[] = [];
    const page = render(<I18nextProvider i18n={i18n}><ModelProtocolDetectionControl model='step-5-preview' capability={capability}
      snapshot={chatProtocolDetectionSnapshot('step-5-preview', capabilityDraftFromResponse(capability))}
      context={{ platform: 'stepfun', baseUrl: aliasProviderBaseUrl, authScheme: 'bearer', credentials: { api_keys: ['test'] } }}
      onSelect={value => applied.push(value)} /></I18nextProvider>);
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDetection.action }));
    page.unmount(); await act(async () => complete(verified()));
    expect(applied).toEqual([]);
  });
});
