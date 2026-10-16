import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { aliasProtocolManifest, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { default: AddPlatformModal } = await import('./AddPlatformModal');
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

const BASE_URL = 'https://gateway.example/v1';
const ANTHROPIC_BASE_URL = 'https://gateway.example';
const OPENAI_BASE_URL = 'https://gateway.example/openai/v1';
const MODEL = 'custom-chat-model';
const manifest = {
  ...aliasProtocolManifest,
  preset: 'custom', platform: 'custom', platform_default_base_url: BASE_URL,
  auth_schemes: [{ scheme: 'bearer', parameterized: false }, { scheme: 'header_key:<name>', parameterized: true }],
  recommendation: { ...aliasProtocolManifest.recommendation!, default_base_url: BASE_URL },
  protocols: [
    aliasProtocolManifest.protocols[0]!,
    { ...aliasProtocolManifest.protocols[0]!, protocol_id: 'openai.responses', endpoints: [
      { ...aliasProtocolManifest.protocols[0]!.endpoints[0]!, default_value: '/responses' },
    ] },
    { ...aliasProtocolManifest.protocols[0]!, protocol_id: 'anthropic.messages', root_shape: 'origin_root' as const,
      allowed_auth_schemes: ['header_key:<name>'], endpoints: [
        { ...aliasProtocolManifest.protocols[0]!.endpoints[0]!, root_shape: 'origin_root' as const, default_value: '/v1/messages' },
      ] },
  ],
};

describe('add-provider interface configuration', () => {
  test('reuses setup and waits for explicit detection after credentials change while preserving the chosen connection', async () => {
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue({ candidates: [], elapsed_ms: 1 });
    const listProtocols = spyOn(ipcBridge.modelProtocol.list, 'invoke').mockResolvedValue(manifest);
    const listModels = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockResolvedValue({ models: [] });
    const save = spyOn(ipcBridge.mode.createProvider, 'invoke').mockResolvedValue({
      id: aliasProviderId, platform: 'custom', name: '自定义网关', base_url: ANTHROPIC_BASE_URL,
      auth_scheme: 'header_key:x-api-key', has_credentials: true, models: [],
    });
    const probe = spyOn(ipcBridge.mode.probeConnection, 'invoke').mockImplementation(async request => {
      const anthropic = request.protocol === 'anthropic.messages';
      const replacementKey = (request.credentials as { api_keys?: string[] }).api_keys?.[0] === 'replacement-secret';
      const suggestedBaseUrl = replacementKey
        ? (anthropic ? undefined : OPENAI_BASE_URL)
        : (anthropic ? ANTHROPIC_BASE_URL : undefined);
      return {
        reachability: anthropic && replacementKey ? 'unreachable' : 'reachable',
        protocol: request.protocol!, task: request.task!,
        root_shape: anthropic ? 'origin_root' : 'versioned_root',
        attempted_url: `${request.base_url}/probe`, elapsed_ms: 5,
        ...(suggestedBaseUrl ? { suggested_base_url: suggestedBaseUrl, candidates: [{
          base_url: suggestedBaseUrl, attempted_url: `${suggestedBaseUrl}/probe`,
          reachability: 'reachable' as const,
        }] } : { candidates: [] }),
      };
    });
    restorers.push(() => discover.mockRestore(), () => listProtocols.mockRestore(),
      () => listModels.mockRestore(), () => save.mockRestore(), () => probe.mockRestore());
    const deepLinkData = { platform: 'custom', name: '自定义网关', base_url: BASE_URL };
    function Harness() {
      const [control, modal] = AddPlatformModal.useModal({ deepLinkData, initialTask: 'chat', onSubmit: () => {} });
      return <><button onClick={() => control.open()}>Open provider configuration</button>{modal}</>;
    }
    const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map() }}>
      <ThemeProvider><Harness /></ThemeProvider>
    </SWRConfig></I18nextProvider>);
    fireEvent.click(page.getByRole('button', { name: 'Open provider configuration' }));
    const modelInput = await page.findByLabelText(zhSettings.modelId);
    fireEvent.change(page.getByPlaceholderText(zhSettings.apiKeyPlaceholder), { target: { value: 'test-secret' } });
    const choices = within(await page.findByRole('group', { name: zhSettings.providerAutoConfiguration.results }));
    expect(choices.getAllByRole('button')).toHaveLength(3);
    for (const protocol of ['openai.chat_text', 'openai.responses', 'anthropic.messages']) {
      expect(choices.getByRole('button', { name: protocol })).toBeTruthy();
    }
    expect(probe).toHaveBeenCalledTimes(3);
    expect(choices.getByRole('button', { name: 'openai.chat_text' }).getAttribute('aria-pressed')).toBe('true');

    fireEvent.change(modelInput, { target: { value: MODEL } });
    fireEvent.keyDown(modelInput, { key: 'Enter' });
    fireEvent.blur(modelInput);
    await waitFor(() => expect((page.getByRole('button', { name: 'anthropic.messages' }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(page.getByRole('button', { name: 'anthropic.messages' }));
    await waitFor(() => expect(page.getByRole('button', { name: 'anthropic.messages' }).getAttribute('aria-pressed')).toBe('true'));
    expect((page.getByLabelText(zhSettings.apiEndpoint) as HTMLInputElement).value).toBe(ANTHROPIC_BASE_URL);
    expect((page.getByPlaceholderText('bearer / token / header_key:x-api-key') as HTMLInputElement).value).toBe('header_key:x-api-key');
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 650)); });
    expect(probe).toHaveBeenCalledTimes(3);
    expect(discover).not.toHaveBeenCalled();
    expect(within(page.getByRole('group', { name: zhSettings.providerAutoConfiguration.results })).getAllByRole('button')).toHaveLength(3);
    const confirm = page.getByRole('button', { name: zhCommon.confirm }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);

    fireEvent.change(page.getByPlaceholderText(zhSettings.apiKeyPlaceholder), { target: { value: 'replacement-secret' } });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 650)); });
    expect(probe).toHaveBeenCalledTimes(3);
    expect(discover).not.toHaveBeenCalled();
    expect(page.getByText('配置已修改，尚未重新检测')).toBeTruthy();
    for (const option of within(page.getByRole('group', { name: zhSettings.providerAutoConfiguration.results })).getAllByRole('button')) {
      expect((option as HTMLButtonElement).disabled).toBe(true);
    }
    expect((page.getByLabelText(zhSettings.apiEndpoint) as HTMLInputElement).value).toBe(ANTHROPIC_BASE_URL);
    expect((page.getByPlaceholderText('bearer / token / header_key:x-api-key') as HTMLInputElement).value).toBe('header_key:x-api-key');
    const recheck = page.baseElement.querySelector<HTMLButtonElement>('[data-provider-protocol-recheck]')!;
    expect(recheck.disabled).toBe(false);
    fireEvent.click(recheck);
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(6));
    await waitFor(() => expect(confirm.disabled).toBe(false));
    expect(probe.mock.calls.slice(3).map(([request]) => request.credentials)).toEqual([
      { api_keys: ['replacement-secret'] }, { api_keys: ['replacement-secret'] }, { api_keys: ['replacement-secret'] },
    ]);
    const selectedInterface = page.getByRole('button', { name: 'anthropic.messages' });
    expect(selectedInterface.getAttribute('aria-pressed')).toBe('true');
    expect(selectedInterface.parentElement?.textContent).toContain(zhSettings.providerAutoConfiguration.unconfirmed);
    expect(page.getByRole('button', { name: 'openai.chat_text' }).parentElement?.textContent).toContain(OPENAI_BASE_URL);
    expect((page.getByLabelText(zhSettings.apiEndpoint) as HTMLInputElement).value).toBe(ANTHROPIC_BASE_URL);
    expect((page.getByPlaceholderText('bearer / token / header_key:x-api-key') as HTMLInputElement).value).toBe('header_key:x-api-key');
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 650)); });
    expect(probe).toHaveBeenCalledTimes(6);
    expect(discover).not.toHaveBeenCalled();

    fireEvent.click(confirm);
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0]![0]).toMatchObject({
      platform: 'custom', name: '自定义网关', base_url: ANTHROPIC_BASE_URL,
      auth_scheme: 'header_key:x-api-key', credentials: { api_keys: ['replacement-secret'] },
      initial_model: { model: MODEL, enabled: true, capabilities: [{
        task: 'chat', protocol: 'anthropic.messages', connection_role: 'default',
      }] },
    });
    expect(probe).toHaveBeenCalledTimes(6);
    expect(discover).not.toHaveBeenCalled();
  });

  test('uses a named connection for one task when another task shares the default connection', async () => {
    const imageManifest = {
      ...manifest, tasks: ['image_generation' as const], requested_task: 'image_generation' as const,
      recommendation: { ...manifest.recommendation, protocol_id: 'openai.images' },
      protocols: [{
        ...aliasProtocolManifest.protocols[0]!, protocol_id: 'openai.images', supported_tasks: ['image_generation' as const],
        endpoints: [{ ...aliasProtocolManifest.protocols[0]!.endpoints[0]!, task: 'image_generation' as const, default_value: '/images/generations' }],
      }],
    };
    const discover = spyOn(ipcBridge.mode.discoverModelProtocols, 'invoke').mockResolvedValue({ candidates: [], elapsed_ms: 1 });
    const listProtocols = spyOn(ipcBridge.modelProtocol.list, 'invoke').mockImplementation(async request =>
      request.task === 'image_generation' ? imageManifest : manifest);
    const listModels = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockResolvedValue({ models: [] });
    const save = spyOn(ipcBridge.mode.createProvider, 'invoke').mockResolvedValue({
      id: aliasProviderId, platform: 'custom', name: '多用途网关', base_url: BASE_URL,
      auth_scheme: 'bearer', has_credentials: true, models: [],
    });
    const probe = spyOn(ipcBridge.mode.probeConnection, 'invoke').mockImplementation(async request => {
      const anthropic = request.protocol === 'anthropic.messages';
      return {
        reachability: 'reachable', protocol: request.protocol!, task: request.task!,
        root_shape: anthropic ? 'origin_root' : 'versioned_root', attempted_url: `${request.base_url}/probe`, elapsed_ms: 5,
        ...(anthropic ? { suggested_base_url: ANTHROPIC_BASE_URL, candidates: [{
          base_url: ANTHROPIC_BASE_URL, attempted_url: `${ANTHROPIC_BASE_URL}/v1/messages`, reachability: 'reachable' as const,
        }] } : { candidates: [] }),
      };
    });
    restorers.push(() => discover.mockRestore(), () => listProtocols.mockRestore(),
      () => listModels.mockRestore(), () => save.mockRestore(), () => probe.mockRestore());
    const deepLinkData = { platform: 'custom', name: '多用途网关', base_url: BASE_URL };
    function Harness() {
      const [control, modal] = AddPlatformModal.useModal({ deepLinkData, initialTask: 'chat', onSubmit: () => {} });
      return <><button onClick={() => control.open()}>Open provider configuration</button>{modal}</>;
    }
    const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map() }}>
      <ThemeProvider><Harness /></ThemeProvider>
    </SWRConfig></I18nextProvider>);
    fireEvent.click(page.getByRole('button', { name: 'Open provider configuration' }));
    const modelInput = await page.findByLabelText(zhSettings.modelId);
    fireEvent.change(page.getByPlaceholderText(zhSettings.apiKeyPlaceholder), { target: { value: 'shared-secret' } });
    await page.findByRole('button', { name: 'anthropic.messages' });
    expect(probe).toHaveBeenCalledTimes(3);
    fireEvent.change(modelInput, { target: { value: MODEL } });
    fireEvent.keyDown(modelInput, { key: 'Enter' });
    fireEvent.click(page.getByRole('button', { name: zhSettings.addModelCallRoute }));
    fireEvent.click(page.getByRole('combobox', { name: zhSettings.selectModelCallRoute }));
    fireEvent.click(await page.findByRole('option', { name: zhSettings.modelTask.image_generation }));
    const imageChoiceName = `${zhSettings.modelTask.image_generation} · openai.images`;
    const chatChoiceName = `${zhSettings.modelTask.chat} · anthropic.messages`;
    await waitFor(() => expect((page.getByRole('button', { name: imageChoiceName }) as HTMLButtonElement).disabled).toBe(false));
    expect(probe).toHaveBeenCalledTimes(4);
    expect(probe.mock.calls[3]![0].task).toBe('image_generation');
    fireEvent.click(page.getByRole('button', { name: imageChoiceName }));
    fireEvent.click(page.getByRole('button', { name: chatChoiceName }));
    await waitFor(() => expect(page.getByRole('button', { name: chatChoiceName }).getAttribute('aria-pressed')).toBe('true'));
    fireEvent.click(page.getByRole('button', { name: chatChoiceName }));
    expect(page.getByRole('button', { name: imageChoiceName }).getAttribute('aria-pressed')).toBe('true');
    expect((page.getByLabelText(zhSettings.apiEndpoint) as HTMLInputElement).value).toBe(BASE_URL);
    expect((page.getByPlaceholderText('bearer / token / header_key:x-api-key') as HTMLInputElement).value).toBe('bearer');
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 650)); });
    expect(probe).toHaveBeenCalledTimes(4);
    expect(discover).not.toHaveBeenCalled();
    const confirm = page.getByRole('button', { name: zhCommon.confirm }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0]![0]).toMatchObject({
      base_url: BASE_URL, auth_scheme: 'bearer', credentials: { api_keys: ['shared-secret'] },
      initial_model: { model: MODEL, capabilities: [
        { task: 'chat', protocol: 'anthropic.messages', connection_role: 'detected_chat' },
        { task: 'image_generation', protocol: 'openai.images', connection_role: 'default' },
      ] },
      connections: [{ role: 'detected_chat', base_url: ANTHROPIC_BASE_URL,
        auth_scheme: 'header_key:x-api-key', credentials: { api_keys: ['shared-secret'] } }],
    });
    expect(save.mock.calls[0]![0].connections).toHaveLength(1);
    expect(probe).toHaveBeenCalledTimes(4);
    expect(discover).not.toHaveBeenCalled();
  });
});
