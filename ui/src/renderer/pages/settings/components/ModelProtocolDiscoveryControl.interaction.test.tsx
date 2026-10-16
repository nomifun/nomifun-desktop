import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { DiscoverModelProtocolsResponse } from '@/common/protocolBindings/DiscoverModelProtocolsResponse';
import type { ModelProtocolCandidate } from '@/common/protocolBindings/ModelProtocolCandidate';
import type { ProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import ModelProtocolDiscoveryControl from './ModelProtocolDiscoveryControl';
import { applyDiscoveredModelProtocol, modelProtocolDiscoveryAppliesDefaults, modelProtocolDiscoverySnapshot } from './modelProtocolDiscovery';
import { capabilityDraftFromResponse, capabilityInputsFromDefinition, changeCapabilityProtocol, emptyCapabilityDraft, patchCapabilityDraft, type ModelDefinitionDraft, type ModelProtocolManifestMap } from './providerModelAdvanced';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings } } }, interpolation: { escapeValue: false } });
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); });
const capability: ProviderModelCapabilityInput = {
  task: 'chat', protocol: 'openai.responses', connection_role: 'default', endpoint: '/responses',
  context_limit: 400_000, output_limit: 64_000, traits: ['vision_input'],
  provider_params: { reasoning_effort: 'high', temperature: 0.4, future: { enabled: true } },
};
const chat = { ...capability, protocol: 'openai.chat_text', endpoint: '/chat/completions' };
const candidate = (input: ProviderModelCapabilityInput, overrides: Partial<ModelProtocolCandidate> = {}): ModelProtocolCandidate => ({
  protocol: input.protocol, capability: input, status: 'available', evidence: 'verified_stream', recommended: input.protocol === 'openai.responses', elapsed_ms: 5, ...overrides,
});
const response = (selected = capability, candidates = [candidate(capability), candidate(chat)]): DiscoverModelProtocolsResponse => ({
  selected_protocol: selected.protocol, selected_capability: selected, candidates, elapsed_ms: 12,
});
const manifests: ModelProtocolManifestMap = { chat: { ...aliasProtocolManifest, protocols: [
  ...aliasProtocolManifest.protocols, { ...aliasProtocolManifest.protocols[0], protocol_id: 'openai.responses' },
] }, image_generation: { ...aliasProtocolManifest, tasks: ['image_generation'], requested_task: 'image_generation', recommendation: null,
  protocols: ['openai.images','ark.images'].map(protocol_id => ({ ...aliasProtocolManifest.protocols[0], protocol_id, supported_tasks: ['image_generation'],
    endpoints: [{ ...aliasProtocolManifest.protocols[0].endpoints[0], task: 'image_generation', default_value: '/images/generations' }] })),
} };
const noErrors: never[] = [];
function Harness({ stored = false, initialModel = 'step-5-preview', initialCapability = capability, initialCapabilities, catalog = false }: {
  stored?: boolean; initialModel?: string; initialCapability?: ProviderModelCapabilityInput; initialCapabilities?: ProviderModelCapabilityInput[]; catalog?: boolean;
}) {
  const [value, setValue] = useState<ModelDefinitionDraft>({ model: initialModel, capabilities: (initialCapabilities ?? [initialCapability]).map(input => ({ ...capabilityDraftFromResponse(input),
    protocolSelectionSource: stored ? 'manual' : 'automatic', transportSource: stored ? 'persisted' : 'user',
  })) });
  const [pending, setPending] = useState(false);
  const [credentialVersion, setCredentialVersion] = useState(0);
  return <I18nextProvider i18n={i18n}>
    <button onClick={() => setValue(current => ({ ...current, capabilities: current.capabilities.map(row => ({
      ...changeCapabilityProtocol(row, 'openai.chat_text'), endpoint: '/manual/chat',
    })) }))}>Choose manual Chat</button>
    <button onClick={() => setCredentialVersion(version => version + 1)}>Change credentials</button>
    <button onClick={() => setValue(current => ({ ...current, displayName: 'Renamed' }))}>Change alias</button>
    <button onClick={() => setValue(current => ({ ...current, capabilities: current.capabilities.map(row => patchCapabilityDraft(row, {
      providerParamsJson: JSON.stringify({ ...JSON.parse(row.providerParamsJson), temperature: Number((Number(JSON.parse(row.providerParamsJson).temperature) + 0.1).toFixed(1)) }),
      providerParamsEditedKeys: ['temperature'],
    })) }))}>Change parameters</button>
    <ModelDefinitionEditor value={value} onChange={setValue} providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer'
      manifests={manifests} validationErrors={noErrors} modelReadOnly={stored}
      catalogSuggestions={catalog ? [{ value: 'catalog-model', label: 'Catalog Model', tasks: ['chat'], traits: [], tasksSource: 'provider_declared' }] : []}
      protocolDiscovery={{ platform: 'stepfun', baseUrl: aliasProviderBaseUrl, authScheme: 'bearer',
        ...(stored ? { providerId: aliasProviderId } : { credentials: { api_keys: [`test-key-${credentialVersion}`] } }),
      }} onProtocolDiscoveryPendingChange={setPending} />
    <button data-testid='save' disabled={pending}>Save</button>
    <output data-testid='draft'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
  </I18nextProvider>;
}
const pause = async (ms = 450) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });

