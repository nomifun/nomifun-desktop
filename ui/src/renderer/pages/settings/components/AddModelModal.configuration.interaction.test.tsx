import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig, unstable_serialize } from 'swr';
import type { IProvider } from '@/common/config/storage';
import type { DiscoverModelProtocolsResponse } from '@/common/protocolBindings/DiscoverModelProtocolsResponse';
import type { ProviderModelCapabilityInput, ProviderModelResponse } from '@/common/types/provider/providerModel';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasCapability, aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { default: AddModelModal } = await import('./AddModelModal');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
restoreTransport();
beforeEach(installTransport);
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); restoreTransport(); });

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings, common: zhCommon } } },
  interpolation: { escapeValue: false },
});
const MODEL = 'pending-model';
const manifest = { ...aliasProtocolManifest, protocols: [
  ...aliasProtocolManifest.protocols,
  { ...aliasProtocolManifest.protocols[0]!, protocol_id: 'openai.responses', endpoints: [
    { ...aliasProtocolManifest.protocols[0]!.endpoints[0]!, default_value: '/responses' },
  ] },
] };
const selectedCapability: ProviderModelCapabilityInput = {
  task: 'chat', protocol: 'openai.responses', connection_role: 'default', endpoint: '/responses',
  context_limit: 200_000, output_limit: 8_192, provider_params: { reasoning_effort: 'low' },
};
const completedDiscovery = (): DiscoverModelProtocolsResponse => ({
  selected_protocol: selectedCapability.protocol, selected_capability: selectedCapability,
  candidates: [{ protocol: selectedCapability.protocol, capability: selectedCapability, status: 'available',
    evidence: 'verified_stream', recommended: true, elapsed_ms: 5 }], elapsed_ms: 5,
});
const modelResponse: ProviderModelResponse = {
  provider_id: aliasProviderId, model: MODEL, enabled: true, sort_order: 0,
  capabilities: [aliasCapability], created_at: 1, updated_at: 1,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(complete => { resolve = complete; });
  return { promise, resolve };
}

function mount({ duplicate = false, holdSave = false, platform = 'preview' } = {}) {
  const discovery = deferred<DiscoverModelProtocolsResponse>();
  const persistence = deferred<ProviderModelResponse>();
  const discover = spyOn(ipcBridge.mode.discoverProviderModelProtocols, 'invoke').mockImplementation(() => discovery.promise);
  const save = spyOn(ipcBridge.providerModel.save, 'invoke').mockImplementation(async () =>
    holdSave ? persistence.promise : modelResponse);
  const listProtocols = spyOn(ipcBridge.modelProtocol.list, 'invoke').mockResolvedValue(manifest);
  const listConnections = spyOn(ipcBridge.providerConnection.list, 'invoke').mockResolvedValue([]);
  const listModels = spyOn(ipcBridge.mode.fetchProviderModels, 'invoke').mockResolvedValue({ models: [] });
  const probe = spyOn(ipcBridge.mode.probeProviderConnection, 'invoke').mockImplementation(async request => ({
    reachability: 'reachable', protocol: request.protocol!, task: request.task!, root_shape: 'versioned_root',
    attempted_url: `${aliasProviderBaseUrl}/probe`, elapsed_ms: 5, candidates: [],
  }));
  restorers.push(() => discover.mockRestore(), () => save.mockRestore(), () => listProtocols.mockRestore(),
    () => listConnections.mockRestore(), () => listModels.mockRestore(), () => probe.mockRestore());
  const provider: IProvider = { id: aliasProviderId, platform, name: '配置测试',
    base_url: aliasProviderBaseUrl, auth_scheme: 'bearer', has_credentials: true,
    models: duplicate ? [modelResponse] : [] };
  const fallback: Record<string, unknown> = {
    [`provider-connections:${aliasProviderId}`]: [],
    [unstable_serialize(['provider-models', aliasProviderId, undefined])]: { models: [] },
  };
  for (const model of ['', MODEL]) {
    const requestKey = JSON.stringify([platform, aliasProviderBaseUrl, ['chat'], ...(model ? [model] : [])]);
    fallback[unstable_serialize(['model-protocol-manifests', requestKey])] = {
      requestKey, manifests: { chat: manifest }, errorTasks: [],
    };
  }
  function Harness() {
    const [control, modal] = AddModelModal.useModal({ data: provider, initialTask: 'chat', onSubmit: () => {} });
    return <><button onClick={() => control.open()}>Open model configuration</button>{modal}</>;
  }
  const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false, fallback }}>
    <ThemeProvider><Harness /></ThemeProvider>
  </SWRConfig></I18nextProvider>);
  const open = async () => {
    fireEvent.click(page.getByRole('button', { name: 'Open model configuration' }));
    return await page.findByLabelText(zhSettings.modelId);
  };
  const start = async () => {
    const input = await open();
    fireEvent.change(input, { target: { value: MODEL } });
    await act(async () => {});
    fireEvent.blur(page.getByLabelText(zhSettings.modelId));
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
  };
  const confirm = () => page.getByRole('button', { name: zhCommon.confirm }) as HTMLButtonElement;
  const force = () => page.getByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave }) as HTMLButtonElement;
  return { page, start, open, confirm, force, discover, save, discovery, persistence, probe };
}