describe('automatic model protocol discovery', () => {
  test('only a committed pending configuration shows a loading indicator and busy status until it finishes', async () => {
    let complete!: (value: DiscoverModelProtocolsResponse) => void;
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialModel='' />);
    const region = page.container.querySelector('[data-model-protocol-discovery]')!;
    const input = page.getByLabelText(zhSettings.modelId);
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    fireEvent.change(input, { target: { value: 'step-' } });
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.connected)).toBeNull();
    fireEvent.change(input, { target: { value: 'step-5-preview' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(discover).not.toHaveBeenCalled();
    expect(region.getAttribute('aria-busy')).toBe('true');
    expect(region.querySelector('[role="status"]')?.textContent).toBe(zhSettings.modelProtocolDiscovery.checking);
    expect(region.querySelector('[role="status"]')?.getAttribute('aria-atomic')).toBe('true');
    expect(region.querySelector('.arco-spin')?.getAttribute('aria-hidden')).toBe('true');
    expect(region.querySelector('.arco-icon-loading')?.classList.contains('motion-reduce:!animate-none')).toBe(true);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(region.getAttribute('aria-busy')).toBe('true');
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    await act(async () => complete(response()));
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
    expect(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics })).toBeNull();
  });

  test('editing a checked model ID cancels its loading indicator and cannot show its late success', async () => {
    let complete!: (value: DiscoverModelProtocolsResponse) => void;
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    const region = page.container.querySelector('[data-model-protocol-discovery]')!;
    const signal = discover.mock.calls[0][1]!;
    expect(region.getAttribute('aria-busy')).toBe('true');
    fireEvent.change(page.getByLabelText(zhSettings.modelId), { target: { value: 'another-' } });
    const edited = page.getByTestId('draft').textContent;
    expect(signal.aborted).toBe(true);
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    expect(region.querySelector('[role="status"]')?.textContent).toBe(zhSettings.modelProtocolDiscovery.awaitCommit);
    await act(async () => complete(response(chat)));
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.connected)).toBeNull();
    expect(page.getByTestId('draft').textContent).toBe(edited);
    await pause();
    expect(discover).toHaveBeenCalledTimes(1);
    expect(region.getAttribute('aria-busy')).toBe('false');
  });

  test('rechecking replaces previous success with loading and removes the indicator after a failed check', async () => {
    let fail!: () => void;
    const rawError = 'A transient transport error';
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValueOnce(response())
      .mockImplementation(() => new Promise((_resolve, reject) => { fail = () => reject(new Error(rawError)); }));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    const configured = page.getByTestId('draft').textContent;
    const region = page.container.querySelector('[data-model-protocol-discovery]')!;
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings }));
    const recheck = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.recheck }) as HTMLButtonElement;
    fireEvent.click(recheck);
    expect(region.getAttribute('aria-busy')).toBe('true');
    expect(region.querySelector('.arco-spin')).toBeTruthy();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.connected)).toBeNull();
    expect(recheck.disabled).toBe(true);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    await act(async () => fail());
    await page.findByText(zhSettings.modelProtocolDiscovery.unavailable);
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    expect(page.queryByText(rawError)).toBeNull();
    expect(page.getByTestId('draft').textContent).toBe(configured);
    expect(recheck.disabled).toBe(false);
  });

  test('typing prepares an ID, Enter checks once, and automatic selection cannot trigger itself again', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response(chat, [
      candidate(capability, { status: 'incompatible', capability: undefined, message: 'Model does not support Responses' }), candidate(chat),
    ]));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialModel='' />);
    const input = page.getByLabelText(zhSettings.modelId);
    fireEvent.change(input, { target: { value: 'step-' } }); await pause();
    fireEvent.change(input, { target: { value: 'step-5-preview' } }); await pause();
    expect(discover).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(chat));
    expect(discover.mock.calls[0][0]).toMatchObject({ model: 'step-5-preview', capability,
      platform: 'stepfun', base_url: aliasProviderBaseUrl, credentials: { api_keys: ['test-key-0'] } });
    expect(discover.mock.calls[0][0]).not.toHaveProperty('preferred_protocol');
    expect(page.container.querySelector('[data-model-protocol-discovery] [role="status"]')?.textContent).toBe(zhSettings.modelProtocolDiscovery.connected);
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.incompatible)).toBeNull();
    expect(page.queryByText('Model does not support Responses')).toBeNull();
    expect(page.queryByText(/可能产生少量用量/)).toBeNull();
    expect(page.queryByRole('button', { name: 'Responses' })).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.diagnostics)).toBeNull();
    fireEvent.blur(input); fireEvent.click(page.getByRole('button', { name: 'Change alias' })); await pause();
    expect(discover).toHaveBeenCalledTimes(1);
  });

  test('a completed manual ID checks on blur without clicking a detection button', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response());
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialModel='' />);
    const input = page.getByLabelText(zhSettings.modelId);
    fireEvent.change(input, { target: { value: 'future/vendor-model' } }); fireEvent.blur(input);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(discover.mock.calls[0][0].model).toBe('future/vendor-model');
  });

  test('catalog selection automatically checks the selected model', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response());
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialModel='' catalog />);
    fireEvent.focus(page.getByLabelText(zhSettings.modelId));
    fireEvent.click(await page.findByText('Catalog Model'));
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(discover.mock.calls[0][0].model).toBe('catalog-model');
  });

  test('a delayed input blur cannot revoke the model committed by catalog selection', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response());
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialModel='' catalog />);
    const input = page.getByLabelText(zhSettings.modelId);
    fireEvent.change(input, { target: { value: 'catalog-' } });
    fireEvent.focus(input);
    const option = await page.findByText('Catalog Model');
    fireEvent.blur(input);
    fireEvent.click(option);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(discover.mock.calls[0][0].model).toBe('catalog-model');
    expect((page.getByLabelText(zhSettings.modelId) as HTMLInputElement).value).toBe('catalog-model');
  });

  test('opening an existing model automatically checks and preserves its available saved choice', async () => {
    const discover = spyOn(ipcBridge.mode.discoverProviderModelProtocols, 'invoke').mockResolvedValue(response(chat));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness stored initialCapability={chat} />);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(discover.mock.calls[0][0]).toMatchObject({ provider_id: aliasProviderId, preferred_protocol: 'openai.chat_text', capability: chat });
    expect(discover.mock.calls[0][0].apply_recommended_defaults).toBe(false);
    await waitFor(() => expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(chat));
    await pause(); expect(discover).toHaveBeenCalledTimes(1);
  });

  test('connection settings reveal available candidates and a user choice is retained without another paid request', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response());
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    const region = page.container.querySelector('[data-model-protocol-discovery]')!;
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    expect(page.queryByRole('button', { name: 'Chat Completions' })).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings }));
    const chooseChat = page.getByRole('button', { name: 'Chat Completions' });
    expect(page.getByRole('button', { name: 'Responses' })).toBeTruthy();
    expect(page.getByText(zhSettings.modelProtocolDiscovery.recommended)).toBeTruthy();
    fireEvent.click(chooseChat);
    expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(chat);
    expect(region.getAttribute('aria-busy')).toBe('false');
    expect(region.querySelector('.arco-spin')).toBeNull();
    fireEvent.click(page.getByRole('button', { name: 'Change alias' })); await pause();
    expect(discover).toHaveBeenCalledTimes(1);
    expect(JSON.parse(page.getByTestId('draft').textContent!)[0].protocol).toBe('openai.chat_text');
  });

  test('an inconclusive result keeps the draft and remains distinct from incompatibility', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue({ elapsed_ms: 10,
      candidates: [candidate(capability, { status: 'inconclusive', capability: undefined, message: 'Credentials rejected', error_kind: 'unauthorized', http_status: 401 })],
    });
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />); const original = page.getByTestId('draft').textContent;
    await page.findByText(zhSettings.modelProtocolDiscovery.unavailable);
    expect(page.getByTestId('draft').textContent).toBe(original);
    expect(page.queryByText('Credentials rejected')).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.inconclusive)).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings }));
    expect(page.queryByText('Credentials rejected')).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics }));
    expect(page.getByText(zhSettings.modelProtocolDiscovery.inconclusive)).toBeTruthy();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.incompatible)).toBeNull();
  });

  test('raw request failures stay behind two disclosures and keep the default status concise', async () => {
    const rawError = 'HTTP 400 {"error":{"code":"unsupported_parameter","message":"Unexpected vendor option"}}';
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockRejectedValue(new Error(rawError));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    const original = page.getByTestId('draft').textContent;
    await page.findByText(zhSettings.modelProtocolDiscovery.unavailable);
    expect(page.container.querySelector('[data-model-protocol-discovery] [role="status"]')?.textContent).toBe(zhSettings.modelProtocolDiscovery.unavailable);
    expect(page.queryByText(rawError)).toBeNull();
    expect(page.queryByRole('alert')).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings }));
    expect(page.queryByText(rawError)).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics }));
    expect(page.getByText(rawError)).toBeTruthy();
    expect(page.getByTestId('draft').textContent).toBe(original);
  });

  test('only an entirely unusable configuration asks for connection changes without exposing its raw error', async () => {
    const rawError = 'The configured endpoint cannot be converted: {"endpoint":"/custom"}';
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue({ elapsed_ms: 10,
      candidates: [candidate(capability, { status: 'blocked', capability: undefined, message: rawError, error_kind: 'invalid_request' })],
    });
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await page.findByText(zhSettings.modelProtocolDiscovery.reviewConfiguration);
    expect(page.queryByText(rawError)).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.blocked)).toBeNull();
    expect(page.queryByRole('alert')).toBeNull();
  });

  test('changing the model closes connection settings and technical diagnostics, including after the new result arrives', async () => {
    const failure = 'Responses is not available for this model';
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response(chat, [
      candidate(capability, { status: 'incompatible', capability: undefined, message: failure }), candidate(chat),
    ]));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    const connectionSettings = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings });
    fireEvent.click(connectionSettings);
    const diagnostics = page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics });
    expect(page.queryByText(failure)).toBeNull();
    fireEvent.click(diagnostics);
    expect(page.getByText(failure)).toBeTruthy();
    const input = page.getByLabelText(zhSettings.modelId);
    fireEvent.change(input, { target: { value: 'another-model' } });
    expect(connectionSettings.getAttribute('aria-expanded')).toBe('false');
    expect(page.queryByText(failure)).toBeNull();
    expect(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics })).toBeNull();
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    expect(connectionSettings.getAttribute('aria-expanded')).toBe('false');
    expect(page.queryByText(failure)).toBeNull();
    fireEvent.click(connectionSettings);
    expect(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics }).getAttribute('aria-expanded')).toBe('false');
    expect(page.queryByText(failure)).toBeNull();
    expect(discover.mock.calls[1][0].model).toBe('another-model');
  });

  for (const change of ['Choose manual Chat', 'Change credentials']) test(`cancels and ignores a stale in-flight result after ${change}`, async () => {
    let complete!: (value: DiscoverModelProtocolsResponse) => void;
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    const signal = discover.mock.calls[0][1]!;
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(page.getByRole('button', { name: change }));
    expect(signal.aborted).toBe(true);
    const changed = page.getByTestId('draft').textContent;
    await act(async () => complete(response(chat)));
    expect(page.getByTestId('draft').textContent).toBe(changed);
  });

  test('closing the editor cancels its request and ignores the result', async () => {
    let complete!: (value: DiscoverModelProtocolsResponse) => void;
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    restorers.push(() => discover.mockRestore());
    const applied: unknown[] = [];
    const page = render(<I18nextProvider i18n={i18n}><ModelProtocolDiscoveryControl model='step-5-preview' committed applyRecommendedDefaults={false} capability={capability}
      snapshot={modelProtocolDiscoverySnapshot('step-5-preview', capabilityDraftFromResponse(capability))}
      context={{ platform: 'stepfun', baseUrl: aliasProviderBaseUrl, authScheme: 'bearer', credentials: { api_keys: ['test'] } }}
      onSelect={value => applied.push(value)} /></I18nextProvider>);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    const signal = discover.mock.calls[0][1]!;
    page.unmount(); expect(signal.aborted).toBe(true); await act(async () => complete(response(chat)));
    expect(applied).toEqual([]);
  });

  test('non-chat candidates identify documentary support instead of claiming a generation test', async () => {
    const image: ProviderModelCapabilityInput = { task: 'image_generation', protocol: 'openai.images', connection_role: 'default', provider_params: {} };
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue(response(image, [candidate(image, { evidence: 'official_model' })]));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialCapability={image} />);
    await page.findByText(zhSettings.modelProtocolDiscovery.configured);
    expect(discover.mock.calls[0][0].capability.task).toBe('image_generation');
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.connected)).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.documented)).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings }));
    expect(page.getByText(zhSettings.modelProtocolDiscovery.documented)).toBeTruthy();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.verified)).toBeNull();
  });

  test('parameter edits are debounced to one exact new check and the applied result is deduplicated', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(async request => response(request.capability, [candidate(request.capability)]));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness />);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    const change = page.getByRole('button', { name: 'Change parameters' });
    fireEvent.click(change); fireEvent.click(change); fireEvent.click(change);
    expect(discover).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    expect(discover.mock.calls[1][0].capability.provider_params).toMatchObject({ temperature: 0.7 });
    await pause(); expect(discover).toHaveBeenCalledTimes(2);
  });

  test('save stays pending until every configured task finishes its check', async () => {
    const image: ProviderModelCapabilityInput = { task: 'image_generation', protocol: 'openai.images', connection_role: 'default', provider_params: {} };
    let completeImage!: (value: DiscoverModelProtocolsResponse) => void;
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(async request => request.capability.task === 'chat'
      ? response(request.capability, [candidate(request.capability)]) : new Promise(resolve => { completeImage = resolve; }));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialCapabilities={[capability, image]} />);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    await page.findByText(zhSettings.modelProtocolDiscovery.connected);
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(true);
    await act(async () => completeImage(response(image, [candidate(image, { evidence: 'official_model' })])));
    expect((page.getByTestId('save') as HTMLButtonElement).disabled).toBe(false);
  });

  test('blocked native candidates offer explicit reversible setup without deleting authored JSON', async () => {
    const original: ProviderModelCapabilityInput = { task: 'image_generation', protocol: 'openai.images', connection_role: 'default',
      endpoint: '/old-custom-images', provider_params: { response_format: 'b64_json', future: { colors: ['red'] } } };
    const native: ProviderModelCapabilityInput = { task: 'image_generation', protocol: 'ark.images', connection_role: 'default', provider_params: {} };
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(async request => request.capability.protocol === 'ark.images'
      ? response(native, [candidate(native, { evidence: 'official_model' })])
      : response(original, [candidate(original, { evidence: 'catalog_task' }), candidate(native, { status: 'blocked', capability: undefined,
          message: 'native protocol parameters cannot be automatically reinterpreted', evidence: 'official_model' })]));
    restorers.push(() => discover.mockRestore());
    const page = render(<Harness initialCapability={original} />);
    await page.findByText(zhSettings.modelProtocolDiscovery.configured);
    expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(original);
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.nativeSwitchImpact)).toBeNull();
    expect(page.queryByText(zhSettings.modelProtocolDiscovery.reviewConfiguration)).toBeNull();
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.connectionSettings }));
    fireEvent.click(page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.diagnostics }));
    expect(page.getByText(zhSettings.modelProtocolDiscovery.nativeSwitchImpact)).toBeTruthy();
    fireEvent.click(page.getByRole('button', { name: '暂存旧设置并配置 ark.images' }));
    const nativeWire = capabilityInputsFromDefinition({ model: 'step-5-preview', capabilities: [capabilityDraftFromResponse(native)] })![0];
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(JSON.parse(page.getByTestId('draft').textContent!)[0]).toEqual(nativeWire));
    expect(discover.mock.calls[1][0]).toMatchObject({ preferred_protocol: 'ark.images', capability: { protocol: 'ark.images' },
      credentials: { api_keys: ['test-key-0'] }, base_url: aliasProviderBaseUrl, apply_recommended_defaults: true });
    expect(discover.mock.calls[1][0].capability).not.toHaveProperty('provider_params');
    expect(discover.mock.calls[1][0].capability).not.toHaveProperty('endpoint');
    expect(page.getByTestId('draft').textContent).not.toContain('protocolParameterStash');
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.modelAdvanced.protocol }));
    fireEvent.click(await page.findByRole('option', { name: /openai.images/ }));
    const returned = JSON.parse(page.getByTestId('draft').textContent!)[0];
    expect(returned.provider_params).toEqual(original.provider_params);
    expect(returned).not.toHaveProperty('endpoint');
  });

  test('discovery waits for the manifest and sends the final recommendation-owned defaults once', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(async request => response(request.capability, [candidate(request.capability)]));
    restorers.push(() => discover.mockRestore());
    function DefaultsHarness() {
      const [value, setValue] = useState<ModelDefinitionDraft>({ model: 'new-model', capabilities: [emptyCapabilityDraft('chat')] });
      const [ready, setReady] = useState(false);
      const defaults: ModelProtocolManifestMap = { chat: { ...manifests.chat!, recommendation: { ...aliasProtocolManifest.recommendation!, protocol_id: 'openai.responses' },
        model_defaults: { model: 'new-model', protocol_id: 'openai.responses', context_limit: 200_000, output_limit: 32_000,
          provider_params: { reasoning_effort: 'low' }, sources: ['https://example.invalid/docs'], checked_at: '2026-10-16' } } };
      return <I18nextProvider i18n={i18n}>
        <button onClick={() => setReady(true)}>Release manifest</button>
        <ModelDefinitionEditor value={value} onChange={setValue} providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer'
          manifests={ready ? defaults : {}} manifestLoadingTasks={ready ? [] : ['chat']} validationErrors={noErrors}
          protocolDiscovery={{ platform: 'custom', baseUrl: aliasProviderBaseUrl, authScheme: 'bearer', credentials: { api_keys: ['test-key'] } }} />
      </I18nextProvider>;
    }
    const page = render(<DefaultsHarness />); await pause(); expect(discover).not.toHaveBeenCalled();
    fireEvent.click(page.getByRole('button', { name: 'Release manifest' }));
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(discover.mock.calls[0][0].capability).toMatchObject({ protocol: 'openai.responses', context_limit: 200_000, output_limit: 32_000,
      provider_params: { reasoning_effort: 'low' } });
    expect(discover.mock.calls[0][0].apply_recommended_defaults).toBe(true);
    await pause(); expect(discover).toHaveBeenCalledTimes(1);
  });

  test('clearing a recommended voice disables injection and keeps the backend-verified empty result without a loop', async () => {
    const native: ProviderModelCapabilityInput = { task: 'speech_synthesis', protocol: 'gemini.interactions_tts', connection_role: 'default' };
    const withDefault = { ...native, provider_params: { voice: 'Kore' } };
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(async request => request.apply_recommended_defaults
      ? response(withDefault, [candidate(withDefault, { evidence: 'official_model' })])
      : response(native, [candidate(native, { evidence: 'official_model' })]));
    restorers.push(() => discover.mockRestore());
    function PolicyHarness() {
      const [value, setValue] = useState<ModelDefinitionDraft>({ model: 'native-tts', capabilities: [{ ...emptyCapabilityDraft('speech_synthesis'),
        protocol: native.protocol, routeSource: 'user', transportSource: 'recommendation' }] });
      const current = value.capabilities[0];
      return <I18nextProvider i18n={i18n}>
        <button onClick={() => setValue(previous => ({ ...previous, capabilities: previous.capabilities.map(capability => patchCapabilityDraft(capability,
          { providerParamsJson: '', providerParamsEditedKeys: ['voice'] })) }))}>Clear voice</button>
        <ModelProtocolDiscoveryControl model={value.model} committed applyRecommendedDefaults={modelProtocolDiscoveryAppliesDefaults(current)}
          capability={capabilityInputsFromDefinition(value)![0]} snapshot={modelProtocolDiscoverySnapshot(value.model, current)}
          context={{ platform: 'gemini', baseUrl: aliasProviderBaseUrl, authScheme: 'none', credentials: {} }}
          onSelect={(snapshot, selected, source) => setValue(previous => applyDiscoveredModelProtocol(previous, snapshot, selected, source))} />
        <output data-testid='policy-draft'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
      </I18nextProvider>;
    }
    const page = render(<PolicyHarness />);
    await waitFor(() => expect(JSON.parse(page.getByTestId('policy-draft').textContent!)[0].provider_params).toEqual({ voice: 'Kore' }));
    expect(discover.mock.calls[0][0].apply_recommended_defaults).toBe(true);
    await pause(); expect(discover).toHaveBeenCalledTimes(1);
    fireEvent.click(page.getByRole('button', { name: 'Clear voice' }));
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    expect(discover.mock.calls[1][0].apply_recommended_defaults).toBe(false);
    expect(discover.mock.calls[1][0].capability).not.toHaveProperty('provider_params');
    await pause(); expect(discover).toHaveBeenCalledTimes(2);
    expect(JSON.parse(page.getByTestId('policy-draft').textContent!)[0]).toEqual(native);
  });

  test('a same-JSON ownership change cancels a defaults-enabled request and ignores its late recommendation', async () => {
    const native: ProviderModelCapabilityInput = { task: 'speech_synthesis', protocol: 'gemini.interactions_tts', connection_role: 'default' };
    let complete!: (result: DiscoverModelProtocolsResponse) => void;
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockImplementation(async request => request.apply_recommended_defaults
      ? new Promise(resolve => { complete = resolve; }) : response(native, [candidate(native, { evidence: 'official_model' })]));
    restorers.push(() => discover.mockRestore());
    function OwnershipHarness() {
      const [value, setValue] = useState<ModelDefinitionDraft>({ model: 'native-tts', capabilities: [{ ...emptyCapabilityDraft('speech_synthesis'),
        protocol: native.protocol, routeSource: 'user', transportSource: 'recommendation' }] });
      const current = value.capabilities[0];
      return <I18nextProvider i18n={i18n}>
        <button onClick={() => setValue(previous => ({ ...previous, capabilities: previous.capabilities.map(capability => patchCapabilityDraft(capability,
          { providerParamsJson: '' })) }))}>Own empty parameters</button>
        <ModelProtocolDiscoveryControl model={value.model} committed applyRecommendedDefaults={modelProtocolDiscoveryAppliesDefaults(current)}
          capability={capabilityInputsFromDefinition(value)![0]} snapshot={modelProtocolDiscoverySnapshot(value.model, current)}
          context={{ platform: 'gemini', baseUrl: aliasProviderBaseUrl, authScheme: 'none', credentials: {} }}
          onSelect={(snapshot, selected, source) => setValue(previous => applyDiscoveredModelProtocol(previous, snapshot, selected, source))} />
        <output data-testid='owned-draft'>{JSON.stringify(capabilityInputsFromDefinition(value))}</output>
      </I18nextProvider>;
    }
    const page = render(<OwnershipHarness />);
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    const originalWire = page.getByTestId('owned-draft').textContent;
    const signal = discover.mock.calls[0][1]!;
    fireEvent.click(page.getByRole('button', { name: 'Own empty parameters' }));
    expect(page.getByTestId('owned-draft').textContent).toBe(originalWire);
    expect(signal.aborted).toBe(true);
    await act(async () => complete(response({ ...native, provider_params: { voice: 'Kore' } }, [candidate({ ...native, provider_params: { voice: 'Kore' } })])));
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
    expect(discover.mock.calls[1][0].apply_recommended_defaults).toBe(false);
    await pause(); expect(page.getByTestId('owned-draft').textContent).toBe(originalWire);
    expect(discover).toHaveBeenCalledTimes(2);
  });
});