describe('add-model configuration save controls', () => {
  for (const platform of ['custom', 'new-api']) {
    test(`${platform} reuses endpoint checks and saves the chosen protocol after one automatic model verification`, async () => {
      const { page, open, confirm, discover, save, probe, discovery } = mount({ platform });
      const input = await open();
      await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
      await page.findByRole('button', { name: 'openai.responses' });
      fireEvent.change(input, { target: { value: MODEL } });
      fireEvent.keyDown(input, { key: 'Enter' });
      await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
      expect(confirm().disabled).toBe(true);
      await act(async () => discovery.resolve(completedDiscovery()));
      await waitFor(() => expect(confirm().disabled).toBe(false));
      fireEvent.click(page.getByRole('button', { name: 'openai.responses' }));
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 650)); });
      expect(page.getByRole('button', { name: 'openai.responses' }).getAttribute('aria-pressed')).toBe('true');
      expect(probe).toHaveBeenCalledTimes(2);
      expect(discover).toHaveBeenCalledTimes(1);
      expect(confirm().disabled).toBe(false);
      fireEvent.click(confirm());
      await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
      expect(save.mock.calls[0]![0].model.capabilities[0]!.protocol).toBe('openai.responses');
    });
  }

  test('custom providers automatically verify the model and save its selected tested capability', async () => {
    const { open, confirm, discover, save, discovery } = mount({ platform: 'custom' });
    const input = await open();
    fireEvent.change(input, { target: { value: MODEL } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1));
    expect(confirm().disabled).toBe(true);
    await act(async () => discovery.resolve(completedDiscovery()));
    await waitFor(() => expect(confirm().disabled).toBe(false));
    fireEvent.click(confirm());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0]![0].model.capabilities).toEqual([selectedCapability]);
  });

  test('pending discovery disables ordinary confirmation without saving or warning', async () => {
    const { page, start, confirm, force, save } = mount();
    await start();
    expect(confirm().disabled).toBe(true);
    expect(confirm().getAttribute('aria-busy')).toBe('true');
    expect(force().disabled).toBe(false);
    fireEvent.click(confirm());
    expect(save).not.toHaveBeenCalled();
    expect(Boolean(page.baseElement.querySelector('.arco-message-warning'))).toBe(false);
  });

  test('force saves the current snapshot, aborts discovery, and ignores its late success', async () => {
    const { page, start, open, force, discover, save, discovery, persistence } = mount({ holdSave: true });
    await start();
    const request = discover.mock.calls[0]![0];
    const signal = discover.mock.calls[0]![1]!;
    fireEvent.click(force());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(signal.aborted).toBe(true));
    expect(save.mock.calls[0]![0]).toEqual({ provider_id: aliasProviderId,
      model: { model: MODEL, enabled: true, capabilities: [request.capability] } });
    expect(Boolean(page.baseElement.querySelector('[data-model-protocol-discovery]'))).toBe(false);
    await act(async () => discovery.resolve(completedDiscovery()));
    expect(save).toHaveBeenCalledTimes(1);
    expect(save.mock.calls[0]![0].model.capabilities[0]!.protocol).toBe('openai.chat_text');
    expect(save.mock.calls[0]![0].model.capabilities[0]!.output_limit).toBeUndefined();
    await act(async () => persistence.resolve(modelResponse));
    await waitFor(() => expect(Boolean(page.queryByLabelText(zhSettings.modelId))).toBe(false));
    const input = await open();
    fireEvent.change(input, { target: { value: MODEL } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2));
  });

  test('normal confirmation becomes available after discovery and saves its exact capability', async () => {
    const { page, start, confirm, discover, save, discovery } = mount();
    await start();
    await act(async () => discovery.resolve(completedDiscovery()));
    await waitFor(() => expect(confirm().disabled).toBe(false));
    expect(Boolean(page.queryByRole('button', { name: zhSettings.modelProtocolDiscovery.forceSave }))).toBe(false);
    fireEvent.click(confirm());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0]![0].model.capabilities).toEqual([selectedCapability]);
    expect(discover).toHaveBeenCalledTimes(1);
  });

  test('a failed force save keeps the same draft for retry without restarting discovery', async () => {
    const { page, start, confirm, force, discover, save, discovery } = mount();
    const saveError = spyOn(console, 'error').mockImplementation(() => {});
    restorers.push(() => saveError.mockRestore());
    save.mockRejectedValueOnce(new Error('local persistence fixture failed'));
    await start();
    fireEvent.click(force());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(confirm().disabled).toBe(false));
    expect((page.getByLabelText(zhSettings.modelId) as HTMLInputElement).value).toBe(MODEL);
    expect(discover.mock.calls[0]![1]!.aborted).toBe(true);
    await act(async () => discovery.resolve(completedDiscovery()));
    expect(discover).toHaveBeenCalledTimes(1);
    expect(Boolean(page.baseElement.querySelector('[data-model-protocol-discovery]'))).toBe(false);
    fireEvent.click(confirm());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    expect(save.mock.calls[1]![0]).toEqual(save.mock.calls[0]![0]);
    expect(save.mock.calls[1]![0].model.capabilities[0]!.protocol).toBe('openai.chat_text');
    await waitFor(() => expect(Boolean(page.queryByLabelText(zhSettings.modelId))).toBe(false));
  });

  test('force cannot bypass a duplicate-model validation error while discovery is pending', async () => {
    const { start, confirm, force, save } = mount({ duplicate: true });
    await start();
    expect(confirm().disabled).toBe(true);
    expect(force().disabled).toBe(true);
    fireEvent.click(force());
    expect(save).not.toHaveBeenCalled();
  });
});
